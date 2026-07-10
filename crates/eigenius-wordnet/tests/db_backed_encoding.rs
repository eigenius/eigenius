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
//! (adaptive supertagging) keeps the chart tractable on long sentences; with `--features use-llm`
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
//!     cargo test -p eigenius-wordnet --features use-llm --test db_backed_encoding -- --ignored --nocapture

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::dcg::{
    extract_abbreviations, glossary_resources, ground_abbreviation, is_nonprose, pretty_term,
    segment_sentences, tokenize, Identity, Lemmatizer, LexicalIndex,
};
use eigenius_kernel::layer::{resolve_active_value_indexes, Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;

/// Default snapshot location — the out-of-tree `db-snapshot/` sibling of the repo (where
/// `scripts/reseed-lexicon-db.sh` / the native reseed write, `SNAPSHOT_ROOT = <repo>/../db-snapshot`),
/// resolved from `CARGO_MANIFEST_DIR` (portable, CWD-independent) rather than a hardcoded home path —
/// same convention as `DICT` below. Override with `EIGENIUS_DB_SNAPSHOT`.
const DEFAULT_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../db-snapshot/wordnet-umls-all-2026-07-08"
);

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
/// reranker when built with `--features use-llm` and `ANTHROPIC_API_KEY` is set.
fn build_index(head: &Arc<Layer>) -> LexicalIndex {
    // Combinatory-core spike: `EIGENIUS_COMBINATORY_CORE=1` enables the extra CCG combinators for the
    // A/B port measurement (default off = the established rule-by-rule path).
    let core = std::env::var("EIGENIUS_COMBINATORY_CORE")
        .map(|v| v == "1")
        .unwrap_or(false);
    if core {
        eprintln!("combinatory-core: ON");
    }
    // Cross-POS prune experiment (GH#97): EIGENIUS_POS_PRUNE=1 drops function words' open-class
    // nominal readings at seed time (can→container, for→noun, is→beryllium).
    let pos_prune = std::env::var("EIGENIUS_POS_PRUNE").is_ok();
    if pos_prune {
        eprintln!("cross-POS prune: ON");
    }
    let index = LexicalIndex::build(Arc::clone(head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(CELL_BEAM)
        .with_combinatory_core(core)
        .with_pos_prune(pos_prune);
    #[cfg(feature = "use-llm")]
    {
        if let Some(ranker) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
            eprintln!("contextual reranker: AnthropicSenseRanker (live)");
            return index.with_sense_ranker(Box::new(ranker));
        }
        eprintln!("contextual reranker: none (ANTHROPIC_API_KEY unset) — cap-only");
    }
    #[cfg(not(feature = "use-llm"))]
    eprintln!("contextual reranker: none (built without --features use-llm) — cap-only");
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
            is_prop: gates_to_prop(layer, closed[0].sem()),
        },
        n => Outcome::Ambiguous {
            count: n,
            is_prop: gates_to_prop(layer, closed[0].sem()),
        },
    }
}

/// VERIFY the sense lever (D62/GH#97): A/B the PAGE-beam (64) parse outcome for the 5 sentences
/// with the static cap (`baseline`) vs the contextual LLM reranker (`+llm`, only with
/// `--features use-llm` + ANTHROPIC_API_KEY). Measures whether contextual sense ranking frees enough
/// beam to parse at the operational beam. (The deterministic "closed-class-wins" filter was tried
/// and REVERTED — harmful; it can't distinguish `be`-verb from beryllium — see the d63 note.)
///   cargo test -p eigenius-wordnet --features use-llm --test db_backed_encoding \
///       verify_sense_lever_at_page_beam -- --ignored --nocapture
///
/// Beam-sensitivity (Lever 2, GH#97, measured 2026-06-30): at a fixed cell beam the 5
/// grammar-complete sentences cross to parsing at — S2 b64, S3 b128, S1/S5 b256, S4 not even at
/// b1024 (needs structural reduction). That measurement motivated **beam widen-on-failure**
/// (`CELL_BEAM_WIDEN_MAX`): `parse_scoped_open` now escalates the beam (with the sense cap) for a
/// known sentence that gaps, so the base beam stays the long-sentence OOM defense while
/// beam-limited short sentences are recovered. (So a fixed-beam sweep is no longer meaningful here —
/// `parse_scoped_open` auto-widens.)
#[test]
#[ignore = "diagnostic: A/B the sense lever at the page beam; run with --ignored --nocapture"]
fn verify_sense_lever_at_page_beam() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let sentences = [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic lethal targets.",
    ];
    let outcome = |idx: &LexicalIndex, s: &str| -> String {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };
    let mk = || {
        LexicalIndex::build(Arc::clone(&head))
            .with_sense_cap(SENSE_CAP)
            .with_cell_beam(CELL_BEAM)
    };

    // The variants to compare. The LLM variant only exists with `--features use-llm` +
    // ANTHROPIC_API_KEY (one reranker call per sentence).
    #[allow(unused_mut)]
    let mut variants: Vec<(String, LexicalIndex)> = vec![("baseline".into(), mk())];
    #[cfg(feature = "use-llm")]
    {
        if let Some(r) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
            variants.push(("+llm".into(), mk().with_sense_ranker(Box::new(r))));
        }
    }

    eprintln!("\n=== sense-lever A/B at PAGE beam ({CELL_BEAM}) ===");
    eprintln!(
        "variants: {:?}",
        variants.iter().map(|(l, _)| l).collect::<Vec<_>>()
    );
    for s in sentences {
        let cells: Vec<String> = variants
            .iter()
            .map(|(l, idx)| format!("{l}={}", outcome(idx, s)))
            .collect();
        eprintln!("  {}  {s:?}", cells.join("  "));
    }
}

/// D63 compound-morphology §2a diagnostic: show *exactly* how `based on X` parses TODAY (before the
/// Step 2b object+PP extension) — the adjective(`based`, data.adj) + `on`-adjunct reading, NOT the
/// verb-argument `base(x, X)`. Dumps every distinct closed sem. Run with:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       show_based_on_x_reading -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: show the today `based on X` adjective+adjunct reading; --ignored --nocapture"]
fn show_based_on_x_reading() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "Cells are based on genes.",
        "The method is based on sequencing.",
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        eprintln!(
            "\n=== {s:?} — {} closed, {} open ===",
            closed.len(),
            open.len()
        );
        let mut sems: Vec<String> = closed.iter().map(|it| pretty_term(it.sem())).collect();
        sems.sort();
        sems.dedup();
        for (i, sem) in sems.iter().enumerate() {
            eprintln!("  [{i}] {sem}");
        }
    }
}

/// D63 §8 C4 milestone: verify #8 (degree comparatives) parses AT SCALE — i.e. against the
/// WordNet-derived `dependence`/`dependent`/`sensitive` entries emitted by the importer (C1 bare
/// cat_measure, C2 nominalization projection, C3 relational/governed-prep reading), NOT the
/// hand-authored demo. The closed-class operators (`greater`/`more`/`less`, `than`) come from the
/// seeded `closed-class.esl`. Expected: `greater dependence on Y` and `more dependent on Y than Z`
/// produce `gt(deg_dependent(_,_), deg_dependent(_,_))`; `more sensitive than Z` the same over
/// `deg_sensitive`. #9 cardinality (`fewer genes`) is re-probed as a regression.
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_degree_comparative_at_scale -- --ignored --nocapture
/// RC-8 (d63-parse-gap-closure §Phase-2 backlog) — the sentence-2 shape `… is not simply a result of
/// …` over the real WordNet lexicon. Every grammar piece closes in the demo (copula + predicate
/// nominal + of-PP + negation + clausal complement), so isolate whether the residual is the ADVERB
/// `simply` (modifying a predicate nominal) or lexical/scale, with and without it.
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_rc8_at_scale -- --ignored --nocapture
#[test]
#[ignore = "probe: RC-8 `is not simply a result of` at scale; --ignored --nocapture"]
fn probe_rc8_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "genes are a result of mutations",     // predicate nominal + of-PP
        "genes are not a result of mutations", // + negation
        "genes are not simply a result of mutations", // + adverb `simply` (the s2 embedded clause)
        "cells suggest that genes are a result of mutations", // clausal + predicate nominal
        "cells suggest that genes are not simply a result of mutations", // full s2 shape
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  {tag:<10} {s:?}");
    }
}

/// FAITHFUL s20 isolation — the corpus sentence `WRN dependency may require specific lineages or a
/// stronger mutation phenotype` STILL gaps in the fresh-store measure despite the attributive-comparative
/// + coordination fixes (verified only on the SIMPLER demo proxy `HeLa may affect a gene or a larger cell
/// line`). Isolate which of the FULL structure — compound subject / adj+bare-plural coordinand /
/// compound-noun-in-comparative — actually gaps, over the real lexicon (WordNet words; WRN→gene proxy).
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_s20_isolation_at_scale -- --ignored --nocapture
#[test]
#[ignore = "probe: faithful s20 full-structure isolation at scale; --ignored --nocapture"]
fn probe_s20_isolation_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let outcome = |idx: &LexicalIndex, s: &str| -> String {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };
    // #2 verification on the --umls-all reseed (UMLS process/function-TUI mass fix): methylation
    // (C0025723, T044) / hypermethylation are now in-vocab AND mass, so bare `from methylation` should
    // CLOSE (was GAP). The full corpus sentence either closes (7→6) or reveals a residual search limit.
    let idx = build_index(&head);
    for w in ["methylation", "hypermethylation", "methylate"] {
        eprintln!("  has_token({w:?}) = {}", idx.has_token(w, &lem));
    }
    for (tag, s) in [
        ("#2 min-methyl", "inactivation arises from methylation"), //     was GAP → expect CLOSED
        ("#2 min-hyper", "inactivation arises from hypermethylation"), // CLOSED if its TUI is process/function
        (
            "#2 corpus-methyl",
            "Somatic MMR inactivation typically arises from methylation of the MLH1 promoter",
        ),
        (
            "#2 corpus-hyper",
            "Somatic MMR inactivation typically arises from hypermethylation of the MLH1 promoter",
        ), // the actual corpus #2
    ] {
        eprintln!("  {tag:<16} {:<10} {s:?}", outcome(&idx, s));
    }
    // Grammar vs search for the corpus #2 sentence: cap8/beam512, static rank.
    let hi = LexicalIndex::build(Arc::clone(&head))
        .with_sense_cap(8)
        .with_cell_beam(512);
    let c = "Somatic MMR inactivation typically arises from hypermethylation of the MLH1 promoter";
    eprintln!("  #2 corpus@cap8   {:<10} {c:?}", outcome(&hi, c));
}

#[test]
#[ignore = "diagnostic: #8 degree comparatives against the WordNet lexicon; --ignored --nocapture"]
fn verify_degree_comparative_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // #8 all-WordNet frames (bare-plural NPs sidestep domain-entity grounding), plus the demo frame
    // at scale, plus the #9 regression. `sensitive` is WordNet-only (absent from the demo lexicon).
    let sentences = [
        "cells show greater dependence on genes than mutations", // #8 nominalization + governed prep
        "cells are more dependent on genes than mutations",      // #8 predicative adjective
        "cells are more sensitive than mutations", // #8 predicative degree (WN-only adj)
        "HeLa affects greater dependence on BRCA1 than MSH2", // #8 demo frame, domain entities
        "HeLa affects fewer genes than MSH2",      // #9 cardinality regression
    ];
    for s in sentences {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("\n=== {tag}  {s:?} ===");
        let mut sems: Vec<(String, bool)> = closed
            .iter()
            .map(|it| (pretty_term(it.sem()), gates_to_prop(&head, it.sem())))
            .collect();
        sems.sort();
        sems.dedup();
        for (i, (sem, is_prop)) in sems.iter().enumerate() {
            eprintln!("  [{i}]{} {sem}", if *is_prop { " ⊨Prop" } else { "" });
        }
    }
}

