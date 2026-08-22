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

//! `extract_typed` machinery for the Reasoning institution.
//!
//! `extract_typed` is the kernel's standard "lift a chain
//! resource into a typed `Val`" abstraction; every institution that
//! exposes its data to the kernel's term language goes through it.
//! The Reasoning institution's job in this file is to translate a
//! `JustificationTerm` chain-resident value (D32 §3.7 tagged-dict
//! shape on the `reasoning:justification` property) into a kernel
//! `Val::InductiveVal` typed at `reasoning:JustificationTerm`.
//!
//! Why this lives in the institution crate, not in the kernel:
//!
//! - D32 §3.7 specifies the *wire format* for inductive values, but
//!   not how to lift them into kernel `Val`. Numerical institutions
//!   (Symbolics, Catalyst, …) reify D32-shape values into their own
//!   runtime's representation (Julia structs) at the institution
//!   boundary; they never go through kernel `Val`. The Reasoning
//!   institution is different because its "runtime" *is* the kernel's
//!   NbE checker — there's no external worker to reify into, and the
//!   validate handler needs a `Val` to construct
//!   `JustifiedBy(justification, proposition)` for type-checking.
//! - Routing the lift through `extract_typed` (rather than a free
//!   function in the kernel) keeps the kernel surface scoped to
//!   abstractions it has specs for. The "chain inductive value → Val"
//!   bridge is Reasoning-institution-specific machinery; it belongs
//!   here.
//!
//! The lift goes through `Exp` as an intermediate: chain JSON →
//! `Exp::InductiveCtor` (a syntactic ctor application) → `Val` via
//! [`eigenius_kernel::nbe::eval::eval`]. The Exp step lets the kernel's
//! existing inductive machinery (positivity, recursor, etc.) see the
//! value uniformly with everything else it manipulates.

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, PrimitiveType};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::ground::resolve_class_type;

use crate::institution::iris;

/// `extract_typed` handler for `proc:extract_justification`.
///
/// Reads the `justification` property off the supplied
/// `ReasoningSentence` resource, lifts the chain-resident inductive
/// value into a `Val::InductiveVal` typed at `JustificationTerm`.
pub fn extract_justification(
    sentence: &Resource,
    ctx: &ExecutionContext,
) -> Result<Val, InstitutionError> {
    let exp = justification_exp(sentence, ctx)?;
    eval(&exp, &Rho::Nil).map_err(|e| {
        InstitutionError::ComputationFailed(format!("failed to evaluate justification: {e:?}"))
    })
}

/// The same decode, stopped one step earlier: the SYNTACTIC `Exp::InductiveCtor` tree.
///
/// [`extract_justification`] evaluates this into a `Val` because the validate handler needs a value
/// to build `JustifiedBy(j, p)` from. The projections of D73 §1.2 want the opposite — they walk the
/// constructor application itself, since a `JustificationTerm` IS its tree and evaluating it only
/// obscures the shape (eigenius#204).
pub fn justification_exp(
    sentence: &Resource,
    ctx: &ExecutionContext,
) -> Result<Exp, InstitutionError> {
    let value = sentence
        .get(&Iri::parse(iris::PROP_JUSTIFICATION).expect("static IRI"))
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "ReasoningSentence missing required `justification` property".to_string(),
            )
        })?;

    let jt_iri = Iri::parse(iris::JUSTIFICATION_TERM).expect("static IRI");
    let jt_decl = match resolve_class_type(&jt_iri, ctx.head()) {
        Ok(Val::InductiveType { decl, .. }) => decl,
        Ok(other) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "`{}` resolved to a non-inductive value: {other:?}",
                iris::JUSTIFICATION_TERM
            )));
        }
        Err(e) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "failed to resolve JustificationTerm inductive: {e}"
            )));
        }
    };

    chain_value_to_exp(value, &jt_decl)
        .map_err(|e| InstitutionError::ComputationFailed(format!("malformed justification: {e}")))
}

