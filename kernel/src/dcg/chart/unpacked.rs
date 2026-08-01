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
//! **The unpacked chart driver** — the flat, beamed, item-level CKY. Every cell holds a `Vec<Item>`, and
//! every rule is tried on every item PAIR.
//!
//! It is not the production path (the router sends most sentences to the packed forest), and it is kept
//! for three reasons:
//! 1. **The differential oracle.** Packed ≡ unpacked is the property that licenses packing at all, and
//!    it is only testable because an independent implementation exists to compare against
//!    (`packed_forest_equals_unpacked_on_core_grammar`).
//! 2. **Pied-piping.** A quaternary rule the packed forest has no edge shape for; the router
//!    (`parse_needs_unpacked`) diverts those sentences here, and the rule is inline below.
//! 3. **The combinatory-core spike** (`with_combinatory_core`), which adds the extra CCG combinators.
//!
//! Its growth is bounded by a per-cell BEAM rather than by packing — hence the widen-on-failure ladder
//! escalates the beam here (and only the sense cap on the packed path).

use super::super::category::{is_ctor, is_sentence_premod, is_vp_adjunct_prep, slash_parts};
use super::super::grammar::Grammar;
use super::super::item::Item;
use super::super::pretty::pretty_term;
use super::super::reserved::ReservedKind;
use super::super::rules::combinators::{apply, apply_core};
use super::super::rules::constructions::pied_pipe;
use super::super::rules::registry::unary_shifts;
use super::{beam_cell, cell_histogram};

