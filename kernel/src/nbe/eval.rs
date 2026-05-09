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

//! Mini-TT evaluator: terms → values.
//!
//! Ported from `Main.hs` lines 198-217 in the Mini-TT reference.
//! Extended with capability modes (Pure/Read/IO) per D9.

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
        Exp::Set => Ok(Val::Set),
        Exp::Type(n) => Ok(Val::Type(*n)),
        Exp::One => Ok(Val::One),
        Exp::Unit => Ok(Val::Unit),

        Exp::Dec(d, e) => {
            match ctx {
                EvalCtx::Pure => {
                    // Pure mode: lazy evaluation via UpDec (standard Mini-TT)
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

        // Inductive types (Phase 11b, D19)
        // Step 1 lands the AST and value shells; Step 2 will add iota
        // reduction for the recursor.
        Exp::Inductive(decl) => Ok(Val::InductiveType {
            decl: decl.clone(),
            params: Vec::new(),
        }),
        Exp::InductiveType(decl, params) => {
            let params: Result<Vec<_>, _> = params.iter().map(&ev).collect();
            Ok(Val::InductiveType {
                decl: decl.clone(),
                params: params?,
            })
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

        // --- All other forms: structural, no trace ---
        _ => Ok((eval_ctx(exp, rho, ctx)?, None)),
    }
}

/// Evaluate Map(f, collection).
///
/// Applies `f` to each element of a finite list. Accepts both
/// `Val::List` (primary, from resource arrays) and cons-pair chains
/// (legacy, from algebraic construction). Returns `Val::List`.
fn eval_map(f: Val, coll: Val, ctx: &EvalCtx) -> Result<Val, EvalError> {
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
fn eval_reduce(f: Val, acc: Val, coll: Val, ctx: &EvalCtx) -> Result<Val, EvalError> {
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

/// Iota reduction for an inductive recursor (Phase 11b step 2, D19 §3.4).
///
/// Reduces `I.rec params motive m₁..mₖ (cⱼ args)` to
/// `mⱼ(args, ih₁, …, ihₘ)` where each `ihᵢ` is the recursor applied to a
/// recursive sub-argument of `cⱼ`. Recursive sub-arguments are identified
/// by walking the constructor's type telescope: any binder (after the
/// parameter prefix) whose type is a self-reference `Exp::InductiveType(I, _)`
/// contributes one IH, computed by recursing into `iota_reduce` for
/// constructor sub-values or producing a blocked `Neut::NtRec` for
/// neutrals.
///
/// Higher-order recursive arguments (e.g. `(Nat → I) → I`) are rejected
/// elsewhere by the positivity checker (Phase 11b step 4) — here they
/// would simply fail the recursive-arg-type check and produce an
/// arity-mismatch error.
fn iota_reduce(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    motive: &Val,
    minors: &[Val],
    ctor_name: &str,
    args: &[Val],
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    let ctor_idx = decl
        .ctors
        .iter()
        .position(|c| c.name == ctor_name)
        .ok_or_else(|| {
            EvalError::ConstructorNotFound(format!(
                "{}.{ctor_name} (no such constructor in inductive `{}`)",
                decl.name, decl.name
            ))
        })?;

    if minors.len() != decl.ctors.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "InductiveRec on `{}`: expected {} minors, got {}",
            decl.name,
            decl.ctors.len(),
            minors.len()
        )));
    }

    let arg_types = extract_ctor_arg_types(decl, &decl.ctors[ctor_idx].typ);
    if arg_types.len() != args.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "InductiveRec: constructor `{}.{ctor_name}` expects {} args, got {}",
            decl.name,
            arg_types.len(),
            args.len()
        )));
    }

    // Apply minor to each constructor argument (in original order).
    let mut result = minors[ctor_idx].clone();
    for arg in args {
        result = result.app_ctx(arg.clone(), ctx)?;
    }

    // Then apply an induction hypothesis for each recursive argument,
    // in the order the recursive arguments appear.
    for (arg, arg_typ) in args.iter().zip(arg_types.iter()) {
        if is_recursive_arg_type(decl, arg_typ) {
            let ih = build_recursor_ih(decl, motive, minors, arg, ctx)?;
            result = result.app_ctx(ih, ctx)?;
        }
    }

    Ok(result)
}

/// Walk the constructor's full type expression, skip the parameter
/// prefix, and return the remaining argument types in order.
///
/// The returned slice references the original `Exp` nodes — no
/// substitution is performed. Callers only inspect the syntactic head
/// (specifically, looking for `Exp::InductiveType` to detect recursive
/// arguments), so leaving free variable references intact is fine.
fn extract_ctor_arg_types<'a>(
    decl: &crate::nbe::term::InductiveDecl,
    ctor_typ: &'a Exp,
) -> Vec<&'a Exp> {
    // Sentinel for size-binder positions — these carry a size value
    // at runtime but aren't recursive occurrences, so iota-reduction
    // treats them the same as non-inductive value args (skip IH).
    // Use `SizeSort` itself as the stand-in domain type; only the
    // `is_recursive_arg_type` predicate inspects these entries and
    // `SizeSort` is never a recursive reference.
    static SIZE_SORT: Exp = Exp::SizeSort;
    let mut types = Vec::new();
    let mut current = ctor_typ;
    let mut params_to_skip = decl.params.len();
    loop {
        match current {
            Exp::Pi(_, dom, body) => {
                if params_to_skip > 0 {
                    params_to_skip -= 1;
                } else {
                    types.push(dom.as_ref());
                }
                current = body;
            }
            Exp::SizedPi { body, .. } => {
                // Size binders never appear in the param prefix.
                types.push(&SIZE_SORT);
                current = body;
            }
            _ => break,
        }
    }
    types
}

/// Whether a constructor argument type is a direct self-reference to
/// the inductive being eliminated.
///
/// Phase 11b is non-nested, strictly positive — recursive arguments
/// have type exactly `Exp::InductiveType(I, _)` for the same inductive.
/// Higher-order or nested forms are rejected at type-check time.
fn is_recursive_arg_type(decl: &crate::nbe::term::InductiveDecl, typ: &Exp) -> bool {
    matches!(typ, Exp::InductiveType(d, _) if d.name == decl.name)
}

/// Build the induction hypothesis for a recursive constructor argument.
///
/// Either recurses into `iota_reduce` (if the argument is itself a
/// constructor) or produces a blocked `Neut::NtRec` (if the argument
/// is neutral).
fn build_recursor_ih(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    motive: &Val,
    minors: &[Val],
    arg: &Val,
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    match arg {
        Val::InductiveVal {
            ctor_name, args, ..
        } => iota_reduce(decl, motive, minors, ctor_name, args, ctx),
        Val::Nt(n) => Ok(Val::Nt(Neut::NtRec {
            decl: decl.clone(),
            motive: Box::new(motive.clone()),
            minors: minors.to_vec(),
            major: Box::new(n.clone()),
        })),
        other => Err(EvalError::InvalidCaseTarget(format!(
            "InductiveRec: recursive argument is not an inductive value: {other:?}"
        ))),
    }
}

/// Compute a deterministic content-hash IRI for a resource produced
/// during program execution (D14 §9.3 step 4 — comorphism reify
/// outputs; the program-run final output).
///
/// Shape: `urn:eigenius:<namespace>:<origin-tail>:<hex>` where
/// `<hex>` is the first 16 hex chars of SHA-256 over the canonical
/// Eigon-CBOR of the resource with `@id` cleared. Two calls that
/// produce identical resource content collide on the same IRI —
/// that is the dedup we want.
pub(crate) fn deterministic_run_output_iri(
    namespace: &str,
    origin_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
) -> Iri {
    use sha2::{Digest, Sha256};
    let mut for_hashing = resource.clone();
    for_hashing.set_id(None);
    let cbor = crate::ontology::eigon_cbor::canonicalize(&for_hashing);
    let digest = Sha256::digest(&cbor);
    let hex = digest
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let tail = origin_iri.as_str().rsplit(':').next().unwrap_or("anon");
    Iri::parse(format!("urn:eigenius:{namespace}:{tail}:{hex}").as_str())
        .expect("deterministic run-output IRI is well-formed")
}

