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

//! Parse Eigon-JSON expression resources into EigenTT terms.
//!
//! Each expression form (Let, Apply, Var, Lambda, etc.) maps 1:1
//! to a EigenTT term. No translation layer needed.

use crate::layer::Layer;
use crate::nbe::term::{Branch, Decl, Exp, InductiveDecl, Patt, PrimitiveType};
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::ground::{is_inductive_type, resolve_class_type, resolve_inductive_type};
use std::sync::Arc;
const LET: &str = "urn:eigenius:program:Let";
const APPLY: &str = "urn:eigenius:program:Apply";
const CTOR_APPLY: &str = "urn:eigenius:program:CtorApply";
const VAR: &str = "urn:eigenius:program:Var";
const LAMBDA: &str = "urn:eigenius:program:Lambda";
const CASE: &str = "urn:eigenius:program:Case";
const MATCH: &str = "urn:eigenius:program:Match";
const PAIR: &str = "urn:eigenius:program:Pair";
const CONSTRUCT: &str = "urn:eigenius:program:Construct";
const PROJECT: &str = "urn:eigenius:program:Project";
const MAP: &str = "urn:eigenius:program:Map";
const REDUCE: &str = "urn:eigenius:program:Reduce";
const LITERAL: &str = "urn:eigenius:program:Literal";
const CORECORD: &str = "urn:eigenius:program:CoRecord";
/// Phase 11e: comorphism translation invocation via ESL
/// `f(source)` where `f` is a registered Comorphism IRI.
const COMORPHISM_INVOKE_APPLY: &str = "urn:eigenius:program:ComorphismInvokeApply";
/// Phase 11e: decide-predicate invocation via ESL
/// `f(arg1, arg2, …)` where `f` is a registered decide procedure IRI.
const DECIDE_APPLY: &str = "urn:eigenius:program:DecideApply";

/// Parse a Program resource into a EigenTT term with its type.
///
/// Returns (term, type) where:
/// - term is `Exp::Lam(input, body)`
/// - type is `Exp::Pi(input_type, output_type)`
pub fn parse_program(resource: &Resource, layer: &Layer) -> Result<(Exp, Exp), String> {
    let input_type_iri = get_iri(resource, "urn:eigenius:program:input_type")?;
    let output_type_iri = get_iri(resource, "urn:eigenius:program:output_type")?;

    let input_type = resolve_class_type(&input_type_iri, layer)?;
    let output_type = resolve_class_type(&output_type_iri, layer)?;

    let input_exp = crate::nbe::readback::readback_val(0, &input_type);
    let output_exp = crate::nbe::readback::readback_val(0, &output_type);

    let body_resource = get_embedded(resource, "urn:eigenius:program:body")?;
    let body = parse_expression(&body_resource, layer)?;

    let term = Exp::Lam(Patt::Var("input".to_string()), Box::new(body));
    let typ = Exp::Pi(
        Patt::Var("input".to_string()),
        Box::new(input_exp),
        Box::new(output_exp),
    );

    Ok((term, typ))
}

/// D37 §5.1 — decode a `program:type` value into a EigenTT `Exp`.
///
/// The Pi-type for a standalone Lambda resource lives on its
/// `urn:eigenius:program:type` slot. The slot can hold any of these
/// shapes (matching what `compile_type_expr` in the ESL compiler
/// emits for `pi`, `Arrow`, and class-reference type expressions):
///
/// - `Value::ResourceRef(iri)` — a class IRI (the leaf type).
///   Resolves through the layer chain via `resolve_class_type`.
/// - `Value::String(iri-str)` — same as ResourceRef, with the IRI
///   in string form (the pre-canonicalisation shape).
/// - `Value::Embedded(r)` with `is_a` of `TypeBinderArrow` — a
///   value-typed Pi binder `pi name : kind. body`. Recursively
///   decodes the kind + body.
/// - `Value::Embedded(r)` with `is_a` of `InductiveArgType` — a
///   parametric class application like `Option<A>`. Resolves
///   through the layer chain.
/// - `Value::Embedded(r)` with `is_a` of `TypeArrow` — a non-
///   dependent arrow `A -> B`, lowered to an anonymous-binder Pi.
///
/// Returns the EigenTT `Exp` ready for evaluation via
/// `nbe::eval::eval` and subsequent use as a checking type by
/// `nbe::check::check`.
pub fn decode_program_type(value: &Value, layer: &Layer) -> Result<Exp, String> {
    use crate::ontology::well_known as wk;
    match value {
        Value::ResourceRef(iri) => {
            let val = resolve_class_type(iri, layer)?;
            Ok(crate::nbe::readback::readback_val(0, &val))
        }
        Value::String(s) => {
            let iri = Iri::parse(s)
                .map_err(|e| format!("decode_program_type: invalid type IRI '{s}': {e}"))?;
            let val = resolve_class_type(&iri, layer)?;
            Ok(crate::nbe::readback::readback_val(0, &val))
        }
        Value::Embedded(r) => {
            let is_a: Vec<String> = r.is_a().iter().map(|i| i.as_str().to_string()).collect();
            if is_a.iter().any(|s| s == wk::TYPE_BINDER_ARROW) {
                let name = match r.get(&Iri::parse(wk::BINDER_NAME).unwrap()) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("TypeBinderArrow missing `binder_name`".to_string()),
                };
                let kind_value = r
                    .get(&Iri::parse(wk::BINDER_KIND).unwrap())
                    .ok_or_else(|| "TypeBinderArrow missing `binder_kind`".to_string())?;
                let body_value = r
                    .get(&Iri::parse(wk::BINDER_BODY).unwrap())
                    .ok_or_else(|| "TypeBinderArrow missing `binder_body`".to_string())?;
                let kind_exp = decode_program_type(kind_value, layer)?;
                let body_exp = decode_program_type(body_value, layer)?;
                Ok(Exp::Pi(
                    Patt::Var(name),
                    Box::new(kind_exp),
                    Box::new(body_exp),
                ))
            } else if is_a.iter().any(|s| s == wk::TYPE_ARROW) {
                let dom_value = r
                    .get(&Iri::parse(wk::ARROW_DOMAIN).unwrap())
                    .ok_or_else(|| "TypeArrow missing `arrow_domain`".to_string())?;
                let cod_value = r
                    .get(&Iri::parse(wk::ARROW_CODOMAIN).unwrap())
                    .ok_or_else(|| "TypeArrow missing `arrow_codomain`".to_string())?;
                let dom_exp = decode_program_type(dom_value, layer)?;
                let cod_exp = decode_program_type(cod_value, layer)?;
                Ok(Exp::Pi(Patt::Unit, Box::new(dom_exp), Box::new(cod_exp)))
            } else if is_a.iter().any(|s| s == wk::INDUCTIVE_ARG_TYPE) {
                // Parametric type — e.g., `Option<Patient>`. Mirrors
                // `decode_arg_type` in `program::ground` but without
                // the self-reference machinery (D37 type-resources
                // don't self-reference). Emit `Exp::InductiveType`
                // directly with a name-only stub decl and recursively-
                // decoded args; the type checker resolves the stub
                // by name at use time.
                let type_name = match r.get(&Iri::parse(wk::TYPE_NAME).unwrap()) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::ResourceRef(i)) => i.as_str().to_string(),
                    _ => return Err("InductiveArgType missing `type_name`".to_string()),
                };
                let class_iri = Iri::parse(&type_name).map_err(|e| {
                    format!("InductiveArgType type_name '{type_name}' invalid IRI: {e}")
                })?;
                let type_args_arr = match r.get(&Iri::parse(wk::TYPE_ARGS).unwrap()) {
                    Some(Value::Array(a)) => a.clone(),
                    None => Vec::new(),
                    Some(_) => {
                        return Err("InductiveArgType `type_args` must be an array".to_string());
                    }
                };
                let resource_arc = layer.resolve(&class_iri).ok_or_else(|| {
                    format!(
                        "InductiveArgType type_name '{type_name}' does not resolve in layer chain"
                    )
                })?;
                let resource: &Resource = &resource_arc;
                if !is_inductive_type(resource) {
                    // Not an inductive — fall back to a plain class
                    // reference. This handles class IRIs that happen
                    // to be wrapped in an InductiveArgType node with
                    // no type args.
                    let val = resolve_class_type(&class_iri, layer)?;
                    return Ok(crate::nbe::readback::readback_val(0, &val));
                }
                let name_of_iri = match resource.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
                    Some(Value::String(s)) => s.clone(),
                    _ => class_iri.local_name().to_string(),
                };
                let stub = Arc::new(InductiveDecl {
                    iri: class_iri.clone(),
                    name: name_of_iri,
                    params: Vec::new(),
                    indices: Vec::new(),
                    sort: Exp::Sort(1),
                    ctors: Vec::new(),
                });
                let sub_args: Result<Vec<Exp>, String> = type_args_arr
                    .iter()
                    .map(|a| decode_program_type(a, layer))
                    .collect();
                Ok(Exp::InductiveType(stub, sub_args?))
            } else {
                Err(format!(
                    "decode_program_type: unrecognised embedded type-resource shape with is_a={is_a:?}"
                ))
            }
        }
        other => Err(format!(
            "decode_program_type: unrecognised value shape: {other:?}"
        )),
    }
}

