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
//! **The two parse paths** — the routed alternatives the widen ladder actually calls.
//!
//! Each is the same three-step orchestration, and each is `impl Parser` because each needs all three
//! stages: **seed** the leaf cells (needs the lexicon), **drive** the chart (needs only the grammar —
//! `super::super::chart`), and **gate** the full-span candidates through the kernel (`super::felicity`).
//!
//! They live here, with the bridge, rather than with the chart drivers they call. That is the split the
//! compiler insisted on: a driver is a pure grammar operation and can be handed to anyone; ORCHESTRATING
//! one means touching the lexicon and the felicity gate, which is the bridge's job, not the chart's.

use super::super::segment::tokenize;
use super::*;

/// Collapse candidates carrying the **same sem term**, keeping the lowest-cost derivation of each.
///
/// Call on a cost-sorted list, immediately before the [`CLASSIFY_BUDGET`] truncation: `retain` keeps
/// the first occurrence, which after the sort is the cheapest. Syntactic (pre-β) identity only —
/// derivations that differ merely in reduction order are left for `subsume_duplicates` to collapse
/// on definitional equality after the felicity gate. That is the point: this pass is cheap (one
/// `Debug` render per candidate) and runs BEFORE the budget is spent, so a reading that is genuinely
/// distinct is not evicted by n copies of a reading already in the window. See [`CLASSIFY_BUDGET`]
/// for the witnessed case (376 candidates → 44 sems, correct readings truncated away).
fn retain_distinct_sems<T>(candidates: &mut Vec<T>, sem: impl Fn(&T) -> &Exp) {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    candidates.retain(|c| seen.insert(format!("{:?}", sem(c))));
}

/// Spread the felicity budget over distinct BRACKETINGS — see [`skeleton::spread_over_keys`] for
/// why a flat cost prefix is systematically biased against the correct (deeply-nested) readings.
fn spread_over_skeletons<T>(candidates: Vec<T>, sem: impl Fn(&T) -> &Exp) -> Vec<T> {
    crate::dcg::skeleton::spread_over_keys(candidates, |c| {
        crate::dcg::skeleton::skeleton_of(sem(c))
    })
}

impl Parser {
    /// Packed-forest parse (D63 Option A, blueprint §11 3d): the shared attempt policy
    /// ([`Self::parse_widening`]) over the packed forest + cube-pruning extractor
    /// ([`Self::parse_packed_at_cap`]), with **no beam rung** — packing bounds the chart by cube
    /// pruning, so only the sense cap can drop a needed sense and only the cap escalates. Reached only
    /// for index-independent, construct-free sentences (the router's guard), so it is equivalent to
    /// [`Self::parse_unpacked`] on those (the differential oracle, 3f).
    pub(super) fn parse_packed(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        self.parse_widening(text, lemmatizer, scope, None, |cap, _beam, ranks| {
            self.parse_packed_at_cap(text, lemmatizer, scope, cap, ranks)
        })
    }

    /// One packed-forest parse at a fixed sense `cap` (the widen-loop body of [`Self::parse_packed`]):
    /// build the shared forest ([`Self::build_forest`]), extract the top-span k-best via cube pruning
    /// ([`Self::kbest`]), and apply the felicity pop-filter ([`Self::classify_felicitous`]) — routing
    /// each survivor to the closed or open forest, exactly as [`Self::parse_at_cap`] does.
    fn parse_packed_at_cap(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return (Vec::new(), Vec::new());
        }
        // Seeding is the PARSER's job (it needs the lexicon); driving the chart is the GRAMMAR's (it
        // does not). Hoisting the seed out of `build_forest` is what makes the packed driver
        // lexicon-free — it is handed the leaf cells and never asks where they came from.
        let (leaves, _drops) = self.seed_leaves(&tokens, lemmatizer, scope, cap, ranks, None);
        // Multiword preference (prefer a span-covering lexicalized multiword over its compositional
        // kind-compound). Kept through the ENTIRE cap-widen ladder, lifted only at the FINAL rung
        // (cap == SENSE_CAP_WIDEN_MAX): a sentence that widens for unrelated (sense-crowding) reasons
        // keeps its compounds collapsed instead of re-exploding, and coverage still falls back to the
        // compositional splits at the last rung, so `grammar-gap 0` holds. Safe to keep on through
        // widen because the overlap boundary-exception (`multiword_protected_splits`) stops an
        // overlapping multiword from gapping — otherwise it would stay gapping to the max rung and
        // explode there (the ×88 regression this cut had before the root fix).
        let prefer_multiword = !matches!(cap, Some(c) if c >= SENSE_CAP_WIDEN_MAX);
        let forest = self
            .grammar
            .build_forest(&leaves, &tokens, prefer_multiword);
        let mut memo: Vec<Option<Vec<Item>>> = vec![None; forest.nodes.len()];

