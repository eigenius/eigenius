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

//! Map/Reduce evaluation over collections (Phase 11a; inductive
//! `List` backing per Phase 11b step 7). Split from `eval.rs`.

use super::{EvalCtx, EvalError};
use crate::nbe::val::{Neut, Val};

/// Evaluate Map(f, collection).
///
/// Applies `f` to each element of a finite list. Accepts both
/// `Val::List` (primary, from resource arrays) and cons-pair chains
/// (legacy, from algebraic construction). Returns `Val::List`.
pub(super) fn eval_map(f: Val, coll: Val, ctx: &EvalCtx) -> Result<Val, EvalError> {
    match coll {
        Val::List(items) => {
            let mapped: Result<Vec<Val>, EvalError> = items
                .into_iter()
                .map(|elem| f.clone().app_ctx(elem, ctx))
                .collect();
            Ok(Val::List(mapped?))
        }
        Val::Con(ref name, _) if name == "nil" || name == "cons" => {
            match crate::nbe::val::cons_to_vec(&coll) {
                Some(items) => {
                    let mapped: Result<Vec<Val>, EvalError> = items
                        .into_iter()
                        .map(|elem| f.clone().app_ctx(elem, ctx))
                        .collect();
                    Ok(Val::List(mapped?))
                }
                None => Err(EvalError::InvalidCaseTarget(format!(
                    "Map: malformed cons list: {coll:?}"
                ))),
            }
        }
        Val::InductiveVal { ref decl, .. } if decl.name == "List" => {
            match crate::nbe::val::inductive_list_to_vec(&coll) {
                Some(items) => {
                    let mapped: Result<Vec<Val>, EvalError> = items
                        .into_iter()
                        .map(|elem| f.clone().app_ctx(elem, ctx))
                        .collect();
                    Ok(Val::List(mapped?))
                }
                None => Err(EvalError::InvalidCaseTarget(format!(
                    "Map: malformed inductive list: {coll:?}"
                ))),
            }
        }
        Val::Nt(n) => Ok(Val::Nt(Neut::NtMap(Box::new(f), Box::new(n)))),
        other => Err(EvalError::InvalidCaseTarget(format!(
            "Map: expected list, got {other:?}"
        ))),
    }
}

