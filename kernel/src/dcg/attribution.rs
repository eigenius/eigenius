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

//! **Page-level ambiguity roll-up** — aggregate the per-unit [`crate::dcg::chart::attribute`] sites
//! across a whole sweep into ranked levers: which SURFACE FORM drives the most sense multiplicity, and
//! which named CONSTRUCTION drives the most structural branching, summed over every unit. The thing the
//! per-unit block can't tell you — where the *next* lever is — computed instead of guessed.
//!
//! A thread-local accumulator keyed by the unit's token string: the parse's cap-widen ladder re-runs a
//! unit's forest build several times, and each fires the recording hook, so keying by tokens and
//! OVERWRITING means the final (successful) attempt's attribution wins — no double-count. Enable with
//! [`begin`] before the sweep loop, drain the formatted report with [`take`] after. Same-thread only
//! (the accumulator is thread-local and the packed parse runs on the caller's thread).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::dcg::chart::attribute::{SiteKind, UnitAttribution};

/// One site flattened for aggregation: `sense` = which surface word; else which construction.
struct Row {
    sense: bool,
    key: String,
    factor: usize,
    /// A WordNet and a UMLS sense both survived here — a reconciliation candidate.
    cross_lexicon: bool,
}

thread_local! {
    static ROLLUP: RefCell<Option<BTreeMap<String, Vec<Row>>>> = const { RefCell::new(None) };
}

/// Arm the accumulator (call before a sweep). Idempotent; clears any prior state.
pub fn begin() {
    ROLLUP.with(|r| *r.borrow_mut() = Some(BTreeMap::new()));
}

/// Whether recording is armed — the parse hook checks this before computing an attribution to record.
pub(crate) fn is_enabled() -> bool {
    ROLLUP.with(|r| r.borrow().is_some())
}

/// Record one unit's attribution (overwriting any earlier attempt for the same tokens).
pub(crate) fn record(tokens: &[String], attr: &UnitAttribution) {
    ROLLUP.with(|r| {
        if let Some(map) = r.borrow_mut().as_mut() {
            // Sense rows carry the FELICITOUS count (senses that survived into a reading) — the real
            // multiplicity. A site whose alternatives all collapsed to one survivor is not ambiguity
            // and is dropped. Structure rows stay raw; they are reported, never ranked.
            let rows = attr
                .sites
                .iter()
                .filter(|s| s.kind == SiteKind::Structure || s.felicitous > 1)
                .map(|s| Row {
                    sense: s.kind == SiteKind::Sense,
                    key: if s.kind == SiteKind::Sense {
                        s.text.clone()
                    } else {
                        s.labels.join("+")
                    },
                    factor: if s.kind == SiteKind::Sense {
                        s.felicitous
                    } else {
                        s.factor
                    },
                    cross_lexicon: s.cross_lexicon,
                })
                .collect();
            map.insert(tokens.join(" "), rows);
        }
    });
}

/// Generic structural labels that carry no lever (plain application/composition, and the residual
/// attachment the felicity check mostly prunes) — lumped in the report so the NAMED constructions
/// (coordination, compound, adjective, PP, essive, relative, appositive, …) stand out.
fn is_generic(key: &str) -> bool {
    key.split('+').all(|k| {
        matches!(
            k,
            "apply" | "compose" | "Other" | "nominal-mod" | "type-raise"
        )
    })
}

/// One aggregated lever.
struct Agg {
    units: BTreeSet<String>,
    excess: u64,
    max: usize,
}

impl Agg {
    fn add(&mut self, unit: &str, factor: usize) {
        self.units.insert(unit.to_string());
        self.excess += (factor.saturating_sub(1)) as u64;
        self.max = self.max.max(factor);
    }
}

/// Drain the accumulator and format the ranked page roll-up, or `None` if not armed.
pub fn take() -> Option<String> {
    ROLLUP.with(|r| {
        let map = r.borrow_mut().take()?;
        Some(format_rollup(&map))
    })
}

/// Format the aggregate so far WITHOUT draining — a multi-minute sweep emits progress snapshots, so an
/// interrupted run still leaves its partial roll-up in the log instead of losing everything.
pub fn snapshot() -> Option<String> {
    ROLLUP.with(|r| r.borrow().as_ref().map(format_rollup))
}

