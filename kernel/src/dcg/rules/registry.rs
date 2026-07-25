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
use super::super::holes::{freshen_anaphor, hole_base};
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
        // Interpreter over the rule table (Phase 2c): each [`TokBinRule`] contributes its firing sites
        // via its own `trigger`. Site order across rules is not load-bearing — both drivers build ALL
        // sites, and the forest is a set of edges.
        let mut sites: Vec<BinSite> = Vec::new();
        for rule in bin_rules() {
            (rule.trigger)(self, tokens, i, j, &mut sites);
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
        // Interpreter over the rule table (Phase 2c): dispatch to the matched rule's `build`. The
        // `BinRule` tag carries per-firing data (the coordination connective), so it is looked up by
        // its `BinKind` discriminant.
        let desc = bin_rules().iter().find(|d| d.kind == rule.kind())?;
        (desc.build)(self, rule, l, r)
    }
}

/// A **token-keyed binary construction**, fully described in one place (Phase 2c,
/// `docs/notes/grammar-formalization-plan.md`): its `trigger` geometry (where it fires in the token
/// stream), its sem-`build`er, and whether its DECISION reads the sem. Both [`Grammar::binary_sites`]
/// and [`Grammar::apply_bin_rule`] are interpreters over the [`bin_rules`] table; the trigger/build
/// logic stays named functions (as the categorial builders do), the SET of rules is the data.
struct TokBinRule {
    /// Rule identity — for tracing / on-chain naming; carried, not consumed at runtime.
    #[allow(dead_code)]
    name: &'static str,
    /// Discriminant linking a firing site's [`BinRule`] tag back to this descriptor.
    kind: BinKind,
    /// Emits this rule's firing sites for a cell `[i, j]` into `out` (the token geometry as code —
    /// span arithmetic over reserved-word predicates).
    trigger: TriggerFn,
    /// Materialises the result item for one `(left, right)` pair — the sem-builder half.
    build: BinBuild,
    /// **Escape-hatch declaration.** Whether this rule's *decision* (whether it fires) reads the sem —
    /// which requires the packing [`super::super::chart::forest::Sig`] to carry the coordination bit.
    /// The categorial rules are sem-blind; these item-level rules need not be. Pinned by
    /// `escape_hatch_matches_sig` below; a future forest change can consume it to enforce soundness.
    #[allow(dead_code)]
    reads_sem: bool,
}

type TriggerFn = fn(&Grammar, &[String], usize, usize, &mut Vec<BinSite>);
type BinBuild = fn(&Grammar, BinRule, &Item, &Item) -> Option<Item>;

/// The discriminant of a [`BinRule`] — a [`BinRule`] carries per-firing data (the coordination
/// connective IRI), so it is not itself an `Eq` table key; this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BinKind {
    Relativize,
    Coordinate,
    ButNot,
    Reciprocal,
    AppositiveSubj,
    AppositiveObj,
    ApposeGroup,
}

impl BinRule {
    fn kind(self) -> BinKind {
        match self {
            BinRule::Relativize => BinKind::Relativize,
            BinRule::Coordinate(_) => BinKind::Coordinate,
            BinRule::ButNot => BinKind::ButNot,
            BinRule::Reciprocal => BinKind::Reciprocal,
            BinRule::AppositiveSubj => BinKind::AppositiveSubj,
            BinRule::AppositiveObj => BinKind::AppositiveObj,
            BinRule::ApposeGroup => BinKind::ApposeGroup,
        }
    }
}

/// The token-keyed binary rule table (all `const`, so no `LazyLock`). Priority = order, but site
/// order is not load-bearing (both drivers build every site).
static BIN_RULES: [TokBinRule; 7] = [
    TokBinRule {
        name: "relativize",
        kind: BinKind::Relativize,
        trigger: trig_relativize,
        build: build_relativize,
        reads_sem: false,
    },
    TokBinRule {
        name: "coordinate",
        kind: BinKind::Coordinate,
        trigger: trig_coordinate,
        build: build_coordinate,
        reads_sem: true,
    },
    TokBinRule {
        name: "but_not",
        kind: BinKind::ButNot,
        trigger: trig_but_not,
        build: build_but_not,
        reads_sem: true,
    },
    TokBinRule {
        name: "appositive_subj",
        kind: BinKind::AppositiveSubj,
        trigger: trig_appositive_subj,
        build: build_appositive_subj,
        reads_sem: false,
    },
    TokBinRule {
        name: "appositive_obj",
        kind: BinKind::AppositiveObj,
        trigger: trig_appositive_obj,
        build: build_appositive_obj,
        reads_sem: false,
    },
    TokBinRule {
        name: "appose_group",
        kind: BinKind::ApposeGroup,
        trigger: trig_appose_group,
        build: build_appose_group,
        reads_sem: false,
    },
    TokBinRule {
        name: "reciprocal",
        kind: BinKind::Reciprocal,
        trigger: trig_reciprocal,
        build: build_reciprocal,
        reads_sem: false,
    },
];

