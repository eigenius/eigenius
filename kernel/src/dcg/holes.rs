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
//! **The referent-hole protocol** (D64) — how a hole is NAMED and FRESHENED.
//!
//! A pronoun or possessor seeds a `lexicon:anaphor` placeholder, which becomes a fresh free variable
//! per occurrence. That variable's name is derived from the SPAN it was created on
//! (`$anaphor$<i>_<j>`), which is what makes it stable: a unary shift that rebuilds an item on a
//! different span must re-freshen its holes to the new span, or two distinct referents collide.
//!
//! This is deliberately its own module, because it belongs to no single stage and is used by all of
//! them: `seed` CREATES holes, both chart drivers RE-FRESHEN them when a unary shift moves an item to a
//! new span, `felicity` binds and TYPES them, and `resolve` (D64) finally SUBSTITUTES antecedents for
//! them. It previously lived in `felicity`, which meant the two chart drivers had to reach into the
//! felicity gate for a naming convention — a dependency that says nothing true about the code.

use crate::nbe::term::Exp;

/// The placeholder axiom a pronoun's `sem` carries in the lexicon, before freshening.
const ANAPHOR_IRI: &str = "urn:eigenius:lexicon:anaphor";

/// The placeholder a FACTIVE subordinator's `sem` carries for each of its two proof arguments
/// (D62 2d Fork A). The `Prop` sibling of `anaphor_of`: it stands in the derivation as a closed
/// constant so the entry gate-passes and the term stays a `Prop` through composition, and the
/// felicity gate freshens it into a typed free variable — a [`HoleKind::ProofObligation`].
///
/// [`HoleKind::ProofObligation`]: super::parse::HoleKind::ProofObligation
const HYPOTHESIS_OF_IRI: &str = "urn:eigenius:lexicon:hypothesis_of";

/// Base name of the referent-hole free variable for a pronoun/possessive spanning tokens
/// `[i, j]`. Position-keyed, so distinct occurrences are distinct holes.
pub(super) fn hole_base(i: usize, j: usize) -> String {
    format!("$anaphor${i}_{j}")
}

/// Replace every `lexicon:anaphor` placeholder in `exp` with the free variable `fresh` (the
/// referent-hole freshening, D64). The anaphor is a leaf constant (no binders to capture), so
/// this is a plain structural replace. It appears only in authored pronoun sems (the whole
/// sem) and possessive-determiner sems (nested inside the λ — `poss_of(A, x, anaphor)`); the
/// compound forms those traverse are covered below, and every other form is returned
/// unchanged (no anaphor occurs there).
/// Is this sem a **bare referential pronoun** — exactly the `lexicon:anaphor` placeholder, with
/// nothing wrapped around it?
///
/// EXACTLY, not "contains": the possessive `its` / `their` carries the same axiom NESTED inside the
/// determiner's λ (it is the possessor, not the referent), and the demonstrative DETERMINER carries
/// the restrictor-typed sibling `anaphor_of` instead. Only a bare pronoun — `it`, `they`, and the
/// bare demonstratives — denotes the hole itself. Checked before [`freshen_anaphor`] runs, since
/// after it the placeholder is an ordinary `Exp::Var` indistinguishable from any other.
pub(super) fn is_bare_pronoun_sem(exp: &Exp) -> bool {
    matches!(exp, Exp::EigonAxiom(a) if a.as_str() == ANAPHOR_IRI)
}

pub(super) fn freshen_anaphor(exp: &Exp, fresh: &str) -> Exp {
    let go = |e: &Exp| freshen_anaphor(e, fresh);
    match exp {
        Exp::EigonAxiom(a) if a.as_str() == ANAPHOR_IRI => Exp::Var(fresh.to_string()),
        Exp::App(f, x) => Exp::App(Box::new(go(f)), Box::new(go(x))),
        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(go(b))),
        Exp::Pi(p, a, b) => Exp::Pi(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Sig(p, a, b) => Exp::Sig(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(go(a)), Box::new(go(b))),
        Exp::Times(a, b) => Exp::Times(Box::new(go(a)), Box::new(go(b))),
        Exp::Fst(e) => Exp::Fst(Box::new(go(e))),
        Exp::Snd(e) => Exp::Snd(Box::new(go(e))),
        Exp::Pair(a, b) => Exp::Pair(Box::new(go(a)), Box::new(go(b))),
        Exp::Ann(e, t) => Exp::Ann(Box::new(go(e)), Box::new(go(t))),
        // Constructor nodes carry subterms too, so the freshener must descend into
        // them (else the hole stays an unfreshened closed constant). An inductive
        // *type* application — `logic:And(P, Q)` — is an `App` spine since D76
        // Phase B and is covered by the `App` arm above.
        Exp::InductiveCtor(d, n, args) => {
            Exp::InductiveCtor(d.clone(), n.clone(), args.iter().map(go).collect())
        }
        other => other.clone(),
    }
}

/// The POLYMORPHIC placeholder a demonstrative determiner's `sem` applies to its restrictor
/// (`lexicon:anaphor_of : ∀(A:Set) → A`, `d64-demonstratives-as-holes.md` §2). Unlike the bare
/// pronoun placeholder it cannot be freshened at seed time — the determiner has not met its noun,
/// so the hole's TYPE is unknown; the applied form rides the derivation as a closed constant and
/// is freshened by the felicity gate, where β-reduction has made the restrictor concrete.
const ANAPHOR_OF_IRI: &str = "urn:eigenius:lexicon:anaphor_of";