        // Top-span candidates: finite-clause / wh-question nodes spanning the whole sentence.
        let top: Vec<super::super::chart::forest::NodeId> = forest.cells[0][n - 1]
            .values()
            .copied()
            .filter(|&id| {
                let c = forest.nodes[id].rep.cat();
                is_finite_clause(c) || is_ctor(c, "cat_q").is_some()
            })
            .collect();

        // Forest derivation trace (set `EIGENIUS_TRACE_FOREST`, see `chart::trace`): print HOW the
        // forest was built — the hyperedge tree, which cells combine under which rule at which split.
        // Fires once per cap attempt; the header carries the `protected_split` vector so a base-cap
        // run (integrity on) and a widened one (integrity off) can be diffed to name the edge
        // multiword span-integrity removed.
        if let Ok(spec) = std::env::var("EIGENIUS_TRACE_FOREST") {
            let mw_spans = super::super::chart::multiword_spans(&leaves);
            let protected =
                super::super::chart::multiword_protected_splits(&leaves, prefer_multiword);
            let header = format!(
                "===== FOREST TRACE cap={cap:?} prefer_multiword={prefer_multiword} \
                 multiword_spans={mw_spans:?} protected_split={protected:?} nodes={} tokens={tokens:?} =====",
                forest.nodes.len(),
            );
            eprint!(
                "{}",
                super::super::chart::trace::forest_trace(&forest, &tokens, &top, &spec, &header)
            );
        }

        let mut candidates: Vec<Item> = Vec::new();
        for id in top.iter().copied() {
            candidates.extend(
                self.grammar
                    .kbest(&forest, id, DEFAULT_FOREST_CAP, &mut memo),
            );
        }
        candidates.sort_by_key(|it| it.cost());
        let raw_candidates = candidates.len();
        retain_distinct_sems(&mut candidates, |it| it.sem());
        let distinct_sems = candidates.len();
        let mut candidates = spread_over_skeletons(candidates, |it| it.sem());
        let skels_available = candidates
            .iter()
            .map(|it| crate::dcg::skeleton::skeleton_of(it.sem()))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        candidates.truncate(CLASSIFY_BUDGET);
        if distinct_sems > candidates.len() {
            // Never silent: a truncation here drops readings the grammar DID derive, and the unit
            // still parses, so no coverage metric can see it (§5b false-comfort coverage). After
            // `spread_over_skeletons` what is dropped is depth WITHIN a bracketing, not a bracketing
            // — every structure is represented before any structure gets a second reading.
            eprintln!(
                "dcg::parse (packed): CLASSIFY_BUDGET dropped {} distinct reading(s) of {distinct_sems} for {text:?}",
                distinct_sems - candidates.len(),
            );
        }
        if std::env::var("EIGENIUS_PARSE_DEBUG").is_ok() {
            eprintln!(
                "dcg::parse (packed): cap={:?} {:?} forest nodes={} finite candidates={} \
                 (raw={raw_candidates} distinct-sem={distinct_sems} distinct-skel={skels_available})",
                cap,
                text,
                forest.nodes.len(),
                candidates.len()
            );
        }

        // Hole context for classification — identical to the unpacked path. Only the referent
        // (`EntityRef`, pronoun/possessor → D64) hole remains; the bare-plural/mass quantification hole
        // was retired with the kind-predication reshape (Phase B).
        let entity_ty = Exp::EigonClass(iri(ENTITY_IRI));
        let types_ok = eval(&entity_ty, &Rho::Nil).is_ok();
        let mut hole_specs: Vec<(String, Exp, HoleKind)> = Vec::new();
        if types_ok {
            for i in 0..n {
                for j in i..n {
                    hole_specs.push((hole_base(i, j), entity_ty.clone(), HoleKind::EntityRef));
                }
            }
        }

        // Felicity pop-filter → closed / open forests (the only type-check, at the top span).
        let mut forest_out: Vec<Item> = Vec::new();
        let mut open: Vec<OpenParse> = Vec::new();
        for it in &candidates {
            if types_ok {
                match self.classify_felicitous(it, &hole_specs) {
                    Some(FelicitousOutcome::Closed(c)) => forest_out.push(c),
                    Some(FelicitousOutcome::Open(o)) => open.push(o),
                    None => {}
                }
            } else if let Some(c) = self.reduced_felicitous(it) {
                forest_out.push(c);
            }
        }
        Self::subsume_duplicates(&mut forest_out); // D3: collapse definitionally-equal readings
        forest_out.sort_by_key(|it| it.cost());
        forest_out.truncate(DEFAULT_FOREST_CAP);