fn bin_rules() -> &'static [TokBinRule] {
    &BIN_RULES
}

// ── Trigger geometries (extracted verbatim from the former `binary_sites`) ───────────────────────

/// Restrictive relative `[noun] that/which [body]`: a relativizer BETWEEN the operands.
// `c` is a split-point index used for both operand spans, not just to index `tokens`.
#[allow(clippy::needless_range_loop)]
fn trig_relativize(g: &Grammar, tokens: &[String], i: usize, j: usize, out: &mut Vec<BinSite>) {
    for c in (i + 1)..j {
        if g.reserved.is_relativizer(tokens[c].as_str()) {
            out.push(BinSite::new((i, c - 1), (c + 1, j), BinRule::Relativize));
        }
    }
}

/// Coordination `[X] and/or/`,` [Y]`: a coordinating connective BETWEEN the operands (the connective
/// IRI rides in the `BinRule::Coordinate` tag).
// `c` is a split-point index used for both operand spans, not just to index `tokens`.
#[allow(clippy::needless_range_loop)]
fn trig_coordinate(g: &Grammar, tokens: &[String], i: usize, j: usize, out: &mut Vec<BinSite>) {
    for c in (i + 1)..j {
        if let Some(op) = g.reserved.coord_connective(tokens[c].as_str()) {
            out.push(BinSite::new(
                (i, c - 1),
                (c + 1, j),
                BinRule::Coordinate(op),
            ));
        }
    }
}

/// Contrastive `[O₁] but not [O₂]`: a TWO-token coordinator (`but` + `not`), so the right operand
/// starts at `c + 2`.
fn trig_but_not(g: &Grammar, tokens: &[String], i: usize, j: usize, out: &mut Vec<BinSite>) {
    for c in (i + 1)..j {
        if g.reserved.is(&tokens[c], ReservedKind::ContrastiveBut)
            && tokens
                .get(c + 1)
                .is_some_and(|t| g.reserved.is(t, ReservedKind::Negator))
            && c + 2 <= j
        {
            out.push(BinSite::new((i, c - 1), (c + 2, j), BinRule::ButNot));
        }
    }
}

/// The `(antecedent, body)` spans of a non-restrictive (appositive) relative `[NP] , that/which
/// [body] [,]` — a relativizer at `c` preceded by a comma, with trailing-comma absorption. Shared by
/// the subject- and object-position readings (each is its own rule with its own builder).
fn appositive_spans(
    g: &Grammar,
    tokens: &[String],
    i: usize,
    j: usize,
) -> Vec<((usize, usize), (usize, usize))> {
    let mut spans = Vec::new();
    for c in (i + 2)..=j {
        if !g.reserved.is_relativizer(tokens[c].as_str()) || !g.reserved.is_comma(&tokens[c - 1]) {
            continue;
        }
        let body_end = if g.reserved.is_comma(&tokens[j]) {
            j - 1
        } else {
            j
        };
        if c < body_end {
            spans.push(((i, c - 2), (c + 1, body_end)));
        }
    }
    spans
}

fn trig_appositive_subj(
    g: &Grammar,
    tokens: &[String],
    i: usize,
    j: usize,
    out: &mut Vec<BinSite>,
) {
    for (ante, body) in appositive_spans(g, tokens, i, j) {
        out.push(BinSite::new(ante, body, BinRule::AppositiveSubj));
    }
}

fn trig_appositive_obj(g: &Grammar, tokens: &[String], i: usize, j: usize, out: &mut Vec<BinSite>) {
    for (ante, body) in appositive_spans(g, tokens, i, j) {
        out.push(BinSite::new(ante, body, BinRule::AppositiveObj));
    }
}

/// Close nominal apposition `[head] [name-group]`: ADJACENT operands, every split a candidate.
fn trig_appose_group(_g: &Grammar, _tokens: &[String], i: usize, j: usize, out: &mut Vec<BinSite>) {
    for m in i..j {
        out.push(BinSite::new((i, m), (m + 1, j), BinRule::ApposeGroup));
    }
}

