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
use crate::nbe::env::{gen_val, up_gamma, Rho};

pub(crate) fn ctx() -> CheckCtx {
    CheckCtx::new(Rho::Nil, vec![])
}

use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};
use crate::nbe::val::Val;
use std::sync::Arc;

pub(crate) fn sized_stream_decl() -> Arc<InductiveDecl> {
    // Minimal sized type former: `SizedStream(i : SizeSort, A : Set)`.
    // We don't need real constructors for the subtyping tests —
    // `PartialEq` on `InductiveDecl` goes by name, so two calls to
    // this helper produce decls that compare equal.
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse("urn:test:SizedStream").unwrap(),
        name: "SizedStream".to_string(),
        params: vec![
            (Patt::Var("i".to_string()), Exp::SizeSort),
            (Patt::Var("A".to_string()), Exp::Sort(1)),
        ],
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: vec![],
    })
}

pub(crate) fn mk_sized_type(decl: Arc<InductiveDecl>, size: Val, elem: Val) -> Val {
    Val::InductiveType {
        decl,
        params: vec![size, elem],
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

pub(crate) fn nat_zero_exp(decl: &Arc<InductiveDecl>) -> Exp {
    Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new())
}

pub(crate) fn sized_nat_decl() -> Arc<InductiveDecl> {
    // SizedNat(i : SizeSort) with
    //   zero : Π i:SizeSort. SizedNat i       (exists at every size)
    //   succ : Π i:SizeSort. SizedNat i → SizedNat (↑ i)
    let self_ref = Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse("urn:test:SizedNat").unwrap(),
        name: "SizedNat".to_string(),
        params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: Vec::new(),
    });
    let snat_i = Exp::InductiveType(self_ref.clone(), vec![Exp::Var("i".to_string())]);
    let snat_succ_i = Exp::InductiveType(
        self_ref,
        vec![Exp::SizeSucc(Box::new(Exp::Var("i".to_string())))],
    );
    Arc::new(InductiveDecl {
        iri: crate::ontology::iri::Iri::parse("urn:test:SizedNat").unwrap(),
        name: "SizedNat".to_string(),
        params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: vec![
            InductiveCtorDecl {
                name: "zero".to_string(),
                typ: Exp::Pi(
                    Patt::Var("i".to_string()),
                    Box::new(Exp::SizeSort),
                    Box::new(snat_i.clone()),
                ),
            },
            InductiveCtorDecl {
                name: "succ".to_string(),
                typ: Exp::Pi(
                    Patt::Var("i".to_string()),
                    Box::new(Exp::SizeSort),
                    Box::new(Exp::Pi(Patt::Unit, Box::new(snat_i), Box::new(snat_succ_i))),
                ),
            },
        ],
    })
}

/// Build a context with `i : SizeSort` bound as a rigid size
/// variable at level 0. Returns the ctx and i's value.
pub(crate) fn ctx_with_size_var(name: &str) -> (CheckCtx, Val) {
    let i_val = gen_val(&Rho::Nil);
    let rho1 = Rho::Nil.extend(Patt::Var(name.to_string()), i_val.clone());
    let gamma1 = up_gamma(
        &Vec::new(),
        &Patt::Var(name.to_string()),
        &Val::SizeSort,
        &i_val,
    )
    .unwrap();
    (CheckCtx::new(rho1, gamma1), i_val)
}

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
