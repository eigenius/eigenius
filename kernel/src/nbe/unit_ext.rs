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

//! The units kernel extension (D93): `units:mul` and `units:pow` reduce.
//!
//! **nanoda's pattern.** Lean's kernel extends arithmetic without new term forms: `Nat.add`,
//! `Nat.mul` and the rest are ordinary declared constants recognised by name, reduced when their
//! arguments are literals (`references/nanoda_lib/src/tc.rs:395-458`, `try_reduce_nat`). The only
//! new term node is the literal. So here `units:mul` and `units:pow` are axioms in `units.esl`,
//! recognised by IRI when their application spine is built (`Val::app_impl`), and `LitUnit` is the
//! literal. Recognising the spine by its `Neut::EigonAxiom` head means the reduction fires only when
//! the chain DECLARES the constant — as nanoda's name cache holds only names the environment has.
//!
//! **Beyond nanoda: open products normalise.** `try_reduce_nat` returns on free variables, so Lean's
//! kernel decides no `x + y ≡ y + x`. D93 requires that for units — Kennedy's normal form covers
//! open terms — so a product with unit-valued neutrals in it is normalised too: a closed part and
//! ATOMS with rational exponents, rebuilt as one canonical spine. Equality then needs nothing new:
//! `eq_nf` reads both sides back and compares the terms, and equal normal forms read back
//! identically.
//!
//! This rests on `units::Unit` being a group. Until D93's kinds became metadata it was not — the
//! product was not associative — and no normal form over it could have been.

use crate::nbe::eval::EvalError;
use crate::nbe::readback::try_readback_val;
use crate::nbe::term::Exp;
use crate::nbe::val::{Neut, Val};
use crate::ontology::iri::Iri;
use crate::ontology::well_known as wk;
use crate::units::{Exponent, Unit, UnitError};

/// The read-back level for atom keys: above every free variable, so a binder INSIDE an atom takes a
/// level no free variable has, and two different atoms cannot read back to one term.
const ATOM_KEY_LEVEL: usize = usize::MAX / 2;

/// One non-literal factor of a product, with its exponent.
struct Atom {
    /// The value itself, which the rebuilt spine carries.
    val: Val,
    /// Its read-back at [`ATOM_KEY_LEVEL`]. Two atoms merge exactly when these are equal — `eq_nf`'s
    /// own notion — so merging can never equate different units.
    term: Exp,
    /// The rendering of `term`, which ORDERS atoms. A collision between different terms could only
    /// leave two equal products in different orders — unrecognised as equal, never wrongly equal.
    key: String,
    exp: Exponent,
}

/// A product in normal form: a closed part and atoms, the atoms merged, non-zero and in key order.
struct UnitNf {
    closed: Unit,
    atoms: Vec<Atom>,
}

/// Reduces `n` when it is a complete `units:mul` or `units:pow` application; `None` leaves it
/// neutral.
///
/// It stays neutral when an argument is not unit-shaped — an ill-typed application, which type
/// checking refuses — and when a `units:pow` exponent is not a literal. A stuck `units:pow` is then
/// an ordinary atom of any product it enters.
pub(crate) fn try_reduce_unit(n: &Neut) -> Result<Option<Val>, EvalError> {
    let Some((op, a, b)) = binary_spine(n) else {
        return Ok(None);
    };
    let nf = match op.as_str() {
        wk::UNITS_MUL => {
            let (Some(x), Some(y)) = (normal_form(a)?, normal_form(b)?) else {
                return Ok(None);
            };
            x.mul(y)
        }
        wk::UNITS_POW => {
            let Val::LitRat(r) = b else {
                return Ok(None);
            };
            let Some(x) = normal_form(a)? else {
                return Ok(None);
            };
            let e = Exponent::from_rational(r).map_err(|err| overflow(err, &format!("^{r}")))?;
            x.pow(e)
        }
        _ => return Ok(None),
    }
    .map_err(|err| overflow(err, op.as_str()))?;
    Ok(Some(nf.into_val()))
}

fn overflow(err: UnitError, what: &str) -> EvalError {
    EvalError::UnitOverflow(format!("{what}: {err}"))
}