/// Reciprocal `[group] <TV> each other`: keyed on the TRAILING reserved pair, verb `[s, j-2]`.
fn trig_reciprocal(g: &Grammar, tokens: &[String], i: usize, j: usize, out: &mut Vec<BinSite>) {
    if j >= 3
        && g.reserved.is(&tokens[j - 1], ReservedKind::ReciprocalEach)
        && g.reserved.is(&tokens[j], ReservedKind::ReciprocalOther)
    {
        for s in (i + 1)..=(j - 2) {
            out.push(BinSite::new((i, s - 1), (s, j - 2), BinRule::Reciprocal));
        }
    }
}

// ── Builders (extracted verbatim from the former `apply_bin_rule`) ───────────────────────────────

fn build_relativize(_g: &Grammar, _rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    let cost = l.cost().saturating_add(r.cost());
    relativize(l.cat(), r.cat(), r.sem()).map(|(cat, sem)| Item::with_cost(cat, sem, cost))
}

/// The list-with-operator model (D63 §8.4 Phase 3): a prop-ending conjunct builds/extends a deferred
/// `cat_coord`; an NP conjunct builds a `cat_group`. Each enforces its own left-branching NF, so no
/// outer `is_coordination` guard here — the sem-reading NF check lives inside `coordinate_prop`.
fn build_coordinate(g: &Grammar, rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    let BinRule::Coordinate(op) = rule else {
        return None;
    };
    let cost = l.cost().saturating_add(r.cost());
    coordinate_prop(op, l.cat(), l.sem(), r.cat(), r.sem(), &g.layer)
        .or_else(|| coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &g.layer))
        .or_else(|| coordinate_mod(l.cat(), l.sem(), r.cat(), r.sem(), &g.layer))
        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
}

fn build_but_not(g: &Grammar, _rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    let cost = l.cost().saturating_add(r.cost());
    if cats_coordinate(l.cat(), r.cat(), &g.layer) {
        // Escape hatch: the DECISION reads the sem (a completed coordination cannot be a `but not`
        // right operand) — this is why `ButNot` declares `reads_sem` and `Sig` carries the bit.
        if sem_is_coordination(r.sem()) {
            return None;
        }
        coordinate_but_not_sem(l.cat(), l.sem(), r.sem(), &g.layer)
            .map(|sem| Item::with_cost(l.cat().clone(), sem, cost))
    } else {
        coordinate_but_not(l.cat(), l.sem(), r.cat(), r.sem(), &g.layer)
            .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
    }
}

fn build_reciprocal(g: &Grammar, _rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    let cost = l.cost().saturating_add(r.cost());
    reciprocate(l.cat(), l.sem(), r.cat(), r.sem(), &g.layer)
        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
}

fn build_appositive_subj(g: &Grammar, _rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    let cost = l.cost().saturating_add(r.cost());
    relativize_appos(l.cat(), l.sem(), r.cat(), r.sem(), &g.layer)
        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
}

fn build_appositive_obj(g: &Grammar, _rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    g.appositive_obj(l, r)
}