impl Grammar {
    /// Run the item-level CKY over a seeded `chart`, in place. Returns the number of items the per-cell
    /// beam dropped (0 when no beam is set). `combinatory_core` adds the extra CCG combinators.
    pub(crate) fn drive_unpacked(
        &self,
        chart: &mut [Vec<Vec<Item>>],
        tokens: &[String],
        beam: Option<usize>,
        combinatory_core: bool,
        prefer_multiword: bool,
        debug: bool,
    ) -> usize {
        let n = tokens.len();
        let mut beam_drops = 0usize;
        // Multiword span integrity (base cap only; mirrors the packed driver): protect the interior
        // split points of every multiword `cat_n` leaf so no compositional constituent splits it.
        // Widen-on-failure clears `prefer_multiword`, re-admitting the splits, so `grammar-gap 0` holds.
        // Shared marking (single source of truth with the packed driver and the tracer).
        let protected_split = super::multiword_protected_splits(chart, prefer_multiword);
        // 2. CKY composition, appending combined items to each cell's seeds (so a multiword leaf and a
        //    compositional derivation of the same span both remain available, EXCEPT where span
        //    integrity forbids splitting a multiword).
        for len in 2..=n {
            for i in 0..=(n - len) {
                let j = i + len - 1;
                let mut produced = Vec::new();
                // The cell's right context — constant for every item in it, so packing stays sound.
                let rctx = super::super::rules::RightContext::after(&self.reserved, tokens, j);
                for k in i..j {
                    if protected_split.get(k).copied().unwrap_or(false) {
                        continue;
                    }
                    let lefts = &chart[i][k];
                    let rights = &chart[k + 1][j];
                    for l in lefts {
                        for r in rights {
                            if let Some(item) = apply(l, r, &self.layer, rctx) {
                                produced.push(item);
                            }
                            // Combinatory-core spike: the extra CCG combinators (crossed + backward
                            // composition), applied alongside the hand-built rules when enabled.
                            if combinatory_core {
                                produced.extend(apply_core(l, r, &self.layer, rctx));
                            }
                        }
                    }
                }
                // Token-keyed sem-reading binary rules: coordination, close apposition, `but not`,
                // the reciprocal, and the restrictive / non-restrictive relatives. WHERE each fires
                // comes from the shared registry ([`Self::binary_sites`]) — the same list the packed
                // forest turns into `Binary` edges — and each pair is built by the shared
                // [`Self::apply_bin_rule`], so the two chart paths cannot drift apart.
                for site in self.binary_sites(tokens, i, j) {
                    let lefts = &chart[site.left.0][site.left.1];
                    let rights = &chart[site.right.0][site.right.1];
                    for l in lefts {
                        for r in rights {
                            if let Some(item) = self.apply_bin_rule(site.rule, l, r) {
                                produced.push(item);
                            }
                        }
                    }
                }
                // Pied-piping restrictive relative (D62 §2 #2B): `[noun] [prep] which [subj] [VP]` →
                // refine the noun with the clause + the FRONTED preposition relating the antecedent to
                // the clause subject (`Σg:C. And(VP(subj), prep(subj,g))`). Reuses the VP-adjunct prep
                // sem (no PP-gap extraction). The clause after `prep which` is decomposed into its
                // subject NP + `S\NP` VP at every split, so it handles the ordinary subject-predicate
                // clause; `which` here is the fronted prep's object, distinct from the bare relativizer.
                for p in (i + 1)..j {
                    if !tokens
                        .get(p + 1)
                        .is_some_and(|t| self.reserved.is(t, ReservedKind::WhRelativizer))
                    {
                        continue;
                    }
                    if p < i + 1 || p + 2 > j {
                        continue;
                    }
                    // The fronted preposition is an ordinary TOKEN, so its items are already seeded in
                    // cell `[p, p]` — read them from there. This rule used to fetch them straight out of
                    // the lexicon (`entries_for(tokens[p])`), which quietly bypassed everything the chart
                    // had already done to them: the lexicon SCOPE filter (D65 §4 — an out-of-scope
                    // preposition was admitted anyway, witnessed by `pied_piping_respects_the_lexicon_scope`),
                    // the sense cap, the contextual reranker, the cross-POS prune, and the cell beam. It
                    // also dropped the preposition's `Cost` from the result (witnessed by
                    // `pied_piping_counts_the_prepositions_cost`), zeroing its `lexicon_order` — the
                    // PRIMARY rank key — for that word. A chart item has been through all of it.
                    let preps: Vec<&Item> = chart[p][p]
                        .iter()
                        .filter(|it| is_vp_adjunct_prep(it.cat()))
                        .collect();
                    if preps.is_empty() {
                        continue;
                    }
                    for k in (p + 2)..j {
                        for noun in &chart[i][p - 1] {
                            if is_ctor(noun.cat(), "cat_n").is_none() {
                                continue;
                            }
                            for subj in &chart[p + 2][k] {
                                if is_ctor(subj.cat(), "cat_np").is_none() {
                                    continue;
                                }
                                for vp in &chart[k + 1][j] {
                                    // VP must be `S\NP` (a clause missing its subject).
                                    if !matches!(slash_parts(vp.cat(), "bwd"),
                                        Some((_m, s, _)) if is_ctor(s, "cat_s").is_some())
                                    {
                                        continue;
                                    }
                                    for prep in &preps {
                                        if let Some((cat, sem)) =
                                            pied_pipe(noun.cat(), prep.sem(), subj.sem(), vp.sem())
                                        {
                                            // Sum EVERY operand's cost, the preposition included — as
                                            // every other rule does.
                                            produced.push(Item::with_cost(
                                                cat,
                                                sem,
                                                noun.cost()
                                                    .saturating_add(prep.cost())
                                                    .saturating_add(subj.cost())
                                                    .saturating_add(vp.cost()),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let produced_n = produced.len();
                chart[i][j].extend(produced);
                // Composed-cell UNARY shifts (Phase 2d): the shared `unary_shifts()` table, iterated IN
                // ORDER — coordination list-completion, bare-nominal shift, type-raise (over the updated
                // cell, so it sees the shifted NPs), fronted participial. Each shift extends the cell
                // before the next reads it, so the ordering is load-bearing exactly as in the packed
                // forest's edge-creation loop (single source of truth: `registry::unary_shifts`). Every
                // shift is per-item-independent, so the flat-map equals the former whole-cell calls.
                for shift in unary_shifts() {
                    let produced: Vec<Item> = chart[i][j]
                        .iter()
                        .flat_map(|it| shift.run(self, it, (i, j), rctx))
                        .collect();
                    chart[i][j].extend(produced);
                }
                // Fronted-modifier comma absorption (D62 §2 #5): a SENTENCE-INITIAL `S/S` modifier
                // (`Thus,` / `More commonly,` / later a fronted participial) absorbs a trailing comma
                // so it can then forward-apply to the matrix clause. The comma is otherwise a reserved
                // coordinator with no chart item, leaving a gap the modifier can't bridge. Restricted to
                // `i == 0` (sentence-initial) to avoid competing with list-coordination commas.
                if i == 0 && len >= 2 && self.reserved.is_comma(&tokens[j]) {
                    let absorbed: Vec<Item> = chart[i][j - 1]
                        .iter()
                        .filter(|it| is_sentence_premod(it.cat()))
                        .cloned()
                        .collect();
                    chart[i][j].extend(absorbed);
                }
                // Lever B: beam this composed cell (non-top; the top cell `len == n` is left to the
                // forest cap). Done after type-raise so the raised items compete in the beam too.
                if len < n {
                    if let Some(b) = beam {
                        beam_drops += beam_cell(&mut chart[i][j], b);
                    }
                }
                if debug {
                    eprintln!(
                        "  [parse-debug] cell[{i}..{j}] len={len} produced={produced_n} kept={} | {}",
                        chart[i][j].len(),
                        cell_histogram(&chart[i][j])
                    );
                }
                // Targeted dump (set `EIGENIUS_DUMP_CELL=i..j`): print the FULL category (indices
                // intact) + provenance of a sample of this cell's items, to see exactly which
                // sense/derivation combinations accumulate.
                if let Ok(want) = std::env::var("EIGENIUS_DUMP_CELL") {
                    if want == format!("{i}..{j}") {
                        eprintln!(
                            "  ===== DUMP cell[{i}..{j}] ({} items, sample 20) =====",
                            chart[i][j].len()
                        );
                        for it in chart[i][j].iter().take(20) {
                            eprintln!(
                                "    [{:?} cost={:?}] {}",
                                it.prov(),
                                it.cost(),
                                pretty_term(it.cat())
                            );
                        }
                    }
                }
            }
        }
        beam_drops
    }
}