/// D14 four-step InstitutionInvoke pipeline.
///
/// Returns:
/// - `Ok(Some(translated))` if the comorphism IRI resolved through the
///   D14 index and the pipeline ran end-to-end.
/// - `Ok(None)` if the D14 index / runtime aren't attached to the
///   evaluation context, or the comorphism IRI isn't found in the
///   index — the caller falls back to legacy.
/// - `Err(_)` if the index *did* find the comorphism but a downstream
///   step failed (missing format, missing institution, transformation
///   error, marshalling error). Failure of a configured pipeline is
///   not a reason to fall back — the comorphism is structurally
///   broken and the caller should surface the error.
fn try_d14_institution_invoke(
    comorphism_iri: &Iri,
    source_val: &Val,
    target_iri: Option<&Iri>,
    ctx: &EvalCtx,
) -> Result<Option<Val>, EvalError> {
    let (Some(index), Some(runtime)) = (ctx.institution_index(), ctx.institution_runtime()) else {
        return Ok(None);
    };
    let Some(comorphism) = index.comorphism(comorphism_iri) else {
        return Ok(None);
    };

    // Step 1: source-side ExportFormat.
    let export = index
        .export_format(&comorphism.export_format)
        .ok_or_else(|| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: export_format `{}` not in InstitutionIndex",
                comorphism.export_format
            ))
        })?;
    let source_inst = runtime.get(&export.institution_ref).ok_or_else(|| {
        EvalError::InvalidCaseTarget(format!(
            "comorphism `{comorphism_iri}`: source institution `{}` not registered in runtime",
            export.institution_ref
        ))
    })?;

    // Marshal the source Val into a Resource for the boundary call —
    // M5 supports ResourceVal directly; primitives are wrapped in a
    // single-property resource (matching the legacy fallback).
    let source_resource = match val_to_resource_value(source_val) {
        crate::ontology::resource::Value::Embedded(r) => *r,
        other => {
            let mut r = crate::ontology::resource::Resource::new_embedded();
            r.set(
                Iri::parse("urn:eigenius:core:value").expect("well-known IRI"),
                other,
            );
            r
        }
    };

    let storage = crate::layer::LayerStorage::in_memory();
    let head = ctx.layer().cloned().unwrap_or_else(|| {
        Arc::new(
            crate::layer::LayerBuilder::new("__invoke_empty_layer__", None).build(storage.clone()),
        )
    });
    let exec_ctx = crate::context::ExecutionContext::new(
        Arc::clone(&head),
        "__invoke__",
        crate::context::ExecutionMode::ReadOnly,
        storage,
    );

    // The chain stores resource-typed properties as IRI references
    // (post `canonicalise_resource_refs`). Substrate-runtime decoders
    // expect each nested chain resource embedded — a bare IRI string
    // hits no decoder and surfaces as `MethodError(decode_X, ("urn:...",))`
    // at the boundary. Walk the source resource and dereference every
    // resource-typed IRI to its embedded form before handing off.
    // Same fix the FIBER path applies (`embed_typed_resource_param`)
    // and the AutoOnLoad path applies (D14 §9.1 dispatch).
    let source_resource =
        crate::institution::marshal::embed_typed_resource_refs_recursively(source_resource, &head)
            .map_err(|e| {
                EvalError::InvalidCaseTarget(format!(
                    "comorphism `{comorphism_iri}`: source-resource embedding failed before \
                     extract_typed via `{}`: {e}",
                    export.procedure
                ))
            })?;

    // Step 2: extract typed payload from source-side resource.
    let typed_source = source_inst
        .extract_typed(&export.procedure, &source_resource, &exec_ctx)
        .map_err(|e| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: extract_typed via `{}` failed: {e}",
                export.procedure
            ))
        })?;

    // Step 3: apply the transformation Component to the typed payload.
    // The Component must be in the kernel's ComponentRegistry, which
    // means the eval context must be IO mode. If it isn't, the four-
    // step pipeline can't complete here — surface the error rather
    // than silently falling back.
    if !matches!(ctx, EvalCtx::IO { .. }) {
        return Err(EvalError::ModeError(format!(
            "comorphism `{comorphism_iri}`: D14 InstitutionInvoke requires IO mode \
             (transformation Component application); found {ctx_kind}",
            ctx_kind = match ctx {
                EvalCtx::Pure => "Pure",
                EvalCtx::Read { .. } => "Read",
                EvalCtx::Check { .. } => "Check",
                EvalCtx::IO { .. } => unreachable!(),
            }
        )));
    }
    let transformed =
        dispatch_component(comorphism.transformation.as_str(), &typed_source, None, ctx)?;

    // Step 4: target-side ImportFormat reifies the typed result.
    let import = index
        .import_format(&comorphism.import_format)
        .ok_or_else(|| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: import_format `{}` not in InstitutionIndex",
                comorphism.import_format
            ))
        })?;
    let target_inst = runtime.get(&import.institution_ref).ok_or_else(|| {
        EvalError::InvalidCaseTarget(format!(
            "comorphism `{comorphism_iri}`: target institution `{}` not registered in runtime",
            import.institution_ref
        ))
    })?;
    let mut target_resource = target_inst
        .reify(&import.procedure, &transformed, &exec_ctx)
        .map_err(|e| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: reify via `{}` failed: {e}",
                import.procedure
            ))
        })?;

    // D14 §9.3 step 4: assign a chain-resident IRI to the produced
    // target-class resource. Caller-supplied `target_iri` (e.g. from
    // EigenQL `INTO`) overrides; otherwise the kernel mints a
    // deterministic content-hash IRI so identical reify outputs
    // dedupe naturally on commit.
    let assigned_iri = match target_iri {
        Some(iri) => iri.clone(),
        None => deterministic_run_output_iri("comorphism-output", comorphism_iri, &target_resource),
    };
    target_resource.set_id(Some(assigned_iri));

    // Step 5 (D14 §9.3): post-translation validation invariant. Run
    // any AutoOnLoad QueryClasses bound to the produced target
    // resource's class. A `Fails` here indicates the comorphism
    // produced a target-invalid result — a comorphism-implementation
    // bug, not a user error. Surface as a typed error rather than
    // committing the bad resource.
    let post_errors = crate::institution::dispatch::dispatch_auto_on_load_for_resource(
        &target_resource,
        index,
        runtime,
        &exec_ctx,
    )
    .flatten_to_errors();
    if !post_errors.is_empty() {
        let reasons = post_errors
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(EvalError::InvalidCaseTarget(format!(
            "comorphism `{comorphism_iri}`: post-translation validation rejected the \
             reified resource: {reasons}"
        )));
    }

    // Push the IRI'd resource into the run-boundary collector so the
    // server's RunProgram path commits it to the chain alongside
    // ProgramTrace + ComponentTrace observability resources.
    if let EvalCtx::IO {
        produced_resources, ..
    } = ctx
    {
        produced_resources
            .lock()
            .expect("produced_resources mutex poisoned")
            .push(target_resource.clone());
    }

    Ok(Some(Val::ResourceVal(Box::new(target_resource))))
}