/// Decode a D32 §3.7-shaped chain inductive value into the kernel's
/// `Exp::InductiveCtor` form.
///
/// Every argument is decoded **against the declared domain type in the
/// chosen constructor's Π-telescope** ([`InductiveCtorDecl::typ`]), not
/// against the JSON shape it happens to have. Both paths that build a
/// ctor telescope — `build_ctor_type` from `core:arg_types` and the D47
/// codec from `core:ctor_type` — fold primitive class IRIs to
/// [`Exp::EigonPrimitive`], so the mapping is:
///
/// | declared domain type              | accepted JSON | `Exp`               |
/// |-----------------------------------|---------------|---------------------|
/// | `EigonPrimitive(String)`          | string        | [`Exp::LitString`]  |
/// | `EigonPrimitive(Integer)`         | i64 integer   | [`Exp::LitInt`]     |
/// | `EigonPrimitive(Float)`           | any number    | [`Exp::LitFloat`]   |
/// | `EigonPrimitive(Boolean)`         | bool          | [`Exp::LitBool`]    |
/// | `InductiveType(self, _)`          | object        | recursive ctor      |
///
/// `Float` accepting a JSON integer matches the kernel validator's
/// per-arg check (`walk_inductive_value` uses `is_number` for
/// `core:float`, `is_i64` for `core:integer`); the two must agree or a
/// value that commits would fail to decode.
///
/// A JSON shape that disagrees with its declared type is
/// [`ChainDecodeError::ArgTypeMismatch`], naming both — never a
/// coercion.
///
/// There is **no fallback arm**. A declared type outside the table is
/// [`ChainDecodeError::UnsupportedArgType`], naming it — guessing from
/// the JSON shape is the defect this decoder exists to remove, and a
/// fallback would reinstate it exactly when the ontology has moved
/// somewhere the mapping has not followed. The cases that reach it
/// today:
///
/// - An argument declared at *another* inductive. The recursion has only
///   the outer `Arc<InductiveDecl>` in hand and no layer to resolve a
///   sibling declaration from; `resolve_inductive_type` hands back
///   ctor-internal inductive references as name-only stubs with empty
///   `ctors`, so decoding into one would have nothing to validate the
///   inner ctor name against. `JustificationTerm` — the only inductive
///   this decoder serves (see [`extract_justification`]) — is
///   homogeneous. See gh #74.
/// - An argument declared at `core:json` or at a plain `core:Class`:
///   the kernel's `Exp` has no literal for either.
/// - An argument whose domain is a *type parameter* of the inductive
///   rather than a ground type. A D32 §3.7 chain value records no
///   instantiation for the parameter, so there is no shape to check.
fn chain_value_to_exp(value: &Value, decl: &Arc<InductiveDecl>) -> Result<Exp, ChainDecodeError> {
    let json = match value {
        Value::Json(j) => j,
        other => {
            return Err(ChainDecodeError::NotJson(format!("{other:?}")));
        }
    };
    decode_json(json, decl, "<root>")
}

#[derive(Debug, Clone, PartialEq)]
enum ChainDecodeError {
    NotJson(String),
    NotObject(String),
    MissingCtor(String),
    MissingArgs(String),
    /// The declared domain type is one this decoder has no `Exp`
    /// literal for (`core:json`, a plain class, another inductive).
    UnsupportedArgType {
        path: String,
        declared: String,
        details: String,
    },
    /// The JSON shape disagrees with the declared domain type.
    ArgTypeMismatch {
        path: String,
        declared: String,
        found: String,
    },
    /// The `args` array length disagrees with the ctor's telescope.
    ArityMismatch {
        path: String,
        ctor_name: String,
        expected: usize,
        got: usize,
    },
    /// The ctor's `typ` is not the Π-telescope shape the decoder needs.
    MalformedCtorType {
        decl_name: String,
        ctor_name: String,
        details: String,
    },
    UnknownCtor {
        decl_name: String,
        ctor_name: String,
        available: Vec<String>,
    },
}

impl std::fmt::Display for ChainDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(s) => write!(f, "expected Value::Json, got {s}"),
            Self::NotObject(p) => write!(f, "{p}: expected JSON object"),
            Self::MissingCtor(p) => write!(f, "{p}: missing string `ctor` field"),
            Self::MissingArgs(p) => write!(f, "{p}: missing array `args` field"),
            Self::UnsupportedArgType {
                path,
                declared,
                details,
            } => write!(
                f,
                "{path}: argument is declared `{declared}`, which this decoder does not \
                 support: {details}"
            ),
            Self::ArgTypeMismatch {
                path,
                declared,
                found,
            } => write!(
                f,
                "{path}: argument is declared `{declared}` but the value is {found}"
            ),
            Self::ArityMismatch {
                path,
                ctor_name,
                expected,
                got,
            } => write!(
                f,
                "{path}: ctor `{ctor_name}` declares {expected} argument(s), got {got}"
            ),
            Self::MalformedCtorType {
                decl_name,
                ctor_name,
                details,
            } => write!(
                f,
                "ctor `{ctor_name}` of inductive `{decl_name}` has a malformed \
                 constructor type: {details}"
            ),
            Self::UnknownCtor {
                decl_name,
                ctor_name,
                available,
            } => write!(
                f,
                "ctor `{ctor_name}` not declared on inductive `{decl_name}`; \
                 available ctors: {available:?}"
            ),
        }
    }
}

