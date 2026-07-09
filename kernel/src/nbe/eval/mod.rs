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

//! EigenTT evaluator: terms → values.
//!
//! Ported from `Main.hs` lines 198-217 in the EigenTT reference.
//! Extended with capability modes (Pure/Read/IO) per D9.

mod hooks;
mod iota;
mod mapreduce;
mod marshal;
#[cfg(test)]
mod testutil;
mod tracer;

pub use hooks::{Decision, EffectHooks};
use iota::iota_reduce_impl;
use mapreduce::{eval_map_impl, eval_reduce_impl};
pub use marshal::{resource_value_to_val, val_to_resource_value};
pub(crate) use tracer::{NoTrace, Tracer, TreeTracer};

/// Evaluation error — replaces panics in the NbE evaluator (issue #19).
///
/// Covers all error conditions that previously caused `panic!` in
/// `eval_ctx`, `eval_traced`, and the Val/Clos methods in `val.rs`.
#[derive(Debug, Clone)]
pub enum EvalError {
    /// Variable not found in the evaluation environment.
    UnboundVariable(String),
    /// Constructor name not found in a case/Fun dispatch.
    ConstructorNotFound(String),
    /// Case function applied to a non-constructor, non-neutral value.
    InvalidCaseTarget(String),
    /// Application of a non-function value.
    NotAFunction(String),
    /// First/second projection on a non-pair value.
    NotAPair(String),
    /// Observation on a non-corecord value.
    NotACorecord(String),
    /// Named observation not found in a corecord.
    ObservationNotFound(String),
    /// Function called outside its required capability mode.
    ModeError(String),
    /// A code path is acknowledged but not yet implemented.
    /// Used while incrementally landing larger features (e.g. the
    /// inductive recursor stub during Phase 11b).
    NotImplemented(String),
    /// An IO or deterministic component dispatch errored out.
    /// `dispatch_component` previously masked these by returning an
    /// empty embedded resource, which then flowed silently into
    /// downstream `Construct` fields and surfaced (at best) as a
    /// chain-validation error with no link back to the actual dispatch
    /// failure. Propagating the original error gives the user a
    /// useful diagnostic.
    ComponentDispatchFailed {
        component_iri: String,
        message: String,
    },
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnboundVariable(s) => write!(f, "unbound variable: {s}"),
            Self::ConstructorNotFound(s) => write!(f, "constructor not found: {s}"),
            Self::InvalidCaseTarget(s) => write!(f, "invalid case target: {s}"),
            Self::NotAFunction(s) => write!(f, "not a function: {s}"),
            Self::NotAPair(s) => write!(f, "not a pair: {s}"),
            Self::NotACorecord(s) => write!(f, "not a corecord: {s}"),
            Self::ObservationNotFound(s) => write!(f, "observation not found: {s}"),
            Self::ModeError(s) => write!(f, "mode error: {s}"),
            Self::NotImplemented(s) => write!(f, "not yet implemented: {s}"),
            Self::ComponentDispatchFailed {
                component_iri,
                message,
            } => write!(f, "component '{component_iri}' failed: {message}"),
        }
    }
}

impl std::error::Error for EvalError {}

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Constraint, Exp, Patt};
use crate::nbe::val::{Clos, Neut, Val};
use crate::observability::{field, operation};
use crate::ontology::iri::Iri;
use crate::program::trace::Trace;
use std::sync::Arc;

/// Evaluation context controlling what effects are available.
///
/// `Pure` is standard NbE — no side effects, no chain access (the type
/// checker's default). `Effectful` carries an [`EffectHooks`]
/// implementation ([`crate::institution::eval_hooks::InstitutionEngine`])
/// that the three effectful expression forms delegate to; the
/// capability tier (full IO vs. check-time institution deciding) is a
/// property of the hooks impl, not the enum. Optional `layer` gives the
/// evaluator read access to the chain for the few places that need it.
#[derive(Clone)]
pub enum EvalCtx {
    /// Standard NbE: normalize terms, check types. No side effects.
    Pure,
    /// Effectful evaluation: institution dispatch / IO component
    /// invocation delegated to `hooks`.
    Effectful {
        layer: Option<Arc<Layer>>,
        hooks: Arc<dyn EffectHooks>,
    },
}

impl EvalCtx {
    /// A static Pure context for convenience.
    pub fn pure() -> Self {
        EvalCtx::Pure
    }

    /// Construct an effectful context from a hooks implementation.
    pub fn effectful(layer: Option<Arc<Layer>>, hooks: Arc<dyn EffectHooks>) -> Self {
        EvalCtx::Effectful { layer, hooks }
    }

    /// Layer for this evaluation context, if any.
    pub fn layer(&self) -> Option<&Arc<Layer>> {
        match self {
            EvalCtx::Pure => None,
            EvalCtx::Effectful { layer, .. } => layer.as_ref(),
        }
    }

    /// Effect hooks for this context, if effectful.
    pub fn hooks(&self) -> Option<&Arc<dyn EffectHooks>> {
        match self {
            EvalCtx::Pure => None,
            EvalCtx::Effectful { hooks, .. } => Some(hooks),
        }
    }
}

/// Evaluate an expression in an environment to produce a semantic value.
/// Pure mode — no IO, no layer access. Used by the type checker.
pub fn eval(exp: &Exp, rho: &Rho) -> Result<Val, EvalError> {
    eval_ctx(exp, rho, &EvalCtx::Pure)
}

/// Evaluate an expression with a capability mode.
pub fn eval_ctx(exp: &Exp, rho: &Rho, ctx: &EvalCtx) -> Result<Val, EvalError> {
    eval_impl::<NoTrace>(exp, rho, ctx).map(|(v, ())| v)
}

/// Evaluate an expression with tracing.
///
/// Same evaluator as [`eval_ctx`] (`eval_impl` instantiated with
/// [`TreeTracer`] instead of [`NoTrace`]); additionally returns the
/// tree-structured provenance trace used to build ProgramTraces at the
/// IO run boundary (D6b §2). Pure subtrees carry `None`.
pub fn eval_traced(exp: &Exp, rho: &Rho, ctx: &EvalCtx) -> Result<(Val, Option<Trace>), EvalError> {
    eval_impl::<TreeTracer>(exp, rho, ctx)
}