/// The counted sibling, `lexicon:anaphor_of_count : ∀(A:Set) → Quantity(u"1") → A` — a demonstrative
/// before a numeral (`these two genetic events`, D95 slice 7d, decision 13). Freshened like
/// `anaphor_of(A)`, with its count recorded on the hole.
const ANAPHOR_OF_COUNT_IRI: &str = "urn:eigenius:lexicon:anaphor_of_count";

/// The whole number a count term states — `mk_quantity(n, 0)` at the dimensionless unit, as a
/// numeral seeds it — or `None` for any other term.
pub(crate) fn count_value(q: &Exp) -> Option<usize> {
    match q {
        Exp::Ann(inner, _) => count_value(inner),
        Exp::InductiveCtor(_, ctor, parts) if ctor == "mk_quantity" => match parts.as_slice() {
            [Exp::LitRat(n), Exp::LitInt(0)] if n.is_integer() => usize::try_from(n.numer()).ok(),
            _ => None,
        },
        _ => None,
    }
}

/// A hole [`freshen_gate_holes`] finds in a normal form: the fresh variable, the type it stands at,
/// the count a numeral after a demonstrative states, and WHICH placeholder produced it.
pub(super) struct GateHole {
    pub(super) var: String,
    pub(super) ty: Exp,
    pub(super) count: Option<Exp>,
    /// `true` for a factive subordinator's proof argument (`hypothesis_of`), `false` for a
    /// demonstrative's referent (`anaphor_of`). The two are freshened in ONE traversal because they
    /// have the same shape — a one-argument axiom whose argument IS the hole's type — and differ
    /// only in what resolves them: an antecedent from the chain, or a witness from the
    /// justification layer.
    pub(super) is_proof: bool,
}

/// Freshen every `lexicon:anaphor_of(A)` and `lexicon:hypothesis_of(P)` application in a NORMAL
/// FORM into a typed hole. ONE traversal for both, because they have the same shape — a
/// one-argument axiom whose argument IS the hole's type — and differ only in what resolves them.
///
/// For the referent holes:
/// each occurrence is replaced by a fresh variable (`$demref$<k>_0`, traversal order — the
/// skeleton normalizer α-normalizes the `$name$digits_digits` shape, so the naming is
/// drift-free), and `(var, A)` is returned so the gate can carry the hole at the RESTRICTOR type
/// — the kernel re-gate then vetoes a type-wrong antecedent ("these findings" resolves only to
/// findings). Two occurrences in one reading are two distinct subterm sites → two distinct holes
/// (independent referents, resolved independently). Runs post-readback, so a partially-applied
/// `anaphor_of` head cannot occur in a well-typed nf; a bare un-applied constant is left alone
/// (nothing to type the hole with) and the gate's closed `check` rejects the candidate. A counted
/// occurrence, `anaphor_of_count(A, q)`, returns its count `q` beside the type.
pub(super) fn freshen_gate_holes(exp: &Exp) -> (Exp, Vec<GateHole>) {
    fn walk(e: &Exp, holes: &mut Vec<GateHole>) -> Exp {
        if let Exp::App(f, a) = e {
            let head = |g: &Exp, iri: &str| matches!(g, Exp::EigonAxiom(i) if i.as_str() == iri);
            let (ty, count, is_proof) = if head(f, ANAPHOR_OF_IRI) {
                (Some(a.as_ref().clone()), None, false)
            } else if head(f, HYPOTHESIS_OF_IRI) {
                // `hypothesis_of(P)` — the argument IS the proposition the hypothesis proves, so it
                // types the hole directly, exactly as the restrictor does for `anaphor_of`.
                (Some(a.as_ref().clone()), None, true)
            } else if let Exp::App(g, t) = f.as_ref() {
                if head(g, ANAPHOR_OF_COUNT_IRI) {
                    (Some(t.as_ref().clone()), Some(a.as_ref().clone()), false)
                } else {
                    (None, None, false)
                }
            } else {
                (None, None, false)
            };
            if let Some(ty) = ty {
                let var = if is_proof {
                    format!("$proof${}_0", holes.len())
                } else {
                    format!("$demref${}_0", holes.len())
                };
                holes.push(GateHole {
                    var: var.clone(),
                    ty,
                    count,
                    is_proof,
                });
                return Exp::Var(var);
            }
        }
        match e {
            Exp::App(f, x) => Exp::App(Box::new(walk(f, holes)), Box::new(walk(x, holes))),
            Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(walk(b, holes))),
            Exp::Pi(p, a, b) => Exp::Pi(
                p.clone(),
                Box::new(walk(a, holes)),
                Box::new(walk(b, holes)),
            ),
            Exp::Sig(p, a, b) => Exp::Sig(
                p.clone(),
                Box::new(walk(a, holes)),
                Box::new(walk(b, holes)),
            ),
            Exp::Arrow(a, b) => Exp::Arrow(Box::new(walk(a, holes)), Box::new(walk(b, holes))),
            Exp::Times(a, b) => Exp::Times(Box::new(walk(a, holes)), Box::new(walk(b, holes))),
            Exp::Fst(x) => Exp::Fst(Box::new(walk(x, holes))),
            Exp::Snd(x) => Exp::Snd(Box::new(walk(x, holes))),
            Exp::Pair(a, b) => Exp::Pair(Box::new(walk(a, holes)), Box::new(walk(b, holes))),
            Exp::Ann(x, t) => Exp::Ann(Box::new(walk(x, holes)), Box::new(walk(t, holes))),
            Exp::InductiveCtor(d, n, args) => Exp::InductiveCtor(
                d.clone(),
                n.clone(),
                args.iter().map(|x| walk(x, holes)).collect(),
            ),
            other => other.clone(),
        }
    }
    let mut holes = Vec::new();
    let out = walk(exp, &mut holes);
    (out, holes)
}
