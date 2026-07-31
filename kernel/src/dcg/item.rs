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
//! **The parse item** — the datum every stage of the engine passes around.
//!
//! An [`Item`] is a constituent: a categorial [`CategoryPayload`] (the `lexicon:Cat` term, the producing
//! [`Combinator`], and the additive [`Cost`] rank key) paired with a [`SemanticPayload`] (the assembled
//! EigenTT term). The split is load-bearing, not cosmetic: the combination rules receive only the
//! CATEGORY payload, so they *cannot* branch on a sem — which is the compile-time guarantee that makes
//! the packed forest's `(cat_shape, prov)` signature sound.
//!
//! This is DATA, and it lives on its own because everything uses it: the rules build items, the chart
//! stores them, the lexicon seeds them, and the felicity gate judges them. It used to live in
//! `parser.rs` alongside the combinators — which meant every module that merely needed to *hold* an item
//! had to import the module that *composes* them.

use crate::nbe::term::Exp;

/// The combinator that produced a constituent — its **provenance**, tracked so the
/// **Eisner normal form** (D63 §8.5 Slice 5c, §8.9 Slice 6-T) can constrain a
/// derivation by how its inputs were built. ENF's forward constraint keys on
/// `ForwardComp` (a `>B` output may not be the primary functor of a subsequent
/// `>` / `>B`) and on `TypeRaised` (a raised functor may only *compose*, never
/// *apply*).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Combinator {
    /// Forward application (`>`) or the dependent `cat_forall` application.
    ForwardApp,
    /// Backward application (`<`).
    BackwardApp,
    /// Forward composition (`>B`) — the one ENF's forward constraint blocks as a functor.
    ForwardComp,
    /// Backward harmonic composition (`<B`, combinatory-core spike): `Y\Z · X\Y → X\Z`.
    BackwardComp,
    /// Crossed composition (`>Bx` / `<Bx`, combinatory-core spike): `A/B · B\C → A\C` and
    /// `Y/Z · X\Y → X/Z`. Like `ForwardComp`, an ENF-constrained functor (may not be the primary
    /// of a subsequent application/composition).
    CrossedComp,
    /// Forward bounded **type-raising** (`T`, D63 §8.9 Slice 6-T): an `NP_X` raised to
    /// `S/(S\NP_X)`. ENF blocks it from forward *application* — a raised functor may
    /// only *compose* (`>B`), which is what builds the object-extraction `S/NP` body
    /// of a relative clause. This kills the spurious `T`-application duplicate of plain
    /// backward application, keeping declaratives single-parse (the regression gate).
    TypeRaised,
    /// A **nominal-modification** step (N-N compound / named-entity compound / PP-noun-modifier /
    /// attributive-adjective refinement) — every one builds a refined noun `cat_n(Σ…)`. Carried so
    /// [`apply`] can add a per-step **cost penalty** ([`COMPOUND_STEP_PENALTY`]): summed by the
    /// combinators, a DEEP noun-pile (many modification steps) then costs strictly more than the
    /// shallow correct parse, so the beam / forest cap keeps the real reading and thins the pile
    /// (GH#97 — the content-noun-compound explosion the cross-POS prune can't touch). ENF-inert.
    Compound,
    /// A **bare mass/plural kind-raise** (`kind_raised_nps`): a bare `cat_n` shifted to a
    /// determiner-form GQ denoting its kind. ENF-inert (may forward-apply, unlike `TypeRaised`).
    /// Carried so the attributive rule can REFUSE it as a pre-nominal modifier: its predicative
    /// `S[adj]\NP` form is for argument/predication slots, not to modify a noun — consuming it there
    /// is the bare-mass `And` over-generation (`experiments/parsing/near-encoded-bucket-analysis.md`).
    KindRaised,
    /// A **modal / do-support auxiliary output** — a finite VP built by applying a functor of shape
    /// `(S[dcl,fin]\NP)/(S[dcl,bse]\NP)` (a base VP → finite VP; `can`/`may`/…, and the declarative
    /// do-support). Carried so a **VP-adjunct PP may not attach ABOVE it**: the modal wraps its VP in
    /// `Possible(…)`, and an adjunct attaching to `can arise` (rather than the bare `arise`) escapes
    /// that scope — `And(Possible(arise), prep_from(…))` instead of `Possible(And(arise, prep_from(…)))`,
    /// which wrongly asserts the PP as fact ("adjuncts attach below auxiliaries",
    /// `experiments/parsing/near-encoded-bucket-analysis.md`). The `backward_app` guard
    /// [`ProvGuard::LeftNotModal`] refuses this modal-tagged VP as a backward-application ARGUMENT (the
    /// VP-adjunct case) while leaving subject application untouched (there the argument is the subject
    /// NP, not the modal VP). ENF-inert.
    Modal,
    /// A **close-apposition output** (`appose_group`): a designator group whose members the classifier
    /// has already been distributed over. It is a NORMAL FORM, and the tag is what enforces that —
    /// carried so `build_appose_group` can refuse it as a further apposition's group (a second
    /// classifier would nest a definite designation inside `named`, whose second argument is a naming
    /// TOKEN and never a description) and `build_coordinate` can refuse it as a conjunct (a member
    /// appended afterwards would escape the classifier's scope, so only a prefix of the list would be
    /// classified). Both re-applications were INVISIBLE while the rule passed the group through
    /// unchanged, because re-applying an identity is an identity; distributing made them
    /// term-distinct and they multiplied — the reference page's germline unit went 112 → 497
    /// skeletons, with up to 8 `named`s over 4 designators. ENF-inert.
    Apposed,
    /// A **derived individual** — an entity reached by COERCION rather than denoted by a name. Two
    /// producers, and they are one notion:
    /// - a **definite designation** (`definite_designation`), `the(Σx:C. named(x, d)).1`: the
    ///   individual a naming-refined noun uniquely picks out — description-derived;
    /// - a **kind realization**, `kind_of(K)`: a bare `cat_n` conjunct made argument-fillable by
    ///   `coordinate_np`'s `np_conjunct` — kind-derived.
    ///
    /// Either way the category is an ordinary `cat_np(C, num)`, indistinguishable from a proper name's,
    /// so only provenance can tell them apart — the same reason the `name` rule needs `NotKindRaised`
    /// alongside `ProperName`.
    ///
    /// Carried so a derived individual cannot be a **DESIGNATOR**. `ontology:named`'s second argument is
    /// a naming TOKEN: not a description, so `named(x, the(Σy:C. named(y, d)).1)` — "the gene named the
    /// gene named MSH2" — is not a naming; and not a kind, which is the same thing the singular path
    /// already refuses ("nucleotide repeat regions" read as a nucleotide *named* a repeat region). The
    /// designation case arises because one string admits two bracketings: "[the MMR genes] [MSH2, …]"
    /// apposes plain names, while "[the MMR] [genes MSH2, …]" designates "genes MSH2" and apposes THAT.
    /// Refusing the second keeps the first, so no span loses its analysis.
    ///
    /// **Propagated through coordination** (`build_coordinate`): a group inherits the tag from any
    /// conjunct that is derived, which is what lets `appose_group` refuse an impure designator group by
    /// reading provenance — a property `Sig` already carries — instead of the member sems, which would
    /// make its firing decision sem-dependent and need a new packing bit. Coordinating derived
    /// individuals is untouched ("the gene MSH2 and the gene MSH6", "MSI and MMR deficiency create
    /// vulnerabilities"); only classifying them AGAIN is blocked. ENF-inert.
    DerivedIndividual,
    /// A **seed-time oblique participial lift** (`oblique_participial_lifts`): a post-nominal modifier
    /// still awaiting its PP argument, `cat_pp/cat_pp_arg(P)`.
    ///
    /// **ENF-constrained, and the exact mirror of [`Self::TypeRaised`]** — that one may only *compose*,
    /// never forward-apply; this one may only *forward-apply*, never be the primary of a composition
    /// ([`ProvGuard::LeftNotObliqueParticipial`] on `forward_comp`). The reason is the same: the other
    /// route re-derives a reading plain application already gives. Composing the lift with the
    /// preposition and then applying to the object,
    ///
    /// ```text
    /// [cat_pp/cat_pp_arg] · [cat_pp_arg/NP] -> [cat_pp/NP],  then · [NP] -> cat_pp
    /// ```
    ///
    /// yields the same `cat_pp` over the same span with the same sem as applying the lift directly to
    /// the assembled `cat_pp_arg`. Measured on "… the top preferential dependency in MSI cell lines
    /// compared to MSS cell lines" (2026-07-27): the span `[16..20]` carried FOUR derivations of
    /// `cat_pp` across two nodes (the routes split by provenance, so `Sig` keeps them apart) where the
    /// shift route it replaced carried one.
    ObliqueParticipial,
    /// A **scope-bearing operator's lexical leaf** — an entry declaring `lexicon:scope_bearing`
    /// (sentential negation `not`, the modals, do-support). Set once at seeding by
    /// [`crate::dcg::lexicon::entry_to_item`]; the tag exists so [`Self::Modal`] can be put on the
    /// operator's OUTPUT without the combinator having to sniff the category or read a sem.
    ///
    /// The distinction it encodes is core-en's. There, negation is an auxiliary-verb family
    /// (`auxv.xsl`, `family name="Negation" pos="V" closed="true"`) whose category
    /// `(s.1.from-6.E\np)/(s.6.E2\np)` derives a NEW situation index from its argument's, while an
    /// adverb (`adv.xsl`, `pos="Adv"`) is `s.1.E\s.1.E` with LF `HasProp(E, P)` — the SAME index,
    /// decorated. A VP-adjunct attaching above an index-preserving modifier lands on the same event
    /// and is the same claim; above an index-SHIFTING operator it lands on the outer index and
    /// escapes the embedded one. Only the latter must be refused.
    ///
    /// A category test cannot draw that line here: `not_adj` is `fwd(VP[adj], VP[adj])`, which is
    /// byte-identical to the adverb adjective-modifier category ([`crate::dcg::category`]'s
    /// `adverb_modifier_cats`). Hence the property is DECLARED on the entry rather than inferred,
    /// and read once — a combine-time decision then reads only provenance, which `Sig` already
    /// carries, so no packing bit is added (the constraint [`Self::DerivedIndividual`] documents).
    ///
    /// ENF-inert: it must still forward-apply to its VP, so no `ProvGuard` refuses it.
    ScopeOperator,
    /// Any other producer (lexical leaf, coordination, group/distributive rules) —
    /// not a composition output, so ENF never constrains it.
    Other,
}