fn decode_json(
    json: &serde_json::Value,
    decl: &Arc<InductiveDecl>,
    path: &str,
) -> Result<Exp, ChainDecodeError> {
    let obj = json
        .as_object()
        .ok_or_else(|| ChainDecodeError::NotObject(path.to_string()))?;
    let ctor_name = obj
        .get("ctor")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ChainDecodeError::MissingCtor(path.to_string()))?;
    let args = obj
        .get("args")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ChainDecodeError::MissingArgs(path.to_string()))?;

    // Last line of defense before the kernel type-checker — a clear
    // available-list diagnostic beats letting the type-checker crash
    // on a malformed `InductiveCtor`. The kernel validator's
    // `walk_inductive_value` (Rule 16's walk, reached for
    // `reasoning:justification` through the `class_types` rule, since
    // that property's `data_type` is `core:resource`) catches an
    // unknown ctor, a wrong arity and a mistyped argument at commit;
    // the handler is dispatched after commit and may run against a
    // sentence committed before those checks existed.
    let ctor = decl
        .ctors
        .iter()
        .find(|c| c.name == ctor_name)
        .ok_or_else(|| ChainDecodeError::UnknownCtor {
            decl_name: decl.name.clone(),
            ctor_name: ctor_name.to_string(),
            available: decl.ctors.iter().map(|c| c.name.clone()).collect(),
        })?;

    let declared = ctor_arg_types(decl, ctor)?;
    if declared.len() != args.len() {
        return Err(ChainDecodeError::ArityMismatch {
            path: path.to_string(),
            ctor_name: ctor_name.to_string(),
            expected: declared.len(),
            got: args.len(),
        });
    }

    let decoded_args: Result<Vec<Exp>, ChainDecodeError> = args
        .iter()
        .zip(declared)
        .enumerate()
        .map(|(i, (a, ty))| decode_arg(a, ty, decl, &format!("{path}.args[{i}]")))
        .collect();

    Ok(Exp::InductiveCtor(
        decl.clone(),
        ctor_name.to_string(),
        decoded_args?,
    ))
}

/// The declared domain types of a constructor's value arguments, in
/// order.
///
/// `InductiveCtorDecl::typ` is `Π params. Π args. Self(params)` — the
/// parameter binders come first and are not supplied by the chain
/// value's `args` array (the kernel validator pairs `args` with
/// `core:arg_types`, which likewise excludes parameters), so strip
/// `decl.params.len()` binders before collecting the argument domains.
///
/// Same peel as the kernel type-checker's `peel_ctor_telescope`
/// (`kernel/src/nbe/check/inductive.rs`), which is private to that
/// module. The one difference is `Exp::SizedPi`: the checker collects
/// size binders as arguments, and this decoder rejects them, because
/// they occupy no slot in the chain value's `args` array and skipping
/// one would misalign every argument after it.
fn ctor_arg_types<'a>(
    decl: &'a InductiveDecl,
    ctor: &'a InductiveCtorDecl,
) -> Result<Vec<&'a Exp>, ChainDecodeError> {
    let malformed = |details: String| ChainDecodeError::MalformedCtorType {
        decl_name: decl.name.clone(),
        ctor_name: ctor.name.clone(),
        details,
    };

    let mut cursor = &ctor.typ;
    for i in 0..decl.params.len() {
        match cursor {
            Exp::Pi(_, _, body) => cursor = body,
            other => {
                return Err(malformed(format!(
                    "expected a Π binder for parameter {i} of {}, got {other:?}",
                    decl.params.len()
                )));
            }
        }
    }

    let mut domains = Vec::new();
    loop {
        match cursor {
            Exp::Pi(_, dom, body) => {
                domains.push(dom.as_ref());
                cursor = body;
            }
            // A `Size` binder is a type-level argument with no slot in
            // the chain value's `args` array. Silently skipping it
            // would misalign every following argument, so report.
            Exp::SizedPi { patt, .. } => {
                return Err(malformed(format!(
                    "sized binder `{patt:?}` in the argument telescope is not \
                     representable in a D32 §3.7 chain value"
                )));
            }
            _ => break,
        }
    }
    Ok(domains)
}