/// The evaluator, generic over the tracing strategy `T`.
///
/// Every arm returns the value plus a `T::Node` describing the
/// provenance of its computation. With `T = NoTrace` the node is `()`
/// and the whole tracing dimension compiles away — this instantiation
/// is the type checker's hot path. With `T = TreeTracer` the nodes
/// form the D6b trace tree; structural arms combine their children's
/// nodes so nested effects are never dropped (F-5, NbE analysis §3.2).
pub(crate) fn eval_impl<T: Tracer>(
    exp: &Exp,
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<(Val, T::Node), EvalError> {
    // Shorthand for recursive calls in the same env/ctx.
    let ev = |e: &Exp| -> Result<(Val, T::Node), EvalError> { eval_impl::<T>(e, rho, ctx) };

    match exp {
        Exp::Sort(n) => Ok((Val::Sort(*n), T::leaf())),
        Exp::One => Ok((Val::One, T::leaf())),
        Exp::Unit => Ok((Val::Unit, T::leaf())),

        // eigenius#71 / D49 — literals normalise to themselves; no
        // reduction, no neutral substructure.
        Exp::LitString(s) => Ok((Val::LitString(s.clone()), T::leaf())),
        Exp::LitInt(n) => Ok((Val::LitInt(*n), T::leaf())),
        Exp::LitFloat(f) => Ok((Val::LitFloat(*f), T::leaf())),

        Exp::Dec(d, e) => {
            match ctx {
                EvalCtx::Pure => {
                    // Pure mode: lazy evaluation via UpDec (standard
                    // EigenTT). No Let node — the value is not forced here.
                    eval_impl::<T>(e, &Rho::UpDec(Box::new(rho.clone()), d.clone()), ctx)
                }
                _ => {
                    // IO/Read mode: eagerly evaluate the declaration value
                    // so that IO dispatch happens in the correct context.
                    match d {
                        crate::nbe::term::Decl::Def(patt, _typ, body) => {
                            let (val, value_node) = eval_impl::<T>(body, rho, ctx)?;
                            let rho2 = rho.clone().extend(patt.clone(), val);
                            let (body_val, body_node) = eval_impl::<T>(e, &rho2, ctx)?;
                            Ok((body_val, T::let_node(patt, value_node, body_node)))
                        }
                        crate::nbe::term::Decl::Drec(patt, _typ, body) => {
                            // Recursive: evaluate in extended env
                            let rho_ext = Rho::UpDec(Box::new(rho.clone()), d.clone());
                            let (val, value_node) = eval_impl::<T>(body, &rho_ext, ctx)?;
                            let rho2 = rho.clone().extend(patt.clone(), val);
                            let (body_val, body_node) = eval_impl::<T>(e, &rho2, ctx)?;
                            Ok((body_val, T::let_node(patt, value_node, body_node)))
                        }
                    }
                }
            }
        }

        Exp::Lam(p, e) => Ok((
            Val::Lam(Clos::new(p.clone(), *e.clone(), rho.clone())),
            T::leaf(),
        )),

        Exp::Pi(p, a, b) => {
            let (a_val, a_node) = ev(a)?;
            Ok((
                Val::Pi(
                    Box::new(a_val),
                    Clos::new(p.clone(), *b.clone(), rho.clone()),
                ),
                a_node,
            ))
        }

        Exp::Sig(p, a, b) => {
            let (a_val, a_node) = ev(a)?;
            Ok((
                Val::Sig(
                    Box::new(a_val),
                    Clos::new(p.clone(), *b.clone(), rho.clone()),
                ),
                a_node,
            ))
        }

        Exp::Fst(e) => {
            let (v, node) = ev(e)?;
            Ok((v.vfst()?, node))
        }
        Exp::Snd(e) => {
            let (v, node) = ev(e)?;
            Ok((v.vsnd()?, node))
        }

        Exp::App(e1, e2) => {
            // In effectful mode, intercept component-call-shaped
            // applications: when the LHS is a Var naming a registered
            // Component, dispatch through the effect hooks. A component
            // call evaluates its argument first (then dispatches); an
            // ordinary application evaluates the function first — the
            // `is_component` predicate lets us pick the branch before
            // evaluating either side, preserving evaluation order.
            // Institution capabilities don't appear here — programs
            // reach institutions only via `Exp::InstitutionInvoke`
            // (comorphisms) and `Exp::NativeDecide` (Decidable
            // QueryClasses).
            if let (EvalCtx::Effectful { hooks, .. }, Exp::Var(name)) = (ctx, e1.as_ref()) {
                if hooks.is_component(name) {
                    let (arg_val, arg_node) = ev(e2)?;
                    let (val, comp_trace) = hooks.dispatch_component(name, &arg_val)?;
                    let node = T::combine(vec![arg_node, T::component(comp_trace)]);
                    return Ok((val, node));
                }
            }
            let (f_val, f_node) = ev(e1)?;
            let (arg_val, arg_node) = ev(e2)?;
            let (result, app_node) = f_val.app_impl::<T>(arg_val, arg_node, ctx)?;
            Ok((result, T::combine(vec![f_node, app_node])))
        }

        // Type annotations are runtime-erased: `⟦(e : T)⟧ = ⟦e⟧`. (The
        // annotation only matters to `check_infer` — see check.rs.)
        Exp::Ann(e, _t) => ev(e),

        Exp::Var(x) => match rho.get(x) {
            Ok(val) => Ok((val, T::leaf())),
            Err(e) => match ctx {
                EvalCtx::Pure => Err(EvalError::UnboundVariable(e)),
                _ => {
                    // IO/Read mode: unbound variables may be component IRIs
                    // that will be intercepted at the App level.
                    Ok((Val::Nt(Neut::Gen(usize::MAX, x.clone())), T::leaf()))
                }
            },
        },

        Exp::Pair(e1, e2) => {
            let (v1, n1) = ev(e1)?;
            let (v2, n2) = ev(e2)?;
            Ok((
                Val::Pair(Box::new(v1), Box::new(v2)),
                T::combine(vec![n1, n2]),
            ))
        }

        Exp::Con(c, e) => {
            let (v, node) = ev(e)?;
            Ok((Val::Con(c.clone(), Box::new(v)), node))
        }

        Exp::Data(summands) => Ok((
            Val::Data(
                summands
                    .iter()
                    .map(|s| (s.name.clone(), s.typ.clone()))
                    .collect(),
                rho.clone(),
            ),
            T::leaf(),
        )),

        Exp::Case(branches) => Ok((
            Val::Fun(
                branches
                    .iter()
                    .map(|b| (b.name.clone(), b.body.clone()))
                    .collect(),
                rho.clone(),
            ),
            T::leaf(),
        )),

        // Sugar: A → B = Π _ : A. B  (direct construction, Phase 10c)
        Exp::Arrow(a, b) => {
            let (a_val, a_node) = ev(a)?;
            Ok((
                Val::Pi(
                    Box::new(a_val),
                    Clos::new(Patt::Unit, *b.clone(), rho.clone()),
                ),
                a_node,
            ))
        }
        // Sugar: A × B = Σ _ : A. B  (direct construction, Phase 10c)
        Exp::Times(a, b) => {
            let (a_val, a_node) = ev(a)?;
            Ok((
                Val::Sig(
                    Box::new(a_val),
                    Clos::new(Patt::Unit, *b.clone(), rho.clone()),
                ),
                a_node,
            ))
        }

        // Identity type
        Exp::Id(a, x, y) => {
            let (a_val, an) = ev(a)?;
            let (x_val, xn) = ev(x)?;
            let (y_val, yn) = ev(y)?;
            Ok((
                Val::Id(Box::new(a_val), Box::new(x_val), Box::new(y_val)),
                T::combine(vec![an, xn, yn]),
            ))
        }
        Exp::Refl(a) => {
            let (v, node) = ev(a)?;
            Ok((Val::Refl(Box::new(v)), node))
        }
        Exp::IdJ(args) => {
            let [_a, _c, d, _x, _y, p] = args.as_ref();
            let (p_val, p_node) = ev(p)?;
            match p_val {
                Val::Refl(a_val) => {
                    let (d_val, d_node) = ev(d)?;
                    let (result, app_node) = d_val.app_impl::<T>(*a_val, T::leaf(), ctx)?;
                    Ok((result, T::combine(vec![p_node, d_node, app_node])))
                }
                Val::Nt(n) => {
                    // Blocked — all args become neutral
                    Ok((Val::Nt(Neut::App(Box::new(n), Box::new(Val::Unit))), p_node))
                }
                _ => {
                    // Stuck — proof argument is neither Refl nor neutral.
                    // Return a stuck neutral rather than panicking (Phase 10c).
                    Ok((
                        Val::Nt(Neut::Gen(usize::MAX, "__j_stuck".to_string())),
                        p_node,
                    ))
                }
            }
        }

        // Cross-institution translation via declared comorphism.
        //
        // D14 §9.3 four-step pipeline: resolve the Comorphism resource
        // in the InstitutionIndex, extract a typed payload via the
        // source institution's ExportFormat procedure, apply the
        // transformation Component, reify a target-class resource via
        // the target institution's ImportFormat procedure. The
        // post-translation validation invariant (D14 §9.3 step 5)
        // runs as part of [`try_institution_invoke`].
        //
        // When the evaluator has no institution backing attached (bare Pure
        // mode used during type-check / conversion), the call reduces
        // to a passthrough neutral so the conversion checker can
        // compare two `InstitutionInvoke`s structurally. When the
        // backing IS attached but the comorphism cannot be resolved,
        // the dispatch surfaces a typed error.
        Exp::InstitutionInvoke {
            comorphism_iri,
            source,
            target_iri,
        } => {
            let (source_val, source_node) = ev(source)?;
            // No effect hooks (Pure), or hooks with no institution
            // backing → passthrough neutral so the conversion checker
            // can compare two `InstitutionInvoke`s structurally.
            let passthrough = || {
                Val::Nt(Neut::Gen(
                    usize::MAX,
                    format!("__institution_invoke_no_registry:{comorphism_iri}"),
                ))
            };
            match ctx.hooks() {
                None => Ok((passthrough(), source_node)),
                Some(hooks) => {
                    match hooks.institution_invoke(
                        comorphism_iri,
                        &source_val,
                        target_iri.as_ref(),
                    )? {
                        Some(translated) => {
                            let node = T::comorphism(comorphism_iri, source_node, &translated);
                            Ok((translated, node))
                        }
                        None => Ok((passthrough(), source_node)),
                    }
                }
            }
        }

        // Native constraint checking. Structural constraints reduce in
        // the pure core (available in any mode); institution-bound ones
        // dispatch through the effect hooks (Undecidable without them).
        Exp::NativeDecide(constraint, val) => {
            let (v, node) = ev(val)?;
            let decision = match constraint {
                Constraint::Institution { .. } => match ctx.hooks() {
                    Some(hooks) => hooks.decide_institution(constraint, &v, rho, ctx)?,
                    None => Decision::Undecidable,
                },
                structural => decide_structural(structural, &v),
            };
            let result = match decision {
                Decision::Holds => Val::Refl(Box::new(v)),
                Decision::Fails => {
                    Val::Nt(Neut::Gen(usize::MAX, "__constraint_failed".to_string()))
                }
                Decision::Undecidable => Val::Nt(Neut::Gen(
                    usize::MAX,
                    "__constraint_undecidable".to_string(),
                )),
            };
            Ok((result, node))
        }

        // Decidable equality on ground types
        Exp::DecEq(_a, x, y) => {
            let (x_val, xn) = ev(x)?;
            let (y_val, yn) = ev(y)?;
            let result = if ground_values_equal(&x_val, &y_val) {
                Val::Refl(Box::new(x_val))
            } else {
                Val::Nt(Neut::Gen(usize::MAX, "__deceq_false".to_string()))
            };
            Ok((result, T::combine(vec![xn, yn])))
        }

        // Template literal — evaluate type expressions for each reference
        Exp::Template(s, refs) => {
            let mut resolved = Vec::new();
            let mut nodes = Vec::new();
            for (iri, typ) in refs {
                let (v, node) = ev(typ)?;
                resolved.push((iri.clone(), v));
                nodes.push(node);
            }
            Ok((Val::TemplateVal(s.clone(), resolved), T::combine(nodes)))
        }

        // Eigenius extensions
        Exp::EigonClass(iri) => Ok((Val::EigonClass(iri.clone()), T::leaf())),
        // Axiom references evaluate to a neutral spine head — the
        // existing `Neut::App` machinery then handles applications
        // (`stats:lt(a, b)` → `Val::Nt(Neut::App(Neut::App(Neut::EigonAxiom(lt), a), b))`).
        Exp::EigonAxiom(iri) => Ok((
            Val::Nt(crate::nbe::val::Neut::EigonAxiom(iri.clone())),
            T::leaf(),
        )),
        Exp::EigonPrimitive(p) => Ok((Val::EigonPrimitive(*p), T::leaf())),
        Exp::EigonResource(r) => Ok((Val::ResourceVal(r.clone()), T::leaf())),

        Exp::PropAccess(e, prop) => {
            let (v, source_node) = ev(e)?;
            match v {
                Val::ResourceVal(r) => {
                    // Direct property access on a known resource
                    let result = match r.get(prop) {
                        Some(val) => resource_value_to_val(val),
                        None => {
                            tracing::warn!(
                                { field::OPERATION } = operation::NBE_EVAL,
                                { field::ERROR_KIND } = "property_missing",
                                { field::PROPERTY_IRI } = %prop,
                                "property not found on resource during eval; returning Unit"
                            );
                            Val::Unit
                        }
                    };
                    Ok((result, T::project(source_node, prop)))
                }
                // Codata observation: the "property" IRI's local name is
                // treated as the observation name (D11 §8). Evaluate the
                // matching field body in the corecord's captured env.
                Val::CoRecord(fields, corecord_rho) => {
                    let obs_name = prop.local_name();
                    for (name, body) in &fields {
                        if name == obs_name {
                            let (v, body_node) = eval_impl::<T>(body, &corecord_rho, ctx)?;
                            return Ok((v, T::combine(vec![source_node, body_node])));
                        }
                    }
                    tracing::warn!(
                        { field::OPERATION } = operation::NBE_EVAL,
                        { field::ERROR_KIND } = "observation_missing",
                        observation = %obs_name,
                        "observation not found in corecord during eval; returning Unit"
                    );
                    Ok((Val::Unit, source_node))
                }
                Val::Nt(n) => Ok((
                    Val::Nt(Neut::PropAccess(Box::new(n), prop.clone())),
                    T::project(source_node, prop),
                )),
                _other => {
                    tracing::warn!(
                        { field::OPERATION } = operation::NBE_EVAL,
                        { field::ERROR_KIND } = "property_access_non_resource",
                        "property access on non-resource value during eval; returning Unit"
                    );
                    Ok((Val::Unit, source_node))
                }
            }
        }

        Exp::Construct(class_iri, fields) => {
            use crate::ontology::resource::{Resource, Value};
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse("urn:eigenius:core:is_a").unwrap(),
                Value::Array(vec![Value::String(class_iri.as_str().to_string())]),
            );
            let mut field_nodes = Vec::with_capacity(fields.len());
            for (prop_iri, expr) in fields {
                let (val, node) = ev(expr)?;
                let rval = val_to_resource_value(&val);
                r.set(prop_iri.clone(), rval);
                field_nodes.push((prop_iri.clone(), node));
            }
            Ok((Val::ResourceVal(Box::new(r)), T::construct(field_nodes)))
        }

        // Codata (D11, Phase 9b-i)
        Exp::Codata(observations) => Ok((
            Val::Codata(
                observations
                    .iter()
                    .map(|o| (o.name.clone(), o.typ.clone()))
                    .collect(),
                rho.clone(),
            ),
            T::leaf(),
        )),

        Exp::CoRecord(fields) => Ok((
            Val::CoRecord(
                fields
                    .iter()
                    .map(|f| (f.name.clone(), f.body.clone()))
                    .collect(),
                rho.clone(),
            ),
            T::leaf(),
        )),

        Exp::Observe(e, name) => {
            let (v, source_node) = ev(e)?;
            let (result, obs_node) = v.vobserve_impl::<T>(name, ctx)?;
            Ok((result, T::combine(vec![source_node, obs_node])))
        }

        // Map/Reduce (Phase 11a)
        Exp::Map(f, coll) => {
            let (f_val, f_node) = ev(f)?;
            let (coll_val, coll_node) = ev(coll)?;
            let (result, map_node) = eval_map_impl::<T>(f_val, coll_val, ctx)?;
            Ok((result, T::combine(vec![f_node, coll_node, map_node])))
        }
        Exp::Reduce(f, init, coll) => {
            let (f_val, f_node) = ev(f)?;
            let (acc, init_node) = ev(init)?;
            let (coll_val, coll_node) = ev(coll)?;
            let (result, reduce_node) = eval_reduce_impl::<T>(f_val, acc, coll_val, ctx)?;
            Ok((
                result,
                T::combine(vec![f_node, init_node, coll_node, reduce_node]),
            ))
        }

        // Inductive types (Phase 11b, D19; D48 adds indices)
        // Step 1 lands the AST and value shells; Step 2 will add iota
        // reduction for the recursor. Pre-D48 callers always have
        // `indices: Vec::new()` (non-indexed default).
        Exp::Inductive(decl) => Ok((
            Val::InductiveType {
                decl: decl.clone(),
                params: Vec::new(),
                indices: Vec::new(),
            },
            T::leaf(),
        )),
        Exp::InductiveType(decl, args) => {
            // D48: `Exp::InductiveType(decl, args)` carries `params ++ indices`
            // — `decl.params.len()` parameters followed by `decl.indices.len()`
            // index expressions. For pre-D48 (non-indexed) decls, `indices`
            // is empty and `args` equals the parameter prefix.
            //
            // The kernel uses "stub" InductiveDecls inside ctor type
            // bodies (self-references with empty `params` / `ctors`,
            // see `term.rs` around `InductiveDecl::PartialEq` — name-
            // based equality). Stubs are detected by `decl.indices`
            // being empty; for those we preserve the pre-D48 behaviour
            // (all args treated as params, no arity check) so the
            // stub-Arc pattern keeps working. Genuine indexed decls
            // (`decl.indices` non-empty) get the strict split.
            let mut vals = Vec::with_capacity(args.len());
            let mut nodes = Vec::with_capacity(args.len());
            for a in args {
                let (v, n) = ev(a)?;
                vals.push(v);
                nodes.push(n);
            }
            let node = T::combine(nodes);
            if decl.indices.is_empty() {
                Ok((
                    Val::InductiveType {
                        decl: decl.clone(),
                        params: vals,
                        indices: Vec::new(),
                    },
                    node,
                ))
            } else {
                let n_params = decl.params.len();
                let n_indices = decl.indices.len();
                let expected = n_params + n_indices;
                if vals.len() != expected {
                    return Err(EvalError::InvalidCaseTarget(format!(
                        "indexed InductiveType `{}`: expected {} arg(s) \
                         (params + indices: {} + {}), got {}",
                        decl.name,
                        expected,
                        n_params,
                        n_indices,
                        vals.len()
                    )));
                }
                let indices = vals.split_off(n_params);
                Ok((
                    Val::InductiveType {
                        decl: decl.clone(),
                        params: vals,
                        indices,
                    },
                    node,
                ))
            }
        }
        Exp::CodataType(decl, params) => {
            let mut vals = Vec::with_capacity(params.len());
            let mut nodes = Vec::with_capacity(params.len());
            for p in params {
                let (v, n) = ev(p)?;
                vals.push(v);
                nodes.push(n);
            }
            Ok((
                Val::CodataType {
                    decl: decl.clone(),
                    params: vals,
                },
                T::combine(nodes),
            ))
        }
        Exp::InductiveCtor(decl, ctor_name, args) => {
            let mut vals = Vec::with_capacity(args.len());
            let mut nodes = Vec::with_capacity(args.len());
            for a in args {
                let (v, n) = ev(a)?;
                vals.push(v);
                nodes.push(n);
            }
            Ok((
                Val::InductiveVal {
                    decl: decl.clone(),
                    ctor_name: ctor_name.clone(),
                    args: vals,
                },
                T::combine(nodes),
            ))
        }
        Exp::InductiveRec {
            decl,
            motive,
            minors,
            major,
        } => {
            let (motive_val, motive_node) = ev(motive)?;
            let mut minor_vals = Vec::with_capacity(minors.len());
            let mut nodes = vec![motive_node];
            for m in minors {
                let (v, n) = ev(m)?;
                minor_vals.push(v);
                nodes.push(n);
            }
            let (major_val, major_node) = ev(major)?;
            nodes.push(major_node);
            match major_val {
                Val::Nt(n) => Ok((
                    Val::Nt(Neut::NtRec {
                        decl: decl.clone(),
                        motive: Box::new(motive_val),
                        minors: minor_vals,
                        major: Box::new(n),
                    }),
                    T::combine(nodes),
                )),
                Val::InductiveVal {
                    ctor_name, args, ..
                } => {
                    let (result, iota_node) = iota_reduce_impl::<T>(
                        decl,
                        &motive_val,
                        &minor_vals,
                        &ctor_name,
                        &args,
                        ctx,
                    )?;
                    nodes.push(iota_node);
                    Ok((result, T::combine(nodes)))
                }
                other => Err(EvalError::InvalidCaseTarget(format!(
                    "InductiveRec: expected inductive value, got {other:?}"
                ))),
            }
        }

        // Pattern-match elimination (Phase 11b step 12, D19 §10).
        // Motive-free: dispatches on a constructor scrutinee directly to
        // the matching arm, binding the constructor's arguments to the
        // arm's binding patterns. IHs from the recursor are deliberately
        // not exposed to user code (a future "IH-aware match" extension
        // would expose them).
        Exp::Match { scrutinee, arms } => {
            let (scrutinee_val, scrutinee_node) = ev(scrutinee)?;
            match scrutinee_val {
                Val::InductiveVal {
                    ctor_name, args, ..
                } => {
                    let (result, body_node) =
                        match_dispatch::<T>(arms, &ctor_name, &args, rho, ctx)?;
                    Ok((result, T::case(scrutinee_node, &ctor_name, body_node)))
                }
                Val::Nt(n) => Ok((
                    Val::Nt(Neut::NtMatch {
                        scrutinee: Box::new(n),
                        arms: arms.clone(),
                        env: rho.clone(),
                    }),
                    scrutinee_node,
                )),
                other => Err(EvalError::InvalidCaseTarget(format!(
                    "Match: expected inductive value, got {other:?}"
                ))),
            }
        }

        // Sized types (Phase 11b step 14, D19 §8).
        Exp::SizeSort => Ok((Val::SizeSort, T::leaf())),
        // ∞ is a fixed point of successor: `ŝ(∞) = ∞`. Matches
        // MiniAgda's `sizeSuccE Infty = Infty` (Abstract.hs:300).
        // Without this absorption, `SizeSucc(SizeInf)` and `SizeInf`
        // would compare unequal, creating spurious type mismatches
        // whenever code mixes sized and unsized (`∞`-indexed) uses.
        Exp::SizeSucc(s) => {
            let (v, node) = ev(s)?;
            match v {
                Val::SizeInf => Ok((Val::SizeInf, node)),
                other => Ok((Val::SizeSucc(Box::new(other)), node)),
            }
        }
        Exp::SizeInf => Ok((Val::SizeInf, T::leaf())),

        Exp::SizedPi { patt, upper, body } => {
            let (upper_val, node) = ev(upper)?;
            Ok((
                Val::SizedPi(
                    Box::new(upper_val),
                    Clos::new(patt.clone(), *body.clone(), rho.clone()),
                ),
                node,
            ))
        }
    }
}