/// Per-step cost penalty for a nominal-modification ([`Combinator::Compound`]) output (GH#97). Added
/// to `Cost::sense_rank`, which is summed across a parse's steps — so cost grows with modification
/// DEPTH, ranking a deep noun-pile below the shallow correct parse. Small enough not to disturb the
/// lexicon-order primary key; large enough to dominate per-leaf sense-rank noise at a few steps.
pub const COMPOUND_STEP_PENALTY: u32 = 8;

/// The 2-component additive **rank key** for a parse (D65 §4.2): lexicon
/// precedence (primary) then sense-frequency (secondary). The combinators **sum**
/// both components across a parse's leaves; the forest sorts **lexicographically**
/// by `(lexicon_order, sense_rank)` then caps. Derived `Ord` compares fields in
/// declaration order, giving exactly that lexicographic order.
///
/// The unordered, single-lexicon default leaves `lexicon_order = 0` everywhere —
/// behaviour-identical to the prior scalar `sense_rank` cost (D63 §8.7 Stage B).
/// The kernel never learns either component *means* anything — it sums opaque
/// weights, keeping the engine sense-/lexicon-agnostic (the §6 boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cost {
    /// Σ of each leaf's position in the parse scope's ordered lexicon list
    /// (0 = first / most-preferred; 0 for the unordered default). Primary key.
    pub lexicon_order: u32,
    /// Σ of each leaf's `lexicon:sense_rank` (0 = most-frequent sense). Secondary key.
    pub sense_rank: u32,
}

