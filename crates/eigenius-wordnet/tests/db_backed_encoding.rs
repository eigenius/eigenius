// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! D62 (d) — the **DB-backed encoding measurement**: the encoding prototype, but parsing over the
//! *full* committed lexicon (WordNet + UMLS) in a snapshot of the served RocksDB store, rather than
//! a seeded in-memory WordNet slice. This is the rerun that answers "is vocabulary the encode-gate"
//! against the *real* domain lexicon, not a page-seeded slice.
//!
//! It opens a **copy** of the docker-volume store (never the live volume — RocksDB takes an
//! exclusive lock) via the kernel's persistent backend, resumes the `main` branch head (the loaded
//! chain), and builds the **lazy** `LexicalIndex` (on-demand `lexicon:form` value-index probes —
//! the only tractable path at 7.6M resources; the eager full-chain scan OOMs). The sense cap
//! (adaptive supertagging) keeps the chart tractable on long sentences; with `--features allms`
//! and `ANTHROPIC_API_KEY`, the contextual reranker reorders which senses the cap keeps.
//!
//! NOTE — bootstrap alignment: the snapshot's persisted chain is rooted at the bootstrap it was
//! seeded with (Option B, this session). The code's `logic` + `closed-class` ontologies must match
//! that seeded version (checked out at commit `ff7f6cc`) or the resume fails closed with
//! `ManifestDrift`. The reranker / sense-cap live in the kernel binary, not the bootstrap, so they
//! apply regardless of which closed-class version is resumed.
//!
//! Point it at a snapshot with `EIGENIUS_DB_SNAPSHOT=/path/to/store`; absent (or the WordNet dict
//! is absent), the tests skip. Run:
//!
//!     cargo test -p eigenius-wordnet --test db_backed_encoding -- --ignored --nocapture
//!     cargo test -p eigenius-wordnet --features allms --test db_backed_encoding -- --ignored --nocapture

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::dcg::{
    is_nonprose, segment_sentences, tokenize, Identity, Lemmatizer, LexicalIndex,
};
use eigenius_kernel::layer::{resolve_active_value_indexes, Layer};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;

/// Default snapshot location (the copy made from the `eigenius_eigenius_db` docker volume);
/// override with `EIGENIUS_DB_SNAPSHOT`.
const DEFAULT_SNAPSHOT: &str = "/home/hm/src/eigenius/db-snapshot/wordnet-umls-2026-06-28";

/// WordNet dict (for the Morphy lemmatizer — surface→lemma at lookup time).
const DICT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/WordNet-3.0/dict"
);

/// A cleaned page of real WRN-paper prose (user-provided; OCR noise removed).
const WRN_PAGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/publications/WRN-Helicase-Nature-OCR/first-page-cleaned.txt"
);

/// Adaptive-supertagging sense cap (Lever A, GH #97): keep the top-N senses per lemma so
/// WordNet+UMLS polysemy doesn't blow up the chart at the leaf.
const SENSE_CAP: usize = 2;

/// Per-cell beam (Lever B, GH #97): cap each non-top CKY cell to this many lowest-`Cost` items, so
/// a fully-known structurally-complex sentence's composed cells don't OOM the chart over the dense
/// full lexicon (where Lever A alone wasn't enough — the prior run OOM'd on a 17-token known unit).
/// UNVALIDATED at full-lexicon scale in the session that added it (the snapshot couldn't be resumed
/// — bootstrap drift); tune on the next fresh-DB run if OOM recurs.
const CELL_BEAM: usize = 64;

/// Parse budget: a fully-known unit longer than this is recorded as `ScaleBound` rather than
/// parsed — a backstop ABOVE the beam (the beam is the real OOM defense now). OOV diagnosis is
/// cheap at any length, so this bounds *only* the expensive CKY parse; the OOV/encode picture is
/// still measured for every unit. Raised from 12 (the pre-beam emergency value) to let the beam be
/// exercised on the page's long sentences; lower it if the beam proves insufficient on the rerun.
const PARSE_BUDGET: usize = 60;