/// Dispatch an IO component call.
///
/// Converts the Val argument to a Resource, calls the component via the
/// registry, and converts the result back to a Val.
fn dispatch_component(
    component_iri: &str,
    input_val: &Val,
    component_arg: Option<&Val>,
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    let (registry, layer, trace_store, dispatched_traces, task_context) = match ctx {
        EvalCtx::IO {
            registry,
            layer,
            trace_store,
            dispatched_traces,
            task_context,
            ..
        } => (
            registry,
            layer,
            trace_store,
            dispatched_traces,
            task_context,
        ),
        _ => {
            return Err(EvalError::ModeError(
                "dispatch_component called outside IO mode".into(),
            ))
        }
    };

    let component = match registry.get(component_iri) {
        Some(c) => c,
        None => {
            // Unknown component — return input unchanged (identity fallback)
            return Ok(input_val.clone());
        }
    };

    // Convert Val to Resource for the component interface
    let input_resource = val_to_resource(input_val);
    let mut arg_resource = component_arg.map(val_to_resource);

    // Ontology-driven schema generation:
    // Look up the component definition, find its argument_type class,
    // scan argument properties for Class-valued references that need
    // JSON Schema generation. Phase 18e.2: the function also embeds
    // the short-name table on the argument for the orchestrator's
    // CompleteJson handler. Return value is ignored — the kernel no
    // longer uses the table post-hoc.
    let _ = resolve_component_schemas(component_iri, &mut arg_resource, layer);

    // Cache routing is determinism-gated (D21 §3.3):
    //   - Deterministic components (!is_io): content-address memo —
    //     identical input yields identical output, so cross-task
    //     reuse is sound.
    //   - IO components: positional per-task keys only, via
    //     TaskContext. The content-address cache would silently
    //     collapse distinct observations into one (the Phase-9a bug
    //     D21 §1 motivates); without a TaskContext, IO is simply
    //     re-dispatched every time rather than mis-cached.
    if component.is_io() {
        // D21 §3.2 replay path: when a TaskContext is attached, look
        // up this step's trace by (task_id, step_seq) first. A hit
        // means we're re-running after a crash and this IO call has
        // already completed — return the cached output without
        // re-dispatching.
        //
        // `step_seq` is consumed (fetch_add) whether the lookup hits
        // or misses — it's the monotonic position in the task's IO
        // log.
        let replay_slot = task_context.as_ref().map(|tc| (tc.clone(), tc.next_step()));
        if let Some((tc, step)) = replay_slot.as_ref() {
            if let Ok(Some(bytes)) =
                tc.task_store
                    .get_trace_bytes(&tc.session_id, &tc.task_id, *step)
            {
                // `parse_resource_lenient` — IO component outputs are
                // often anonymous embedded Resources with no `@id`,
                // which the strict parser rejects.
                if let Ok(output) = crate::ontology::eigon_cbor::parse_resource_lenient(&bytes) {
                    return Ok(Val::ResourceVal(Box::new(output)));
                }
                // Corrupt trace bytes — fall through to re-dispatch.
            }
        }

        match component.execute(&input_resource, arg_resource.as_ref(), layer) {
            Ok(result) => {
                // Phase 18e.2: orchestrator-side `CompleteJson` handler now
                // applies the `ShortNameTable` translation before returning,
                // so the kernel sees a properly-IRI-keyed Eigon resource
                // here and no post-hoc translation is needed.
                let output = result.output.clone();

                let ct = ComponentTrace {
                    component: component_iri.to_string(),
                    // input_hash is retained for reflection / audit
                    // even though IO dispatch no longer routes
                    // through the content-address cache.
                    input_hash: crate::program::trace::compute_trace_key(
                        component_iri,
                        &input_resource,
                    ),
                    argument_hash: None,
                    output: output.clone(),
                    cached: false,
                    metrics: result.metrics,
                };

                // Persist the per-task trace via commit_step so the
                // output bytes and updated TaskRecord land atomically
                // (D21 §8 step atomicity). Without a TaskContext
                // there is no safe place to cache an IO output, so
                // we just record the trace for the layer commit.
                //
                // If this dispatch was `components:Checkpoint`, also
                // build a Checkpoint alongside the trace — the
                // commit_step method writes all three (trace, record,
                // checkpoint) atomically (D21 §4).
                if let Some((tc, step)) = replay_slot.as_ref() {
                    let output_bytes = crate::ontology::eigon_cbor::serialize_resource(&output);
                    let is_checkpoint =
                        component_iri == crate::program::component::CHECKPOINT_COMPONENT_IRI;
                    let checkpoint = if is_checkpoint {
                        let state_bytes =
                            crate::ontology::eigon_cbor::serialize_resource(&input_resource);
                        Some(crate::task::Checkpoint {
                            session_id: tc.session_id,
                            task_id: tc.task_id,
                            step_seq: *step,
                            state: state_bytes,
                            created_at: now_millis(),
                        })
                    } else {
                        None
                    };
                    if let Ok(Some(mut record)) =
                        tc.task_store.get_task(&tc.session_id, &tc.task_id)
                    {
                        record.step_seq = step + 1;
                        record.latest_trace_seq = *step;
                        if is_checkpoint {
                            record.last_checkpoint = Some(*step);
                        }
                        record.updated_at = now_millis();
                        if let Err(e) = tc.task_store.commit_step(
                            &record,
                            Some((*step, output_bytes)),
                            checkpoint.as_ref(),
                        ) {
                            tracing::warn!(
                                { field::OPERATION } = operation::TASK_CHECKPOINT,
                                { field::ERROR_KIND } = "commit_step_failed",
                                { field::TASK_ID } = ?tc.task_id,
                                { field::ERROR_MESSAGE } = %e,
                                "task commit_step failed"
                            );
                        }
                    }
                }

                // Record for trace layer commit
                if let Ok(mut traces) = dispatched_traces.lock() {
                    traces.push(ct);
                }
                Ok(Val::ResourceVal(Box::new(output)))
            }
            Err(e) => {
                tracing::warn!(
                    { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                    { field::ERROR_KIND } = "dispatch_failed",
                    { field::COMPONENT_IRI } = %component_iri,
                    { field::ERROR_MESSAGE } = %e,
                    "IO component dispatch failed; returning empty resource"
                );
                // Return empty resource instead of panicking
                Ok(Val::ResourceVal(Box::new(
                    crate::ontology::resource::Resource::new_embedded(),
                )))
            }
        }
    } else {
        // Deterministic component — content-address memo is sound
        // and reused cross-task (D21 §3.3). Identical input across
        // two tasks hits the same entry, amortizing the dispatch.
        let cache_key = crate::program::trace::compute_trace_key(component_iri, &input_resource);
        if let Some(store) = trace_store {
            if let Some(cached) = store.get_component_trace(&cache_key) {
                return Ok(Val::ResourceVal(Box::new(cached.output)));
            }
        }

        match component.execute(&input_resource, arg_resource.as_ref(), layer) {
            Ok(result) => {
                let output = result.output.clone();
                let ct = ComponentTrace {
                    component: component_iri.to_string(),
                    input_hash: cache_key,
                    argument_hash: None,
                    output: output.clone(),
                    cached: false,
                    metrics: result.metrics,
                };
                if let Some(store) = trace_store {
                    store.put_component_trace(cache_key, ct.clone());
                }
                if let Ok(mut traces) = dispatched_traces.lock() {
                    traces.push(ct);
                }
                Ok(Val::ResourceVal(Box::new(output)))
            }
            Err(e) => {
                tracing::warn!(
                    { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                    { field::ERROR_KIND } = "pure_dispatch_failed",
                    { field::COMPONENT_IRI } = %component_iri,
                    { field::ERROR_MESSAGE } = %e,
                    "pure component dispatch failed; returning empty resource"
                );
                Ok(Val::ResourceVal(Box::new(
                    crate::ontology::resource::Resource::new_embedded(),
                )))
            }
        }
    }
}

/// Current time in milliseconds since the Unix epoch. Falls back to 0
/// if the system clock is before the epoch (shouldn't happen in
/// practice).
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a Val to a Resource for component dispatch.
fn val_to_resource(val: &Val) -> crate::ontology::resource::Resource {
    match val {
        Val::ResourceVal(r) => r.as_ref().clone(),
        Val::Unit => crate::ontology::resource::Resource::new_embedded(),
        _ => {
            // Lossy conversion — not all Vals map to Resources.
            // Fire in debug builds so tests surface unexpected Val types
            // reaching component dispatch (Phase 10c, defence-in-depth layer 3).
            debug_assert!(
                false,
                "val_to_resource: lossy conversion of {:?} to empty resource",
                val
            );
            crate::ontology::resource::Resource::new_embedded()
        }
    }
}

/// Ontology-driven schema resolution for component arguments.
///
/// Looks up the component's `argument_type` class in the layer chain.
/// For each property on that class whose value in the actual argument
/// resolves to a Class IRI, generates a JSON Schema and packs it into
/// the argument. Returns the ShortNameTable and class IRI if schema was generated.
fn resolve_component_schemas(
    component_iri: &str,
    arg_resource: &mut Option<crate::ontology::resource::Resource>,
    layer: &crate::layer::Layer,
) -> Option<(crate::program::schema::ShortNameTable, Iri)> {
    let arg = arg_resource.as_mut()?;

    // Look up the component definition
    let comp_iri = Iri::parse(component_iri).ok()?;
    let comp_def = layer.resolve(&comp_iri)?;

    // Get the argument_type class
    let arg_type_prop = Iri::parse("urn:eigenius:program:component:argument_type").ok()?;
    let arg_type_str = comp_def.get(&arg_type_prop)?.as_str()?;
    let arg_type_iri = Iri::parse(arg_type_str).ok()?;
    let arg_type_def = layer.resolve(&arg_type_iri)?;

    // Collect all property IRIs from requires + recommends on the argument class
    let requires_iri = Iri::parse("urn:eigenius:core:requires").ok()?;
    let recommends_iri = Iri::parse("urn:eigenius:core:recommends").ok()?;
    let mut prop_iris = Vec::new();
    if let Some(req) = arg_type_def.get(&requires_iri) {
        prop_iris.extend(req.as_iri_array());
    }
    if let Some(rec) = arg_type_def.get(&recommends_iri) {
        prop_iris.extend(rec.as_iri_array());
    }

    // For each property, check if its value in the actual argument references a Class
    let class_types_iri = Iri::parse("urn:eigenius:core:class_types").ok()?;
    let class_iri = Iri::parse("urn:eigenius:core:Class").ok()?;
    let data_type_iri = Iri::parse("urn:eigenius:core:data_type").ok()?;

    for prop_iri in &prop_iris {
        // Look up the property definition
        let prop_def = match layer.resolve(prop_iri) {
            Some(d) => d,
            None => continue,
        };

        // Check if this property has class_types: [Class] (meaning it references a class)
        let is_class_ref = if let Some(ct) = prop_def.get(&class_types_iri) {
            ct.as_iri_array().contains(&class_iri)
        } else {
            false
        };

        // Check if the data_type is 'resource' (not 'template' or 'string')
        let is_resource = prop_def
            .get(&data_type_iri)
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == "urn:eigenius:core:resource");

        if is_class_ref && is_resource {
            // This property references a Class — check if the actual argument has a value
            if let Some(crate::ontology::resource::Value::String(class_iri_str)) = arg.get(prop_iri)
            {
                if let Ok(schema_class_iri) = Iri::parse(class_iri_str) {
                    // Generate JSON Schema from this class
                    match crate::program::schema::schema_for_class(&schema_class_iri, layer) {
                        Ok((json_schema, table)) => {
                            // Replace the class IRI with the actual JSON Schema
                            arg.set(
                                prop_iri.clone(),
                                crate::ontology::resource::Value::Json(json_schema),
                            );
                            // Phase 18e.2: embed the short-name table on the
                            // argument so the orchestrator-side handler can
                            // translate LLM short-name output back to
                            // IRI-keyed shape before returning. Replaces the
                            // kernel-side post-hoc translation that lived
                            // in dispatch_component until 18e.1.
                            let table_iri =
                                Iri::parse("urn:eigenius:program:components:short_name_table")
                                    .expect("static IRI is well-formed");
                            arg.set(
                                table_iri,
                                crate::ontology::resource::Value::Json(
                                    table.to_json(&schema_class_iri),
                                ),
                            );
                            return Some((table, schema_class_iri));
                        }
                        Err(e) => {
                            tracing::warn!(
                                { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                                { field::ERROR_KIND } = "schema_generation_failed",
                                { field::CLASS_IRI } = %class_iri_str,
                                { field::ERROR_MESSAGE } = %e,
                                "schema generation failed for class"
                            );
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extract the payload value from a single-property wrapper resource.
///
/// Convention: `resource_value_to_val` wraps primitive values in a
/// Resource with one property keyed on the type IRI (e.g.
/// `urn:eigenius:core:string`). This function extracts that value.
fn resource_payload(
    r: &crate::ontology::resource::Resource,
) -> Option<&crate::ontology::resource::Value> {
    let props = r.properties();
    if props.len() == 1 {
        props.values().next()
    } else {
        // Multi-property resources don't follow the wrapper convention;
        // fall back to first value for backwards compatibility.
        props.values().next()
    }
}

/// Decide a constraint against a value, three-valued.
///
/// Kernel-hardcoded scalar constraints (MinValue/MaxValue/…) fold
/// to `Holds` or `Fails` based on the structural check. Institution-
/// dispatched constraints (`Constraint::Institution { iri, args }`)
/// resolve the IRI as a Decidable QueryClass via the D14 institution
/// index; arguments are marshalled via [`val_to_resource_value`] onto
/// the synthetic input resource and the call dispatches through
/// `Institution::query` (D14 §9.2). Without an attached index/runtime
/// or a matching Decidable QueryClass, the constraint reduces to
/// `Undecidable` so downstream reducers leave it as a passthrough
/// neutral.
fn decide_constraint(
    constraint: &crate::nbe::term::Constraint,
    val: &Val,
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<crate::institution::DecResult, EvalError> {
    use crate::institution::DecResult;
    use crate::nbe::term::Constraint;
    let bool_to_dec = |b: bool| {
        if b {
            DecResult::Holds
        } else {
            DecResult::Fails
        }
    };
    match constraint {
        Constraint::MinValue(min) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n >= *min),
            _ => false,
        })),
        Constraint::MaxValue(max) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n <= *max),
            _ => false,
        })),
        Constraint::MinLength(min) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 >= *min),
            _ => false,
        })),
        Constraint::MaxLength(max) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 <= *max),
            _ => false,
        })),
        Constraint::Pattern(pattern) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| {
                    let full = format!("^(?:{pattern})$");
                    regex::Regex::new(&full).is_ok_and(|re| re.is_match(s))
                }),
            _ => false,
        })),
        Constraint::Format(fmt) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| match fmt.as_str() {
                    "date" => s.len() == 10 && s.chars().nth(4) == Some('-'),
                    "uuid" => s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4,
                    _ => true,
                }),
            _ => false,
        })),
        Constraint::Institution { iri, args } => {
            // D14 §9.2 dispatch: a Decidable QueryClass declares the
            // constraint IRI; args are marshalled onto a synthetic
            // input resource and the call goes through the institution
            // runtime. When no index/runtime is attached or no
            // Decidable QueryClass matches the IRI, the constraint
            // reduces to Undecidable so downstream reducers leave it
            // as a passthrough neutral.
            try_d14_decide(iri, args, rho, ctx).map(|opt| opt.unwrap_or(DecResult::Undecidable))
        }
    }
}

