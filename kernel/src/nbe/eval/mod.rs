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

mod dispatch;
mod iota;
mod mapreduce;
mod marshal;
#[cfg(test)]
mod testutil;

pub(crate) use dispatch::deterministic_run_output_iri;
use dispatch::{decide_constraint, dispatch_component, try_d14_institution_invoke};
use iota::iota_reduce;
use mapreduce::{eval_map, eval_reduce};
pub use marshal::{resource_value_to_val, val_to_resource_value};

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

use crate::institution::registry::InstitutionIndex;
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Clos, Neut, Val};
use crate::observability::{field, operation};
use crate::ontology::iri::Iri;
use crate::program::component::ComponentRegistry;
use crate::program::trace::{ComponentTrace, Trace, TraceStore};
use crate::task::TaskContext;
use std::sync::{Arc, Mutex};

/// Evaluation context controlling what effects are available.
#[derive(Clone)]
pub enum EvalCtx {
    /// Standard NbE: normalize terms, check types. No side effects.
    Pure,
    /// Pure + read access to the layer chain.
    Read { layer: Arc<Layer> },
    /// Read + IO component dispatch + trace production.
    IO {
        layer: Arc<Layer>,
        registry: Arc<ComponentRegistry>,
        trace_store: Option<Arc<dyn TraceStore>>,
        /// ComponentTraces produced during this evaluation (for trace layer commits).
        dispatched_traces: Arc<Mutex<Vec<ComponentTrace>>>,
        /// Top-level resources produced during this evaluation that
        /// must be committed to the chain at the run-boundary
        /// (D14 §9.3 step 4 — comorphism reify outputs). Each entry
        /// has a deterministic content-hash IRI assigned at the
        /// reify boundary; the run-boundary commits them as part of
        /// the program-run layer.
        produced_resources: Arc<Mutex<Vec<crate::ontology::resource::Resource>>>,
        /// Optional task context. When present, IO dispatches route
        /// through per-task positional trace keys (D21 §3.2) instead
        /// of the cross-task content-address cache. Synchronous
        /// `RunProgram` and the type-checker leave this `None`.
        task_context: Option<Arc<TaskContext>>,
        /// D14 institution index — derived view of the layer chain
        /// keyed by institution / format / query / comorphism IRIs.
        /// When `Some` and a runtime (below) is also `Some`,
        /// `Exp::InstitutionInvoke` dispatches via the D14 four-step
        /// pipeline (D14 §9.3).
        institution_index: Option<Arc<InstitutionIndex>>,
        /// D14 institution runtime — registry of `Institution` trait
        /// objects keyed by institution IRI.
        institution_runtime: Option<Arc<InstitutionRuntime>>,
    },
    /// Pure evaluation with access to the D14 institution index +
    /// runtime for check-time dispatch of `Constraint::Institution`
    /// predicates. No component registry, no trace store — this is
    /// what the type-checker uses when it wants institution
    /// resolution but not full IO. Comorphism dispatch (which applies
    /// a transformation Component) is unavailable here; only Decidable
    /// QueryClass dispatch and AutoOnLoad readers are wired.
    Check {
        layer: Option<Arc<Layer>>,
        institution_index: Option<Arc<InstitutionIndex>>,
        institution_runtime: Option<Arc<InstitutionRuntime>>,
    },
}

impl EvalCtx {
    /// A static Pure context for convenience.
    pub fn pure() -> Self {
        EvalCtx::Pure
    }

    /// Layer for this evaluation context, if any.
    pub fn layer(&self) -> Option<&Arc<Layer>> {
        match self {
            EvalCtx::Pure => None,
            EvalCtx::Read { layer } => Some(layer),
            EvalCtx::IO { layer, .. } => Some(layer),
            EvalCtx::Check { layer, .. } => layer.as_ref(),
        }
    }

    /// D14 institution index for this evaluation context, if any.
    pub fn institution_index(&self) -> Option<&Arc<InstitutionIndex>> {
        match self {
            EvalCtx::IO {
                institution_index, ..
            } => institution_index.as_ref(),
            EvalCtx::Check {
                institution_index, ..
            } => institution_index.as_ref(),
            EvalCtx::Pure | EvalCtx::Read { .. } => None,
        }
    }

