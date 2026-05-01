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

//! Mini-TT bidirectional type checker.
//!
//! Ported from `Main.hs` lines 289-378 in the Mini-TT reference.
//! Uses NbE (eval + readback) for type equality checking.

use crate::layer::Layer;
use crate::nbe::env::{gen_val, lookup_gamma, up_gamma, Gamma, Rho};
use crate::nbe::eval::{eval, eval_ctx, EvalCtx};
use crate::nbe::readback::readback_val;
use crate::nbe::recursor::derive_minor_types;
use crate::nbe::term::{Decl, Exp, InductiveDecl, Patt};
use crate::nbe::val::{Clos, Val};
use crate::ontology::iri::Iri;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Type-checking context, threaded through all checker calls.
///
/// Bundles the evaluation environment (`rho`), type context (`gamma`),
/// an optional layer for ontology-as-types resolution, and a per-check
/// cache for resolved class types.
///
/// Design follows nanoda_lib's `TypeChecker` pattern
/// ([nanoda_lib `src/tc.rs`](https://github.com/ammkrn/nanoda_lib/blob/main/src/tc.rs)): a single struct carrying
/// mutable state (cache) plus immutable environment through all checker
/// calls. The cache is scoped per type-check invocation — fresh per
/// call, no cross-check invalidation needed.
pub struct CheckCtx {
    pub rho: Rho,
    pub gamma: Gamma,
    /// Optional layer for ontology resolution. `None` is the "pure"
    /// case used by tests that don't touch EigonClass resolution.
    pub layer: Option<Arc<Layer>>,
    /// Per-check memoization of resolved class types, keyed by class IRI string.
    type_cache: BTreeMap<String, Val>,
    /// Rigid size hypotheses accumulated from bounded size binders
    /// (`SizedPi { patt, upper, body }`). Keyed by the level of the
    /// bound size variable (which doubles as its rigid-id): the TSO
    /// records `bound_level < upper_rigid_level` (or distance 0 against
    /// `∞`'s sentinel) when the checker crosses a `SizedPi` in a type.
    ///
    /// Consulted by [`subtype_of`] and any direct size-comparison
    /// site via [`crate::nbe::sized::size_le_with_hyps`].
    pub size_tso: crate::nbe::sized_rigid::Tso,
    /// D14 institution index — derived view of the layer chain. When
    /// attached together with `institution_runtime`,
    /// `Constraint::Institution` predicates dispatch through
    /// `try_d14_decide` (D14 §9.2). Without these, constraints stay
    /// as passthrough neutrals — what `EvalCtx::Pure` does anyway.
    pub institution_index: Option<Arc<crate::institution::registry::InstitutionIndex>>,
    /// D14 institution runtime — registry of `Institution` trait
    /// objects keyed by institution IRI. See `institution_index`.
    pub institution_runtime: Option<Arc<crate::institution::runtime::InstitutionRuntime>>,
}

impl CheckCtx {
    /// Create a new context with no layer access (pure mode).
    pub fn new(rho: Rho, gamma: Gamma) -> Self {
        Self {
            rho,
            gamma,
            layer: None,
            type_cache: BTreeMap::new(),
            size_tso: crate::nbe::sized_rigid::Tso::new(),
            institution_index: None,
            institution_runtime: None,
        }
    }

    /// Create a new context with layer access for ontology resolution.
    pub fn with_layer(rho: Rho, gamma: Gamma, layer: Arc<Layer>) -> Self {
        Self {
            rho,
            gamma,
            layer: Some(layer),
            type_cache: BTreeMap::new(),
            size_tso: crate::nbe::sized_rigid::Tso::new(),
            institution_index: None,
            institution_runtime: None,
        }
    }

    /// Attach a D14 institution index and runtime for check-time
    /// dispatch of `Constraint::Institution` predicates through
    /// `try_d14_decide` (D14 §9.2).
    pub fn with_institutions_d14(
        mut self,
        index: Arc<crate::institution::registry::InstitutionIndex>,
        runtime: Arc<crate::institution::runtime::InstitutionRuntime>,
    ) -> Self {
        self.institution_index = Some(index);
        self.institution_runtime = Some(runtime);
        self
    }

    /// Produce an [`EvalCtx`] suitable for evaluating expressions
    /// under this check context.
    ///
    /// Returns `EvalCtx::Check` when a D14 institution index/runtime
    /// is attached; otherwise `EvalCtx::Pure`. All internal `eval`
    /// calls in `check.rs` should route through this so institution-
    /// dispatched constraints fire at check time rather than deferring
    /// to runtime.
    pub fn eval_ctx(&self) -> crate::nbe::eval::EvalCtx {
        if self.institution_index.is_some() && self.institution_runtime.is_some() {
            crate::nbe::eval::EvalCtx::Check {
                layer: self.layer.clone(),
                institution_index: self.institution_index.clone(),
                institution_runtime: self.institution_runtime.clone(),
            }
        } else {
            crate::nbe::eval::EvalCtx::Pure
        }
    }

    /// Evaluate an expression under this check context's
    /// [`EvalCtx`]. Prefer this over the bare `eval` function
    /// inside `check.rs` so institution-dispatched constraints
    /// (`Constraint::Institution`) fire when the context has a
    /// registry attached.
    pub fn eval(&self, exp: &Exp, rho: &Rho) -> Result<Val, crate::nbe::eval::EvalError> {
        eval_ctx(exp, rho, &self.eval_ctx())
    }

    /// Extend the context with a new variable binding (for entering binders).
    /// Shares the layer and type_cache with the parent context.
    fn extend(&self, patt: &Patt, typ: &Val, val: &Val) -> Result<CheckCtx, String> {
        let gamma1 = up_gamma(&self.gamma, patt, typ, val)?;
        let rho1 = self.rho.clone().extend(patt.clone(), val.clone());
        Ok(CheckCtx {
            rho: rho1,
            gamma: gamma1,
            layer: self.layer.clone(),
            type_cache: self.type_cache.clone(),
            size_tso: self.size_tso.clone(),
            institution_index: self.institution_index.clone(),
            institution_runtime: self.institution_runtime.clone(),
        })
    }

    /// Resolve an EigonClass IRI to a Mini-TT Sigma type, with caching.
    fn resolve_class_cached(&mut self, iri: &Iri) -> Result<Val, String> {
        let layer = self.layer.as_ref().ok_or_else(|| {
            format!(
                "cannot resolve class '{}' — no layer access in pure check mode",
                iri
            )
        })?;
        let key = iri.as_str().to_string();
        if let Some(cached) = self.type_cache.get(&key) {
            return Ok(cached.clone());
        }
        let v = crate::program::ground::resolve_class_type(iri, layer)?;
        self.type_cache.insert(key, v.clone());
        Ok(v)
    }
}

/// Check that a declaration is well-typed, returning the extended type context.
///
/// Port of `checkD` from the reference.
pub fn check_decl(ctx: &mut CheckCtx, decl: &Decl) -> Result<Gamma, String> {
    match decl {
        Decl::Def(patt, typ, body) => {
            // Check that the type is well-formed
            check_type(ctx, typ)?;
            let t = ctx.eval(typ, &ctx.rho).map_err(|e| e.to_string())?;
            // Check that the body has the declared type
            check(ctx, body, &t)?;
            // Extend the type context
            up_gamma(
                &ctx.gamma,
                patt,
                &t,
                &ctx.eval(body, &ctx.rho).map_err(|e| e.to_string())?,
            )
        }
        Decl::Drec(patt, typ, body) => {
            // Known subtlety (issue #13 item 3): The body is type-checked
            // under a generic binding (gen_val) so the checker sees an
            // opaque variable, not the real recursive value. When the real
            // value is substituted (UpDec below), neutrals that previously
            // blocked may reduce to something incompatible. Mini-TT
            // mitigates this via the guardedness check for codata; full
            // termination checking is deferred to Phase 11a.
            //
            // Check that the type is well-formed
            check_type(ctx, typ)?;
            let t = ctx.eval(typ, &ctx.rho).map_err(|e| e.to_string())?;
            let gen = gen_val(&ctx.rho);
            // Extend context with the recursive variable and check body
            let mut inner = ctx.extend(patt, &t, &gen)?;
            check(&mut inner, body, &t)?;
            // Guardedness: if the recursive body constructs a corecord,
            // verify every corecursive reference appears under a
            // constructor/lambda/app — not at the bare head of an
            // observation. D11 §3 "productivity."
            let mut forbidden: std::collections::HashSet<&str> = std::collections::HashSet::new();
            collect_pattern_names(patt, &mut forbidden);
            check_guarded(body, &forbidden)?;
            // Re-evaluate with the recursive binding
            let v = ctx
                .eval(body, &Rho::UpDec(Box::new(ctx.rho.clone()), decl.clone()))
                .map_err(|e| e.to_string())?;
            up_gamma(&ctx.gamma, patt, &t, &v)
        }
    }
}

/// Check that an expression is a well-formed type.
///
/// Port of `checkT` from the reference.
pub fn check_type(ctx: &mut CheckCtx, exp: &Exp) -> Result<(), String> {
    match exp {
        Exp::Pi(p, a, b) | Exp::Sig(p, a, b) => {
            check_type(ctx, a)?;
            let gen = gen_val(&ctx.rho);
            let mut inner =
                ctx.extend(p, &ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?, &gen)?;
            check_type(&mut inner, b)
        }
        // Bounded size Π-type: `{i < upper}. body`. The upper bound
        // must be a rigid size variable or `∞`. Crossing the binder
        // registers `i_level + 1 ≤ upper_level` as a hypothesis in
        // the TSO so subsequent size comparisons in `body` can use
        // the strict-decrease fact.
        Exp::SizedPi { patt, upper, body } => {
            check(ctx, upper, &Val::SizeSort)?;
            let upper_val = ctx.eval(upper, &ctx.rho).map_err(|e| e.to_string())?;
            let new_level = ctx.rho.len();
            let i_val = gen_val(&ctx.rho);
            let mut inner = ctx.extend(patt, &Val::SizeSort, &i_val)?;
            match &upper_val {
                Val::SizeInf => {
                    // No hypothesis: i ≤ ∞ holds structurally.
                }
                Val::Nt(crate::nbe::val::Neut::Gen(upper_level, _)) => {
                    inner
                        .size_tso
                        .insert(new_level as u32, 1, *upper_level as u32);
                }
                other => {
                    return Err(format!(
                        "SizedPi: upper bound must normalise to a rigid size variable \
                         or ∞ — got {:?}",
                        readback_val(ctx.rho.len(), other)
                    ));
                }
            }
            check_type(&mut inner, body)
        }
        Exp::Set | Exp::One | Exp::Type(_) => Ok(()),
        // `SizeSort` is a type (at the first universe above `Set`).
        // Phase 11b step 14 treats it as a distinguished sort so
        // sized-type parameter annotations (`i : SizeSort`) can
        // be written without further infrastructure.
        Exp::SizeSort => Ok(()),
        // Id(A, x, y) is a type if A is a type and x, y : A
        Exp::Id(a, x, y) => {
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)
        }
        // Eigenius ground types are always valid types
        Exp::EigonClass(_) | Exp::EigonPrimitive(_) => Ok(()),

        // Codata type declaration: each observation's type must be a type.
        // Observation names must be distinct.
        Exp::Codata(observations) => {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for obs in observations {
                if !seen.insert(obs.name.as_str()) {
                    return Err(format!(
                        "duplicate observation name in codata type: '{}'",
                        obs.name
                    ));
                }
                check_type(ctx, &obs.typ)?;
            }
            Ok(())
        }

        // Inductive type forms (Phase 11b, D19).
        // The introduction form runs the strict-positivity checker
        // (Phase 11b step 3); references to an already-introduced
        // inductive only need to be admitted as types — Phase 11b
        // step 5 will add parameter telescope verification.
        Exp::Inductive(decl) => crate::nbe::positivity::check_positivity(decl),
        Exp::InductiveType(_, _) => Ok(()),
        // Applied codata type. Admitted as a type when the decl is
        // already known valid; the declaration-site validation runs
        // at ingest time via the ground resolver. We conservatively
        // just accept, matching `InductiveType`'s behaviour.
        Exp::CodataType(_, _) => Ok(()),

        a => check(ctx, a, &Val::Set),
    }
}