/// Parse an expression resource into a EigenTT term.
pub fn parse_expression(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let is_a = resource.is_a();
    let class_str = is_a.first().map(|i| i.as_str()).unwrap_or("");

    match class_str {
        LET => parse_let(resource, layer),
        APPLY => parse_apply(resource, layer),
        CTOR_APPLY => parse_ctor_apply(resource, layer),
        VAR => parse_var(resource, layer),
        LAMBDA => parse_lambda(resource, layer),
        CASE => parse_case(resource, layer),
        MATCH => parse_match(resource, layer),
        PAIR => parse_pair(resource, layer),
        CONSTRUCT => parse_construct(resource, layer),
        PROJECT => parse_project(resource, layer),
        MAP => parse_map(resource, layer),
        REDUCE => parse_reduce(resource, layer),
        LITERAL => parse_literal(resource),
        CORECORD => parse_corecord(resource, layer),
        COMORPHISM_INVOKE_APPLY => parse_comorphism_invoke_apply(resource, layer),
        DECIDE_APPLY => parse_decide_apply(resource, layer),
        _ => Err(format!("unknown expression class: '{class_str}'")),
    }
}

/// Phase 11e: `function(source)` where `function` is a registered
/// Comorphism — emits
/// `Exp::InstitutionInvoke { iri, source, target_iri }`.
///
/// `target_iri` is the optional explicit IRI override that surface
/// languages may set (EigenQL `INTO` clause). When absent the kernel
/// assigns a deterministic content-hash IRI at evaluation time.
fn parse_comorphism_invoke_apply(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let func_str = get_string(resource, "urn:eigenius:program:function")?;
    let comorphism_iri = Iri::parse(&func_str)
        .map_err(|e| format!("ComorphismInvokeApply function `{func_str}` invalid IRI: {e}"))?;
    let source_resource = get_embedded(resource, "urn:eigenius:program:source")?;
    let source_exp = parse_expression(&source_resource, layer)?;
    let target_iri_prop = Iri::parse("urn:eigenius:program:target_iri").unwrap();
    let target_iri = match resource.get(&target_iri_prop) {
        Some(Value::String(s)) => Some(
            Iri::parse(s)
                .map_err(|e| format!("ComorphismInvokeApply target_iri `{s}` invalid IRI: {e}"))?,
        ),
        Some(_) => {
            return Err("ComorphismInvokeApply target_iri must be a string IRI".to_string());
        }
        None => None,
    };
    Ok(Exp::InstitutionInvoke {
        comorphism_iri,
        source: Box::new(source_exp),
        target_iri,
    })
}

/// Phase 11e: `function(args…)` where `function` is a registered
/// decide procedure — emits
/// `Exp::NativeDecide(Constraint::Institution { iri, args }, Exp::Unit)`.
/// The `Unit` placeholder is the "witness value" slot; user-facing
/// decide calls don't naturally bind a value, so we use unit and
/// let the `Refl` result be a proof token.
fn parse_decide_apply(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let func_str = get_string(resource, "urn:eigenius:program:function")?;
    let procedure_iri = Iri::parse(&func_str)
        .map_err(|e| format!("DecideApply function `{func_str}` invalid IRI: {e}"))?;
    let args_prop = Iri::parse("urn:eigenius:program:arguments").unwrap();
    let arg_values = match resource.get(&args_prop) {
        Some(Value::Array(arr)) => arr.as_slice(),
        None => &[],
        Some(_) => return Err("DecideApply: `arguments` must be an array".to_string()),
    };
    let args: Result<Vec<Exp>, String> = arg_values
        .iter()
        .map(|v| match v {
            Value::Embedded(r) => parse_expression(r, layer),
            Value::String(s) => Ok(Exp::Var(s.clone())),
            other => Err(format!(
                "DecideApply argument must be embedded or string, got {other:?}"
            )),
        })
        .collect();
    Ok(Exp::NativeDecide(
        crate::nbe::term::Constraint::Institution {
            iri: procedure_iri,
            args: args?,
        },
        Box::new(Exp::Unit),
    ))
}

/// Resolve a possible constructor IRI of the form `<parent_iri>:<ctor_name>`.
///
/// Returns `(decl, ctor_idx, arity)` for the matching constructor, where
/// `arity` is the number of non-parameter binders in the constructor's
/// Π-telescope. Returns `None` if `s` doesn't look like a ctor IRI or
/// the implied parent isn't an inductive type in the layer.
///
/// IRI-keyed (Phase 11b step 9): no layer-wide name search. The split
/// is by the last `:` — the ESL compiler builds ctor IRIs as exactly
/// `parent_iri + ":" + ctor_name`, so this round-trips by construction.
fn resolve_ctor_iri(s: &str, layer: &Layer) -> Option<(Arc<InductiveDecl>, usize, usize)> {
    let (parent_str, ctor_name) = s.rsplit_once(':')?;
    let parent_iri = Iri::parse(parent_str).ok()?;
    let resource_arc = layer.resolve(&parent_iri)?;
    let resource: &Resource = &resource_arc;
    if !is_inductive_type(resource) {
        return None;
    }
    let val = resolve_inductive_type(&parent_iri, resource, layer).ok()?;
    let Val::InductiveType { decl, .. } = val else {
        return None;
    };
    let idx = decl.ctors.iter().position(|c| c.name == ctor_name)?;
    let arity = ctor_arity(&decl, idx);
    Some((decl, idx, arity))
}

/// Number of non-parameter argument binders in a constructor's
/// Π-telescope.
fn ctor_arity(decl: &InductiveDecl, idx: usize) -> usize {
    let mut current = &decl.ctors[idx].typ;
    let mut params_to_skip = decl.params.len();
    let mut count = 0;
    while let Exp::Pi(_, _, body) = current {
        if params_to_skip > 0 {
            params_to_skip -= 1;
        } else {
            count += 1;
        }
        current = body;
    }
    count
}

/// let name : type = value; body
fn parse_let(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let name = get_string(resource, "urn:eigenius:program:name")?;

    let type_iri = get_iri(resource, "urn:eigenius:program:type")?;
    let type_val = resolve_class_type(&type_iri, layer)?;
    let type_exp = crate::nbe::readback::readback_val(0, &type_val);

    let value_resource = get_embedded(resource, "urn:eigenius:program:value")?;
    let value_exp = parse_expression(&value_resource, layer)?;

    let body_resource = get_embedded(resource, "urn:eigenius:program:body")?;
    let body_exp = parse_expression(&body_resource, layer)?;

    let decl = Decl::Def(Patt::Var(name), Box::new(type_exp), Box::new(value_exp));

    Ok(Exp::Dec(decl, Box::new(body_exp)))
}

/// f(arg) with optional component_argument
fn parse_apply(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    // Constructor application no longer flows through this path: the
    // ESL compiler routes ctor references to `CtorApply` resources
    // (parsed by `parse_ctor_apply`). Anything reaching here is a
    // genuine function application or a component dispatch.
    let func_prop = Iri::parse("urn:eigenius:program:function").unwrap();

    let func_exp = match resource.get(&func_prop) {
        // Resource references in function/argument positions are
        // typed as `data_type: resource`, so the canonical shape is
        // `ResourceRef`; `String` survives for pre-canonicalisation
        // intermediates (RPC payloads, FIBER-synthesised programs).
        Some(Value::ResourceRef(i)) => Exp::Var(i.as_str().to_string()),
        Some(Value::String(s)) => Exp::Var(s.clone()),
        Some(Value::Embedded(r)) => parse_expression(r, layer)?,
        _ => return Err("Apply: missing 'function' property".to_string()),
    };

    let arg_prop = Iri::parse("urn:eigenius:program:argument").unwrap();
    let arg_exp = match resource.get(&arg_prop) {
        Some(Value::ResourceRef(i)) => Exp::Var(i.as_str().to_string()),
        Some(Value::String(s)) => Exp::Var(s.clone()),
        Some(Value::Embedded(r)) => parse_expression(r, layer)?,
        _ => Exp::Unit, // No argument
    };

    // Check for component_argument (static config for IO components)
    let comp_arg_prop = Iri::parse("urn:eigenius:program:component_argument").unwrap();
    let effective_arg = match resource.get(&comp_arg_prop) {
        Some(Value::Embedded(comp_arg)) => {
            // A component_argument that references a published `RuntimeScript`
            // by IRI (`runtime:script`) is resolved against the graph here
            // and expanded into the flat shape the substrate consumes (D26
            // §6.2); the inline ad-hoc-script form passes through unchanged.
            let resolved = expand_runtime_script_argument(comp_arg, layer)?;
            // Pack as Pair(arg, EigonResource(comp_arg)) so the dispatcher can extract it
            Exp::Pair(
                Box::new(arg_exp),
                Box::new(Exp::EigonResource(Box::new(resolved))),
            )
        }
        _ => arg_exp,
    };

    Ok(Exp::App(Box::new(func_exp), Box::new(effective_arg)))
}

