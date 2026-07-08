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

//! Shared test helpers for the `eval` submodule tests: minimal
//! inductive declarations (Nat, self-reference stubs) and value
//! builders.

use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};
use crate::nbe::val::Val;
use std::sync::Arc;

/// Stub self-reference for use inside an inductive's own constructor
/// types. Carries the matching name with empty `ctors`; iota
/// reduction only inspects names on inner refs, so this is enough
/// to drive the algorithm without genuinely cyclic Arc allocation.
pub(crate) fn ind_self_ref(name: &str) -> Arc<InductiveDecl> {
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
        name: name.to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: Vec::new(),
    })
}

/// inductive Nat { zero : Nat, succ : Nat → Nat }
pub(crate) fn nat_decl() -> Arc<InductiveDecl> {
    let s = ind_self_ref("Nat");
    let nat_ty = Exp::InductiveType(s, Vec::new());
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
        name: "Nat".to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: vec![
            InductiveCtorDecl {
                name: "zero".to_string(),
                typ: nat_ty.clone(),
            },
            InductiveCtorDecl {
                name: "succ".to_string(),
                typ: Exp::Pi(Patt::Unit, Box::new(nat_ty.clone()), Box::new(nat_ty)),
            },
        ],
    })
}

pub(crate) fn ind_zero(decl: &Arc<InductiveDecl>) -> Val {
    Val::InductiveVal {
        decl: decl.clone(),
        ctor_name: "zero".to_string(),
        args: Vec::new(),
    }
}

pub(crate) fn ind_succ(decl: &Arc<InductiveDecl>, n: Val) -> Val {
    Val::InductiveVal {
        decl: decl.clone(),
        ctor_name: "succ".to_string(),
        args: vec![n],
    }
}

pub(crate) fn nat_n(decl: &Arc<InductiveDecl>, n: usize) -> Val {
    let mut v = ind_zero(decl);
    for _ in 0..n {
        v = ind_succ(decl, v);
    }
    v
}

/// Helper: build a cons-pair list from values.
pub(crate) fn cons_list(items: Vec<Val>) -> Val {
    let mut result = Val::Con("nil".into(), Box::new(Val::Unit));
    for item in items.into_iter().rev() {
        result = Val::Con(
            "cons".into(),
            Box::new(Val::Pair(Box::new(item), Box::new(result))),
        );
    }
    result
}