        // Ambiguity attribution (`chart::attribute`): which span, which rule, which senses drive this
        // unit's reading count. Runs HERE — after the felicity filter and dedup — so a sense site can
        // be intersected against the readings that actually SURVIVED; attributing the raw forest
        // instead over-counts (~60x) and ranks nothing. `EIGENIUS_TRACE_ATTRIBUTION` prints the
        // per-unit block; the `dcg::attribution` roll-up (armed by the sweep) records it for the
        // cross-unit aggregate. Read-only: the parse result is already final above.
        let want_render = std::env::var("EIGENIUS_TRACE_ATTRIBUTION").is_ok();
        let want_record = super::super::attribution::is_enabled();
        if !top.is_empty() && (want_render || want_record) {
            let attr = forest.attribute(&tokens, &top, &forest_out, &self.grammar.layer);
            if want_render {
                if let Some(report) = attr.render(&tokens.join(" ")) {
                    eprint!("{report}");
                }
            }
            if want_record {
                super::super::attribution::record(&tokens, &attr);
            }
        }

        (forest_out, open)
    }
}

impl Parser {
    /// Unpacked (flat, beamed) parse — the differential-oracle baseline, and the path the router takes
    /// for the combinatory-core spike and pied-piping. The shared attempt policy
    /// ([`Self::parse_widening`]) over the item-level CKY ([`Self::parse_at_cap`]), with
    /// [`Self::cell_beam`] as the escalation ladder's beam rung (the packed path passes `None` there).
    pub(super) fn parse_unpacked(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        self.parse_widening(
            text,
            lemmatizer,
            scope,
            self.config.cell_beam,
            |cap, beam, ranks| self.parse_at_cap(text, lemmatizer, scope, cap, ranks, beam),
        )
    }

    fn parse_at_cap(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
        beam: Option<usize>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return (Vec::new(), Vec::new());
        }
        // Parse-failure instrumentation (set `EIGENIUS_PARSE_DEBUG=1`): per-cell stats, flushed, so
        // the last line before an OOM/SIGKILL localizes the blow-up cell + cap level.
        let debug = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();

        // 1. Seed the leaf cells (shared with the packed path, §11 3c.3).
        let (mut chart, mut beam_drops) =
            self.seed_leaves(&tokens, lemmatizer, scope, cap, ranks, beam);

        // 2. Drive the chart. Seeding needed the lexicon; DRIVING does not — the grammar is handed the
        //    seeded cells and composes them, so the two CKY drivers are pure grammar operations.
        // Multiword-preference cut kept through the ENTIRE widen ladder (beam-first, then cap), lifted
        // only at the FINAL rung (both maxed) so a sentence that widens for unrelated reasons keeps its
        // compounds collapsed; the last rung re-admits the compositional kind-compound, keeping
        // `grammar-gap 0`. Mirrors the packed path above so the differential oracle holds.
        let prefer_multiword = !(matches!(cap, Some(c) if c >= SENSE_CAP_WIDEN_MAX)
            && matches!(beam, Some(b) if b >= CELL_BEAM_WIDEN_MAX));
        beam_drops += self.grammar.drive_unpacked(
            &mut chart,
            &tokens,
            beam,
            self.config.combinatory_core,
            prefer_multiword,
            debug,
        );
        if beam_drops > 0 {
            eprintln!(
                "dcg::parse: cell-beam (Lever B) dropped {beam_drops} items (beam={})",
                beam.unwrap_or(0),
            );
        }

        // 3. The forest: full-span `S` items whose assembled sem — once **NbE-
        //    reduced** (the determiner lambdas β-apply away to a normal form) — the
        //    kernel confirms inhabits `Prop`. Reducing first is essential: a
        //    composed determiner sentence is a redex-heavy `App(λ…, …)` tree, and
        //    `check_infer` cannot synthesize a bare lambda's type.
        // The hole context for classification (D64 carrier): for every span `[i,j]`, a referent hole
        // (`Entity`/`EntityRef`, a pronoun/possessor). The bare-plural/mass quantification hole was
        // retired with the kind-predication reshape (Phase B — bare plural/mass now commit to
        // `kind_of(t)`, `Parser::kind_raised_nps`). A candidate mentions only the hole vars it
        // actually carries; `classify_felicitous` filters to those.
        let entity_ty = Exp::EigonClass(iri(ENTITY_IRI));
        // Degenerate guard (preserved): if the hole type can't even be evaluated, fall back to the
        // closed-only path. Normally it evals fine.
        let types_ok = eval(&entity_ty, &Rho::Nil).is_ok();
        let mut hole_specs: Vec<(String, Exp, HoleKind)> = Vec::new();
        if types_ok {
            for i in 0..n {
                for j in i..n {
                    hole_specs.push((hole_base(i, j), entity_ty.clone(), HoleKind::EntityRef));
                }
            }
        }