/// D63 §5.3 C3-precision — the AT-SCALE witness: on the real WordNet lexicon, `dependent`'s gloss
/// governs `on`, so the importer emits `cat_measure / cat_pp_arg(prep_on)`; the WRONG preposition is
/// rejected at the feature-meet. The two sentences differ ONLY in the preposition. Unlike the unit
/// test (hand-authored demo entry), this proves the govern-prep detection + prep-tagged emission
/// survive the full-lexicon importer path. ASSERTS (skips cleanly when no snapshot / ManifestDrift).
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_governed_preposition_at_scale -- --ignored --nocapture
#[test]
#[ignore = "diagnostic+witness: C3-precision rejects *dependent to at scale; --ignored --nocapture"]
fn verify_governed_preposition_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // The witness is on the RELATIONAL reading, not full-sentence closure: at scale `dependent` also
    // has a bare `cat_measure` (C1) + a count-noun reading, which close the sentence regardless of the
    // preposition. C3-precision's claim is narrower — the ground-taking `deg_..._rel(ground, subject)`
    // term (built only through `cat_measure/cat_pp_arg(prep)`) must appear with the GOVERNED prep and
    // be ABSENT with the wrong one.
    let rel_terms = |s: &str| -> Vec<String> {
        let (c, _) = index.parse_open(s, &lem);
        let mut rels: Vec<String> = c
            .iter()
            .map(|it| pretty_term(it.sem()))
            .filter(|t| t.contains("_rel("))
            .collect();
        rels.sort();
        rels.dedup();
        eprintln!(
            "\n=== {s:?} — {} closed, {} relational ===",
            c.len(),
            rels.len()
        );
        for (i, t) in rels.iter().enumerate() {
            eprintln!("  rel[{i}] {t}");
        }
        rels
    };
    let on_rel = rel_terms("cells are more dependent on genes than mutations");
    let to_rel = rel_terms("cells are more dependent to genes than mutations");

    assert!(
        !on_rel.is_empty(),
        "`more dependent ON genes` must yield the relational deg_rel reading (prep_on marker meets the \
         importer-emitted governed prep_on)"
    );
    assert!(
        to_rel.is_empty(),
        "C3-precision: `*more dependent TO genes` must yield NO relational deg_rel reading — `dependent` \
         governs `on`, so cat_pp_arg(prep_to) fails the feature-meet. (Bare-measure / noun readings may \
         still close the sentence; the gate is on the relational term.) got: {to_rel:?}"
    );
}

/// D63 §8.5 / d63-comparative-phrasal §8 — AT-SCALE witness: an attributive comparative (`a stronger
/// gene`, s20's `a stronger mutation phenotype`) parses OPEN with a comparison-standard hole on the real
/// WordNet lexicon (the importer's `cmp_attrib_sem` bare `S[adj]\NP` reading). Was a grammar-GAP before.
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_attributive_comparative_at_scale -- --ignored --nocapture
#[test]
#[ignore = "diagnostic+witness: attributive comparative opens with a standard hole at scale; --ignored --nocapture"]
fn verify_attributive_comparative_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "cells affect a stronger gene",
        "cells require a stronger phenotype",
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        // The attributive-comparative reading is an OPEN parse (a comparison-standard hole) whose sem
        // compares a degree: `gt(deg_X(x), deg_X($anaphor$))`.
        let attrib = open.iter().find(|o| {
            !o.holes.is_empty() && {
                let t = pretty_term(o.item.sem());
                t.contains("gt(") && t.contains("deg_")
            }
        });
        eprintln!(
            "\n=== {s:?} — {} closed, {} open ===",
            closed.len(),
            open.len()
        );
        if let Some(o) = attrib {
            eprintln!(
                "  attributive-comparative OPEN (holes={}): {}",
                o.holes.len(),
                pretty_term(o.item.sem())
            );
        }
        assert!(
            attrib.is_some(),
            "`{s}` must have an OPEN attributive-comparative reading (gt(deg(x),deg(anaphor)) + hole) at scale"
        );
    }
}

/// D63 lexicon-augmentation diagnostic: are the UMLS `RecQ` atoms (C0084304 "RecQ Helicases") seeded as
/// `lexicon:form` entries in the snapshot? If so, a `TextIndex` over `lexicon:form` (BM25/token) would
/// ground the OOV surface `recq` → those atoms → the concept — without an HGNC import. The exact
/// `ValueIndex` misses them (`recq` ≠ `recq helicases`), which is why `recq` is OOV today. Run with:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_recq_atoms_in_snapshot -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: are RecQ atoms seeded (form-text-index grounding path)? --ignored --nocapture"]
fn probe_recq_atoms_in_snapshot() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for form in [
        "recq",
        "recq helicase",
        "recq helicases",
        "helicase, recq",
        "recq protein",
        "recq family of dna helicases",
        "recq helicase-like",
    ] {
        let known = index.has_token(form, &lem);
        let entries = index.debug_form_entries(form, &lem);
        eprintln!(
            "\n=== {form:?} — has_token={known}, {} entries ===",
            entries.len()
        );
        for (closed, cat, sense) in entries.iter().take(10) {
            eprintln!("  closed={closed}  sense={sense}  cat={cat}");
        }
    }
}

/// D2 (nominal-modification NF, d63-nominal-modification-normal-form.md §4/§8): does the snapshot carry
/// the corpus's genuine collocations as LEXICAL UNITS? A form with a `cat_n`/`cat_np` entry + a sense
/// (a `wn:`/`umlscui:` id) seeds as a multi-token span, so its compound reading is a leaf — not a
/// bracketing the compound rule reconstructs. Absent = the NF forces the all-adjective tree on it (the
/// coverage-policy decision D2). Run:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       d2_collocation_coverage -- --ignored --nocapture
#[test]
#[ignore = "D2: collocation-as-lexical-unit coverage over the snapshot; --ignored --nocapture"]
fn d2_collocation_coverage() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // Corpus collocations (space-joined lowercase, as `by_form` keys). The adjective-position
    // `synthetic lethal` is THE one the NF's interleaving hinges on; the rest are the first-5 CNL
    // compounds. `cell`/`lethality` are sanity controls (known heads).
    for form in [
        "synthetic lethality",
        "synthetic lethal",
        "synthetic lethal target",
        "synthetic lethal targets",
        "cell death",
        "dna repair",
        "repair process",
        "repair processes",
        "dna repair process",
        "dna repair processes",
        "cancer therapeutics",
        "genetic event",
        "genetic events",
        "co-occurrence",
        // controls:
        "cell",
        "lethality",
    ] {
        let known = index.has_token(form, &lem);
        let entries = index.debug_form_entries(form, &lem);
        // A collocation counts as a UNIT iff some entry is a nominal category carrying a sense id.
        let unit = entries.iter().any(|(_c, cat, sense)| {
            !sense.is_empty() && (cat.contains("cat_n") || cat.contains("cat_np"))
        });
        eprintln!(
            "\n=== {form:?} — has_token={known}  UNIT={unit}  {} entries ===",
            entries.len()
        );
        for (closed, cat, sense) in entries.iter().take(8) {
            eprintln!("  closed={closed}  sense={sense}  cat={cat}");
        }
    }
}

/// STEP 0 of the compound-pile plan (d63-compound-pile-collapse-plan.md): localize WHICH domain compound
/// tips each residual sentence over, and get its ROUTING (packed vs unpacked) — the fork that decides
/// Lever 1 (extend packing) vs Lever 2 (collapse structure). Bounded frames (one domain compound swapped
/// into a parseable generic base at a time; NO full sentence / double-swaps → avoids the OOM). Cap-only.
/// Run: cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       diagnose_compound_pile -- --ignored --nocapture
#[test]
#[ignore = "STEP 0: localize the exploding domain compound + its routing; --ignored --nocapture"]
fn diagnose_compound_pile() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // ROUTING-ONLY (fast: routes_packed does NOT parse; parsing the domain frames explodes/OOMs). The
    // fork — packed vs unpacked — is the Step-0 answer that picks Lever 1 (extend packing) vs Lever 2.
    let row = |idx: &LexicalIndex, s: &str| {
        let toks = tokenize(s);
        let unk: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !idx.has_token(t, &lem))
            .cloned()
            .collect();
        let routed = if idx.routes_packed(s, &lem) {
            "PACKED"
        } else {
            "UNPACK"
        };
        let oov = if unk.is_empty() {
            String::new()
        } else {
            format!("  OOV {unk:?}")
        };
        eprintln!("   [{routed}]{oov} {s:?}");
    };
    // (label, base generic frame, [one-domain-compound-swap frames])
    let groups: &[(&str, &str, &[&str])] = &[
        (
            "#7 — swap one domain compound into the generic base (×162)",
            "cells from lineages showed greater dependence on genes than counterparts",
            &[
                "MSI cell lines from lineages showed greater dependence on genes than counterparts", // subj compound
                "cells from these four lineages showed greater dependence on genes than counterparts", // from-PP
                "cells from lineages showed greater dependence on WRN than counterparts",  // obj (named indiv)
                "cells from lineages showed greater dependence on genes than their MSS counterparts", // than-obj
            ],
        ),
        (
            "#4 — swap one domain compound into the generic base (×121)",
            "we identified genes as a dependency in cells compared to lines",
            &[
                "we identified WRN as a dependency in cells compared to lines", // obj (named indiv)
                "we identified genes as the top preferential dependency in cells compared to lines", // as-complement
                "we identified genes as a dependency in MSI cell lines compared to lines", // in-PP compound
                "we identified genes as a dependency in cells compared to MSS cell lines", // compared-to compound
            ],
        ),
        (
            "#3 — swap one domain compound into the generic base (×6)",
            "some lines and some lines were represented by data sets",
            &[
                "some MSI lines and some MSS lines were represented by data sets", // coord subj compounds
                "some lines and some lines were represented by these screening data sets", // agent compound
            ],
        ),
    ];
    for (label, base, swaps) in groups {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("{label}");
        row(&index, base);
        for s in *swaps {
            row(&index, s);
        }
    }
    // TRIGGER LOCALIZATION: which construct forces unpacked? (baselines that SHOULD pack + one construct)
    eprintln!("\n════════════════════════════════════════════════════════════════");
    eprintln!("TRIGGER (expect PACKED baselines; UNPACK isolates the culprit construct)");
    for s in [
        "genes affect cells",                                // SVO baseline
        "genes are large",                                   // copula baseline
        "genes are attractive targets",                      // adj + compound baseline
        "cells showed dependence on genes", // relational noun + governed-prep PP (no comparative)
        "cells showed greater dependence than counterparts", // #7 comparative
        "cells are larger than genes",      // bare comparative-than
        "lines were represented by sets",   // #3 passive
        "we identified genes as a dependency", // #4 V-as-Y
        "genes affect cells compared to lines", // 'compared to' adjunct
    ] {
        row(&index, s);
    }
}