/// IRI of the `runtime:script` reference a `RunRuntimeScript`
/// component_argument may carry instead of inline `source`/`language`.
const RUNTIME_SCRIPT_REF: &str = "urn:eigenius:runtime:script";
const RUNTIME_LANGUAGE: &str = "urn:eigenius:runtime:language";
const RUNTIME_SOURCE: &str = "urn:eigenius:runtime:source";
const RUNTIME_IMAGE_DIGEST: &str = "urn:eigenius:runtime:image_digest";
const RUNTIME_REQUIRES_ENV: &str = "urn:eigenius:runtime:requires_environment";

/// Resolve a `RunRuntimeScript` component_argument that references a
/// published [`RuntimeScript`] by IRI into the flat
/// `{language, source, image_digest}` shape the substrate's
/// `dispatch_run_runtime_script` consumes (D26 §6.2).
///
/// The published script stays the run-time source of truth: the program
/// references it by IRI (preserving the D26 §6.5 reproducibility anchor
/// `script_iri + env_iri + …`), and the kernel resolves `source` +
/// `language` from the `RuntimeScript` and `image_digest` from its
/// `requires_environment` at execution. A component_argument with no
/// `runtime:script` reference is the inline ad-hoc-script form and is
/// returned unchanged.
///
/// [`RuntimeScript`]: https://example.invalid (ontology class
/// `urn:eigenius:runtime:RuntimeScript`)
fn expand_runtime_script_argument(comp_arg: &Resource, layer: &Layer) -> Result<Resource, String> {
    let read = |r: &Resource, p: &str| -> Option<String> {
        r.get(&Iri::parse(p).unwrap()).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            other => other.as_iri_str().map(str::to_string),
        })
    };

    let script_ref = match read(comp_arg, RUNTIME_SCRIPT_REF) {
        Some(s) => s,
        // Inline form (carries source/language directly): no resolution.
        None => return Ok(comp_arg.clone()),
    };

    let script_iri = Iri::parse(&script_ref).map_err(|_| {
        format!("component_argument `runtime:script` is not a valid IRI: {script_ref}")
    })?;
    let script = layer.resolve(&script_iri).ok_or_else(|| {
        format!("RunRuntimeScript: published RuntimeScript `{script_ref}` not found on the chain")
    })?;

    let language = read(&script, RUNTIME_LANGUAGE)
        .ok_or_else(|| format!("RuntimeScript `{script_ref}` missing required `language`"))?;
    let source = read(&script, RUNTIME_SOURCE)
        .ok_or_else(|| format!("RuntimeScript `{script_ref}` missing required `source`"))?;
    let env_ref = read(&script, RUNTIME_REQUIRES_ENV).ok_or_else(|| {
        format!("RuntimeScript `{script_ref}` missing required `requires_environment`")
    })?;

    let env_iri = Iri::parse(&env_ref).map_err(|_| {
        format!("RuntimeScript `{script_ref}` has invalid `requires_environment`: {env_ref}")
    })?;
    let env = layer.resolve(&env_iri).ok_or_else(|| {
        format!(
            "RuntimeScript `{script_ref}` references RuntimeEnvironment `{env_ref}` not found on the chain"
        )
    })?;
    let image_digest = read(&env, RUNTIME_IMAGE_DIGEST).ok_or_else(|| {
        format!("RuntimeEnvironment `{env_ref}` missing `image_digest` (build the env image first)")
    })?;

    let mut expanded = Resource::new_embedded();
    expanded.set(
        Iri::parse(RUNTIME_LANGUAGE).unwrap(),
        Value::String(language),
    );
    expanded.set(Iri::parse(RUNTIME_SOURCE).unwrap(), Value::String(source));
    expanded.set(
        Iri::parse(RUNTIME_IMAGE_DIGEST).unwrap(),
        Value::String(image_digest),
    );
    Ok(expanded)
}

/// Inductive constructor application (Phase 11b step 10, multi-arg).
///
/// Resource shape: `function: <ctor_iri>` plus `arguments: Array<Expr>`
/// — the canonical path for constructor application produced by the
/// ESL compiler. Decodes each argument expression and assembles
/// `Exp::InductiveCtor(decl, ctor_name, args)`.
///
/// Arity is checked against the declared constructor — mismatches
/// fail here rather than later at type-check time so the diagnostic
/// points to the ESL source.
fn parse_ctor_apply(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let func_str = get_string(resource, "urn:eigenius:program:function")?;
    let (decl, idx, arity) = resolve_ctor_iri(&func_str, layer).ok_or_else(|| {
        format!("CtorApply function `{func_str}` does not resolve to a known inductive constructor")
    })?;
    let ctor_name = decl.ctors[idx].name.clone();

    let args_prop = Iri::parse("urn:eigenius:program:arguments").unwrap();
    let arg_values = match resource.get(&args_prop) {
        Some(Value::Array(arr)) => arr.as_slice(),
        None => &[],
        Some(_) => return Err("CtorApply: `arguments` must be an array".to_string()),
    };
    if arg_values.len() != arity {
        return Err(format!(
            "CtorApply `{}.{ctor_name}` expects {arity} args, got {}",
            decl.name,
            arg_values.len()
        ));
    }
    let args: Result<Vec<Exp>, String> = arg_values
        .iter()
        .map(|v| match v {
            Value::Embedded(r) => parse_expression(r, layer),
            Value::String(s) => Ok(Exp::Var(s.clone())),
            other => Err(format!(
                "CtorApply argument must be embedded or string, got {other:?}"
            )),
        })
        .collect();
    Ok(Exp::InductiveCtor(decl, ctor_name, args?))
}

/// x
fn parse_var(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let name = get_string(resource, "urn:eigenius:program:name")?;
    // Constructor reference special case (Phase 11b step 9): if the
    // var name is a ctor IRI of the form `<parent>:<ctor>`, emit
    // `Exp::InductiveCtor` rather than a free variable. The ESL
    // compiler resolves bare names against its per-file ctor table
    // and writes the canonical IRI here.
    if let Some((decl, idx, arity)) = resolve_ctor_iri(&name, layer) {
        if arity == 0 {
            let ctor_name = decl.ctors[idx].name.clone();
            return Ok(Exp::InductiveCtor(decl, ctor_name, Vec::new()));
        }
    }
    Ok(Exp::Var(name))
}

/// λ param : type. body
fn parse_lambda(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let param = get_string(resource, "urn:eigenius:program:parameter")?;
    let body_resource = get_embedded(resource, "urn:eigenius:program:body")?;
    let body_exp = parse_expression(&body_resource, layer)?;
    Ok(Exp::Lam(Patt::Var(param), Box::new(body_exp)))
}

/// case scrutinee of c₁ → e₁ | c₂ → e₂
fn parse_case(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let scrutinee_resource = get_embedded(resource, "urn:eigenius:program:scrutinee")?;
    let scrutinee_exp = parse_expression(&scrutinee_resource, layer)?;

    let branches_prop = Iri::parse("urn:eigenius:program:branches").unwrap();
    let branches_arr = match resource.get(&branches_prop) {
        Some(Value::Array(arr)) => arr,
        _ => return Err("Case: missing 'branches' array".to_string()),
    };

    let mut branches = Vec::new();
    for branch_val in branches_arr {
        let branch_resource = match branch_val {
            Value::Embedded(r) => r,
            _ => return Err("Case branch must be an embedded resource".to_string()),
        };

        let constructor = get_string(branch_resource, "urn:eigenius:program:constructor")?;
        let body_resource = get_embedded(branch_resource, "urn:eigenius:program:body")?;
        let body_exp = parse_expression(&body_resource, layer)?;

        branches.push(Branch {
            name: constructor,
            body: body_exp,
        });
    }

    // case e of branches → App(Case(branches), e)
    let case_fn = Exp::Case(branches);
    Ok(Exp::App(Box::new(case_fn), Box::new(scrutinee_exp)))
}

