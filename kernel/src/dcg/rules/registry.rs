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
//! **The token-keyed rule registry** — the single definition of the sem-reading binary constructions:
//! relative clauses, coordination, `but not`, the reciprocal, the appositives, and close apposition.
//!
//! Two halves, and both are shared by BOTH chart paths:
//! - [`Parser::binary_sites`] says WHERE each rule fires inside a chart cell (the operand
//!   sub-spans). The unpacked CKY cross-products the operand cells' ITEMS through it; the packed forest
//!   turns the same sites into `Edge::Binary` hyperedges over NODES.
//! - [`Parser::apply_bin_rule`] BUILDS the result for one (left, right) item-pair — used by the
//!   unpacked path per item-pair, and by the packed path twice: on representatives to decide an edge,
//!   and again per item-pair in the cube extractor to materialise it.
//!
//! Before this existed the span arithmetic was written once per driver, and "mirrors the unpacked rule
//! exactly" was a comment rather than a property of the code. Each rule's DECISION is a function of the
//! packing signature (`super::super::chart::forest::Sig` — category, ENF provenance, and the coordination-sem
//! bit), which is what makes deciding on a representative sound.
//!
//! NOT here: **pied-piping**, a QUATERNARY rule (noun + subject + VP + a preposition it reaches out of
//! the lexicon for) that the packed forest has no edge shape for. It stays inline on the unpacked path
//! and the router (`parse_needs_unpacked`) diverts sentences containing it.
use crate::nbe::term::{Exp, Patt};

use super::super::category::*;
use super::super::grammar::Grammar;
use super::super::item::{Combinator, Item};
use super::super::reserved::ReservedKind;
use super::constructions::*;

impl Grammar {
    /// Object-position non-restrictive (appositive) relative NP (D62 §2 #2A, object slot): the
    /// antecedent NP `cat_np(C, _)` + a comma-set-off relative `, which/that [body]` raised into a
    /// transitive verb's OBJECT slot (mirroring `a_obj`), conjoining the appositive assertion —
    /// `(S\NP)\((S\NP)/NP)` with sem `λTV. λs. logic:And(TV(r)(s), body(r))`. Reuses the `a` object
    /// determiner's raised cat (instantiating its bound `T := C`), as [`Self::bare_plural_nps`]
    /// reuses `these`, so it composes with any transitive verb. The SUBJECT-position appositive is
    /// [`relativize_appos`] (type-raised `S/(S\NP)`); prep-object position rides that subject form
    /// through the GQ-as-preposition-object rule. `None` unless the antecedent is a `cat_np`, the body
    /// a declarative `S/NP`/`S\NP`, the `a_obj` cat is loaded, and `logic:And` resolves.
    pub(crate) fn appositive_obj(&self, ante: &Item, body: &Item) -> Option<Item> {
        let [c, _num] = is_ctor(ante.cat(), "cat_np")? else {
            return None;
        };
        let body_args = is_ctor(body.cat(), "fwd").or_else(|| is_ctor(body.cat(), "bwd"))?;
        let [s, _np] = body_args else {
            return None;
        };
        if !matches!(is_ctor(s, "cat_s"),
            Some([mood, _]) if matches!(mood, Exp::InductiveCtor(_, n, _) if n == "dcl"))
        {
            return None;
        }
        let and = super::super::category::resolve_inductive(&self.layer, "urn:eigenius:logic:And")?;
        // The `a` object determiner's raised cat `cat_forall(sg, λT. (S\NP)\((S\NP)/NP_T))` (the
        // `bwd`-headed body); instantiate `T := C` for this antecedent's class.
        let det_cat = self
            .dets
            .a
            .iter()
            .find(|c| cat_forall_body_head(c) == Some("bwd"))?;
        let [_dnum, body_lam] = is_ctor(det_cat, "cat_forall")? else {
            return None;
        };
        let Exp::Lam(Patt::Var(tvar), obj_body) = body_lam else {
            return None;
        };
        let mut subst = CatSubst::new();
        subst.insert(tvar.clone(), c.clone());
        let cat = subst_cat(obj_body, &subst);
        // sem: λTV. λsubj. And(TV(r)(subj), body(r)) — the in-situ object raise conjoining the
        // appositive assertion on the antecedent referent `r`.
        let (tv, sj) = ("__appos_tv", "__appos_s");
        let r = ante.sem().clone();
        let tv_r_s = Exp::App(
            Box::new(Exp::App(Box::new(Exp::Var(tv.into())), Box::new(r.clone()))),
            Box::new(Exp::Var(sj.into())),
        );
        let body_r = Exp::App(Box::new(body.sem().clone()), Box::new(r));
        let sem = Exp::Lam(
            Patt::Var(tv.into()),
            Box::new(Exp::Lam(
                Patt::Var(sj.into()),
                Box::new(Exp::InductiveType(and, vec![tv_r_s, body_r])),
            )),
        );
        Some(Item::with_cost(
            cat,
            sem,
            ante.cost().saturating_add(body.cost()),
        ))
    }

