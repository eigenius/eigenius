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

use crate::institution::InstitutionRegistry;
use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Clos, Neut, Val};
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
        institutions: Arc<InstitutionRegistry>,
        trace_store: Option<Arc<dyn TraceStore>>,
        /// ComponentTraces produced during this evaluation (for trace layer commits).
        dispatched_traces: Arc<Mutex<Vec<ComponentTrace>>>,
        /// Optional task context. When present, IO dispatches route
        /// through per-task positional trace keys (D21 §3.2) instead
        /// of the cross-task content-address cache. Synchronous
        /// `RunProgram` and the type-checker leave this `None`.
        task_context: Option<Arc<TaskContext>>,
    },
}

impl EvalCtx {
    /// A static Pure context for convenience.
    pub fn pure() -> Self {
        EvalCtx::Pure
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
            // In IO mode, check if the function is a component or institution dispatch
            if let EvalCtx::IO {
                registry,
                institutions,
                ..
            } = ctx
            {
                if let Exp::Var(name) = e1.as_ref() {
                    // Check component registry first
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
                    // Check institution registry for fiber queries
                    if let Ok(inst_iri) = Iri::parse(name) {
                        if institutions.get(&inst_iri).is_some() {
                            let arg_val = ev(e2)?;
                            return dispatch_fiber_query(&inst_iri, &arg_val, ctx);
                        }
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

        // Native constraint checking
        Exp::NativeDecide(constraint, val) => {
            let v = ev(val)?;
            if check_native_constraint(constraint, &v) {
                Ok(Val::Refl(Box::new(v)))
            } else {
                Ok(Val::Nt(Neut::Gen(
                    usize::MAX,
                    "__constraint_failed".to_string(),
                )))
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
                            eprintln!("warning: property {prop} not found on resource");
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
                    eprintln!("warning: observation '{obs_name}' not found in corecord");
                    Ok(Val::Unit)
                }
                Val::Nt(n) => Ok(Val::Nt(Neut::PropAccess(Box::new(n), prop.clone()))),
                _other => {
                    eprintln!("warning: property access on non-resource value");
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
                Val::InductiveVal { .. } => Err(EvalError::NotImplemented(
                    "inductive recursor iota reduction (Phase 11b step 2)".to_string(),
                )),
                other => Err(EvalError::InvalidCaseTarget(format!(
                    "InductiveRec: expected inductive value, got {other:?}"
                ))),
            }
        }
    }
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
                institutions,
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
                    if let Ok(inst_iri) = Iri::parse(name) {
                        if institutions.get(&inst_iri).is_some() {
                            let arg_val = eval_ctx(e2, rho, ctx)?;
                            return Ok((dispatch_fiber_query(&inst_iri, &arg_val, ctx)?, None));
                        }
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
                            eprintln!("warning: property {prop} not found on resource");
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
                    eprintln!("warning: observation '{obs_name}' not found in corecord");
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
                    eprintln!("warning: property access on non-resource value");
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
        Val::Nt(n) => Ok(Val::Nt(Neut::NtMap(Box::new(f), Box::new(n)))),
        other => Err(EvalError::InvalidCaseTarget(format!(
            "Map: expected list, got {other:?}"
        ))),
    }
}

/// Evaluate Reduce(f, accumulator, collection).
///
/// Left-folds `f` over a finite list starting with `acc`. Accepts both
/// `Val::List` and cons-pair chains. `f` is applied as `f(acc, elem)`.
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
    // JSON Schema generation.
    let schema_table = resolve_component_schemas(component_iri, &mut arg_resource, layer);

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
                // For CompleteJson: convert the short-name JSON response back to a typed Resource
                let output = if let Some((ref table, ref class_iri)) = schema_table {
                    // Check if the output has raw JSON (short-name keys from LLM)
                    let raw_json_iri = Iri::parse("urn:eigenius:core:raw_json").unwrap();
                    let json_val = if let Some(crate::ontology::resource::Value::Json(j)) =
                        result.output.get(&raw_json_iri)
                    {
                        j.clone()
                    } else {
                        // Already an Eigon resource — serialize for conversion
                        crate::ontology::eigon_json::serialize_resource(&result.output)
                    };
                    match crate::program::schema::convert_json_to_resource(
                        &json_val, table, class_iri,
                    ) {
                        Ok(converted) => converted,
                        Err(e) => {
                            eprintln!("convert_json_to_resource failed: {e}");
                            result.output.clone()
                        }
                    }
                } else {
                    result.output.clone()
                };

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
                            eprintln!("task commit_step failed: {e}");
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
                eprintln!("component dispatch failed: {e}");
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
                eprintln!("pure component dispatch failed: {e}");
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
                            return Some((table, schema_class_iri));
                        }
                        Err(e) => {
                            eprintln!("schema generation failed for {class_iri_str}: {e}");
                        }
                    }
                }
            }
        }
    }

    None
}