/// `match scrutinee : T { ctor -> body; ctor(bindings) -> body; ... }`
/// (Phase 11b step 11, D19 §10).
///
/// Desugars to `Exp::InductiveRec` with:
/// - **motive**: `λ_:I. T` — constant function returning the
///   user-annotated result type. Motive inference is a future
///   extension.
/// - **minors**: one per constructor in the inductive, in declaration
///   order. Each minor is the matching arm's body wrapped in lambdas
///   for the bindings and (anonymous) lambdas for any IH arguments
///   the recursor produces for recursive constructor arguments.
///
/// Surfaces structural errors at parse time:
/// - non-exhaustive matches (missing case for some declared ctor)
/// - duplicate arms (two arms for the same ctor)
/// - arm referencing an unknown ctor of the parent inductive
/// - binding count mismatch (e.g. `cons(x)` for a 2-arg `cons`)
fn parse_match(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let scrutinee_resource = get_embedded(resource, "urn:eigenius:program:scrutinee")?;
    let scrutinee_exp = parse_expression(&scrutinee_resource, layer)?;

    let arms_prop = Iri::parse("urn:eigenius:program:arms").unwrap();
    let arms_arr = match resource.get(&arms_prop) {
        Some(Value::Array(arr)) => arr,
        _ => return Err("Match: missing or non-array `arms`".to_string()),
    };
    if arms_arr.is_empty() {
        return Err("Match: must have at least one arm".to_string());
    }

    // Decode each arm into a generic structure usable by both code
    // paths (motive-eager `InductiveRec` and motive-deferred `Match`).
    let parsed_arms = decode_match_arms(arms_arr, layer)?;

    // Branch on whether a motive was annotated in the source:
    //
    // - `program:result_motive` (eigenius#72 Layer 3): a D47-encoded
    //   `Exp` chain — typically `Exp::Lam(idx_1, Exp::Lam(idx_2, …,
    //   body))` from a `fun (i_1 : T_1, …) => body` motive. Used as
    //   the motive directly in `Exp::InductiveRec`. Required for
    //   indexed inductives whose result type depends on the indices.
    // - `program:result_type` (pre-Layer-3, Phase 11b step 11): a
    //   resource IRI naming a class. Wrapped as the constant motive
    //   `λ_. T`. Still supported for non-indexed inductives.
    // - Neither → produce `Exp::Match`; the kernel's type checker
    //   synthesises the motive from checking-mode context (Phase 11b
    //   step 12, D19 §10).
    let result_motive_prop = Iri::parse("urn:eigenius:program:result_motive").unwrap();
    let result_type_prop = Iri::parse("urn:eigenius:program:result_type").unwrap();
    if let Some(motive_value) = resource.get(&result_motive_prop) {
        let motive = crate::program::eigentt_type_mirror::decode_type(motive_value, layer)
            .map_err(|e| format!("invalid `result_motive` payload: {e:?}"))?;
        build_inductive_rec(parsed_arms, scrutinee_exp, motive, layer)
    } else if let Some(rt_iri_str) = resource.get(&result_type_prop).and_then(|v| v.as_iri_str()) {
        // `program:result_type` is `data_type: resource`, so post-
        // `canonicalise_resource_refs` the value is `ResourceRef`,
        // not `String`. `as_iri_str` handles both shapes.
        let result_type_iri = Iri::parse(rt_iri_str)
            .map_err(|e| format!("invalid `result_type` IRI '{rt_iri_str}': {e}"))?;
        let result_type_val = resolve_class_type(&result_type_iri, layer)?;
        let motive_body = crate::nbe::readback::readback_val(0, &result_type_val);
        let motive = Exp::Lam(Patt::Unit, Box::new(motive_body));
        build_inductive_rec(parsed_arms, scrutinee_exp, motive, layer)
    } else {
        build_match_exp(parsed_arms, scrutinee_exp, layer)
    }
}

/// Parsed match arm shared between `Exp::InductiveRec` and
/// `Exp::Match` build paths.
struct ParsedArm {
    parent_iri_str: String,
    ctor_local: String,
    binding_names: Vec<String>,
    body: Exp,
}

/// Decode the embedded `MatchArm` resources into a uniform shape.
fn decode_match_arms(arms_arr: &[Value], layer: &Layer) -> Result<Vec<ParsedArm>, String> {
    let mut out = Vec::with_capacity(arms_arr.len());
    for entry in arms_arr {
        let ar = match entry {
            Value::Embedded(r) => r.as_ref(),
            _ => return Err("Match arm must be embedded".to_string()),
        };
        let ctor_iri = get_string(ar, "urn:eigenius:program:ctor")?;
        let (parent_str, ctor_local) = ctor_iri.rsplit_once(':').ok_or_else(|| {
            format!("Match arm ctor IRI `{ctor_iri}` is not in `<parent>:<ctor>` form")
        })?;
        let bindings_arr = match ar.get(&Iri::parse("urn:eigenius:program:bindings").unwrap()) {
            Some(Value::Array(arr)) => arr,
            _ => return Err("Match arm missing `bindings` array".to_string()),
        };
        let binding_names: Result<Vec<String>, String> = bindings_arr
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.clone()),
                _ => Err("Match arm binding must be a string".to_string()),
            })
            .collect();
        let body_resource = get_embedded(ar, "urn:eigenius:program:body")?;
        let body = parse_expression(&body_resource, layer)?;
        out.push(ParsedArm {
            parent_iri_str: parent_str.to_string(),
            ctor_local: ctor_local.to_string(),
            binding_names: binding_names?,
            body,
        });
    }
    Ok(out)
}

/// Verify all arms address the same inductive, then resolve it to
/// `Arc<InductiveDecl>`.
fn resolve_match_parent(arms: &[ParsedArm], layer: &Layer) -> Result<Arc<InductiveDecl>, String> {
    let parent_iri_str = &arms[0].parent_iri_str;
    for arm in &arms[1..] {
        if arm.parent_iri_str != *parent_iri_str {
            return Err(format!(
                "Match arms reference different inductives: `{parent_iri_str}` vs `{}`",
                arm.parent_iri_str
            ));
        }
    }
    let parent_iri = Iri::parse(parent_iri_str)
        .map_err(|e| format!("invalid match arm parent IRI `{parent_iri_str}`: {e}"))?;
    let parent_resource_arc = layer
        .resolve(&parent_iri)
        .ok_or_else(|| format!("match arm parent inductive `{parent_iri_str}` not in layer"))?;
    let parent_resource: &Resource = &parent_resource_arc;
    if !is_inductive_type(parent_resource) {
        return Err(format!(
            "match arm parent `{parent_iri_str}` is not an inductive type"
        ));
    }
    let parent_val = resolve_inductive_type(&parent_iri, parent_resource, layer)?;
    match parent_val {
        Val::InductiveType { decl, .. } => Ok(decl),
        _ => unreachable!("is_inductive_type checked"),
    }
}

/// Build `Exp::InductiveRec` from match arms with a pre-built motive.
/// Validates exhaustiveness, binding counts, and unknown ctors. The
/// motive is supplied by the caller — either a `Lam`-chain over the
/// scrutinee's indices (Layer 3 `fun (i : T) => body` path) or a
/// `λ_. T` constant motive (pre-Layer-3 `returning T` path).
fn build_inductive_rec(
    parsed_arms: Vec<ParsedArm>,
    scrutinee_exp: Exp,
    motive: Exp,
    layer: &Layer,
) -> Result<Exp, String> {
    use std::collections::BTreeMap;
    let decl = resolve_match_parent(&parsed_arms, layer)?;

    let mut arms_by_ctor: BTreeMap<String, ParsedArm> = BTreeMap::new();
    for arm in parsed_arms {
        let ctor = arm.ctor_local.clone();
        if arms_by_ctor.insert(ctor.clone(), arm).is_some() {
            return Err(format!("Match has duplicate arms for ctor `{ctor}`"));
        }
    }
    for ctor_name in arms_by_ctor.keys() {
        if !decl.ctors.iter().any(|c| &c.name == ctor_name) {
            return Err(format!(
                "Match arm references ctor `{}.{ctor_name}` which is not declared in that inductive",
                decl.name
            ));
        }
    }

    let mut minors = Vec::with_capacity(decl.ctors.len());
    for ctor in &decl.ctors {
        let arm = arms_by_ctor.remove(&ctor.name).ok_or_else(|| {
            format!(
                "non-exhaustive match: missing case for `{}.{}`",
                decl.name, ctor.name
            )
        })?;
        let arity = ctor_arity(
            &decl,
            decl.ctors.iter().position(|c| c.name == ctor.name).unwrap(),
        );
        if arm.binding_names.len() != arity {
            return Err(format!(
                "match arm `{}.{}` expects {arity} bindings, got {}",
                decl.name,
                ctor.name,
                arm.binding_names.len()
            ));
        }
        let recursive_count = recursive_arg_count(&decl, &ctor.typ);
        let mut minor = arm.body;
        for _ in 0..recursive_count {
            minor = Exp::Lam(Patt::Unit, Box::new(minor));
        }
        for binding in arm.binding_names.iter().rev() {
            let patt = patt_for_binding(binding);
            minor = Exp::Lam(patt, Box::new(minor));
        }
        minors.push(minor);
    }

    Ok(Exp::InductiveRec {
        decl,
        motive: Box::new(motive),
        minors,
        major: Box::new(scrutinee_exp),
    })
}

/// Build `Exp::Match` from match arms when the source had no
/// `returning T` annotation (Phase 11b step 12). The kernel's type
/// checker synthesises the motive from checking-mode context.
fn build_match_exp(
    parsed_arms: Vec<ParsedArm>,
    scrutinee_exp: Exp,
    layer: &Layer,
) -> Result<Exp, String> {
    // Validate the parent inductive resolves cleanly so any
    // `unknown inductive` errors fire here rather than later in the
    // type checker.
    let _ = resolve_match_parent(&parsed_arms, layer)?;

    let arms: Vec<crate::nbe::term::MatchArm> = parsed_arms
        .into_iter()
        .map(|arm| crate::nbe::term::MatchArm {
            ctor_name: arm.ctor_local,
            bindings: arm.binding_names.iter().map(patt_for_binding).collect(),
            body: arm.body,
        })
        .collect();

    Ok(Exp::Match {
        scrutinee: Box::new(scrutinee_exp),
        arms,
    })
}

/// Convert a binding name string to a `Patt`. Underscore is the
/// wildcard binding (`Patt::Unit`); everything else is `Patt::Var`.
fn patt_for_binding(name: impl AsRef<str>) -> Patt {
    let n = name.as_ref();
    if n == "_" {
        Patt::Unit
    } else {
        Patt::Var(n.to_string())
    }
}