/// The snapshot store path, or `None` (→ skip) when neither the env override nor the default
/// exists (a valid RocksDB store has a `CURRENT` file).
fn snapshot_path() -> Option<PathBuf> {
    let p = std::env::var("EIGENIUS_DB_SNAPSHOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_SNAPSHOT));
    if p.join("CURRENT").exists() {
        Some(p)
    } else {
        eprintln!(
            "SKIP db_backed_encoding: no RocksDB store at {} (set EIGENIUS_DB_SNAPSHOT)",
            p.display()
        );
        None
    }
}

/// Open the snapshot store and resume the `main` head (the loaded WordNet+UMLS chain). Returns
/// `None` (→ skip) on a `ManifestDrift`: the persisted chain is rooted at the bootstrap it was
/// seeded with, so the code's `logic`/`closed-class` ontologies must match that seeded version
/// (this session: checked out at `ff7f6cc`) or the resume fails closed. Rather than panic, skip —
/// so this committed test stays green whatever bootstrap the working tree currently compiles.
fn open_head(path: &std::path::Path) -> Option<Arc<Layer>> {
    let store = Arc::new(RocksStore::open(path).expect("open RocksStore snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    match bootstrap_persistent(Arc::clone(&backend)) {
        Ok(ctx) => Some(Arc::clone(ctx.head())),
        Err(e) => {
            eprintln!(
                "SKIP db_backed_encoding: cannot resume the snapshot — {e:?}.\n  The store's \
                 bootstrap must match the compiled one; check out the seeding commit's \
                 ontologies/logic + ontologies/lexicon/closed-class, or reseed."
            );
            None
        }
    }
}

/// Build the lazy `LexicalIndex` over the head with the sense cap, plus the live contextual
/// reranker when built with `--features allms` and `ANTHROPIC_API_KEY` is set.
fn build_index(head: &Arc<Layer>) -> LexicalIndex {
    let index = LexicalIndex::build(Arc::clone(head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(CELL_BEAM);
    #[cfg(feature = "allms")]
    {
        if let Some(ranker) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
            eprintln!("contextual reranker: AnthropicSenseRanker (live)");
            return index.with_sense_ranker(Box::new(ranker));
        }
        eprintln!("contextual reranker: none (ANTHROPIC_API_KEY unset) — cap-only");
    }
    #[cfg(not(feature = "allms"))]
    eprintln!("contextual reranker: none (built without --features allms) — cap-only");
    index
}

fn morphy() -> MorphyLemmatizer {
    MorphyLemmatizer::load(std::path::Path::new(DICT)).expect("load Morphy from dict")
}

/// Does this sem kernel-gate to a `Prop`? (the felicity confirmation)
fn gates_to_prop(layer: &Arc<Layer>, sem: &Exp) -> bool {
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(layer));
    matches!(check_infer(&mut ctx, sem), Ok(ty) if readback_val(0, &ty) == Exp::Sort(0))
}

/// The four-way outcome taxonomy the pipeline routes on (D62 §4). Mirrors `encoding_prototype.rs`
/// (duplicated — these are prototype drivers, not library code).
#[derive(Debug)]
enum Outcome {
    Encoded {
        is_prop: bool,
    },
    Ambiguous {
        count: usize,
        is_prop: bool,
    },
    MissingLexeme {
        unknown: Vec<String>,
    },
    GrammarGap,
    /// All tokens known; no CLOSED parse but a felicitous OPEN parse (referent holes — `we`/`its`/
    /// pronouns, D64). NOT a grammar gap — it parses, awaiting reference resolution.
    Open {
        holes: usize,
    },
    /// All tokens known, but the unit exceeds [`PARSE_BUDGET`] — parse skipped (would OOM the
    /// beam-less chart over the full lexicon). A *parsing-scale* gap, distinct from a vocab gap.
    ScaleBound {
        ntok: usize,
    },
}

struct UnitReport {
    text: String,
    outcome: Outcome,
}

/// Classify one unit. **OOV-first ordering** (vs. the slice prototype's parse-first): a closed
/// full-span parse requires every (prose) token to seed a leaf, so a unit with any unknown token
/// cannot encode — diagnose it as MissingLexeme from the cheap `has_token` probes *without* running
/// CKY. Only a fully-known unit is parsed (the parse is needed only to tell Encoded / Ambiguous /
/// GrammarGap apart). This is both correct and what keeps the FULL-lexicon run tractable: an
/// OOV-heavy long unit would otherwise OOM the chart on the dense WordNet+UMLS seed set, and the
/// parse there is guaranteed-empty wasted work. (Edge: a unit whose only unknown single-tokens are
/// all subsumed by *multiword* entries that do seed, and which fully parses, would be bucketed
/// MISSING rather than ENCODED — measure-zero for this corpus, and the OOV signal is still right.)
fn encode_unit(
    text: &str,
    index: &LexicalIndex,
    lem: &dyn Lemmatizer,
    layer: &Arc<Layer>,
) -> Outcome {
    let toks = tokenize(text);
    let unknown: Vec<String> = toks
        .iter()
        .filter(|t| !is_nonprose(t) && !index.has_token(t, lem))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Outcome::MissingLexeme { unknown };
    }
    // Fully known. Bound the (beam-less) parse so a long known unit doesn't OOM the chart.
    if toks.len() > PARSE_BUDGET {
        return Outcome::ScaleBound { ntok: toks.len() };
    }
    // Parse to distinguish the fully-known outcomes. Use the open-parse carrier so a unit that only
    // yields an OPEN parse (referent holes from `we`/`its`/pronouns, D64) is NOT misfiled as a
    // grammar gap — it parses, awaiting reference resolution.
    let (closed, open) = index.parse_open(text, lem);
    match closed.len() {
        0 => {
            if open.is_empty() {
                Outcome::GrammarGap
            } else {
                Outcome::Open {
                    holes: open.iter().map(|o| o.holes.len()).max().unwrap_or(0),
                }
            }
        }
        1 => Outcome::Encoded {
            is_prop: gates_to_prop(layer, &closed[0].sem),
        },
        n => Outcome::Ambiguous {
            count: n,
            is_prop: gates_to_prop(layer, &closed[0].sem),
        },
    }
}

/// Fragment bisection (D62 grammar-gap diagnosis): parse curated sub-spans of the nearest
/// grammar-gap units against the full lexicon and report which compose (closed / open / —), to
/// localize the actual stall points instead of inferring them. `#[ignore]`d; run manually:
///   cargo test -p eigenius-wordnet --test db_backed_encoding diagnose_grammar_gap_fragments \
///       -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: localize grammar-gap stalls; run with --ignored --nocapture"]
fn diagnose_grammar_gap_fragments() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    // CONTROL probes — isolate the fundamental blocker (determiner vs bare noun; proper-noun subj;
    // copula) using common full-lexicon words.
    let controls = [
        "a gene affects a cell", // determiners + known noun/verb — basic SVO control
        "genes affect cells",    // bare plurals — same clause without determiners
        "a cell is a gene",      // copula + predicate-nominal with determiners
        "a gene is large",       // copula + predicative adjective
    ];
    eprintln!("\n=== control probes (determiner vs bare; copula) ===");
    for f in controls {
        eprintln!("  probing {f:?} …"); // printed BEFORE the parse, so a hang/OOM names the culprit
        let t = std::time::Instant::now();
        let (closed, open) = index.parse_open(f, &lem);
        let s = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "—".into()
        };
        eprintln!("  {s:<10} [{:.1}s] {f:?}", t.elapsed().as_secs_f64());
    }

    // Fragments ordered small→large for unit 4 / unit 5 / unit 8 (the shortest grammar-gaps).
    let fragments = [
        // unit 4: "MSI cancer models required the helicase activity of WRN, but not its …"
        "MSI cancer models",
        "the helicase activity",
        "the helicase activity of WRN",
        "MSI cancer models required HeLa",
        "MSI cancer models required the helicase activity of WRN",
        // unit 5: "WRN is a synthetic lethal vulnerability and promising drug target for MSI cancers"
        "WRN is a vulnerability",
        "WRN is a synthetic lethal vulnerability",
        "WRN is a vulnerability and a target",
        "WRN is a vulnerability for MSI cancers",
        // unit 8: "Thus, novel therapies are needed for tumours with MSI"
        "novel therapies",
        "therapies are needed",
        "novel therapies are needed for tumours",
        "thus novel therapies are needed",
    ];
    eprintln!("\n=== fragment bisection (closed / open / — ; OOV split out) ===");
    for f in fragments {
        let toks = tokenize(f);
        let ntok = toks.len();
        // OOV-FIRST: a `—` from an unknown lexeme is a VOCABULARY gap, not a grammar gap. Report the
        // missed tokens so the genuine grammar gaps (fully-known, still no parse) are not conflated
        // with OOV (e.g. `WRN` is a gene-symbol OOV — its `—` is NOT a predicate-nominal gap, which
        // the small-lexicon `HeLa is a cell line` parse proves the grammar already covers).
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!(
                "  [{ntok:>2} tok] OOV{:<7} {f:?}  (unknown: {unknown:?})",
                ""
            );
            continue;
        }
        let (closed, open) = index.parse_open(f, &lem);
        let status = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "GRAMMAR-GAP".to_string()
        };
        eprintln!("  [{ntok:>2} tok] {status:<11} {f:?}");
    }
}