    /// D14 institution runtime for this evaluation context, if any.
    pub fn institution_runtime(&self) -> Option<&Arc<InstitutionRuntime>> {
        match self {
            EvalCtx::IO {
                institution_runtime,
                ..
            } => institution_runtime.as_ref(),
            EvalCtx::Check {
                institution_runtime,
                ..
            } => institution_runtime.as_ref(),
            EvalCtx::Pure | EvalCtx::Read { .. } => None,
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
    // Shorthand for recursive calls
    let ev = |e: &Exp| -> Result<Val, EvalError> { eval_ctx(e, rho, ctx) };

    match exp {
        Exp::Sort(n) => Ok(Val::Sort(*n)),
        Exp::One => Ok(Val::One),
        Exp::Unit => Ok(Val::Unit),

        // eigenius#71 / D49 — literals normalise to themselves; no
        // reduction, no neutral substructure.
        Exp::LitString(s) => Ok(Val::LitString(s.clone())),
        Exp::LitInt(n) => Ok(Val::LitInt(*n)),
        Exp::LitFloat(f) => Ok(Val::LitFloat(*f)),

        Exp::Dec(d, e) => {
            match ctx {
                EvalCtx::Pure => {
                    // Pure mode: lazy evaluation via UpDec (standard EigenTT)
                    eval_ctx(e, &Rho::UpDec(Box::new(rho.clone()), d.clone()), ctx)
                }
                _ => {
                    // IO/Read mode: eagerly evaluate the declaration value
                    // so that IO dispatch happens in the correct context
                    match d {
                        crate::nbe::term::Decl::Def(patt, _typ, body) => {
                            let val = eval_ctx(body, rho, ctx)?;
                            let rho2 = rho.clone().extend(patt.clone(), val);
                            eval_ctx(e, &rho2, ctx)
                        }
                        crate::nbe::term::Decl::Drec(patt, _typ, body) => {
                            // Recursive: evaluate in extended env
                            let rho_ext = Rho::UpDec(Box::new(rho.clone()), d.clone());
                            let val = eval_ctx(body, &rho_ext, ctx)?;
                            let rho2 = rho.clone().extend(patt.clone(), val);
                            eval_ctx(e, &rho2, ctx)
                        }
                    }
                }
            }
        }

        Exp::Lam(p, e) => Ok(Val::Lam(Clos::new(p.clone(), *e.clone(), rho.clone()))),

        Exp::Pi(p, a, b) => Ok(Val::Pi(
            Box::new(ev(a)?),
            Clos::new(p.clone(), *b.clone(), rho.clone()),
        )),

        Exp::Sig(p, a, b) => Ok(Val::Sig(
            Box::new(ev(a)?),
            Clos::new(p.clone(), *b.clone(), rho.clone()),
        )),

        Exp::Fst(e) => ev(e)?.vfst(),
        Exp::Snd(e) => ev(e)?.vsnd(),

        Exp::App(e1, e2) => {
            // In IO mode, intercept component-call-shaped applications:
            // when the LHS is a Var resolving to a registered Component,
            // dispatch through the component runtime. Institution
            // capabilities don't appear here under D14 — programs reach
            // institutions only via `Exp::InstitutionInvoke` (comorphisms)
            // and `Exp::NativeDecide(Constraint::Institution{..}, _)`
            // (Decidable QueryClasses). The ESL compiler emits those
            // AST nodes via the InstitutionIndex classifier (D2 v2 §3.8).
            if let EvalCtx::IO { registry, .. } = ctx {
                if let Exp::Var(name) = e1.as_ref() {
                    if registry.get(name).is_some() {
                        let arg_val = ev(e2)?;
                        let (input_val, comp_arg) = match &arg_val {
                            Val::Pair(input, comp_arg) => {
                                (input.as_ref().clone(), Some(comp_arg.as_ref()))
                            }
                            other => (other.clone(), None),
                        };
                        return dispatch_component(name, &input_val, comp_arg, ctx);
                    }
                }
            }
            ev(e1)?.app_ctx(ev(e2)?, ctx)
        }

        // Type annotations are runtime-erased: `⟦(e : T)⟧ = ⟦e⟧`. (The
        // annotation only matters to `check_infer` — see check.rs.)
        Exp::Ann(e, _t) => ev(e),

        Exp::Var(x) => match rho.get(x) {
            Ok(val) => Ok(val),
            Err(e) => match ctx {
                EvalCtx::Pure => Err(EvalError::UnboundVariable(e)),
                _ => {
                    // IO/Read mode: unbound variables may be component IRIs
                    // that will be intercepted at the App level.
                    Ok(Val::Nt(Neut::Gen(usize::MAX, x.clone())))
                }
            },
        },

        Exp::Pair(e1, e2) => Ok(Val::Pair(Box::new(ev(e1)?), Box::new(ev(e2)?))),

        Exp::Con(c, e) => Ok(Val::Con(c.clone(), Box::new(ev(e)?))),

        Exp::Data(summands) => Ok(Val::Data(
            summands
                .iter()
                .map(|s| (s.name.clone(), s.typ.clone()))
                .collect(),
            rho.clone(),
        )),

        Exp::Case(branches) => Ok(Val::Fun(
            branches
                .iter()
                .map(|b| (b.name.clone(), b.body.clone()))
                .collect(),
            rho.clone(),
        )),

        // Sugar: A → B = Π _ : A. B  (direct construction, Phase 10c)
        Exp::Arrow(a, b) => Ok(Val::Pi(
            Box::new(ev(a)?),
            Clos::new(Patt::Unit, *b.clone(), rho.clone()),
        )),
        // Sugar: A × B = Σ _ : A. B  (direct construction, Phase 10c)
        Exp::Times(a, b) => Ok(Val::Sig(
            Box::new(ev(a)?),
            Clos::new(Patt::Unit, *b.clone(), rho.clone()),
        )),

        // Identity type
        Exp::Id(a, x, y) => Ok(Val::Id(
            Box::new(ev(a)?),
            Box::new(ev(x)?),
            Box::new(ev(y)?),
        )),
        Exp::Refl(a) => Ok(Val::Refl(Box::new(ev(a)?))),
        Exp::IdJ(args) => {
            let [_a, _c, d, _x, _y, p] = args.as_ref();
            let p_val = ev(p)?;
            match p_val {
                Val::Refl(a_val) => {
                    let d_val = ev(d)?;
                    d_val.app_ctx(*a_val, ctx)
                }
                Val::Nt(n) => {
                    // Blocked — all args become neutral
                    Ok(Val::Nt(Neut::App(Box::new(n), Box::new(Val::Unit))))
                }
                _ => {
                    // Stuck — proof argument is neither Refl nor neutral.
                    // Return a stuck neutral rather than panicking (Phase 10c).
                    Ok(Val::Nt(Neut::Gen(usize::MAX, "__j_stuck".to_string())))
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
        // runs as part of [`try_d14_institution_invoke`].
        //
        // When the evaluator has no D14 backing attached (bare Pure
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
            let source_val = ev(source)?;
            if ctx.institution_index().is_none() || ctx.institution_runtime().is_none() {
                return Ok(Val::Nt(Neut::Gen(
                    usize::MAX,
                    format!("__institution_invoke_no_registry:{comorphism_iri}"),
                )));
            }
            match try_d14_institution_invoke(comorphism_iri, &source_val, target_iri.as_ref(), ctx)?
            {
                Some(translated) => Ok(translated),
                None => Err(EvalError::InvalidCaseTarget(format!(
                    "no Comorphism declaration found in the InstitutionIndex for `{comorphism_iri}`"
                ))),
            }
        }

        // Native constraint checking
        Exp::NativeDecide(constraint, val) => {
            let v = ev(val)?;
            match decide_constraint(constraint, &v, rho, ctx)? {
                crate::institution::DecResult::Holds => Ok(Val::Refl(Box::new(v))),
                crate::institution::DecResult::Fails => Ok(Val::Nt(Neut::Gen(
                    usize::MAX,
                    "__constraint_failed".to_string(),
                ))),
                crate::institution::DecResult::Undecidable => Ok(Val::Nt(Neut::Gen(
                    usize::MAX,
                    "__constraint_undecidable".to_string(),
                ))),
            }
        }

        // Decidable equality on ground types
        Exp::DecEq(_a, x, y) => {
            let x_val = ev(x)?;
            let y_val = ev(y)?;
            if ground_values_equal(&x_val, &y_val) {
                Ok(Val::Refl(Box::new(x_val)))
            } else {
                Ok(Val::Nt(Neut::Gen(usize::MAX, "__deceq_false".to_string())))
            }
        }

        // Template literal — evaluate type expressions for each reference
        Exp::Template(s, refs) => {
            let mut resolved = Vec::new();
            for (iri, typ) in refs {
                resolved.push((iri.clone(), ev(typ)?));
            }
            Ok(Val::TemplateVal(s.clone(), resolved))
        }

        // Eigenius extensions
        Exp::EigonClass(iri) => Ok(Val::EigonClass(iri.clone())),
        // Axiom references evaluate to a neutral spine head — the
        // existing `Neut::App` machinery then handles applications
        // (`stats:lt(a, b)` → `Val::Nt(Neut::App(Neut::App(Neut::EigonAxiom(lt), a), b))`).
        Exp::EigonAxiom(iri) => Ok(Val::Nt(crate::nbe::val::Neut::EigonAxiom(iri.clone()))),
        Exp::EigonPrimitive(p) => Ok(Val::EigonPrimitive(*p)),
        Exp::EigonResource(r) => Ok(Val::ResourceVal(r.clone())),

        Exp::PropAccess(e, prop) => {
            let v = ev(e)?;
            match v {
                Val::ResourceVal(r) => {
                    // Direct property access on a known resource
                    match r.get(prop) {
                        Some(val) => Ok(resource_value_to_val(val)),
                        None => {
                            tracing::warn!(
                                { field::OPERATION } = operation::NBE_EVAL,
                                { field::ERROR_KIND } = "property_missing",
                                { field::PROPERTY_IRI } = %prop,
                                "property not found on resource during eval; returning Unit"
                            );
                            Ok(Val::Unit)
                        }
                    }
                }
                // Codata observation: the "property" IRI's local name is
                // treated as the observation name (D11 §8). Evaluate the
                // matching field body in the corecord's captured env.
                Val::CoRecord(fields, corecord_rho) => {
                    let obs_name = prop.local_name();
                    for (name, body) in &fields {
                        if name == obs_name {
                            return eval_ctx(body, &corecord_rho, ctx);
                        }
                    }
                    tracing::warn!(
                        { field::OPERATION } = operation::NBE_EVAL,
                        { field::ERROR_KIND } = "observation_missing",
                        observation = %obs_name,
                        "observation not found in corecord during eval; returning Unit"
                    );
                    Ok(Val::Unit)
                }
                Val::Nt(n) => Ok(Val::Nt(Neut::PropAccess(Box::new(n), prop.clone()))),
                _other => {
                    tracing::warn!(
                        { field::OPERATION } = operation::NBE_EVAL,
                        { field::ERROR_KIND } = "property_access_non_resource",
                        "property access on non-resource value during eval; returning Unit"
                    );
                    Ok(Val::Unit)
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
            for (prop_iri, expr) in fields {
                let val = ev(expr)?;
                let rval = val_to_resource_value(&val);
                r.set(prop_iri.clone(), rval);
            }
            Ok(Val::ResourceVal(Box::new(r)))
        }

        // Codata (D11, Phase 9b-i)
        Exp::Codata(observations) => Ok(Val::Codata(
            observations
                .iter()
                .map(|o| (o.name.clone(), o.typ.clone()))
                .collect(),
            rho.clone(),
        )),

        Exp::CoRecord(fields) => Ok(Val::CoRecord(
            fields
                .iter()
                .map(|f| (f.name.clone(), f.body.clone()))
                .collect(),
            rho.clone(),
        )),

        Exp::Observe(e, name) => ev(e)?.vobserve_ctx(name, ctx),

        // Map/Reduce (Phase 11a)
        Exp::Map(f, coll) => {
            let f_val = ev(f)?;
            let coll_val = ev(coll)?;
            eval_map(f_val, coll_val, ctx)
        }
        Exp::Reduce(f, init, coll) => {
            let f_val = ev(f)?;
            let acc = ev(init)?;
            let coll_val = ev(coll)?;
            eval_reduce(f_val, acc, coll_val, ctx)
        }

        // Inductive types (Phase 11b, D19; D48 adds indices)
        // Step 1 lands the AST and value shells; Step 2 will add iota
        // reduction for the recursor. Pre-D48 callers always have
        // `indices: Vec::new()` (non-indexed default).
        Exp::Inductive(decl) => Ok(Val::InductiveType {
            decl: decl.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        }),
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
            let vals: Result<Vec<_>, _> = args.iter().map(&ev).collect();
            let mut vals = vals?;
            if decl.indices.is_empty() {
                Ok(Val::InductiveType {
                    decl: decl.clone(),
                    params: vals,
                    indices: Vec::new(),
                })
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
                Ok(Val::InductiveType {
                    decl: decl.clone(),
                    params: vals,
                    indices,
                })
            }
        }
        Exp::CodataType(decl, params) => {
            let params: Result<Vec<_>, _> = params.iter().map(&ev).collect();
            Ok(Val::CodataType {
                decl: decl.clone(),
                params: params?,
            })
        }
        Exp::InductiveCtor(decl, ctor_name, args) => {
            let args: Result<Vec<_>, _> = args.iter().map(&ev).collect();
            Ok(Val::InductiveVal {
                decl: decl.clone(),
                ctor_name: ctor_name.clone(),
                args: args?,
            })
        }
        Exp::InductiveRec {
            decl,
            motive,
            minors,
            major,
        } => {
            let motive_val = ev(motive)?;
            let minor_vals: Result<Vec<_>, _> = minors.iter().map(&ev).collect();
            let minor_vals = minor_vals?;
            let major_val = ev(major)?;
            match major_val {
                Val::Nt(n) => Ok(Val::Nt(Neut::NtRec {
                    decl: decl.clone(),
                    motive: Box::new(motive_val),
                    minors: minor_vals,
                    major: Box::new(n),
                })),
                Val::InductiveVal {
                    ctor_name, args, ..
                } => iota_reduce(decl, &motive_val, &minor_vals, &ctor_name, &args, ctx),
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
            let scrutinee_val = ev(scrutinee)?;
            match scrutinee_val {
                Val::InductiveVal {
                    ctor_name, args, ..
                } => match_dispatch(arms, &ctor_name, &args, rho, ctx),
                Val::Nt(n) => Ok(Val::Nt(Neut::NtMatch {
                    scrutinee: Box::new(n),
                    arms: arms.clone(),
                    env: rho.clone(),
                })),
                other => Err(EvalError::InvalidCaseTarget(format!(
                    "Match: expected inductive value, got {other:?}"
                ))),
            }
        }

        // Sized types (Phase 11b step 14, D19 §8).
        Exp::SizeSort => Ok(Val::SizeSort),
        // ∞ is a fixed point of successor: `ŝ(∞) = ∞`. Matches
        // MiniAgda's `sizeSuccE Infty = Infty` (Abstract.hs:300).
        // Without this absorption, `SizeSucc(SizeInf)` and `SizeInf`
        // would compare unequal, creating spurious type mismatches
        // whenever code mixes sized and unsized (`∞`-indexed) uses.
        Exp::SizeSucc(s) => match ev(s)? {
            Val::SizeInf => Ok(Val::SizeInf),
            other => Ok(Val::SizeSucc(Box::new(other))),
        },
        Exp::SizeInf => Ok(Val::SizeInf),

        Exp::SizedPi { patt, upper, body } => Ok(Val::SizedPi(
            Box::new(ev(upper)?),
            Clos::new(patt.clone(), *body.clone(), rho.clone()),
        )),
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
fn match_dispatch(
    arms: &[crate::nbe::term::MatchArm],
    ctor_name: &str,
    args: &[Val],
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
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
    eval_ctx(&arm.body, &env, ctx)
}

/// Evaluate an expression with tracing.
///
/// Mirrors `eval_ctx` but produces a trace tree alongside the value.
/// Pure leaf forms (Var, Set, Type, etc.) return `None` trace.
/// Used by the execution engine to build tree-structured ProgramTraces (D6b §2).
pub fn eval_traced(exp: &Exp, rho: &Rho, ctx: &EvalCtx) -> Result<(Val, Option<Trace>), EvalError> {
    match exp {
        // --- Dec → Trace::Let (IO/Read mode only) ---
        Exp::Dec(d, e) => {
            if matches!(ctx, EvalCtx::Pure) {
                return Ok((eval_ctx(exp, rho, ctx)?, None));
            }
            match d {
                crate::nbe::term::Decl::Def(patt, _typ, body) => {
                    let (val, val_trace) = eval_traced(body, rho, ctx)?;
                    let rho2 = rho.clone().extend(patt.clone(), val);
                    let (body_val, body_trace) = eval_traced(e, &rho2, ctx)?;
                    let name = match patt {
                        Patt::Var(n) => n.clone(),
                        _ => "_".to_string(),
                    };
                    let trace = if val_trace.is_some() || body_trace.is_some() {
                        Some(Trace::Let {
                            name,
                            value_trace: val_trace.map(Box::new),
                            body_trace: body_trace.map(Box::new),
                        })
                    } else {
                        None
                    };
                    Ok((body_val, trace))
                }
                crate::nbe::term::Decl::Drec(patt, _typ, body) => {
                    let rho_ext = Rho::UpDec(Box::new(rho.clone()), d.clone());
                    let (val, val_trace) = eval_traced(body, &rho_ext, ctx)?;
                    let rho2 = rho.clone().extend(patt.clone(), val);
                    let (body_val, body_trace) = eval_traced(e, &rho2, ctx)?;
                    let name = match patt {
                        Patt::Var(n) => n.clone(),
                        _ => "_".to_string(),
                    };
                    let trace = if val_trace.is_some() || body_trace.is_some() {
                        Some(Trace::Let {
                            name,
                            value_trace: val_trace.map(Box::new),
                            body_trace: body_trace.map(Box::new),
                        })
                    } else {
                        None
                    };
                    Ok((body_val, trace))
                }
            }
        }

        // --- App: component dispatch (with Trace::Component) or delegate ---
        Exp::App(e1, e2) => {
            if let EvalCtx::IO {
                registry,
                dispatched_traces,
                ..
            } = ctx
            {
                if let Exp::Var(name) = e1.as_ref() {
                    if registry.get(name).is_some() {
                        let arg_val = eval_ctx(e2, rho, ctx)?;
                        let (input_val, comp_arg) = match &arg_val {
                            Val::Pair(input, comp_arg) => {
                                (input.as_ref().clone(), Some(comp_arg.as_ref()))
                            }
                            other => (other.clone(), None),
                        };
                        let before = dispatched_traces.lock().unwrap().len();
                        let val = dispatch_component(name, &input_val, comp_arg, ctx)?;
                        let trace = {
                            let traces = dispatched_traces.lock().unwrap();
                            if traces.len() > before {
                                Some(Trace::Component(traces.last().unwrap().clone()))
                            } else {
                                None
                            }
                        };
                        return Ok((val, trace));
                    }
                }
            }
            let f_val = eval_ctx(e1, rho, ctx)?;
            let arg_val = eval_ctx(e2, rho, ctx)?;
            f_val.app_ctx_traced(arg_val, ctx)
        }

        // --- PropAccess → Trace::Project ---
        Exp::PropAccess(e, prop) => {
            let (v, source_trace) = eval_traced(e, rho, ctx)?;
            match v {
                Val::ResourceVal(r) => {
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
                    Ok((
                        result,
                        Some(Trace::Project {
                            source_trace: source_trace.map(Box::new),
                            property: prop.clone(),
                        }),
                    ))
                }
                Val::CoRecord(fields, corecord_rho) => {
                    let obs_name = prop.local_name();
                    for (name, body) in &fields {
                        if name == obs_name {
                            return eval_traced(body, &corecord_rho, ctx);
                        }
                    }
                    tracing::warn!(
                        { field::OPERATION } = operation::NBE_EVAL,
                        { field::ERROR_KIND } = "observation_missing",
                        observation = %obs_name,
                        "observation not found in corecord during eval; returning Unit"
                    );
                    Ok((Val::Unit, None))
                }
                Val::Nt(n) => Ok((
                    Val::Nt(Neut::PropAccess(Box::new(n), prop.clone())),
                    Some(Trace::Project {
                        source_trace: source_trace.map(Box::new),
                        property: prop.clone(),
                    }),
                )),
                _other => {
                    tracing::warn!(
                        { field::OPERATION } = operation::NBE_EVAL,
                        { field::ERROR_KIND } = "property_access_non_resource",
                        "property access on non-resource value during eval; returning Unit"
                    );
                    Ok((Val::Unit, None))
                }
            }
        }

        // --- Construct → Trace::Construct ---
        Exp::Construct(class_iri, fields) => {
            use crate::ontology::resource::{Resource, Value};
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse("urn:eigenius:core:is_a").unwrap(),
                Value::Array(vec![Value::String(class_iri.as_str().to_string())]),
            );
            let mut field_traces = std::collections::BTreeMap::new();
            for (prop_iri, expr) in fields {
                let (val, trace) = eval_traced(expr, rho, ctx)?;
                let rval = val_to_resource_value(&val);
                r.set(prop_iri.clone(), rval);
                field_traces.insert(prop_iri.clone(), trace);
            }
            let has_traces = field_traces.values().any(|t| t.is_some());
            let trace = if has_traces {
                Some(Trace::Construct { field_traces })
            } else {
                None
            };
            Ok((Val::ResourceVal(Box::new(r)), trace))
        }

        // --- Observe: delegate to vobserve_ctx_traced ---
        Exp::Observe(e, name) => {
            let v = eval_ctx(e, rho, ctx)?;
            v.vobserve_ctx_traced(name, ctx)
        }

        // --- Map: traced evaluation producing Trace::Map ---
        Exp::Map(f, coll) => {
            let f_val = eval_ctx(f, rho, ctx)?;
            let coll_val = eval_ctx(coll, rho, ctx)?;
            match coll_val {
                Val::List(items) => {
                    let mut mapped = Vec::with_capacity(items.len());
                    let mut element_traces = Vec::with_capacity(items.len());
                    for elem in items {
                        let (val, trace) = f_val.clone().app_ctx_traced(elem, ctx)?;
                        mapped.push(val);
                        element_traces.push(trace);
                    }
                    let has_traces = element_traces.iter().any(|t| t.is_some());
                    let trace = if has_traces {
                        Some(Trace::Map { element_traces })
                    } else {
                        None
                    };
                    Ok((Val::List(mapped), trace))
                }
                Val::Con(ref name, _) if name == "nil" || name == "cons" => {
                    match crate::nbe::val::cons_to_vec(&coll_val) {
                        Some(items) => {
                            let mut mapped = Vec::with_capacity(items.len());
                            let mut element_traces = Vec::with_capacity(items.len());
                            for elem in items {
                                let (val, trace) = f_val.clone().app_ctx_traced(elem, ctx)?;
                                mapped.push(val);
                                element_traces.push(trace);
                            }
                            let has_traces = element_traces.iter().any(|t| t.is_some());
                            let trace = if has_traces {
                                Some(Trace::Map { element_traces })
                            } else {
                                None
                            };
                            Ok((Val::List(mapped), trace))
                        }
                        None => Err(EvalError::InvalidCaseTarget(
                            "Map: malformed cons list".to_string(),
                        )),
                    }
                }
                Val::InductiveVal { ref decl, .. } if decl.name == "List" => {
                    match crate::nbe::val::inductive_list_to_vec(&coll_val) {
                        Some(items) => {
                            let mut mapped = Vec::with_capacity(items.len());
                            let mut element_traces = Vec::with_capacity(items.len());
                            for elem in items {
                                let (val, trace) = f_val.clone().app_ctx_traced(elem, ctx)?;
                                mapped.push(val);
                                element_traces.push(trace);
                            }
                            let has_traces = element_traces.iter().any(|t| t.is_some());
                            let trace = if has_traces {
                                Some(Trace::Map { element_traces })
                            } else {
                                None
                            };
                            Ok((Val::List(mapped), trace))
                        }
                        None => Err(EvalError::InvalidCaseTarget(
                            "Map: malformed inductive list".to_string(),
                        )),
                    }
                }
                Val::Nt(n) => Ok((Val::Nt(Neut::NtMap(Box::new(f_val), Box::new(n))), None)),
                other => Err(EvalError::InvalidCaseTarget(format!(
                    "Map: expected list, got {other:?}"
                ))),
            }
        }

        // --- Reduce: traced evaluation producing Trace::Reduce ---
        Exp::Reduce(f, init, coll) => {
            let f_val = eval_ctx(f, rho, ctx)?;
            let acc = eval_ctx(init, rho, ctx)?;
            let coll_val = eval_ctx(coll, rho, ctx)?;
            match coll_val {
                Val::List(items) => {
                    let mut result = acc;
                    let mut step_traces = Vec::with_capacity(items.len());
                    for elem in items {
                        let (step_fn, t1) = f_val.clone().app_ctx_traced(result, ctx)?;
                        let (next, t2) = step_fn.app_ctx_traced(elem, ctx)?;
                        result = next;
                        step_traces.push(t1.or(t2));
                    }
                    let has_traces = step_traces.iter().any(|t| t.is_some());
                    let trace = if has_traces {
                        Some(Trace::Reduce { step_traces })
                    } else {
                        None
                    };
                    Ok((result, trace))
                }
                Val::Con(ref name, _) if name == "nil" || name == "cons" => {
                    match crate::nbe::val::cons_to_vec(&coll_val) {
                        Some(items) => {
                            let mut result = acc;
                            let mut step_traces = Vec::with_capacity(items.len());
                            for elem in items {
                                let (step_fn, t1) = f_val.clone().app_ctx_traced(result, ctx)?;
                                let (next, t2) = step_fn.app_ctx_traced(elem, ctx)?;
                                result = next;
                                step_traces.push(t1.or(t2));
                            }
                            let has_traces = step_traces.iter().any(|t| t.is_some());
                            let trace = if has_traces {
                                Some(Trace::Reduce { step_traces })
                            } else {
                                None
                            };
                            Ok((result, trace))
                        }
                        None => Err(EvalError::InvalidCaseTarget(
                            "Reduce: malformed cons list".to_string(),
                        )),
                    }
                }
                Val::InductiveVal { ref decl, .. } if decl.name == "List" => {
                    match crate::nbe::val::inductive_list_to_vec(&coll_val) {
                        Some(items) => {
                            let mut result = acc;
                            let mut step_traces = Vec::with_capacity(items.len());
                            for elem in items {
                                let (step_fn, t1) = f_val.clone().app_ctx_traced(result, ctx)?;
                                let (next, t2) = step_fn.app_ctx_traced(elem, ctx)?;
                                result = next;
                                step_traces.push(t1.or(t2));
                            }
                            let has_traces = step_traces.iter().any(|t| t.is_some());
                            let trace = if has_traces {
                                Some(Trace::Reduce { step_traces })
                            } else {
                                None
                            };
                            Ok((result, trace))
                        }
                        None => Err(EvalError::InvalidCaseTarget(
                            "Reduce: malformed inductive list".to_string(),
                        )),
                    }
                }
                Val::Nt(n) => Ok((
                    Val::Nt(Neut::NtReduce(Box::new(f_val), Box::new(acc), Box::new(n))),
                    None,
                )),
                other => Err(EvalError::InvalidCaseTarget(format!(
                    "Reduce: expected list, got {other:?}"
                ))),
            }
        }

        // --- InstitutionInvoke: comorphism dispatch (D14 §9.3) ---
        //
        // The four-step pipeline (extract → m → reify) lives in
        // `try_d14_institution_invoke`; here we wrap the source
        // expression in `eval_traced` to nest its trace, drive the
        // pipeline through `eval_ctx`, and synthesise a
        // `Trace::Comorphism` node from the produced
        // `Val::ResourceVal`'s `@id` and class. Pure-mode passthrough
        // (no D14 backing attached) and downstream errors propagate
        // unchanged.
        Exp::InstitutionInvoke {
            comorphism_iri,
            source,
            target_iri,
        } => {
            let (source_val, source_trace) = eval_traced(source, rho, ctx)?;
            if ctx.institution_index().is_none() || ctx.institution_runtime().is_none() {
                return Ok((
                    Val::Nt(Neut::Gen(
                        usize::MAX,
                        format!("__institution_invoke_no_registry:{comorphism_iri}"),
                    )),
                    None,
                ));
            }
            let translated = match try_d14_institution_invoke(
                comorphism_iri,
                &source_val,
                target_iri.as_ref(),
                ctx,
            )? {
                Some(v) => v,
                None => {
                    return Err(EvalError::InvalidCaseTarget(format!(
                            "no Comorphism declaration found in the InstitutionIndex for `{comorphism_iri}`"
                        )));
                }
            };
            let (target_iri_str, target_class_str) = match &translated {
                Val::ResourceVal(r) => {
                    let id = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
                    let class = r
                        .is_a()
                        .first()
                        .map(|i| i.as_str().to_string())
                        .unwrap_or_default();
                    (id, class)
                }
                _ => (String::new(), String::new()),
            };
            let trace = Trace::Comorphism {
                comorphism_iri: comorphism_iri.as_str().to_string(),
                source_trace: source_trace.map(Box::new),
                target_iri: target_iri_str,
                target_class: target_class_str,
            };
            Ok((translated, Some(trace)))
        }

        // --- All other forms: structural, no trace ---
        _ => Ok((eval_ctx(exp, rho, ctx)?, None)),
    }
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
        EvalCtx::IO {
            layer: std::sync::Arc::new(
                crate::layer::LayerBuilder::new("empty", None)
                    .build(crate::layer::LayerStorage::in_memory()),
            ),
            registry: std::sync::Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            produced_resources: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            task_context: None,
            institution_index: None,
            institution_runtime: None,
        }
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