/// Evaluate Reduce(f, accumulator, collection).
///
/// Left-folds `f` over a finite list starting with `acc`. Accepts the
/// same three list shapes as [`eval_map`].
pub(super) fn eval_reduce(f: Val, acc: Val, coll: Val, ctx: &EvalCtx) -> Result<Val, EvalError> {
    match coll {
        Val::List(items) => {
            let mut result = acc;
            for elem in items {
                result = f.clone().app_ctx(result, ctx)?.app_ctx(elem, ctx)?;
            }
            Ok(result)
        }
        Val::Con(ref name, _) if name == "nil" || name == "cons" => {
            match crate::nbe::val::cons_to_vec(&coll) {
                Some(items) => {
                    let mut result = acc;
                    for elem in items {
                        result = f.clone().app_ctx(result, ctx)?.app_ctx(elem, ctx)?;
                    }
                    Ok(result)
                }
                None => Err(EvalError::InvalidCaseTarget(format!(
                    "Reduce: malformed cons list: {coll:?}"
                ))),
            }
        }
        Val::InductiveVal { ref decl, .. } if decl.name == "List" => {
            match crate::nbe::val::inductive_list_to_vec(&coll) {
                Some(items) => {
                    let mut result = acc;
                    for elem in items {
                        result = f.clone().app_ctx(result, ctx)?.app_ctx(elem, ctx)?;
                    }
                    Ok(result)
                }
                None => Err(EvalError::InvalidCaseTarget(format!(
                    "Reduce: malformed inductive list: {coll:?}"
                ))),
            }
        }
        Val::Nt(n) => Ok(Val::Nt(Neut::NtReduce(
            Box::new(f),
            Box::new(acc),
            Box::new(n),
        ))),
        other => Err(EvalError::InvalidCaseTarget(format!(
            "Reduce: expected list, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::env::Rho;
    use crate::nbe::eval::testutil::*;
    use crate::nbe::eval::{eval, EvalCtx, EvalError};
    use crate::nbe::term::{Exp, Patt};
    use crate::nbe::val::{Clos, Val};
    // --- Map/Reduce tests (Phase 11a) ---

    /// Helper: identity lambda: λx. x
    fn id_lam() -> Exp {
        Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        )
    }

    #[test]
    fn map_empty_list() -> Result<(), EvalError> {
        let exp = Exp::Map(
            Box::new(id_lam()),
            Box::new(Exp::Con("nil".into(), Box::new(Exp::Unit))),
        );
        let v = eval(&exp, &Rho::Nil)?;
        match v {
            Val::List(items) => assert!(items.is_empty()),
            other => panic!("expected List, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn map_two_elements() -> Result<(), EvalError> {
        // Map(λx. x, [Unit, Set]) → [Unit, Set]
        let list = cons_list(vec![Val::Unit, Val::Sort(1)]);
        let rho = Rho::Nil.extend(Patt::Var("lst".to_string()), list);
        let exp = Exp::Map(Box::new(id_lam()), Box::new(Exp::Var("lst".to_string())));
        let v = eval(&exp, &rho)?;
        match v {
            Val::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Val::Unit));
                assert!(matches!(items[1], Val::Sort(1)));
            }
            other => panic!("expected List, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn map_over_val_list() -> Result<(), EvalError> {
        // Map over Val::List (primary representation)
        let rho = Rho::Nil.extend(
            Patt::Var("lst".to_string()),
            Val::List(vec![Val::Unit, Val::One]),
        );
        let exp = Exp::Map(Box::new(id_lam()), Box::new(Exp::Var("lst".to_string())));
        let v = eval(&exp, &rho)?;
        match v {
            Val::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Val::Unit));
                assert!(matches!(items[1], Val::One));
            }
            other => panic!("expected List, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn map_neutral_collection() -> Result<(), EvalError> {
        let rho = Rho::Nil.extend(
            Patt::Var("n".to_string()),
            Val::Nt(Neut::Gen(0, "n".to_string())),
        );
        let exp = Exp::Map(Box::new(id_lam()), Box::new(Exp::Var("n".to_string())));
        let v = eval(&exp, &rho)?;
        assert!(matches!(v, Val::Nt(Neut::NtMap(_, _))));
        Ok(())
    }

    #[test]
    fn reduce_empty_list() -> Result<(), EvalError> {
        // Reduce(f, Unit, []) → Unit
        let rho = Rho::Nil.extend(
            Patt::Var("f".to_string()),
            Val::Lam(Clos::new(
                Patt::Var("acc".to_string()),
                Exp::Lam(
                    Patt::Var("x".to_string()),
                    Box::new(Exp::Var("acc".to_string())),
                ),
                Rho::Nil,
            )),
        );
        let exp = Exp::Reduce(
            Box::new(Exp::Var("f".to_string())),
            Box::new(Exp::Unit),
            Box::new(Exp::Con("nil".into(), Box::new(Exp::Unit))),
        );
        let v = eval(&exp, &rho)?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn reduce_neutral_collection() -> Result<(), EvalError> {
        let rho = Rho::Nil
            .extend(
                Patt::Var("f".to_string()),
                Val::Lam(Clos::new(
                    Patt::Var("acc".to_string()),
                    Exp::Lam(
                        Patt::Var("x".to_string()),
                        Box::new(Exp::Var("acc".to_string())),
                    ),
                    Rho::Nil,
                )),
            )
            .extend(
                Patt::Var("n".to_string()),
                Val::Nt(Neut::Gen(0, "n".to_string())),
            );
        let exp = Exp::Reduce(
            Box::new(Exp::Var("f".to_string())),
            Box::new(Exp::Unit),
            Box::new(Exp::Var("n".to_string())),
        );
        let v = eval(&exp, &rho)?;
        assert!(matches!(v, Val::Nt(Neut::NtReduce(_, _, _))));
        Ok(())
    }

    // --- Map/Reduce on InductiveVal-backed List values (Phase 11b step 7) ---

    /// Build a `Val::InductiveVal`-backed `List(_)` from `items`,
    /// terminated by `nil`. Uses the canonical `list_decl()` so the
    /// inductive name matches what Map/Reduce dispatch on.
    fn ind_list(items: Vec<Val>) -> Val {
        let list = crate::nbe::term::list_decl();
        let mut current = Val::InductiveVal {
            decl: list.clone(),
            ctor_name: "nil".to_string(),
            args: Vec::new(),
        };
        for item in items.into_iter().rev() {
            current = Val::InductiveVal {
                decl: list.clone(),
                ctor_name: "cons".to_string(),
                args: vec![item, current],
            };
        }
        current
    }

    #[test]
    fn map_over_inductive_list() {
        // Map identity over a 3-element InductiveVal list → Val::List of 3 items.
        let id_lam = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Var("x".to_string()),
            Exp::Var("x".to_string()),
            Rho::Nil,
        ));
        let lst = ind_list(vec![Val::Unit, Val::Unit, Val::Unit]);
        let result = eval_map(id_lam, lst, &EvalCtx::Pure).expect("eval_map");
        match result {
            Val::List(items) => assert_eq!(items.len(), 3),
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn reduce_over_inductive_list() {
        // Reduce a constant function (λacc x. acc) over an inductive list.
        let lst = ind_list(vec![Val::Unit, Val::Unit]);
        // λacc. λx. acc
        let f = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Var("acc".to_string()),
            Exp::Lam(Patt::Unit, Box::new(Exp::Var("acc".to_string()))),
            Rho::Nil,
        ));
        let result = eval_reduce(f, Val::Sort(1), lst, &EvalCtx::Pure).expect("eval_reduce");
        assert!(matches!(result, Val::Sort(1)));
    }

    #[test]
    fn map_over_empty_inductive_list() {
        let id_lam = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Var("x".to_string()),
            Exp::Var("x".to_string()),
            Rho::Nil,
        ));
        let lst = ind_list(Vec::new());
        let result = eval_map(id_lam, lst, &EvalCtx::Pure).expect("eval_map");
        match result {
            Val::List(items) => assert!(items.is_empty()),
            other => panic!("expected empty List, got {other:?}"),
        }
    }
}
