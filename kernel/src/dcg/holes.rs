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
        // Inductive nodes (e.g. `logic:And(P, Q)` as an `InductiveType`) carry subterms too — a
        // fronted-participial conjunct nests the anaphor inside an `And`, so the freshener must
        // descend into them (else the hole stays an unfreshened closed constant).
        Exp::InductiveType(d, args) => Exp::InductiveType(d.clone(), args.iter().map(go).collect()),
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

/// Freshen every `lexicon:anaphor_of(A)` application in a NORMAL FORM into a typed referent hole:
/// each occurrence is replaced by a fresh variable (`$demref$<k>_0`, traversal order — the
/// skeleton normalizer α-normalizes the `$name$digits_digits` shape, so the naming is
/// drift-free), and `(var, A)` is returned so the gate can carry the hole at the RESTRICTOR type
/// — the kernel re-gate then vetoes a type-wrong antecedent ("these findings" resolves only to
/// findings). Two occurrences in one reading are two distinct subterm sites → two distinct holes
/// (independent referents, resolved independently). Runs post-readback, so a partially-applied
/// `anaphor_of` head cannot occur in a well-typed nf; a bare un-applied constant is left alone
/// (nothing to type the hole with) and the gate's closed `check` rejects the candidate.
pub(super) fn freshen_anaphor_of(exp: &Exp) -> (Exp, Vec<(String, Exp)>) {
    fn walk(e: &Exp, holes: &mut Vec<(String, Exp)>) -> Exp {
        if let Exp::App(f, a) = e {
            if matches!(f.as_ref(), Exp::EigonAxiom(i) if i.as_str() == ANAPHOR_OF_IRI) {
                let var = format!("$demref${}_0", holes.len());
                holes.push((var.clone(), a.as_ref().clone()));
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
            Exp::InductiveType(d, args) => {
                Exp::InductiveType(d.clone(), args.iter().map(|x| walk(x, holes)).collect())
            }
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