fn decode_arg(
    json: &serde_json::Value,
    declared: &Exp,
    decl: &Arc<InductiveDecl>,
    path: &str,
) -> Result<Exp, ChainDecodeError> {
    let mismatch = || ChainDecodeError::ArgTypeMismatch {
        path: path.to_string(),
        declared: describe_type(declared),
        found: describe_json(json),
    };

    match declared {
        Exp::EigonPrimitive(PrimitiveType::String) => json
            .as_str()
            .map(|s| Exp::LitString(s.to_string()))
            .ok_or_else(mismatch),
        Exp::EigonPrimitive(PrimitiveType::Integer) => {
            json.as_i64().map(Exp::LitInt).ok_or_else(mismatch)
        }
        // Any JSON number: `core:float` slots accept integer-valued
        // literals, matching the validator's `is_number` check.
        Exp::EigonPrimitive(PrimitiveType::Float) => {
            json.as_f64().map(Exp::LitFloat).ok_or_else(mismatch)
        }
        Exp::EigonPrimitive(PrimitiveType::Boolean) => {
            json.as_bool().map(Exp::LitBool).ok_or_else(mismatch)
        }
        Exp::EigonPrimitive(PrimitiveType::Json) => Err(ChainDecodeError::UnsupportedArgType {
            path: path.to_string(),
            declared: describe_type(declared),
            details: "the kernel term language has no literal for opaque JSON".to_string(),
        }),
        Exp::InductiveType(arg_decl, _) if arg_decl.iri == decl.iri => {
            if !json.is_object() {
                return Err(mismatch());
            }
            decode_json(json, decl, path)
        }
        Exp::InductiveType(arg_decl, _) => Err(ChainDecodeError::UnsupportedArgType {
            path: path.to_string(),
            declared: describe_type(declared),
            details: format!(
                "argument is declared at a different inductive than the enclosing `{}`; \
                 decoding it needs the layer chain to resolve `{}`'s ctors (gh #74)",
                decl.iri, arg_decl.iri
            ),
        }),
        // A bound type parameter or index. There is no ground type to
        // map a JSON shape onto — the slot's type depends on how the
        // inductive was instantiated, which a D32 §3.7 chain value does
        // not record.
        Exp::Var(name) => Err(ChainDecodeError::UnsupportedArgType {
            path: path.to_string(),
            declared: name.clone(),
            details: format!(
                "`{name}` is a type parameter of `{}`, not a ground type; a chain value \
                 carries no instantiation for it",
                decl.iri
            ),
        }),
        // Deliberately no shape-guessing fallback: an unrecognised
        // declared type is reported, naming it, so a change to the
        // ontology surfaces here instead of silently reinstating the
        // JSON-shape guess this decoder replaced.
        other => Err(ChainDecodeError::UnsupportedArgType {
            path: path.to_string(),
            declared: describe_type(other),
            details: "no chain-value encoding is defined for this argument type".to_string(),
        }),
    }
}

/// Name a declared argument type the way it is written in the
/// ontology, so diagnostics quote `urn:eigenius:core:string` rather
/// than the `Exp` debug shape (whose `InductiveType` arm prints an
/// entire stub declaration).
fn describe_type(typ: &Exp) -> String {
    match typ {
        Exp::EigonPrimitive(PrimitiveType::String) => wk::STRING.to_string(),
        Exp::EigonPrimitive(PrimitiveType::Integer) => wk::INTEGER.to_string(),
        Exp::EigonPrimitive(PrimitiveType::Float) => wk::FLOAT.to_string(),
        Exp::EigonPrimitive(PrimitiveType::Boolean) => wk::BOOLEAN.to_string(),
        Exp::EigonPrimitive(PrimitiveType::Json) => wk::JSON.to_string(),
        Exp::InductiveType(d, _) => d.iri.to_string(),
        Exp::EigonClass(iri) | Exp::EigonAxiom(iri) => iri.to_string(),
        Exp::Var(n) => n.clone(),
        other => format!("{other:?}"),
    }
}

