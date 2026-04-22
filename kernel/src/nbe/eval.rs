//! Mini-TT evaluator: terms → values.
//!
//! Ported from `Main.hs` lines 198-217 in the Mini-TT reference.
//! Extended with capability modes (Pure/Read/IO) per D9.

use crate::institution::InstitutionRegistry;
use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Clos, Neut, Val};
use crate::ontology::iri::Iri;
use crate::program::component::ComponentRegistry;
use crate::program::trace::{ComponentTrace, TraceStore};
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
pub fn eval(exp: &Exp, rho: &Rho) -> Val {
    eval_ctx(exp, rho, &EvalCtx::Pure)
}

/// Evaluate an expression with a capability mode.
pub fn eval_ctx(exp: &Exp, rho: &Rho, ctx: &EvalCtx) -> Val {
    // Shorthand for recursive calls
    let ev = |e: &Exp| eval_ctx(e, rho, ctx);

    match exp {
        Exp::Set => Val::Set,
        Exp::Type(n) => Val::Type(*n),
        Exp::One => Val::One,
        Exp::Unit => Val::Unit,

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
                            let val = eval_ctx(body, rho, ctx);
                            let rho2 = rho.clone().extend(patt.clone(), val);
                            eval_ctx(e, &rho2, ctx)
                        }
                        crate::nbe::term::Decl::Drec(patt, _typ, body) => {
                            // Recursive: evaluate in extended env
                            let rho_ext = Rho::UpDec(Box::new(rho.clone()), d.clone());
                            let val = eval_ctx(body, &rho_ext, ctx);
                            let rho2 = rho.clone().extend(patt.clone(), val);
                            eval_ctx(e, &rho2, ctx)
                        }
                    }
                }
            }
        }

        Exp::Lam(p, e) => Val::Lam(Clos::new(p.clone(), *e.clone(), rho.clone())),

        Exp::Pi(p, a, b) => Val::Pi(
            Box::new(ev(a)),
            Clos::new(p.clone(), *b.clone(), rho.clone()),
        ),

        Exp::Sig(p, a, b) => Val::Sig(
            Box::new(ev(a)),
            Clos::new(p.clone(), *b.clone(), rho.clone()),
        ),

        Exp::Fst(e) => ev(e).vfst(),
        Exp::Snd(e) => ev(e).vsnd(),

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
                        let arg_val = ev(e2);
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
                            let arg_val = ev(e2);
                            return dispatch_fiber_query(&inst_iri, &arg_val, ctx);
                        }
                    }
                }
            }
            ev(e1).app_ctx(ev(e2), ctx)
        }

        Exp::Var(x) => rho.get(x).unwrap_or_else(|e| {
            match ctx {
                EvalCtx::Pure => {
                    // Pure mode: unbound variables are a bug — type checker
                    // should have caught them. Panic to surface the error.
                    panic!("eval (pure): {e}")
                }
                _ => {
                    // IO/Read mode: unbound variables may be component IRIs
                    // that will be intercepted at the App level.
                    Val::Nt(Neut::Gen(usize::MAX, x.clone()))
                }
            }
        }),

        Exp::Pair(e1, e2) => Val::Pair(Box::new(ev(e1)), Box::new(ev(e2))),

        Exp::Con(c, e) => Val::Con(c.clone(), Box::new(ev(e))),

        Exp::Data(summands) => Val::Data(
            summands
                .iter()
                .map(|s| (s.name.clone(), s.typ.clone()))
                .collect(),
            rho.clone(),
        ),

        Exp::Case(branches) => Val::Fun(
            branches
                .iter()
                .map(|b| (b.name.clone(), b.body.clone()))
                .collect(),
            rho.clone(),
        ),

        // Sugar: A → B = Π _ : A. B
        Exp::Arrow(a, b) => eval_ctx(&Exp::Pi(Patt::Unit, a.clone(), b.clone()), rho, ctx),
        // Sugar: A × B = Σ _ : A. B
        Exp::Times(a, b) => eval_ctx(&Exp::Sig(Patt::Unit, a.clone(), b.clone()), rho, ctx),

        // Identity type
        Exp::Id(a, x, y) => Val::Id(Box::new(ev(a)), Box::new(ev(x)), Box::new(ev(y))),
        Exp::Refl(a) => Val::Refl(Box::new(ev(a))),
        Exp::IdJ(args) => {
            let [_a, _c, d, _x, _y, p] = args.as_ref();
            let p_val = ev(p);
            match p_val {
                Val::Refl(a_val) => {
                    let d_val = ev(d);
                    d_val.app_ctx(*a_val, ctx)
                }
                Val::Nt(n) => {
                    // Blocked — all args become neutral
                    Val::Nt(Neut::App(Box::new(n), Box::new(Val::Unit)))
                }
                _ => panic!("J: proof argument is not refl or neutral"),
            }
        }

        // Native constraint checking
        Exp::NativeDecide(constraint, val) => {
            let v = ev(val);
            if check_native_constraint(constraint, &v) {
                Val::Refl(Box::new(v))
            } else {
                Val::Nt(Neut::Gen(usize::MAX, "__constraint_failed".to_string()))
            }
        }

        // Decidable equality on ground types
        Exp::DecEq(_a, x, y) => {
            let x_val = ev(x);
            let y_val = ev(y);
            if ground_values_equal(&x_val, &y_val) {
                Val::Refl(Box::new(x_val))
            } else {
                Val::Nt(Neut::Gen(usize::MAX, "__deceq_false".to_string()))
            }
        }

        // Template literal — evaluate type expressions for each reference
        Exp::Template(s, refs) => Val::TemplateVal(
            s.clone(),
            refs.iter()
                .map(|(iri, typ)| (iri.clone(), ev(typ)))
                .collect(),
        ),

        // Eigenius extensions
        Exp::EigonClass(iri) => Val::EigonClass(iri.clone()),
        Exp::EigonPrimitive(p) => Val::EigonPrimitive(*p),
        Exp::EigonResource(r) => Val::ResourceVal(r.clone()),

        Exp::PropAccess(e, prop) => {
            let v = ev(e);
            match v {
                Val::ResourceVal(r) => {
                    // Direct property access on a known resource
                    match r.get(prop) {
                        Some(val) => resource_value_to_val(val),
                        None => panic!("property {} not found on resource", prop),
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
                    panic!("observation '{}' not found in corecord", obs_name);
                }
                Val::Nt(n) => Val::Nt(Neut::PropAccess(Box::new(n), prop.clone())),
                other => panic!("property access on non-resource: {:?}", other),
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
                let val = ev(expr);
                let rval = val_to_resource_value(&val);
                r.set(prop_iri.clone(), rval);
            }
            Val::ResourceVal(Box::new(r))
        }

        // Codata (D11, Phase 9b-i)
        Exp::Codata(observations) => Val::Codata(
            observations
                .iter()
                .map(|o| (o.name.clone(), o.typ.clone()))
                .collect(),
            rho.clone(),
        ),

        Exp::CoRecord(fields) => Val::CoRecord(
            fields
                .iter()
                .map(|f| (f.name.clone(), f.body.clone()))
                .collect(),
            rho.clone(),
        ),

        Exp::Observe(e, name) => ev(e).vobserve_ctx(name, ctx),
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
) -> Val {
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
        _ => panic!("dispatch_component called outside IO mode"),
    };

    let component = match registry.get(component_iri) {
        Some(c) => c,
        None => {
            // Unknown component — return input unchanged (identity fallback)
            return input_val.clone();
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
                    return Val::ResourceVal(Box::new(output));
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
                Val::ResourceVal(Box::new(output))
            }
            Err(e) => {
                eprintln!("component dispatch failed: {e}");
                // Return empty resource instead of panicking
                Val::ResourceVal(Box::new(crate::ontology::resource::Resource::new_embedded()))
            }
        }
    } else {
        // Deterministic component — content-address memo is sound
        // and reused cross-task (D21 §3.3). Identical input across
        // two tasks hits the same entry, amortizing the dispatch.
        let cache_key = crate::program::trace::compute_trace_key(component_iri, &input_resource);
        if let Some(store) = trace_store {
            if let Some(cached) = store.get_component_trace(&cache_key) {
                return Val::ResourceVal(Box::new(cached.output));
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
                Val::ResourceVal(Box::new(output))
            }
            Err(e) => {
                eprintln!("pure component dispatch failed: {e}");
                Val::ResourceVal(Box::new(crate::ontology::resource::Resource::new_embedded()))
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
            // For other Val types, create an embedded resource
            // This is a lossy conversion — not all Vals map to Resources
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
fn dispatch_fiber_query(institution_iri: &Iri, query_val: &Val, ctx: &EvalCtx) -> Val {
    let (institutions, layer) = match ctx {
        EvalCtx::IO {
            institutions,
            layer,
            ..
        } => (institutions, layer),
        _ => panic!("dispatch_fiber_query called outside IO mode"),
    };

    let reasoner = match institutions.get(institution_iri) {
        Some(r) => r,
        None => return query_val.clone(), // Unknown institution — return input
    };

    let query_resource = val_to_resource(query_val);

    // Create a temporary ExecutionContext for the institution
    let exec_ctx = crate::context::ExecutionContext::new(
        Arc::clone(layer),
        "fiber_query",
        crate::context::ExecutionMode::ReadOnly,
    );

    match reasoner.query(&query_resource, &exec_ctx) {
        Ok(result) => Val::ResourceVal(Box::new(result)),
        Err(e) => panic!("fiber query failed: {e}"),
    }
}

/// Check a native constraint against a value.
fn check_native_constraint(constraint: &crate::nbe::term::Constraint, val: &Val) -> bool {
    use crate::nbe::term::Constraint;
    match constraint {
        Constraint::MinValue(min) => match val {
            Val::ResourceVal(r) => r
                .properties()
                .values()
                .next()
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n >= *min),
            _ => false,
        },
        Constraint::MaxValue(max) => match val {
            Val::ResourceVal(r) => r
                .properties()
                .values()
                .next()
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n <= *max),
            _ => false,
        },
        Constraint::MinLength(min) => match val {
            Val::ResourceVal(r) => r
                .properties()
                .values()
                .next()
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 >= *min),
            _ => false,
        },
        Constraint::MaxLength(max) => match val {
            Val::ResourceVal(r) => r
                .properties()
                .values()
                .next()
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 <= *max),
            _ => false,
        },
        Constraint::Pattern(pattern) => match val {
            Val::ResourceVal(r) => r
                .properties()
                .values()
                .next()
                .and_then(|v| v.as_str())
                .is_some_and(|s| {
                    let full = format!("^(?:{pattern})$");
                    regex::Regex::new(&full).is_ok_and(|re| re.is_match(s))
                }),
            _ => false,
        },
        Constraint::Format(fmt) => match val {
            Val::ResourceVal(r) => r
                .properties()
                .values()
                .next()
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
        _ => Val::Unit,
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
        _ => RVal::Embedded(Box::new(crate::ontology::resource::Resource::new_embedded())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;

    #[test]
    fn eval_set() {
        let v = eval(&Exp::Set, &Rho::Nil);
        assert!(matches!(v, Val::Set));
    }

    #[test]
    fn eval_unit() {
        let v = eval(&Exp::Unit, &Rho::Nil);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_one() {
        let v = eval(&Exp::One, &Rho::Nil);
        assert!(matches!(v, Val::One));
    }

    #[test]
    fn eval_var() {
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        let v = eval(&Exp::Var("x".to_string()), &rho);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_pair() {
        let v = eval(
            &Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)),
            &Rho::Nil,
        );
        assert!(matches!(v, Val::Pair(_, _)));
    }

    #[test]
    fn eval_fst() {
        let v = eval(
            &Exp::Fst(Box::new(Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)))),
            &Rho::Nil,
        );
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_snd() {
        let v = eval(
            &Exp::Snd(Box::new(Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)))),
            &Rho::Nil,
        );
        assert!(matches!(v, Val::Set));
    }

    #[test]
    fn eval_lambda_app() {
        // (λx. x) () = ()
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let v = eval(&Exp::App(Box::new(lam), Box::new(Exp::Unit)), &Rho::Nil);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_constructor() {
        let v = eval(&Exp::Con("ok".to_string(), Box::new(Exp::Unit)), &Rho::Nil);
        assert!(matches!(v, Val::Con(ref c, _) if c == "ok"));
    }

    #[test]
    fn eval_let() {
        // let x : 1 = (); x
        let d = crate::nbe::term::Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Unit),
        );
        let v = eval(&Exp::Dec(d, Box::new(Exp::Var("x".to_string()))), &Rho::Nil);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_neutral_var() {
        // An unbound variable in the environment produces a neutral
        let rho = Rho::Nil.extend(
            Patt::Var("x".to_string()),
            Val::Nt(Neut::Gen(0, "x".to_string())),
        );
        let v = eval(&Exp::Var("x".to_string()), &rho);
        assert!(matches!(v, Val::Nt(Neut::Gen(0, _))));
    }

    #[test]
    fn eval_neutral_app() {
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
        );
        assert!(matches!(v, Val::Nt(Neut::App(_, _))));
    }

    #[test]
    fn eval_eigon_primitive() {
        let v = eval(&Exp::EigonPrimitive(PrimitiveType::String), &Rho::Nil);
        assert!(matches!(v, Val::EigonPrimitive(PrimitiveType::String)));
    }
}