/// Count the recursive (self-referential) constructor argument
/// binders in a constructor's Π-telescope, skipping the parameter
/// prefix.
fn recursive_arg_count(decl: &InductiveDecl, ctor_typ: &Exp) -> usize {
    let mut current = ctor_typ;
    let mut params_to_skip = decl.params.len();
    let mut count = 0;
    while let Exp::Pi(_, dom, body) = current {
        if params_to_skip > 0 {
            params_to_skip -= 1;
        } else if matches!(dom.as_ref(), Exp::InductiveType(d, _) if d.name == decl.name) {
            count += 1;
        }
        current = body;
    }
    count
}

/// (first, second)
fn parse_pair(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let first_resource = get_embedded(resource, "urn:eigenius:program:first")?;
    let second_resource = get_embedded(resource, "urn:eigenius:program:second")?;
    let first_exp = parse_expression(&first_resource, layer)?;
    let second_exp = parse_expression(&second_resource, layer)?;
    Ok(Exp::Pair(Box::new(first_exp), Box::new(second_exp)))
}

/// Construct ClassName { prop₁: e₁, prop₂: e₂ }
fn parse_construct(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let class_iri = get_iri(resource, "urn:eigenius:program:class")?;

    let fields_prop = Iri::parse("urn:eigenius:program:fields").unwrap();
    let fields = match resource.get(&fields_prop) {
        Some(Value::Embedded(r)) => r,
        _ => return Err("Construct: missing 'fields'".to_string()),
    };

    // Build named fields: [(prop_iri, expr), ...]
    let mut named_fields: Vec<(Iri, Box<Exp>)> = Vec::new();
    for (prop_iri, val) in fields.properties() {
        let field_exp = match val {
            Value::Embedded(r) => parse_expression(r, layer)?,
            Value::String(s) => Exp::Var(s.clone()),
            _ => Exp::Unit,
        };
        named_fields.push((prop_iri.clone(), Box::new(field_exp)));
    }

    Ok(Exp::Construct(class_iri, named_fields))
}

/// e.property
fn parse_project(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let expr_resource = get_embedded(resource, "urn:eigenius:program:expression")?;
    let expr_exp = parse_expression(&expr_resource, layer)?;

    let prop_iri = get_iri(resource, "urn:eigenius:program:property")?;

    Ok(Exp::PropAccess(Box::new(expr_exp), prop_iri))
}

/// corecord { obs = e; ... }
fn parse_corecord(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    use crate::nbe::term::CoField;
    let cofields_prop = Iri::parse("urn:eigenius:program:cofields").unwrap();
    let cofields = match resource.get(&cofields_prop) {
        Some(Value::Array(arr)) => arr,
        _ => return Err("CoRecord missing 'cofields' array".to_string()),
    };
    let mut fields = Vec::new();
    for entry in cofields {
        let cf = match entry {
            Value::Embedded(r) => r.as_ref(),
            _ => {
                return Err(
                    "CoRecord 'cofields' must contain embedded CoField resources".to_string(),
                )
            }
        };
        let name = get_string(cf, "urn:eigenius:program:observation_name")?;
        let body_resource = get_embedded(cf, "urn:eigenius:program:body")?;
        let body_exp = parse_expression(&body_resource, layer)?;
        fields.push(CoField {
            name,
            body: body_exp,
        });
    }
    Ok(Exp::CoRecord(fields))
}

/// map(f, collection)
fn parse_map(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let func_resource = get_embedded(resource, "urn:eigenius:program:function")?;
    let func_exp = parse_expression(&func_resource, layer)?;

    let coll_resource = get_embedded(resource, "urn:eigenius:program:collection")?;
    let coll_exp = parse_expression(&coll_resource, layer)?;

    Ok(Exp::Map(Box::new(func_exp), Box::new(coll_exp)))
}

/// reduce(f, initial, collection)
fn parse_reduce(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let func_resource = get_embedded(resource, "urn:eigenius:program:function")?;
    let func_exp = parse_expression(&func_resource, layer)?;

    let init_resource = get_embedded(resource, "urn:eigenius:program:initial")?;
    let init_exp = parse_expression(&init_resource, layer)?;

    let coll_resource = get_embedded(resource, "urn:eigenius:program:collection")?;
    let coll_exp = parse_expression(&coll_resource, layer)?;

    Ok(Exp::Reduce(
        Box::new(func_exp),
        Box::new(init_exp),
        Box::new(coll_exp),
    ))
}

/// Literal value
fn parse_literal(resource: &Resource) -> Result<Exp, String> {
    let val_prop = Iri::parse("urn:eigenius:program:value").unwrap();
    match resource.get(&val_prop) {
        // Canonical IRI reference shape after `canonicalise_resource_refs`.
        Some(Value::ResourceRef(i)) => Ok(Exp::Var(i.as_str().to_string())),
        Some(Value::String(s)) => {
            // Pre-canonicalisation: a string literal that *might* be
            // an IRI reference (heuristic on `urn:` / `http`).
            if Iri::parse(s).is_ok() && (s.starts_with("urn:") || s.starts_with("http")) {
                return Ok(Exp::Var(s.clone())); // Resource reference
            }
            Ok(Exp::EigonPrimitive(PrimitiveType::String)) // String literal
        }
        Some(Value::Integer(_)) => Ok(Exp::EigonPrimitive(PrimitiveType::Integer)),
        Some(Value::Float(_)) => Ok(Exp::EigonPrimitive(PrimitiveType::Float)),
        Some(Value::Boolean(_)) => Ok(Exp::EigonPrimitive(PrimitiveType::Boolean)),
        _ => Ok(Exp::Unit),
    }
}

// --- Helpers ---

fn get_string(resource: &Resource, prop: &str) -> Result<String, String> {
    let prop_iri = Iri::parse(prop).unwrap();
    match resource.get(&prop_iri) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(format!("missing string property '{prop}'")),
    }
}

fn get_iri(resource: &Resource, prop: &str) -> Result<Iri, String> {
    let prop_iri = Iri::parse(prop).unwrap();
    // IRI-typed property values canonicalise to `ResourceRef`;
    // `as_iri` accepts both that and the pre-canonical `String`
    // shape from intermediate (uncommitted) resources.
    resource
        .get(&prop_iri)
        .and_then(|v| v.as_iri())
        .ok_or_else(|| format!("missing IRI property '{prop}'"))
}