impl Cost {
    /// The zero cost — the default for closed-class / unranked / unscoped leaves.
    pub const ZERO: Cost = Cost {
        lexicon_order: 0,
        sense_rank: 0,
    };

    /// A leaf cost from just a sense-frequency rank (`lexicon_order = 0`); the
    /// lexical index stamps this, and the scope (if any) overwrites `lexicon_order`.
    pub fn from_sense_rank(sense_rank: u32) -> Cost {
        Cost {
            lexicon_order: 0,
            sense_rank,
        }
    }

    /// Component-wise saturating sum — how the combinators aggregate child costs.
    pub fn saturating_add(self, other: Cost) -> Cost {
        Cost {
            lexicon_order: self.lexicon_order.saturating_add(other.lexicon_order),
            sense_rank: self.sense_rank.saturating_add(other.sense_rank),
        }
    }
}

/// The **category payload** of a parse item — everything the combination rules
/// ([`apply`]/[`apply_combine`]) are allowed to consume: the category (`lexicon:Cat`
/// term), the producing [`Combinator`] (Eisner normal form), and the additive [`Cost`]
/// rank key. Deliberately **carries no semantics**: the packed-forest design (D63
/// blueprint) requires combinability to be a function of `(category, prov)` alone, so
/// the sem is segregated into [`SemanticPayload`] and never reachable from a rule that
/// only holds a `CategoryPayload`. This is the compile-time form of the "postcondition
/// vs. carry" separation (Hopkins & Langmead 2009).
#[derive(Clone)]
pub struct CategoryPayload {
    pub cat: Exp,
    pub prov: Combinator,
    pub cost: Cost,
}