/// Name what the JSON value actually is, including the offending
/// scalar so the diagnostic pins the value and not just its kind.
fn describe_json(json: &serde_json::Value) -> String {
    match json {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => format!("the boolean `{b}`"),
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => {
            format!("the integer `{n}`")
        }
        serde_json::Value::Number(n) => format!("the float `{n}`"),
        serde_json::Value::String(s) => format!("the string `{s}`"),
        serde_json::Value::Array(_) => "a JSON array".to_string(),
        serde_json::Value::Object(_) => "a JSON object".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::esl;
    use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
    use eigenius_kernel::ontology::eigon_json;
    use serde_json::json;

    /// Core-only root layer — enough for `resolve_class_type` to fold
    /// the primitive class IRIs and to resolve sibling inductives
    /// declared in the layer on top.
    fn core_layer() -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut builder = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).expect("core ontology parses") {
            builder.add_resource(r).expect("core resource admitted");
        }
        Arc::new(builder.build(LayerStorage::in_memory()))
    }

    /// Compile an ESL source over core and resolve one of its
    /// inductives to the `Arc<InductiveDecl>` the decoder consumes.
    /// Everything comes from the real resolver, so the tests below
    /// assert against the telescope spelling the chain actually
    /// produces rather than a hand-built approximation.
    fn inductive_from_esl(source: &str, iri: &str) -> Arc<InductiveDecl> {
        let mut builder = LayerBuilder::new("test", Some(core_layer()));
        for r in esl::compile(source).expect("ESL compiles") {
            builder.add_resource(r).expect("resource admitted");
        }
        let layer = Arc::new(builder.build(LayerStorage::in_memory()));
        match resolve_class_type(&Iri::parse(iri).expect("valid IRI"), &layer) {
            Ok(Val::InductiveType { decl, .. }) => decl,
            other => panic!("`{iri}` did not resolve to an inductive: {other:?}"),
        }
    }

    /// The real `reasoning:JustificationTerm`, resolved from
    /// `ontologies/reasoning/reasoning.esl` through the same chain the
    /// institution stands up.
    fn justification_term() -> Arc<InductiveDecl> {
        let reflection_json =
            include_str!("../../../ontologies/reflection/reflection-ontology.json");
        let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
        let institution_json =
            include_str!("../../../ontologies/institution/institution-ontology.json");
        let mut builder = LayerBuilder::new("reflection", Some(core_layer()));
        for src in [reflection_json, eigentt_json, institution_json] {
            for r in eigon_json::parse_document(src).expect("ontology parses") {
                builder.add_resource(r).expect("resource admitted");
            }
        }
        let reflection = Arc::new(builder.build(LayerStorage::in_memory()));

        let reasoning_source = include_str!("../../../ontologies/reasoning/reasoning.esl");
        let mut builder = LayerBuilder::new("reasoning", Some(reflection));
        for r in esl::compile(reasoning_source).expect("reasoning.esl compiles") {
            builder.add_resource(r).expect("resource admitted");
        }
        let reasoning = Arc::new(builder.build(LayerStorage::in_memory()));

        match resolve_class_type(
            &Iri::parse(iris::JUSTIFICATION_TERM).expect("static IRI"),
            &reasoning,
        ) {
            Ok(Val::InductiveType { decl, .. }) => decl,
            other => panic!("JustificationTerm did not resolve to an inductive: {other:?}"),
        }
    }

    fn decode(json: serde_json::Value, decl: &Arc<InductiveDecl>) -> Result<Exp, ChainDecodeError> {
        chain_value_to_exp(&Value::Json(json), decl)
    }

    /// One ctor per literal kind, plus a self-recursive ctor and a
    /// cross-inductive one.
    const LITS_ESL: &str = r#"
namespace core = "urn:eigenius:core";
namespace probe = "urn:eigenius:probe";

data probe:Other {
    O(core:string),
}