/// `f a b` with `f` a declared axiom: the head IRI and both arguments.
fn binary_spine(n: &Neut) -> Option<(&Iri, &Val, &Val)> {
    let Neut::App(inner, b) = n else {
        return None;
    };
    let Neut::App(head, a) = inner.as_ref() else {
        return None;
    };
    let Neut::EigonAxiom(iri) = head.as_ref() else {
        return None;
    };
    Some((iri, a, b))
}

/// The normal form of a unit-valued value; `None` when `v` is not unit-shaped.
///
/// A `units:mul` or `units:pow` spine here is one this module built — every complete application
/// reduces — so it is flattened back into its factors rather than taken as an atom.
fn normal_form(v: &Val) -> Result<Option<UnitNf>, EvalError> {
    match v {
        Val::LitUnit(u) => Ok(Some(UnitNf {
            closed: u.clone(),
            atoms: Vec::new(),
        })),
        Val::Nt(n) => {
            if let Some((op, a, b)) = binary_spine(n) {
                match (op.as_str(), b) {
                    (wk::UNITS_MUL, _) => {
                        let (Some(x), Some(y)) = (normal_form(a)?, normal_form(b)?) else {
                            return Ok(None);
                        };
                        return x.mul(y).map(Some).map_err(|err| overflow(err, op.as_str()));
                    }
                    (wk::UNITS_POW, Val::LitRat(r)) => {
                        let Some(x) = normal_form(a)? else {
                            return Ok(None);
                        };
                        let e = Exponent::from_rational(r)
                            .map_err(|err| overflow(err, &format!("^{r}")))?;
                        return x.pow(e).map(Some).map_err(|err| overflow(err, op.as_str()));
                    }
                    _ => {}
                }
            }
            Ok(atom(v))
        }
        _ => Ok(None),
    }
}

/// `v` as a product with one factor, itself.
fn atom(v: &Val) -> Option<UnitNf> {
    let term = try_readback_val(ATOM_KEY_LEVEL, v).ok()?;
    let key = format!("{term:?}");
    Some(UnitNf {
        closed: Unit::dimensionless(),
        atoms: vec![Atom {
            val: v.clone(),
            term,
            key,
            exp: Exponent::ONE,
        }],
    })
}

impl UnitNf {
    fn mul(mut self, other: UnitNf) -> Result<UnitNf, UnitError> {
        self.closed = self.closed.mul(&other.closed)?;
        for b in other.atoms {
            match self.atoms.iter_mut().find(|a| a.term == b.term) {
                Some(a) => a.exp = a.exp.checked_add(b.exp)?,
                None => self.atoms.push(b),
            }
        }
        self.atoms.retain(|a| !a.exp.is_zero());
        self.atoms.sort_by(|x, y| x.key.cmp(&y.key));
        Ok(self)
    }

    fn pow(mut self, e: Exponent) -> Result<UnitNf, UnitError> {
        self.closed = self.closed.pow(e)?;
        for a in &mut self.atoms {
            a.exp = a.exp.checked_mul(e)?;
        }
        self.atoms.retain(|a| !a.exp.is_zero());
        Ok(self)
    }

    /// The canonical value: a `LitUnit` when no atom remains, else the closed part (unless it is
    /// `1`) followed by each atom in key order — raised by `units:pow` when its exponent is not 1 —
    /// right-nested under `units:mul`.
    ///
    /// Built directly, not through `app_impl`, which would reduce it again. Normalising the result
    /// gives back this normal form, so it reads back to the same term however it is reached.
    fn into_val(self) -> Val {
        if self.atoms.is_empty() {
            return Val::LitUnit(self.closed);
        }
        let mut factors = Vec::with_capacity(self.atoms.len() + 1);
        if self.closed != Unit::dimensionless() {
            factors.push(Val::LitUnit(self.closed));
        }
        for a in self.atoms {
            factors.push(if a.exp == Exponent::ONE {
                a.val
            } else {
                apply2(wk::UNITS_POW, a.val, Val::LitRat(a.exp.to_rational()))
            });
        }
        let mut acc = factors.pop().expect("a product with atoms has a factor");
        while let Some(f) = factors.pop() {
            acc = apply2(wk::UNITS_MUL, f, acc);
        }
        acc
    }
}