fn format_rollup(map: &BTreeMap<String, Vec<Row>>) -> String {
    {
        let n_units = map.len();
        let mut sense: BTreeMap<String, Agg> = BTreeMap::new();
        let mut named: BTreeMap<String, Agg> = BTreeMap::new();
        let mut generic = Agg {
            units: BTreeSet::new(),
            excess: 0,
            max: 0,
        };
        let mut crossed: BTreeMap<String, Agg> = BTreeMap::new();
        for (unit, rows) in map {
            for row in rows {
                if row.cross_lexicon {
                    crossed
                        .entry(row.key.clone())
                        .or_insert_with(|| Agg {
                            units: BTreeSet::new(),
                            excess: 0,
                            max: 0,
                        })
                        .add(unit, row.factor);
                }
                let bucket = if row.sense {
                    &mut sense
                } else if is_generic(&row.key) {
                    generic.add(unit, row.factor);
                    continue;
                } else {
                    &mut named
                };
                bucket
                    .entry(row.key.clone())
                    .or_insert_with(|| Agg {
                        units: BTreeSet::new(),
                        excess: 0,
                        max: 0,
                    })
                    .add(unit, row.factor);
            }
        }

        let mut out = format!(
            "=== PAGE ATTRIBUTION ROLL-UP ({n_units} units) ===\n\
             excess = Σ(factor−1). SENSE counts only senses that SURVIVED into a reading, so it ranks\n\
             levers. STRUCTURE is RAW forest branching (kbest records no per-reading derivation, so it\n\
             cannot be intersected) — it is an upper bound and ranks NOTHING; do not size a grammar\n\
             change from it.\n"
        );
        out.push_str("SENSE levers (surviving senses, by surface form) — units · excess · max×:\n");
        out.push_str(&render_ranked(&sense, 15));
        out.push_str("STRUCTURE branching (RAW — not a ranking) — units · excess · max×:\n");
        out.push_str(&render_ranked(&named, 15));
        out.push_str(&format!(
            "  [generic attachment (apply/compose/…): {} units · excess {} — raw]\n",
            generic.units.len(),
            generic.excess,
        ));
        out.push_str(
            "CROSS-LEXICON co-survival (a WordNet AND a UMLS sense both reach a reading at the same\n             span) — each costs a real reading; either alignment never considered the pair or it\n             adjudicated them distinct. Check before assuming a missed merge:\n",
        );
        out.push_str(&render_ranked(&crossed, 20));
        out
    }
}

/// Rank a bucket map by excess (desc), then unit-breadth, and render the top `n`.
fn render_ranked(buckets: &BTreeMap<String, Agg>, n: usize) -> String {
    let mut rows: Vec<(&String, &Agg)> = buckets.iter().collect();
    rows.sort_by(|a, b| {
        b.1.excess
            .cmp(&a.1.excess)
            .then(b.1.units.len().cmp(&a.1.units.len()))
            .then(a.0.cmp(b.0))
    });
    let mut out = String::new();
    for (key, agg) in rows.into_iter().take(n) {
        out.push_str(&format!(
            "  «{key}»  {} units · excess {} · max ×{}\n",
            agg.units.len(),
            agg.excess,
            agg.max,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcg::chart::attribute::Site;

    /// `factor` = raw alternatives, `felicitous` = how many survived into a reading.
    fn site(sense: bool, key: &str, factor: usize, felicitous: usize) -> Site {
        Site {
            span: (0, 0),
            text: if sense {
                key.to_string()
            } else {
                String::new()
            },
            kind: if sense {
                SiteKind::Sense
            } else {
                SiteKind::Structure
            },
            factor,
            felicitous,
            inside: 1,
            labels: if sense { vec![] } else { vec![key.to_string()] },
            cross_lexicon: false,
        }
    }

    #[test]
    fn rollup_aggregates_by_surface_and_construction_and_dedups_retries() {
        begin();
        // `lines` raw ×4 but only 3 senses survive felicity ⇒ contributes 3, not 4.
        let a = UnitAttribution {
            readings: 8,
            sites: vec![site(true, "lines", 4, 3), site(false, "adjective", 2, 2)],
        };
        record(&["the".into(), "lines".into()], &a);
        record(&["the".into(), "lines".into()], &a); // widen retry, same tokens → overwrite
        let b = UnitAttribution {
            readings: 6,
            sites: vec![
                site(true, "lines", 3, 3),
                // raw ×5 but a single survivor ⇒ not ambiguity at all, must be dropped.
                site(true, "pruned_away", 5, 1),
                site(false, "coord(And)", 2, 2),
                site(false, "apply", 2, 2),
            ],
        };
        record(&["two".into(), "lines".into()], &b);

        let out = take().expect("armed");
        // "lines" in 2 units, excess (3−1)+(3−1)=4 — felicitous counts, and the retry did not double.
        assert!(out.contains("«lines»  2 units · excess 4"), "{out}");
        // A site whose alternatives all collapsed to one survivor is not a lever.
        assert!(!out.contains("pruned_away"), "{out}");
        assert!(out.contains("«adjective»  1 units · excess 1"), "{out}");
        assert!(out.contains("«coord(And)»  1 units · excess 1"), "{out}");
        // Generic `apply` is lumped, not surfaced as a named lever.
        assert!(!out.contains("«apply»"), "{out}");
        assert!(out.contains("generic attachment"), "{out}");
        // Drained: a second take is empty.
        assert!(take().is_none());
    }

    #[test]
    fn is_generic_needs_every_part_generic() {
        assert!(is_generic("apply"));
        assert!(is_generic("apply+compose"));
        assert!(!is_generic("adjective"));
        assert!(!is_generic("apply+adjective")); // any named part ⇒ a lever
    }
}