/// RE-ASSESS the 3 residual reranked gaps (#3 passive, #4 V-as-Y+compared-to, #7 comparative+PP): for
/// each, walk a fragment ladder (isolate the construction with generic fillers) at the DEFAULT beam,
/// then parse the full sentence at DEFAULT vs WIDE (cell_beam=1024). The verdict per sentence:
///   - construction parses in a fragment but full sentence GAPs at default, parses at WIDE ⇒ SEARCH-limited
///     (beam pressure), and the fragment where it first breaks localizes the driver;
///   - gaps even at WIDE ⇒ a real composition gap (grammar / missing rule), NOT beam pressure.
/// Cap-only. Run:
///   cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       diagnose_residual_gaps -- --ignored --nocapture
#[test]
#[ignore = "re-assess the 3 residual gaps (search vs grammar, per sentence); --ignored --nocapture"]
fn diagnose_residual_gaps() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    let outcome = |c: usize, o: usize| -> String {
        if c > 0 {
            format!("CLOSED×{c}")
        } else if o > 0 {
            format!("open×{o}")
        } else {
            "GRAMMAR-GAP".into()
        }
    };
    let probe = |idx: &LexicalIndex, s: &str| -> String {
        let toks = tokenize(s);
        let unk: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !idx.has_token(t, &lem))
            .cloned()
            .collect();
        if !unk.is_empty() {
            return format!("OOV {unk:?}");
        }
        let (c, o) = idx.parse_open(s, &lem);
        outcome(c.len(), o.len())
    };

    // (label, ladder fragments [default beam], full sentence [default + WIDE])
    let groups: &[(&str, &[&str], &str)] = &[
        (
            "#7 COMPARATIVE + PP (greater … on … than …)",
            &[
                "cells showed greater dependence than counterparts", // comparative alone
                "cells showed greater dependence on genes than counterparts", // + on-PP (governed)
                "cells from lineages showed greater dependence on genes than counterparts", // + subj from-PP
            ],
            "MSI cell lines from these four lineages showed greater dependence on WRN than their MSS counterparts.",
        ),
        (
            "#4 V-as-Y + in-PP + compared-to",
            &[
                "we identified genes as a dependency", // V-as-Y alone
                "we identified genes as a dependency in cells", // + in-PP
                "we identified genes as a dependency compared to cells", // + compared-to
                "we identified genes as a dependency in cells compared to lines", // both PPs
            ],
            "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines.",
        ),
        (
            "#3 PASSIVE + coordinated subject + complex agent",
            &[
                "lines were represented by sets",          // passive, minimal
                "lines were represented by data sets",     // + compound agent
                "some lines were represented by data sets", // + some-det
                "some lines and some lines were represented by data sets", // + coordinated subject
            ],
            "Some MSI lines and some MSS lines were represented by these screening data sets.",
        ),
    ];

    for (label, ladder, full) in groups {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("{label}");
        for f in *ladder {
            eprintln!("   [default] {:<12} {f:?}", probe(&index, f));
        }
        eprintln!("   ── full sentence ──");
        eprintln!("   [default] {:<12} {full:?}", probe(&index, full));
    }
}

#[test]
#[ignore = "TEMP dump of as/a/the/identified categories; --ignored --nocapture"]
fn dump_as_cats() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for w in ["DRIVE", "drive"] {
        eprintln!("── {w:?} ──");
        for (inj, cat, sense) in index.debug_form_entries(w, &lem).iter().take(8) {
            eprintln!("   inj={inj} sense={sense:?}  {cat}");
        }
    }
    for s in [
        "Project Achilles affects cells", // single named individual — WORKS
        "project DRIVE affects cells",    // does DRIVE name?
        "HeLa and BRCA1 affect cells",    // coordinate two plain names — control
        "Project Achilles and BRCA1 affect cells", // named individual + plain name
        "Project Achilles and project DRIVE affect cells", // two named individuals
    ] {
        let (c, o) = index.parse_open(s, &lem);
        eprintln!("\n{s:?}: closed={} open={}", c.len(), o.len());
        for it in c.iter().take(1) {
            eprintln!("   sem = {}", pretty_term(it.sem()));
        }
    }
}

/// ISOLATE the #4 "Project Achilles …" residual: start from the generic base that closes
/// (`we identified genes as a dependency in cells compared to lines`, CLOSED×112) and swap ONE domain
/// feature back in at a time — coordinated named subject, named object, superlative as-complement, and
/// each domain-compound PP — then a cumulative build-up, to localize what tips it into a GAP. A
/// WIDE-beam pass on the tipping cases separates SEARCH pressure (closes at WIDE) from a real grammar
/// gap (gaps even at WIDE). Cap-only. Run:
///   cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       diagnose_project_achilles -- --ignored --nocapture
#[test]
#[ignore = "isolate the #4 Project Achilles gap (which swap tips it); --ignored --nocapture"]
fn diagnose_project_achilles() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let wide = build_index(&head).with_cell_beam(1024);
    let lem = morphy();
    let outcome = |c: usize, o: usize| -> String {
        if c > 0 {
            format!("CLOSED×{c}")
        } else if o > 0 {
            format!("open×{o}")
        } else {
            "GRAMMAR-GAP".into()
        }
    };
    let probe = |idx: &LexicalIndex, s: &str| -> String {
        let toks = tokenize(s);
        let unk: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !idx.has_token(t, &lem))
            .cloned()
            .collect();
        if !unk.is_empty() {
            return format!("OOV {unk:?}");
        }
        let (c, o) = idx.parse_open(s, &lem);
        outcome(c.len(), o.len())
    };

    // Drill into the tipping phrase "the top preferential dependency" as an as-complement: vary the
    // determiner, each modifier alone, and the stacking, to localize the composition gap. Also probe
    // the NP in plain object position to see if the as-complement is implicated or the NP itself.
    let isolated: &[(&str, &str)] = &[
        (
            "BASE: as a dependency",
            "we identified genes as a dependency",
        ),
        ("as the dependency", "we identified genes as the dependency"),
        (
            "as a preferential dep.",
            "we identified genes as a preferential dependency",
        ),
        (
            "as a top dependency",
            "we identified genes as a top dependency",
        ),
        (
            "as the top dependency",
            "we identified genes as the top dependency",
        ),
        (
            "as a top pref. dep.",
            "we identified genes as a top preferential dependency",
        ),
        (
            "as the top pref. dep.",
            "we identified genes as the top preferential dependency",
        ),
        (
            "OBJ: affect the top pref dep",
            "genes affect the top preferential dependency",
        ),
        (
            "OBJ: affect a top pref dep",
            "genes affect a top preferential dependency",
        ),
        (
            "OBJ: affect a preferential dep",
            "genes affect a preferential dependency",
        ),
        ("OBJ: affect a top dep", "genes affect a top dependency"),
    ];
    // Cumulative: add the domain features together (generic subject first, then the real subject).
    let cumulative: &[(&str, &str)] = &[
        ("+obj+asY", "we identified WRN as the top preferential dependency in cells compared to lines"),
        ("+obj+asY+inPP", "we identified WRN as the top preferential dependency in MSI cell lines compared to lines"),
        ("+obj+asY+bothPP (generic subj)", "we identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines"),
        ("FULL (real subj)", "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines."),
    ];

    eprintln!("\n═══ ISOLATED single-feature swaps (default beam) ═══");
    for (label, s) in isolated {
        eprintln!("   {label:<28} {:<12} {s:?}", probe(&index, s));
    }
    eprintln!("\n═══ CUMULATIVE build-up (default | WIDE cell_beam=1024) ═══");
    for (label, s) in cumulative {
        eprintln!(
            "   {label:<32} default={:<12} wide={:<12} {s:?}",
            probe(&index, s),
            probe(&wide, s)
        );
    }
}

/// D1 diagnostic (nominal-modification NF §8): run the `modifier_class` discriminator over the v3
/// corpus's REAL adjective lexicon entries (per WordNet sense), confirming its verdict on actual data
/// — `attractive` must screen as `Gradable`, classificatory adjectives (`genetic`/`somatic`/`immune`)
/// must be `Intersective` (the only collapse-eligible class). Cap-only (no parsing/rerank needed —
/// this seeds adjective leaves and classifies their sems). Run:
///   cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       d1_modifier_class_over_corpus -- --ignored --nocapture
#[test]
#[ignore = "D1 diagnostic: modifier_class over the corpus's real adjectives; --ignored --nocapture"]
fn d1_modifier_class_over_corpus() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // The v3 corpus's attributive modifiers, grouped by the verdict expected of a correct D1:
    let modifiers = [
        // the §5 hazard + the hyphenated domain term (S5):
        "attractive",
        "synthetic-lethal",
        // scalar / evaluative → expect Gradable (screened, not collapsed):
        "greater",
        "stronger",
        "strong",
        "rare",
        "frequent",
        "novel",
        "promising",
        "essential",
        // classificatory → expect Intersective (collapse-eligible):
        "genetic",
        "somatic",
        "germline",
        "immune",
        "homologous",
        "colorectal",
        "endometrial",
        // hyphen state-compounds:
        "double-stranded",
        "microsatellite-stable",
        // mixed / to observe:
        "specific",
        "deficient",
        "hypermutable",
        "independent",
        "predictive",
        "preferential",
    ];
    let mut tally: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for m in modifiers {
        let rows = index.debug_modifier_classes(m, &lem);
        if rows.is_empty() {
            eprintln!("\n{m:?} — (no adjective entry seeded)");
            continue;
        }
        eprintln!("\n{m:?} — {} adjective entries:", rows.len());
        for (cat, sense, class) in &rows {
            eprintln!("   {class:<12} sense={sense:<26} cat={cat}");
            *tally.entry(class.clone()).or_default() += 1;
        }
    }
    eprintln!("\n=== ModifierClass tally over all adjective entries ===");
    for (class, n) in &tally {
        eprintln!("  {class:<12} {n}");
    }
}

/// D63 lexicon-augmentation §6a — VERIFY both grounding indexes over the RESEEDED snapshot:
/// **(a)** the form `core:TextIndex` grounds the OOV surface `recq` → its UMLS concept C0084304
/// (`augment_lexicon_backed`, the RecQ finding over the real atoms), and **(c)** the concept
/// `core:description` `core:TextIndex` is populated over verb/adjective **axiom** glosses — the
/// converter fix (axioms now carry `core:description`; nouns/instances already did). Run:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_grounding_indexes_over_snapshot -- --ignored --nocapture
#[test]
#[ignore = "verifies form+description grounding over a reseeded snapshot; --ignored --nocapture"]
fn verify_grounding_indexes_over_snapshot() {
    use eigenius_kernel::dcg::{
        augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
    };
    use eigenius_kernel::layer::resolve_active_text_indexes;
    use eigenius_kernel::ontology::resource::Value;
    use eigenius_kernel::query::text::analyzer::registry::analyzer_for;
    use eigenius_kernel::query::text::search::run_text_search;

    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };

    // Both indexes must be active over the reseeded head (declared in the lexicon schema layer).
    let active = resolve_active_text_indexes(&head);
    eprintln!(
        "=== active text indexes over snapshot head: {} ===",
        active.len()
    );
    for a in &active {
        eprintln!(
            "  idx={} target={} analyzer={}",
            a.iri.as_str(),
            a.target_property.as_str(),
            a.analyzer
        );
    }
    let form_prop = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let desc_prop = Iri::parse("urn:eigenius:core:description").unwrap();
    assert!(
        active.iter().any(|a| a.target_property == form_prop),
        "form_text_index active over the snapshot"
    );
    let desc_idx = active
        .iter()
        .find(|a| a.target_property == desc_prop)
        .expect("description_text_index active over the snapshot");

    // (a) FORM path — bare `recq` (OOV under the exact ValueIndex) grounds to C0084304 via the form
    // text index (BM25 over the seeded atoms), summed per concept.
    let lem = morphy();
    let aug = augment_lexicon_backed(
        &head,
        "recq affects HeLa.",
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );
    let recq = aug
        .added
        .iter()
        .find(|b| b.provenance.surface.to_lowercase() == "recq");
    match &recq {
        Some(b) => eprintln!(
            "\n(a) recq grounded_to={:?} confidence={:?}",
            b.provenance.grounded_to.as_ref().map(|i| i.as_str()),
            b.provenance.confidence
        ),
        None => eprintln!(
            "\n(a) recq NOT grounded; missing_oov={:?}",
            aug.missing_oov
                .iter()
                .map(|g| g.surface.as_str())
                .collect::<Vec<_>>()
        ),
    }
    let recq = recq.expect("recq grounds via the form text index");
    assert!(
        recq.provenance
            .grounded_to
            .as_ref()
            .map(|i| i.as_str().contains("C0084304"))
            .unwrap_or(false),
        "recq grounds to the RecQ family concept C0084304 (got {:?})",
        recq.provenance.grounded_to.as_ref().map(|i| i.as_str())
    );

    // (c) DESCRIPTION path — a verb axiom carries its synset gloss on `core:description`, and the
    // description index retrieves it by a distinctive gloss token (proves the converter fix +
    // index population over verb/adjective axioms, not just noun classes).
    let axiom_iri = Iri::parse("urn:eigenius:wn:v00860482_t").unwrap();
    let axiom = head
        .resolve(&axiom_iri)
        .expect("verb axiom wn:v00860482_t resolves in the snapshot");
    let gloss = match axiom.get(&desc_prop) {
        Some(Value::String(s)) => s.clone(),
        other => panic!("verb axiom carries no core:description gloss (got {other:?})"),
    };
    eprintln!(
        "\n(c) axiom {} core:description = {gloss:?}",
        axiom_iri.as_str()
    );
    assert!(
        gloss.contains("bravo"),
        "the axiom's description is the synset gloss"
    );
    let analyzer = analyzer_for(&desc_idx.analyzer).expect("analyzer for the description index");
    let hits = run_text_search(
        &head,
        head.storage().text_index.as_ref(),
        &desc_idx.iri,
        analyzer.as_ref(),
        "applaud bravo",
    )
    .expect("description search ok");
    eprintln!(
        "\n(c) description search 'applaud bravo' → {} hits (top 10):",
        hits.len()
    );
    for h in hits.iter().take(10) {
        eprintln!("  subj={} score={}", h.subject.as_str(), h.score);
    }
    assert!(
        hits.iter().any(|h| h.subject == axiom_iri),
        "the verb axiom is retrievable via its gloss token in the description index"
    );
}