/// The neutral `op a b`.
fn apply2(op: &str, a: Val, b: Val) -> Val {
    Val::Nt(Neut::App(
        Box::new(Neut::App(
            Box::new(Neut::EigonAxiom(wk::iri(op))),
            Box::new(a),
        )),
        Box::new(b),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::readback::readback_val;
    use crate::units::BaseDimension;

    fn var(level: usize, name: &str) -> Val {
        Val::Nt(Neut::Gen(level, name.to_string()))
    }
    fn lit(u: Unit) -> Val {
        Val::LitUnit(u)
    }
    fn m() -> Unit {
        Unit::base(BaseDimension::Length)
    }
    fn s() -> Unit {
        Unit::base(BaseDimension::Time)
    }
    /// `units:mul a b` through the real application path, so the hook runs.
    fn mul(a: Val, b: Val) -> Val {
        Val::Nt(Neut::EigonAxiom(wk::iri(wk::UNITS_MUL)))
            .app(a)
            .unwrap()
            .app(b)
            .unwrap()
    }
    fn pow(a: Val, n: i64, d: i64) -> Val {
        let r = crate::numeric::Rational::new(n.into(), d.into()).unwrap();
        Val::Nt(Neut::EigonAxiom(wk::iri(wk::UNITS_POW)))
            .app(a)
            .unwrap()
            .app(Val::LitRat(r))
            .unwrap()
    }
    fn same(a: &Val, b: &Val) -> bool {
        readback_val(10, a) == readback_val(10, b)
    }

    #[test]
    fn a_closed_product_reduces_to_a_literal() {
        let v = mul(lit(m()), pow(lit(s()), -1, 1));
        assert!(matches!(&v, Val::LitUnit(u) if *u == m().div(&s()).unwrap()));
    }

    #[test]
    fn an_open_product_commutes() {
        let (u, w) = (var(0, "u"), var(1, "v"));
        assert!(same(&mul(u.clone(), w.clone()), &mul(w, u)));
    }

    #[test]
    fn an_open_product_is_associative() {
        let (a, b, c) = (var(0, "a"), var(1, "b"), var(2, "c"));
        let left = mul(mul(a.clone(), b.clone()), c.clone());
        let right = mul(a, mul(b, c));
        assert!(same(&left, &right));
    }

    #[test]
    fn closed_factors_gather_whatever_the_order() {
        let u = var(0, "u");
        let one = mul(mul(lit(m()), u.clone()), pow(lit(s()), -1, 1));
        let two = mul(u, lit(m().div(&s()).unwrap()));
        assert!(same(&one, &two));
    }

    #[test]
    fn a_variable_times_its_inverse_is_the_literal_one() {
        let u = var(0, "u");
        let v = mul(u.clone(), pow(u, -1, 1));
        assert!(matches!(&v, Val::LitUnit(x) if *x == Unit::dimensionless()));
    }

    #[test]
    fn repeated_atoms_merge_their_exponents() {
        let u = var(0, "u");
        assert!(same(&mul(u.clone(), u.clone()), &pow(u, 2, 1)));
    }

    #[test]
    fn powers_compose() {
        let u = var(0, "u");
        assert!(same(&pow(pow(u.clone(), 2, 1), 1, 2), &u));
    }

    /// The canonical spine normalises to itself, so reaching a normal form twice changes nothing.
    #[test]
    fn normalisation_is_idempotent() {
        let (u, w) = (var(0, "u"), var(1, "v"));
        let v = mul(lit(m()), mul(pow(w, -1, 1), u));
        let again = match &v {
            Val::Nt(n) => try_reduce_unit(n)
                .unwrap()
                .expect("the spine is a full application"),
            other => panic!("expected a spine, got {other:?}"),
        };
        assert!(same(&v, &again));
    }

    #[test]
    fn distinct_variables_do_not_merge() {
        let (u, w) = (var(0, "u"), var(1, "v"));
        assert!(!same(&mul(u.clone(), u.clone()), &mul(u, w)));
    }

    #[test]
    fn an_exponent_past_sixteen_bits_is_refused() {
        let r = crate::numeric::Rational::new(40000.into(), 1.into()).unwrap();
        let err = Val::Nt(Neut::EigonAxiom(wk::iri(wk::UNITS_POW)))
            .app(lit(m()))
            .unwrap()
            .app(Val::LitRat(r));
        assert!(matches!(err, Err(EvalError::UnitOverflow(_))));
    }
}
