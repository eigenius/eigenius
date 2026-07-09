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

//! EigenTT evaluation and type environments.
//!
//! Ported from `Main.hs` lines 169-280 in the EigenTT reference.

use crate::nbe::term::{Decl, Name, Patt};
use crate::nbe::val::Val;

/// Evaluation environment: a linked list of bindings.
///
/// Maps pattern-bound names to values. Declarations are
/// evaluated lazily when looked up.
#[derive(Debug, Clone)]
pub enum Rho {
    /// Empty environment
    Nil,
    /// Extend with a pattern binding: ρ, p = v
    UpVar(Box<Rho>, Patt, Box<Val>),
    /// Extend with a declaration: ρ, d
    UpDec(Box<Rho>, Decl),
}

impl Rho {
    /// Extend with a variable binding.
    pub fn extend(self, patt: Patt, val: Val) -> Rho {
        Rho::UpVar(Box::new(self), patt, Box::new(val))
    }

    /// Look up a name in the environment.
    ///
    /// Port of `getRho` from the reference.
    pub fn get(&self, name: &str) -> Result<Val, String> {
        match self {
            Rho::Nil => Err(format!("unbound variable: {name}")),
            Rho::UpVar(rho, patt, val) => {
                if patt.contains(name) {
                    pat_proj(patt, name, val)
                } else {
                    rho.get(name)
                }
            }
            Rho::UpDec(rho, decl) => match decl {
                Decl::Def(patt, _typ, body) => {
                    if patt.contains(name) {
                        let val = crate::nbe::eval::eval(body, rho).map_err(|e| e.to_string())?;
                        pat_proj(patt, name, &val)
                    } else {
                        rho.get(name)
                    }
                }
                Decl::Drec(patt, _typ, body) => {
                    if patt.contains(name) {
                        // For recursive definitions, evaluate in the extended environment
                        let rho_ext = Rho::UpDec(rho.clone(), decl.clone());
                        let val =
                            crate::nbe::eval::eval(body, &rho_ext).map_err(|e| e.to_string())?;
                        pat_proj(patt, name, &val)
                    } else {
                        rho.get(name)
                    }
                }
            },
        }
    }

    /// Length of the environment (number of variable bindings).
    ///
    /// Port of `lRho` from the reference.
    pub fn len(&self) -> usize {
        match self {
            Rho::Nil => 0,
            Rho::UpVar(rho, _, _) => rho.len() + 1,
            Rho::UpDec(rho, _) => rho.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Project a value out of a pattern binding.
///
/// Port of `patProj` from the reference.
fn pat_proj(patt: &Patt, name: &str, val: &Val) -> Result<Val, String> {
    match patt {
        Patt::Var(n) if n == name => Ok(val.clone()),
        Patt::Pair(p1, p2) => {
            if p1.contains(name) {
                pat_proj(p1, name, &val.clone().vfst().map_err(|e| e.to_string())?)
            } else if p2.contains(name) {
                pat_proj(p2, name, &val.clone().vsnd().map_err(|e| e.to_string())?)
            } else {
                Err(format!("patProj: {name} not in pattern"))
            }
        }
        _ => Err(format!("patProj: {name} not in pattern")),
    }
}

/// Type environment: maps names to their types.
pub type Gamma = Vec<(Name, Val)>;

/// Look up a name in the type environment.
pub fn lookup_gamma(gamma: &Gamma, name: &str) -> Result<Val, String> {
    for (n, t) in gamma {
        if n == name {
            return Ok(t.clone());
        }
    }
    Err(format!("unbound variable in type context: {name}"))
}

/// Extend the type environment with pattern bindings.
///
/// Port of `upG` from the reference:
/// Gamma |- p : t = v => Gamma'
pub fn up_gamma(gamma: &Gamma, patt: &Patt, typ: &Val, val: &Val) -> Result<Gamma, String> {
    match patt {
        Patt::Unit => Ok(gamma.clone()),
        Patt::Var(x) => {
            let mut g = gamma.clone();
            g.push((x.clone(), typ.clone()));
            Ok(g)
        }
        Patt::Pair(p1, p2) => {
            if let Val::Sig(t, g) = typ {
                let g1 = up_gamma(
                    gamma,
                    p1,
                    t,
                    &val.clone().vfst().map_err(|e| e.to_string())?,
                )?;
                let t2 = g
                    .apply(val.clone().vfst().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                up_gamma(
                    &g1,
                    p2,
                    &t2,
                    &val.clone().vsnd().map_err(|e| e.to_string())?,
                )
            } else {
                Err(format!(
                    "upG: expected Sig type for pair pattern, got {:?}",
                    typ
                ))
            }
        }
    }
}

/// Generate a fresh variable value for type checking, at the current
/// environment depth. Port of `genV` from the reference.
///
/// Not to be conflated with `readback::gen_val`: the name tag is
/// load-bearing — `Neut::Gen(j, name)` reads back as `Exp::Var("{name}{j}")`
/// — so the checker's `TC#` convention and readback's `G#` (paired with
/// `readback::gen_patt`) are deliberately distinct, not a duplication.
pub fn gen_val(rho: &Rho) -> Val {
    Val::Nt(crate::nbe::val::Neut::Gen(rho.len(), "TC#".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_env() {
        let rho = Rho::Nil;
        assert!(rho.get("x").is_err());
        assert_eq!(rho.len(), 0);
    }

    #[test]
    fn simple_lookup() {
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        assert!(matches!(rho.get("x"), Ok(Val::Unit)));
        assert!(rho.get("y").is_err());
        assert_eq!(rho.len(), 1);
    }

    #[test]
    fn pair_pattern_lookup() {
        let rho = Rho::Nil.extend(
            Patt::Pair(
                Box::new(Patt::Var("a".to_string())),
                Box::new(Patt::Var("b".to_string())),
            ),
            Val::Pair(Box::new(Val::Unit), Box::new(Val::Sort(1))),
        );
        assert!(matches!(rho.get("a"), Ok(Val::Unit)));
        assert!(matches!(rho.get("b"), Ok(Val::Sort(1))));
    }

    #[test]
    fn lookup_gamma_found() {
        let gamma: Gamma = vec![("x".to_string(), Val::One)];
        assert!(matches!(lookup_gamma(&gamma, "x"), Ok(Val::One)));
    }

    #[test]
    fn lookup_gamma_not_found() {
        let gamma: Gamma = vec![];
        assert!(lookup_gamma(&gamma, "x").is_err());
    }
}