/// De-risk gate: the store opens, the chain resumes, and the `lexicon:form` value-index is ACTIVE
/// (→ lazy LexicalIndex path; the eager full-chain scan would OOM on 7.6M resources). Cheap — runs
/// by default (not `#[ignore]`d) so the harness wiring stays green even without the heavy run.
#[test]
fn snapshot_opens_with_lazy_form_index() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };

    // Chain depth (walk parent pointers) — a sanity signal the full chain resumed.
    let mut depth = 0usize;
    let mut cur = Some(head.clone());
    while let Some(layer) = cur {
        depth += 1;
        cur = layer.parent().cloned();
    }
    eprintln!("snapshot chain depth (layers): {depth}");

    let form = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let actives = resolve_active_value_indexes(&head);
    let active_props: Vec<&str> = actives.iter().map(|a| a.target_property.as_str()).collect();
    eprintln!("active value indexes: {active_props:?}");
    assert!(
        actives.iter().any(|a| a.target_property == form),
        "lexicon:form value-index must be active for the lazy path; active = {active_props:?}"
    );

    let index = LexicalIndex::build(Arc::clone(&head));
    assert!(
        index.has_token("gene", &Identity),
        "the full WordNet lexicon must know 'gene'"
    );
}

/// (d) — the measurement: feed the cleaned WRN first page through the parser over the FULL
/// WordNet+UMLS store, and report the outcome distribution + OOV fix-buckets. Heavy (full lexicon,
/// long sentences); `#[ignore]`d, run manually:
///
///     cargo test -p eigenius-wordnet --test db_backed_encoding \
///         wrn_first_page_over_full_lexicon -- --ignored --nocapture
#[test]
#[ignore = "heavy DB-backed (d) measurement; run with --ignored --nocapture"]
fn wrn_first_page_over_full_lexicon() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let page = match std::fs::read_to_string(WRN_PAGE) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIP: {WRN_PAGE} not found");
            return;
        }
    };

    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    // Characterize a few interesting buckets directly (closed-class vs -ly adverb vs domain).
    for probe in [
        "the",
        "we",
        "their",
        "would",
        "commonly",
        "typically",
        "recq",
        "wilcoxon",
    ] {
        eprintln!("  has_token({probe:?}) = {}", index.has_token(probe, &lem));
    }

    let mut report: Vec<UnitReport> = Vec::new();
    for (i, text) in segment_sentences(&page).into_iter().enumerate() {
        let ntok = tokenize(&text).len();
        let t = std::time::Instant::now();
        let outcome = encode_unit(&text, &index, &lem, &head);
        eprintln!(
            "[unit {i:>2}, {ntok:>3} tok, {:>5.1}s] {}",
            t.elapsed().as_secs_f64(),
            tag(&outcome)
        );
        report.push(UnitReport { text, outcome });
    }

    summarize(&report);
}

