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
