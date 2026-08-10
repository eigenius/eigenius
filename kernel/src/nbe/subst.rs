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

//! **Capture-avoiding substitution on `Exp`** (D66 slice 2).
//!
//! Instantiating a chain-resident definition means peeling its leading `Lam`s and substituting the
//! call's arguments into the body — *without* forming a β-redex, so the result stays normal and the
//! two ends of the witness key keep agreeing (D66 D8/D9). That needs a real substitution, and the
//! kernel had none: the only one in the tree is `beta_normalize`'s private helper
//! (`dcg/rules/combinators.rs`), which is **deliberately partial** — it declines to reduce when the
//! argument shares a name with a binder in the body rather than freshening, because it only feeds a
//! sort key where a missed reduction costs nothing. Decode cannot fail soft that way: a declined
//! substitution leaves a redex and silently breaks the hash agreement.
//!
//! ## Why this is restricted to a fragment, and refuses outside it
//!
//! `Exp` has forty-odd variants. A definition body, however, arrives by decoding a stored
//! `eigentt:TypeExpr`, whose eighteen constructors cover a much smaller shape. Two ways to write
//! this were available and both are wrong:
//!
//! - **A catch-all `other => other.clone()`**, the pattern `freshen_anaphor` and `abstract_class`
//!   use. For *those* it is fine — they rewrite constructs they know occur. Here it would mean a
//!   variant nobody thought about silently passes through **unsubstituted**, producing a term that
//!   looks fine and hashes wrong. That is the worst possible failure for this function.
//! - **All forty arms**, most of them unreachable, each an opportunity to get a binder case subtly
//!   wrong.
//!
//! So: the fragment is handled exhaustively, and everything else returns
//! [`SubstError::OutsideFragment`]. A definition body containing something unexpected is then a
//! loud, located error at commit rather than a silent hash change later.
//!
//! ## Capture
//!
//! Arguments at an instantiation site are closed chain terms, so capture cannot arise in practice.
//! [`subst`] still checks, and **errors** rather than freshening. Freshening would be the more
//! general choice, but it renames binders, and binder names reach the witness key through
//! `alpha_canonicalize_proposition_json`; refusing keeps this function's output a pure function of
//! its inputs with no naming policy of its own. If a real need for capture ever appears, freshening
//! can be added — deliberately, with the canonicalization interaction thought through.

use crate::nbe::term::{Exp, Patt};
use std::collections::BTreeSet;

/// Why a substitution could not be performed. Both variants are commit-time errors, never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstError {
    /// The term contains an `Exp` variant a definition body may not contain. Carries the variant
    /// name so the diagnostic points at the construct rather than at the whole term.
    OutsideFragment(&'static str),
    /// A binder in the body would capture a free variable of the argument. Cannot occur with the
    /// closed arguments an instantiation site supplies; reported rather than silently freshened.
    Capture { binder: String, variable: String },
}

impl std::fmt::Display for SubstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideFragment(v) => write!(
                f,
                "`{v}` is outside the fragment a definition body may contain; \
                 substitution refuses rather than leaving it unsubstituted"
            ),
            Self::Capture { binder, variable } => write!(
                f,
                "binder `{binder}` would capture the argument's free variable `{variable}`"
            ),
        }
    }
}

impl std::error::Error for SubstError {}

/// The free variables of `e`, for the capture check.
///
/// Restricted to the same fragment as [`subst`] — a variant outside it contributes nothing, which is
/// safe here because [`subst`] refuses such a term before the result could matter.
pub fn free_vars(e: &Exp) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_free(e, &mut BTreeSet::new(), &mut out);
    out
}

fn collect_free(e: &Exp, bound: &mut BTreeSet<String>, out: &mut BTreeSet<String>) {
    let under =
        |p: &Patt, sub: &Exp, bound: &mut BTreeSet<String>, out: &mut BTreeSet<String>| match p {
            Patt::Var(n) => {
                let fresh = bound.insert(n.to_string());
                collect_free(sub, bound, out);
                if fresh {
                    bound.remove(n.as_str());
                }
            }
            _ => collect_free(sub, bound, out),
        };
    match e {
        Exp::Var(n) => {
            if !bound.contains(n.as_str()) {
                out.insert(n.to_string());
            }
        }
        Exp::Lam(p, b) => under(p, b, bound, out),
        Exp::Pi(p, d, b) | Exp::Sig(p, d, b) => {
            collect_free(d, bound, out);
            under(p, b, bound, out);
        }
        Exp::App(a, b) | Exp::Ann(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) => {
            collect_free(a, bound, out);
            collect_free(b, bound, out);
        }
        Exp::Fst(a) | Exp::Snd(a) | Exp::Refl(a) => collect_free(a, bound, out),
        Exp::Id(a, b, c) => {
            for x in [a, b, c] {
                collect_free(x, bound, out);
            }
        }
        Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
            for x in args {
                collect_free(x, bound, out);
            }
        }
        _ => {}
    }
}