/// The **semantic payload** of a parse item — the assembled EigenTT sem. Segregated
/// from [`CategoryPayload`] so the combination rules cannot branch on it (the
/// packed-forest soundness invariant). In the eventual lazy design (Harper 1994
/// "Method 3") this becomes a deferred procedure call materialised on demand; today it
/// is the eagerly-built term.
#[derive(Clone)]
pub struct SemanticPayload {
    pub sem: Exp,
}

/// A parse item: a [`CategoryPayload`] (category + provenance + cost — all combination
/// consumes) paired with its [`SemanticPayload`] (the sem — never seen by combination).
/// A leaf's cost is set by whoever builds it (the lexical index from the entry's
/// `sense_rank`, the parse scope from the entry's lexicon position); the kernel only
/// sums opaque weights, staying sense-/lexicon-agnostic (the §6 forest-returns boundary).
#[derive(Clone)]
pub struct Item {
    pub category: CategoryPayload,
    pub semantics: SemanticPayload,
}

/// Arrow depth of `⟦cat⟧` — the number of arguments a category declares.
fn cat_arity_of(c: &Exp) -> Option<usize> {
    let mut t = crate::dcg::category::denote_cat(c).ok()?;
    let mut n = 0;
    while let Exp::Arrow(_, cod) | Exp::Pi(_, _, cod) = t {
        t = *cod;
        n += 1;
    }
    Some(n)
}

/// Leading-λ count of a sem's VALUE — the number of arguments it actually takes.
fn sem_arity_of(sem: &Exp) -> Option<usize> {
    let mut v = crate::nbe::eval::eval(sem, &crate::nbe::env::Rho::Nil).ok()?;
    let mut n = 0;
    while let crate::nbe::val::Val::Lam(g) = v {
        v = g
            .apply(crate::nbe::val::Val::Nt(crate::nbe::val::Neut::Gen(
                n,
                format!("__ar{n}"),
            )))
            .ok()?;
        n += 1;
        if n > 8 {
            break;
        }
    }
    Some(n)
}

/// The constructor name of a category, for the mismatch probe's one-line output.
fn cat_head(c: &Exp) -> String {
    match c {
        Exp::InductiveCtor(_, n, args) => format!("{n}/{}", args.len()),
        other => format!("{other:?}").chars().take(24).collect(),
    }
}

impl Item {
    /// Assemble an item from its parts — the single constructor the composition rules
    /// and lexical index build through (replacing the old `Item::from_parts(cat, sem, prov, cost)`
    /// literal now that the fields live in two payloads).
    pub fn from_parts(cat: Exp, sem: Exp, prov: Combinator, cost: Cost) -> Self {
        // `EIGENIUS_TRACE_MISMATCH=1` — the CATEGORY/SEM AGREEMENT invariant. Every rule's output
        // must satisfy `sem : ⟦cat⟧`; nothing checks that until the full-span felicity gate, so a
        // rule may mint an item whose category says `Prop` while its sem is still an abstraction.
        // Categories then agree at every subsequent step (which is why every rule fires) and only the
        // gate objects — reporting an unexplained grammar-gap far from the cause.
        //
        // This prints the PROVENANCE, so the offending rule names itself.
        // EXCLUDES a hole abstraction. An unresolved referent is a free variable that the felicity
        // gate's OPEN path abstracts into a TYPED PARAMETER and checks against a Π-type, so an item
        // with a `cat_s` category and a `λ$anaphor$…` sem is a legitimate open parse, not a mismatch.
        // Without this the probe reported ~3128 false positives against 30 real ones on the WRN page.
        if std::env::var("EIGENIUS_TRACE_MISMATCH").is_ok()
            && !matches!(&sem, Exp::Lam(crate::nbe::term::Patt::Var(v), _) if v.starts_with("$anaphor$"))
            && matches!(sem, Exp::Lam(..))
            && matches!(crate::dcg::category::denote_cat(&cat), Ok(Exp::Sort(0)))
        {
            let shape = match &sem {
                Exp::Lam(p, b) => format!(
                    "λ{p:?}. {}",
                    match b.as_ref() {
                        Exp::Lam(..) => "λ…",
                        Exp::App(..) => "App(…)",
                        Exp::InductiveType(d, _) => d.name.as_str(),
                        _ => "…",
                    }
                ),
                _ => "?".to_string(),
            };
            eprintln!(
                "  !! CAT/SEM MISMATCH prov={prov:?} cat={} sem={shape}",
                cat_head(&cat)
            );
        }
        // `EIGENIUS_TRACE_UNDERAPP=1` — THE CATEGORY/SEM ARITY INVARIANT, on every item.
        //
        // A category declares how many arguments a constituent takes (the arrow depth of `⟦cat⟧`); its
        // sem must take exactly that many. Nothing checks this until the full-span felicity gate, so a
        // rule may mint an item whose sem is over-abstracted; categories then agree at every later
        // step, every rule fires, and only the gate objects — as a type error far from its cause.
        //
        // Tested on the VALUE. An `Exp::App(f, x)` given fewer arguments than `f`'s arity is
        // syntactically an application and only becomes a closure under evaluation, so a syntactic
        // λ-count cannot see the dominant case. Free-variable sems (referent holes) are skipped: they
        // evaluate to a closure over a neutral and are legitimate open parses.
        if std::env::var("EIGENIUS_TRACE_UNDERAPP").is_ok()
            && !matches!(&sem, Exp::Lam(crate::nbe::term::Patt::Var(v), _) if v.starts_with("$anaphor$"))
        {
            if let (Some(ca), Some(sa)) = (cat_arity_of(&cat), sem_arity_of(&sem)) {
                if sa > ca {
                    eprintln!(
                        "  !! ARITY-INVARIANT prov={prov:?} cat={} cat_arity={ca} sem_arity={sa}",
                        cat_head(&cat)
                    );
                }
            }
        }
        Item {
            category: CategoryPayload { cat, prov, cost },
            semantics: SemanticPayload { sem },
        }
    }