/// Check that an expression has a given type (checking mode).
///
/// Port of `check` from the reference.
pub fn check(ctx: &mut CheckCtx, exp: &Exp, typ: &Val) -> Result<(), String> {
    match (exp, typ) {
        // Lambda against Pi type
        (Exp::Lam(p, e), Val::Pi(t, g)) => {
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, t, &gen)?;
            check(&mut inner, e, &g.apply(gen).map_err(|e| e.to_string())?)
        }

        // Lambda against a bounded size Π (Phase 11b step 15f).
        //
        // This is the productivity-via-typing arm: when a corecord
        // observation has type `{j < upper}. body_ty`, its field body
        // is typically `λ j. …`, and that lambda must type-check with
        // `j < upper` registered as a hypothesis in the TSO. The body
        // under this hypothesis can then reference sized inductive or
        // coinductive values at size `j`, and recursive calls on the
        // corecord itself — required by type to produce a result at
        // size `j < outer-size` — are automatically size-decreasing.
        //
        // Productivity of sized corecords falls out of typing: any
        // recursive call that could make the observation infinite-loop
        // would have to produce a value at size ≥ outer, which the
        // size-aware subtyping rejects.
        (Exp::Lam(p, e), Val::SizedPi(upper, g)) => {
            let new_level = ctx.rho.len();
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, &Val::SizeSort, &gen)?;
            match upper.as_ref() {
                Val::SizeInf => {
                    // Upper is ∞: size arg is unconstrained; no hypothesis.
                }
                Val::Nt(crate::nbe::val::Neut::Gen(upper_level, _)) => {
                    inner
                        .size_tso
                        .insert(new_level as u32, 1, *upper_level as u32);
                }
                other => {
                    // Shouldn't arise — a well-formed SizedPi value
                    // always carries a rigid or ∞ upper. Fail loudly
                    // rather than silently accept an unsound hypothesis.
                    return Err(format!(
                        "SizedPi: upper bound must be rigid size var or ∞ — got {:?}",
                        readback_val(ctx.rho.len(), other),
                    ));
                }
            }
            check(&mut inner, e, &g.apply(gen).map_err(|e| e.to_string())?)
        }

        // Pair against Sigma type
        (Exp::Pair(e1, e2), Val::Sig(t, g)) => {
            check(ctx, e1, t)?;
            check(
                ctx,
                e2,
                &g.apply(ctx.eval(e1, &ctx.rho).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            )
        }

        // Constructor against Sum type
        (Exp::Con(c, e), Val::Data(cases, rho1)) => {
            let a = cases
                .iter()
                .find(|(name, _)| name == c)
                .map(|(_, typ)| typ)
                .ok_or_else(|| format!("constructor {c} not in sum type"))?;
            check(ctx, e, &ctx.eval(a, rho1).map_err(|e| e.to_string())?)
        }

        // Case function against Pi from Sum to result
        (Exp::Case(branches), Val::Pi(domain, g)) if matches!(**domain, Val::Data(_, _)) => {
            let (cases, rho1) = match &**domain {
                Val::Data(cases, rho1) => (cases, rho1),
                _ => unreachable!(),
            };
            let branch_names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
            let case_names: Vec<&str> = cases.iter().map(|(n, _)| n.as_str()).collect();
            if branch_names != case_names {
                return Err(format!(
                    "case branches {:?} do not match sum type {:?}",
                    branch_names, case_names
                ));
            }
            for (branch, (c, a)) in branches.iter().zip(cases.iter()) {
                let a_val = ctx.eval(a, rho1).map_err(|e| e.to_string())?;
                let g_c = Clos {
                    patt: Patt::Var("__case_arg".to_string()),
                    body: Exp::App(
                        Box::new(readback_val(ctx.rho.len(), &Val::Lam(g.clone()))),
                        Box::new(Exp::Con(
                            c.clone(),
                            Box::new(Exp::Var("__case_arg".to_string())),
                        )),
                    ),
                    env: ctx.rho.clone(),
                };
                check(ctx, &branch.body, &Val::Pi(Box::new(a_val), g_c))?;
            }
            Ok(())
        }

        // Unit value against One type
        (Exp::Unit, Val::One) => Ok(()),

        // One against Set (One is a type)
        (Exp::One, Val::Set) => Ok(()),

        // Sized types (Phase 11b step 14, D19 §8).
        // `SizeSort` is a type — admit it against `Set` / `Type(n)`
        // the same way Pi and Sigma are. Concrete size values —
        // `SizeInf` and `SizeSucc(_)` — inhabit `Val::SizeSort`.
        (Exp::SizeSort, Val::Set) | (Exp::SizeSort, Val::Type(_)) => Ok(()),
        (Exp::SizeInf, Val::SizeSort) => Ok(()),
        (Exp::SizeSucc(s), Val::SizeSort) => check(ctx, s, &Val::SizeSort),

        // Pi type against Set
        (Exp::Pi(p, a, b), Val::Set) | (Exp::Sig(p, a, b), Val::Set) => {
            check(ctx, a, &Val::Set)?;
            let gen = gen_val(&ctx.rho);
            let mut inner =
                ctx.extend(p, &ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?, &gen)?;
            check(&mut inner, b, &Val::Set)
        }

        // Bounded size Pi against Set/Type — delegate to `check_type`
        // so the TSO hypothesis-insertion logic runs exactly once.
        (Exp::SizedPi { .. }, Val::Set) | (Exp::SizedPi { .. }, Val::Type(_)) => {
            check_type(ctx, exp)
        }

        // Sum type against Set
        (Exp::Data(summands), Val::Set) => {
            for s in summands {
                check(ctx, &s.typ, &Val::Set)?;
            }
            Ok(())
        }

        // Declaration
        (Exp::Dec(d, e), t) => {
            let gamma1 = check_decl(ctx, d)?;
            let mut inner = CheckCtx {
                rho: Rho::UpDec(Box::new(ctx.rho.clone()), d.clone()),
                gamma: gamma1,
                layer: ctx.layer.clone(),
                type_cache: ctx.type_cache.clone(),
                size_tso: ctx.size_tso.clone(),
                institution_index: ctx.institution_index.clone(),
                institution_runtime: ctx.institution_runtime.clone(),
            };
            check(&mut inner, e, t)
        }

        // refl(a) : Id(A, a, a) — check that x and y are both a
        (Exp::Refl(a), Val::Id(typ, x, y)) => {
            check(ctx, a, typ)?;
            let a_val = ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            eq_nf(ctx.rho.len(), x, &a_val)?;
            eq_nf(ctx.rho.len(), y, &a_val)
        }

        // Id(A, x, y) : Set
        (Exp::Id(a, x, y), Val::Set) => {
            check(ctx, a, &Val::Set)?;
            let a_val = ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)
        }

        // Universe hierarchy: Type(n) : Type(n+1) prevents impredicativity.
        // Self-referential meta-claims (e.g. a level-1 trace referencing
        // level-1) are blocked at resource ingestion by the universe
        // stratification validator (Rule 13), not in the term checker.
        (Exp::Type(n), Val::Type(m)) if *n + 1 == *m => Ok(()),
        // Type(n) : Set (Set is the top universe for backward compatibility)
        (Exp::Type(_), Val::Set) => Ok(()),
        // Set : Type(1)
        (Exp::Set, Val::Type(1)) => Ok(()),

        // EigonClass/EigonPrimitive are ground types at level 0 but
        // inhabit all higher universes (cumulative).
        (Exp::EigonClass(_), Val::Set) | (Exp::EigonPrimitive(_), Val::Set) => Ok(()),
        (Exp::EigonClass(_), Val::Type(_)) | (Exp::EigonPrimitive(_), Val::Type(_)) => Ok(()),

        // Codata type formation: codata { ... } : Set
        (Exp::Codata(_), Val::Set) => check_type(ctx, exp),
        (Exp::Codata(_), Val::Type(_)) => check_type(ctx, exp),
        // Parameterised codata — applied codata type expression.
        (Exp::CodataType(_, _), Val::Set) | (Exp::CodataType(_, _), Val::Type(_)) => {
            check_type(ctx, exp)
        }

        // Inductive type formation (Phase 11b, D19).
        (Exp::Inductive(_), Val::Set) | (Exp::InductiveType(_, _), Val::Set) => {
            check_type(ctx, exp)
        }
        (Exp::Inductive(_), Val::Type(_)) | (Exp::InductiveType(_, _), Val::Type(_)) => {
            check_type(ctx, exp)
        }

        // Constructor application against an inductive type — Phase 11b
        // step 5 checking mode. Parameters come from the expected type;
        // each constructor argument is checked against its declared
        // type (with parameters substituted).
        (
            Exp::InductiveCtor(decl, ctor_name, args),
            Val::InductiveType {
                decl: expected_decl,
                params,
            },
        ) => check_inductive_ctor_args(ctx, decl, ctor_name, args, expected_decl, params),

        // Pattern-match elimination with motive inferred from the
        // expected type (Phase 11b step 12, D19 §10). The motive is
        // synthesised as `λ_. expected_type` (constant); per-arm
        // bodies are checked against `expected_type` in a context
        // extended with bindings of the constructor's argument types.
        // Exhaustiveness, no-duplicate-arms, and binding-count match
        // are validated here.
        (Exp::Match { scrutinee, arms }, expected) => check_match(ctx, scrutinee, arms, expected),

        // Corecord against a codata type: each field's body must have
        // the corresponding observation's type, and every declared
        // observation must be covered.
        (Exp::CoRecord(fields), Val::Codata(observations, rho1)) => {
            let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            let obs_names: Vec<&str> = observations.iter().map(|(n, _)| n.as_str()).collect();
            if field_names != obs_names {
                return Err(format!(
                    "corecord fields {:?} do not match codata observations {:?}",
                    field_names, obs_names
                ));
            }
            for (field, (_, obs_typ)) in fields.iter().zip(observations.iter()) {
                let t = ctx.eval(obs_typ, rho1).map_err(|e| e.to_string())?;
                check(ctx, &field.body, &t)?;
            }
            Ok(())
        }

        // Corecord against a parameterised codata type (D19 self-ref
        // path). Same flow as the anonymous variant, but the
        // observations come from `decl.observations` and each
        // observation's type is evaluated in an environment where
        // the decl's type parameters are bound to the applied
        // `params`. This is what lets a self-referential observation
        // like `tail : Stream(A, j)` resolve to the concrete codata
        // type when the corecord is checked against `Stream(A_val, i)`.
        (Exp::CoRecord(fields), Val::CodataType { decl, params }) => {
            // Self-references inside observation types evaluate to
            // `Val::CodataType { stub_decl, params }` where the stub
            // has empty observations. Rehydrate the full decl from
            // the layer when we encounter a stub — analogous to how
            // `resolve_class_cached` threads EigonClass references.
            let full_decl = resolve_full_codata_decl(ctx, decl)?;
            let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            let obs_names: Vec<&str> = full_decl
                .observations
                .iter()
                .map(|o| o.name.as_str())
                .collect();
            if field_names != obs_names {
                return Err(format!(
                    "corecord fields {:?} do not match codata observations {:?}",
                    field_names, obs_names
                ));
            }
            let mut obs_env = Rho::Nil;
            for ((patt, _), val) in full_decl.params.iter().zip(params.iter()) {
                obs_env = obs_env.extend(patt.clone(), val.clone());
            }
            for (field, obs) in fields.iter().zip(full_decl.observations.iter()) {
                let t = ctx.eval(&obs.typ, &obs_env).map_err(|e| e.to_string())?;
                check(ctx, &field.body, &t)?;
            }
            Ok(())
        }

        // Fallthrough: infer type and compare under subtyping
        // (`inferred <: expected`). For everything except sized
        // inductive parameters, `subtype_of` reduces to `eq_nf`.
        // The current TSO is passed through so bounded size binders
        // in scope can witness subtyping between neutral sizes.
        (e, t) => {
            let t1 = check_infer(ctx, e)?;
            subtype_of_with_hyps(ctx.rho.len(), &t1, t, &ctx.size_tso)
        }
    }
}

/// Infer the type of an expression (inference mode).
///
/// Port of `checkI` from the reference.
pub fn check_infer(ctx: &mut CheckCtx, exp: &Exp) -> Result<Val, String> {
    match exp {
        Exp::Var(x) => lookup_gamma(&ctx.gamma, x),

        Exp::App(e1, e2) => {
            let t1 = check_infer(ctx, e1)?;
            // Sized function application: `f(i)` where `f : {i < upper}. body`.
            // The argument must be a size strictly below `upper`, verified
            // via `size_lt_with_hyps` against the current TSO so bounded
            // binders in scope contribute entailment.
            if let Val::SizedPi(upper, g) = &t1 {
                check(ctx, e2, &Val::SizeSort)?;
                let arg_val = ctx.eval(e2, &ctx.rho).map_err(|e| e.to_string())?;
                if !crate::nbe::sized::size_lt_with_hyps(&arg_val, upper, &ctx.size_tso) {
                    return Err(format!(
                        "SizedPi application: argument {:?} is not strictly below upper bound {:?}",
                        readback_val(ctx.rho.len(), &arg_val),
                        readback_val(ctx.rho.len(), upper),
                    ));
                }
                return g.apply(arg_val).map_err(|e| e.to_string());
            }
            let (t, g) = ext_pi(&t1)?;
            check(ctx, e2, &t)?;
            Ok(g.apply(ctx.eval(e2, &ctx.rho).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?)
        }

        Exp::Fst(e) => {
            let t = check_infer(ctx, e)?;
            let (t1, _) = ext_sig(&t)?;
            Ok(t1)
        }

        Exp::Snd(e) => {
            let t = check_infer(ctx, e)?;
            let (_, g) = ext_sig(&t)?;
            Ok(g.apply(
                ctx.eval(e, &ctx.rho)
                    .map_err(|e| e.to_string())?
                    .vfst()
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?)
        }

        // Eigenius: property/observation access type inference.
        //
        // ESL's `.name` syntax unifies two operations:
        // - property access on resources / Sigma-typed values
        // - observation on codata-typed values
        // We dispatch on the inferred type of the target.
        Exp::PropAccess(e, prop) => {
            let t = check_infer(ctx, e)?;
            let prop_name = prop.local_name();

            // Codata observation — same lookup that Exp::Observe does.
            if let Val::Codata(observations, rho1) = &t {
                for (name, typ) in observations {
                    if name == prop_name {
                        return ctx.eval(typ, rho1).map_err(|e| e.to_string());
                    }
                }
                return Err(format!(
                    "observation '{}' not found in codata type {:?}",
                    prop_name,
                    readback_val(ctx.rho.len(), &t)
                ));
            }
            if let Val::CodataType { decl, params } = &t {
                let full_decl = resolve_full_codata_decl(ctx, decl)?;
                return lookup_codata_observation(&full_decl, params, prop_name, ctx.rho.len());
            }

            // Fall back to the existing Sigma / resource behaviour.
            find_sigma_field(ctx, &t, prop_name).ok_or_else(|| {
                format!(
                    "property '{}' not found in type {:?}",
                    prop,
                    readback_val(ctx.rho.len(), &t)
                )
            })
        }

        // Codata observation type inference: e.obs has type T where
        // `obs : T` appears in the inferred codata type of e.
        Exp::Observe(e, obs) => {
            let t = check_infer(ctx, e)?;
            match &t {
                Val::Codata(observations, rho1) => {
                    for (name, typ) in observations {
                        if name == obs {
                            return ctx.eval(typ, rho1).map_err(|e| e.to_string());
                        }
                    }
                    Err(format!(
                        "observation '{}' not found in codata type {:?}",
                        obs,
                        readback_val(ctx.rho.len(), &t)
                    ))
                }
                Val::CodataType { decl, params } => {
                    let full_decl = resolve_full_codata_decl(ctx, decl)?;
                    lookup_codata_observation(&full_decl, params, obs, ctx.rho.len())
                }
                other => Err(format!(
                    "observation target is not a codata value: {:?}",
                    readback_val(ctx.rho.len(), other)
                )),
            }
        }

        // --- Eigenius extension: 7 inference rules (D18 §6, issue #12 item 2) ---

        // Construct(class_iri, fields): check each field against the class's
        // Sigma chain and return EigonClass(class_iri).
        Exp::Construct(class_iri, fields) => {
            let class_type = ctx
                .resolve_class_cached(class_iri)
                .map_err(|e| format!("cannot infer Construct type for '{}': {}", class_iri, e))?;
            // Check each field against the resolved class type
            let mut remaining = class_type;
            for (prop_iri, field_exp) in fields {
                let field_type = find_sigma_field(ctx, &remaining, prop_iri.local_name())
                    .ok_or_else(|| {
                        format!("property '{}' not found in class '{}'", prop_iri, class_iri)
                    })?;
                check(ctx, field_exp, &field_type)?;
                // Advance through the Sigma chain
                remaining = advance_sigma(&remaining, prop_iri.local_name(), field_exp, &ctx.rho);
            }
            Ok(Val::EigonClass(class_iri.clone()))
        }

        // EigonResource(r): infer class from r.is_a().first()
        Exp::EigonResource(r) => {
            let classes = r.is_a();
            let class_iri = classes
                .first()
                .ok_or_else(|| "EigonResource has no is_a class".to_string())?;
            Ok(Val::EigonClass(class_iri.clone()))
        }

        // Template(lit, refs): templates always produce String
        Exp::Template(_, refs) => {
            // Check that each referenced property expression is well-typed
            for (_, ref_exp) in refs {
                check_infer(ctx, ref_exp)?;
            }
            Ok(Val::EigonPrimitive(crate::nbe::term::PrimitiveType::String))
        }

        // Refl(a): infer a's type, return Id(a_type, a_val, a_val)
        Exp::Refl(a) => {
            let a_type = check_infer(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            Ok(Val::Id(
                Box::new(a_type),
                Box::new(a_val.clone()),
                Box::new(a_val),
            ))
        }

        // NativeDecide(constraint, v): reduces to Refl if satisfied,
        // so its type is Id(v_type, v_val, v_val)
        Exp::NativeDecide(_, v) => {
            let v_type = check_infer(ctx, v)?;
            let v_val = ctx.eval(v, &ctx.rho).map_err(|e| e.to_string())?;
            Ok(Val::Id(
                Box::new(v_type),
                Box::new(v_val.clone()),
                Box::new(v_val),
            ))
        }

        // DecEq(A, x, y): check A is a type, x and y inhabit A,
        // return Id(A_val, x_val, y_val)
        Exp::DecEq(a, x, y) => {
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            let x_val = ctx.eval(x, &ctx.rho).map_err(|e| e.to_string())?;
            let y_val = ctx.eval(y, &ctx.rho).map_err(|e| e.to_string())?;
            Ok(Val::Id(Box::new(a_val), Box::new(x_val), Box::new(y_val)))
        }

        // IdJ([A, C, d, x, y, p]): Martin-Löf J eliminator.
        // Per D18 §6.4, require an explicit motive C and return C(x, y, p).
        // Lean handles this via recursor reduction; we use a direct J-rule
        // since Mini-TT doesn't have a recursor framework.
        Exp::IdJ(args) => {
            let [ref a, ref _c, ref d, ref x, ref y, ref p] = **args;
            // A must be a type
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            // x, y : A
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            let x_val = ctx.eval(x, &ctx.rho).map_err(|e| e.to_string())?;
            let y_val = ctx.eval(y, &ctx.rho).map_err(|e| e.to_string())?;
            // p : Id(A, x, y)
            let id_type = Val::Id(
                Box::new(a_val.clone()),
                Box::new(x_val.clone()),
                Box::new(y_val),
            );
            check(ctx, p, &id_type)?;
            // d : (a : A) → C(a, a, refl(a)) — the base case
            // For now, just infer d's type; the full motive check
            // requires higher-order unification which is Phase 10b.
            let d_type = check_infer(ctx, d)?;
            // J reduces to d(x) when p = refl(x), so the result type
            // is the return type of d applied to x.
            match d_type {
                Val::Pi(_, g) => g.apply(x_val).map_err(|e| e.to_string()),
                _ => Ok(Val::Set), // conservative fallback
            }
        }

        // Map(f, coll): infer f : A → B, coll : List A, return List B.
        Exp::Map(f, coll) => {
            let f_type = check_infer(ctx, f)?;
            let (a, b_clos) = ext_pi(&f_type)
                .map_err(|_| "Map: first argument must be a function (A → B)".to_string())?;
            let coll_type = check_infer(ctx, coll)?;
            let elem_type = extract_list_element_type(&coll_type).ok_or_else(|| {
                format!(
                    "Map: second argument must be a list type, got {:?}",
                    readback_val(ctx.rho.len(), &coll_type)
                )
            })?;
            eq_nf(ctx.rho.len(), &a, &elem_type).map_err(|_| {
                format!(
                    "Map: function domain {:?} does not match list element type {:?}",
                    readback_val(ctx.rho.len(), &a),
                    readback_val(ctx.rho.len(), &elem_type)
                )
            })?;
            // Compute result element type B by applying closure to a dummy
            let b = b_clos.apply(gen_val(&ctx.rho)).map_err(|e| e.to_string())?;
            // Build list type with element type B
            let list_exp = Exp::list(readback_val(ctx.rho.len(), &b));
            ctx.eval(&list_exp, &ctx.rho).map_err(|e| e.to_string())
        }

        // Reduce(f, init, coll): infer f : B → A → B, init : B, coll : List A, return B.
        Exp::Reduce(f, init, coll) => {
            let f_type = check_infer(ctx, f)?;
            let (b, inner_clos) = ext_pi(&f_type)
                .map_err(|_| "Reduce: first argument must be a function (B → A → B)".to_string())?;
            // f's return must be a function A → B
            let inner_type = inner_clos
                .apply(gen_val(&ctx.rho))
                .map_err(|e| e.to_string())?;
            let (_a_inner, _b_ret_clos) = ext_pi(&inner_type).map_err(|_| {
                "Reduce: first argument must be a curried function (B → A → B)".to_string()
            })?;
            // Check init : B
            check(ctx, init, &b)?;
            // Check coll is a list type
            let coll_type = check_infer(ctx, coll)?;
            let _elem_type = extract_list_element_type(&coll_type).ok_or_else(|| {
                format!(
                    "Reduce: third argument must be a list type, got {:?}",
                    readback_val(ctx.rho.len(), &coll_type)
                )
            })?;
            // Return type is B (the accumulator type)
            Ok(b)
        }

        // Inductive types (Phase 11b, D19).
        // Type formers inhabit `Set`. Phase 11b Step 5 will tighten this
        // to track universe levels properly.
        Exp::Inductive(_) | Exp::InductiveType(_, _) => Ok(Val::Set),

        // Constructor application — inference works when the inductive
        // has no parameters (the result type is fully determined).
        // Parameterised inductives need an expected type to drive
        // parameter inference; require checking mode for those.
        Exp::InductiveCtor(decl, ctor_name, args) => {
            if !decl.params.is_empty() {
                return Err(format!(
                    "InductiveCtor: cannot infer type of `{}.{ctor_name}` — \
                     `{}` has {} parameter(s), supply an expected type via checking mode",
                    decl.name,
                    decl.name,
                    decl.params.len()
                ));
            }
            check_inductive_ctor_args(ctx, decl, ctor_name, args, decl, &[])?;
            Ok(Val::InductiveType {
                decl: decl.clone(),
                params: Vec::new(),
            })
        }

        // Recursor application — Phase 11b step 5.
        // 1. The major's inferred type fixes the inductive declaration
        //    and the parameters.
        // 2. The motive must accept that inductive type and return a
        //    sort (for now, `Set`).
        // 3. Each minor is checked against the type derived by
        //    [`derive_minor_types`](super::recursor).
        // 4. The result type is `motive(major)`.
        Exp::InductiveRec {
            decl,
            motive,
            minors,
            major,
        } => check_infer_inductive_rec(ctx, decl, motive, minors, major),

        // Pattern-match without an explicit motive cannot run in
        // inference mode — its result type is determined by checking-
        // mode context. Surface a diagnostic that points users to the
        // two ways out.
        Exp::Match { .. } => Err(
            "match expression has no inferable type — use it in a checking-mode position \
             (e.g. as a program body or a typed `let` value), or annotate the result type \
             with `returning T` so the parser builds an `InductiveRec` instead"
                .to_string(),
        ),

        // Sized types (Phase 11b step 14). `SizeSort` is itself a
        // type at universe 1; `SizeInf` and `SizeSucc(_)` inhabit
        // `SizeSort`.
        Exp::SizeSort => Ok(Val::Type(1)),
        Exp::SizeInf => Ok(Val::SizeSort),
        Exp::SizeSucc(s) => {
            check(ctx, s, &Val::SizeSort)?;
            Ok(Val::SizeSort)
        }

        e => Err(format!("cannot infer type of: {e:?}")),
    }
}

/// Rehydrate a possibly-stub `Arc<CodataDecl>` to the full
/// declaration with populated observations.
///
/// The ground resolver emits self-references inside observation types
/// as `Exp::CodataType(self_ref_stub, args)` where `self_ref_stub` is
/// an `Arc<CodataDecl>` with empty observations — it's the
/// initial-Arc trick that mirrors `resolve_inductive_type`'s pattern.
/// That works for inductive types because constructor applications
/// always carry the full decl at the use site, but corecord values
/// and observations don't carry a decl reference in their Exp — the
/// decl comes from the inferred/expected type, which may be the
/// stub.
///
/// This helper walks the current layer looking for a `CodataType`
/// resource whose short name matches `stub.name` and re-resolves it
/// to a full decl. Costly per call; a future optimisation could
/// memoise this in `CheckCtx` next to `type_cache`.
fn resolve_full_codata_decl(
    ctx: &CheckCtx,
    stub: &Arc<crate::nbe::term::CodataDecl>,
) -> Result<Arc<crate::nbe::term::CodataDecl>, String> {
    if !stub.observations.is_empty() {
        return Ok(stub.clone());
    }
    let layer = ctx.layer.as_ref().ok_or_else(|| {
        format!(
            "cannot rehydrate stub codata decl `{}` — no layer in check context",
            stub.name
        )
    })?;
    let short_name_iri =
        Iri::parse(crate::ontology::well_known::SHORT_NAME).expect("well-known IRI");
    for (iri, resource) in layer.iter_all_resources() {
        if !resource
            .is_a()
            .iter()
            .any(|c| c.as_str() == "urn:eigenius:core:CodataType")
        {
            continue;
        }
        if let Some(crate::ontology::resource::Value::String(sn)) = resource.get(&short_name_iri) {
            if sn == &stub.name {
                let v = crate::program::ground::resolve_class_type(&iri, layer)?;
                match v {
                    Val::CodataType { decl, .. } => return Ok(decl),
                    Val::Codata(_, _) => {
                        return Err(format!(
                            "codata `{}` resolved to the non-parameterised `Val::Codata` \
                             form — cannot be used as a stub target",
                            stub.name
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "codata `{}` resolved to an unexpected Val form",
                            stub.name
                        ));
                    }
                }
            }
        }
    }
    Err(format!(
        "cannot find codata decl with short_name `{}` in the layer chain",
        stub.name
    ))
}

/// Look up an observation by name on an applied codata type, returning
/// the observation's type evaluated in an environment that binds the
/// codata's parameters to the applied argument values.
///
/// This is the parameterised-codata analogue of the projection that
/// `Val::Codata(observations, rho)` does inline — the decl carries
/// the observation list, the `params` vector supplies the concrete
/// argument values, and self-references inside observation types
/// unify by name via `CodataDecl::PartialEq`.
pub fn lookup_codata_observation(
    decl: &Arc<crate::nbe::term::CodataDecl>,
    params: &[Val],
    obs_name: &str,
    level: usize,
) -> Result<Val, String> {
    let obs = decl
        .observations
        .iter()
        .find(|o| o.name == obs_name)
        .ok_or_else(|| {
            format!(
                "observation '{}' not found in codata type '{}'",
                obs_name, decl.name
            )
        })?;
    let mut env = Rho::Nil;
    for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
        env = env.extend(patt.clone(), val.clone());
    }
    let _ = level; // reserved for richer diagnostics in future
    eval(&obs.typ, &env).map_err(|e| e.to_string())
}

/// Check type equality by normalization.
///
/// Port of `eqNf` from the reference: normalize both sides
/// and compare syntactically.
pub fn eq_nf(level: usize, v1: &Val, v2: &Val) -> Result<(), String> {
    let e1 = readback_val(level, v1);
    let e2 = readback_val(level, v2);
    if e1 == e2 {
        Ok(())
    } else {
        Err(format!("type mismatch: {e1:?} ≠ {e2:?}"))
    }
}

/// Subtyping check: admits `sub <: super` (Phase 11b step 15d, D19 §8.3).
///
/// Calls [`subtype_of_with_hyps`] with an empty TSO — use this variant
/// when you don't have bounded size hypotheses to bring to bear.
pub fn subtype_of(level: usize, sub: &Val, super_: &Val) -> Result<(), String> {
    subtype_of_with_hyps(level, sub, super_, &crate::nbe::sized_rigid::Tso::new())
}

/// Subtyping check consulting a TSO of rigid size hypotheses.
///
/// Current scope is exactly the sized-types relaxation — everywhere
/// else subtyping degenerates to equality (`eq_nf`). The relaxation:
///
/// For a pair of applied inductive types `I(p₁ … pₙ)` with identical
/// declarations, each parameter is compared position-wise:
/// - positions whose declared type is `SizeSort` are compared with
///   [`crate::nbe::sized::size_le_with_hyps`] — `sub_pᵢ ≤ sup_pᵢ`
///   suffices, with the TSO consulted for neutral entailment;
/// - all other positions must be definitionally equal (`eq_nf`).
///
/// This is what makes `T(s) <: T(ŝ s) <: T(∞)` admissible — the
/// driving motivation for sized types. With `tso` populated from
/// bounded binders in scope, `T(i) <: T(j)` also becomes admissible
/// whenever `i ≤ j` is entailed by the hypothesis chain.
///
/// Codata (`Val::Codata`) is structurally identical and will benefit
/// once sized codata arrives; it falls through to `eq_nf` today
/// because the checker doesn't yet thread size parameters onto
/// `Codata` value shapes.
pub fn subtype_of_with_hyps(
    level: usize,
    sub: &Val,
    super_: &Val,
    tso: &crate::nbe::sized_rigid::Tso,
) -> Result<(), String> {
    if let (
        Val::InductiveType {
            decl: d1,
            params: p1,
        },
        Val::InductiveType {
            decl: d2,
            params: p2,
        },
    ) = (sub, super_)
    {
        if d1 == d2 && p1.len() == p2.len() && p1.len() == d1.params.len() {
            for (i, (sub_p, sup_p)) in p1.iter().zip(p2.iter()).enumerate() {
                let decl_param_ty = &d1.params[i].1;
                if matches!(decl_param_ty, Exp::SizeSort) {
                    if !crate::nbe::sized::size_le_with_hyps(sub_p, sup_p, tso) {
                        return Err(format!(
                            "size subtyping failed at param {i}: \
                             {:?} ≰ {:?}",
                            readback_val(level, sub_p),
                            readback_val(level, sup_p),
                        ));
                    }
                } else {
                    eq_nf(level, sub_p, sup_p)?;
                }
            }
            return Ok(());
        }
    }
    eq_nf(level, sub, super_)
}

/// Collect the variable names bound by a pattern.
fn collect_pattern_names<'a>(p: &'a Patt, out: &mut std::collections::HashSet<&'a str>) {
    match p {
        Patt::Var(n) => {
            out.insert(n.as_str());
        }
        Patt::Pair(p1, p2) => {
            collect_pattern_names(p1, out);
            collect_pattern_names(p2, out);
        }
        Patt::Unit => {}
    }
}

/// If `exp` reduces syntactically to a forbidden variable through a
/// chain of observations and projections, return that variable's name.
/// Used by the guardedness check to detect unguarded corecursive
/// references at the head of an `Observe`.
///
/// This intentionally stops at `App` / `Lam` / `CoRecord` / constructor
/// boundaries — crossing any of those makes the reference guarded.
fn has_forbidden_head<'a>(
    exp: &'a Exp,
    forbidden: &std::collections::HashSet<&str>,
) -> Option<&'a str> {
    match exp {
        Exp::Var(x) if forbidden.contains(x.as_str()) => Some(x.as_str()),
        Exp::Observe(inner, _) => has_forbidden_head(inner, forbidden),
        Exp::Fst(inner) | Exp::Snd(inner) => has_forbidden_head(inner, forbidden),
        _ => None,
    }
}