    /// **The token-keyed binary construction registry** (reorganization plan Phase 2): the SINGLE
    /// definition of *where* each sem-reading binary rule fires inside a chart cell `[i, j]` — the two
    /// operand sub-spans, and the rule to apply. The reserved word(s) keying each construction sit
    /// BETWEEN (or after) the operands and have no chart node of their own.
    ///
    /// Both chart paths consume this one list: the unpacked CKY cross-products the operand cells'
    /// ITEMS through [`Self::apply_bin_rule`], and the packed forest records one
    /// [`super::super::chart::forest::Edge::Binary`] per node-pair ([`Self::binary_edges`]) and re-applies the same
    /// rule per item-pair at extraction. Before Phase 2 this span arithmetic was written TWICE — once
    /// per driver — and "mirrors the unpacked rule exactly" was a comment, not a property.
    ///
    /// NOT here: **pied-piping** (`[noun] [prep] which [subj] [VP]`), a TERNARY rule the packed forest
    /// builds no edge for. It stays inline on the unpacked path and is routed there by
    /// [`Self::parse_needs_unpacked`]; Phase 3 (marker-category nodes) is what lets it join the
    /// registry. Also not here: the categorial rules ([`apply`]), which are sem-blind and need no
    /// token trigger, and the group/distributive rules ([`super::super::item::apply_group`]).
    pub(crate) fn binary_sites(&self, tokens: &[String], i: usize, j: usize) -> Vec<BinSite> {
        let mut sites: Vec<BinSite> = Vec::new();
        // --- constructions keyed by a reserved word BETWEEN the two operands ---
        for c in (i + 1)..j {
            // Restrictive relative: `[noun] that/which [body]` → a refined noun.
            if self.reserved.is_relativizer(tokens[c].as_str()) {
                sites.push(BinSite::new((i, c - 1), (c + 1, j), BinRule::Relativize));
            }
            // Coordination: `[X] and/or/`,` [Y]` → a `cat_coord` list or a `cat_group`.
            if let Some(op) = self.reserved.coord_connective(tokens[c].as_str()) {
                sites.push(BinSite::new(
                    (i, c - 1),
                    (c + 1, j),
                    BinRule::Coordinate(op),
                ));
            }
            // Contrastive: `[O₁] but not [O₂]` — a TWO-token coordinator (`but` + `not`), so the right
            // operand starts at `c + 2`. (`but` alone stays the sentential subordinator, an ordinary
            // lexical leaf, so the two never conflict.)
            if self.reserved.is(&tokens[c], ReservedKind::ContrastiveBut)
                && tokens
                    .get(c + 1)
                    .is_some_and(|t| self.reserved.is(t, ReservedKind::Negator))
                && c + 2 <= j
            {
                sites.push(BinSite::new((i, c - 1), (c + 2, j), BinRule::ButNot));
            }
        }
        // --- non-restrictive (appositive) relative: `[NP] , that/which [body] [,]` ---
        // The comma BEFORE the relativizer is what distinguishes it from the restrictive rule (whose
        // noun must be relativizer-adjacent), and a trailing comma is absorbed into the span so the
        // appositive NP ends up adjacent to the matrix VP. Both the subject-position (type-raised) and
        // the verb-object-position (in-situ raise) readings are offered; the builders gate by category.
        for c in (i + 2)..=j {
            if !self.reserved.is_relativizer(tokens[c].as_str())
                || !self.reserved.is_comma(&tokens[c - 1])
            {
                continue;
            }
            let body_end = if self.reserved.is_comma(&tokens[j]) {
                j - 1
            } else {
                j
            };
            if c < body_end {
                let (ante, body) = ((i, c - 2), (c + 1, body_end));
                sites.push(BinSite::new(ante, body, BinRule::AppositiveSubj));
                sites.push(BinSite::new(ante, body, BinRule::AppositiveObj));
            }
        }
        // --- close nominal apposition: `[head] [name-group]` ---
        // ADJACENT (no reserved token between them), so — like the plain categorial `Combine` loop —
        // every split is a candidate and `appose_group` gates by shape + head kind.
        for m in i..j {
            sites.push(BinSite::new((i, m), (m + 1, j), BinRule::ApposeGroup));
        }
        // --- reciprocal: `[group] <TV> each other` --- keyed on the TRAILING reserved pair, so the
        // verb spans `[s, j-2]` and the subject group `[i, s-1]` at every split `s`.
        if j >= 3
            && self
                .reserved
                .is(&tokens[j - 1], ReservedKind::ReciprocalEach)
            && self.reserved.is(&tokens[j], ReservedKind::ReciprocalOther)
        {
            for s in (i + 1)..=(j - 2) {
                sites.push(BinSite::new((i, s - 1), (s, j - 2), BinRule::Reciprocal));
            }
        }
        sites
    }