/// End-to-end **OOV closure** over the WRN first page against the full lexicon — the DETERMINISTIC
/// (no-LLM) grounding pipeline. Measures the token-level OOV the augmentation leaves: baseline
/// (`augment_document_only`, deterministic Schwartz-Hearst abbreviations) vs after form+description
/// grounding (`augment_lexicon_backed`, nominal). The residual gaps are the fail-closed findings — what
/// the (B) LLM POS proposer (verb/adjective OOVs) and Phase-3 synthesis (genuinely novel terms) would
/// target next. Run:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       wrn_page_oov_closure_deterministic -- --ignored --nocapture
#[test]
#[ignore = "OOV closure over the WRN page (deterministic, nominal); --ignored --nocapture"]
fn wrn_page_oov_closure_deterministic() {
    use eigenius_kernel::dcg::{
        augment_document_only, augment_lexicon_backed, NoAbbreviationProposer,
        NominalCategoryProposer,
    };
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let doc = std::fs::read_to_string(&page_path).expect("read WRN page");
    let lem = morphy();

    let base = augment_document_only(&head, &doc, &NoAbbreviationProposer, &lem);
    let full = augment_lexicon_backed(
        &head,
        &doc,
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );

    eprintln!("=== WRN page OOV closure (deterministic, nominal) ===");
    eprintln!("baseline OOV (document-only): {}", base.missing_oov.len());
    eprintln!("added (abbrev + grounded):    {}", full.added.len());
    eprintln!("residual OOV:                 {}", full.missing_oov.len());
    eprintln!("\n-- grounded / added --");
    for b in &full.added {
        eprintln!(
            "  {:?} → {:?}  [{:?}]",
            b.provenance.surface,
            b.provenance.grounded_to.as_ref().map(|i| i.as_str()),
            b.provenance.method
        );
    }
    eprintln!("\n-- residual OOV (fail-closed findings) --");
    let mut res: Vec<&str> = full
        .missing_oov
        .iter()
        .map(|g| g.surface.as_str())
        .collect();
    res.sort();
    res.dedup();
    for s in &res {
        eprintln!("  {s:?}");
    }
}

/// Verify the `--umls-all` coverage win directly: `wilcoxon` (C0871608, T170 — outside the WRN-subset
/// TUIs) grounds over the full corpus, and `pcr-based` closes via the SHIPPED `X-based` compound rule
/// once its base `pcr` (C0032520, T063) is loaded (`docs/notes/d63-compound-morphology.md` §2a). Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_wilcoxon_pcr_grounding -- --ignored --nocapture
#[test]
#[ignore = "verify wilcoxon/pcr grounding over the --umls-all snapshot; --ignored --nocapture"]
fn probe_wilcoxon_pcr_grounding() {
    use eigenius_kernel::dcg::{
        augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
    };
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for t in ["pcr", "pcr-based", "wilcoxon", "cas9-mediated"] {
        eprintln!("has_token({t:?}) = {}", index.has_token(t, &lem));
    }
    let aug = augment_lexicon_backed(
        &head,
        "The wilcoxon test compared MSI and MSS cell lines. A pcr-based assay confirmed the result.",
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );
    eprintln!("-- grounded --");
    for b in &aug.added {
        eprintln!(
            "  {:?} → {:?}",
            b.provenance.surface,
            b.provenance.grounded_to.as_ref().map(|i| i.as_str())
        );
    }
    eprintln!(
        "residual OOV: {:?}",
        aug.missing_oov
            .iter()
            .map(|g| g.surface.as_str())
            .collect::<Vec<_>>()
    );
}