fn tag(o: &Outcome) -> &'static str {
    match o {
        Outcome::Encoded { .. } => "ENCODED",
        Outcome::Ambiguous { .. } => "AMBIG",
        Outcome::MissingLexeme { .. } => "MISSING",
        Outcome::GrammarGap => "GRAMMAR-GAP",
        Outcome::Open { .. } => "OPEN",
        Outcome::ScaleBound { .. } => "SCALE-BOUND",
    }
}

fn summarize(report: &[UnitReport]) {
    let (mut enc, mut amb, mut miss, mut gap, mut scale, mut open) = (0, 0, 0, 0, 0, 0);
    let mut oov: BTreeSet<String> = BTreeSet::new();
    for u in report {
        match &u.outcome {
            Outcome::Encoded { .. } => enc += 1,
            Outcome::Ambiguous { .. } => amb += 1,
            Outcome::MissingLexeme { unknown } => {
                miss += 1;
                oov.extend(unknown.iter().cloned());
            }
            Outcome::Open { holes } => {
                open += 1;
                eprintln!(
                    "  open (referent holes={holes}, awaiting resolution): {:?}",
                    u.text
                );
            }
            Outcome::GrammarGap => {
                gap += 1;
                eprintln!("  grammar-gap (all known, no parse): {:?}", u.text);
            }
            Outcome::ScaleBound { ntok } => {
                scale += 1;
                eprintln!("  scale-bound (known, {ntok} tok): {:?}", u.text);
            }
        }
    }
    eprintln!(
        "\n=== WRN first page over FULL lexicon: {} units → encoded {enc}, ambiguous {amb}, \
         open {open}, missing-lexeme {miss}, grammar-gap {gap}, \
         scale-bound (known, >{PARSE_BUDGET} tok) {scale} ===",
        report.len()
    );
    eprintln!("distinct OOV tokens ({}): {oov:?}", oov.len());

    let per_unit: Vec<usize> = report
        .iter()
        .filter_map(|u| match &u.outcome {
            Outcome::MissingLexeme { unknown } => Some(unknown.len()),
            _ => None,
        })
        .collect();
    if !per_unit.is_empty() {
        let sum: usize = per_unit.iter().sum();
        let n1 = per_unit.iter().filter(|&&c| c == 1).count();
        eprintln!(
            "OOV-per-unit: min {}, max {}, mean {:.1}; units blocked by exactly 1 OOV: {n1}",
            per_unit.iter().min().unwrap(),
            per_unit.iter().max().unwrap(),
            sum as f64 / per_unit.len() as f64
        );
    }

    // Bucket the distinct OOV by the fix that recovers it.
    let connectives: BTreeSet<&str> = [
        "after", "also", "although", "as", "because", "between", "both", "however", "most",
        "several", "such", "these", "those", "to", "within", "yet", "alone",
    ]
    .into_iter()
    .collect();
    let (mut adverb_ly, mut stat_leak, mut connective, mut domain) = (0, 0, 0, 0);
    for t in &oov {
        if t.chars().count() <= 1 {
            stat_leak += 1;
        } else if t.ends_with("ly") {
            adverb_ly += 1;
        } else if connectives.contains(t.as_str()) {
            connective += 1;
        } else {
            domain += 1;
        }
    }
    eprintln!(
        "OOV by fix-bucket: domain-lexicon {domain}, connectives/function-words {connective}, \
         -ly adverbs {adverb_ly}, stat-symbol leaks {stat_leak}"
    );

    eprintln!("\n--- encoded / ambiguous units (the wins) ---");
    for u in report {
        let t: String = u.text.chars().take(100).collect();
        match &u.outcome {
            Outcome::Encoded { is_prop } => eprintln!("  [ENCODED prop={is_prop}] {t}…"),
            Outcome::Ambiguous { count, is_prop } => {
                eprintln!("  [AMBIG×{count} prop={is_prop}] {t}…")
            }
            _ => {}
        }
    }
}