        // Full-span candidates: a **finite** declarative/polar `S` (denotes `Prop`) or a
        // wh-question `Q(T)` (denotes `T → Prop`, D63 §8.5). The finiteness gate rejects a bare
        // base/infinitival clause as a standalone root; partial functors are dropped.
        // FAIL-CLOSED OOM GUARD: cost-sort and keep only the lowest-cost [`CLASSIFY_BUDGET`] BEFORE
        // the felicity loop — the top cell is unbeamed and can hold thousands of candidates over the
        // full lexicon, and each felicity check NbE-evals an impredicative-∃ GQ sem, so classifying
        // all of them OOMs. (Normal forests have far fewer candidates → no-op.)
        let mut candidates: Vec<&Item> = chart[0][n - 1]
            .iter()
            .filter(|it| {
                // Complete results: a **finite** declarative/polar `S` (denotes `Prop`) or a
                // wh-question `Q(T)` (denotes `T → Prop`, D63 §8.5). The finiteness gate rejects a
                // bare base/infinitival clause (`S[_,bse]`) as a standalone root; partial functors
                // are dropped. NOTE: the sem shape cannot discriminate here — a well-formed
                // determiner-subject clause is an unreduced `App` redex (subject-GQ applied to the
                // VP), structurally identical to a pathological reading; only β-reduction in the
                // felicity gate below tells them apart.
                is_finite_clause(it.cat()) || is_ctor(it.cat(), "cat_q").is_some()
            })
            .collect();
        let n_candidates = candidates.len();
        candidates.sort_by_key(|it| it.cost());
        retain_distinct_sems(&mut candidates, |it| it.sem());
        let distinct_sems = candidates.len();
        let mut candidates = spread_over_skeletons(candidates, |it| it.sem());
        candidates.truncate(CLASSIFY_BUDGET);
        if distinct_sems > candidates.len() {
            eprintln!(
                "dcg::parse: CLASSIFY_BUDGET dropped {} distinct reading(s) of {distinct_sems} for {text:?}",
                distinct_sems - candidates.len(),
            );
        }
        if debug {
            eprintln!(
                "  [parse-debug cap={cap:?}] full-span candidates {n_candidates} → {distinct_sems} \
                 distinct sem(s) → felicity-checking {} (CLASSIFY_BUDGET)",
                candidates.len()
            );
        }

        // Split into the CLOSED forest (felicitous closed `Prop`) and the OPEN forest (felicitous
        // but carrying unresolved referent holes — D64).
        let mut forest: Vec<Item> = Vec::new();
        let mut open: Vec<OpenParse> = Vec::new();
        for (k, it) in candidates.into_iter().enumerate() {
            if debug {
                eprintln!(
                    "  [parse-debug cap={cap:?}] classify candidate {k}/{n_candidates}\n      cat={}\n      sem={}",
                    super::super::pretty_term(it.cat()),
                    super::super::pretty_term(it.sem())
                );
            }
            if types_ok {
                match self.classify_felicitous(it, &hole_specs) {
                    Some(FelicitousOutcome::Closed(c)) => forest.push(c),
                    Some(FelicitousOutcome::Open(o)) => open.push(o),
                    None => {}
                }
            } else if let Some(c) = self.reduced_felicitous(it) {
                // Hole types unavailable (should not happen): closed path only.
                forest.push(c);
            }
        }
        Self::subsume_duplicates(&mut forest); // D3: collapse definitionally-equal readings

        // RANK + CAP (D63 §8.7 Stage B): order each forest by ascending cost — the sum
        // of the parse's leaf `sense_rank`s — so the most-frequent-sense readings come
        // first, then cap to [`DEFAULT_FOREST_CAP`]. WordNet sense-polysemy yields
        // 100s–1000s of well-typed parses for a short sentence (the felicity gate prunes
        // none of it), so an unbounded forest is unusable; the cap bounds it without
        // silent loss — the dropped tail is logged. Stable sort + cost 0 everywhere
        // (closed-class / demo entries) ⇒ no ranking or cap effect there (order
        // preserved, sizes well under the cap), so exact-count tests are unaffected.
        forest.sort_by_key(|it| it.cost());
        if forest.len() > DEFAULT_FOREST_CAP {
            let dropped = forest.len() - DEFAULT_FOREST_CAP;
            eprintln!(
                "dcg::parse: ranked forest capped {} → {DEFAULT_FOREST_CAP} \
                 (dropped {dropped} higher-cost / rarer-sense parses)",
                forest.len(),
            );
            forest.truncate(DEFAULT_FOREST_CAP);
        }
        open.sort_by_key(|o| o.item.cost());
        if open.len() > DEFAULT_FOREST_CAP {
            open.truncate(DEFAULT_FOREST_CAP);
        }
        (forest, open)
    }
}