fn build_appose_group(g: &Grammar, _rule: BinRule, l: &Item, r: &Item) -> Option<Item> {
    let cost = l.cost().saturating_add(r.cost());
    appose_group(l.cat(), r.cat(), r.sem(), &g.layer)
        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
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
                                       // PLAIN `cat_np`, following core-en's `bnp` type-changing rule (`n $1 → np $1`,
                                       // `unary-rules.xsl`). A bare kind is NOT a generalized quantifier — core-en type-raises only
                                       // `QuantNP` — so under Chierchia's `∩` it denotes an INDIVIDUAL (`kind_of(t) : Entity`) and a
                                       // plain referential NP is its correct category.
                                       //
                                       // Needed because type-raising FIXES the result category: the object raise
                                       // `(S\NP)\((S\NP)/NP)` only satisfies a functor whose innermost argument is its NP with
                                       // result `S\NP`. A verb with a further argument after its object — the ESSIVE
                                       // `((S\NP)/cat_pp_arg(prep_as))/NP`, the ditransitive — is unreachable, and no composition
                                       // degree repairs it (`<Bⁿ` needs `((S\NP)/NP)/Z…`; the essive's argument order is reversed).
                                       // Witnessed: "We evaluated WRN as a biomarker" parsed (proper noun → plain `cat_np`) while
                                       // "We evaluated MSI as a biomarker" gapped (bare kind → raised only). A plain `cat_np` fills
                                       // ANY argument slot, closing the whole class of frame instead of one raise shape per frame.
                                       //
                                       // AGREEMENT: take the number the determiner-derived raise supplied — a bare MASS noun agrees
                                       // SINGULAR ("instability affects HeLa"); a bare plural stays plural. Keeping the noun's own
                                       // `mass` feature would fail to agree with a 3sg verb.
        let mut out: Vec<Item> = Vec::new();
        if let Exp::InductiveCtor(decl, _, _) = noun.cat() {
            if let Exp::InductiveCtor(num_decl, _, _) = num {
                out.push(Item::from_parts(
                    Exp::InductiveCtor(
                        decl.clone(),
                        "cat_np".into(),
                        vec![
                            base_class(t),
                            Exp::InductiveCtor(
                                num_decl.clone(),
                                if want_num == "mass" { "sg" } else { want_num }.into(),
                                Vec::new(),
                            ),
                        ],
                    ),
                    kind_of(t.clone()),
                    Combinator::KindRaised,
                    noun.cost(),
                ));
            }
        }
        out
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
#[derive(Clone, Copy, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Pre-nominal modifier lift (D63 coordinated-modifier category): a modifier-eligible item →
    /// `cat_mod` (`mod_lifts`), so modifiers can coordinate before meeting the head noun. Fires on
    /// composed cells; leaves are lifted at seed time (mirroring `BareNp`).
    ModLift,
    /// Elided-`than` standard defaulting (D63 §8.12): a comparative awaiting its `than` complement,
    /// `X / cat_pp_than` → `X` with the standard bound to the anaphoric placeholder (`elided_than`),
    /// an OPEN parse. Replaces the `more_deg_bare`/`less_deg_bare` lexical entries.
    ElidedThan,
}

/// A **composed-cell unary shift** (Phase 2c/2d): its [`UnaryKind`] tag and how it applies to one
/// cell item. The ordered table [`unary_shifts`] is the single source of truth all three shift sites
/// consume — the unpacked CKY (extends the cell), the packed forest builder (adds `Edge::Unary`), and
/// `materialize_unary` (re-applies per item at extraction) — replacing the former triplicated
/// orchestration. `AbsorbComma` is NOT here: it is a sentence-initial cross-cell special case both
/// drivers keep inline.
pub(crate) struct UnaryShift {
    pub(crate) kind: UnaryKind,
    /// Rule identity — tracing / on-chain naming; carried, not consumed at runtime.
    #[allow(dead_code)]
    name: &'static str,
    apply: UnaryApply,
}

/// Apply a shift to ONE cell item, given the cell span (for hole freshening). Every shift is
/// **per-item-independent** — `raise_nps`/`bare_nominal_shifts` map each item alone — so applying the
/// shift item-by-item (packed) equals applying it to the whole cell at once (unpacked).
type UnaryApply = fn(&Grammar, &Item, (usize, usize)) -> Vec<Item>;

impl UnaryShift {
    pub(crate) fn run(&self, g: &Grammar, it: &Item, span: (usize, usize)) -> Vec<Item> {
        (self.apply)(g, it, span)
    }
}

/// The ordered composed-cell shift table (all `const`). **ORDER IS LOAD-BEARING**: modifier-lift,
/// coordination completion, elided-`than` completion, bare-nominal, then type-raise (so the raise sees
/// the shifted NPs), then fronted participial — matching the CKY. Both drivers iterate this in order,
/// each shift reading the cell state left by the previous. Elided-`than` sits after coordination and
/// before the NP shifts: its input `X/cat_pp_than` and output `S[adj]\NP` are untouched by the others,
/// so its position only keeps it out of `ModLift` (no attributive comparative modifier is minted).
static UNARY_SHIFTS: [UnaryShift; 6] = [
    UnaryShift {
        kind: UnaryKind::ModLift,
        name: "mod_lift",
        apply: apply_mod_lift,
    },
    UnaryShift {
        kind: UnaryKind::CoordComplete,
        name: "coord_complete",
        apply: apply_coord_complete,
    },
    UnaryShift {
        kind: UnaryKind::ElidedThan,
        name: "elided_than",
        apply: apply_elided_than,
    },
    UnaryShift {
        kind: UnaryKind::BareNp,
        name: "bare_np",
        apply: apply_bare_np,
    },
    UnaryShift {
        kind: UnaryKind::Raise,
        name: "raise",
        apply: apply_raise,
    },
    UnaryShift {
        kind: UnaryKind::FrontParticipial,
        name: "front_participial",
        apply: apply_front_participial,
    },
];