/// D14 §9.2 dispatch for an institution-bound Decidable constraint.
///
/// Returns:
/// - `Ok(Some(_))` if the D14 index has a Decidable QueryClass
///   declaring the constraint IRI and the dispatch ran end-to-end.
/// - `Ok(None)` if either the index or runtime is unattached, or the
///   constraint IRI doesn't resolve to a Decidable QueryClass. The
///   caller folds this to `DecResult::Undecidable` so reducers leave
///   the constraint as a passthrough neutral.
/// - `Err(_)` if the index *did* find the QueryClass but a downstream
///   step failed (missing institution, bad Verdict shape, etc.). A
///   configured-but-broken QueryClass is a structural error and not a
///   reason to silently fold to Undecidable.
fn try_d14_decide(
    iri: &Iri,
    args: &[Exp],
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<Option<crate::institution::DecResult>, EvalError> {
    use crate::institution::registry::DispatchRole;

    let (Some(index), Some(runtime)) = (ctx.institution_index(), ctx.institution_runtime()) else {
        return Ok(None);
    };
    let Some(query_class) = index.query_class(iri) else {
        return Ok(None);
    };
    if !query_class
        .dispatch_roles
        .contains(&DispatchRole::Decidable)
    {
        return Ok(None);
    }
    let Some(institution) = runtime.get(&query_class.institution_ref) else {
        return Err(EvalError::InvalidCaseTarget(format!(
            "QueryClass `{iri}` declares institution `{}` not registered in runtime",
            query_class.institution_ref
        )));
    };

    // Marshal args into a synthetic input resource via the shared
    // `institution::marshal::marshal_decidable_input` helper. Same
    // logic as the EigenQL-side `query::evaluate::try_dispatch_decidable`
    // — input class typed required properties get populated in
    // `requires` declaration order, IRI-shaped values targeting
    // `core:resource` properties dereference to embedded resources.
    let arg_values: Result<Vec<_>, EvalError> = args
        .iter()
        .map(|a| eval_ctx(a, rho, ctx).map(|v| val_to_resource_value(&v)))
        .collect();
    let arg_values = arg_values?;
    let layer = ctx.layer().ok_or_else(|| {
        EvalError::InvalidCaseTarget(format!(
            "QueryClass `{iri}` Decidable call: no layer attached to EvalCtx — cannot \
             resolve input class `{}` for typed-property marshaling",
            query_class.query_class
        ))
    })?;
    let input = crate::institution::marshal::marshal_decidable_input(
        &query_class.query_class,
        &arg_values,
        layer,
    )
    .map_err(|e| EvalError::InvalidCaseTarget(format!("QueryClass `{iri}` Decidable call: {e}")))?;

    let head = ctx.layer().cloned().unwrap_or_else(|| {
        Arc::new(
            crate::layer::LayerBuilder::new("__decide_empty_layer__", None)
                .build(crate::layer::LayerStorage::in_memory()),
        )
    });
    let storage = head.storage().clone();
    let exec_ctx = crate::context::ExecutionContext::new(
        head,
        "__decide__",
        crate::context::ExecutionMode::ReadOnly,
        storage,
    );

    // Component-implemented QueryClasses go through extract → component
    // → reify; institution-runtime ones land in `Institution::query`.
    // M6 wires the institution-runtime path; the Component path lands
    // alongside Component-driven AutoOnLoad in M7.
    let outcome = institution
        .query(&query_class.query_handler, &input, &exec_ctx)
        .map_err(|e| {
            EvalError::InvalidCaseTarget(format!(
                "QueryClass `{iri}` Decidable handler `{}` failed: {e}",
                query_class.query_handler
            ))
        })?;

    // Read off the Verdict from the result resource. The result must
    // be (or wrap) a `Verdict` inductive value with one of the three
    // constructor names — `Holds`, `Fails`, `Undecidable`. Decidable
    // dispatch is type-check-time and produces no chain-side
    // RuntimeInvocation commit, so the partial provenance (if any) is
    // intentionally dropped here.
    Ok(Some(parse_verdict(&outcome.output).map_err(|e| {
        EvalError::InvalidCaseTarget(format!(
            "QueryClass `{iri}` Decidable handler returned a non-Verdict result: {e}"
        ))
    })?))
}

/// Read a `Verdict` inductive value off a result resource.
///
/// The institution handler is expected to set `is_a` to one of the
/// three Verdict constructor IRIs:
///   urn:eigenius:institution:verdicts:holds
///   urn:eigenius:institution:verdicts:fails
///   urn:eigenius:institution:verdicts:undecidable
///
/// (or, equivalently, set a `ctor_name` property to one of "Holds" /
/// "Fails" / "Undecidable" against an is_a of `Verdict`). Both shapes
/// are accepted.
fn parse_verdict(
    result: &crate::ontology::resource::Resource,
) -> Result<crate::institution::DecResult, String> {
    use crate::institution::DecResult;
    use crate::ontology::well_known as wk;

    // First look for an explicit `ctor_name` property — produced when
    // a Component returns a Mini-TT Verdict value via the inductive
    // serialisation.
    if let Some(ctor) = result
        .get(&Iri::parse(wk::CTOR_NAME).expect("well-known IRI"))
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        return match ctor.as_str() {
            "Holds" => Ok(DecResult::Holds),
            "Fails" => Ok(DecResult::Fails),
            "Undecidable" => Ok(DecResult::Undecidable),
            other => Err(format!("unknown Verdict ctor_name `{other}`")),
        };
    }

    // Otherwise check `is_a` against the three Verdict constructor
    // IRIs the institution might tag the result with.
    for class_iri in result.is_a() {
        match class_iri.as_str() {
            "urn:eigenius:institution:verdicts:holds" => return Ok(DecResult::Holds),
            "urn:eigenius:institution:verdicts:fails" => return Ok(DecResult::Fails),
            "urn:eigenius:institution:verdicts:undecidable" => return Ok(DecResult::Undecidable),
            _ => {}
        }
    }

    Err(format!(
        "result resource is_a={:?} carries no Verdict marker",
        result.is_a()
    ))
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

/// Convert an Eigon resource Value to a Mini-TT Val.
///
/// Uses a heuristic IRI check: strings starting with "urn:" or "http"
/// are treated as class references (`Val::EigonClass`). This can
/// misclassify string property values that happen to look like IRIs.
/// The principled fix is type-directed conversion consulting the
/// property's declared `data_type` — deferred to Phase 11+ when the
/// type checker has full property-type awareness during evaluation.
fn resource_value_to_val(v: &crate::ontology::resource::Value) -> Val {
    use crate::ontology::resource::Value as RVal;
    match v {
        RVal::String(s) => {
            // Check if it looks like an IRI reference
            if let Ok(iri) = Iri::parse(s) {
                if s.starts_with("urn:") || s.starts_with("http") {
                    return Val::EigonClass(iri);
                }
            }
            Val::ResourceVal(Box::new({
                let mut r = crate::ontology::resource::Resource::new_embedded();
                let str_iri = Iri::parse("urn:eigenius:core:string").unwrap();
                r.set(str_iri, RVal::String(s.clone()));
                r
            }))
        }
        RVal::Integer(_) | RVal::Float(_) | RVal::Boolean(_) => {
            Val::ResourceVal(Box::new(crate::ontology::resource::Resource::new_embedded()))
        }
        RVal::Embedded(r) => Val::ResourceVal(r.clone()),
        RVal::Array(items) => Val::List(items.iter().map(resource_value_to_val).collect()),
        RVal::ResourceRef(iri) => Val::EigonClass(iri.clone()),
        RVal::Json(_) => Val::Unit,
    }
}

/// Convert a Mini-TT Val to an Eigon resource Value (for Construct).
fn val_to_resource_value(val: &Val) -> crate::ontology::resource::Value {
    use crate::ontology::resource::Value as RVal;
    match val {
        Val::ResourceVal(r) => {
            // If the resource has a single string value (e.g. CompleteText output),
            // extract it. Otherwise embed the full resource.
            let props: Vec<_> = r.properties().iter().collect();
            if props.len() == 1 {
                if let (_, RVal::String(s)) = props[0] {
                    return RVal::String(s.clone());
                }
            }
            RVal::Embedded(r.clone())
        }
        Val::Unit => RVal::String(String::new()),
        Val::EigonClass(iri) => RVal::String(iri.as_str().to_string()),
        Val::List(items) => RVal::Array(items.iter().map(val_to_resource_value).collect()),
        Val::Con(ref name, _) if name == "nil" || name == "cons" => {
            match crate::nbe::val::cons_to_vec(val) {
                Some(items) => RVal::Array(items.iter().map(val_to_resource_value).collect()),
                None => {
                    RVal::Embedded(Box::new(crate::ontology::resource::Resource::new_embedded()))
                }
            }
        }
        // Phase 11c: marshal inductive constructor values to embedded
        // resources so institution-registered decide can pattern-match
        // on them. The ctor name is stamped as is_a and each argument
        // recursively marshalled under a positional `ctor_arg_{i}`
        // property. This keeps the shape stable across decl changes —
        // institutions inspect by position, not by user-chosen names
        // (which the kernel doesn't record on ctor args).
        Val::InductiveVal {
            decl,
            ctor_name,
            args,
        } => {
            use crate::ontology::well_known as wk;
            let mut r = crate::ontology::resource::Resource::new_embedded();
            let qualified = format!("{}:{}", decl.name, ctor_name);
            r.set(
                crate::ontology::iri::Iri::parse(wk::IS_A).unwrap(),
                RVal::Array(vec![RVal::String(qualified)]),
            );
            for (i, arg) in args.iter().enumerate() {
                let key_iri =
                    crate::ontology::iri::Iri::parse(&format!("urn:eigenius:kernel:ctor_arg_{i}"))
                        .unwrap();
                r.set(key_iri, val_to_resource_value(arg));
            }
            RVal::Embedded(Box::new(r))
        }
        _ => RVal::Embedded(Box::new(crate::ontology::resource::Resource::new_embedded())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;

    #[test]
    fn eval_set() -> Result<(), EvalError> {
        let v = eval(&Exp::Set, &Rho::Nil)?;
        assert!(matches!(v, Val::Set));
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
            &Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)),
            &Rho::Nil,
        )?;
        assert!(matches!(v, Val::Pair(_, _)));
        Ok(())
    }

    #[test]
    fn eval_fst() -> Result<(), EvalError> {
        let v = eval(
            &Exp::Fst(Box::new(Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)))),
            &Rho::Nil,
        )?;
        assert!(matches!(v, Val::Unit));
        Ok(())
    }

    #[test]
    fn eval_snd() -> Result<(), EvalError> {
        let v = eval(
            &Exp::Snd(Box::new(Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)))),
            &Rho::Nil,
        )?;
        assert!(matches!(v, Val::Set));
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
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Set);
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
            &Exp::Arrow(Box::new(Exp::One), Box::new(Exp::Set)),
            &Rho::Nil,
        )?;
        let pi_val = eval(
            &Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::Set)),
            &Rho::Nil,
        )?;
        // Both should be Val::Pi
        assert!(
            matches!(arrow_val, Val::Pi(_, _)),
            "Arrow should produce Val::Pi"
        );
        assert!(matches!(pi_val, Val::Pi(_, _)), "Pi should produce Val::Pi");

        let times_val = eval(
            &Exp::Times(Box::new(Exp::One), Box::new(Exp::Set)),
            &Rho::Nil,
        )?;
        let sig_val = eval(
            &Exp::Sig(Patt::Unit, Box::new(Exp::One), Box::new(Exp::Set)),
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

    // --- Map/Reduce tests (Phase 11a) ---

    /// Helper: build a cons-pair list from values.
    fn cons_list(items: Vec<Val>) -> Val {
        let mut result = Val::Con("nil".into(), Box::new(Val::Unit));
        for item in items.into_iter().rev() {
            result = Val::Con(
                "cons".into(),
                Box::new(Val::Pair(Box::new(item), Box::new(result))),
            );
        }
        result
    }

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
        let list = cons_list(vec![Val::Unit, Val::Set]);
        let rho = Rho::Nil.extend(Patt::Var("lst".to_string()), list);
        let exp = Exp::Map(Box::new(id_lam()), Box::new(Exp::Var("lst".to_string())));
        let v = eval(&exp, &rho)?;
        match v {
            Val::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Val::Unit));
                assert!(matches!(items[1], Val::Set));
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

    #[test]
    fn resource_value_array_to_list_val() {
        use crate::ontology::resource::Value as RVal;
        let arr = RVal::Array(vec![RVal::Integer(1), RVal::Integer(2), RVal::Integer(3)]);
        let v = resource_value_to_val(&arr);
        match v {
            Val::List(items) => assert_eq!(items.len(), 3),
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn list_val_to_resource_value_array() {
        use crate::ontology::resource::Value as RVal;
        let list = Val::List(vec![Val::Unit, Val::Unit]);
        let rv = val_to_resource_value(&list);
        match rv {
            RVal::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn cons_list_to_resource_value_array() {
        use crate::ontology::resource::Value as RVal;
        let list = cons_list(vec![Val::Unit, Val::Unit]);
        let rv = val_to_resource_value(&list);
        match rv {
            RVal::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected Array, got {other:?}"),
        }
    }

    // --- Inductive recursor (iota reduction) tests (Phase 11b step 2) ---

    use crate::nbe::term::{InductiveCtorDecl, InductiveDecl};

    /// Stub self-reference for use inside an inductive's own constructor
    /// types. Carries the matching name with empty `ctors`; iota
    /// reduction only inspects names on inner refs, so this is enough
    /// to drive the algorithm without genuinely cyclic Arc allocation.
    fn ind_self_ref(name: &str) -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            name: name.to_string(),
            params: Vec::new(),
            sort: Exp::Set,
            ctors: Vec::new(),
        })
    }

    /// inductive Nat { zero : Nat, succ : Nat → Nat }
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

    fn ind_zero(decl: &Arc<InductiveDecl>) -> Val {
        Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "zero".to_string(),
            args: Vec::new(),
        }
    }

    fn ind_succ(decl: &Arc<InductiveDecl>, n: Val) -> Val {
        Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "succ".to_string(),
            args: vec![n],
        }
    }

    fn nat_n(decl: &Arc<InductiveDecl>, n: usize) -> Val {
        let mut v = ind_zero(decl);
        for _ in 0..n {
            v = ind_succ(decl, v);
        }
        v
    }

    #[test]
    fn iota_zero_arity_constructor() {
        // inductive Bool { True, False }
        // Bool.rec C true_minor false_minor True ↝ true_minor
        let s = ind_self_ref("Bool");
        let bool_ty = Exp::InductiveType(s, Vec::new());
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
                    typ: bool_ty,
                },
            ],
        });
        let true_minor = Val::Con("yes".to_string(), Box::new(Val::Unit));
        let false_minor = Val::Con("no".to_string(), Box::new(Val::Unit));
        let result = iota_reduce(
            &bool_decl,
            &Val::Set,
            &[true_minor, false_minor],
            "True",
            &[],
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");
        match result {
            Val::Con(c, _) if c == "yes" => {}
            other => panic!("expected Con(\"yes\", _), got {other:?}"),
        }
    }

    #[test]
    fn iota_recursive_constructor_double() {
        // Nat.rec zero (λ_n. λih. succ (succ ih)) (succ (succ zero))
        // ↝ succ (succ (succ (succ zero)))
        let nat = nat_decl();
        let zero_minor = nat_n(&nat, 0);

        // succ_minor body: succ (succ ih)
        let succ_body = Exp::InductiveCtor(
            nat.clone(),
            "succ".to_string(),
            vec![Exp::InductiveCtor(
                nat.clone(),
                "succ".to_string(),
                vec![Exp::Var("ih".to_string())],
            )],
        );
        // λ_n. λih. body
        let succ_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Var("ih".to_string()), Box::new(succ_body)),
            Rho::Nil,
        ));

        let result = iota_reduce(
            &nat,
            &Val::Set,
            &[zero_minor, succ_minor],
            "succ",
            &[nat_n(&nat, 1)],
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");

        let expected = nat_n(&nat, 4);
        let result_exp = crate::nbe::readback::readback_val(0, &result);
        let expected_exp = crate::nbe::readback::readback_val(0, &expected);
        assert_eq!(result_exp, expected_exp);
    }

    #[test]
    fn iota_list_length() {
        // List.rec zero (λa rest ih. succ ih) [_, _, _] = succ (succ (succ zero))
        let nat = nat_decl();
        let s = ind_self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list_decl = Arc::new(InductiveDecl {
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Set)],
            sort: Exp::Set,
            ctors: vec![
                InductiveCtorDecl {
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::Set),
                        Box::new(list_ty.clone()),
                    ),
                },
                InductiveCtorDecl {
                    name: "cons".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::Set),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(Exp::Var("A".to_string())),
                            Box::new(Exp::Pi(
                                Patt::Unit,
                                Box::new(list_ty.clone()),
                                Box::new(list_ty),
                            )),
                        )),
                    ),
                },
            ],
        });

        let elem = Val::Unit;
        let nil_val = Val::InductiveVal {
            decl: list_decl.clone(),
            ctor_name: "nil".to_string(),
            args: Vec::new(),
        };
        let cons = |a: Val, l: Val| Val::InductiveVal {
            decl: list_decl.clone(),
            ctor_name: "cons".to_string(),
            args: vec![a, l],
        };
        let three = cons(elem.clone(), cons(elem.clone(), cons(elem, nil_val)));

        let nil_minor = nat_n(&nat, 0);
        // λ_a. λ_rest. λih. succ ih
        let cons_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(
                Patt::Unit,
                Box::new(Exp::Lam(
                    Patt::Var("ih".to_string()),
                    Box::new(Exp::InductiveCtor(
                        nat.clone(),
                        "succ".to_string(),
                        vec![Exp::Var("ih".to_string())],
                    )),
                )),
            ),
            Rho::Nil,
        ));

        let three_args = match &three {
            Val::InductiveVal { args, .. } => args.clone(),
            _ => unreachable!(),
        };
        let result = iota_reduce(
            &list_decl,
            &Val::Set,
            &[nil_minor, cons_minor],
            "cons",
            &three_args,
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");

        let expected = nat_n(&nat, 3);
        let result_exp = crate::nbe::readback::readback_val(0, &result);
        let expected_exp = crate::nbe::readback::readback_val(0, &expected);
        assert_eq!(result_exp, expected_exp);
    }

    #[test]
    fn iota_neutral_major_blocks() {
        // Eval Exp::InductiveRec on a neutral major must produce Neut::NtRec.
        let nat = nat_decl();
        let neutral = Val::Nt(Neut::Gen(0, "n".to_string()));
        let zero_minor = ind_zero(&nat);
        // Dummy succ minor body: Unit
        let succ_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Unit, Box::new(Exp::Unit)),
            Rho::Nil,
        ));
        let rho = Rho::Nil
            .extend(Patt::Var("n".to_string()), neutral)
            .extend(Patt::Var("zero_min".to_string()), zero_minor)
            .extend(Patt::Var("succ_min".to_string()), succ_minor);
        let exp = Exp::InductiveRec {
            decl: nat.clone(),
            motive: Box::new(Exp::Set),
            minors: vec![
                Exp::Var("zero_min".to_string()),
                Exp::Var("succ_min".to_string()),
            ],
            major: Box::new(Exp::Var("n".to_string())),
        };
        let result = eval_ctx(&exp, &rho, &EvalCtx::Pure).expect("eval");
        match result {
            Val::Nt(Neut::NtRec { decl: d, .. }) => assert_eq!(d.name, "Nat"),
            other => panic!("expected NtRec, got {other:?}"),
        }
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
        let result = eval_reduce(f, Val::Set, lst, &EvalCtx::Pure).expect("eval_reduce");
        assert!(matches!(result, Val::Set));
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

    // --- Exp::InstitutionInvoke eval dispatch ---

    /// Pure-mode `Exp::InstitutionInvoke` produces a passthrough
    /// neutral when no institution context is attached. Verified
    /// in pure mode (no `EvalCtx::IO` / `Check`); the legacy
    /// `__institution_invoke_no_registry:<cm>` neutral name keeps
    /// the surface stable.
    #[test]
    fn institution_invoke_without_context_produces_passthrough_neutral() {
        let src_iri = Iri::parse("urn:eigenius:test:src").unwrap();
        let src_resource = crate::ontology::resource::Resource::new(src_iri);
        let source = Exp::EigonResource(Box::new(src_resource));

        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:marker_cm").unwrap(),
            source: Box::new(source),
            target_iri: None,
        };
        let v = eval(&exp, &Rho::Nil).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) => {
                assert!(name.starts_with("__institution_invoke_no_registry"));
            }
            other => panic!("expected passthrough neutral, got {other:?}"),
        }
    }

    // ─── D14 four-step InstitutionInvoke pipeline ──────────────────

    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::ontology::well_known as wk;
    use std::sync::Mutex;

    /// In-process Institution that records every dispatched call so a
    /// test can assert on the four-step pipeline routing — extract on
    /// the source side, reify on the target side. Both sides are the
    /// same institution here for setup brevity; production
    /// deployments cross institution boundaries.
    struct PipelineLogger {
        iri: Iri,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Institution for PipelineLogger {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }

        fn extract_typed(
            &self,
            procedure_iri: &Iri,
            resource: &crate::ontology::resource::Resource,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<Val, crate::institution::error::InstitutionError> {
            let id = resource
                .id()
                .map(|i| i.as_str().to_string())
                .unwrap_or_else(|| "<embedded>".to_string());
            self.log
                .lock()
                .unwrap()
                .push(format!("extract@{procedure_iri}({id})"));
            // Tag the resource with a provenance marker so reify can
            // confirm it received the extracted payload.
            let mut tagged = resource.clone();
            tagged.set(
                Iri::parse("urn:eigenius:test:d14_pipeline:extracted_via").expect("well-known IRI"),
                crate::ontology::resource::Value::String(procedure_iri.as_str().into()),
            );
            Ok(Val::ResourceVal(Box::new(tagged)))
        }

        fn reify(
            &self,
            procedure_iri: &Iri,
            value: &Val,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<crate::ontology::resource::Resource, crate::institution::error::InstitutionError>
        {
            self.log
                .lock()
                .unwrap()
                .push(format!("reify@{procedure_iri}"));
            let payload = match value {
                Val::ResourceVal(r) => r.as_ref().clone(),
                other => panic!("PipelineLogger.reify: expected ResourceVal, got {other:?}"),
            };
            // Tag the produced resource so the test can assert reify ran.
            let mut tagged = payload;
            tagged.set(
                Iri::parse("urn:eigenius:test:d14_pipeline:reified_via").expect("well-known IRI"),
                crate::ontology::resource::Value::String(procedure_iri.as_str().into()),
            );
            Ok(tagged)
        }
    }

    fn build_d14_pipeline_chain() -> Arc<crate::layer::Layer> {
        // Layer holds: Institution + ExportFormat + ImportFormat +
        // Comorphism declarations. Same institution_ref for source
        // and target — deliberately, to keep the runtime registry
        // setup minimal.
        let mut b = crate::layer::LayerBuilder::new("test", None);

        let mut institution = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:inst").unwrap(),
        );
        institution.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:institution:Institution".into(),
                ),
            ]),
        );
        institution.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:inst".into()),
        );
        institution.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            crate::ontology::resource::Value::String("Pipeline test institution".into()),
        );
        b.add_resource(institution).unwrap();

        let mut export = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:export").unwrap(),
        );
        export.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::EXPORT_FORMAT_CLASS.into()),
            ]),
        );
        export.set(
            Iri::parse(wk::FROM_CLASS).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:SourceClass".into(),
            ),
        );
        export.set(
            Iri::parse(wk::PAYLOAD_TYPE).unwrap(),
            crate::ontology::resource::Value::String(wk::FLOAT.into()),
        );
        export.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:inst".into()),
        );
        export.set(
            Iri::parse(wk::PROCEDURE).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:proc:extract".into(),
            ),
        );
        b.add_resource(export).unwrap();

        let mut import = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:import").unwrap(),
        );
        import.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::IMPORT_FORMAT_CLASS.into()),
            ]),
        );
        import.set(
            Iri::parse(wk::TO_CLASS).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:TargetClass".into(),
            ),
        );
        import.set(
            Iri::parse(wk::PAYLOAD_TYPE).unwrap(),
            crate::ontology::resource::Value::String(wk::FLOAT.into()),
        );
        import.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:inst".into()),
        );
        import.set(
            Iri::parse(wk::PROCEDURE).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:proc:reify".into(),
            ),
        );
        b.add_resource(import).unwrap();

        let mut comorphism = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:cm").unwrap(),
        );
        comorphism.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::COMORPHISM.into()),
            ]),
        );
        comorphism.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:export".into()),
        );
        comorphism.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            // No real Component — dispatch_component falls back to
            // identity for unknown component IRIs, which is what we
            // want for this structural test.
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:identity_transform".into(),
            ),
        );
        comorphism.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:import".into()),
        );
        comorphism.set(
            Iri::parse(wk::EXACT).unwrap(),
            crate::ontology::resource::Value::Boolean(false),
        );
        b.add_resource(comorphism).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    fn build_d14_pipeline_ctx(log: Arc<Mutex<Vec<String>>>) -> (EvalCtx, Arc<InstitutionIndex>) {
        let layer = build_d14_pipeline_chain();
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "index errors: {errors:?}");
        let idx = Arc::new(idx);

        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(PipelineLogger {
                iri: Iri::parse("urn:eigenius:test:d14_pipe:inst").unwrap(),
                log,
            }))
            .unwrap();

        let ctx = EvalCtx::IO {
            layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::clone(&idx)),
            institution_runtime: Some(Arc::new(runtime)),
        };
        (ctx, idx)
    }

    #[test]
    fn institution_invoke_runs_d14_four_step_pipeline_end_to_end() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (ctx, _idx) = build_d14_pipeline_ctx(Arc::clone(&log));

        let source = Exp::EigonResource(Box::new(crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:source_instance").unwrap(),
        )));
        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:d14_pipe:cm").unwrap(),
            source: Box::new(source),
            target_iri: None,
        };
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("D14 pipeline eval");
        let result = match v {
            Val::ResourceVal(r) => *r,
            other => panic!("expected ResourceVal from pipeline, got {other:?}"),
        };

        // Extract → identity-transform → reify all ran:
        let extracted_via = result
            .get(&Iri::parse("urn:eigenius:test:d14_pipeline:extracted_via").unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            extracted_via.as_deref(),
            Some("urn:eigenius:test:d14_pipe:proc:extract"),
            "extract_typed should have tagged the resource with the export procedure IRI"
        );
        let reified_via = result
            .get(&Iri::parse("urn:eigenius:test:d14_pipeline:reified_via").unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            reified_via.as_deref(),
            Some("urn:eigenius:test:d14_pipe:proc:reify"),
            "reify should have tagged the resource with the import procedure IRI"
        );

        // Order: extract first, reify last — confirms the four-step
        // pipeline shape (transformation in between is the identity
        // fallback for the unregistered Component IRI).
        let trail = log.lock().unwrap().clone();
        assert_eq!(
            trail,
            vec![
                "extract@urn:eigenius:test:d14_pipe:proc:extract(urn:eigenius:test:d14_pipe:source_instance)".to_string(),
                "reify@urn:eigenius:test:d14_pipe:proc:reify".to_string(),
            ]
        );
    }

    #[test]
    fn institution_invoke_d14_missing_format_surfaces_typed_error() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (ctx, idx) = build_d14_pipeline_ctx(Arc::clone(&log));

        // Sanity: the index has the comorphism we'll reference.
        assert!(idx
            .comorphism(&Iri::parse("urn:eigenius:test:d14_pipe:cm").unwrap())
            .is_some());

        // Build a *separate* comorphism that points at an
        // ExportFormat IRI not in the index. Must drop it into a new
        // layer above the existing chain so the InstitutionIndex can
        // still see the original declarations.
        let mut top =
            crate::layer::LayerBuilder::new("orphan_cm", Some(Arc::clone(ctx.layer().unwrap())));
        let mut orphan = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:orphan_cm").unwrap(),
        );
        orphan.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::COMORPHISM.into()),
            ]),
        );
        orphan.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:not_in_index".into(),
            ),
        );
        orphan.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:identity_transform".into(),
            ),
        );
        orphan.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:import".into()),
        );
        top.add_resource(orphan).unwrap();
        let new_layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        // Re-derive the index over the new chain so it picks up the
        // orphan comorphism.
        let (new_idx, _errs) = InstitutionIndex::from_layer(&new_layer);
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(PipelineLogger {
                iri: Iri::parse("urn:eigenius:test:d14_pipe:inst").unwrap(),
                log,
            }))
            .unwrap();
        let ctx = EvalCtx::IO {
            layer: new_layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::new(new_idx)),
            institution_runtime: Some(Arc::new(runtime)),
        };

        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:d14_pipe:orphan_cm").unwrap(),
            source: Box::new(Exp::EigonResource(Box::new(
                crate::ontology::resource::Resource::new(
                    Iri::parse("urn:eigenius:test:src").unwrap(),
                ),
            ))),
            target_iri: None,
        };
        let err = eval_ctx(&exp, &Rho::Nil, &ctx).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("export_format")
                && msg.contains("not_in_index")
                && msg.contains("not in InstitutionIndex"),
            "expected typed error about the missing ExportFormat; got: {msg}"
        );
    }

    // ─── D14 NativeDecide dispatch ─────────────────────────────────

    /// In-process Institution that answers Decidable QueryClasses by
    /// inspecting the `decide_args` array on the input resource and
    /// returning a Verdict resource. The verdict is configured at
    /// construction time so the test can assert on each branch.
    struct VerdictInstitution {
        iri: Iri,
        verdict_class: &'static str,
    }

    impl Institution for VerdictInstitution {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }
        fn extract_typed(
            &self,
            _procedure_iri: &Iri,
            _resource: &crate::ontology::resource::Resource,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<Val, crate::institution::error::InstitutionError> {
            unreachable!("VerdictInstitution exposes no ExportFormats")
        }
        fn reify(
            &self,
            _procedure_iri: &Iri,
            _value: &Val,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<crate::ontology::resource::Resource, crate::institution::error::InstitutionError>
        {
            unreachable!("VerdictInstitution exposes no ImportFormats")
        }
        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &crate::ontology::resource::Resource,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<
            crate::institution::runtime::QueryOutcome,
            crate::institution::error::InstitutionError,
        > {
            // Confirm the kernel stamped `is_a` to the input class IRI
            // (Phase 19d.7: positional args ride on typed required
            // properties, not on a `decide_args` array; the
            // structural marker we can rely on regardless of arity is
            // the auto-stamped is_a).
            let _ = input
                .get(&Iri::parse(crate::ontology::well_known::IS_A).unwrap())
                .expect("kernel must stamp is_a onto the synthetic input resource");
            let mut verdict = crate::ontology::resource::Resource::new_embedded();
            verdict.set(
                Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
                crate::ontology::resource::Value::Array(vec![
                    crate::ontology::resource::Value::String(self.verdict_class.into()),
                ]),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(
                verdict,
            ))
        }
    }

    fn build_d14_decide_ctx(verdict_class: &'static str, arg_count: usize) -> EvalCtx {
        use crate::ontology::well_known as wk;
        let mut b = crate::layer::LayerBuilder::new("test", None);

        let inst_iri = "urn:eigenius:test:d14_decide:inst";
        let constraint_iri = "urn:eigenius:test:d14_decide:has_property";
        let input_class = "urn:eigenius:test:d14_decide:Subject";

        // Phase 19d.7: the input class must declare typed required
        // properties for the kernel's typed-property marshaling to
        // populate. Each arg slot is its own Property resource named
        // `arg_N`, listed in `requires` in declaration order.
        let mut requires = Vec::with_capacity(arg_count);
        for n in 0..arg_count {
            let prop_iri = format!("{input_class}:arg_{n}");
            let mut p = crate::ontology::resource::Resource::new(Iri::parse(&prop_iri).unwrap());
            p.set(
                Iri::parse(wk::IS_A).unwrap(),
                crate::ontology::resource::Value::Array(vec![
                    crate::ontology::resource::Value::String(wk::PROPERTY.into()),
                ]),
            );
            b.add_resource(p).unwrap();
            requires.push(crate::ontology::resource::Value::String(prop_iri));
        }
        let mut input_class_res =
            crate::ontology::resource::Resource::new(Iri::parse(input_class).unwrap());
        input_class_res.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::CLASS.into()),
            ]),
        );
        input_class_res.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            crate::ontology::resource::Value::Array(requires),
        );
        b.add_resource(input_class_res).unwrap();

        // QueryClass declaring Decidable role for `constraint_iri`.
        let mut qc = crate::ontology::resource::Resource::new(Iri::parse(constraint_iri).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::QUERY_CLASS_CLASS.into()),
            ]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            crate::ontology::resource::Value::String(input_class.into()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            crate::ontology::resource::Value::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::DISPATCH_DECIDABLE.into()),
            ]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_decide:proc:check".into(),
            ),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");

        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(VerdictInstitution {
                iri: Iri::parse(inst_iri).unwrap(),
                verdict_class,
            }))
            .unwrap();

        EvalCtx::IO {
            layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::new(idx)),
            institution_runtime: Some(Arc::new(runtime)),
        }
    }

    #[test]
    fn native_decide_d14_holds_reduces_to_refl() {
        let ctx = build_d14_decide_ctx("urn:eigenius:institution:verdicts:holds", 1);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:has_property").unwrap(),
            args: vec![Exp::Unit],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Refl(_) => {}
            other => panic!("expected Refl from Holds verdict, got {other:?}"),
        }
    }

    #[test]
    fn native_decide_d14_fails_produces_failing_neutral() {
        let ctx = build_d14_decide_ctx("urn:eigenius:institution:verdicts:fails", 0);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:has_property").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) if name == "__constraint_failed" => {}
            other => panic!("expected __constraint_failed neutral, got {other:?}"),
        }
    }

    #[test]
    fn native_decide_d14_undecidable_produces_passthrough_neutral() {
        let ctx = build_d14_decide_ctx("urn:eigenius:institution:verdicts:undecidable", 0);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:has_property").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) if name == "__constraint_undecidable" => {}
            other => panic!("expected __constraint_undecidable neutral, got {other:?}"),
        }
    }

    #[test]
    fn native_decide_d14_falls_back_to_legacy_when_no_decidable_query_class() {
        // Constraint IRI not in the D14 index → fallback to legacy
        // institutions registry. With neither configured the legacy
        // path returns Undecidable (passthrough).
        let layer = Arc::new(
            crate::layer::LayerBuilder::new("test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let (idx, _) = InstitutionIndex::from_layer(&layer);
        let ctx = EvalCtx::IO {
            layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::new(idx)),
            institution_runtime: Some(Arc::new(InstitutionRuntime::new())),
        };
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:not_declared").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) if name == "__constraint_undecidable" => {}
            other => panic!("expected fallback Undecidable, got {other:?}"),
        }
    }
}