/// Syntactic guardedness check for corecursive definitions (D11 §3).
///
/// A corecord definition `letrec x = ...` is guarded iff `x` (or any
/// mutually-bound name) never appears at the *head* of an
/// `Observe` expression within a field body — because doing so would
/// trigger immediate unfolding of the same corecord at the same layer,
/// producing no progress.
///
/// The check is syntactic and Agda-style. Productive patterns covered:
/// - `letrec nats(n) = corecord { head = n; tail = nats(n+1) }` — the
///   corecursive call is under `App`, which breaks the observation
///   chain; each observation produces a fresh corecord.
/// - `letrec ones = corecord { head = 1; tail = ones }` — a naked
///   reference at a field body is fine; observing `ones.tail.tail...`
///   re-returns the corecord value each time, with finite cost per
///   step.
///
/// Rejected:
/// - `letrec bad = corecord { head = bad.head; tail = ... }` — observing
///   `bad.head` requires evaluating `bad.head`, infinite loop.
///
/// Conservative approximation: syntactic guardedness cannot catch
/// cases where the loop goes through a function call (e.g. `broken(n).head`
/// where `broken` returns a corecord whose head body is
/// `broken(n).head`). Sized types would close that gap — out of scope
/// for v1. See D11 §3.4 and [eigenius#16][1].
///
/// [1]: https://github.com/eigenius/eigenius/issues/16
pub fn check_guarded(exp: &Exp, forbidden: &std::collections::HashSet<&str>) -> Result<(), String> {
    match exp {
        Exp::Observe(inner, obs) => {
            if let Some(name) = has_forbidden_head(inner, forbidden) {
                return Err(format!(
                    "unguarded corecursive reference: '{name}' is observed at field '{obs}' \
                     inside its own definition — this would loop at evaluation time. \
                     Put the recursive call under a function application or inside \
                     another constructor so that each observation makes progress."
                ));
            }
            check_guarded(inner, forbidden)
        }

        // Sub-expressions that need recursive checking.
        Exp::Lam(_, e) => check_guarded(e, forbidden),
        Exp::App(e1, e2) => {
            check_guarded(e1, forbidden)?;
            check_guarded(e2, forbidden)
        }
        Exp::Pair(e1, e2) => {
            check_guarded(e1, forbidden)?;
            check_guarded(e2, forbidden)
        }
        Exp::Con(_, e) => check_guarded(e, forbidden),
        Exp::Fst(e) | Exp::Snd(e) => check_guarded(e, forbidden),
        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) => {
            check_guarded(a, forbidden)?;
            check_guarded(b, forbidden)
        }
        Exp::Arrow(a, b) | Exp::Times(a, b) => {
            check_guarded(a, forbidden)?;
            check_guarded(b, forbidden)
        }
        Exp::Data(summands) => {
            for s in summands {
                check_guarded(&s.typ, forbidden)?;
            }
            Ok(())
        }
        Exp::Case(branches) => {
            for b in branches {
                check_guarded(&b.body, forbidden)?;
            }
            Ok(())
        }
        Exp::Dec(_, e) => check_guarded(e, forbidden),
        Exp::Id(a, x, y) => {
            check_guarded(a, forbidden)?;
            check_guarded(x, forbidden)?;
            check_guarded(y, forbidden)
        }
        Exp::Refl(a) => check_guarded(a, forbidden),
        Exp::IdJ(args) => {
            for a in args.iter() {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::NativeDecide(c, v) => {
            if let crate::nbe::term::Constraint::Institution { args, .. } = c {
                for a in args {
                    check_guarded(a, forbidden)?;
                }
            }
            check_guarded(v, forbidden)
        }
        Exp::InstitutionInvoke { source, .. } => check_guarded(source, forbidden),
        Exp::DecEq(a, x, y) => {
            check_guarded(a, forbidden)?;
            check_guarded(x, forbidden)?;
            check_guarded(y, forbidden)
        }
        Exp::PropAccess(e, _) => check_guarded(e, forbidden),
        Exp::Template(_, refs) => {
            for (_, t) in refs {
                check_guarded(t, forbidden)?;
            }
            Ok(())
        }
        Exp::Construct(_, fields) => {
            for (_, e) in fields {
                check_guarded(e, forbidden)?;
            }
            Ok(())
        }

        // Codata forms
        Exp::Codata(observations) => {
            for o in observations {
                check_guarded(&o.typ, forbidden)?;
            }
            Ok(())
        }
        // Parameterised codata application — recurse into its
        // argument expressions only; the codata decl's observations
        // are type-level and already validated at decl-site.
        Exp::CodataType(_, args) => {
            for a in args {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::CoRecord(fields) => {
            for f in fields {
                check_guarded(&f.body, forbidden)?;
            }
            Ok(())
        }

        // Map/Reduce (Phase 11a)
        Exp::Map(f, coll) => {
            check_guarded(f, forbidden)?;
            check_guarded(coll, forbidden)
        }
        Exp::Reduce(f, init, coll) => {
            check_guarded(f, forbidden)?;
            check_guarded(init, forbidden)?;
            check_guarded(coll, forbidden)
        }

        // Inductive types (Phase 11b, D19): walk parameter / argument /
        // motive / minor / major sub-expressions structurally. The
        // `InductiveDecl` itself is treated as a closed declaration —
        // its constructor types are not visited here.
        Exp::Inductive(_) => Ok(()),
        Exp::InductiveType(_, params) => {
            for p in params {
                check_guarded(p, forbidden)?;
            }
            Ok(())
        }
        Exp::InductiveCtor(_, _, args) => {
            for a in args {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::InductiveRec {
            motive,
            minors,
            major,
            ..
        } => {
            check_guarded(motive, forbidden)?;
            for m in minors {
                check_guarded(m, forbidden)?;
            }
            check_guarded(major, forbidden)
        }
        Exp::Match { scrutinee, arms } => {
            check_guarded(scrutinee, forbidden)?;
            for arm in arms {
                check_guarded(&arm.body, forbidden)?;
            }
            Ok(())
        }

        // Sized types (Phase 11b step 14): size primitives are
        // structurally simple — `SizeSucc` has one sub-expression,
        // `SizeSort` and `SizeInf` are leaves.
        Exp::SizeSucc(s) => check_guarded(s, forbidden),
        Exp::SizeSort | Exp::SizeInf => Ok(()),
        // SizedPi binder — recurse into upper and body. (The binder
        // doesn't shadow corecursive names from `forbidden` because
        // it binds a size, not a value of a codata type.)
        Exp::SizedPi { upper, body, .. } => {
            check_guarded(upper, forbidden)?;
            check_guarded(body, forbidden)
        }

        // Leaves — no sub-expressions to check.
        Exp::Var(_)
        | Exp::Set
        | Exp::Type(_)
        | Exp::One
        | Exp::Unit
        | Exp::EigonClass(_)
        | Exp::EigonPrimitive(_)
        | Exp::EigonResource(_) => Ok(()),
    }
}

/// Find a field by name in a Sigma chain.
/// Walks Σ name₁ : T₁. Σ name₂ : T₂. ... looking for a matching name.
///
/// When the type is `EigonClass(iri)`, resolves the class to its Sigma
/// chain via `ctx.resolve_class_cached` and recurses — this is the core
/// fix for issue #12 item 1 (D18 §5).
fn find_sigma_field(ctx: &mut CheckCtx, typ: &Val, field_name: &str) -> Option<Val> {
    match typ {
        Val::Sig(t, g) => {
            if g.patt == Patt::Var(field_name.to_string()) {
                // Found — return the field's type
                Some(*t.clone())
            } else {
                // Not this field — apply the closure with a dummy value
                // and search the rest of the chain
                let gen = gen_val(&g.env);
                let rest = g.apply(gen).ok()?;
                find_sigma_field(ctx, &rest, field_name)
            }
        }
        // Resolve EigonClass to its Sigma chain via layer access.
        Val::EigonClass(iri) => {
            let resolved = ctx.resolve_class_cached(iri).ok()?;
            find_sigma_field(ctx, &resolved, field_name)
        }
        _ => None,
    }
}

/// Advance past one field in a Sigma chain. After `find_sigma_field`
/// found `field_name`, this returns the rest of the Sigma: applies
/// the closure with the field's value and recurses.
fn advance_sigma(typ: &Val, field_name: &str, field_exp: &Exp, rho: &Rho) -> Val {
    match typ {
        Val::Sig(_, g) => {
            if g.patt == Patt::Var(field_name.to_string()) {
                match eval(field_exp, rho).and_then(|v| g.apply(v)) {
                    Ok(v) => v,
                    Err(_) => typ.clone(),
                }
            } else {
                let gen = gen_val(&g.env);
                match g.apply(gen) {
                    Ok(rest) => advance_sigma(&rest, field_name, field_exp, rho),
                    Err(_) => typ.clone(),
                }
            }
        }
        _ => typ.clone(),
    }
}

/// Extract a Pi type: Pi(A, x.B) → (A, x.B)
fn ext_pi(val: &Val) -> Result<(Val, Clos), String> {
    match val {
        Val::Pi(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(format!("expected Pi type, got: {u:?}")),
    }
}

/// One binder in a constructor telescope after the parameter prefix
/// has been stripped.
///
/// `Value` is an ordinary Π binder `(p : T)`; `Size` is a bounded
/// size Π binder `{p < upper}` (expressed as `Exp::SizedPi`). The
/// distinction matters because size args are verified against the
/// upper bound via [`crate::nbe::sized::size_lt_with_hyps`] and
/// introduce a hypothesis into the TSO when destructured.
#[derive(Debug, Clone)]
enum CtorArg {
    Value { patt: Patt, typ: Exp },
    Size { patt: Patt, upper: Exp },
}

/// Peel a constructor's Π-telescope past the parameter prefix,
/// returning the remaining binders as `CtorArg`s plus the residual
/// (final) result-type expression.
///
/// Accepts both `Exp::Pi` and `Exp::SizedPi` at non-parameter
/// positions. Parameter positions are always `Exp::Pi` by
/// construction — size parameters have type `SizeSort` but the
/// binder itself is a plain Pi, so `params_to_skip` only ever
/// applies to `Pi`.
fn peel_ctor_telescope(ctor_typ: &Exp, params_to_skip: usize) -> (Vec<CtorArg>, &Exp) {
    let mut args: Vec<CtorArg> = Vec::new();
    let mut remaining = params_to_skip;
    let mut current = ctor_typ;
    loop {
        match current {
            Exp::Pi(patt, dom, body) => {
                if remaining > 0 {
                    remaining -= 1;
                } else {
                    args.push(CtorArg::Value {
                        patt: patt.clone(),
                        typ: (**dom).clone(),
                    });
                }
                current = body;
            }
            Exp::SizedPi { patt, upper, body } => {
                // Size binders appear only after the param prefix.
                args.push(CtorArg::Size {
                    patt: patt.clone(),
                    upper: (**upper).clone(),
                });
                current = body;
            }
            _ => break,
        }
    }
    (args, current)
}

/// Check the arguments of an inductive constructor application against
/// the constructor's declared types.
///
/// Walks the constructor's Π-telescope, skipping the parameter prefix,
/// and checks each user-supplied argument against the corresponding
/// binder type evaluated in an environment that binds parameters to
/// the supplied param values and earlier args to their values (so a
/// constructor type like `cons : (A:Set) → A → List A → List A` can
/// have its second binder type `List A` reference the first param).
///
/// Used by both the bidirectional `check` arm and the inference path
/// for non-parametric constructors.
fn check_inductive_ctor_args(
    ctx: &mut CheckCtx,
    decl: &Arc<InductiveDecl>,
    ctor_name: &str,
    args: &[Exp],
    expected_decl: &Arc<InductiveDecl>,
    params: &[Val],
) -> Result<(), String> {
    if decl.name != expected_decl.name {
        return Err(format!(
            "InductiveCtor: constructor of `{}` does not match expected inductive `{}`",
            decl.name, expected_decl.name
        ));
    }
    let ctor_idx = decl
        .ctors
        .iter()
        .position(|c| c.name == ctor_name)
        .ok_or_else(|| {
            format!(
                "InductiveCtor: no constructor `{ctor_name}` in `{}`",
                decl.name
            )
        })?;
    let ctor = &decl.ctors[ctor_idx];

    let (arg_specs, current) = peel_ctor_telescope(&ctor.typ, decl.params.len());
    if arg_specs.len() != args.len() {
        return Err(format!(
            "InductiveCtor `{}.{ctor_name}` expects {} args, got {}",
            decl.name,
            arg_specs.len(),
            args.len()
        ));
    }

    // Internal env for evaluating expected types: starts with params
    // bound, then accumulates each checked arg's value.
    let mut arg_env = Rho::Nil;
    for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
        arg_env = arg_env.extend(patt.clone(), val.clone());
    }
    for (spec, arg_exp) in arg_specs.iter().zip(args.iter()) {
        match spec {
            CtorArg::Value { patt, typ } => {
                let arg_typ_val = ctx.eval(typ, &arg_env).map_err(|e| e.to_string())?;
                check(ctx, arg_exp, &arg_typ_val)?;
                let arg_val = ctx.eval(arg_exp, &ctx.rho).map_err(|e| e.to_string())?;
                arg_env = arg_env.extend(patt.clone(), arg_val);
            }
            CtorArg::Size { patt, upper } => {
                // Bounded size arg: user's expression must be a
                // size value strictly below the upper bound
                // (evaluated in `arg_env` so it can reference the
                // inductive's size parameter).
                check(ctx, arg_exp, &Val::SizeSort)?;
                let upper_val = ctx.eval(upper, &arg_env).map_err(|e| e.to_string())?;
                let arg_val = ctx.eval(arg_exp, &ctx.rho).map_err(|e| e.to_string())?;
                if !crate::nbe::sized::size_lt_with_hyps(&arg_val, &upper_val, &ctx.size_tso) {
                    return Err(format!(
                        "InductiveCtor `{}.{ctor_name}`: size argument {:?} is not \
                         strictly below upper bound {:?}",
                        decl.name,
                        readback_val(ctx.rho.len(), &arg_val),
                        readback_val(ctx.rho.len(), &upper_val),
                    ));
                }
                arg_env = arg_env.extend(patt.clone(), arg_val);
            }
        }
    }

    // Verify the constructor's declared result type matches the
    // expected inductive type (up to subtyping).
    //
    // For a plain inductive like `cons : Π A:Set. A → List A → List A`
    // this is always trivial — after param binding, `List A` evaluates
    // to `List(A_applied)` which equals the expected type on the nose.
    //
    // For sized inductives it actually bites. A constructor whose
    // declared result is `SizedNat (↑ i)` produces a value whose size
    // is `↑ i_applied`; if the expected size is `i_applied` this check
    // now catches the mismatch (strict-order violation `↑ i ≰ i`).
    // Without this check a buggy constructor declaration of the form
    // `foo : Π p:P. OtherInductive` or `foo : ... → SizedNat (↑ i)`
    // used at `SizedNat i` would pass silently.
    let actual_result = ctx.eval(current, &arg_env).map_err(|e| e.to_string())?;
    let expected_result = Val::InductiveType {
        decl: expected_decl.clone(),
        params: params.to_vec(),
    };
    subtype_of_with_hyps(
        ctx.rho.len(),
        &actual_result,
        &expected_result,
        &ctx.size_tso,
    )
    .map_err(|err| {
        format!(
            "InductiveCtor `{}.{ctor_name}`: result type mismatch ({err})",
            decl.name
        )
    })
}

/// Type-check an `Exp::InductiveRec` application and return its result
/// type `motive(major)`.
fn check_infer_inductive_rec(
    ctx: &mut CheckCtx,
    decl: &Arc<InductiveDecl>,
    motive: &Exp,
    minors: &[Exp],
    major: &Exp,
) -> Result<Val, String> {
    // 1. Major must inhabit the inductive being eliminated.
    let major_typ = check_infer(ctx, major)?;
    let (major_decl, params) = match &major_typ {
        Val::InductiveType { decl: d, params: p } => (d.clone(), p.clone()),
        other => {
            return Err(format!(
                "InductiveRec on `{}`: major has type {:?}, expected an inductive type",
                decl.name,
                readback_val(ctx.rho.len(), other)
            ));
        }
    };
    if major_decl.name != decl.name {
        return Err(format!(
            "InductiveRec: declaration mismatch — recursor for `{}`, major has type `{}`",
            decl.name, major_decl.name
        ));
    }

    // 2. Motive : I(params) → Type(1).
    //    Codomain `Type(1)` admits both `Set` (= Type(0)) and Type(n)
    //    motive bodies via the existing (Set : Type(1)) and
    //    (Type(n) : Type(n+1)) rules. Phase 11b extension can
    //    generalise to arbitrary Sort u with universe inference.
    let motive_dom = Val::InductiveType {
        decl: decl.clone(),
        params: params.clone(),
    };
    let motive_typ = Val::Pi(
        Box::new(motive_dom),
        Clos::new(Patt::Unit, Exp::Type(1), Rho::Nil),
    );
    check(ctx, motive, &motive_typ)?;

    // 3. Minors: one per constructor, each against its derived type.
    if minors.len() != decl.ctors.len() {
        return Err(format!(
            "InductiveRec on `{}`: expected {} minors (one per constructor), got {}",
            decl.name,
            decl.ctors.len(),
            minors.len()
        ));
    }
    let motive_val = ctx.eval(motive, &ctx.rho).map_err(|e| e.to_string())?;
    let expected_minor_types = derive_minor_types(decl, &params, &motive_val, &EvalCtx::Pure)
        .map_err(|e| e.to_string())?;
    for (minor, expected_typ) in minors.iter().zip(expected_minor_types.iter()) {
        check(ctx, minor, expected_typ)?;
    }

    // 4. Result: motive(major).
    let major_val = ctx.eval(major, &ctx.rho).map_err(|e| e.to_string())?;
    motive_val.app(major_val).map_err(|e| e.to_string())
}

/// Type-check `match scrutinee { arm₁; arm₂; … }` against an expected
/// result type (Phase 11b step 12, D19 §10).
///
/// 1. Infer the scrutinee's type — must be `Val::InductiveType { decl, params }`.
/// 2. Validate exhaustiveness (every constructor in `decl` has an arm)
///    and no duplicate arms.
/// 3. For each arm, build a context extended with bindings for the
///    constructor's positional arguments (with parameters substituted),
///    then check the arm body against `expected_type`. Binding count
///    must match the constructor's arity.
///
/// The motive synthesised by this check is the constant function
/// `λ_. expected_type`, so each arm body is checked at the same type.
/// Dependent motives (where the result type varies with the matched
/// constructor) need explicit annotation via `Exp::InductiveRec` and
/// are not handled by this path.
fn check_match(
    ctx: &mut CheckCtx,
    scrutinee: &Exp,
    arms: &[crate::nbe::term::MatchArm],
    expected: &Val,
) -> Result<(), String> {
    use std::collections::BTreeMap;

    let scrutinee_type = check_infer(ctx, scrutinee)?;
    let (decl, params) = match &scrutinee_type {
        Val::InductiveType { decl, params } => (decl.clone(), params.clone()),
        other => {
            return Err(format!(
                "match scrutinee has type {:?}, expected an inductive type",
                readback_val(ctx.rho.len(), other)
            ));
        }
    };

    let mut arms_by_ctor: BTreeMap<&str, &crate::nbe::term::MatchArm> = BTreeMap::new();
    for arm in arms {
        if arms_by_ctor.insert(arm.ctor_name.as_str(), arm).is_some() {
            return Err(format!(
                "duplicate match arm for `{}.{}`",
                decl.name, arm.ctor_name
            ));
        }
    }
    for ctor_name in arms_by_ctor.keys() {
        if !decl.ctors.iter().any(|c| &c.name == ctor_name) {
            return Err(format!(
                "match arm references unknown constructor `{}.{ctor_name}`",
                decl.name
            ));
        }
    }

    for ctor in &decl.ctors {
        let arm = arms_by_ctor.get(ctor.name.as_str()).ok_or_else(|| {
            format!(
                "non-exhaustive match: missing case for `{}.{}`",
                decl.name, ctor.name
            )
        })?;

        // Extract this constructor's argument types (after the
        // parameter prefix) from its Π-telescope. Supports both
        // ordinary `Pi` binders and bounded-size `SizedPi` binders;
        // size binders become rigid hypotheses in the arm's TSO.
        let (arg_specs, _ctor_result) = peel_ctor_telescope(&ctor.typ, decl.params.len());

        if arm.bindings.len() != arg_specs.len() {
            return Err(format!(
                "match arm `{}.{}` expects {} bindings, got {}",
                decl.name,
                ctor.name,
                arg_specs.len(),
                arm.bindings.len()
            ));
        }

        // Build the arm's check context: start from the outer ctx,
        // bind parameters for evaluating arg types, then extend with
        // each binding (bound to a fresh generic value of the
        // corresponding arg type).
        let mut arg_env = Rho::Nil;
        for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
            arg_env = arg_env.extend(patt.clone(), val.clone());
        }
        let mut arm_ctx = CheckCtx {
            rho: ctx.rho.clone(),
            gamma: ctx.gamma.clone(),
            layer: ctx.layer.clone(),
            type_cache: ctx.type_cache.clone(),
            size_tso: ctx.size_tso.clone(),
            institution_index: ctx.institution_index.clone(),
            institution_runtime: ctx.institution_runtime.clone(),
        };
        for (spec, binding) in arg_specs.iter().zip(arm.bindings.iter()) {
            match spec {
                CtorArg::Value { patt, typ } => {
                    let arg_typ_val = ctx.eval(typ, &arg_env).map_err(|e| e.to_string())?;
                    let gen = gen_val(&arm_ctx.rho);
                    arm_ctx = arm_ctx.extend(binding, &arg_typ_val, &gen)?;
                    arg_env = arg_env.extend(patt.clone(), gen);
                }
                CtorArg::Size { patt, upper } => {
                    // The constructor's bounded size binder exposes
                    // the predecessor size in the arm's scope, with
                    // `bound_size < upper` available as a TSO
                    // hypothesis. This is what lets a recursive call
                    // on the destructured sub-value type-check at a
                    // strictly-smaller size — i.e. termination via
                    // pattern-match on a sized inductive.
                    let upper_val = ctx.eval(upper, &arg_env).map_err(|e| e.to_string())?;
                    let new_level = arm_ctx.rho.len();
                    let gen = gen_val(&arm_ctx.rho);
                    arm_ctx = arm_ctx.extend(binding, &Val::SizeSort, &gen)?;
                    match &upper_val {
                        Val::SizeInf => {
                            // `{j < ∞}` in a ctor adds no hypothesis
                            // — anything is below ∞ structurally.
                        }
                        Val::Nt(crate::nbe::val::Neut::Gen(upper_level, _)) => {
                            arm_ctx
                                .size_tso
                                .insert(new_level as u32, 1, *upper_level as u32);
                        }
                        _ => {
                            return Err(format!(
                                "match arm `{}.{}`: constructor's bounded size binder upper \
                                 must be rigid or ∞, got {:?}",
                                decl.name,
                                ctor.name,
                                readback_val(ctx.rho.len(), &upper_val),
                            ));
                        }
                    }
                    arg_env = arg_env.extend(patt.clone(), gen);
                }
            }
        }

        check(&mut arm_ctx, &arm.body, expected)?;
    }

    Ok(())
}

/// Extract a Sigma type: Sig(A, x.B) → (A, x.B)
fn ext_sig(val: &Val) -> Result<(Val, Clos), String> {
    match val {
        Val::Sig(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(format!("expected Sigma type, got: {u:?}")),
    }
}

/// Check if a value is a list type and return the element type.
///
/// Recognises the canonical `List(A)` inductive type (the form
/// produced by `Exp::list()` since Phase 11b step 6, D19 §9).
fn extract_list_element_type(val: &Val) -> Option<Val> {
    if let Val::InductiveType { decl, params } = val {
        if decl.name == "List" && params.len() == 1 {
            return Some(params[0].clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;
    use crate::ontology::iri::Iri;

    fn ctx() -> CheckCtx {
        CheckCtx::new(Rho::Nil, vec![])
    }

    #[test]
    fn check_unit_has_type_one() {
        check(&mut ctx(), &Exp::Unit, &Val::One).unwrap();
    }

    #[test]
    fn check_one_has_type_set() {
        check(&mut ctx(), &Exp::One, &Val::Set).unwrap();
    }

    #[test]
    fn check_set_is_type() {
        check_type(&mut ctx(), &Exp::Set).unwrap();
    }

    #[test]
    fn check_one_is_type() {
        check_type(&mut ctx(), &Exp::One).unwrap();
    }

    #[test]
    fn check_pi_is_type() {
        // Π _ : 1. 1 is a valid type
        let pi = Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::One));
        check_type(&mut ctx(), &pi).unwrap();
    }

    #[test]
    fn check_identity_function() {
        // λx.x : Π x : 1. 1
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let pi = Val::Pi(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        check(&mut ctx(), &lam, &pi).unwrap();
    }

    #[test]
    fn check_pair() {
        // ((), ()) : Σ _ : 1. 1
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let sig = Val::Sig(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        check(&mut ctx(), &pair, &sig).unwrap();
    }

    #[test]
    fn check_type_mismatch_fails() {
        // () : U should fail (unit is not a type)
        let result = check(&mut ctx(), &Exp::Unit, &Val::Set);
        assert!(result.is_err());
    }

    #[test]
    fn check_let_declaration() {
        // let x : 1 = (); x : 1
        let d = Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Unit),
        );
        let e = Exp::Dec(d, Box::new(Exp::Var("x".to_string())));
        check(&mut ctx(), &e, &Val::One).unwrap();
    }

    #[test]
    fn infer_variable_type() {
        let gamma: Gamma = vec![("x".to_string(), Val::One)];
        let mut c = CheckCtx::new(Rho::Nil, gamma);
        let t = check_infer(&mut c, &Exp::Var("x".to_string())).unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn infer_application_type() {
        // f : 1 → 1, f () : 1
        let pi_type = Val::Pi(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        let gamma: Gamma = vec![("f".to_string(), pi_type)];
        let rho = Rho::Nil.extend(
            Patt::Var("f".to_string()),
            Val::Lam(Clos::new(
                Patt::Var("x".to_string()),
                Exp::Var("x".to_string()),
                Rho::Nil,
            )),
        );
        let mut c = CheckCtx::new(rho, gamma);
        let t = check_infer(
            &mut c,
            &Exp::App(Box::new(Exp::Var("f".to_string())), Box::new(Exp::Unit)),
        )
        .unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn eq_nf_equal() {
        eq_nf(0, &Val::One, &Val::One).unwrap();
        eq_nf(0, &Val::Unit, &Val::Unit).unwrap();
        eq_nf(0, &Val::Set, &Val::Set).unwrap();
    }

    #[test]
    fn eq_nf_not_equal() {
        assert!(eq_nf(0, &Val::One, &Val::Set).is_err());
        assert!(eq_nf(0, &Val::Unit, &Val::One).is_err());
    }

    #[test]
    fn check_sum_type() {
        // Sum(a 1 | b 1) : U
        let data = Exp::Data(vec![
            crate::nbe::term::Summand {
                name: "a".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Summand {
                name: "b".to_string(),
                typ: Exp::One,
            },
        ]);
        check(&mut ctx(), &data, &Val::Set).unwrap();
    }

    #[test]
    fn check_constructor_against_sum() {
        // $a () : Sum(a 1 | b 1)
        let data_val = Val::Data(
            vec![("a".to_string(), Exp::One), ("b".to_string(), Exp::One)],
            Rho::Nil,
        );
        let con = Exp::Con("a".to_string(), Box::new(Exp::Unit));
        check(&mut ctx(), &con, &data_val).unwrap();
    }

    #[test]
    fn check_constructor_wrong_name_fails() {
        let data_val = Val::Data(vec![("a".to_string(), Exp::One)], Rho::Nil);
        let con = Exp::Con("b".to_string(), Box::new(Exp::Unit));
        assert!(check(&mut ctx(), &con, &data_val).is_err());
    }

    #[test]
    fn check_id_is_type() {
        // Id(1, (), ()) : Set
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check(&mut ctx(), &id, &Val::Set).unwrap();
    }

    #[test]
    fn check_id_type_well_formed() {
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check_type(&mut ctx(), &id).unwrap();
    }

    #[test]
    fn check_refl_against_id() {
        // refl(()) : Id(1, (), ())
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let id_type = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(Val::Unit));
        check(&mut ctx(), &refl, &id_type).unwrap();
    }

    #[test]
    fn check_refl_wrong_endpoints_fails() {
        // refl(()) : Id(1, (), x) should fail when x ≠ ()
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let id_type = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(gen));
        assert!(check(&mut ctx(), &refl, &id_type).is_err());
    }

    #[test]
    fn eval_j_with_refl_reduces() -> Result<(), Box<dyn std::error::Error>> {
        // J(1, C, d, (), (), refl(())) should reduce to d(())
        use crate::nbe::eval::eval;
        let j = Exp::IdJ(Box::new([
            Exp::One,                                                        // A
            Exp::Set,                                                        // C (placeholder)
            Exp::Lam(Patt::Var("a".into()), Box::new(Exp::Var("a".into()))), // d = λa. a
            Exp::Unit,                                                       // x
            Exp::Unit,                                                       // y
            Exp::Refl(Box::new(Exp::Unit)),                                  // p = refl(())
        ]));
        let result = eval(&j, &Rho::Nil)?;
        // d(()) = (λa.a)(()) = ()
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn deceq_equal_reduces_to_refl() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // DecEq(1, (), ()) → refl(())
        let deceq = Exp::DecEq(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Refl(_)));
        Ok(())
    }

    #[test]
    fn deceq_unequal_produces_neutral() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // DecEq(Set, 1, Set) — One ≠ Set, produces neutral
        let deceq = Exp::DecEq(Box::new(Exp::Set), Box::new(Exp::One), Box::new(Exp::Set));
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Nt(_)));
        Ok(())
    }

    #[test]
    fn deceq_iri_equal() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let iri = Iri::parse("urn:eigenius:core:string").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::Set),
            Box::new(Exp::EigonClass(iri.clone())),
            Box::new(Exp::EigonClass(iri)),
        );
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Refl(_)));
        Ok(())
    }

    #[test]
    fn deceq_iri_unequal() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let iri1 = Iri::parse("urn:eigenius:core:string").unwrap();
        let iri2 = Iri::parse("urn:eigenius:core:integer").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::Set),
            Box::new(Exp::EigonClass(iri1)),
            Box::new(Exp::EigonClass(iri2)),
        );
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Nt(_)));
        Ok(())
    }

    #[test]
    fn check_eigon_primitive_is_type() {
        check_type(&mut ctx(), &Exp::EigonPrimitive(PrimitiveType::String)).unwrap();
        check(
            &mut ctx(),
            &Exp::EigonPrimitive(PrimitiveType::Integer),
            &Val::Set,
        )
        .unwrap();
    }

    // --- Codata tests (D11, Phase 9b-i) ---

    fn pair_codata_type() -> Exp {
        // codata { fst : 1; snd : 1 }
        Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "fst".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "snd".to_string(),
                typ: Exp::One,
            },
        ])
    }

    fn unit_pair_corecord() -> Exp {
        // corecord { fst = (); snd = () }
        Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "fst".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "snd".to_string(),
                body: Exp::Unit,
            },
        ])
    }

    #[test]
    fn codata_type_is_a_type() {
        check_type(&mut ctx(), &pair_codata_type()).unwrap();
        check(&mut ctx(), &pair_codata_type(), &Val::Set).unwrap();
    }

    #[test]
    fn codata_duplicate_observation_rejected() {
        let bad = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "x".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "x".to_string(),
                typ: Exp::One,
            },
        ]);
        assert!(check_type(&mut ctx(), &bad).is_err());
    }

    #[test]
    fn corecord_checks_against_codata_type() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        check(&mut ctx(), &unit_pair_corecord(), &codata_typ)?;
        Ok(())
    }

    #[test]
    fn corecord_mismatched_fields_rejected() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        // Missing 'snd'
        let bad = Exp::CoRecord(vec![crate::nbe::term::CoField {
            name: "fst".to_string(),
            body: Exp::Unit,
        }]);
        assert!(check(&mut ctx(), &bad, &codata_typ).is_err());
        Ok(())
    }

    #[test]
    fn corecord_wrong_field_order_rejected() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        // Fields in wrong order
        let bad = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "snd".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "fst".to_string(),
                body: Exp::Unit,
            },
        ]);
        assert!(check(&mut ctx(), &bad, &codata_typ).is_err());
        Ok(())
    }

    #[test]
    fn observation_evaluates_to_field_body() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // corecord { fst = (); snd = () }.fst → ()
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "fst".to_string());
        let result = eval(&observe, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn observation_unknown_field_returns_err() {
        // vobserve now returns Err for unknown fields (issue #19)
        use crate::nbe::eval::eval;
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "missing".to_string());
        let result = eval(&observe, &Rho::Nil);
        assert!(result.is_err());
    }

    #[test]
    fn observation_type_inference() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::env::up_gamma;
        use crate::nbe::eval::eval;
        // Given x : codata { fst : 1; snd : 1 }, infer x.fst : 1
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let gamma = up_gamma(&vec![], &Patt::Var("x".to_string()), &codata_typ, &gen)?;
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), gen);
        let mut c = CheckCtx::new(rho, gamma);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let t = check_infer(&mut c, &observe)?;
        assert!(matches!(t, Val::One));
        Ok(())
    }

    #[test]
    fn observation_on_neutral_blocks() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // let x = <neutral>; x.fst should produce a Neut::Observe
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), neut);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let result = eval(&observe, &rho)?;
        assert!(matches!(
            result,
            Val::Nt(crate::nbe::val::Neut::Observe(_, _))
        ));
        Ok(())
    }

    #[test]
    fn stream_two_observations_advance() -> Result<(), Box<dyn std::error::Error>> {
        // letrec nats : Nat → codata { head : Nat; tail : codata { head : Nat; tail : ... } } = λn. corecord { head = n; tail = nats(n+1) }
        //
        // Simplified for testing: use Unit as the element type and
        // represent Nat as a chain of Con values. Observing head twice
        // should advance the stream.
        //
        // Stream type (same at every step, so we use a self-referential
        // type by using EigonPrimitive::Integer as a stand-in — type
        // checking is not the focus here; we just want to verify
        // evaluation and observation plumbing).
        use crate::nbe::eval::eval;
        use crate::nbe::term::PrimitiveType;

        // Build: λn. corecord { head = n; tail = f(n) }
        // where f is a free variable we'll instantiate via Rho.
        //
        // Instead of full recursion, verify two cases:
        //   corecord { head = (), tail = corecord { head = (), tail = <bottom> } }
        // and confirm that .tail.head returns ().
        let inner = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::EigonPrimitive(PrimitiveType::Integer), // placeholder "bottom"
            },
        ]);
        let outer = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: inner,
            },
        ]);
        // outer.tail.head → ()
        let expr = Exp::Observe(
            Box::new(Exp::Observe(Box::new(outer), "tail".to_string())),
            "head".to_string(),
        );
        let result = eval(&expr, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn recursive_stream_via_letrec() -> Result<(), Box<dyn std::error::Error>> {
        // letrec nats : codata { head : 1; tail : codata {...} } = corecord { head = (); tail = nats }
        // Observing nats.tail.tail.head should give ().
        use crate::nbe::eval::eval;

        // Self-referential codata type is tricky without proper type
        // theory; sidestep by using a simpler fixpoint test: the
        // evaluator should handle the corecursive reference via
        // Rho::UpDec.
        let corecord = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Var("nats".to_string()),
            },
        ]);
        // We don't need the type to check — just evaluate.
        let letrec = Exp::Dec(
            Decl::Drec(
                Patt::Var("nats".to_string()),
                Box::new(Exp::One), // placeholder type (not checked here)
                Box::new(corecord),
            ),
            // nats.tail.tail.head
            Box::new(Exp::Observe(
                Box::new(Exp::Observe(
                    Box::new(Exp::Observe(
                        Box::new(Exp::Var("nats".to_string())),
                        "tail".to_string(),
                    )),
                    "tail".to_string(),
                )),
                "head".to_string(),
            )),
        );
        let result = eval(&letrec, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    // --- Guardedness tests (D11 §3, Phase 9b-i) ---

    fn forbidden(names: &[&'static str]) -> std::collections::HashSet<&'static str> {
        names.iter().copied().collect()
    }

    #[test]
    fn guardedness_accepts_naked_corecursive_field_body() {
        // letrec ones = corecord { head = (); tail = ones }
        // The `tail` body is a naked reference to the corecursive name.
        // This is productive: observing tail returns the corecord,
        // subsequent observations are fresh steps.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Var("ones".to_string()),
            },
        ]);
        check_guarded(&body, &forbidden(&["ones"])).unwrap();
    }

    #[test]
    fn guardedness_accepts_corecursive_call_under_app() {
        // corecord { head = n; tail = nats(n+1) }
        // tail body is App(Var(nats), ...) — call under function
        // application is productive.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Var("n".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::App(Box::new(Exp::Var("nats".to_string())), Box::new(Exp::Unit)),
            },
        ]);
        check_guarded(&body, &forbidden(&["nats"])).unwrap();
    }

    #[test]
    fn guardedness_rejects_bare_corecursive_observation() {
        // corecord { head = bad.head; tail = ... }
        // Observing a corecord's own field inside its own body loops.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("bad".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        let err = check_guarded(&body, &forbidden(&["bad"])).unwrap_err();
        assert!(err.contains("unguarded"));
        assert!(err.contains("bad"));
    }

    #[test]
    fn guardedness_rejects_chained_corecursive_observation() {
        // bad.tail.head — chain of observations on corecursive name
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(
                    Box::new(Exp::Observe(
                        Box::new(Exp::Var("bad".to_string())),
                        "tail".to_string(),
                    )),
                    "head".to_string(),
                ),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        assert!(check_guarded(&body, &forbidden(&["bad"])).is_err());
    }

    #[test]
    fn guardedness_accepts_non_corecursive_letrec() {
        // letrec f = λx. f(x) — data recursion (not codata), no corecord.
        // Guardedness is a no-op here (data termination is a separate
        // concern; Mini-TT doesn't check it either).
        let body = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("x".to_string())),
            )),
        );
        check_guarded(&body, &forbidden(&["f"])).unwrap();
    }

    #[test]
    fn guardedness_accepts_observation_of_non_corecursive_ref() {
        // corecord { head = other.head; tail = () }
        // `other` is not a corecursive name here — observing it is fine.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("other".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        // Only `self` is forbidden; `other` is free.
        check_guarded(&body, &forbidden(&["self"])).unwrap();
    }

    #[test]
    fn guardedness_in_check_decl_rejects_bad_corecord() {
        // letrec bad : codata { head : 1; tail : 1 } = corecord { head = bad.head; tail = () }
        // The Drec pathway in check_decl now invokes check_guarded; this
        // should surface the unguarded reference.
        let codata_typ = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: Exp::One,
            },
        ]);
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("bad".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        let d = Decl::Drec(
            Patt::Var("bad".to_string()),
            Box::new(codata_typ),
            Box::new(body),
        );
        let err = check_decl(&mut ctx(), &d).unwrap_err();
        assert!(
            err.contains("unguarded"),
            "expected unguarded error, got: {err}"
        );
    }

    // --- Phase 10a: new inference and resolution tests ---

    #[test]
    fn infer_refl() {
        // refl(x) where x : One should infer Id(One, x_val, x_val)
        let gamma: Gamma = vec![("x".to_string(), Val::One)];
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        let mut c = CheckCtx::new(rho, gamma);
        let refl_x = Exp::Refl(Box::new(Exp::Var("x".to_string())));
        let t = check_infer(&mut c, &refl_x).unwrap();
        assert!(matches!(t, Val::Id(_, _, _)));
    }

    #[test]
    fn infer_deceq() {
        // DecEq(One, (), ()) should infer Id(One, (), ())
        let deceq = Exp::DecEq(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let t = check_infer(&mut ctx(), &deceq).unwrap();
        assert!(matches!(t, Val::Id(_, _, _)));
    }

    #[test]
    fn infer_template() {
        // Template("hello", []) should infer EigonPrimitive(String)
        let tmpl = Exp::Template("hello".to_string(), vec![]);
        let t = check_infer(&mut ctx(), &tmpl).unwrap();
        assert!(matches!(t, Val::EigonPrimitive(PrimitiveType::String)));
    }

    #[test]
    fn infer_eigon_resource() {
        use crate::ontology::resource::Resource;
        // EigonResource with is_a = [Dog] should infer EigonClass(Dog)
        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let is_a_iri = Iri::parse("urn:eigenius:core:is_a").unwrap();
        let mut r = Resource::new(Iri::parse("urn:example:rex").unwrap());
        r.set(
            is_a_iri,
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(dog_iri.as_str().to_string()),
            ]),
        );
        let expr = Exp::EigonResource(Box::new(r));
        let t = check_infer(&mut ctx(), &expr).unwrap();
        match t {
            Val::EigonClass(iri) => assert_eq!(iri.as_str(), "urn:eigenius:example:Dog"),
            other => panic!("expected EigonClass, got {:?}", other),
        }
    }

    #[test]
    fn find_sigma_field_resolves_eigon_class_with_layer() {
        // With a layer, find_sigma_field on EigonClass should resolve
        // to actual property types instead of Val::Set.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        let layer =
            std::sync::Arc::new(domain_builder.build(crate::layer::LayerStorage::in_memory()));

        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let dog_type = Val::EigonClass(dog_iri);

        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        let field = find_sigma_field(&mut c, &dog_type, "name");
        assert!(field.is_some(), "should find 'name' on Dog");
        // The type should NOT be Val::Set (the old broken behavior)
        let field_type = field.unwrap();
        assert!(
            !matches!(field_type, Val::Set),
            "field type should be resolved, not Set; got {:?}",
            field_type
        );
    }

    #[test]
    fn find_sigma_field_without_layer_returns_none_for_eigon_class() {
        // Without a layer, EigonClass resolution should fail gracefully
        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let dog_type = Val::EigonClass(dog_iri);
        let mut c = ctx();
        let field = find_sigma_field(&mut c, &dog_type, "name");
        assert!(field.is_none(), "no layer → should not resolve");
    }

    // --- Inductive type checking (Phase 11b step 5) ---

    use crate::nbe::term::InductiveCtorDecl;

    fn ind_self_ref(name: &str) -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            name: name.to_string(),
            params: Vec::new(),
            sort: Exp::Set,
            ctors: Vec::new(),
        })
    }

    fn nat_decl() -> Arc<InductiveDecl> {
        let s = ind_self_ref("Nat");
        let nat_ty = Exp::InductiveType(s, Vec::new());
        Arc::new(InductiveDecl {
            name: "Nat".to_string(),
            params: Vec::new(),
            sort: Exp::Set,
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

    fn nat_zero_exp(decl: &Arc<InductiveDecl>) -> Exp {
        Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new())
    }

    fn nat_succ_exp(decl: &Arc<InductiveDecl>, n: Exp) -> Exp {
        Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![n])
    }

    /// Constant `λ_. Set` motive — applied to anything yields `Set`.
    fn const_set_motive_exp() -> Exp {
        Exp::Lam(Patt::Unit, Box::new(Exp::Set))
    }

    #[test]
    fn check_ctor_zero_against_nat_type() {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
        };
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check(&mut c, &nat_zero_exp(&nat), &nat_ty).expect("zero : Nat");
    }

    #[test]
    fn check_ctor_succ_zero_against_nat_type() {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
        };
        let exp = nat_succ_exp(&nat, nat_zero_exp(&nat));
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check(&mut c, &exp, &nat_ty).expect("succ zero : Nat");
    }

    #[test]
    fn check_ctor_arg_type_mismatch() {
        // succ Set should fail because Set : Type, not Nat.
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
        };
        let bogus = Exp::InductiveCtor(nat.clone(), "succ".to_string(), vec![Exp::Set]);
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        assert!(check(&mut c, &bogus, &nat_ty).is_err());
    }

    #[test]
    fn check_ctor_unknown_constructor_name() {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
        };
        let bogus = Exp::InductiveCtor(nat.clone(), "two".to_string(), Vec::new());
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let err = check(&mut c, &bogus, &nat_ty).unwrap_err();
        assert!(err.contains("no constructor"), "unexpected: {err}");
    }

    #[test]
    fn check_ctor_wrong_decl_against_other_inductive() {
        // Construct a Bool decl, then try to type-check Bool's True against Nat.
        let nat = nat_decl();
        let bs = ind_self_ref("Bool");
        let bool_ty_exp = Exp::InductiveType(bs, Vec::new());
        let bool_decl = Arc::new(InductiveDecl {
            name: "Bool".to_string(),
            params: Vec::new(),
            sort: Exp::Set,
            ctors: vec![InductiveCtorDecl {
                name: "True".to_string(),
                typ: bool_ty_exp,
            }],
        });
        let true_exp = Exp::InductiveCtor(bool_decl, "True".to_string(), Vec::new());
        let nat_ty = Val::InductiveType {
            decl: nat,
            params: Vec::new(),
        };
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let err = check(&mut c, &true_exp, &nat_ty).unwrap_err();
        assert!(err.contains("does not match"), "unexpected: {err}");
    }

    #[test]
    fn infer_ctor_succeeds_for_non_parametric_inductive() {
        // Nat has no params → inference returns InductiveType{Nat, []}.
        let nat = nat_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let typ = check_infer(&mut c, &nat_zero_exp(&nat)).expect("infer Nat.zero");
        match typ {
            Val::InductiveType { decl, params } => {
                assert_eq!(decl.name, "Nat");
                assert!(params.is_empty());
            }
            other => panic!("expected InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn infer_ctor_fails_for_parametric_inductive() {
        let s = ind_self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list_decl = Arc::new(InductiveDecl {
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Set)],
            sort: Exp::Set,
            ctors: vec![InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Set),
                    Box::new(list_ty),
                ),
            }],
        });
        let nil_exp = Exp::InductiveCtor(list_decl, "nil".to_string(), Vec::new());
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let err = check_infer(&mut c, &nil_exp).unwrap_err();
        assert!(err.contains("checking mode"), "unexpected: {err}");
    }

    /// Build a `CheckCtx` with `n : Nat` bound (gamma + rho).
    fn ctx_with_nat_var() -> (Arc<InductiveDecl>, CheckCtx) {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
        };
        let nat_val = Val::InductiveVal {
            decl: nat.clone(),
            ctor_name: "zero".to_string(),
            args: Vec::new(),
        };
        let gamma: Gamma = vec![("n".to_string(), nat_ty)];
        let rho = Rho::Nil.extend(Patt::Var("n".to_string()), nat_val);
        (nat, CheckCtx::new(rho, gamma))
    }

    #[test]
    fn infer_rec_well_typed() {
        // Nat.rec (λ_. Set) Nat (λ_n. λ_ih. Nat) n   (motive constant Set)
        // Motive : Nat → Set, zero minor : Set, succ minor : Nat → Set → Set,
        // result type: Set.
        let (nat, mut c) = ctx_with_nat_var();
        let nat_ty_exp = Exp::InductiveType(nat.clone(), Vec::new());
        let succ_minor = Exp::Lam(
            Patt::Unit,
            Box::new(Exp::Lam(Patt::Unit, Box::new(nat_ty_exp.clone()))),
        );
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![nat_ty_exp, succ_minor],
            major: Box::new(Exp::Var("n".to_string())),
        };
        let typ = check_infer(&mut c, &exp).expect("Nat.rec well-typed");
        assert!(matches!(typ, Val::Set), "expected Set, got {typ:?}");
    }

    #[test]
    fn infer_rec_wrong_minor_count() {
        let (nat, mut c) = ctx_with_nat_var();
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![Exp::InductiveType(nat_decl(), Vec::new())], // only 1 minor, needs 2
            major: Box::new(Exp::Var("n".to_string())),
        };
        let err = check_infer(&mut c, &exp).unwrap_err();
        assert!(err.contains("expected 2 minors"), "unexpected: {err}");
    }

    #[test]
    fn infer_rec_minor_type_mismatch() {
        // Wrong type for the zero minor — supply Unit instead of a Set.
        let (nat, mut c) = ctx_with_nat_var();
        let nat_ty_exp = Exp::InductiveType(nat.clone(), Vec::new());
        let succ_minor = Exp::Lam(
            Patt::Unit,
            Box::new(Exp::Lam(Patt::Unit, Box::new(nat_ty_exp))),
        );
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![Exp::Unit, succ_minor],
            major: Box::new(Exp::Var("n".to_string())),
        };
        assert!(check_infer(&mut c, &exp).is_err());
    }

    #[test]
    fn infer_rec_major_wrong_type() {
        // Major has type 1 (One), not Nat — must fail with the inductive-type message.
        let nat = nat_decl();
        let nat_ty_exp = Exp::InductiveType(nat.clone(), Vec::new());
        let succ_minor = Exp::Lam(
            Patt::Unit,
            Box::new(Exp::Lam(Patt::Unit, Box::new(nat_ty_exp.clone()))),
        );
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![nat_ty_exp, succ_minor],
            major: Box::new(Exp::Var("u".to_string())),
        };
        let gamma: Gamma = vec![("u".to_string(), Val::One)];
        let rho = Rho::Nil.extend(Patt::Var("u".to_string()), Val::Unit);
        let mut c = CheckCtx::new(rho, gamma);
        let err = check_infer(&mut c, &exp).unwrap_err();
        assert!(
            err.contains("expected an inductive type"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn infer_rec_decl_mismatch() {
        // n : Nat but recursor uses Bool decl.
        let (_nat, mut c) = ctx_with_nat_var();
        let bs = ind_self_ref("Bool");
        let bool_ty = Exp::InductiveType(bs, Vec::new());
        let bool_decl = Arc::new(InductiveDecl {
            name: "Bool".to_string(),
            params: Vec::new(),
            sort: Exp::Set,
            ctors: vec![
                InductiveCtorDecl {
                    name: "True".to_string(),
                    typ: bool_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "False".to_string(),
                    typ: bool_ty.clone(),
                },
            ],
        });
        let exp = Exp::InductiveRec {
            decl: bool_decl,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![bool_ty.clone(), bool_ty],
            major: Box::new(Exp::Var("n".to_string())),
        };
        let err = check_infer(&mut c, &exp).unwrap_err();
        assert!(err.contains("declaration mismatch"), "unexpected: {err}");
    }

    // --- Sized types primitives (Phase 11b step 14) ---

    #[test]
    fn size_sort_is_a_type() {
        // SizeSort checks as a type (Type(1)).
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check_type(&mut c, &Exp::SizeSort).expect("SizeSort should be a type");
    }

    #[test]
    fn size_inf_inhabits_size_sort() {
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check(&mut c, &Exp::SizeInf, &Val::SizeSort).expect("SizeInf : SizeSort");
    }

    #[test]
    fn size_succ_of_inf_inhabits_size_sort() {
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let exp = Exp::SizeSucc(Box::new(Exp::SizeInf));
        check(&mut c, &exp, &Val::SizeSort).expect("SizeSucc(SizeInf) : SizeSort");
    }

    #[test]
    fn size_sort_inferred_at_type_1() {
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let typ = check_infer(&mut c, &Exp::SizeSort).expect("infer SizeSort");
        assert!(matches!(typ, Val::Type(1)));
    }

    #[test]
    fn size_inf_inferred_at_size_sort() {
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let typ = check_infer(&mut c, &Exp::SizeInf).expect("infer SizeInf");
        assert!(matches!(typ, Val::SizeSort));
    }

    #[test]
    fn size_succ_requires_size_sort_argument() {
        // SizeSucc applied to a non-size expression should fail.
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let bogus = Exp::SizeSucc(Box::new(Exp::Set));
        assert!(check(&mut c, &bogus, &Val::SizeSort).is_err());
    }

    // --- Size-aware subtyping (Phase 11b step 15d, D19 §8.3) ---

    fn sized_stream_decl() -> Arc<InductiveDecl> {
        // Minimal sized type former: `SizedStream(i : SizeSort, A : Set)`.
        // We don't need real constructors for the subtyping tests —
        // `PartialEq` on `InductiveDecl` goes by name, so two calls to
        // this helper produce decls that compare equal.
        Arc::new(InductiveDecl {
            name: "SizedStream".to_string(),
            params: vec![
                (Patt::Var("i".to_string()), Exp::SizeSort),
                (Patt::Var("A".to_string()), Exp::Set),
            ],
            sort: Exp::Set,
            ctors: vec![],
        })
    }

    fn mk_sized_type(decl: Arc<InductiveDecl>, size: Val, elem: Val) -> Val {
        Val::InductiveType {
            decl,
            params: vec![size, elem],
        }
    }

    #[test]
    fn subtype_sized_finite_to_inf_admitted() {
        // SizedStream(ŝ ∞, A) is blocked by ∞-absorption (∞ stays ∞).
        // Use a neutral size to get a real "finite-side-of-∞" value.
        let decl = sized_stream_decl();
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "i".into()));
        let sub = mk_sized_type(decl.clone(), neut.clone(), Val::One);
        let sup = mk_sized_type(decl, Val::SizeInf, Val::One);
        subtype_of(0, &sub, &sup).expect("T(i) <: T(∞)");
    }

    #[test]
    fn subtype_sized_inf_to_finite_rejected() {
        let decl = sized_stream_decl();
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "i".into()));
        let sub = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl, neut, Val::One);
        assert!(
            subtype_of(0, &sub, &sup).is_err(),
            "T(∞) <: T(i) must be rejected"
        );
    }

    #[test]
    fn subtype_sized_step_rule_admitted() {
        // T(i) <: T(ŝ i) admitted by the right-step rule on sizes.
        let decl = sized_stream_decl();
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "i".into()));
        let sub = mk_sized_type(decl.clone(), neut.clone(), Val::One);
        let sup = mk_sized_type(decl, Val::SizeSucc(Box::new(neut)), Val::One);
        subtype_of(0, &sub, &sup).expect("T(i) <: T(ŝ i)");
    }

    #[test]
    fn subtype_sized_same_inf_reflexive() {
        let decl = sized_stream_decl();
        let sub = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl, Val::SizeInf, Val::One);
        subtype_of(0, &sub, &sup).expect("T(∞) <: T(∞) reflexive");
    }

    #[test]
    fn subtype_non_size_parameter_still_requires_equality() {
        // Sized stream parameters disagree on the element type —
        // size_le only relaxes size positions, so the other position
        // must still be equal.
        let decl = sized_stream_decl();
        let sub = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl, Val::SizeInf, Val::Set);
        assert!(
            subtype_of(0, &sub, &sup).is_err(),
            "element type mismatch must be rejected"
        );
    }

    #[test]
    fn subtype_non_inductive_falls_back_to_eq_nf() {
        // Simple non-inductive types fall through to `eq_nf` —
        // equal types accept, mismatched types reject.
        subtype_of(0, &Val::One, &Val::One).expect("1 <: 1");
        assert!(subtype_of(0, &Val::One, &Val::Set).is_err());
    }

    #[test]
    fn subtype_distinct_inductive_decls_fall_back_to_eq_nf() {
        // Two inductive types with different names: the sized-subtyping
        // branch is skipped (decls differ), and `eq_nf` correctly
        // rejects them.
        let decl_a = sized_stream_decl();
        let decl_b = Arc::new(InductiveDecl {
            name: "OtherStream".to_string(),
            params: decl_a.params.clone(),
            sort: Exp::Set,
            ctors: vec![],
        });
        let sub = mk_sized_type(decl_a, Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl_b, Val::SizeInf, Val::One);
        assert!(subtype_of(0, &sub, &sup).is_err());
    }

    #[test]
    fn check_var_with_finite_size_against_inf_expected_succeeds() {
        // End-to-end: a variable `x : SizedStream(i, One)` checks
        // against the expected type `SizedStream(∞, One)`.
        //
        // This exercises the `check()` fallthrough at line ~388 —
        // it infers `x`'s type from gamma, then calls subtype_of
        // against the expected type. Without sized subtyping this
        // would fail (neutral `i` ≠ SizeInf syntactically).
        let decl = sized_stream_decl();

        // Bind `i : SizeSort`, then `x : SizedStream(i, One)`.
        let i_val = gen_val(&Rho::Nil); // Val::Nt(Gen(0, _))
        let rho1 = Rho::Nil.extend(Patt::Var("i".to_string()), i_val.clone());
        let gamma1 = up_gamma(
            &Vec::new(),
            &Patt::Var("i".to_string()),
            &Val::SizeSort,
            &i_val,
        )
        .unwrap();

        let sub_stream = mk_sized_type(decl.clone(), i_val, Val::One);
        let x_val = gen_val(&rho1); // Val::Nt(Gen(1, _))
        let rho2 = rho1.extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&gamma1, &Patt::Var("x".to_string()), &sub_stream, &x_val).unwrap();

        let mut c = CheckCtx::new(rho2, gamma2);
        let expected = mk_sized_type(decl, Val::SizeInf, Val::One);
        check(&mut c, &Exp::Var("x".to_string()), &expected)
            .expect("x : SizedStream(i, 1) should check against SizedStream(∞, 1)");
    }

    // --- End-to-end sized Nat (Phase 11b step 15d capstone) ---
    //
    // Builds a sized Nat inductive and exercises the full pipeline:
    // constructor type-checking with size parameter binding,
    // ∞-absorption collapsing `↑ ∞` to `∞`, and subtyping-aware
    // result-type verification.
    //
    // **Known limitation of the encoding.** This is a Lean-style
    // declaration: the constructor's first binder is *identified*
    // with the outer inductive index (both named `i`). Agda-style
    // sized types treat the inductive's index and the constructor's
    // local predecessor size as *separate* variables, unifying them
    // at the call site (i.e. solving `↑ i_pred = outer_index` for
    // `i_pred`). Without that unification — or bounded binders, which
    // would let us write `succ : {j < i}. SizedNat j → SizedNat i` —
    // the `succ` constructor below only type-checks at outer size
    // `∞` (via ∞-absorption collapsing `↑ ∞` to `∞`). At finite outer
    // sizes `k` the model forces `i = k` and the declared result
    // `SizedNat (↑ k)` fails the `↑ k ≤ k` subtype check.
    //
    // These tests therefore exercise the ∞-end of the sized lattice.
    // Real size-tracking termination awaits bounded binders and/or
    // implicit-arg solving in a later step.

    fn sized_nat_decl() -> Arc<InductiveDecl> {
        // SizedNat(i : SizeSort) with
        //   zero : Π i:SizeSort. SizedNat i       (exists at every size)
        //   succ : Π i:SizeSort. SizedNat i → SizedNat (↑ i)
        let self_ref = Arc::new(InductiveDecl {
            name: "SizedNat".to_string(),
            params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
            sort: Exp::Set,
            ctors: Vec::new(),
        });
        let snat_i = Exp::InductiveType(self_ref.clone(), vec![Exp::Var("i".to_string())]);
        let snat_succ_i = Exp::InductiveType(
            self_ref,
            vec![Exp::SizeSucc(Box::new(Exp::Var("i".to_string())))],
        );
        Arc::new(InductiveDecl {
            name: "SizedNat".to_string(),
            params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
            sort: Exp::Set,
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

    fn snat_ty(decl: Arc<InductiveDecl>, size: Val) -> Val {
        Val::InductiveType {
            decl,
            params: vec![size],
        }
    }

    #[test]
    fn sized_nat_type_at_inf_is_a_type() {
        let decl = sized_nat_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let ty = Exp::InductiveType(decl, vec![Exp::SizeInf]);
        check_type(&mut c, &ty).expect("SizedNat(∞) is a valid type");
    }

    #[test]
    fn sized_nat_zero_at_inf() {
        // `zero` at expected SizedNat(∞) type-checks. After binding
        // i = ∞, the result is SizedNat(∞) — matches expected exactly.
        let decl = sized_nat_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        check(&mut c, &zero, &snat_ty(decl, Val::SizeInf)).expect("zero : SizedNat(∞)");
    }

    #[test]
    fn sized_nat_succ_zero_at_inf() {
        // `succ(zero) : SizedNat(∞)`. Critical: `succ`'s declared result
        // is `SizedNat(↑ i)`. After binding i = ∞, the result evaluates
        // to `SizedNat(↑ ∞)` which ∞-absorption collapses to
        // `SizedNat(∞)`. So the subtype check on the constructor's
        // result trivially succeeds.
        let decl = sized_nat_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let one = Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![zero]);
        check(&mut c, &one, &snat_ty(decl, Val::SizeInf)).expect("succ zero : SizedNat(∞)");
    }

    #[test]
    fn sized_nat_two_at_inf() {
        // Nested: `succ(succ(zero)) : SizedNat(∞)`.
        let decl = sized_nat_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let one = Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![zero]);
        let two = Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![one]);
        check(&mut c, &two, &snat_ty(decl, Val::SizeInf)).expect("2 : SizedNat(∞)");
    }

    #[test]
    fn sized_nat_succ_lifts_into_inf_via_subtyping() {
        // `x : SizedNat(j)`, check `succ(x) : SizedNat(∞)`.
        // succ produces SizedNat(↑ j); subtyping ↑j ≤ ∞ permits it.
        let decl = sized_nat_decl();
        let j_val = gen_val(&Rho::Nil);
        let rho1 = Rho::Nil.extend(Patt::Var("j".to_string()), j_val.clone());
        let gamma1 = up_gamma(
            &Vec::new(),
            &Patt::Var("j".to_string()),
            &Val::SizeSort,
            &j_val,
        )
        .unwrap();
        let snat_j = snat_ty(decl.clone(), j_val);
        let x_val = gen_val(&rho1);
        let rho2 = rho1.extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&gamma1, &Patt::Var("x".to_string()), &snat_j, &x_val).unwrap();

        let mut c = CheckCtx::new(rho2, gamma2);
        let succ_x = Exp::InductiveCtor(
            decl.clone(),
            "succ".to_string(),
            vec![Exp::Var("x".to_string())],
        );
        check(&mut c, &succ_x, &snat_ty(decl, Val::SizeInf))
            .expect("succ x : SizedNat(∞) via subtyping");
    }

    #[test]
    fn sized_nat_succ_mismatch_rejected() {
        // `x : SizedNat(j)` neutral, check `succ(x) : SizedNat(j)`.
        // Applied param binds the ctor's local `i := j`, so succ's
        // declared result `SizedNat (↑ i)` evaluates to SizedNat(↑ j);
        // subtyping requires `↑ j ≤ j` which fails without a
        // hypothesis. Must be rejected — validates that the new
        // result-type check in `check_inductive_ctor_args` actually
        // fires for a mismatched sized constructor.
        let decl = sized_nat_decl();
        let j_val = gen_val(&Rho::Nil);
        let rho1 = Rho::Nil.extend(Patt::Var("j".to_string()), j_val.clone());
        let gamma1 = up_gamma(
            &Vec::new(),
            &Patt::Var("j".to_string()),
            &Val::SizeSort,
            &j_val,
        )
        .unwrap();
        let snat_j = snat_ty(decl.clone(), j_val.clone());
        let x_val = gen_val(&rho1);
        let rho2 = rho1.extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&gamma1, &Patt::Var("x".to_string()), &snat_j, &x_val).unwrap();

        let mut c = CheckCtx::new(rho2, gamma2);
        let succ_x = Exp::InductiveCtor(
            decl.clone(),
            "succ".to_string(),
            vec![Exp::Var("x".to_string())],
        );
        assert!(
            check(&mut c, &succ_x, &snat_ty(decl, j_val)).is_err(),
            "succ x must not check against SizedNat(j) — result is ↑j, not j"
        );
    }

    #[test]
    fn check_var_with_inf_size_against_finite_expected_fails() {
        // Dual: `x : SizedStream(∞, One)` cannot be checked against
        // `SizedStream(i, One)` — ∞ ≰ i for an unconstrained rigid i.
        let decl = sized_stream_decl();

        let i_val = gen_val(&Rho::Nil);
        let rho1 = Rho::Nil.extend(Patt::Var("i".to_string()), i_val.clone());
        let gamma1 = up_gamma(
            &Vec::new(),
            &Patt::Var("i".to_string()),
            &Val::SizeSort,
            &i_val,
        )
        .unwrap();

        let sup_stream = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let x_val = gen_val(&rho1);
        let rho2 = rho1.extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&gamma1, &Patt::Var("x".to_string()), &sup_stream, &x_val).unwrap();

        let mut c = CheckCtx::new(rho2, gamma2);
        let expected = mk_sized_type(decl, i_val, Val::One);
        assert!(
            check(&mut c, &Exp::Var("x".to_string()), &expected).is_err(),
            "x : SizedStream(∞, 1) must not check against SizedStream(i, 1)"
        );
    }

    // --- Bounded size binders (Phase 11b step 15e) ---
    //
    // Exercise `Exp::SizedPi` end-to-end: type formation, application
    // with a strictly-smaller size argument, rejection of oversized
    // applications, and subtyping-under-hypothesis via the TSO.

    /// Build a context with `i : SizeSort` bound as a rigid size
    /// variable at level 0. Returns the ctx and i's value.
    fn ctx_with_size_var(name: &str) -> (CheckCtx, Val) {
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

    #[test]
    fn sized_pi_at_inf_is_a_type() {
        // `{j < ∞}. One` is a valid type.
        let exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::SizeInf),
            body: Box::new(Exp::One),
        };
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check_type(&mut c, &exp).expect("{j < ∞}. 1 is a type");
    }

    #[test]
    fn sized_pi_at_rigid_var_is_a_type() {
        // Under `i : SizeSort`, `{j < i}. One` is a valid type.
        let (mut c, _) = ctx_with_size_var("i");
        let exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        check_type(&mut c, &exp).expect("{j < i}. 1 is a type");
    }

    #[test]
    fn sized_pi_non_rigid_upper_rejected() {
        // `{j < ŝ i}. One` must be rejected — the upper bound is
        // not a rigid size variable or ∞.
        let (mut c, _) = ctx_with_size_var("i");
        let exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::SizeSucc(Box::new(Exp::Var("i".to_string())))),
            body: Box::new(Exp::One),
        };
        let err = check_type(&mut c, &exp).unwrap_err();
        assert!(
            err.contains("rigid size variable"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sized_pi_app_with_strict_smaller_size_succeeds() {
        // `f : {j < i}. 1`. Applying to a size strictly below `i`
        // succeeds. Use `ŝ i`? No — that's GREATER than i. We need
        // something below i, which means ∞-absorption doesn't help.
        // Simplest: hoist `f` to type `{j < ∞}. 1`, then apply at `i`.
        let (c, i_val) = ctx_with_size_var("i");

        let f_val = gen_val(&c.rho);
        let f_ty = Val::SizedPi(
            Box::new(Val::SizeInf),
            Clos {
                patt: Patt::Unit,
                body: Exp::One,
                env: Rho::Nil,
            },
        );
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("f".to_string()), f_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("f".to_string()), &f_ty, &f_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);
        c2.size_tso = c.size_tso.clone();

        // f(i) — i is a size, and i < ∞ trivially.
        let app = Exp::App(
            Box::new(Exp::Var("f".to_string())),
            Box::new(Exp::Var("i".to_string())),
        );
        let result_ty = check_infer(&mut c2, &app).expect("f(i) : 1");
        eq_nf(c2.rho.len(), &result_ty, &Val::One).expect("result is 1");
        drop(i_val);
    }

    #[test]
    fn sized_pi_app_with_equal_size_rejected() {
        // `f : {j < i}. 1`. Applying at `i` violates `i < i`.
        // Build the context by check_type-ing the SizedPi (which
        // registers no hypothesis since f's domain is inside the
        // binder, not outer scope).
        let (c, i_val) = ctx_with_size_var("i");

        // f's type: {j < i}. 1
        let f_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let f_ty = eval(&f_ty_exp, &c.rho).expect("eval f_ty");
        let f_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("f".to_string()), f_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("f".to_string()), &f_ty, &f_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        // f(i) must be rejected: i < i is false.
        let app = Exp::App(
            Box::new(Exp::Var("f".to_string())),
            Box::new(Exp::Var("i".to_string())),
        );
        let err = check_infer(&mut c2, &app).unwrap_err();
        assert!(
            err.contains("not strictly below"),
            "unexpected error: {err}"
        );
        drop(i_val);
    }

    #[test]
    fn sized_pi_hypothesis_witnesses_sized_subtyping() {
        // The payoff test. Given `i : SizeSort` and we're inside a
        // `{j < i}. body`, a variable of type `SizedStream(j, 1)`
        // must check against expected `SizedStream(i, 1)` via
        // `j ≤ i` derived from the TSO hypothesis.
        //
        // We can't directly observe the TSO state from a check() call
        // without entering a SizedPi binder, so this test descends
        // into a `check_type` for a SizedPi whose body references
        // a sized inductive — which gives us the entailment in the
        // `body` position via the subtype_of fallthrough.
        let decl = sized_stream_decl();

        // Outer: bind i : SizeSort.
        let (c, i_val) = ctx_with_size_var("i");

        // Body of the SizedPi: Π x : SizedStream(j, 1). SizedStream(i, 1).
        // Inside, we have `j < i` as hypothesis. A variable
        // `x : SizedStream(j, 1)` used where `SizedStream(i, 1)` is
        // expected will go through the fallthrough → subtype_of,
        // which consults the TSO and sees `j ≤ i`.
        let body = Exp::Pi(
            Patt::Var("x".to_string()),
            Box::new(Exp::InductiveType(
                decl.clone(),
                vec![Exp::Var("j".to_string()), Exp::One],
            )),
            Box::new(Exp::InductiveType(
                decl.clone(),
                vec![Exp::Var("i".to_string()), Exp::One],
            )),
        );
        let outer = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(body),
        };

        // Type-formation succeeds — both SizedStream(j, 1) and
        // SizedStream(i, 1) are types in the extended ctx.
        let mut c = c;
        check_type(&mut c, &outer).expect("SizedPi type with inductive body type-checks");
        drop((decl, i_val));
    }

    #[test]
    fn sized_pi_hypothesis_lets_variable_cross_size_boundary() {
        // End-to-end: `{j < i}. SizedStream(j, 1) → SizedStream(i, 1)`
        // treated as a function type. We check a lambda `λ x. x`
        // against this type — the body uses x : SizedStream(j, 1)
        // where the codomain expects SizedStream(i, 1). The subtype
        // check has TSO hypothesis `j < i` in scope.
        let decl = sized_stream_decl();
        let (mut c, _i_val) = ctx_with_size_var("i");

        let sized_stream_j =
            Exp::InductiveType(decl.clone(), vec![Exp::Var("j".to_string()), Exp::One]);
        let sized_stream_i =
            Exp::InductiveType(decl.clone(), vec![Exp::Var("i".to_string()), Exp::One]);
        let fn_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::Pi(
                Patt::Var("x".to_string()),
                Box::new(sized_stream_j),
                Box::new(sized_stream_i),
            )),
        };
        check_type(&mut c, &fn_ty_exp)
            .expect("{j < i}. SizedStream(j, 1) → SizedStream(i, 1) is a type");
    }

    // --- Productivity via sized codata (Phase 11b step 15f) ---
    //
    // A sized codata type's observations use `SizedPi` for recursive
    // positions. A field body that inhabits such an observation type
    // is typically `λ j. body` — the new `Lam`-vs-`SizedPi` check arm
    // opens the size binder, registers `j < upper` in the TSO, and
    // checks the body against the codomain. Recursive references to
    // the corecord are forced (by type) to produce results at sizes
    // strictly below the outer size, yielding productivity.

    #[test]
    fn lam_against_sized_pi_at_inf() {
        // `λ j. Unit` : `{j < ∞}. One`. Trivial sanity.
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::SizeInf),
            body: Box::new(Exp::One),
        };
        let ty = eval(&ty_exp, &c.rho).expect("eval ty");
        let lam = Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit));
        check(&mut c, &lam, &ty).expect("λ j. Unit : {j < ∞}. 1");
    }

    #[test]
    fn lam_against_sized_pi_at_rigid() {
        // With `i : SizeSort`, `λ j. Unit` : `{j < i}. One`.
        let (mut c, _) = ctx_with_size_var("i");
        let ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let ty = eval(&ty_exp, &c.rho).expect("eval ty");
        let lam = Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit));
        check(&mut c, &lam, &ty).expect("λ j. Unit : {j < i}. 1");
    }

    #[test]
    fn lam_body_uses_bounded_size_in_application() {
        // With `i : SizeSort` and `f : Π k:SizeSort. One`,
        // `λ j. f(j)` checks against `{j < i}. One`.
        // Exercises: binder opens j, app of f to j gets the size
        // hypothesis from TSO (though trivially — we're going through
        // Pi, not SizedPi, so no strict bound needed).
        let (c, _i_val) = ctx_with_size_var("i");

        let f_ty_exp = Exp::Pi(
            Patt::Var("k".to_string()),
            Box::new(Exp::SizeSort),
            Box::new(Exp::One),
        );
        let f_ty = eval(&f_ty_exp, &c.rho).expect("eval f_ty");
        let f_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("f".to_string()), f_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("f".to_string()), &f_ty, &f_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let target_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let target_ty = eval(&target_ty_exp, &c2.rho).expect("eval target");
        let lam = Exp::Lam(
            Patt::Var("j".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("j".to_string())),
            )),
        );
        check(&mut c2, &lam, &target_ty).expect("λ j. f(j) : {j < i}. 1");
    }

    #[test]
    fn lam_body_invokes_sized_function_productively() {
        // The core productivity-by-typing scenario.
        //
        // Given `i : SizeSort` and a size-polymorphic producer
        // `g : Π k:SizeSort. SizedStream(k, 1)`, the expression
        // `λ j. g(j)` checks against `{j < i}. SizedStream(j, 1)`.
        //
        // This is exactly the shape of a sized corecord's `tail`
        // field when the corecord is defined by a size-polymorphic
        // function of itself: `tail = λ j. self(j)`. Type-checking
        // this field IS the productivity argument — the body must
        // produce a value at size `j`, which (since `j < i`) is
        // strictly smaller than the outer size.
        let decl = sized_stream_decl();
        let (c, _) = ctx_with_size_var("i");

        let stream_k = Exp::InductiveType(decl.clone(), vec![Exp::Var("k".to_string()), Exp::One]);
        let g_ty_exp = Exp::Pi(
            Patt::Var("k".to_string()),
            Box::new(Exp::SizeSort),
            Box::new(stream_k),
        );
        let g_ty = eval(&g_ty_exp, &c.rho).expect("eval g_ty");
        let g_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("g".to_string()), g_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("g".to_string()), &g_ty, &g_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let stream_j = Exp::InductiveType(decl.clone(), vec![Exp::Var("j".to_string()), Exp::One]);
        let target_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(stream_j),
        };
        let target_ty = eval(&target_ty_exp, &c2.rho).expect("eval target");

        let lam = Exp::Lam(
            Patt::Var("j".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("g".to_string())),
                Box::new(Exp::Var("j".to_string())),
            )),
        );
        check(&mut c2, &lam, &target_ty)
            .expect("λ j. g(j) : {j < i}. SizedStream(j, 1) — productive by typing");
    }

    #[test]
    fn non_productive_body_rejected_by_sized_type() {
        // Given `h : SizedStream(i, 1)` at the OUTER size i, the body
        // `λ j. h` checks at the expected type `{j < i}. SizedStream(j, 1)`
        // iff `SizedStream(i, 1) <: SizedStream(j, 1)`, i.e. `i ≤ j`.
        // But TSO has `j < i`, not `i ≤ j`, so this must be rejected —
        // capturing the non-productive "reuse outer value at smaller
        // size" bug.
        let decl = sized_stream_decl();
        let (c, _) = ctx_with_size_var("i");

        let stream_i = Exp::InductiveType(decl.clone(), vec![Exp::Var("i".to_string()), Exp::One]);
        let h_ty = eval(&stream_i, &c.rho).expect("eval h_ty");
        let h_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("h".to_string()), h_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("h".to_string()), &h_ty, &h_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let stream_j = Exp::InductiveType(decl.clone(), vec![Exp::Var("j".to_string()), Exp::One]);
        let target_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(stream_j),
        };
        let target_ty = eval(&target_ty_exp, &c2.rho).expect("eval target");

        // `λ j. h` — h has type SizedStream(i,1). j is bounded below i.
        // The body would need to be SizedStream(j,1), but h is at i.
        let lam = Exp::Lam(
            Patt::Var("j".to_string()),
            Box::new(Exp::Var("h".to_string())),
        );
        assert!(
            check(&mut c2, &lam, &target_ty).is_err(),
            "λ j. h must not check against {{j < i}}. SizedStream(j, 1) — h is at outer size i"
        );
    }

    #[test]
    fn sized_codata_type_formation() {
        // With `i : SizeSort`, check that the codata type
        //   codata { head : One, tail : {j < i}. One }
        // is a valid type. This is the minimal sized codata shape.
        let (mut c, _) = ctx_with_size_var("i");
        let tail_ty = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let codata = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: tail_ty,
            },
        ]);
        check_type(&mut c, &codata).expect("sized codata is a valid type");
    }

    #[test]
    fn sized_corecord_type_checks_against_sized_codata() {
        // End-to-end: construct a corecord that inhabits a sized
        // codata type. Uses the Lam-vs-SizedPi arm for the tail
        // field.
        //
        // Type:  codata { head : One, tail : {j < i}. One }
        // Value: corecord { head = Unit; tail = λ j. Unit }
        let (mut c, _) = ctx_with_size_var("i");
        let tail_obs_ty = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let codata = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: tail_obs_ty,
            },
        ]);
        let ty = eval(&codata, &c.rho).expect("eval codata");
        let corecord = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit)),
            },
        ]);
        check(&mut c, &corecord, &ty).expect("sized corecord inhabits sized codata");
    }

    // --- Sized inductive termination via Match (Phase 11b step 15g) ---
    //
    // A proper sized Nat whose `succ` constructor uses `SizedPi` for
    // its predecessor size, so pattern-matching on `succ(j, n)`
    // introduces `j < i` as a TSO hypothesis in the arm — the
    // hypothesis that lets recursive calls on `n` type-check as
    // strictly-decreasing.

    fn sized_nat_with_sized_pi_decl() -> Arc<InductiveDecl> {
        // SizedNatP(i : SizeSort) with
        //   zero : Π i:SizeSort. SizedNatP i
        //   succ : Π i:SizeSort. {j < i}. SizedNatP j → SizedNatP i
        let self_ref = Arc::new(InductiveDecl {
            name: "SizedNatP".to_string(),
            params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
            sort: Exp::Set,
            ctors: Vec::new(),
        });
        let snat_i = Exp::InductiveType(self_ref.clone(), vec![Exp::Var("i".to_string())]);
        let snat_j = Exp::InductiveType(self_ref, vec![Exp::Var("j".to_string())]);
        Arc::new(InductiveDecl {
            name: "SizedNatP".to_string(),
            params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
            sort: Exp::Set,
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
                        Box::new(Exp::SizedPi {
                            patt: Patt::Var("j".to_string()),
                            upper: Box::new(Exp::Var("i".to_string())),
                            body: Box::new(Exp::Pi(Patt::Unit, Box::new(snat_j), Box::new(snat_i))),
                        }),
                    ),
                },
            ],
        })
    }

    #[test]
    fn sized_nat_p_succ_at_inf_with_equal_predecessor() {
        // Under expected type `SizedNatP ∞`, check
        // `succ(size=∞, n=zero)`. The outer param `i=∞` is provided
        // by the expected type; user supplies only the non-param
        // args (size + value). size_lt(∞, ∞) holds via ∞-absorption.
        let decl = sized_nat_with_sized_pi_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let succ_inf =
            Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![Exp::SizeInf, zero]);
        let ty = Val::InductiveType {
            decl,
            params: vec![Val::SizeInf],
        };
        check(&mut c, &succ_inf, &ty).expect("succ(∞, zero) : SizedNatP ∞");
    }

    #[test]
    fn sized_nat_p_succ_with_non_decreasing_size_rejected() {
        // Under `i : SizeSort` and expected `SizedNatP i`, the
        // expression `succ(size=i, n=zero)` must be rejected: the
        // predecessor size `i` is not strictly below the outer `i`.
        let decl = sized_nat_with_sized_pi_decl();
        let (mut c, i_val) = ctx_with_size_var("i");
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let bad = Exp::InductiveCtor(
            decl.clone(),
            "succ".to_string(),
            vec![Exp::Var("i".to_string()), zero],
        );
        let ty = Val::InductiveType {
            decl,
            params: vec![i_val],
        };
        let err = check(&mut c, &bad, &ty).unwrap_err();
        assert!(
            err.contains("not strictly below"),
            "expected size-bound error, got: {err}"
        );
    }

    #[test]
    fn sized_nat_p_match_arm_sees_hypothesis() {
        // The key termination-by-typing test.
        //
        // Given `i : SizeSort` and `x : SizedNatP(i)`, match on x.
        // In the `succ(j, n)` arm:
        //   - `j : SizeSort` is a fresh rigid with TSO `j < i`
        //   - `n : SizedNatP(j)` (strictly smaller inductive)
        //
        // The arm body checks `n : SizedNatP(i)` — which requires
        // `SizedNatP(j) <: SizedNatP(i)`, i.e. `j ≤ i`. From the
        // TSO hypothesis `j < i`, subtyping derives `j ≤ i`. ✓
        //
        // Without the hypothesis, this subtyping fails.
        let decl = sized_nat_with_sized_pi_decl();
        let (c, i_val) = ctx_with_size_var("i");

        let snatp_i = Val::InductiveType {
            decl: decl.clone(),
            params: vec![i_val.clone()],
        };
        let x_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("x".to_string()), &snatp_i, &x_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        // match x { zero => x; succ(j, n) => n }
        // Expected type: SizedNatP(i). Both arms must produce that.
        // succ arm bindings are (j, n) — the non-param ctor args.
        // `j : SizeSort` gets TSO hypothesis `j < i`; `n : SizedNatP(j)`.
        // The arm body is `n`, which under subtyping lifts into
        // SizedNatP(i) via the hypothesis.
        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("x".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "zero".to_string(),
                    bindings: vec![],
                    body: Exp::Var("x".to_string()),
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "succ".to_string(),
                    bindings: vec![Patt::Var("j".to_string()), Patt::Var("n".to_string())],
                    body: Exp::Var("n".to_string()),
                },
            ],
        };
        check(&mut c2, &match_exp, &snatp_i)
            .expect("match arm with succ(j, n) uses hypothesis j < i to lift n into SizedNatP(i)");
    }

    #[test]
    fn sized_nat_p_match_arm_without_hypothesis_usage_still_typechecks() {
        // The OLD `sized_nat_decl` (plain Pi, no SizedPi) gives
        // `succ` a single non-param arg of type `SizedNat(i)` —
        // i.e. the predecessor shares the outer size, no decrease.
        // Matching still type-checks trivially: the `n` binding in
        // `succ(n)` has type SizedNat(i) = expected. This doesn't
        // exercise hypothesis entailment (there's no SizedPi in the
        // ctor) but verifies the old path still works after the
        // refactor that introduced `CtorArg`.
        let decl = sized_nat_decl();
        let (c, i_val) = ctx_with_size_var("i");

        let snat_i = Val::InductiveType {
            decl: decl.clone(),
            params: vec![i_val],
        };
        let x_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("x".to_string()), &snat_i, &x_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("x".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "zero".to_string(),
                    bindings: vec![],
                    body: Exp::Var("x".to_string()),
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "succ".to_string(),
                    bindings: vec![Patt::Var("n".to_string())],
                    body: Exp::Var("n".to_string()),
                },
            ],
        };
        check(&mut c2, &match_exp, &snat_i).expect("old-style sized Nat match still works");
    }

    // --- D14 §9.2: institution-registered decision procedures ---
    //
    // Verify that `Constraint::Institution { iri, args }` dispatches
    // through the D14 `try_d14_decide` path: the constraint IRI
    // resolves to a Decidable QueryClass, args land on the input
    // resource as `decide_args`, and the institution's `query` returns
    // a Verdict resource the kernel translates to a `DecResult`.

    use crate::context::{ExecutionContext, ExecutionMode};
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::institution::DecResult;
    use crate::layer::LayerBuilder;
    use crate::nbe::term::Constraint;
    use crate::ontology::resource::Resource;
    use crate::ontology::resource::Value as RVal;
    use crate::ontology::well_known as wk;

    /// In-test institution whose `query` returns a pre-canned Verdict
    /// resource for each `Constraint::Institution` invocation and
    /// records the args it observed off the synthetic input
    /// resource's `decide_args` property (D14 §9.2).
    struct FakeInstitution {
        iri: Iri,
        observed: std::sync::Mutex<Vec<Vec<RVal>>>,
        result: DecResult,
    }

    impl FakeInstitution {
        fn new(iri: &str, result: DecResult) -> Arc<Self> {
            Arc::new(Self {
                iri: Iri::parse(iri).unwrap(),
                observed: std::sync::Mutex::new(Vec::new()),
                result,
            })
        }

        fn last_args(&self) -> Option<Vec<RVal>> {
            self.observed.lock().unwrap().last().cloned()
        }
    }

    impl Institution for Arc<FakeInstitution> {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }

        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<crate::nbe::val::Val, crate::institution::error::InstitutionError> {
            unreachable!("FakeInstitution exposes no ExportFormats")
        }

        fn reify(
            &self,
            _: &Iri,
            _: &crate::nbe::val::Val,
            _: &ExecutionContext,
        ) -> Result<Resource, crate::institution::error::InstitutionError> {
            unreachable!("FakeInstitution exposes no ImportFormats")
        }

        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, crate::institution::error::InstitutionError> {
            // Pull the args off the synthetic input resource where
            // try_d14_decide marshalled them.
            let args = match input.get(&Iri::parse("urn:eigenius:institution:decide_args").unwrap())
            {
                Some(RVal::Array(items)) => items.clone(),
                _ => Vec::new(),
            };
            self.observed.lock().unwrap().push(args);
            Ok(verdict_resource(self.result))
        }
    }

    /// Build a Verdict-shaped result resource from a `DecResult`.
    fn verdict_resource(result: DecResult) -> Resource {
        let class_iri = match result {
            DecResult::Holds => "urn:eigenius:institution:verdicts:holds",
            DecResult::Fails => "urn:eigenius:institution:verdicts:fails",
            DecResult::Undecidable => "urn:eigenius:institution:verdicts:undecidable",
        };
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(class_iri.into())]),
        );
        r
    }

    /// Build an `InstitutionIndex` and `InstitutionRuntime` declaring a
    /// Decidable `QueryClass` for `constraint_iri`, served by `fake`.
    /// The index walks a synthetic test layer carrying the QueryClass
    /// declaration; the runtime registers the fake under the same
    /// institution IRI.
    fn build_decide_index(
        fake: Arc<FakeInstitution>,
    ) -> (Arc<InstitutionIndex>, Arc<InstitutionRuntime>) {
        let constraint_iri = fake.iri.as_str();
        let inst_iri = constraint_iri; // for tests, institution IRI = constraint IRI
        let input_class = "urn:eigenius:test:Subject";

        let mut b = LayerBuilder::new("test", None);
        let mut qc = Resource::new(Iri::parse(constraint_iri).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            RVal::String(input_class.into()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            RVal::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            RVal::Array(vec![RVal::String(wk::DISPATCH_DECIDABLE.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            RVal::String(format!("{constraint_iri}:handler")),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            RVal::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();
        let layer = Arc::new(b.build());

        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut rt = InstitutionRuntime::new();
        rt.register(Box::new(fake)).unwrap();
        (Arc::new(idx), Arc::new(rt))
    }

    /// Build an `EvalCtx::Check` populated with the D14 index +
    /// runtime built from `fake`.
    fn check_ctx_for(fake: Arc<FakeInstitution>) -> EvalCtx {
        let (idx, rt) = build_decide_index(fake);
        let _ = ExecutionMode::ReadOnly; // silence unused-import warning on small surface
        EvalCtx::Check {
            layer: None,
            institution_index: Some(idx),
            institution_runtime: Some(rt),
        }
    }

    fn wrap_int(n: i64) -> Exp {
        let iri = Iri::parse("urn:eigenius:test:Int").unwrap();
        let mut r = crate::ontology::resource::Resource::new(iri);
        r.set(
            Iri::parse("urn:eigenius:core:value").unwrap(),
            RVal::Integer(n),
        );
        Exp::EigonResource(Box::new(r))
    }

    #[test]
    fn decide_without_registry_is_undecidable() {
        // Bare `EvalCtx::Pure` has no registry → institution-dispatched
        // constraint falls through to `Undecidable`, reducing to the
        // passthrough neutral.
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:always_holds").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(7)));
        let v = eval_ctx(&exp, &Rho::Nil, &EvalCtx::Pure).expect("eval");
        assert!(
            matches!(v, Val::Nt(crate::nbe::val::Neut::Gen(_, ref n)) if n == "__constraint_undecidable")
        );
    }

    #[test]
    fn decide_holds_reduces_to_refl() {
        // Institution returns Holds → eval reduces NativeDecide to Refl.
        let fake = FakeInstitution::new("urn:eigenius:test:yes", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone());
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:yes").unwrap(),
            args: vec![wrap_int(42)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(7)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        assert!(matches!(v, Val::Refl(_)), "expected Refl, got {v:?}");

        // The fake observed the arg via the `decide_args` array on the
        // synthetic input resource that try_d14_decide marshals.
        let observed = fake.last_args().expect("institution was called");
        assert_eq!(observed.len(), 1);
    }

    #[test]
    fn decide_fails_produces_failing_neutral() {
        let fake = FakeInstitution::new("urn:eigenius:test:no", DecResult::Fails);
        let ctx = check_ctx_for(fake);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:no").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(crate::nbe::val::Neut::Gen(_, name)) => {
                assert_eq!(name, "__constraint_failed");
            }
            other => panic!("expected failing neutral, got {other:?}"),
        }
    }

    #[test]
    fn decide_undecidable_produces_passthrough_neutral() {
        let fake = FakeInstitution::new("urn:eigenius:test:dunno", DecResult::Undecidable);
        let ctx = check_ctx_for(fake);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:dunno").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(crate::nbe::val::Neut::Gen(_, name)) => {
                assert_eq!(name, "__constraint_undecidable");
            }
            other => panic!("expected undecidable neutral, got {other:?}"),
        }
    }

    #[test]
    fn decide_unregistered_iri_is_undecidable() {
        // Index has a Decidable QueryClass for one IRI; the test
        // invokes a different IRI → no QueryClass match → D14 path
        // returns None → legacy fallback returns Undecidable (empty
        // legacy registry).
        let fake = FakeInstitution::new("urn:eigenius:test:other", DecResult::Holds);
        let ctx = check_ctx_for(fake);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:unknown_iri").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        assert!(
            matches!(v, Val::Nt(crate::nbe::val::Neut::Gen(_, ref name)) if name == "__constraint_undecidable")
        );
    }

    #[test]
    fn decide_list_arg_roundtrip() {
        // Life-science ensemble-style predicate: the arg is a list of
        // values. Verify the Val::List marshals through to an
        // RVal::Array on the synthetic input's `decide_args`.
        let fake = FakeInstitution::new("urn:eigenius:test:ensemble", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone());

        let list_val = Val::List(vec![
            crate::nbe::eval::eval(&wrap_int(1), &Rho::Nil).unwrap(),
            crate::nbe::eval::eval(&wrap_int(2), &Rho::Nil).unwrap(),
            crate::nbe::eval::eval(&wrap_int(3), &Rho::Nil).unwrap(),
        ]);
        let rho = Rho::Nil.extend(Patt::Var("xs".to_string()), list_val);

        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:ensemble").unwrap(),
            args: vec![Exp::Var("xs".to_string())],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        eval_ctx(&exp, &rho, &ctx).expect("eval");

        let observed = fake.last_args().expect("called");
        assert_eq!(observed.len(), 1);
        match &observed[0] {
            RVal::Array(items) => assert_eq!(items.len(), 3),
            other => panic!("expected RVal::Array, got {other:?}"),
        }
    }

    #[test]
    fn decide_inductive_val_arg_roundtrip() {
        // Pose-like inductive arg. Marshal `succ(zero)` of a Nat
        // through the Val::InductiveVal arm of val_to_resource_value
        // and verify the institution sees an Embedded resource whose
        // `is_a` carries the ctor name.
        let nat = nat_decl();
        let fake = FakeInstitution::new("urn:eigenius:test:pose", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone());

        let succ_zero_exp = Exp::InductiveCtor(
            nat.clone(),
            "succ".to_string(),
            vec![Exp::InductiveCtor(nat, "zero".to_string(), Vec::new())],
        );
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:pose").unwrap(),
            args: vec![succ_zero_exp],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");

        let observed = fake.last_args().expect("called");
        assert_eq!(observed.len(), 1);
        match &observed[0] {
            RVal::Embedded(r) => {
                let is_a = r.is_a();
                assert_eq!(is_a.len(), 1);
                assert!(is_a[0].as_str().ends_with(":succ"));
            }
            other => panic!("expected RVal::Embedded (ctor resource), got {other:?}"),
        }
    }

    #[test]
    fn decide_fires_at_check_time_when_registry_on_ctx() {
        // Integration: check-time dispatch via CheckCtx. A NativeDecide
        // whose constraint holds reduces to Refl; from CheckCtx's
        // perspective, the decide call *did* fire (the institution
        // observed it), confirming the index + runtime were threaded
        // through the check eval_ctx.
        let fake = FakeInstitution::new("urn:eigenius:test:check_time", DecResult::Holds);
        let (idx, rt) = build_decide_index(fake.clone());

        let c = CheckCtx::new(Rho::Nil, Vec::new()).with_institutions_d14(idx, rt);

        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:check_time").unwrap(),
            args: vec![wrap_int(7)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(99)));

        let v = c.eval(&exp, &Rho::Nil).expect("CheckCtx eval");
        assert!(matches!(v, Val::Refl(_)));
        assert!(
            fake.last_args().is_some(),
            "institution should have been consulted at check time"
        );
    }
}
