//! Parse Eigon-JSON expression resources into Mini-TT terms.
//!
//! Each expression form (Let, Apply, Var, Lambda, etc.) maps 1:1
//! to a Mini-TT term. No translation layer needed.

use crate::layer::Layer;
use crate::nbe::term::{Branch, Decl, Exp, InductiveDecl, Patt, PrimitiveType};
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::ground::{is_inductive_type, resolve_class_type, resolve_inductive_type};
use std::sync::Arc;
const LET: &str = "urn:eigenius:program:Let";
const APPLY: &str = "urn:eigenius:program:Apply";
const VAR: &str = "urn:eigenius:program:Var";
const LAMBDA: &str = "urn:eigenius:program:Lambda";
const CASE: &str = "urn:eigenius:program:Case";
const PAIR: &str = "urn:eigenius:program:Pair";
const CONSTRUCT: &str = "urn:eigenius:program:Construct";
const PROJECT: &str = "urn:eigenius:program:Project";
const MAP: &str = "urn:eigenius:program:Map";
const REDUCE: &str = "urn:eigenius:program:Reduce";
const LITERAL: &str = "urn:eigenius:program:Literal";
const CORECORD: &str = "urn:eigenius:program:CoRecord";

/// Parse a Program resource into a Mini-TT term with its type.
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

/// Parse an expression resource into a Mini-TT term.
pub fn parse_expression(resource: &Resource, layer: &Layer) -> Result<Exp, String> {
    let is_a = resource.is_a();
    let class_str = is_a.first().map(|i| i.as_str()).unwrap_or("");

    match class_str {
        LET => parse_let(resource, layer),
        APPLY => parse_apply(resource, layer),
        VAR => parse_var(resource, layer),
        LAMBDA => parse_lambda(resource, layer),
        CASE => parse_case(resource, layer),
        PAIR => parse_pair(resource, layer),
        CONSTRUCT => parse_construct(resource, layer),
        PROJECT => parse_project(resource, layer),
        MAP => parse_map(resource, layer),
        REDUCE => parse_reduce(resource, layer),
        LITERAL => parse_literal(resource),
        CORECORD => parse_corecord(resource, layer),
        _ => Err(format!("unknown expression class: '{class_str}'")),
    }
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
    let resource = layer.resolve(&parent_iri)?;
    if !is_inductive_type(resource) {
        return None;
    }
    let val = resolve_inductive_type(&parent_iri, resource).ok()?;
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
    // Function can be a string (component IRI) or embedded expression
    let func_prop = Iri::parse("urn:eigenius:program:function").unwrap();

    // Constructor application special case (Phase 11b step 9): when the
    // function string is a ctor IRI of the form `<parent>:<ctor>`,
    // emit `Exp::InductiveCtor` directly. IRI-keyed lookup — no
    // layer-wide name search.
    //
    // The component_argument slot is ignored on this path. Phase 11b
    // restricts ESL constructor application to single-argument ctors;
    // multi-arg ctors need either a different surface form or pair-
    // decomposition logic (deferred).
    if let Some(Value::String(s)) = resource.get(&func_prop) {
        if let Some((decl, idx, arity)) = resolve_ctor_iri(s, layer) {
            if arity == 1 {
                let arg_prop = Iri::parse("urn:eigenius:program:argument").unwrap();
                let arg_exp = match resource.get(&arg_prop) {
                    Some(Value::Embedded(r)) => parse_expression(r, layer)?,
                    Some(Value::String(s)) => Exp::Var(s.clone()),
                    _ => {
                        return Err(format!(
                            "constructor `{}.{}` application requires an argument",
                            decl.name, decl.ctors[idx].name
                        ))
                    }
                };
                let ctor_name = decl.ctors[idx].name.clone();
                return Ok(Exp::InductiveCtor(decl, ctor_name, vec![arg_exp]));
            }
        }
    }

    let func_exp = match resource.get(&func_prop) {
        Some(Value::String(s)) => {
            // Component reference — treat as a variable
            Exp::Var(s.clone())
        }
        Some(Value::Embedded(r)) => parse_expression(r, layer)?,
        _ => return Err("Apply: missing 'function' property".to_string()),
    };

    let arg_prop = Iri::parse("urn:eigenius:program:argument").unwrap();
    let arg_exp = match resource.get(&arg_prop) {
        Some(Value::String(s)) => {
            // Literal string or resource reference
            Exp::Var(s.clone())
        }
        Some(Value::Embedded(r)) => parse_expression(r, layer)?,
        _ => Exp::Unit, // No argument
    };

    // Check for component_argument (static config for IO components)
    let comp_arg_prop = Iri::parse("urn:eigenius:program:component_argument").unwrap();
    let effective_arg = match resource.get(&comp_arg_prop) {
        Some(Value::Embedded(comp_arg)) => {
            // Pack as Pair(arg, EigonResource(comp_arg)) so the dispatcher can extract it
            Exp::Pair(
                Box::new(arg_exp),
                Box::new(Exp::EigonResource(comp_arg.clone())),
            )
        }
        _ => arg_exp,
    };

    Ok(Exp::App(Box::new(func_exp), Box::new(effective_arg)))
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
        Some(Value::String(s)) => {
            // Check if it's an IRI reference
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
    let s = get_string(resource, prop)?;
    Iri::parse(&s).map_err(|e| format!("invalid IRI in '{prop}': {e}"))
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
        let layer = crate::layer::LayerBuilder::new("empty", None).build();
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
        let layer = crate::layer::LayerBuilder::new("empty", None).build();
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

        let layer = crate::layer::LayerBuilder::new("empty", None).build();
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

        let layer = crate::layer::LayerBuilder::new("empty", None).build();
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
        let core = Arc::new(core_builder.build());

        let user_resources = crate::esl::compile(esl_source).expect("ESL compile failed");
        let mut user_builder = LayerBuilder::new("user", Some(core));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        Arc::new(user_builder.build())
    }

    /// Helper: parse a program by its IRI from a layer, return its body.
    fn parse_program_body(program_iri: &str, layer: &crate::layer::Layer) -> Exp {
        let iri = Iri::parse(program_iri).unwrap();
        let resource = layer.resolve(&iri).expect("program resource");
        let (term, _typ) = parse_program(resource, layer).expect("parse_program");
        match term {
            Exp::Lam(_, body) => *body,
            other => panic!("expected Lam, got {other:?}"),
        }
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
        let (term, typ) = parse_program(resource, &layer).expect("parse_program");

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
}