/// Dispatch a constructor-shaped scrutinee to the matching arm's body.
///
/// Locates the arm whose `ctor_name` matches, binds each constructor
/// argument to the corresponding arm binding pattern, and evaluates
/// the body in the extended environment.
///
/// Mismatch between the constructor's arity and the arm's binding
/// count is a build-time invariant violation (the type checker should
/// have caught it), so we surface a clear runtime error rather than
/// silently truncate.
fn match_dispatch<T: Tracer>(
    arms: &[crate::nbe::term::MatchArm],
    ctor_name: &str,
    args: &[Val],
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<(Val, T::Node), EvalError> {
    let arm = arms
        .iter()
        .find(|a| a.ctor_name == ctor_name)
        .ok_or_else(|| {
            EvalError::InvalidCaseTarget(format!(
                "Match: no arm for constructor `{ctor_name}` (non-exhaustive — this should \
             have been caught at type-check time)"
            ))
        })?;
    if arm.bindings.len() != args.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "Match arm `{ctor_name}` expects {} bindings, got {} args (this should have \
             been caught at type-check time)",
            arm.bindings.len(),
            args.len()
        )));
    }
    let mut env = rho.clone();
    for (patt, val) in arm.bindings.iter().zip(args.iter()) {
        env = env.extend(patt.clone(), val.clone());
    }
    eval_impl::<T>(&arm.body, &env, ctx)
}