/// Grammar-gap ROOT-CAUSE battery (`2026-07-05`): short isolation probes for each construction in the
/// `--umls-all` run's 20 grammar-gaps, over the augmented index (so subjects like MSI/WRN are grounded +
/// overlaid — the run's config). Each prints CLOSED×n / OPEN×n / GAP so the blocker is localized to the
/// construction, not the subject. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_grammar_gap_root_causes -- --ignored --nocapture
#[test]
#[ignore = "grammar-gap root-cause battery over the --umls-all snapshot; --ignored --nocapture"]
fn probe_grammar_gap_root_causes() {
    use eigenius_kernel::dcg::{
        augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
    };
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let probes = [
        // — argument-PP verb (Step-2 fix target): the note's contrast + the actual object —
        "instability contributes to cells",
        "MSI contributes to cells",
        "MSI contributes to several cancers",
        "MSI results from deficiency",
        "cells respond to therapy",
        "MSI is associated with responses",
        // — adjunct-PP verb (should VP-adjoin per Step-1) —
        "MSI occurs in cancers",
        "MSI arises from deficiency",
        // — comparative `than` —
        "cells showed greater dependence than counterparts",
        "cells contained fewer mutations than lineages",
        // — `V X as Y` predicative —
        "we evaluated MSI as a biomarker",
        // — copula compound kind —
        "regions are microsatellites",
        "nucleotide repeat regions are microsatellites",
        // — object coordination (mismatched NPs) —
        "WRN requires lineages or a phenotype",
        // — adjective + PP complement —
        "classifications were concordant with phenotyping",
        // — linking verb + adjective —
        "findings remained true",
        // — named entity —
        "MSI arises from Lynch syndrome",
    ];
    // Augment the whole battery as one document so OOV subjects (MSI/WRN/…) are grounded + overlaid.
    let doc = probes.join(". ");
    let aug = augment_lexicon_backed(
        &head,
        &doc,
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );
    eprintln!(
        "augmentation: {} grounded, {} residual",
        aug.added.len(),
        aug.missing_oov.len()
    );
    let index = build_index(&head).with_document_augmentation(&aug);
    for t in [
        "msi",
        "wrn",
        "lynch syndrome",
        "microsatellites",
        "concordant",
        "remained",
        "biomarker",
    ] {
        eprintln!("has_token({t:?}) = {}", index.has_token(t, &lem));
    }
    eprintln!("-- probes --");
    for s in probes {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// STEP 4 (RC-1) — witness the bare-UMLS-noun-subject mechanism (d63-parse-gap-closure §3/§4).
/// Part 1: the actual `lexicon:cat` of the abbreviation forms in the snapshot (count `num_any` vs `mass`
/// vs `cat_np`). Part 2: a determiner/number/mass battery isolating whether a determiner or a mass/plural
/// reading turns the bare `MSI` subject into a parse — confirming the count-vs-mass diagnosis. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_step4_bare_umls_subject -- --ignored --nocapture
#[test]
#[ignore = "Step 4 (RC-1): bare-UMLS-subject mechanism; --ignored --nocapture"]
fn probe_step4_bare_umls_subject() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    // Part 1 — the emitted cats (count `num_any` vs `mass` vs `cat_np`) for the abbreviation forms and the
    // WordNet mass baseline (`instability`) + a count baseline (`gene`/`genes`).
    for form in ["msi", "mmr", "mss", "instability", "gene", "genes"] {
        let entries = index.debug_form_entries(form, &lem);
        eprintln!("=== {form:?} — {} entries ===", entries.len());
        for (closed, cat, sense) in entries.iter().take(8) {
            eprintln!("  closed={closed} sense={sense:<16} cat={cat}");
        }
    }
    // Part 2 — subject battery: does a determiner / mass / plural fix the bare subject?
    eprintln!("-- subject battery (all forms known; no augmentation) --");
    for s in [
        "MSI contributes to cells",         // bare count (num_any) — GAP expected
        "the MSI contributes to cells",     // + determiner
        "MSI contribute to cells",          // bare, plural agreement
        "instability contributes to cells", // bare MASS (WordNet) — CLOSED expected
        "the instability contributes to cells", // mass + determiner
        "genes contribute to cells",        // bare PLURAL count — kind
        "gene contributes to cells",        // bare SINGULAR count — GAP expected (English)
        "a gene contributes to cells",      // singular count + determiner
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// STEP 5 (RC-6) — localize the coordination gaps (d63-parse-gap-closure §4 Step 5). Isolation probes
/// for each coordination sub-case (a plain baseline, comma-list, quantified `some X and some Y`,
/// proper-noun, mismatched-NP `X or a Y`, apposition `the N genes …`) over the current snapshot, so the
/// fix scope is per-construction, not "coordination" as a monolith. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_step5_coordination -- --ignored --nocapture
#[test]
#[ignore = "Step 5 (RC-6): coordination sub-case localization; --ignored --nocapture"]
fn probe_step5_coordination() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    let probes = [
        // baseline — plain 2-item NP / adjective coordination (the plan says this already parses)
        "cells and genes affect HeLa",
        "colon and gastric cancers affect HeLa",
        // (a) comma-LIST coordination (3+ items) modifying a noun
        "colon, gastric and ovarian cancers affect HeLa",
        // (c) quantified NP coordination `some X and some Y`
        "some cells and some genes affect HeLa",
        // (d) proper-noun coordination as subject
        "HeLa and BRCA1 affect cells",
        // (e) MISMATCHED-NP object coordination — bare-plural `or` singular-indefinite (different cats)
        "WRN affects genes or a phenotype",
        "WRN affects genes or cells", // matched control (both bare plural) — should coordinate
        // (b) noun-name APPOSITION + name-list
        "the genes BRCA1 and MSH2 affect cells",
        // the actual RC-6 sentences (post-mass-shim status)
        "some MSI lines and some MSS lines were represented by data sets",
        "WRN dependency may require specific lineages or a stronger mutation phenotype",
    ];
    for s in probes {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// STEP 5 (RC-6) — VERIFY the close-apposition rule (`appose_group`, category.rs): a definite/bare
/// common-noun head + a coreferential name-group passes the group through (gated on the members being
/// of the head's base kind), so it rides the distributive-subject / -object machinery. Isolates each
/// syntactic POSITION (subject / bare / object / prep-object) + the felicity reject, so a residual GAP
/// localizes to the position, not the apposition rule. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-2026-07-06 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding probe_step5_apposition -- --ignored --nocapture
/// RC-2 comparatives — category dump + gap localization. The gap sentences use ATTRIBUTIVE comparatives
/// (`greater dependence`, `fewer mutations`, `a stronger phenotype`), unlike the existing PREDICATIVE
/// machinery (`X is larger than Y` — `(S[adj]\NP)/cat_pp_than`). This dumps what category the comparative
/// forms actually get on the real lexicon (positive? predicative comparative? lemmatized to base?) and
/// which of the sub-shapes gap. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-2026-07-06 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding probe_rc2_comparatives -- --ignored --nocapture
#[test]
#[ignore = "RC-2 comparatives: category dump + gap localization; --ignored --nocapture"]
fn probe_rc2_comparatives() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for form in [
        "great",
        "greater",
        "few",
        "fewer",
        "strong",
        "stronger",
        "larger",
        "dependence",
        "than",
    ] {
        eprintln!("  TYPES {form}:");
        for (aug, cat, sense) in index.debug_form_entries(form, &lem) {
            let a = if aug { "+" } else { " " };
            eprintln!("     {a} {cat}   [{sense}]");
        }
    }
    for s in [
        "a stronger phenotype affects cells", // #12 attributive comparative, NO than
        "greater dependence affects cells",   // attributive comparative + noun, isolated
        "WRN showed greater dependence than genes", // the than-clause with a comparative
        "cells contained fewer mutations than genes", // #9 shape (simplified)
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

#[test]
#[ignore = "Step 5 (RC-6): apposition-rule verification; --ignored --nocapture"]
fn probe_step5_apposition() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    let probes = [
        // Apposition (Step 5) regression witnesses:
        "the genes BRCA1 and MSH2 affect cells", //             subject apposition
        "mutations in the genes BRCA1 and MSH2 cause cancer", // prep-object apposition
        // Comma-list connective inheritance (Step 5b):
        "MSH2, MSH6, PMS2 or MLH1 affect cells", //             bare comma-OR name list (was GAP)
        "the MMR genes MSH2, MSH6, PMS2 or MLH1 affect cells", // full corpus-shape apposition (was GAP)
        "mutations in the MMR genes MSH2, MSH6, PMS2 or MLH1 cause cancer", // corpus prep-obj shape (GAP)
        // Localize the prep-obj GAP: compound head vs comma-or list, in prep-object position.
        "mutations in the MMR genes BRCA1 and MSH2 cause cancer", // compound head + simple `and`
        "mutations in the genes MSH2, MSH6, PMS2 or MLH1 cause cancer", // plain head + comma-`or`
        "WRN affects the MMR genes MSH2, MSH6, PMS2 or MLH1", // same apposition in OBJECT position
        "colon, gastric and ovarian cancers affect HeLa", //    adjective comma-AND list (no regression)
        // FELICITY reject — genes are not cells; the apposition must NOT license "the cells BRCA1 …".
        "the cells BRCA1 and MSH2 affect HeLa",
    ];
    for s in probes {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// S3 over-prune localization (GH#97): `Each event alone does not lead to cell death` gaps WITH the
/// cross-POS prune but parses without. This dumps what the prune drops for each of S3's function words
/// (closed / open-nominal=dropped / open-other=kept) and A/B-parses S3 sub-variants, to find which
/// dropped nominal reading S3 needs. Run with and without `EIGENIUS_POS_PRUNE=1`:
///   [EIGENIUS_POS_PRUNE=1] cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_s3_localization -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: localize the S3 over-prune; run with --ignored --nocapture"]
fn probe_s3_localization() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head); // honors EIGENIUS_POS_PRUNE
    let lem = morphy();

    eprintln!("=== S3 function-word entries (closed / open-NOMINAL=pruned / open-other=kept) ===");
    for w in [
        "each", "alone", "does", "not", "to", "lead", "cell", "death",
    ] {
        let es = index.debug_form_entries(w, &lem);
        let closed = es.iter().filter(|e| e.0).count();
        let open_nominal = es
            .iter()
            .filter(|e| {
                !e.0 && (e.2.starts_with("cat_n(")
                    || e.2.starts_with("cat_np(")
                    || e.1.contains("cat_n("))
            })
            .count();
        // crude: an entry is nominal if its cat string contains cat_n( or cat_np(
        let nominal = es
            .iter()
            .filter(|e| !e.0 && (e.1.contains("cat_n(") || e.1.contains("cat_np(")))
            .count();
        let open_other = es.iter().filter(|e| !e.0).count() - nominal;
        eprintln!("  {w:<7} closed={closed} open-nominal(pruned)={nominal} open-other(kept)={open_other}  [{open_nominal}]");
    }

    eprintln!("\n=== S3 sub-variants (outcome under current build_index config) ===");
    let variants = [
        "WRN leads to cell death",         // control: lead + to-PP, name subject
        "each event leads to cell death",  // + each
        "events alone lead to cell death", // + alone
        "WRN does not lead to cell death", // + do-support negation
        "WRN does not affect cells",       // do-support TRANSITIVE, no to-PP
        "WRN does not affect a gene",      // do-support transitive, GQ object
        "WRN affects cells",               // control: finite transitive, no do-support
        "each event alone leads to cell death", // each + alone, no do-support
        "each event does not lead to cell death", // each + do-support, no alone
        "Each event alone does not lead to cell death.", // full S3
    ];
    for s in variants {
        let (c, o) = index.parse_open(s, &lem);
        let tag = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        };
        // Print the first parse's sem so we can tell a REAL reading from noun-pile junk.
        let sem = c
            .first()
            .map(|it| it.sem())
            .or_else(|| o.first().map(|op| op.item.sem()));
        // Raw pretty-print (no eval — open parses carry unbound `$quant$` holes that can't be
        // evaluated), enough to tell a real verb/prep reading from noun-pile / mis-typed junk.
        let sem_s = sem
            .map(|e| {
                eigenius_kernel::dcg::pretty_term(e)
                    .chars()
                    .take(160)
                    .collect::<String>()
            })
            .unwrap_or_default();
        eprintln!("  {tag:<11} {s:?}\n      → {sem_s}");
    }
}

/// Function-word-noise enumeration (D62/GH#97): for each function word in the 5 sentences, list its
/// CLOSED-class (grammatical) vs OPEN-class (wordnet/umls noun/verb/adj) entries. The open-class
/// senses on function words are what let the compound rule chain across copulas/determiners into the
/// spurious refined-noun piles that saturate the beam. `#[ignore]`d; run:
///   cargo test -p eigenius-wordnet --test db_backed_encoding enumerate_function_word_noise \
///       -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: enumerate function-word open-class noise; run with --ignored --nocapture"]
fn enumerate_function_word_noise() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // The function/closed-class words occurring across the 5 CNL sentences.
    let words = [
        "is", "an", "a", "the", "are", "between", "two", "these", "each", "of", "for", "to", "can",
        "does", "not", "alone", "this", "and", "or",
    ];
    for w in words {
        let entries = index.debug_form_entries(w, &lem);
        let closed: Vec<&(bool, String, String)> = entries.iter().filter(|e| e.0).collect();
        let open: Vec<&(bool, String, String)> = entries.iter().filter(|e| !e.0).collect();
        eprintln!(
            "\n{w:?}: {} closed-class, {} OPEN-class (noise candidates)",
            closed.len(),
            open.len()
        );
        for (_, cat, sense) in &open {
            eprintln!("    OPEN  {sense:<20} {cat}");
        }
    }
}

/// Pretty-print the EigenTT sem (`Prop`) of the best parse of each of the first 5 CNL v2 sentences.
/// The parses are OPEN (referent/quant holes), so this shows the reduced normal form of the
/// lowest-cost parse. Honors `EIGENIUS_POS_PRUNE`. Run:
///   EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet --test db_backed_encoding \
///       pretty_print_first_five_sems -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: pretty-print the first-5 sems; run with --ignored --nocapture"]
fn pretty_print_first_five_sems() {
    let Some(head) = snapshot_path().and_then(|p| open_head(&p)) else {
        return;
    };
    let index = build_index(&head);
    let lem = morphy();
    let sentences = [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic lethal targets.",
        "Many cancers exhibit an impairment of a DNA repair pathway.",
        "This impairment can lead to dependence on specific repair proteins.",
    ];
    for (i, s) in sentences.iter().enumerate() {
        let (c, o) = index.parse_open(s, &lem);
        let (n, sem) = if !c.is_empty() {
            (c.len(), Some(c[0].sem()))
        } else if !o.is_empty() {
            (o.len(), Some(o[0].item.sem()))
        } else {
            (0, None)
        };
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("S{}  {s}", i + 1);
        eprintln!("     ({n} parse(s); best shown)");
        match sem {
            Some(e) => {
                eprintln!("  ⟦·⟧ = {}", eigenius_kernel::dcg::pretty_term(e));
                let mut iris: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                collect_iris(e, &mut iris);
                eprintln!("  where:");
                for iri_s in &iris {
                    let local = iri_s.rsplit(':').next().unwrap_or(iri_s);
                    // Only the opaque synset/CUI/axiom codes need glossing.
                    if !(local.starts_with('n')
                        || local.starts_with('C')
                        || local.starts_with('v')
                        || local.starts_with("deg_")
                        || local.starts_with('a'))
                    {
                        continue;
                    }
                    let gloss = Iri::parse(iri_s)
                        .ok()
                        .and_then(|i| head.resolve(&i))
                        .and_then(|r| {
                            match r.get(&Iri::parse("urn:eigenius:core:description").unwrap()) {
                                Some(eigenius_kernel::ontology::resource::Value::String(s)) => {
                                    Some(s.clone())
                                }
                                _ => None,
                            }
                        })
                        .map(|d| d.chars().take(60).collect::<String>());
                    if let Some(g) = gloss {
                        eprintln!("     {local:<14} = {g}");
                    }
                }
            }
            None => eprintln!("  (no parse)"),
        }
    }
    eprintln!();
}

/// Collect the opaque IRIs (synset classes, verb/adjective axioms, resources) a sem references.
fn collect_iris(e: &Exp, out: &mut std::collections::BTreeSet<String>) {
    use eigenius_kernel::nbe::term::Exp as E;
    match e {
        E::EigonClass(iri) | E::EigonAxiom(iri) => {
            out.insert(iri.as_str().to_string());
        }
        E::EigonResource(r) => {
            if let Some(id) = r.id() {
                out.insert(id.as_str().to_string());
            }
        }
        E::App(f, a) | E::Arrow(f, a) | E::Times(f, a) | E::Pair(f, a) => {
            collect_iris(f, out);
            collect_iris(a, out);
        }
        E::Lam(_, b) | E::Con(_, b) | E::Fst(b) | E::Snd(b) | E::Ann(b, _) => collect_iris(b, out),
        E::Pi(_, t, b) | E::Sig(_, t, b) => {
            collect_iris(t, out);
            collect_iris(b, out);
        }
        E::InductiveCtor(_, _, args) | E::InductiveType(_, args) => {
            for a in args {
                collect_iris(a, out);
            }
        }
        _ => {}
    }
}

/// The 7 worst noun-pile sentences (CNL v2, GH#97) — outcome + parse TIME, to measure the
/// compound-depth cost penalty (were 36–565s + GRAMMAR-GAP). Honors `EIGENIUS_POS_PRUNE`. Run:
///   EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_noun_pile_sentences -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: noun-pile sentences after the compound penalty; run with --ignored --nocapture"]
fn probe_noun_pile_sentences() {
    let Some(head) = snapshot_path().and_then(|p| open_head(&p)) else {
        return;
    };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "Some cancers do not respond to immune checkpoint blockade.",
        "Project Achilles screened cell lines with a CRISPR library.",
        "These observations suggest that WRN dependency is not simply a result of MMR deficiency.",
        "WRN dependency may require specific lineages or a stronger mutation phenotype.",
        "These cell lines contained fewer deletion mutations in microsatellite regions than typical lineages.",
        "We analysed these data sets for genes that are selectively essential in cancer cells with MSI.",
        "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines.",
    ] {
        let t = std::time::Instant::now();
        let (c, o) = index.parse_open(s, &lem);
        let tag = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  {tag:<11} [{:>6.1}s] {s:?}", t.elapsed().as_secs_f64());
    }
}

/// WIN PROBE for the packed forest (D63 Option A, blueprint §11 3f.4): parse a *packable* pile
/// sentence (no relatives/commas/coordination → the router engages packing) over the full lexicon,
/// with packing OFF vs ON, reporting outcome + wall-clock. With `EIGENIUS_PARSE_DEBUG=1` the packed
/// run also prints `forest nodes=N` — the pile's sense-product collapsed to O(nodes) vs the ~30k flat
/// items of the unpacked cell. Same (closed, open) ⇒ the win is a speed/space gain, not a parse
/// change. Honors `EIGENIUS_POS_PRUNE`. Run:
///   EIGENIUS_PARSE_DEBUG=1 EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding packed_win_probe -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: packed vs unpacked win probe; run with --ignored --nocapture"]
fn packed_win_probe() {
    let Some(head) = snapshot_path().and_then(|p| open_head(&p)) else {
        return;
    };
    let lem = morphy();
    // Packable pile sentences (no `which`/comma/coordination; index-independent verbs). `that` (both
    // restrictive-relative and complementizer) now packs (§11 3g.3).
    let sentences = [
        "DNA repair processes are attractive synthetic lethal targets.",
        "Synthetic lethality is an interaction between two genetic events.",
        // that-RELATIVE pile sentence — one of the worst unpacked (~199s in the noun-pile probe):
        "We analysed these data sets for genes that are selectively essential in cancer cells with MSI.",
    ];
    let unpacked = build_index(&head).with_packing(false);
    let packed = build_index(&head).with_packing(true);
    for s in sentences {
        eprintln!("\n{s:?}");
        for (name, idx) in [("unpacked", &unpacked), ("packed", &packed)] {
            let t = std::time::Instant::now();
            let (c, o) = idx.parse_open(s, &lem);
            eprintln!(
                "  {name:<9} closed×{} open×{} [{:>6.1}s]",
                c.len(),
                o.len(),
                t.elapsed().as_secs_f64()
            );
        }
    }
}

/// A/B witness for GH#97 Fix #2 (construction-time compound-depth CAP): parse the witnessed
/// pure-pile sentence (unit 32 — full-span cell recorded at 34,472 items pre-cap) at a WIDE beam,
/// with `EIGENIUS_PARSE_DEBUG=1`, and report the MAX per-cell `produced` (items BUILT before
/// beaming — the construction cost). Run once with the cap live and once with `MAX_COMPOUND_MODS`
/// bumped high to see the delta. `#[ignore]`d; run:
///   EIGENIUS_PARSE_DEBUG=1 EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding measure_pile_cell_population -- --ignored --nocapture 2>&1 \
///     | grep -oE 'produced=[0-9]+' | sort -t= -k2 -n | tail -1
#[test]
#[ignore = "diagnostic: max cell population of the pure-pile sentence; run with EIGENIUS_PARSE_DEBUG=1 --ignored --nocapture"]
fn measure_pile_cell_population() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = LexicalIndex::build(Arc::clone(&head))
        .with_sense_cap(2)
        .with_cell_beam(1024)
        .with_pos_prune(std::env::var("EIGENIUS_POS_PRUNE").is_ok());
    // Attach the live contextual reranker when built with --features use-llm (mirrors build_index),
    // so this probe measures the reranked serving path, not cap-only.
    #[cfg(feature = "use-llm")]
    let index = match eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
        Some(r) => {
            eprintln!("contextual reranker: AnthropicSenseRanker (live)");
            index.with_sense_ranker(Box::new(r))
        }
        None => {
            eprintln!("contextual reranker: none (ANTHROPIC_API_KEY unset)");
            index
        }
    };
    #[cfg(not(feature = "use-llm"))]
    eprintln!("contextual reranker: none (cap-only)");
    let lem = morphy();
    let s = "Some cancers do not respond to immune checkpoint blockade.";
    eprintln!("MEASURE (pile cell population): {s:?}");
    let (closed, open) = index.parse_open(s, &lem);
    eprintln!("  → closed×{} open×{}", closed.len(), open.len());
}

