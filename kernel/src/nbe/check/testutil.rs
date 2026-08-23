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

//! Shared test helpers for the `check` submodule tests.

use crate::nbe::check::CheckCtx;
use crate::nbe::env::Rho;

pub(crate) fn ctx() -> CheckCtx {
    CheckCtx::new(Rho::Nil, vec![])
}

use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};
use crate::nbe::val::Val;
use std::sync::Arc;

/// A two-parameter inductive former with no constructors — `PartialEq` on `InductiveDecl` goes
/// by IRI, so two calls compare equal. Was `sized_stream_decl` (`SizedStream(i : SizeSort, A : Set)`)
/// until eigenius#218; the subtyping tests that used it care about parameter INVARIANCE, not sizes.
pub(crate) fn two_param_decl() -> Arc<InductiveDecl> {
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse("urn:test:Pair2").unwrap(),
        name: "Pair2".to_string(),
        params: vec![
            (Patt::Var("A".to_string()), Exp::sort(1)),
            (Patt::Var("B".to_string()), Exp::sort(1)),
        ],
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: vec![],
    })
}

pub(crate) fn mk_two_param(decl: Arc<InductiveDecl>, a: Val, b: Val) -> Val {
    Val::InductiveType {
        decl,
        params: vec![a, b],
        indices: Vec::new(),
    }
}

pub(crate) fn nat_decl() -> Arc<InductiveDecl> {
    let s = ind_self_ref("Nat");
    let nat_ty = Exp::InductiveType(s, Vec::new());
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
        name: "Nat".to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(1),
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

pub(crate) fn nat_zero_exp(decl: &Arc<InductiveDecl>) -> Exp {
    Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new())
}

pub(crate) fn ind_self_ref(name: &str) -> Arc<InductiveDecl> {
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
        name: name.to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: Vec::new(),
    })
}