/// Check equality of ground-type values.
/// Returns true for equal concrete values, false otherwise.
/// Handles: EigonPrimitive-wrapped resources, EigonClass IRIs, Unit.
fn ground_values_equal(x: &Val, y: &Val) -> bool {
    match (x, y) {
        (Val::Unit, Val::Unit) => true,
        (Val::EigonClass(a), Val::EigonClass(b)) => a == b,
        (Val::EigonPrimitive(a), Val::EigonPrimitive(b)) => a == b,
        (Val::ResourceVal(a), Val::ResourceVal(b)) => {
            // Compare resource contents for equality
            a.properties() == b.properties() && a.id() == b.id()
        }
        (Val::Con(c1, v1), Val::Con(c2, v2)) => c1 == c2 && ground_values_equal(v1, v2),
        (Val::Pair(a1, b1), Val::Pair(a2, b2)) => {
            ground_values_equal(a1, a2) && ground_values_equal(b1, b2)
        }
        (Val::Refl(a), Val::Refl(b)) => ground_values_equal(a, b),
        _ => false,
    }
}

/// Extract the payload value from a single-property wrapper resource.
/// `resource_value_to_val` wraps primitives in a one-property Resource
/// keyed on the type IRI; this reads that value back out. Multi-property
/// resources fall back to the first value.
fn resource_payload(
    r: &crate::ontology::resource::Resource,
) -> Option<&crate::ontology::resource::Value> {
    r.properties().values().next()
}