fn get_embedded(resource: &Resource, prop: &str) -> Result<Resource, String> {
    let prop_iri = Iri::parse(prop).unwrap();
    match resource.get(&prop_iri) {
        Some(Value::Embedded(r)) => Ok(r.as_ref().clone()),
        _ => Err(format!("missing embedded resource at '{prop}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_var_expression() {
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:program:Var".to_string())]),
        );
        r.set(
            Iri::parse("urn:eigenius:program:name").unwrap(),
            Value::String("x".to_string()),
        );
        let layer = crate::layer::LayerBuilder::new("empty", None)
            .build(crate::layer::LayerStorage::in_memory());
        let exp = parse_expression(&r, &layer).unwrap();
        assert!(matches!(exp, Exp::Var(ref n) if n == "x"));
    }

    #[test]
    fn parse_apply_expression() {
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:program:Apply".to_string(),
            )]),
        );
        r.set(
            Iri::parse("urn:eigenius:program:function").unwrap(),
            Value::String("urn:eigenius:components:Identity".to_string()),
        );
        let layer = crate::layer::LayerBuilder::new("empty", None)
            .build(crate::layer::LayerStorage::in_memory());
        let exp = parse_expression(&r, &layer).unwrap();
        assert!(matches!(exp, Exp::App(_, _)));
    }

    #[test]
    fn parse_lambda_expression() {
        let mut body = Resource::new_embedded();
        body.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:program:Var".to_string())]),
        );
        body.set(
            Iri::parse("urn:eigenius:program:name").unwrap(),
            Value::String("x".to_string()),
        );

        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:program:Lambda".to_string(),
            )]),
        );
        r.set(
            Iri::parse("urn:eigenius:program:parameter").unwrap(),
            Value::String("x".to_string()),
        );
        r.set(
            Iri::parse("urn:eigenius:program:body").unwrap(),
            Value::Embedded(Box::new(body)),
        );

        let layer = crate::layer::LayerBuilder::new("empty", None)
            .build(crate::layer::LayerStorage::in_memory());
        let exp = parse_expression(&r, &layer).unwrap();
        assert!(matches!(exp, Exp::Lam(Patt::Var(ref n), _) if n == "x"));
    }

    #[test]
    fn parse_pair_expression() {
        let mut first = Resource::new_embedded();
        first.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:program:Var".to_string())]),
        );
        first.set(
            Iri::parse("urn:eigenius:program:name").unwrap(),
            Value::String("a".to_string()),
        );

        let mut second = Resource::new_embedded();
        second.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:program:Var".to_string())]),
        );
        second.set(
            Iri::parse("urn:eigenius:program:name").unwrap(),
            Value::String("b".to_string()),
        );

        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:program:Pair".to_string())]),
        );
        r.set(
            Iri::parse("urn:eigenius:program:first").unwrap(),
            Value::Embedded(Box::new(first)),
        );
        r.set(
            Iri::parse("urn:eigenius:program:second").unwrap(),
            Value::Embedded(Box::new(second)),
        );

        let layer = crate::layer::LayerBuilder::new("empty", None)
            .build(crate::layer::LayerStorage::in_memory());
        let exp = parse_expression(&r, &layer).unwrap();
        assert!(matches!(exp, Exp::Pair(_, _)));
    }

    // --- Constructor application resolution (Phase 11b step 9) ---

    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use std::sync::Arc;

    fn build_layer_with_esl(esl_source: &str) -> Arc<crate::layer::Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(crate::layer::LayerStorage::in_memory()));

        let user_resources = crate::esl::compile(esl_source).expect("ESL compile failed");
        let mut user_builder = LayerBuilder::new("user", Some(core));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        Arc::new(user_builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// Helper: parse a program by its IRI from a layer, return its body.
    fn parse_program_body(program_iri: &str, layer: &crate::layer::Layer) -> Exp {
        let iri = Iri::parse(program_iri).unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, _typ) = parse_program(&resource, layer).expect("parse_program");
        match term {
            Exp::Lam(_, body) => *body,
            other => panic!("expected Lam, got {other:?}"),
        }
    }

    /// Build a layer holding a published `RuntimeScript` + its
    /// `RuntimeEnvironment`, for the resolution tests below.
    fn runtime_layer() -> Arc<crate::layer::Layer> {
        let prop = |s: &str| Iri::parse(s).unwrap();
        let mut env = Resource::new(prop("urn:eigenius:test:env"));
        env.set(
            prop("urn:eigenius:runtime:image_digest"),
            Value::String("sha256:deadbeef".to_string()),
        );

        let mut script = Resource::new(prop("urn:eigenius:test:script"));
        script.set(
            prop("urn:eigenius:runtime:language"),
            Value::String("r".to_string()),
        );
        script.set(
            prop("urn:eigenius:runtime:source"),
            Value::String("print(1)\n".to_string()),
        );
        script.set(
            prop("urn:eigenius:runtime:requires_environment"),
            Value::ResourceRef(prop("urn:eigenius:test:env")),
        );

        let mut b = LayerBuilder::new("runtime", None);
        b.add_resource(env).unwrap();
        b.add_resource(script).unwrap();
        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn runtime_script_ref_expands_to_flat_dispatch_shape() {
        let layer = runtime_layer();
        let mut comp_arg = Resource::new_embedded();
        comp_arg.set(
            Iri::parse(RUNTIME_SCRIPT_REF).unwrap(),
            Value::ResourceRef(Iri::parse("urn:eigenius:test:script").unwrap()),
        );
        let expanded = expand_runtime_script_argument(&comp_arg, &layer).expect("resolves");
        let get = |p: &str| {
            expanded
                .get(&Iri::parse(p).unwrap())
                .and_then(|v| v.as_iri_str())
                .map(str::to_string)
        };
        assert_eq!(get(RUNTIME_LANGUAGE).as_deref(), Some("r"));
        assert_eq!(get(RUNTIME_SOURCE).as_deref(), Some("print(1)\n"));
        assert_eq!(
            get(RUNTIME_IMAGE_DIGEST).as_deref(),
            Some("sha256:deadbeef")
        );
        // The script reference is consumed, not echoed.
        assert!(get(RUNTIME_SCRIPT_REF).is_none());
    }

    #[test]
    fn inline_component_argument_passes_through_unchanged() {
        let layer = runtime_layer();
        let mut comp_arg = Resource::new_embedded();
        comp_arg.set(
            Iri::parse(RUNTIME_LANGUAGE).unwrap(),
            Value::String("r".to_string()),
        );
        comp_arg.set(
            Iri::parse(RUNTIME_SOURCE).unwrap(),
            Value::String("cat('hi')\n".to_string()),
        );
        let out = expand_runtime_script_argument(&comp_arg, &layer).expect("passthrough");
        assert_eq!(out, comp_arg);
    }

    #[test]
    fn unresolvable_script_ref_errors() {
        let layer = runtime_layer();
        let mut comp_arg = Resource::new_embedded();
        comp_arg.set(
            Iri::parse(RUNTIME_SCRIPT_REF).unwrap(),
            Value::ResourceRef(Iri::parse("urn:eigenius:test:missing").unwrap()),
        );
        let err = expand_runtime_script_argument(&comp_arg, &layer).unwrap_err();
        assert!(err.contains("not found on the chain"), "got: {err}");
    }

    #[test]
    fn nullary_constructor_resolves_to_inductive_ctor() {
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:zero_program : core:string -> ex:Nat {
                zero
            }
            "#,
        );
        let body = parse_program_body("urn:eigenius:example:zero_program", &layer);
        match body {
            Exp::InductiveCtor(decl, name, args) => {
                assert_eq!(decl.name, "Nat");
                assert_eq!(name, "zero");
                assert!(args.is_empty());
            }
            other => panic!("expected InductiveCtor(zero), got {other:?}"),
        }
    }

    #[test]
    fn unary_constructor_resolves_to_inductive_ctor_application() {
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:two : core:string -> ex:Nat {
                succ(succ(zero))
            }
            "#,
        );
        let body = parse_program_body("urn:eigenius:example:two", &layer);
        // Outer: succ(...)
        let (outer_decl, outer_name, mut outer_args) = match body {
            Exp::InductiveCtor(d, n, a) => (d, n, a),
            other => panic!("expected outer InductiveCtor, got {other:?}"),
        };
        assert_eq!(outer_decl.name, "Nat");
        assert_eq!(outer_name, "succ");
        assert_eq!(outer_args.len(), 1);
        // Middle: succ(zero)
        let (mid_decl, mid_name, mut mid_args) = match outer_args.remove(0) {
            Exp::InductiveCtor(d, n, a) => (d, n, a),
            other => panic!("expected middle InductiveCtor, got {other:?}"),
        };
        assert_eq!(mid_decl.name, "Nat");
        assert_eq!(mid_name, "succ");
        assert_eq!(mid_args.len(), 1);
        // Innermost: zero
        match mid_args.remove(0) {
            Exp::InductiveCtor(d, n, a) => {
                assert_eq!(d.name, "Nat");
                assert_eq!(n, "zero");
                assert!(a.is_empty());
            }
            other => panic!("expected zero InductiveCtor, got {other:?}"),
        }
    }

    #[test]
    fn constructor_program_type_checks_and_evaluates() {
        // End-to-end: ESL → resources → layer → parse_program → check → eval.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:two : core:string -> ex:Nat {
                succ(succ(zero))
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:two").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");

        // Type-check: term should have type `typ` in an empty context
        // with the layer available for class resolution.
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");

        // Evaluate by applying to a dummy string input.
        let input_val = crate::nbe::val::Val::Unit; // placeholder; type unused at runtime
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val.app(input_val).expect("apply program");

        // Result should be succ(succ(zero)) — InductiveVal nested twice.
        match result {
            crate::nbe::val::Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } => {
                assert_eq!(decl.name, "Nat");
                assert_eq!(ctor_name, "succ");
                assert_eq!(args.len(), 1);
                match &args[0] {
                    crate::nbe::val::Val::InductiveVal {
                        decl: d2,
                        ctor_name: n2,
                        args: a2,
                    } => {
                        assert_eq!(d2.name, "Nat");
                        assert_eq!(n2, "succ");
                        match &a2[0] {
                            crate::nbe::val::Val::InductiveVal {
                                decl: d3,
                                ctor_name: n3,
                                args: a3,
                            } => {
                                assert_eq!(d3.name, "Nat");
                                assert_eq!(n3, "zero");
                                assert!(a3.is_empty());
                            }
                            other => panic!("expected innermost zero, got {other:?}"),
                        }
                    }
                    other => panic!("expected middle succ, got {other:?}"),
                }
            }
            other => panic!("expected outer succ InductiveVal, got {other:?}"),
        }
    }

    #[test]
    fn binary_constructor_program_end_to_end() {
        // Phase 11b step 10: 2-arg constructors.
        // Build a non-parametric NatList; construct cons(zero, cons(succ(zero), nil)).
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            data ex:NatList {
                nil,
                cons(ex:Nat, ex:NatList),
            }

            program ex:two_elem_list : core:string -> ex:NatList {
                cons(zero, cons(succ(zero), nil))
            }
            "#,
        );

        let body = parse_program_body("urn:eigenius:example:two_elem_list", &layer);
        // Outer: cons(zero, cons(...)) — 2 args
        let (outer_decl, outer_name, outer_args) = match body {
            Exp::InductiveCtor(d, n, a) => (d, n, a),
            other => panic!("expected outer InductiveCtor, got {other:?}"),
        };
        assert_eq!(outer_decl.name, "NatList");
        assert_eq!(outer_name, "cons");
        assert_eq!(outer_args.len(), 2, "cons should have 2 args");
        // First arg: zero (Nat)
        match &outer_args[0] {
            Exp::InductiveCtor(d, n, a) => {
                assert_eq!(d.name, "Nat");
                assert_eq!(n, "zero");
                assert!(a.is_empty());
            }
            other => panic!("expected zero, got {other:?}"),
        }
        // Second arg: cons(succ(zero), nil)
        match &outer_args[1] {
            Exp::InductiveCtor(d, n, a) => {
                assert_eq!(d.name, "NatList");
                assert_eq!(n, "cons");
                assert_eq!(a.len(), 2);
            }
            other => panic!("expected nested cons, got {other:?}"),
        }

        // Type-check and evaluate end-to-end.
        let iri = Iri::parse("urn:eigenius:example:two_elem_list").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");

        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val.app(crate::nbe::val::Val::Unit).expect("apply");
        // Walk the resulting list: cons(zero, cons(succ(zero), nil))
        let (decl, ctor, args) = match result {
            crate::nbe::val::Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } => (decl, ctor_name, args),
            other => panic!("expected InductiveVal, got {other:?}"),
        };
        assert_eq!(decl.name, "NatList");
        assert_eq!(ctor, "cons");
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn ctor_arity_mismatch_in_application_is_rejected() {
        // Calling a 2-arg ctor with only 1 arg should fail at parse time
        // with a clear arity-mismatch error.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            data ex:NatList {
                nil,
                cons(ex:Nat, ex:NatList),
            }

            program ex:bad : core:string -> ex:NatList {
                cons(zero)
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:bad").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let err = parse_program(&resource, &layer).unwrap_err();
        assert!(
            err.contains("expects 2 args, got 1"),
            "expected arity error, got: {err}"
        );
    }

    #[test]
    fn ternary_constructor_program_end_to_end() {
        // Phase 11b step 10 (proper fix): 3-arg constructor end-to-end.
        // This was previously impossible because ESL `Apply` had only
        // 2 arg slots; the multi-arg refactor (`Apply.args: Vec<Expr>`)
        // makes 3+ arg ctor application a first-class operation.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            data ex:Triple {
                triple(ex:Nat, ex:Nat, ex:Nat),
            }

            program ex:my_triple : core:string -> ex:Triple {
                triple(zero, succ(zero), succ(succ(zero)))
            }
            "#,
        );

        let body = parse_program_body("urn:eigenius:example:my_triple", &layer);
        let (decl, name, args) = match body {
            Exp::InductiveCtor(d, n, a) => (d, n, a),
            other => panic!("expected InductiveCtor, got {other:?}"),
        };
        assert_eq!(decl.name, "Triple");
        assert_eq!(name, "triple");
        assert_eq!(args.len(), 3, "all 3 args preserved end to end");

        // Type-check + evaluate.
        let iri = Iri::parse("urn:eigenius:example:my_triple").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val
            .app(crate::nbe::val::Val::Unit)
            .expect("apply program");
        match result {
            crate::nbe::val::Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } => {
                assert_eq!(decl.name, "Triple");
                assert_eq!(ctor_name, "triple");
                assert_eq!(args.len(), 3);
            }
            other => panic!("expected Triple InductiveVal, got {other:?}"),
        }
    }

    #[test]
    fn non_ctor_three_arg_call_is_compile_error() {
        // Non-ctor 3+ args is now a *compile* error (with role-aware
        // diagnostic), not a parser error. Previously this was a
        // silent corruption (the 3rd arg was dropped).
        let result = crate::esl::compile(
            r#"
            namespace ex = "urn:eigenius:example";

            program ex:bad : ex:Foo -> ex:Bar {
                f(a, b, c)
            }
            "#,
        );
        let errs = result.unwrap_err();
        let msg = &errs[0].message;
        assert!(
            msg.contains("3 positional arguments"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("only defined for declared inductive constructors"),
            "diagnostic should explain the rule: {msg}"
        );
    }

    #[test]
    fn non_ctor_two_arg_legacy_sugar_still_works() {
        // Backward compat: `f(a, b)` for a non-ctor function still
        // means "input + component_argument" (legacy sugar for the
        // block form). Existing ESL programs continue to work.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:demo : core:string -> core:string {
                Identity(input, "config")
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:demo").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let body = match resource
            .get(&Iri::parse("urn:eigenius:program:body").unwrap())
            .expect("program body")
        {
            crate::ontology::resource::Value::Embedded(b) => b.as_ref(),
            other => panic!("body must be embedded, got {other:?}"),
        };
        // The Apply resource should carry both `argument` and
        // `component_argument` (sugar form preserved).
        let is_a = body.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:program:Apply");
        assert!(body
            .get(&Iri::parse("urn:eigenius:program:component_argument").unwrap())
            .is_some());
    }

    // --- Match expressions (Phase 11b step 11) ---

    #[test]
    fn match_on_nat_simple_zero_case_evaluates() {
        // Match `zero` returning Nat: zero -> succ(zero), succ(_) -> zero.
        // For input zero: should return succ(zero).
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:flip : core:string -> ex:Nat {
                match zero returning ex:Nat {
                    zero -> succ(zero);
                    succ(_) -> zero;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:flip").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val
            .app(crate::nbe::val::Val::Unit)
            .expect("apply program");
        // Result: succ(zero)
        match result {
            crate::nbe::val::Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } => {
                assert_eq!(decl.name, "Nat");
                assert_eq!(ctor_name, "succ");
                assert_eq!(args.len(), 1);
                match &args[0] {
                    crate::nbe::val::Val::InductiveVal { ctor_name, .. } => {
                        assert_eq!(ctor_name, "zero");
                    }
                    other => panic!("expected zero arg, got {other:?}"),
                }
            }
            other => panic!("expected succ InductiveVal, got {other:?}"),
        }
    }

    #[test]
    fn match_on_nat_succ_case_binds_predecessor() {
        // Match `succ(zero)` returning Nat: zero -> zero, succ(n) -> n.
        // For input succ(zero): the n binding is zero, so result is zero.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:pred_of_one : core:string -> ex:Nat {
                match succ(zero) returning ex:Nat {
                    zero -> zero;
                    succ(n) -> n;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:pred_of_one").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val
            .app(crate::nbe::val::Val::Unit)
            .expect("apply program");
        match result {
            crate::nbe::val::Val::InductiveVal { ctor_name, .. } => {
                assert_eq!(ctor_name, "zero", "succ(zero) → pred → zero");
            }
            other => panic!("expected zero, got {other:?}"),
        }
    }

    #[test]
    fn match_on_natlist_with_multi_arg_pattern() {
        // Match a 2-element list returning Nat:
        //   nil -> zero
        //   cons(x, _) -> x
        // For cons(succ(zero), nil): should bind x = succ(zero), return succ(zero).
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            data ex:NatList {
                nil,
                cons(ex:Nat, ex:NatList),
            }

            program ex:head_or_zero : core:string -> ex:Nat {
                match cons(succ(zero), nil) returning ex:Nat {
                    nil -> zero;
                    cons(x, _) -> x;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:head_or_zero").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val
            .app(crate::nbe::val::Val::Unit)
            .expect("apply program");
        // Result: succ(zero)
        match result {
            crate::nbe::val::Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } => {
                assert_eq!(decl.name, "Nat");
                assert_eq!(ctor_name, "succ");
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected succ(zero), got {other:?}"),
        }
    }

    #[test]
    fn match_non_exhaustive_is_rejected() {
        // Missing the `succ` case → parse_program should fail.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:bad : core:string -> ex:Nat {
                match zero returning ex:Nat {
                    zero -> zero;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:bad").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let err = parse_program(&resource, &layer).unwrap_err();
        assert!(
            err.contains("non-exhaustive"),
            "expected non-exhaustive error, got: {err}"
        );
    }

    #[test]
    fn match_arm_binding_count_mismatch_is_rejected() {
        // `cons(x)` for 2-arg cons should error.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            data ex:NatList {
                nil,
                cons(ex:Nat, ex:NatList),
            }

            program ex:bad : core:string -> ex:Nat {
                match nil returning ex:Nat {
                    nil -> zero;
                    cons(x) -> x;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:bad").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let err = parse_program(&resource, &layer).unwrap_err();
        assert!(
            err.contains("expects 2 bindings"),
            "expected binding count error, got: {err}"
        );
    }

    // --- Inferred-motive match (Phase 11b step 12, D19 §10) ---

    #[test]
    fn match_without_returning_annotation_uses_exp_match() {
        // No `returning T`: parse_match should produce Exp::Match
        // (motive-deferred), not Exp::InductiveRec.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:flip : core:string -> ex:Nat {
                match zero {
                    zero -> succ(zero);
                    succ(_) -> zero;
                }
            }
            "#,
        );
        let body = parse_program_body("urn:eigenius:example:flip", &layer);
        match body {
            Exp::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                assert_eq!(arms[0].ctor_name, "zero");
                assert_eq!(arms[1].ctor_name, "succ");
            }
            other => panic!("expected Exp::Match (no annotation), got {other:?}"),
        }
    }

    #[test]
    fn match_without_annotation_in_checking_mode_evaluates() {
        // End-to-end: motive inferred from program return type.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:flip : core:string -> ex:Nat {
                match zero {
                    zero -> succ(zero);
                    succ(_) -> zero;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:flip").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check (motive inferred)");
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val
            .app(crate::nbe::val::Val::Unit)
            .expect("apply program");
        // Result should be succ(zero).
        match result {
            crate::nbe::val::Val::InductiveVal {
                ctor_name, args, ..
            } => {
                assert_eq!(ctor_name, "succ");
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected succ InductiveVal, got {other:?}"),
        }
    }

    #[test]
    fn match_with_bindings_and_inferred_motive_works() {
        // succ(n) -> n with motive inferred from program return type.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:pred_of_one : core:string -> ex:Nat {
                match succ(zero) {
                    zero -> zero;
                    succ(n) -> n;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:pred_of_one").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        check(&mut ctx, &term, &typ_val).expect("type check");
        let prog_val = eval(&term, &Rho::Nil).expect("eval program");
        let result = prog_val
            .app(crate::nbe::val::Val::Unit)
            .expect("apply program");
        match result {
            crate::nbe::val::Val::InductiveVal { ctor_name, .. } => {
                assert_eq!(ctor_name, "zero");
            }
            other => panic!("expected zero InductiveVal, got {other:?}"),
        }
    }

    #[test]
    fn match_inferred_non_exhaustive_is_rejected_at_check_time() {
        // Missing arm should fail at type-check time (since the
        // exhaustiveness check moves into the type checker for the
        // motive-deferred path).
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:bad : core:string -> ex:Nat {
                match zero {
                    zero -> zero;
                }
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:bad").unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, typ) = parse_program(&resource, &layer).expect("parse_program");
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        let typ_val = eval(&typ, &Rho::Nil).expect("eval type");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        let err = check(&mut ctx, &term, &typ_val).unwrap_err().to_string();
        assert!(
            err.contains("non-exhaustive"),
            "expected non-exhaustive error, got: {err}"
        );
    }

    // --- ESL surface for institution capabilities ---

    use crate::institution::registry::InstitutionIndex;
    use crate::ontology::well_known as wk;

    const CAP_INSTITUTION_IRI: &str = "urn:eigenius:test:cap_inst";
    const CAP_COMORPHISM_IRI: &str = "urn:eigenius:test:cap_comorphism";
    const CAP_DECIDE_IRI: &str = "urn:eigenius:test:cap_decide";

    /// Build an InstitutionIndex carrying one Comorphism and one
    /// Decidable QueryClass — the two declaration shapes the ESL
    /// classifier needs to specialize call-site emission against.
    fn cap_index() -> Arc<InstitutionIndex> {
        let mut b = LayerBuilder::new("cap_test", None);

        // Institution declaration.
        let mut inst = Resource::new(Iri::parse(CAP_INSTITUTION_IRI).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:Institution".to_string(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(CAP_INSTITUTION_IRI.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("CapInstitution".to_string()),
        );
        b.add_resource(inst).unwrap();

        // Comorphism — minimal but well-formed.
        let mut cm = Resource::new(Iri::parse(CAP_COMORPHISM_IRI).unwrap());
        cm.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::COMORPHISM.to_string())]),
        );
        cm.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            Value::String("urn:eigenius:test:cap_export".to_string()),
        );
        cm.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            Value::String("urn:eigenius:test:cap_transform".to_string()),
        );
        cm.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            Value::String("urn:eigenius:test:cap_import".to_string()),
        );
        cm.set(Iri::parse(wk::EXACT).unwrap(), Value::Boolean(true));
        b.add_resource(cm).unwrap();

        // Decidable QueryClass — its @id is the IRI source code calls.
        let mut qc = Resource::new(Iri::parse(CAP_DECIDE_IRI).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            Value::String("urn:eigenius:test:CapInput".to_string()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            Value::String(wk::VERDICT.to_string()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            Value::String("urn:eigenius:test:cap_decide_handler".to_string()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(CAP_INSTITUTION_IRI.to_string()),
        );
        b.add_resource(qc).unwrap();

        let layer = b.build(crate::layer::LayerStorage::in_memory());
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "fixture index errors: {errors:?}");
        Arc::new(idx)
    }

    #[test]
    fn esl_comorphism_invoke_compiles_and_decodes() {
        // ESL source invoking the comorphism via component-call
        // syntax: `cap:cap_comorphism(source)`.
        let idx = cap_index();

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(crate::layer::LayerStorage::in_memory()));

        let source = r#"
            namespace core = "urn:eigenius:core";
            namespace cap = "urn:eigenius:test";
            namespace ex = "urn:eigenius:example";

            class ex:Thing {
                requires ex:name;
            }
            property ex:name : core:string {
                description = "test";
            }

            program ex:invoke_program : ex:Thing -> ex:Thing {
                cap:cap_comorphism(input)
            }
        "#;
        let user_resources =
            crate::esl::compile_with_institutions(source, idx.clone()).expect("compile");
        let mut user_builder = LayerBuilder::new("user", Some(core));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(user_builder.build(crate::layer::LayerStorage::in_memory()));

        let prog_iri = Iri::parse("urn:eigenius:example:invoke_program").unwrap();
        let prog_resource = layer.resolve(&prog_iri).expect("program");
        let (term, _ty) = parse_program(&prog_resource, &layer).expect("parse");
        // The compiled term should end up as Lam(input, InstitutionInvoke(..)).
        match term {
            Exp::Lam(_, body) => match *body {
                Exp::InstitutionInvoke { comorphism_iri, .. } => {
                    assert_eq!(comorphism_iri.as_str(), CAP_COMORPHISM_IRI);
                }
                other => panic!("expected InstitutionInvoke body, got {other:?}"),
            },
            other => panic!("expected Lam program body, got {other:?}"),
        }
    }

    #[test]
    fn esl_decide_predicate_compiles_and_decodes() {
        // ESL source invoking the decide predicate with N args:
        // `cap:cap_decide(input, input)` — args are positional.
        let idx = cap_index();

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(crate::layer::LayerStorage::in_memory()));

        let source = r#"
            namespace core = "urn:eigenius:core";
            namespace cap = "urn:eigenius:test";
            namespace ex = "urn:eigenius:example";

            class ex:Thing {
                requires ex:name;
            }
            property ex:name : core:string {
                description = "test";
            }

            program ex:decide_program : ex:Thing -> ex:Thing {
                cap:cap_decide(input, input)
            }
        "#;
        let user_resources =
            crate::esl::compile_with_institutions(source, idx.clone()).expect("compile");
        let mut user_builder = LayerBuilder::new("user", Some(core));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(user_builder.build(crate::layer::LayerStorage::in_memory()));

        let prog_iri = Iri::parse("urn:eigenius:example:decide_program").unwrap();
        let prog_resource = layer.resolve(&prog_iri).expect("program");
        let (term, _ty) = parse_program(&prog_resource, &layer).expect("parse");
        // The compiled term should end up as Lam(input, NativeDecide(Institution, Unit)).
        match term {
            Exp::Lam(_, body) => match *body {
                Exp::NativeDecide(
                    crate::nbe::term::Constraint::Institution { iri, args },
                    _value,
                ) => {
                    assert_eq!(iri.as_str(), CAP_DECIDE_IRI);
                    assert_eq!(args.len(), 2);
                }
                other => panic!("expected NativeDecide(Institution) body, got {other:?}"),
            },
            other => panic!("expected Lam program body, got {other:?}"),
        }
    }

    #[test]
    fn esl_comorphism_with_wrong_arity_errors() {
        // A comorphism takes exactly 1 source arg. Two positional
        // args (which would be legal for a component under the
        // legacy arity sugar) must error.
        let idx = cap_index();

        let source = r#"
            namespace core = "urn:eigenius:core";
            namespace cap = "urn:eigenius:test";
            namespace ex = "urn:eigenius:example";

            class ex:Thing { requires ex:name; }
            property ex:name : core:string { description = "test"; }

            program ex:bad_invoke : ex:Thing -> ex:Thing {
                cap:cap_comorphism(input, input)
            }
        "#;
        let result = crate::esl::compile_with_institutions(source, idx);
        let errors = result.unwrap_err();
        assert!(
            errors.iter().any(|e| e.to_string().contains("comorphism"))
                || errors
                    .iter()
                    .any(|e| e.to_string().contains("exactly 1 source argument"))
        );
    }

    #[test]
    fn esl_without_institutions_treats_iri_as_plain_component() {
        // Without the institution registry, the same ESL source
        // compiles as a plain component dispatch (existing behavior) —
        // no classification happens and the IRI is treated opaquely.
        let source = r#"
            namespace core = "urn:eigenius:core";
            namespace cap = "urn:eigenius:test";
            namespace ex = "urn:eigenius:example";

            class ex:Thing { requires ex:name; }
            property ex:name : core:string { description = "test"; }

            program ex:plain_app : ex:Thing -> ex:Thing {
                cap:cap_comorphism(input)
            }
        "#;
        let user_resources = crate::esl::compile(source).expect("compile");
        // Find the program and check its body is plain Apply (not
        // ComorphismInvokeApply).
        let prog_res = user_resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:example:plain_app"))
            .expect("program resource");
        let body = prog_res
            .get(&Iri::parse("urn:eigenius:program:body").unwrap())
            .expect("body");
        let body_r = match body {
            Value::Embedded(r) => r.as_ref(),
            other => panic!("expected embedded body, got {other:?}"),
        };
        let is_a = body_r.is_a();
        assert_eq!(
            is_a.first().map(|i| i.as_str()),
            Some("urn:eigenius:program:Apply"),
            "without institutions the body should be plain Apply"
        );
    }
}