    /// Materialise a token-keyed [`BinRule`] for one (left, right) item-pair — the
    /// single builder BOTH chart paths use (the unpacked CKY calls it per item-pair; the packed path
    /// calls it on representatives to decide an edge, and again per item-pair in `cube` to
    /// materialise). The DECISION (whether it returns `Some`) is a function of the packing
    /// [`super::super::chart::forest::Sig`] — categories, ENF provenance, and the coordination-sem bit — so it is
    /// consistent across every item of a packed node; the sem is built here per pair.
    pub(crate) fn apply_bin_rule(&self, rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
        let cost = l.cost().saturating_add(r.cost());
        match rule {
            BinRule::Relativize => relativize(l.cat(), r.cat(), r.sem())
                .map(|(cat, sem)| Item::with_cost(cat, sem, cost)),
            BinRule::Coordinate(op) => {
                // The list-with-operator model (D63 §8.4 Phase 3): a prop-ending conjunct builds/extends
                // a deferred `cat_coord` (folded later by the `CoordComplete` unary edge); an NP conjunct
                // builds a `cat_group`. Each enforces its own left-branching NF (right conjunct is a
                // single non-list constituent), so no `is_coordination` guard here.
                coordinate_prop(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                    .or_else(|| coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer))
                    .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
            }
            BinRule::ButNot => {
                if cats_coordinate(l.cat(), r.cat(), &self.layer) {
                    if sem_is_coordination(r.sem()) {
                        return None;
                    }
                    coordinate_but_not_sem(l.cat(), l.sem(), r.sem(), &self.layer)
                        .map(|sem| Item::with_cost(l.cat().clone(), sem, cost))
                } else {
                    coordinate_but_not(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
                }
            }
            BinRule::Reciprocal => reciprocate(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                .map(|(cat, sem)| Item::with_cost(cat, sem, cost)),
            BinRule::AppositiveSubj => {
                relativize_appos(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                    .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
            }
            BinRule::AppositiveObj => self.appositive_obj(l, r),
            BinRule::ApposeGroup => appose_group(l.cat(), r.cat(), r.sem(), &self.layer)
                .map(|(cat, sem)| Item::with_cost(cat, sem, cost)),
        }
    }
}

/// One firing **site** of a token-keyed binary construction inside a chart cell: the two operand
/// sub-spans (inclusive cell coordinates) and the rule to apply to them. Produced by the registry
/// ([`Parser::binary_sites`]) and consumed by both chart paths — the unpacked CKY iterates the
/// operand cells' items, the packed forest the operand cells' nodes.
#[derive(Clone, Copy)]
pub(crate) struct BinSite {
    pub(crate) left: (usize, usize),
    pub(crate) right: (usize, usize),
    pub(crate) rule: BinRule,
}

impl BinSite {
    pub(crate) fn new(left: (usize, usize), right: (usize, usize), rule: BinRule) -> Self {
        BinSite { left, right, rule }
    }
}

impl Grammar {
    /// The **bare KIND NP shift** (D63 kind-predication reshape §7.4,
    /// `docs/notes/d63-kind-predication-reshape.md`) — one rule for a determiner-less **mass OR plural**
    /// common noun (core-en's `bnp`, `det=nil`, is likewise a single rule over `pl-or-mass`). The noun
    /// denotes its KIND, and as a bare argument it is that kind *realized as an individual*, `kind_of(t)
    /// : Entity` (Chierchia's ∩): a **closed** reading — "genes affect HeLa" → `affect(hela,
    /// kind_of(Gene))`, "instability affects HeLa" → `affects(kind_of(Instability), hela)`. Not the
    /// earlier deferred-quantifier (`Quantification`-hole, now retired) open parse — a generic is a *complete* proposition
    /// about the kind, and its warrant (citation / observation / derivation) belongs on the claim's
    /// **grade**, not a parser hole.
    ///
    /// `det_form` is the existential determiner whose subject- (`fwd`) and object- (`bwd`) type-raised
    /// CATEGORIES are reused: `a` for mass (singular agreement), `these` for plural. The raised category
    /// is built **directly** — substitute the noun's BASE class for the determiner's type variable — NOT
    /// via [`apply`]. That bypass is **load-bearing**: routing a REFINED (compound / relative) noun
    /// `cat_n(Σx:C. R, num)` through `apply` hits the GQ witness-projection (`DetRefine`, `parser.rs`),
    /// producing the ill-typed `Fst(kind_of(Σ))` — a kind nominalizes the WHOLE type, it does not project
    /// witnesses (this was the bare-plural-compound bug, "nucleotide repeat regions"). Indexing the raised
    /// category by the base `C` (`C ≤ Entity`) lets it fill a verb slot; the sem nominalizes
    /// `kind_of(Σx:C. R)`, keeping the compound's content. Type-raising (vs a plain `cat_np`) keeps it
    /// **argument-only**, so it cannot feed the named-entity compound rule — a noun's prenominal reading
    /// stays the `compound_kind` classifier, no spurious `compound(x, kind_of(C))` duplicate (§7.5).
    fn kind_raised_nps(&self, noun: &Item, det_cats: &[Exp], want_num: &str) -> Vec<Item> {
        let Some([t, num]) = is_ctor(noun.cat(), "cat_n") else {
            return Vec::new();
        };
        if !matches!(num, Exp::InductiveCtor(_, n, _) if n == want_num) {
            return Vec::new();
        }
        let base = base_class(t); // the raised category's NP index (a class in the subsumption lattice)
        let kind = kind_of(t.clone()); // the nominalized whole type — `kind_of(Σx:C.R)` for a compound
        det_cats
            .iter()
            .filter_map(|det_cat| {
                let head = cat_forall_body_head(det_cat)?;
                let Some([_dnum, body_lam]) = is_ctor(det_cat, "cat_forall") else {
                    return None;
                };
                let Exp::Lam(Patt::Var(tvar), body) = body_lam else {
                    return None;
                };
                let mut subst = CatSubst::new();
                subst.insert(tvar.clone(), base.clone());
                let cat = subst_cat(body, &subst);
                let sem = match head {
                    // subject-raised `S/(S\NP)`: `λV. V(kind)`.
                    "fwd" => Exp::Lam(
                        Patt::Var("V".into()),
                        Box::new(Exp::App(
                            Box::new(Exp::Var("V".into())),
                            Box::new(kind.clone()),
                        )),
                    ),
                    // object-raised `(S\NP)\((S\NP)/NP)`: `λTV. λsubj. TV(kind, subj)`.
                    "bwd" => {
                        let tv_app = Exp::App(
                            Box::new(Exp::App(
                                Box::new(Exp::Var("TV".into())),
                                Box::new(kind.clone()),
                            )),
                            Box::new(Exp::Var("subj".into())),
                        );
                        Exp::Lam(
                            Patt::Var("TV".into()),
                            Box::new(Exp::Lam(Patt::Var("subj".into()), Box::new(tv_app))),
                        )
                    }
                    _ => return None,
                };
                Some(Item::with_cost(cat, sem, noun.cost()))
            })
            .collect()
    }

    /// Bare-MASS NP shift — the kind shift over a mass noun, singular agreement (reuse `a`).
    fn bare_mass_nps(&self, noun: &Item) -> Vec<Item> {
        self.kind_raised_nps(noun, &self.dets.a, "mass")
    }

    /// Bare-PLURAL NP shift — the kind shift over a plural noun, plural agreement (reuse `these`). A bare
    /// plural denotes its kind (Carlson 1977), identically to a bare mass noun — only surface number
    /// differs — so it shares [`Self::kind_raised_nps`] (the §7.4 mass/plural unification). A bare
    /// *singular* count noun (`*gene is a vulnerability`) correctly does not shift.
    fn bare_plural_nps(&self, noun: &Item) -> Vec<Item> {
        self.kind_raised_nps(noun, &self.dets.these, "pl")
    }

    /// The full **bare-nominal shift** (core-en's `bnp` unary rule + the copula kind-subject reading,
    /// D63 §8.5 Slice 3c): given a `cat_n`, produce (i) the `cat_kind` **copula-subject** edge
    /// ([`crate::dcg::kind_subject`]; a bare-plural kind, so `are_kind` yields `subclass_of`) and (ii)
    /// the raised **bare-argument NPs** ([`Self::bare_plural_nps`]/[`Self::bare_mass_nps`]). The single
    /// rule applied at BOTH leaf seeding AND to COMPOSED cells in both chart paths, so a compound
    /// `cat_n` (`repeat regions`, formed by the `KindCompound` rule) shifts exactly like a leaf noun —
    /// `bnp` is a rule over any `n`, not a leaf-only shortcut. Non-`cat_n`/non-plural/non-mass → empty.
    pub(crate) fn bare_nominal_shifts(&self, it: &Item) -> Vec<Item> {
        let mut v: Vec<Item> = crate::dcg::kind_subject(it.cat(), it.sem())
            .map(|(cat, sem)| Item::with_cost(cat, sem, it.cost()))
            .into_iter()
            .collect();
        v.extend(self.bare_plural_nps(it));
        v.extend(self.bare_mass_nps(it));
        v
    }
}

impl Grammar {
    /// Forward bounded type-raise (D63 §8.9 Slice 6-T): every name `NP` in a cell's items to
    /// `S/(S\NP)`, tagged `Combinator::TypeRaised` so ENF lets it only *compose*, never apply.
    /// Non-`NP` items (functors, groups, kinds, determined NPs) yield nothing.
    ///
    /// A unary RULE, and it lives with the other unary shifts. It used to sit in `seed.rs` and take the
    /// layer as an argument — but it is not seeding (both chart drivers apply it to COMPOSED cells too),
    /// and the layer is already the grammar's.
    pub(crate) fn raise_nps(&self, items: &[Item]) -> Vec<Item> {
        items
            .iter()
            .filter_map(|it| {
                type_raise(it.cat(), it.sem(), &self.layer)
                    .map(|(cat, sem)| Item::from_parts(cat, sem, Combinator::TypeRaised, it.cost()))
            })
            .collect()
    }
}

/// Which token-keyed binary rule a [`super::super::chart::forest::Edge::Binary`] applies at materialisation (D63 §11 3g.3).
///
/// Every rule here is decided on node REPRESENTATIVES at forest construction and re-applied per
/// item-pair at extraction, so each rule's decision must be a function of the [`super::super::chart::forest::Sig`] alone. Two of
/// them (`Coordinate`, `ButNot`) consult `sem_is_coordination` in that decision — which is exactly why
/// that predicate is a component of `Sig` (see [`super::super::chart::forest::node_sig`]). Adding a rule whose decision reads some
/// OTHER sem property requires extending `Sig` to carry it; it is not enough to "mirror the unpacked
/// rule".
///
/// The rules' trigger geometry — which sub-spans each fires over — is defined ONCE, in
/// `Parser::binary_sites`, and consumed by both chart paths.
#[derive(Clone, Copy)]
pub(crate) enum BinRule {
    /// `[noun] that/which [body] → refined noun` (`relativize`).
    Relativize,
    /// `[X] and/or [Y]` → a Prop-conjunction (same category) or a `cat_group` (`coordinate_sem` /
    /// `coordinate_np`); `op` is the connective IRI (`logic:And` / `logic:Or`).
    Coordinate(&'static str),
    /// `[O₁] but not [O₂]` → contrastive `a ∧ ¬b` or a `conn_but_not` group.
    ButNot,
    /// `[group] <TV> each other → S` (`reciprocate`).
    Reciprocal,
    /// Non-restrictive appositive `[NP] , that/which [body]` — subject/prep-object position
    /// (`relativize_appos`).
    AppositiveSubj,
    /// Non-restrictive appositive in verb-object position (the in-situ object raise).
    AppositiveObj,
    /// Close nominal apposition (D63 §8.4 Phase 6, RC-6): a definite/bare common-noun HEAD immediately
    /// followed by a coreferential NAME-GROUP — "the genes BRCA1 and MSH2" (`appose_group`). Unlike the
    /// other `BinRule`s there is NO reserved token between the two spans; the head and group are
    /// ADJACENT, so every split is tried and the rule gates by shape + head-kind at construction.
    ApposeGroup,
}

/// Which composed-cell unary shift a [`super::super::chart::forest::Edge::Unary`] represents (D63 blueprint §11 3c.4b).
#[derive(Clone, Copy)]
pub(crate) enum UnaryKind {
    /// Forward bounded type-raising `T`: `NP → S/(S\NP)` (`raise_nps`).
    Raise,
    /// Bare-plural / bare-mass argument shift: a plural/mass `cat_n` → a deferred-quantifier NP.
    BareNp,
    /// Fronted participial adjunct: a subject-gapped `ger` VP → a sentence pre-modifier `S/S`.
    FrontParticipial,
    /// Fronted-modifier comma absorption (D62 §2 #5): a sentence-initial `S/S` pre-modifier at
    /// `[0, j-1]` absorbs a trailing comma at `j`, yielding the same modifier over `[0, j]` (so it can
    /// forward-apply across the otherwise node-less comma to the matrix clause). The child is the
    /// narrower `[0, j-1]` node, NOT a same-span transform; the item is carried through unchanged.
    AbsorbComma,
    /// Coordination list-completion (D63 §8.4 Phase 3): fold a prop-ending `cat_coord` list into its
    /// base category (`op(op(m₀, m₁),…)`, via `complete_coord`). The `cat_coord` node stays available
    /// (a longer list extends it); this shift adds the folded base-category node a matrix consumes.
    CoordComplete,
}