/// Replace every free occurrence of `var` in `body` with `arg`.
///
/// Capture-avoiding by refusal (see the module docs). Exhaustive over the definition-body fragment;
/// [`SubstError::OutsideFragment`] for anything else, never a silent pass-through.
pub fn subst(body: &Exp, var: &str, arg: &Exp) -> Result<Exp, SubstError> {
    subst_inner(body, var, arg, &free_vars(arg))
}

fn subst_inner(
    body: &Exp,
    var: &str,
    arg: &Exp,
    arg_free: &BTreeSet<String>,
) -> Result<Exp, SubstError> {
    let go = |e: &Exp| subst_inner(e, var, arg, arg_free);
    // A binder either shadows `var` — in which case the body below is untouched — or must not
    // capture one of the argument's free variables.
    let under = |p: &Patt, sub: &Exp| -> Result<Exp, SubstError> {
        if let Patt::Var(n) = p {
            if n.as_str() == var {
                return Ok(sub.clone());
            }
            if arg_free.contains(n.as_str()) {
                return Err(SubstError::Capture {
                    binder: n.to_string(),
                    variable: n.to_string(),
                });
            }
        }
        subst_inner(sub, var, arg, arg_free)
    };
    Ok(match body {
        Exp::Var(n) if n.as_str() == var => arg.clone(),
        Exp::Var(_) => body.clone(),

        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(under(p, b)?)),
        Exp::Pi(p, d, b) => Exp::Pi(p.clone(), Box::new(go(d)?), Box::new(under(p, b)?)),
        Exp::Sig(p, d, b) => Exp::Sig(p.clone(), Box::new(go(d)?), Box::new(under(p, b)?)),

        Exp::App(a, b) => Exp::App(Box::new(go(a)?), Box::new(go(b)?)),
        Exp::Ann(a, b) => Exp::Ann(Box::new(go(a)?), Box::new(go(b)?)),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(go(a)?), Box::new(go(b)?)),
        Exp::Times(a, b) => Exp::Times(Box::new(go(a)?), Box::new(go(b)?)),
        Exp::Pair(a, b) => Exp::Pair(Box::new(go(a)?), Box::new(go(b)?)),
        Exp::Fst(a) => Exp::Fst(Box::new(go(a)?)),
        Exp::Snd(a) => Exp::Snd(Box::new(go(a)?)),
        Exp::Refl(a) => Exp::Refl(Box::new(go(a)?)),
        Exp::Id(a, b, c) => Exp::Id(Box::new(go(a)?), Box::new(go(b)?), Box::new(go(c)?)),

        Exp::InductiveType(d, args) => Exp::InductiveType(
            d.clone(),
            args.iter().map(&go).collect::<Result<Vec<_>, _>>()?,
        ),
        Exp::InductiveCtor(d, n, args) => Exp::InductiveCtor(
            d.clone(),
            n.clone(),
            args.iter().map(&go).collect::<Result<Vec<_>, _>>()?,
        ),

        // Leaves — nothing to substitute into.
        Exp::Sort(_)
        | Exp::One
        | Exp::Unit
        | Exp::EigonClass(_)
        | Exp::EigonAxiom(_)
        | Exp::EigonPrimitive(_)
        | Exp::EigonResource(_)
        | Exp::LitString(_)
        | Exp::LitInt(_)
        | Exp::LitFloat(_) => body.clone(),

        // Everything else is outside the fragment. Refusing is the point — see the module docs.
        other => return Err(SubstError::OutsideFragment(variant_name(other))),
    })
}