/// Dispatch a fiber query to an institution.
fn dispatch_fiber_query(
    institution_iri: &Iri,
    query_val: &Val,
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    let (institutions, layer) = match ctx {
        EvalCtx::IO {
            institutions,
            layer,
            ..
        } => (institutions, layer),
        _ => {
            return Err(EvalError::ModeError(
                "dispatch_fiber_query called outside IO mode".into(),
            ))
        }
    };

    let reasoner = match institutions.get(institution_iri) {
        Some(r) => r,
        None => return Ok(query_val.clone()), // Unknown institution — return input
    };

    let query_resource = val_to_resource(query_val);

    // Create a temporary ExecutionContext for the institution
    let exec_ctx = crate::context::ExecutionContext::new(
        Arc::clone(layer),
        "fiber_query",
        crate::context::ExecutionMode::ReadOnly,
    );

    match reasoner.query(&query_resource, &exec_ctx) {
        Ok(result) => Ok(Val::ResourceVal(Box::new(result))),
        Err(e) => {
            eprintln!("warning: fiber query failed: {e}");
            Ok(query_val.clone())
        }
    }
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

/// Check a native constraint against a value.
fn check_native_constraint(constraint: &crate::nbe::term::Constraint, val: &Val) -> bool {
    use crate::nbe::term::Constraint;
    match constraint {
        Constraint::MinValue(min) => match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n >= *min),
            _ => false,
        },
        Constraint::MaxValue(max) => match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n <= *max),
            _ => false,
        },
        Constraint::MinLength(min) => match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 >= *min),
            _ => false,
        },
        Constraint::MaxLength(max) => match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 <= *max),
            _ => false,
        },
        Constraint::Pattern(pattern) => match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| {
                    let full = format!("^(?:{pattern})$");
                    regex::Regex::new(&full).is_ok_and(|re| re.is_match(s))
                }),
            _ => false,
        },
        Constraint::Format(fmt) => match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| match fmt.as_str() {
                    "date" => s.len() == 10 && s.chars().nth(4) == Some('-'),
                    "uuid" => s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4,
                    _ => true,
                }),
            _ => false,
        },
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

    use crate::institution::InstitutionRegistry;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::program::component::ComponentRegistry;
    use crate::program::trace::Trace;

    /// Build a minimal IO evaluation context for traced tests.
    fn io_ctx() -> EvalCtx {
        EvalCtx::IO {
            layer: std::sync::Arc::new(crate::layer::LayerBuilder::new("empty", None).build()),
            registry: std::sync::Arc::new(ComponentRegistry::default()),
            institutions: std::sync::Arc::new(InstitutionRegistry::new()),
            trace_store: None,
            dispatched_traces: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            task_context: None,
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
    fn fiber_query_failure_returns_input() -> Result<(), EvalError> {
        // Phase 10c: A fiber query that fails should return the input
        // unchanged instead of panicking.
        use crate::context::ExecutionContext;
        use crate::institution::error::{InstitutionError, MorphismValidation};
        use crate::institution::{FiberDeclaration, FiberReasoner};

        struct FailingInstitution;
        impl FiberReasoner for FailingInstitution {
            fn fiber_declaration(&self) -> FiberDeclaration {
                FiberDeclaration {
                    institution_iri: Iri::parse("urn:eigenius:test:failing").unwrap(),
                    name: "Failing".into(),
                    morphism_types: vec![],
                    query_types: vec![],
                    structural_properties: vec![],
                }
            }
            fn query(
                &self,
                _query: &Resource,
                _ctx: &ExecutionContext,
            ) -> Result<Resource, InstitutionError> {
                Err(InstitutionError::ComputationFailed(
                    "intentional test failure".into(),
                ))
            }
            fn validate_morphism(
                &self,
                _m: &Resource,
                _ctx: &ExecutionContext,
            ) -> Result<MorphismValidation, InstitutionError> {
                Ok(MorphismValidation::Valid)
            }
            fn discover_morphisms(
                &self,
                _r: &[Resource],
                _ctx: &ExecutionContext,
            ) -> Result<Vec<Resource>, InstitutionError> {
                Ok(vec![])
            }
        }

        let mut inst_registry = InstitutionRegistry::new();
        inst_registry
            .register(Box::new(FailingInstitution))
            .unwrap();

        let layer = std::sync::Arc::new(crate::layer::LayerBuilder::new("empty", None).build());
        let ctx = EvalCtx::IO {
            layer,
            registry: std::sync::Arc::new(ComponentRegistry::default()),
            institutions: std::sync::Arc::new(inst_registry),
            trace_store: None,
            dispatched_traces: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            task_context: None,
        };

        // Build a resource to use as input
        let mut input_r = Resource::new_embedded();
        input_r.set(
            Iri::parse("urn:eigenius:test:val").unwrap(),
            Value::String("data".into()),
        );
        let rho = Rho::Nil.extend(
            Patt::Var("q".to_string()),
            Val::ResourceVal(Box::new(input_r)),
        );

        // Apply the failing institution IRI to the input
        let exp = Exp::App(
            Box::new(Exp::Var("urn:eigenius:test:failing".to_string())),
            Box::new(Exp::Var("q".to_string())),
        );
        let (val, _trace) = eval_traced(&exp, &rho, &ctx)?;

        // Should return the input unchanged (not panic)
        match val {
            Val::ResourceVal(r) => {
                assert_eq!(
                    r.get(&Iri::parse("urn:eigenius:test:val").unwrap())
                        .unwrap()
                        .as_str(),
                    Some("data"),
                    "fiber query failure should return input unchanged"
                );
            }
            other => panic!("expected ResourceVal, got {:?}", other),
        }
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
}