/// Decide a structural (kernel-hardcoded) constraint against a value,
/// three-valued. Pure — available in any evaluation mode, so
/// `NativeDecide` reduces in `Pure` context too. `Constraint::Institution`
/// is dispatched through the effect hooks instead and never reaches here.
fn decide_structural(constraint: &Constraint, val: &Val) -> Decision {
    let dec = |b: bool| {
        if b {
            Decision::Holds
        } else {
            Decision::Fails
        }
    };
    let as_int = |v: &Val| match v {
        Val::ResourceVal(r) => resource_payload(r).and_then(|x| x.as_integer()),
        _ => None,
    };
    let as_str = |v: &Val| match v {
        Val::ResourceVal(r) => resource_payload(r).and_then(|x| x.as_str().map(str::to_owned)),
        _ => None,
    };
    match constraint {
        Constraint::MinValue(min) => dec(as_int(val).is_some_and(|n| n >= *min)),
        Constraint::MaxValue(max) => dec(as_int(val).is_some_and(|n| n <= *max)),
        Constraint::MinLength(min) => dec(as_str(val).is_some_and(|s| s.len() as i64 >= *min)),
        Constraint::MaxLength(max) => dec(as_str(val).is_some_and(|s| s.len() as i64 <= *max)),
        Constraint::Pattern(pattern) => dec(as_str(val).is_some_and(|s| {
            let full = format!("^(?:{pattern})$");
            regex::Regex::new(&full).is_ok_and(|re| re.is_match(&s))
        })),
        Constraint::Format(fmt) => dec(as_str(val).is_some_and(|s| match fmt.as_str() {
            "date" => s.len() == 10 && s.chars().nth(4) == Some('-'),
            "uuid" => s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4,
            _ => true,
        })),
        // Dispatched through the hooks, not here.
        Constraint::Institution { .. } => Decision::Undecidable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;

    #[test]
    fn eval_set() -> Result<(), EvalError> {
        let v = eval(&Exp::Sort(1), &Rho::Nil)?;
        assert!(matches!(v, Val::Sort(1)));
        Ok(())
    }

    #[test]
    fn eval_unit() -> Result<(), EvalError> {
        let v = eval(&Exp::Unit, &Rho::Nil)?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn eval_one() -> Result<(), EvalError> {
        let v = eval(&Exp::One, &Rho::Nil)?;
        assert!(matches!(v, Val::One));
        Ok(())
    }

    #[test]
    fn eval_var() -> Result<(), EvalError> {
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        let v = eval(&Exp::Var("x".to_string()), &rho)?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn eval_pair() -> Result<(), EvalError> {
        let v = eval(
            &Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Sort(1))),
            &Rho::Nil,
        )?;
        assert!(matches!(v, Val::Pair(_, _)));
        Ok(())
    }

    #[test]
    fn eval_fst() -> Result<(), EvalError> {
        let v = eval(
            &Exp::Fst(Box::new(Exp::Pair(
                Box::new(Exp::Unit),
                Box::new(Exp::Sort(1)),
            ))),
            &Rho::Nil,
        )?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn eval_snd() -> Result<(), EvalError> {
        let v = eval(
            &Exp::Snd(Box::new(Exp::Pair(
                Box::new(Exp::Unit),
                Box::new(Exp::Sort(1)),
            ))),
            &Rho::Nil,
        )?;
        assert!(matches!(v, Val::Sort(1)));
        Ok(())
    }

    #[test]
    fn eval_lambda_app() -> Result<(), EvalError> {
        // (λx. x) () = ()
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let v = eval(&Exp::App(Box::new(lam), Box::new(Exp::Unit)), &Rho::Nil)?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn eval_constructor() -> Result<(), EvalError> {
        let v = eval(&Exp::Con("ok".to_string(), Box::new(Exp::Unit)), &Rho::Nil)?;
        assert!(matches!(v, Val::Con(ref c, _) if c == "ok"));
        Ok(())
    }

    #[test]
    fn eval_let() -> Result<(), EvalError> {
        // let x : 1 = (); x
        let d = crate::nbe::term::Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Unit),
        );
        let v = eval(&Exp::Dec(d, Box::new(Exp::Var("x".to_string()))), &Rho::Nil)?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn eval_neutral_var() -> Result<(), EvalError> {
        // An unbound variable in the environment produces a neutral
        let rho = Rho::Nil.extend(
            Patt::Var("x".to_string()),
            Val::Nt(Neut::Gen(0, "x".to_string())),
        );
        let v = eval(&Exp::Var("x".to_string()), &rho)?;
        assert!(matches!(v, Val::Nt(Neut::Gen(0, _))));
        Ok(())
    }

    #[test]
    fn eval_neutral_app() -> Result<(), EvalError> {
        // f x where f is neutral — produces neutral application
        let rho = Rho::Nil
            .extend(
                Patt::Var("f".to_string()),
                Val::Nt(Neut::Gen(0, "f".to_string())),
            )
            .extend(Patt::Var("x".to_string()), Val::Unit);
        let v = eval(
            &Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("x".to_string())),
            ),
            &rho,
        )?;
        assert!(matches!(v, Val::Nt(Neut::App(_, _))));
        Ok(())
    }

    #[test]
    fn eval_eigon_primitive() -> Result<(), EvalError> {
        let v = eval(&Exp::EigonPrimitive(PrimitiveType::String), &Rho::Nil)?;
        assert!(matches!(v, Val::EigonPrimitive(PrimitiveType::String)));
        Ok(())
    }

    // --- eval_traced tests (Phase 10b) ---

    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::program::component::ComponentRegistry;
    use crate::program::trace::Trace;

    /// Build a minimal IO evaluation context for traced tests.
    fn io_ctx() -> EvalCtx {
        let layer = std::sync::Arc::new(
            crate::layer::LayerBuilder::new("empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let engine = crate::institution::eval_hooks::InstitutionEngine::for_io(
            std::sync::Arc::clone(&layer),
            std::sync::Arc::new(ComponentRegistry::default()),
            None,
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            None,
            None,
            None,
        );
        EvalCtx::effectful(Some(layer), std::sync::Arc::new(engine))
    }

    #[test]
    fn eval_traced_let_produces_trace() -> Result<(), EvalError> {
        // let x : 1 = resource.prop; x
        // The inner PropAccess should produce a Trace::Project,
        // and the Let should produce a Trace::Let wrapping it.
        let ctx = io_ctx();

        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Alice".into()),
        );

        let rho = Rho::Nil.extend(Patt::Var("r".to_string()), Val::ResourceVal(Box::new(r)));

        // let x : 1 = r.name; x
        let prop_access = Exp::PropAccess(
            Box::new(Exp::Var("r".to_string())),
            Iri::parse("urn:eigenius:test:name").unwrap(),
        );
        let decl = crate::nbe::term::Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(prop_access),
        );
        let body = Exp::Var("x".to_string());
        let exp = Exp::Dec(decl, Box::new(body));

        let (val, trace) = eval_traced(&exp, &rho, &ctx)?;

        // Value should be the extracted property
        assert!(matches!(val, Val::ResourceVal(_)));

        // Trace should be Let with a Project in value_trace
        let trace = trace.expect("Let with PropAccess should produce a trace");
        match trace {
            Trace::Let {
                name,
                value_trace,
                body_trace,
            } => {
                assert_eq!(name, "x");
                assert!(
                    matches!(value_trace.as_deref(), Some(Trace::Project { .. })),
                    "value_trace should be a Project"
                );
                // body is just Var, no trace
                assert!(body_trace.is_none());
            }
            other => panic!("expected Trace::Let, got {:?}", other),
        }
        Ok(())
    }

    #[test]
    fn eval_traced_prop_access_produces_project() -> Result<(), EvalError> {
        let ctx = io_ctx();

        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:test:color").unwrap(),
            Value::String("blue".into()),
        );

        let rho = Rho::Nil.extend(Patt::Var("item".to_string()), Val::ResourceVal(Box::new(r)));

        let exp = Exp::PropAccess(
            Box::new(Exp::Var("item".to_string())),
            Iri::parse("urn:eigenius:test:color").unwrap(),
        );

        let (_val, trace) = eval_traced(&exp, &rho, &ctx)?;
        let trace = trace.expect("PropAccess should always produce a Project trace");
        match trace {
            Trace::Project {
                source_trace,
                property,
            } => {
                assert_eq!(property.as_str(), "urn:eigenius:test:color");
                // source is Var — no sub-trace
                assert!(source_trace.is_none());
            }
            other => panic!("expected Trace::Project, got {:?}", other),
        }
        Ok(())
    }

    #[test]
    fn eval_traced_component_dispatch_produces_component_trace() -> Result<(), EvalError> {
        // Use the built-in Identity component
        let ctx = io_ctx();

        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:test:val").unwrap(),
            Value::String("hello".into()),
        );

        let rho = Rho::Nil.extend(
            Patt::Var("inp".to_string()),
            Val::ResourceVal(Box::new(input)),
        );

        // Identity(inp)
        let exp = Exp::App(
            Box::new(Exp::Var(
                "urn:eigenius:program:components:Identity".to_string(),
            )),
            Box::new(Exp::Var("inp".to_string())),
        );

        let (val, trace) = eval_traced(&exp, &rho, &ctx)?;

        // Value should be the same resource
        assert!(matches!(val, Val::ResourceVal(_)));

        // Trace should be Component
        let trace = trace.expect("Component dispatch should produce a trace");
        match trace {
            Trace::Component(ct) => {
                assert_eq!(ct.component, "urn:eigenius:program:components:Identity");
                assert!(!ct.cached);
            }
            other => panic!("expected Trace::Component, got {:?}", other),
        }
        Ok(())
    }

    /// Shared setup for the F-5 trace-completeness tests: an IO ctx and
    /// an env binding `inp` to a resource, plus the Identity call exp.
    fn f5_setup() -> (EvalCtx, Rho, Exp) {
        let ctx = io_ctx();
        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:test:val").unwrap(),
            Value::String("hello".into()),
        );
        let rho = Rho::Nil.extend(
            Patt::Var("inp".to_string()),
            Val::ResourceVal(Box::new(input)),
        );
        let identity_call = Exp::App(
            Box::new(Exp::Var(
                "urn:eigenius:program:components:Identity".to_string(),
            )),
            Box::new(Exp::Var("inp".to_string())),
        );
        (ctx, rho, identity_call)
    }

    /// F-5 regression (NbE analysis §3.2): a component dispatch nested
    /// inside a structural expression (`Pair`) must appear in the trace
    /// tree. Pre-consolidation, `Pair` fell through the untraced
    /// catch-all and the dispatch was invisible in the tree.
    #[test]
    fn f5_component_inside_pair_appears_in_trace() -> Result<(), EvalError> {
        let (ctx, rho, identity_call) = f5_setup();
        let exp = Exp::Pair(Box::new(identity_call), Box::new(Exp::Unit));
        let (_, trace) = eval_traced(&exp, &rho, &ctx)?;
        match trace.expect("nested dispatch must be visible in the tree") {
            Trace::Component(ct) => {
                assert_eq!(ct.component, "urn:eigenius:program:components:Identity");
            }
            other => panic!("expected Trace::Component, got {other:?}"),
        }
        Ok(())
    }

    /// F-5 regression: two effectful children of one structural node
    /// combine into a `Trace::Seq` — neither is dropped.
    #[test]
    fn f5_components_in_both_pair_legs_produce_seq() -> Result<(), EvalError> {
        let (ctx, rho, identity_call) = f5_setup();
        let exp = Exp::Pair(Box::new(identity_call.clone()), Box::new(identity_call));
        let (_, trace) = eval_traced(&exp, &rho, &ctx)?;
        match trace.expect("both dispatches must be visible") {
            Trace::Seq(children) => {
                assert_eq!(children.len(), 2);
                assert!(children.iter().all(|c| matches!(c, Trace::Component(_))));
                // The metrics walker sees both executions.
                let metrics =
                    crate::program::trace::ProgramMetrics::from_trace(&Some(Trace::Seq(children)));
                assert_eq!(metrics.executed_steps, 2);
            }
            other => panic!("expected Trace::Seq of two Components, got {other:?}"),
        }
        Ok(())
    }

    /// F-5 regression: a component dispatch inside a `Match` arm shows
    /// up as a `Trace::Case` with the branch's dispatch nested (Match
    /// previously fell through the untraced catch-all).
    #[test]
    fn f5_component_inside_match_arm_appears_as_case() -> Result<(), EvalError> {
        use super::testutil::nat_decl;
        let (ctx, rho, identity_call) = f5_setup();
        let nat = nat_decl();
        let exp = Exp::Match {
            scrutinee: Box::new(Exp::InductiveCtor(nat.clone(), "zero".to_string(), vec![])),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "zero".to_string(),
                    bindings: vec![],
                    body: identity_call,
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "succ".to_string(),
                    bindings: vec![Patt::Var("n".to_string())],
                    body: Exp::Unit,
                },
            ],
        };
        let (_, trace) = eval_traced(&exp, &rho, &ctx)?;
        match trace.expect("dispatch in the taken arm must be visible") {
            Trace::Case {
                branch_taken,
                branch_trace,
                ..
            } => {
                assert_eq!(branch_taken, "zero");
                assert!(matches!(branch_trace.as_deref(), Some(Trace::Component(_))));
            }
            other => panic!("expected Trace::Case, got {other:?}"),
        }
        Ok(())
    }

    /// F-5 regression: a `Reduce` step whose BOTH curried applications
    /// carry traces keeps both (pre-consolidation: `t1.or(t2)` dropped
    /// one).
    #[test]
    fn f5_reduce_step_keeps_both_application_traces() -> Result<(), EvalError> {
        let (ctx, rho, identity_call) = f5_setup();
        // f = λacc. let _ = Identity(inp) in λx. Identity(inp)
        // First application (f acc) dispatches once; the resulting
        // lambda's application (· x) dispatches again.
        let f = Exp::Lam(
            Patt::Var("acc".to_string()),
            Box::new(Exp::Dec(
                crate::nbe::term::Decl::Def(
                    Patt::Var("_ignored".to_string()),
                    Box::new(Exp::One),
                    Box::new(identity_call.clone()),
                ),
                Box::new(Exp::Lam(
                    Patt::Var("x".to_string()),
                    Box::new(identity_call),
                )),
            )),
        );
        let rho = rho.extend(Patt::Var("coll".to_string()), Val::List(vec![Val::Unit]));
        let exp = Exp::Reduce(
            Box::new(f),
            Box::new(Exp::Unit),
            Box::new(Exp::Var("coll".to_string())),
        );
        let (_, trace) = eval_traced(&exp, &rho, &ctx)?;
        match trace.expect("reduce step dispatches must be visible") {
            Trace::Reduce { step_traces } => {
                assert_eq!(step_traces.len(), 1);
                let step = step_traces[0].as_ref().expect("step must carry traces");
                let metrics =
                    crate::program::trace::ProgramMetrics::from_trace(&Some(step.clone()));
                assert_eq!(
                    metrics.executed_steps, 2,
                    "both curried applications' dispatches must be kept, got {step:?}"
                );
            }
            other => panic!("expected Trace::Reduce, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn prop_access_missing_property_returns_unit() -> Result<(), EvalError> {
        // Phase 10c: PropAccess on a missing property should return Val::Unit
        // instead of panicking.
        let ctx = io_ctx();
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:test:exists").unwrap(),
            Value::String("yes".into()),
        );
        let rho = Rho::Nil.extend(Patt::Var("r".to_string()), Val::ResourceVal(Box::new(r)));
        let exp = Exp::PropAccess(
            Box::new(Exp::Var("r".to_string())),
            Iri::parse("urn:eigenius:test:missing").unwrap(),
        );
        let (val, _trace) = eval_traced(&exp, &rho, &ctx)?;
        assert!(
            matches!(val, Val::Unit),
            "missing property should return Val::Unit, got {:?}",
            val
        );
        Ok(())
    }

    #[test]
    fn prop_access_on_non_resource_returns_unit() -> Result<(), EvalError> {
        // Phase 10c: PropAccess where the target evaluates to a non-resource
        // Val should return Val::Unit instead of panicking.
        let ctx = io_ctx();
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Sort(1));
        let exp = Exp::PropAccess(
            Box::new(Exp::Var("x".to_string())),
            Iri::parse("urn:eigenius:test:prop").unwrap(),
        );
        let (val, _trace) = eval_traced(&exp, &rho, &ctx)?;
        assert!(
            matches!(val, Val::Unit),
            "PropAccess on non-resource should return Val::Unit, got {:?}",
            val
        );
        Ok(())
    }

    #[test]
    fn arrow_times_direct_evaluation() -> Result<(), EvalError> {
        // Phase 10c: Arrow/Times should produce identical results to Pi/Sig
        // with Patt::Unit, but without the re-recursion overhead.
        let arrow_val = eval(
            &Exp::Arrow(Box::new(Exp::One), Box::new(Exp::Sort(1))),
            &Rho::Nil,
        )?;
        let pi_val = eval(
            &Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::Sort(1))),
            &Rho::Nil,
        )?;
        // Both should be Val::Pi
        assert!(
            matches!(arrow_val, Val::Pi(_, _)),
            "Arrow should produce Val::Pi"
        );
        assert!(matches!(pi_val, Val::Pi(_, _)), "Pi should produce Val::Pi");

        let times_val = eval(
            &Exp::Times(Box::new(Exp::One), Box::new(Exp::Sort(1))),
            &Rho::Nil,
        )?;
        let sig_val = eval(
            &Exp::Sig(Patt::Unit, Box::new(Exp::One), Box::new(Exp::Sort(1))),
            &Rho::Nil,
        )?;
        assert!(
            matches!(times_val, Val::Sig(_, _)),
            "Times should produce Val::Sig"
        );
        assert!(
            matches!(sig_val, Val::Sig(_, _)),
            "Sig should produce Val::Sig"
        );
        Ok(())
    }

    #[test]
    fn eval_traced_pure_leaf_returns_none() -> Result<(), EvalError> {
        // Pure leaf forms (Var, Unit, etc.) should return None trace
        let ctx = io_ctx();
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        let (_val, trace) = eval_traced(&Exp::Var("x".to_string()), &rho, &ctx)?;
        assert!(trace.is_none(), "Var should produce no trace");

        let (_val, trace) = eval_traced(&Exp::Unit, &Rho::Nil, &ctx)?;
        assert!(trace.is_none(), "Unit should produce no trace");
        Ok(())
    }

    #[test]
    fn idj_stuck_returns_neutral() -> Result<(), EvalError> {
        // Phase 10c: J with a non-refl, non-neutral proof should return a
        // stuck neutral instead of panicking.
        let ctx = io_ctx();
        // IdJ(A, C, d, x, y, p) where p = Unit (not Refl or neutral)
        let args = Box::new([
            Exp::One,                                                  // A
            Exp::One,                                                  // C
            Exp::Lam(Patt::Var("z".to_string()), Box::new(Exp::Unit)), // d
            Exp::Unit,                                                 // x
            Exp::Unit,                                                 // y
            Exp::Unit, // p — not Refl, not neutral → stuck
        ]);
        let (val, _trace) = eval_traced(&Exp::IdJ(args), &Rho::Nil, &ctx)?;
        match val {
            Val::Nt(Neut::Gen(_, name)) => {
                assert_eq!(name, "__j_stuck", "should produce __j_stuck neutral");
            }
            other => panic!("expected stuck neutral, got {:?}", other),
        }
        Ok(())
    }

    #[test]
    fn prop_access_missing_observation_returns_unit() -> Result<(), EvalError> {
        // Phase 10c: PropAccess on a CoRecord where the observation name
        // doesn't exist should return Val::Unit instead of panicking.
        let ctx = io_ctx();
        let corecord = Val::CoRecord(vec![("head".to_string(), Exp::Unit)], Rho::Nil);
        let rho = Rho::Nil.extend(Patt::Var("s".to_string()), corecord);
        // Access observation "missing" which doesn't exist in the corecord
        let exp = Exp::PropAccess(
            Box::new(Exp::Var("s".to_string())),
            Iri::parse("urn:eigenius:test:missing").unwrap(),
        );
        let (val, _trace) = eval_traced(&exp, &rho, &ctx)?;
        assert!(
            matches!(val, Val::Unit),
            "missing observation should return Val::Unit, got {:?}",
            val
        );
        Ok(())
    }

    #[test]
    fn native_decide_constraint_check() -> Result<(), EvalError> {
        // Phase 10c: Verify check_native_constraint works correctly through
        // the resource_payload helper after the refactor.
        use crate::nbe::term::Constraint;

        let ctx = io_ctx();

        // Build a string wrapper resource (matching resource_value_to_val convention)
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:core:string").unwrap(),
            Value::String("hello".into()),
        );
        let rho = Rho::Nil.extend(Patt::Var("s".to_string()), Val::ResourceVal(Box::new(r)));

        // MinLength(3) should pass for "hello" (len=5)
        let exp = Exp::NativeDecide(
            Constraint::MinLength(3),
            Box::new(Exp::Var("s".to_string())),
        );
        let (val, _) = eval_traced(&exp, &rho, &ctx)?;
        assert!(
            matches!(val, Val::Refl(_)),
            "MinLength(3) should pass for 'hello', got {:?}",
            val
        );

        // MaxLength(3) should fail for "hello" (len=5)
        let exp = Exp::NativeDecide(
            Constraint::MaxLength(3),
            Box::new(Exp::Var("s".to_string())),
        );
        let (val, _) = eval_traced(&exp, &rho, &ctx)?;
        assert!(
            matches!(val, Val::Nt(_)),
            "MaxLength(3) should fail for 'hello', got {:?}",
            val
        );
        Ok(())
    }

    #[test]
    fn eval_traced_construct_produces_construct_trace() -> Result<(), EvalError> {
        let ctx = io_ctx();

        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:test:src").unwrap(),
            Value::String("data".into()),
        );
        let rho = Rho::Nil.extend(Patt::Var("s".to_string()), Val::ResourceVal(Box::new(r)));

        // Construct ex:Out { ex:val = s.src }
        let class_iri = Iri::parse("urn:eigenius:test:Out").unwrap();
        let prop_iri = Iri::parse("urn:eigenius:test:val").unwrap();
        let field_expr = Exp::PropAccess(
            Box::new(Exp::Var("s".to_string())),
            Iri::parse("urn:eigenius:test:src").unwrap(),
        );
        let exp = Exp::Construct(class_iri, vec![(prop_iri.clone(), Box::new(field_expr))]);

        let (val, trace) = eval_traced(&exp, &rho, &ctx)?;

        // Value should be a ResourceVal
        assert!(matches!(val, Val::ResourceVal(_)));

        // Trace should be Construct with a Project sub-trace
        let trace = trace.expect("Construct with PropAccess field should produce a trace");
        match trace {
            Trace::Construct { field_traces } => {
                assert_eq!(field_traces.len(), 1);
                let field_trace = field_traces.get(&prop_iri).unwrap();
                assert!(
                    matches!(field_trace, Some(Trace::Project { .. })),
                    "field should have a Project trace"
                );
            }
            other => panic!("expected Trace::Construct, got {:?}", other),
        }
        Ok(())
    }

    // --- Sized types primitives (Phase 11b step 14) ---

    #[test]
    fn eval_size_sort() -> Result<(), EvalError> {
        let v = eval(&Exp::SizeSort, &Rho::Nil)?;
        assert!(matches!(v, Val::SizeSort));
        Ok(())
    }

    #[test]
    fn eval_size_inf() -> Result<(), EvalError> {
        let v = eval(&Exp::SizeInf, &Rho::Nil)?;
        assert!(matches!(v, Val::SizeInf));
        Ok(())
    }

    #[test]
    fn size_succ_of_inf_absorbs_to_inf() {
        // `ŝ(∞) = ∞` — MiniAgda's fixed-point absorption
        // (Abstract.hs:300). Prevents spurious inequality between
        // sized types that happen to mix `SizeSucc` and `SizeInf`.
        let exp = Exp::SizeSucc(Box::new(Exp::SizeInf));
        let v = eval(&exp, &Rho::Nil).expect("eval");
        assert!(
            matches!(v, Val::SizeInf),
            "SizeSucc(SizeInf) must collapse to SizeInf, got {v:?}"
        );
    }

    #[test]
    fn nested_size_succ_at_inf_still_absorbs() {
        // ŝ(ŝ(∞)) evaluates inner first, gets ∞, outer ŝ also
        // absorbs — final value is ∞.
        let exp = Exp::SizeSucc(Box::new(Exp::SizeSucc(Box::new(Exp::SizeInf))));
        let v = eval(&exp, &Rho::Nil).expect("eval");
        assert!(
            matches!(v, Val::SizeInf),
            "nested SizeSucc at SizeInf must collapse, got {v:?}"
        );
    }

    #[test]
    fn size_succ_of_variable_does_not_absorb() {
        // SizeSucc over a neutral size variable stays as SizeSucc —
        // absorption only triggers for the concrete ∞ case.
        let rho = Rho::Nil.extend(
            Patt::Var("i".to_string()),
            Val::Nt(Neut::Gen(0, "i".to_string())),
        );
        let exp = Exp::SizeSucc(Box::new(Exp::Var("i".to_string())));
        let v = eval(&exp, &rho).expect("eval");
        match v {
            Val::SizeSucc(inner) => {
                assert!(matches!(*inner, Val::Nt(Neut::Gen(_, _))));
            }
            other => panic!("expected SizeSucc(neutral), got {other:?}"),
        }
    }

    #[test]
    fn finite_size_primitives_round_trip_through_readback() -> Result<(), EvalError> {
        // For non-∞ sizes (neutral variables), readback round-trips
        // the successor chain losslessly.
        let rho = Rho::Nil.extend(
            Patt::Var("j".to_string()),
            Val::Nt(Neut::Gen(0, "j".to_string())),
        );
        let exp = Exp::SizeSucc(Box::new(Exp::SizeSucc(Box::new(Exp::Var("j".to_string())))));
        let v = eval(&exp, &rho)?;
        let readback = crate::nbe::readback::readback_val(0, &v);
        // The neutral variable reads back with its gen-level name,
        // so we can't just assert_eq against the input. Verify
        // structure instead: two SizeSucc wrappers around some Var.
        match &readback {
            Exp::SizeSucc(inner1) => match inner1.as_ref() {
                Exp::SizeSucc(inner2) => {
                    assert!(matches!(inner2.as_ref(), Exp::Var(_)));
                }
                other => panic!("expected nested SizeSucc, got {other:?}"),
            },
            other => panic!("expected outer SizeSucc, got {other:?}"),
        }
        Ok(())
    }
}