/// The variant's name, for the diagnostic. Only ever called on the refusal path.
fn variant_name(e: &Exp) -> &'static str {
    match e {
        Exp::Con(..) => "Con",
        Exp::Data(..) => "Data",
        Exp::Case(..) => "Case",
        Exp::Dec(..) => "Dec",
        Exp::IdJ(..) => "IdJ",
        Exp::NativeDecide(..) => "NativeDecide",
        Exp::DecEq(..) => "DecEq",
        Exp::PropAccess(..) => "PropAccess",
        Exp::Template(..) => "Template",
        Exp::Construct(..) => "Construct",
        Exp::Codata(..) => "Codata",
        Exp::CoRecord(..) => "CoRecord",
        Exp::Observe(..) => "Observe",
        Exp::Map(..) => "Map",
        Exp::Reduce(..) => "Reduce",
        Exp::Inductive(..) => "Inductive",
        Exp::InductiveRec { .. } => "InductiveRec",
        Exp::Match { .. } => "Match",
        Exp::SizeSort => "SizeSort",
        Exp::SizeSucc(..) => "SizeSucc",
        Exp::SizeInf => "SizeInf",
        Exp::CodataType(..) => "CodataType",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    fn v(n: &str) -> Exp {
        Exp::Var(n.into())
    }
    fn cls(n: &str) -> Exp {
        Exp::EigonClass(Iri::parse(n).unwrap())
    }
    const WRN: &str = "urn:eigenius:demo:cls:WRN";

    #[test]
    fn replaces_a_free_occurrence() {
        assert_eq!(subst(&v("x"), "x", &cls(WRN)).unwrap(), cls(WRN));
    }

    #[test]
    fn leaves_other_variables_alone() {
        assert_eq!(subst(&v("y"), "x", &cls(WRN)).unwrap(), v("y"));
    }

    #[test]
    fn descends_into_applications() {
        let body = Exp::App(Box::new(v("f")), Box::new(v("x")));
        let out = subst(&body, "x", &cls(WRN)).unwrap();
        assert_eq!(out, Exp::App(Box::new(v("f")), Box::new(cls(WRN))));
    }

    /// A binder of the same name shadows: the body below it must NOT be substituted.
    #[test]
    fn a_shadowing_binder_stops_substitution() {
        let body = Exp::Sig(
            Patt::Var("x".into()),
            Box::new(Exp::Sort(1)),
            Box::new(v("x")),
        );
        assert_eq!(subst(&body, "x", &cls(WRN)).unwrap(), body);
    }

    /// …but the binder's *domain* is outside its own scope and must still be substituted.
    #[test]
    fn a_shadowing_binder_does_not_protect_its_own_domain() {
        let body = Exp::Sig(Patt::Var("x".into()), Box::new(v("x")), Box::new(v("x")));
        let out = subst(&body, "x", &cls(WRN)).unwrap();
        let Exp::Sig(_, dom, inner) = out else {
            panic!("shape preserved")
        };
        assert_eq!(*dom, cls(WRN), "the domain is substituted");
        assert_eq!(*inner, v("x"), "the body is shadowed");
    }

    #[test]
    fn substitutes_under_a_non_shadowing_binder() {
        let body = Exp::Sig(
            Patt::Var("y".into()),
            Box::new(Exp::Sort(1)),
            Box::new(v("x")),
        );
        let out = subst(&body, "x", &cls(WRN)).unwrap();
        let Exp::Sig(_, _, inner) = out else {
            panic!("shape preserved")
        };
        assert_eq!(*inner, cls(WRN));
    }

    /// Capture is refused, not silently performed and not silently declined.
    #[test]
    fn capture_is_an_error() {
        let body = Exp::Sig(
            Patt::Var("y".into()),
            Box::new(Exp::Sort(1)),
            Box::new(v("x")),
        );
        let err = subst(&body, "x", &v("y")).expect_err("y would be captured");
        assert!(matches!(err, SubstError::Capture { .. }), "{err}");
    }

    /// The refusal that makes this safe: a variant outside the fragment errors rather than passing
    /// through unsubstituted. A silent pass-through would produce a wrong hash that looks right.
    #[test]
    fn a_variant_outside_the_fragment_is_refused() {
        let body = Exp::Observe(Box::new(v("x")), "head".into());
        let err = subst(&body, "x", &cls(WRN)).expect_err("Observe is outside the fragment");
        assert_eq!(err, SubstError::OutsideFragment("Observe"));
    }

    #[test]
    fn free_vars_respects_binders() {
        let e = Exp::Sig(
            Patt::Var("x".into()),
            Box::new(v("a")),
            Box::new(Exp::App(Box::new(v("x")), Box::new(v("b")))),
        );
        let fv = free_vars(&e);
        assert!(fv.contains("a") && fv.contains("b"));
        assert!(!fv.contains("x"), "x is bound in the body");
    }
}