/// Chart-cell population analysis for the 5 CNL v2 sentences (user request 2026-06-30): parse each
/// at a WIDE beam (1024 ≈ uncapped at sense_cap=2) with `EIGENIUS_PARSE_DEBUG=1`, so the per-cell
/// shape histograms (`cat_shape`, type-indices erased) show WHERE the chart population concentrates
/// and WHETHER it is lexical/sense variation (one shape, many indices ⇒ a GH#93 type-narrowing
/// candidate) or structural ambiguity (many shapes ⇒ narrowing won't help). `#[ignore]`d; run:
///   EIGENIUS_PARSE_DEBUG=1 cargo test -p eigenius-wordnet --test db_backed_encoding \
///       analyze_chart_cells_first_five -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: chart-cell population analysis; run with EIGENIUS_PARSE_DEBUG=1 --ignored --nocapture"]
fn analyze_chart_cells_first_five() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    // Wide beam so the dumped cells show the true population, not the page-beam-capped view.
    // Honors EIGENIUS_POS_PRUNE so the pile shown is the residual AFTER the cross-POS prune.
    let index = LexicalIndex::build(Arc::clone(&head))
        .with_sense_cap(2)
        .with_cell_beam(1024)
        .with_pos_prune(std::env::var("EIGENIUS_POS_PRUNE").is_ok());
    let lem = morphy();
    let sentences = [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        // v3: `synthetic-lethal` hyphenated (lexicalized compound modifier, style-guide fix) so it is
        // ONE compound adjective, not a `synthetic` ∧ `lethal` adjective stack (d63-nominal-mod NF §4).
        "DNA repair processes are attractive synthetic-lethal targets.",
    ];
    for s in sentences {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("ANALYZE: {s:?}");
        let (closed, open) = index.parse_open(s, &lem);
        eprintln!("  → closed×{} open×{}", closed.len(), open.len());
    }
}