pub(crate) fn unary_shifts() -> &'static [UnaryShift] {
    &UNARY_SHIFTS
}

/// Coordination list-completion: fold a prop-ending `cat_coord` into its base category.
fn apply_coord_complete(g: &Grammar, it: &Item, _span: (usize, usize)) -> Vec<Item> {
    complete_coord(it.cat(), it.sem(), &g.layer)
        .map(|(cat, sem)| Item::with_cost(cat, sem, it.cost()))
        .into_iter()
        .collect()
}

/// Bare-nominal shift: a plural/mass `cat_n` → the copula kind-subject edge + raised bare-argument NPs.
fn apply_bare_np(g: &Grammar, it: &Item, _span: (usize, usize)) -> Vec<Item> {
    g.bare_nominal_shifts(it)
}

/// Elided-`than` standard defaulting: a comparative `X / cat_pp_than` → `X` with the standard bound to
/// the anaphoric placeholder, freshened to this span (`$anaphor$i_j`) exactly as a pronoun's hole — so
/// the completed clause is an OPEN parse the D64 resolver fills.
fn apply_elided_than(g: &Grammar, it: &Item, span: (usize, usize)) -> Vec<Item> {
    let (i, j) = span;
    elided_than(it.cat(), it.sem(), &g.layer)
        .map(|(cat, sem)| Item::with_cost(cat, freshen_anaphor(&sem, &hole_base(i, j)), it.cost()))
        .into_iter()
        .collect()
}

/// Pre-nominal modifier lift on COMPOSED cells: an adjective → `cat_mod` (`mod_lifts`), plus a
/// transitive past participle → a reduced-passive `cat_mod` (`participial_lifts`). Ungated here — a
/// composed span has no single surface to check for an adjective sibling — so the participial's cost
/// penalty is what bounds it; the leaf gate lives in `parse::seed` (adjective-present ⇒ suppressed).
fn apply_mod_lift(_g: &Grammar, it: &Item, _span: (usize, usize)) -> Vec<Item> {
    let mut v = super::combinators::mod_lifts(it);
    v.extend(super::combinators::participial_lifts(it));
    v
}

/// Forward bounded type-raise: a name `NP` → `S/(S\NP)`.
fn apply_raise(g: &Grammar, it: &Item, _span: (usize, usize)) -> Vec<Item> {
    g.raise_nps(std::slice::from_ref(it))
}

/// Fronted participial adjunct: a subject-gapped `ger` VP → a sentence pre-modifier `S/S`, its
/// controlled-subject hole freshened to this span.
fn apply_front_participial(g: &Grammar, it: &Item, span: (usize, usize)) -> Vec<Item> {
    let (i, j) = span;
    front_participial(it.cat(), it.sem(), &g.layer)
        .map(|(cat, sem)| Item::with_cost(cat, freshen_anaphor(&sem, &hole_base(i, j)), it.cost()))
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Escape-hatch invariant** (Phase 2c). The rules that declare `reads_sem` must be exactly the
    /// ones whose DECISION consults the sem — `Coordinate` and `ButNot` — the two the packing
    /// `super::super::chart::forest::Sig` carries a coordination bit for. Adding a sem-reading rule
    /// without extending `Sig` (and this declaration) would be unsound; this test catches it.
    #[test]
    fn escape_hatch_matches_sig() {
        let sem_reading: Vec<BinKind> = bin_rules()
            .iter()
            .filter(|d| d.reads_sem)
            .map(|d| d.kind)
            .collect();
        assert_eq!(
            sem_reading,
            vec![BinKind::Coordinate, BinKind::ButNot],
            "only Coordinate and ButNot read the sem in their firing decision"
        );
    }

    /// Every `BinKind` a trigger can tag a site with has exactly one descriptor, so
    /// `apply_bin_rule`'s discriminant lookup never misses.
    #[test]
    fn every_kind_has_exactly_one_descriptor() {
        for kind in [
            BinKind::Relativize,
            BinKind::Coordinate,
            BinKind::ButNot,
            BinKind::Reciprocal,
            BinKind::AppositiveSubj,
            BinKind::AppositiveObj,
            BinKind::ApposeGroup,
        ] {
            let n = bin_rules().iter().filter(|d| d.kind == kind).count();
            assert_eq!(n, 1, "exactly one descriptor per BinKind: {kind:?}");
        }
    }
}