    /// A leaf / non-combinatory item (a lexical seed, or any constituent not
    /// produced by a composition rule) — `prov = Other`, cost zero.
    pub fn new(cat: Exp, sem: Exp) -> Self {
        Self::from_parts(cat, sem, Combinator::Other, Cost::ZERO)
    }

    /// Same as [`Item::new`] but with an explicit [`Cost`] — used by the lexical
    /// index to stamp an entry's rank, and by the composition rules that sum costs.
    pub fn with_cost(cat: Exp, sem: Exp, cost: Cost) -> Self {
        Self::from_parts(cat, sem, Combinator::Other, cost)
    }

    /// This item with its cost replaced (preserving cat/sem/prov) — for unary
    /// transforms (type-raise, number refinement) that carry a child's cost through.
    pub(crate) fn at_cost(mut self, cost: Cost) -> Self {
        self.category.cost = cost;
        self
    }

    /// The category term (`&self.category.cat`).
    pub fn cat(&self) -> &Exp {
        &self.category.cat
    }
    /// The assembled sem (`&self.semantics.sem`).
    pub fn sem(&self) -> &Exp {
        &self.semantics.sem
    }
    /// The producing combinator (Eisner normal form provenance).
    pub fn prov(&self) -> Combinator {
        self.category.prov
    }
    /// The additive rank key.
    pub fn cost(&self) -> Cost {
        self.category.cost
    }
    /// Replace the sem in place (the per-span hole freshening).
    pub fn set_sem(&mut self, sem: Exp) {
        self.semantics.sem = sem;
    }
}

#[cfg(test)]
mod tests {
    use super::Cost;

    #[test]
    fn cost_sorts_lexicon_order_before_sense_rank() {
        // D65 §4.2: the rank key is lexicographic — lexicon precedence dominates,
        // sense-frequency tie-breaks within a precedence level.
        let mut v = vec![
            Cost {
                lexicon_order: 1,
                sense_rank: 0,
            }, // preferred lexicon? no — order 1
            Cost {
                lexicon_order: 0,
                sense_rank: 9,
            },
            Cost {
                lexicon_order: 0,
                sense_rank: 1,
            },
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                Cost {
                    lexicon_order: 0,
                    sense_rank: 1
                }, // order 0 beats order 1 …
                Cost {
                    lexicon_order: 0,
                    sense_rank: 9
                }, // … even at a much worse sense_rank
                Cost {
                    lexicon_order: 1,
                    sense_rank: 0
                },
            ]
        );
    }

    #[test]
    fn cost_saturating_add_is_componentwise() {
        let a = Cost {
            lexicon_order: 2,
            sense_rank: 3,
        };
        let b = Cost {
            lexicon_order: 1,
            sense_rank: 4,
        };
        assert_eq!(
            a.saturating_add(b),
            Cost {
                lexicon_order: 3,
                sense_rank: 7
            }
        );
        // Saturates each component independently, no overflow panic.
        let big = Cost {
            lexicon_order: u32::MAX,
            sense_rank: 0,
        };
        assert_eq!(big.saturating_add(a).lexicon_order, u32::MAX);
    }
}