data probe:Lits {
    All(core:string, core:integer, core:float, core:boolean),
    Nest(probe:Lits),
    Wrap(probe:Other),
    Nullary,
}
"#;

    fn lits() -> Arc<InductiveDecl> {
        inductive_from_esl(LITS_ESL, "urn:eigenius:probe:Lits")
    }

    // ── The spelling this decoder is written against ──────────────

    /// Pins the assumption the mapping in [`decode_arg`] rests on: the
    /// resolver folds `core:string` in a ctor telescope to
    /// `Exp::EigonPrimitive(PrimitiveType::String)`, and a
    /// self-reference to `Exp::InductiveType` carrying the inductive's
    /// own IRI. If the resolver ever spells these differently the
    /// decoder must be updated in lockstep, and this test says so.
    #[test]
    fn justification_term_telescope_spells_core_string_as_eigon_primitive() {
        let decl = justification_term();
        assert!(
            decl.params.is_empty() && decl.indices.is_empty(),
            "JustificationTerm is monomorphic and non-indexed"
        );

        let declared_evidence = decl
            .ctors
            .iter()
            .find(|c| c.name == "DeclaredEvidence")
            .expect("DeclaredEvidence declared");
        let args = ctor_arg_types(&decl, declared_evidence).expect("telescope walks");
        assert!(
            matches!(
                args.as_slice(),
                [Exp::EigonPrimitive(PrimitiveType::String)]
            ),
            "DeclaredEvidence : core:string -> J, got {args:?}"
        );

        let spec_str_ctor = decl
            .ctors
            .iter()
            .find(|c| c.name == "SpecStr")
            .expect("SpecStr declared");
        let args = ctor_arg_types(&decl, spec_str_ctor).expect("telescope walks");
        match args.as_slice() {
            [Exp::InductiveType(d, _), Exp::EigonPrimitive(PrimitiveType::String)] => {
                assert_eq!(d.iri.as_str(), iris::JUSTIFICATION_TERM);
            }
            other => panic!("SpecStr : J -> core:string -> J, got {other:?}"),
        }
    }

    // ── Each literal kind decodes against its declared type ───────

    #[test]
    fn every_literal_kind_decodes_against_its_declared_type() {
        let decl = lits();
        let exp =
            decode(json!({ "ctor": "All", "args": ["s", 7, 1.5, true] }), &decl).expect("decodes");
        match exp {
            Exp::InductiveCtor(_, name, args) => {
                assert_eq!(name, "All");
                assert_eq!(args.len(), 4);
                assert_eq!(args[0], Exp::LitString("s".to_string()));
                assert_eq!(args[1], Exp::LitInt(7));
                assert_eq!(args[2], Exp::LitFloat(1.5));
                assert_eq!(args[3], Exp::LitBool(true));
            }
            other => panic!("expected InductiveCtor, got {other:?}"),
        }
    }

    /// The gap issue #198 reported: a JSON `true` in a
    /// `core:boolean` slot. The old shape-guessing decoder had no
    /// `as_bool` arm and rejected it outright.
    #[test]
    fn boolean_argument_decodes_to_lit_bool() {
        let decl = lits();
        let exp = decode(
            json!({ "ctor": "All", "args": ["s", 0, 0.0, false] }),
            &decl,
        )
        .expect("decodes");
        match exp {
            Exp::InductiveCtor(_, _, args) => assert_eq!(args[3], Exp::LitBool(false)),
            other => panic!("expected InductiveCtor, got {other:?}"),
        }
    }

    /// `core:float` accepts an integer-valued JSON number. The kernel
    /// validator's per-arg check uses `is_number` for `core:float`, so
    /// a value written `2` commits; refusing it here would make a
    /// committed sentence undecodable.
    #[test]
    fn integer_json_in_a_float_slot_decodes_as_lit_float() {
        let decl = lits();
        let exp =
            decode(json!({ "ctor": "All", "args": ["s", 1, 2, true] }), &decl).expect("decodes");
        match exp {
            Exp::InductiveCtor(_, _, args) => assert_eq!(args[2], Exp::LitFloat(2.0)),
            other => panic!("expected InductiveCtor, got {other:?}"),
        }
    }

    #[test]
    fn nested_and_recursive_arguments_decode() {
        let decl = justification_term();
        let exp = decode(
            json!({
                "ctor": "App",
                "args": [
                    { "ctor": "DeclaredEvidence", "args": ["urn:eigenius:demo:a"] },
                    {
                        "ctor": "SpecStr",
                        "args": [
                            { "ctor": "ObservedEvidence", "args": ["urn:eigenius:demo:b"] },
                            "urn:eigenius:demo:t"
                        ]
                    }
                ]
            }),
            &decl,
        )
        .expect("decodes");
        match exp {
            Exp::InductiveCtor(d, name, args) => {
                assert_eq!(name, "App");
                assert_eq!(d.iri.as_str(), iris::JUSTIFICATION_TERM);
                assert!(matches!(&args[0], Exp::InductiveCtor(_, n, _) if n == "DeclaredEvidence"));
                match &args[1] {
                    Exp::InductiveCtor(_, n, inner) => {
                        assert_eq!(n, "SpecStr");
                        assert!(
                            matches!(&inner[0], Exp::InductiveCtor(_, n, _) if n == "ObservedEvidence")
                        );
                        assert_eq!(inner[1], Exp::LitString("urn:eigenius:demo:t".to_string()));
                    }
                    other => panic!("expected SpecStr, got {other:?}"),
                }
            }
            other => panic!("expected InductiveCtor, got {other:?}"),
        }
    }

    // ── A mistyped argument is rejected, naming both ──────────────

    /// The behaviour change this commit is about. The old decoder read
    /// `42` as `Exp::LitInt(42)` and handed a `JustificationTerm`
    /// whose `DeclaredEvidence` payload was an integer to the kernel.
    #[test]
    fn integer_in_a_declared_string_slot_is_rejected_naming_both() {
        let decl = justification_term();
        let err = decode(json!({ "ctor": "DeclaredEvidence", "args": [42] }), &decl)
            .expect_err("an integer in a core:string slot must not decode");
        assert_eq!(
            err,
            ChainDecodeError::ArgTypeMismatch {
                path: "<root>.args[0]".to_string(),
                declared: "urn:eigenius:core:string".to_string(),
                found: "the integer `42`".to_string(),
            }
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains("urn:eigenius:core:string") && rendered.contains("42"),
            "diagnostic must name the declared type and what was found: {rendered}"
        );
    }

    #[test]
    fn each_literal_kind_rejects_the_wrong_json_shape() {
        let decl = lits();
        // Position 0 is core:string, 1 core:integer, 2 core:float,
        // 3 core:boolean.
        let cases = [
            (
                json!({ "ctor": "All", "args": [true, 1, 1.0, true] }),
                "urn:eigenius:core:string",
                "the boolean `true`",
                0,
            ),
            (
                json!({ "ctor": "All", "args": ["s", 1.5, 1.0, true] }),
                "urn:eigenius:core:integer",
                "the float `1.5`",
                1,
            ),
            (
                json!({ "ctor": "All", "args": ["s", 1, "x", true] }),
                "urn:eigenius:core:float",
                "the string `x`",
                2,
            ),
            (
                json!({ "ctor": "All", "args": ["s", 1, 1.0, "yes"] }),
                "urn:eigenius:core:boolean",
                "the string `yes`",
                3,
            ),
        ];
        for (value, declared, found, index) in cases {
            let err = decode(value, &decl).expect_err("mistyped argument must not decode");
            assert_eq!(
                err,
                ChainDecodeError::ArgTypeMismatch {
                    path: format!("<root>.args[{index}]"),
                    declared: declared.to_string(),
                    found: found.to_string(),
                }
            );
        }
    }

    /// A nested-ctor object where a primitive is declared, and a
    /// primitive where a nested ctor is declared — both directions.
    #[test]
    fn object_and_scalar_are_rejected_against_the_opposite_declaration() {
        let decl = justification_term();

        let err = decode(
            json!({
                "ctor": "DeclaredEvidence",
                "args": [{ "ctor": "DeclaredEvidence", "args": ["urn:x"] }]
            }),
            &decl,
        )
        .expect_err("a nested ctor in a core:string slot must not decode");
        assert_eq!(
            err,
            ChainDecodeError::ArgTypeMismatch {
                path: "<root>.args[0]".to_string(),
                declared: "urn:eigenius:core:string".to_string(),
                found: "a JSON object".to_string(),
            }
        );

        let err = decode(json!({ "ctor": "App", "args": ["urn:x", "urn:y"] }), &decl)
            .expect_err("a string in a JustificationTerm slot must not decode");
        assert_eq!(
            err,
            ChainDecodeError::ArgTypeMismatch {
                path: "<root>.args[0]".to_string(),
                declared: iris::JUSTIFICATION_TERM.to_string(),
                found: "the string `urn:x`".to_string(),
            }
        );
    }

    // ── Arity ─────────────────────────────────────────────────────

    /// Falls out of pairing `args` with the telescope — there is no
    /// declared type for a surplus argument and no value for a missing
    /// one. The kernel validator's `walk_inductive_value` catches this
    /// at commit; this arm covers a sentence committed before it did.
    #[test]
    fn arity_mismatch_is_rejected_in_both_directions() {
        let decl = justification_term();
        assert_eq!(
            decode(json!({ "ctor": "App", "args": [] }), &decl).expect_err("too few"),
            ChainDecodeError::ArityMismatch {
                path: "<root>".to_string(),
                ctor_name: "App".to_string(),
                expected: 2,
                got: 0,
            }
        );
        assert_eq!(
            decode(
                json!({ "ctor": "DeclaredEvidence", "args": ["a", "b"] }),
                &decl
            )
            .expect_err("too many"),
            ChainDecodeError::ArityMismatch {
                path: "<root>".to_string(),
                ctor_name: "DeclaredEvidence".to_string(),
                expected: 1,
                got: 2,
            }
        );
    }

    #[test]
    fn nullary_ctor_decodes_with_no_arguments() {
        let decl = lits();
        let exp = decode(json!({ "ctor": "Nullary", "args": [] }), &decl).expect("decodes");
        assert!(
            matches!(exp, Exp::InductiveCtor(_, ref n, ref a) if n == "Nullary" && a.is_empty())
        );
    }

    // ── Declared types this decoder does not encode ───────────────

    #[test]
    fn an_argument_at_another_inductive_is_reported_not_guessed() {
        let decl = lits();
        let err = decode(
            json!({ "ctor": "Wrap", "args": [{ "ctor": "O", "args": ["s"] }] }),
            &decl,
        )
        .expect_err("cross-inductive args are unsupported");
        match err {
            ChainDecodeError::UnsupportedArgType {
                ref path,
                ref declared,
                ..
            } => {
                assert_eq!(path, "<root>.args[0]");
                assert_eq!(declared, "urn:eigenius:probe:Other");
            }
            other => panic!("expected UnsupportedArgType, got {other:?}"),
        }
    }

    /// An argument declared at a plain (non-inductive, non-primitive)
    /// class. The old decoder read the IRI string in that slot as
    /// `Exp::LitString`; there is no `Exp` literal for a resource
    /// reference, so the decoder says so instead of guessing.
    ///
    /// D73 §3.1 may move the four `*Evidence` ctors' argument from
    /// `core:string` to a resource, which lands here — loudly — rather
    /// than continuing to decode the old string shape.
    #[test]
    fn an_argument_at_a_plain_class_is_reported_not_guessed() {
        let decl = inductive_from_esl(
            r#"
namespace core = "urn:eigenius:core";
namespace probe = "urn:eigenius:probe";

class probe:Agent {
    description = "A plain class, not an inductive.";
}

data probe:Cited {
    By(probe:Agent),
}
"#,
            "urn:eigenius:probe:Cited",
        );
        let err = decode(
            json!({ "ctor": "By", "args": ["urn:eigenius:probe:someone"] }),
            &decl,
        )
        .expect_err("a plain-class slot has no chain-value encoding");
        match err {
            ChainDecodeError::UnsupportedArgType {
                ref path,
                ref declared,
                ..
            } => {
                assert_eq!(path, "<root>.args[0]");
                assert_eq!(declared, "urn:eigenius:probe:Agent");
            }
            other => panic!("expected UnsupportedArgType, got {other:?}"),
        }
    }

    /// A parameterised inductive: the ctor telescope leads with the
    /// parameter binders (stripped by [`ctor_arg_types`]) and the
    /// argument's declared domain is the bound variable `A`, not a
    /// ground type. D73 §11.3 asks whether `SpecStr` generalises past
    /// `core:string`; if the answer makes its domain a parameter, this
    /// is the diagnostic that fires.
    #[test]
    fn a_type_parameter_domain_is_reported_not_guessed() {
        let decl = inductive_from_esl(
            r#"
namespace core = "urn:eigenius:core";
namespace probe = "urn:eigenius:probe";

data probe:Boxed (A : Prop) : Prop {
    Wrap : forall (A : Prop) => A -> probe:Boxed(A)
}
"#,
            "urn:eigenius:probe:Boxed",
        );
        assert_eq!(decl.params.len(), 1, "Boxed has one parameter");

        let wrap = decl
            .ctors
            .iter()
            .find(|c| c.name == "Wrap")
            .expect("Wrap declared");
        let args = ctor_arg_types(&decl, wrap).expect("telescope walks past the parameter binder");
        assert!(
            matches!(args.as_slice(), [Exp::Var(n)] if n == "A"),
            "the parameter prefix must be stripped, leaving `A`: {args:?}"
        );

        let err = decode(json!({ "ctor": "Wrap", "args": ["anything"] }), &decl)
            .expect_err("a type-parameter slot has no ground shape");
        match err {
            ChainDecodeError::UnsupportedArgType {
                ref path,
                ref declared,
                ..
            } => {
                assert_eq!(path, "<root>.args[0]");
                assert_eq!(declared, "A");
            }
            other => panic!("expected UnsupportedArgType, got {other:?}"),
        }
    }

    // ── Pre-existing diagnostics still fire ───────────────────────

    #[test]
    fn unknown_ctor_lists_the_available_ones() {
        let decl = justification_term();
        let err = decode(json!({ "ctor": "Bogus", "args": [] }), &decl).expect_err("unknown ctor");
        match err {
            ChainDecodeError::UnknownCtor {
                ctor_name,
                available,
                ..
            } => {
                assert_eq!(ctor_name, "Bogus");
                assert!(available.contains(&"App".to_string()));
            }
            other => panic!("expected UnknownCtor, got {other:?}"),
        }
    }

    #[test]
    fn non_json_and_malformed_envelopes_are_rejected() {
        let decl = justification_term();
        assert!(matches!(
            chain_value_to_exp(&Value::String("nope".into()), &decl),
            Err(ChainDecodeError::NotJson(_))
        ));
        assert!(matches!(
            decode(json!("nope"), &decl),
            Err(ChainDecodeError::NotObject(_))
        ));
        assert!(matches!(
            decode(json!({ "args": [] }), &decl),
            Err(ChainDecodeError::MissingCtor(_))
        ));
        assert!(matches!(
            decode(json!({ "ctor": "App" }), &decl),
            Err(ChainDecodeError::MissingArgs(_))
        ));
    }
}
