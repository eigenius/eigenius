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

/// A context whose environment declares `data <name> : Sort(<sort>)` for each
/// `(name, sort)`, plus the reference expression naming each.
///
/// **D76 Phase B makes this necessary.** A type former used to carry its
/// declaration inside the term, so a test could build one with no environment at
/// all. It now names it, so a test that checks how a former behaves in the
/// universe hierarchy has to put the declaration somewhere the checker can find
/// it — which is the point of the phase.
pub(crate) fn ctx_declaring(decls: &[(&str, usize)]) -> (CheckCtx, Vec<Exp>) {
    use crate::ontology::iri::Iri;

    let mut c = CheckCtx::new(Rho::Nil, vec![]);
    let mut refs = Vec::new();
    for (name, sort) in decls {
        let iri = Iri::parse(&format!("urn:test:{name}")).expect("test iri");
        c = c.declaring(Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: iri.clone(),
            name: (*name).to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(*sort),
            ctors: Vec::new(),
        }));
        refs.push(Exp::Const(iri, Vec::new()));
    }
    (c, refs)
}

use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};
use crate::nbe::val::Val;
use std::sync::Arc;

/// A two-parameter inductive former with no constructors — `PartialEq` on `InductiveDecl` goes
/// by IRI, so two calls compare equal. Was `sized_stream_decl` (`SizedStream(i : SizeSort, A : Set)`)
/// until eigenius#218; the subtyping tests that used it care about parameter INVARIANCE, not sizes.
pub(crate) fn two_param_decl() -> Arc<InductiveDecl> {
    Arc::new(InductiveDecl {
        uparams: Vec::new(),
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
    let nat_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
    Arc::new(InductiveDecl {
        uparams: Vec::new(),
        iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
        name: "Nat".to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: vec![
            InductiveCtorDecl {
                implicit: Vec::new(),
                name: "zero".to_string(),
                typ: nat_ty.clone(),
            },
            InductiveCtorDecl {
                implicit: Vec::new(),
                name: "succ".to_string(),
                typ: Exp::Pi(Patt::Unit, Box::new(nat_ty.clone()), Box::new(nat_ty)),
            },
        ],
    })
}

pub(crate) fn nat_zero_exp(decl: &Arc<InductiveDecl>) -> Exp {
    Exp::InductiveCtor(decl.iri.clone(), "zero".to_string(), Vec::new())
}

pub(crate) fn ind_self_ref(name: &str) -> Arc<InductiveDecl> {
    Arc::new(InductiveDecl {
        uparams: Vec::new(),
        iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
        name: name.to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: Vec::new(),
    })
}