/// Per-sentence blocker diagnosis for the FIRST 5 CNL v2 sentences (user request 2026-06-30):
/// for each sentence, print token-level OOV, the full-sentence parse outcome, and a fragment
/// ladder that localizes the exact construction that stalls. `#[ignore]`d; run manually:
///   cargo test -p eigenius-wordnet --test db_backed_encoding diagnose_first_five_cnl \
///       -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: localize per-sentence blockers of CNL v2's first 5; run with --ignored --nocapture"]
fn diagnose_first_five_cnl() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    // SENTENCE-SHAPED minimal pairs (parse_open only returns full-span S parses, so a bare NP
    // fragment is always GRAMMAR-GAP and tells us nothing). Each group varies ONE construction at a
    // time, anchored on the known-good `genes are attractive targets` / `genes affect cells`, using
    // small-lexicon generic slot fillers (genes/cells) so the GRAMMAR is isolated from the specific
    // domain word's vocabulary/countability — the domain word is then swapped in as the LAST probe.
    let sentences: &[(&str, &[&str])] = &[
        (
            "THE 5 ACTUAL CNL v2 SENTENCES (end-to-end verdict)",
            &[
                "Synthetic lethality is an interaction between two genetic events.",
                "The co-occurrence of these two events leads to cell death.",
                "Each event alone does not lead to cell death.",
                "Scientists can exploit synthetic lethality for cancer therapeutics.",
                "DNA repair processes are attractive synthetic lethal targets.",
            ],
        ),
        (
            "ANCHORS (known-good)",
            &[
                "genes are attractive targets", // copula pred-nom, bare-pl subj + adj+noun pred
                "genes affect cells",           // bare-plural SVO control
            ],
        ),
        (
            "COPULA: number / bare predicate / stacked adjectives (S5, S1)",
            &[
                "genes are targets",                             // bare-plural predicate nominal
                "genes are attractive synthetic lethal targets", // 3 stacked attributive adjs
                "genes are interactions", // plural=plural pred-nom (S1 skeleton)
                "a gene is an interaction", // sg=sg pred-nom (S1 determiners)
            ],
        ),
        (
            "COMPOUND SUBJECT (S5 'DNA repair processes', S2 'co-occurrence')",
            &[
                "processes are attractive targets", // single common-noun plural subject
                "repair processes are attractive targets", // 2-noun compound subject
                "DNA repair processes are attractive targets", // 3-noun compound subject
            ],
        ),
        (
            "BARE-MASS OBJECT / SUBJECT (S4 'synthetic lethality', S1)",
            &[
                "genes exploit cells",    // verb 'exploit' + plain plural object (control)
                "genes affect lethality", // bare common-noun object (countability probe)
                "genes affect synthetic lethality", // adj + bare common-noun object
                "lethality affects cells", // bare common-noun SUBJECT
            ],
        ),
        (
            "DETERMINERS: each / these+numeral (S2, S3)",
            &[
                "each gene affects cells",      // 'each' determiner subject
                "these genes affect cells",     // plural 'these' subject
                "these two genes affect cells", // 'these' + numeral subject
                "the two genes affect cells",   // 'the' + numeral subject
            ],
        ),
        (
            "MODAL / DO-SUPPORT / NEGATION (S3, S4)",
            &[
                "genes can affect cells",       // modal + bare-plural subject
                "genes do not affect cells",    // do-support negation, bare plural
                "a gene does not affect cells", // do-support negation, singular
            ],
        ),
        (
            "PP ADJUNCTS: for / between (S4, S1)",
            &[
                "genes affect cells for therapies", // 'for' VP-adjunct, bare-plural object
                "a gene affects cells for a therapy", // 'for' VP-adjunct, singular objects
                "a gene is an interaction between cells", // 'between' noun-mod PP
            ],
        ),
    ];

    // Confirmatory probes for the two non-`to`-prep blockers + remaining constructions.
    let extras: &[&str] = &[
        "the impairment of a gene affects cells", // the + N + of-PP SUBJECT (S2 skeleton, no to-PP)
        "each gene alone affects cells",          // 'alone' floating adverb (S3)
        "a gene affects cell death",              // bare-compound singular OBJECT 'cell death' (S2)
        "genes are cell death", // 'death' bare-mass as predicate (probe countability)
    ];
    eprintln!("\n════════════════════════════════════════════════════════════════");
    eprintln!("EXTRAS (of-PP subj / 'alone' / bare-compound object)");
    for f in extras {
        let ft = tokenize(f);
        let unk: Vec<String> = ft
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unk.is_empty() {
            eprintln!(
                "    [{:>2}t] OOV         {f:?} (unknown: {unk:?})",
                ft.len()
            );
            continue;
        }
        let (c, o) = index.parse_open(f, &lem);
        let s = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GRAMMAR-GAP".into()
        };
        eprintln!("    [{:>2}t] {s:<12} {f:?}", ft.len());
    }
    // WIDE-BEAM test: does the 3-noun compound subject parse at cell_beam=1024? CLOSED/open ⇒ the
    // page-beam GAP is BEAM PRESSURE (GH #97 Lever B), not a missing compound rule.
    let wide = LexicalIndex::build(Arc::clone(&head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(1024);
    for f in [
        "DNA repair processes are attractive targets",
        "DNA repair processes are targets",
        // The 5 actual sentences at a wide beam — beam pressure (GH #97) vs a real composition gap.
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic lethal targets.",
        // S4 localization (gaps even at wide beam) — peel off modal / for-PP / each object.
        "scientists exploit synthetic lethality",
        "scientists exploit cells for therapies",
        "scientists exploit synthetic lethality for therapies",
        "scientists exploit synthetic lethality for cancer therapeutics",
        "scientists can exploit cells",
        "genes affect cancer therapeutics",
    ] {
        let (c, o) = wide.parse_open(f, &lem);
        let s = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GRAMMAR-GAP".into()
        };
        eprintln!("    [wide beam 1024] {s:<12} {f:?}");
    }

    for (sentence, ladder) in sentences {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("SENTENCE: {sentence:?}");
        // token-level OOV
        let toks = tokenize(sentence);
        let oov: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        eprintln!("  tokens: {} | OOV: {oov:?}", toks.len());
        eprintln!("  --- fragment ladder (small→large) ---");
        for f in *ladder {
            let ftoks = tokenize(f);
            let unknown: Vec<String> = ftoks
                .iter()
                .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                eprintln!(
                    "    [{:>2}t] OOV         {f:?}  (unknown: {unknown:?})",
                    ftoks.len()
                );
                continue;
            }
            let t = std::time::Instant::now();
            let (closed, open) = index.parse_open(f, &lem);
            let status = if !closed.is_empty() {
                format!("CLOSED×{}", closed.len())
            } else if !open.is_empty() {
                format!("open×{}", open.len())
            } else {
                "GRAMMAR-GAP".to_string()
            };
            eprintln!(
                "    [{:>2}t] {status:<12} [{:.1}s] {f:?}",
                ftoks.len(),
                t.elapsed().as_secs_f64()
            );
        }
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
        // PREP-OBJECT isolation probes (D62 §2 GQ-as-prep-object): name vs GQ object, and the
        // cat_pp (noun-mod) family vs the VP-adjunct family, to locate the residual gap.
        // D62 §2 GQ-as-prep-object coverage anchors: a quantified/bare-plural NP scopes into a
        // preposition's object slot (was: only a bare NAME could). Both prep families — the
        // post-nominal `cat_pp` noun-mod ("vulnerability for …") and the VP-adjunct ("needed
        // for …") — and all three object kinds (name / singular ∃-GQ / bare-plural deferred-Q).
        "therapies are needed for a gene", // VP-adjunct prep, singular GQ object  ⇒ CLOSED
        "WRN is a vulnerability for a gene", // cat_pp noun-mod, singular GQ object ⇒ CLOSED
        "HeLa affects a gene within cells", // bare-plural prep object (one deferred hole) ⇒ open
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

    // BEAM-PRESSURE probe (records the §2 prep-object residual's cause): "novel therapies are
    // needed for a/an … " is GRAMMAR-GAP at the page beam (64) yet OPENS at a wide beam — so the
    // residual is ambiguity explosion (attributive-adj `novel` over a bare-plural subject + a PP),
    // a Lever-B scale issue (GH #97), NOT a missing prep-object rule (the singular/bare-plural prep
    // objects above already parse). Witnessed: at cell_beam=1024 it yields open×216.
    let wide = LexicalIndex::build(Arc::clone(&head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(1024);
    let (wclosed, wopen) = wide.parse_open("novel therapies are needed for a gene", &lem);
    eprintln!(
        "\n=== beam-pressure probe (cell_beam=1024) ===\n  closed×{} open×{}  \"novel therapies are needed for a gene\"",
        wclosed.len(),
        wopen.len()
    );
}

/// Controlled experiment (does contextual SENSE reranking rescue a STRUCTURAL-ambiguity residual?):
/// parse "novel therapies are needed for a gene" at the PAGE beam (64) — the exact config where it is
/// GRAMMAR-GAP cap-only — using whatever reranker `build_index` wires. Built without `--features
/// use-llm` ⇒ cap-only (baseline GRAMMAR-GAP). Built `--features use-llm` with `ANTHROPIC_API_KEY` ⇒ the
/// live `AnthropicSenseRanker` reorders the over-cap words' senses in sentence context. Hypothesis
/// (Declared): no rescue, because the explosion is derivational (Σ-refine × bare-plural shift × PP
/// attachment) over already-≤2 senses, and the cell beam ranks DERIVATIONS, which the sense ranker
/// never touches. Run live:
///     cargo test -p eigenius-wordnet --features use-llm --test db_backed_encoding \
///         llm_reranker_on_structural_residual -- --ignored --nocapture
#[test]
#[ignore = "live-LLM experiment; needs a snapshot and (for the on-arm) --features use-llm + ANTHROPIC_API_KEY"]
fn llm_reranker_on_structural_residual() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head); // wires the live LLM reranker iff --features use-llm + key
    let lem = morphy();
    let sentence = "novel therapies are needed for a gene";
    let t = std::time::Instant::now();
    let (closed, open) = index.parse_open(sentence, &lem);
    let status = if !closed.is_empty() {
        format!("CLOSED×{}", closed.len())
    } else if !open.is_empty() {
        format!("open×{}", open.len())
    } else {
        "GRAMMAR-GAP".to_string()
    };
    eprintln!(
        "\n=== LLM-reranker @ page beam (64): {status} [{:.1}s] {sentence:?} ===",
        t.elapsed().as_secs_f64()
    );
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
    // The page path is overridable (`EIGENIUS_WRN_PAGE`) so the same measurement can run against a
    // controlled-language rewrite (D62 CNL experiment, `first-page-cnl.txt`) for a coverage A/B.
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let page = match std::fs::read_to_string(&page_path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIP: {page_path} not found");
            return;
        }
    };
    eprintln!("measuring page: {page_path}");

    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    // Stage A — the document augmentation (D63 lexicon-augmentation §6a, this session): ground OOV atoms
    // against the form/description text indexes and OVERLAY the groundings onto the index, so the parser
    // SEES them (uncommitted, doc-scoped — the §7-2 in-memory-overlay path) instead of gapping on them.
    // Deterministic proposers here (reproducible A/B); the live LLM abbreviation/POS proposers are
    // drop-in behind the traits (exercised by the `--features use-llm` smoke tests).
    let aug = {
        use eigenius_kernel::dcg::{
            augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
        };
        augment_lexicon_backed(
            &head,
            &page,
            &NoAbbreviationProposer,
            &NominalCategoryProposer,
            &lem,
        )
    };
    eprintln!(
        "augmentation: {} OOV grounded + injected, {} residual OOV",
        aug.added.len(),
        aug.missing_oov.len()
    );
    let index = build_index(&head).with_document_augmentation(&aug);

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

/// PROBE (D63 next-lever diagnosis): is the prep-verb grammar-gap on CNL-v2 caused by the WordNet
/// importer DROPPING the PP-complement (a documented stage-1 loss — `convert.rs::classify` maps the
/// oblique frames 4/13/22 to Intransitive/Transitive with the preposition discarded), or by the
/// preposition simply not attaching? Minimal pairs over common WordNet verbs/nouns disentangle it:
/// prep-verb (V + obligatory PP, expect GAP if the complement is unmodelled); the SAME verb bare (no
/// PP, should parse — the intransitive frame IS emitted); the same verb + a DIFFERENT prep as a
/// VP-adjunct (isolates whether ANY PP attaches); a transitive control (NPs/lexemes known-good).
///
/// Cap-only (the LLM reranker is irrelevant to a grammar/lexicon probe). `#[ignore]`d; run:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_prep_verb_gap -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_prep_verb_gap() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    eprintln!("── token knownness (function words + probe verbs) ──");
    for t in [
        "from",
        "to",
        "in",
        "of",
        "arise",
        "result",
        "respond",
        "contribute",
        "occur",
        "cause",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }

    let probe = |label: &str, s: &str| {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<20}] OOV {unknown:?} :: {s:?}");
            return;
        }
        let (c, o) = index.parse_open(s, &lem);
        let verdict = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{label:<20}] {verdict:<9} :: {s:?}");
    };

    eprintln!("\n── prep-verb complement (V + obligatory PP) ──");
    probe("prep result-from", "diseases result from mutations");
    probe("prep arise-from", "cancers arise from mutations");
    probe("prep respond-to", "cells respond to genes");
    probe("prep contribute-to", "genes contribute to cancers");
    eprintln!("── bare intransitive (same verb, no PP) ──");
    probe("bare result", "diseases result");
    probe("bare arise", "cancers arise");
    probe("bare respond", "cells respond");
    probe("bare contribute", "genes contribute");
    eprintln!("── intransitive + a DIFFERENT prep as VP-adjunct ──");
    probe("adj arise-in", "cancers arise in cells");
    probe("adj occur-in", "cancers occur in cells");
    eprintln!("── transitive control (lexemes/NPs known-good) ──");
    probe("tv cause", "mutations cause cancers");

    // The prep-verb mechanism PARSES (above), so the real blocker is elsewhere. Run the ACTUAL
    // CNL-v2 grammar-gap sentences (which gapped on FULL-UMLS) here on this snapshot: if they PARSE,
    // the FULL-UMLS gap was a lexicon-crowding beam artifact, not a grammar gap; if they GAP here
    // too, bisect one element at a time (subject / compound object / modal / negation / determiner).
    eprintln!("\n── knownness for the actual-gap tokens ──");
    for t in [
        "msi",
        "lynch",
        "syndrome",
        "several",
        "can",
        "do",
        "not",
        "deficient",
        "mismatch",
        "repair",
        "immune",
        "checkpoint",
        "blockade",
        "regions",
        "microsatellites",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }
    eprintln!("\n── actual CNL-v2 gap sentences (gapped on FULL-UMLS) ──");
    probe(
        "gap MSI-result",
        "MSI results from deficient DNA mismatch repair",
    );
    probe("gap MSI-contrib", "MSI contributes to several cancers");
    probe("gap MSI-can-arise", "MSI can arise from Lynch syndrome");
    probe("gap respond-neg", "some cancers do not respond to genes");
    probe("gap copula-plural", "regions are microsatellites");
    eprintln!("── bisect: MSI subject vs plural, simple vs compound object ──");
    probe("bis MSI+simple", "MSI results from mutations");
    probe(
        "bis plural+compound",
        "cancers result from deficient DNA mismatch repair",
    );
    probe("bis MSI+medium", "MSI results from repair");
    eprintln!("── bisect: modal / negation / determiner in isolation ──");
    probe("bis modal", "cancers can arise from mutations");
    probe("bis negation", "cancers do not respond to genes");
    probe("bis determiner", "genes contribute to several cancers");
    eprintln!("── bisect: is `MSI` a usable subject NP at all? ──");
    probe("bis MSI-bare-tv", "MSI causes cancers");
    probe("bis MSI-copula", "MSI is a disease");
    eprintln!(
        "── confirm mechanism: does a DETERMINER rescue the abbreviation? (→ cat_n, not a name) ──"
    );
    probe("det the-MSI-tv", "the MSI causes cancers");
    probe("det the-MSI-cop", "the MSI is a disease");
    probe("wrn-bare-cop", "WRN is a gene");
    probe("wrn-det-cop", "the WRN is a gene");
    eprintln!("── contrast: a DEMO named individual (HeLa) as bare subject, if present ──");
    probe("hela-bare", "HeLa is a gene");
}

/// PROBE (D63 next-lever #2): are the comparative grammar-gaps a genuine construction gap? The
/// CNL-v2 gaps `greater/fewer/stronger … than`, `compared favourably to` all involve comparatives.
/// Isolate the construction over clean bare-plural subjects / known nouns (so a gap is the comparative
/// itself, not the MSI-subject or compound-object confounds already diagnosed). Cap-only; `#[ignore]`d:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_comparatives -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_comparatives() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    eprintln!("── knownness (comparative function words + -er forms) ──");
    for t in [
        "than",
        "more",
        "less",
        "greater",
        "fewer",
        "stronger",
        "larger",
        "large",
        "strong",
        "essential",
        "common",
        "compared",
        "favourably",
        "dependence",
        "phenotype",
        "mutations",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }

    let probe = |label: &str, s: &str| {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<22}] OOV {unknown:?} :: {s:?}");
            return;
        }
        let (c, o) = index.parse_open(s, &lem);
        let verdict = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{label:<22}] {verdict:<9} :: {s:?}");
    };

    eprintln!("\n── baseline: bare predicative adjective (control, should parse) ──");
    probe("base large", "genes are large");
    probe("base essential", "genes are essential");
    eprintln!("── predicative comparative (X is [more] ADJ than Y) ──");
    probe("pred -er than", "genes are larger than cells");
    probe("pred more-adj than", "genes are more essential than cells");
    probe("pred strong-er than", "cells are stronger than genes");
    eprintln!("── attributive comparative adjective (a STRONGER N, no `than`) ──");
    probe("attr stronger-N", "cells require a stronger phenotype");
    probe("attr greater-mass", "cells show greater dependence");
    eprintln!("── comparative quantifier over NPs (fewer/greater N than N) ──");
    probe(
        "quant fewer-than",
        "cells contain fewer mutations than genes",
    );
    probe(
        "quant greater-than",
        "cells show greater dependence than genes",
    );
    eprintln!("── comparative verb (compared [ADV] to) ──");
    probe("vb compared-fav-to", "cancers compared favourably to genes");
    probe("vb compared-to", "genes compared to cells");
}

/// PROBE (D63): Derive the CAUSE of the remaining CNL-v2 grammar-gaps (sentences not already pinned to
/// the MSI-subject / `than NP` levers). Minimal pairs over clean known vocab isolate each hypothesized
/// construction; the load-bearing one is (G) — whether a domain abbreviation as an attributive
/// MODIFIER (`MSI cells`, `WRN dependency`) also fails, which would widen the abbreviation lever.
/// Cap-only; `#[ignore]`d:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_gap_tail -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_gap_tail() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    eprintln!("── knownness ──");
    for t in [
        "msi",
        "wrn",
        "mmr",
        "dependency",
        "inactivation",
        "somatic",
        "independent",
        "target",
        "targets",
        "region",
        "regions",
        "process",
        "state",
        "lineages",
        "checkpoint",
        "blockade",
        "evaluated",
        "identified",
        "analysed",
        "queried",
        "arises",
        "as",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }

    let probe = |label: &str, s: &str| {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<24}] OOV {unknown:?} :: {s:?}");
            return;
        }
        let (c, o) = index.parse_open(s, &lem);
        let verdict = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{label:<24}] {verdict:<9} :: {s:?}");
    };

    eprintln!("\n── G. abbreviation as attributive MODIFIER (sents 3/14/16/17/19) ──");
    probe("G MSI-mod-plural", "MSI cells contain genes");
    probe("G WRN-mod-subject", "WRN genes cause cancers");
    probe("G MMR-mod-subject", "MMR mutations cause cancers");
    probe("G control N-N", "cancer cells contain genes");
    eprintln!("── A. `as`-predicative (X V Y as Z) (sents 14/15) ──");
    probe("A evaluated-as", "cells evaluated genes as targets");
    probe("A identified-as", "cells identified genes as targets");
    eprintln!("── B. plural copula predicate-nominal (sent 4) ──");
    probe("B plural-predn", "regions are genes");
    probe("B control sg-predn", "a region is a gene");
    eprintln!("── C. PP-stack in object (X V Y in Z with W) (sents 1/13) ──");
    probe("C pp-stack", "cells query genes in cancers with mutations");
    probe("C control 1pp", "cells query genes in cancers");
    eprintln!("── D. numeral + adjective + N-N compound (sent 12) ──");
    probe("D bare", "cells analysed targets");
    probe("D N-N compound", "cells analysed cancer dependency targets");
    probe("D numeral+adj", "cells analysed two independent targets");
    eprintln!("── E. compound-noun prep object (sent 11) ──");
    probe(
        "E compound-obj",
        "cancers respond to immune checkpoint blockade",
    );
    eprintln!("── F. modal + or-coordination of objects (sent 19) ──");
    probe("F modal-or", "genes may require cells or mutations");
    eprintln!("── H. adjective-modified subject + prep-verb (sents 9/3) ──");
    probe(
        "H adj-subj-prepverb",
        "somatic inactivation arises from mutations",
    );
    probe(
        "H that-essential-in",
        "cells found that genes were essential in cells",
    );
}

/// PROBE (D63): are the residual CNL-v2 grammar-gaps (sentences whose constituent constructions all
/// PARSE in isolation) genuine grammar gaps, or full-UMLS beam/lexicon-crowding artifacts? Run the
/// actual sentences VERBATIM on the SUBSET (fewer senses) at the default beam (64, widen→512) and at a
/// wide fixed beam (2048, above the widen ceiling), and compare to their known FULL-UMLS GAP:
/// parses on subset@64 → the full-UMLS gap was LEXICON-CROWDING (extra senses), not grammar; gaps@64
/// but parses@2048 → BEAM-CEILING (the 512 widen cap is too low); gaps at both → a GENUINE grammar gap.
///
/// Cap-only; `#[ignore]`d:
///   EIGENIUS_DB_SNAPSHOT=<subset-snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_beam_crowding -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_beam_crowding() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let def = build_index(&head); // CELL_BEAM=64, widen→512
    let wide = LexicalIndex::build(Arc::clone(&head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(2048); // above CELL_BEAM_WIDEN_MAX → a fixed wide beam

    let verdict = |idx: &LexicalIndex, s: &str| {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };

    for (label, s) in [
        (
            "sent3 found-that",
            "We found that WRN was selectively essential in MSI models",
        ),
        (
            "sent12 two-indep",
            "We analysed two independent cancer dependency data sets",
        ),
        (
            "sent19 may-require",
            "WRN dependency may require specific lineages or a stronger mutation phenotype",
        ),
    ] {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !def.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<20}] OOV {unknown:?} (can't test on subset) :: {s:?}");
            continue;
        }
        eprintln!(
            "  [{label:<20}] subset@64→512={:<10} subset@2048={:<10} (full-UMLS: GAP)",
            verdict(&def, s),
            verdict(&wide, s),
        );
    }
}

/// PHASE 1 MEASUREMENT (D63 `d63-document-preprocessing-scope.md`): run the deterministic Stage-A
/// pipeline against the served snapshot and measure the recovery. Extract `Long Form (SHORT)`
/// definitions from the ORIGINAL page (which carries `microsatellite instability (MSI)` — the CNL-v2
/// rewrite dropped it), ground each long form to its concept, emit the doc-glossary resources, PERSIST
/// them as a chained layer on the SAME backend (so the value index populates and the index resolves
/// lazily — an in-memory overlay OOMs via the eager full-chain scan, §7-2), then compare base vs
/// glossary on the MSI-subject sentences that gapped in the diagnosis. Run:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       measure_abbreviation_glossary -- --ignored --nocapture
#[test]
#[ignore = "DB-backed Phase-1 measurement; run with --ignored --nocapture"]
fn measure_abbreviation_glossary() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    // Open the store keeping the BACKEND (to persist the doc-glossary layer onto it).
    let store = Arc::new(RocksStore::open(&path).expect("open RocksStore snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    let ctx = match bootstrap_persistent(Arc::clone(&backend)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: cannot resume the snapshot — {e:?}");
            return;
        }
    };
    let head = Arc::clone(ctx.head());
    let lem = morphy();

    // The ORIGINAL page carries the `Long Form (ABBR)` definitions. `EIGENIUS_WRN_PAGE` overrides.
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let page = std::fs::read_to_string(&page_path).unwrap_or_default();

    // Stage A: extract → ground (ranked cross-check + fuller candidate) → emit (fresh class on miss).
    let defs = extract_abbreviations(&page);
    eprintln!("extracted {} abbreviation definition(s):", defs.len());
    for d in &defs {
        match ground_abbreviation(&head, &d.short_form, &d.long_form, &d.context) {
            Some(c) => eprintln!(
                "  {:<8} ← {:<32?} → {}",
                d.short_form,
                d.long_form,
                c.as_str()
            ),
            None => eprintln!(
                "  {:<8} ← {:<32?} → (miss → fresh doc-local class)",
                d.short_form, d.long_form
            ),
        }
    }
    let resources = glossary_resources(&head, &defs);

    // Build + persist the doc-glossary layer on the SAME backend.
    let mut b = LayerBuilder::new("doc-glossary", Some(Arc::clone(&head)));
    for r in resources {
        b.add_resource(r).expect("add glossary resource");
    }
    let doc_layer = Arc::new(b.build(LayerStorage::with_persistent(Arc::clone(&backend))));
    backend
        .store_layer(&doc_layer)
        .expect("persist doc-glossary layer");
    eprintln!(
        "\ndoc-glossary layer persisted ({} definition(s))\n",
        defs.len()
    );

    let base = build_index(&head);
    let glossary = build_index(&doc_layer);
    let verdict = |idx: &LexicalIndex, s: &str| {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };

    let sentences = [
        // MSI — "microsatellite instability", head noun `instability` is mass.
        "MSI is a disease",
        "MSI causes cancers",
        "MSI contributes to several cancers",
        "MSI can arise from Lynch syndrome",
        // MMR — "DNA mismatch repair", head noun `repair` is mass.
        "MMR is deficient in cancers",
        "MMR contributes to cancers",
    ];
    // Post-reshape: a mass-phenomenon abbreviation grounds to a CLASS → the alias emits `cat_n(C, mass)`,
    // and a bare subject shifts to the CLOSED kind-predication `kind_of(C)` (no named individual, no
    // deferred hole). So recovery should be GAP → CLOSED, not GAP → OPEN.
    let (mut recovered, mut closed) = (0usize, 0usize);
    eprintln!(
        "── base (bare MSI/MMR = raw UMLS cat_n count noun → no bare-subject shift) vs glossary \
         (mass alias → kind_of, closes via the kind shift) ──"
    );
    for s in sentences {
        let (bv, gv) = (verdict(&base, s), verdict(&glossary, s));
        let flag = if bv == "GAP" && gv.starts_with("CLOSED") {
            recovered += 1;
            closed += 1;
            "  ← RECOVERED (closed)"
        } else if bv == "GAP" && gv != "GAP" {
            recovered += 1;
            "  ← RECOVERED (open)"
        } else {
            ""
        };
        eprintln!("  base={bv:<10} glossary={gv:<10} :: {s:?}{flag}");
    }
    // Witness a recovered sem — a CLOSED kind-predication `kind_of(<CUI>)`, not a reified individual.
    if let Some(p) = glossary
        .parse("MSI contributes to several cancers", &lem)
        .first()
    {
        eprintln!(
            "\n  sem(\"MSI contributes to several cancers\") = {}",
            pretty_term(p.sem())
        );
    }
    eprintln!(
        "\nrecovered {recovered}/{} abbreviation sentences ({closed} as CLOSED kind-predications) via \
         the glossary",
        sentences.len()
    );
}
