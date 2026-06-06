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

//! D47 — chain-mirrored EigenTT type fragment codec.
//!
//! Encodes closed EigenTT type expressions ([`Exp`]) as chain-resident
//! values conforming to the `urn:eigenius:eigentt:TypeExpr` inductive
//! type. Decoder is the inverse, resolving `ConstRef`s through the
//! supplied chain layers.
//!
//! Used by D46 §10 (axiom-as-Resource framework) to encode axiom
//! statements as queryable chain artifacts.
//!
//! See `docs/design/d47-chain-mirrored-eigentt-type-fragment.md`.

use crate::layer::Layer;
use crate::nbe::term::{Exp, Patt};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use serde_json::json;

/// Encoding errors raised when an `Exp` cannot be expressed in the
/// chain-mirrored type-fragment language.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodeError {
    /// The given `Exp` variant is not a type-level form (its term-level
    /// content cannot appear in a closed type expression).
    NotATypeLevelExp(String),
    /// A `Lam` was encountered at type level without an accompanying
    /// type annotation in the encoder context. v1 rejects this case;
    /// type-level Lam is rare in practice (only motives / parametric
    /// definitions), and a future version may carry the annotation
    /// through a parallel env.
    LamWithoutAnnotation,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::NotATypeLevelExp(s) => {
                write!(f, "Exp variant is not a type-level form: {s}")
            }
            EncodeError::LamWithoutAnnotation => write!(
                f,
                "type-level Lam encountered without binder-type annotation in context"
            ),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Encode an EigenTT type expression as a chain-resident
/// `eigentt:TypeExpr` value.
///
/// The output is a [`Value::Json`] wrapping a `{"ctor": ..., "args": [...]}`
/// tree shape (D32 §3.7). The validator at commit time walks the tree
/// against the ctor schema declared in
/// `ontologies/eigentt/eigentt-type-fragment.json`.
///
/// Multi-arg type-former references (e.g., `InductiveType(List, [Nat])`)
/// are encoded by App currying — `App(ConstRef(List), ConstRef(Nat))` —
/// per D47 §3.1.
pub fn encode_type(exp: &Exp) -> Result<Value, EncodeError> {
    Ok(Value::Json(encode_type_json(exp)?))
}

fn encode_type_json(exp: &Exp) -> Result<serde_json::Value, EncodeError> {
    match exp {
        Exp::Sort(n) => Ok(ctor("Sort", vec![json!(*n as i64)])),
        Exp::Var(name) => Ok(ctor("Var", vec![json!(name)])),
        Exp::App(h, a) => Ok(ctor(
            "App",
            vec![encode_type_json(h)?, encode_type_json(a)?],
        )),
        Exp::Pi(p, dom, body) => Ok(ctor(
            "Pi",
            vec![
                json!(binder_name(p)),
                encode_type_json(dom)?,
                encode_type_json(body)?,
            ],
        )),
        Exp::Sig(p, dom, body) => Ok(ctor(
            "Sig",
            vec![
                json!(binder_name(p)),
                encode_type_json(dom)?,
                encode_type_json(body)?,
            ],
        )),
        Exp::Arrow(a, b) => encode_type_json(&Exp::Pi(Patt::Unit, a.clone(), b.clone())),
        Exp::Times(a, b) => encode_type_json(&Exp::Sig(Patt::Unit, a.clone(), b.clone())),
        Exp::Lam(_, _) => Err(EncodeError::LamWithoutAnnotation),
        Exp::One => Ok(ctor("One", vec![])),
        Exp::Id(ty, x, y) => Ok(ctor(
            "Id",
            vec![
                encode_type_json(ty)?,
                encode_type_json(x)?,
                encode_type_json(y)?,
            ],
        )),
        Exp::EigonClass(iri) => Ok(ctor("ConstRef", vec![json!(iri.as_str())])),
        Exp::EigonPrimitive(_) => {
            // EigonPrimitive carries a Rust enum (PrimitiveType), not an IRI
            // directly. Encoding requires a small PrimitiveType→IRI lookup
            // table to canonical core: IRIs — add when the first axiom needs
            // primitive refs.
            Err(EncodeError::NotATypeLevelExp(format!(
                "EigonPrimitive encoding requires a primitive-type IRI table (not yet implemented): {exp:?}"
            )))
        }
        Exp::InductiveType(decl, args) => {
            // Encode `I(a1, a2, ...)` as
            //   App(App(...App(ConstRef(I.iri), a1)..., a_{n-1}), a_n)
            let mut current = ctor("ConstRef", vec![json!(decl.name.clone())]);
            for arg in args {
                current = ctor("App", vec![current, encode_type_json(arg)?]);
            }
            Ok(current)
        }
        Exp::CodataType(decl, args) => {
            let mut current = ctor("ConstRef", vec![json!(decl.name.clone())]);
            for arg in args {
                current = ctor("App", vec![current, encode_type_json(arg)?]);
            }
            Ok(current)
        }

        // ── D48 / eigenius#71 — term-level value encoding ─────────
        // Lets indexed inductive applications with concrete index values
        // (Vec Nat 3, AssayShape 3, etc.) round-trip through the codec.
        Exp::Unit => Ok(ctor("UnitVal", vec![])),
        Exp::Pair(a, b) => Ok(ctor(
            "Pair",
            vec![encode_type_json(a)?, encode_type_json(b)?],
        )),
        Exp::InductiveCtor(decl, ctor_name, args) => {
            // Encode `D.c(a1, ..., aN)` as
            //   App(App(...App(CtorApp(D.iri, c), a1)..., a_{N-1}), aN)
            let mut current = ctor("CtorApp", vec![json!(decl.name.clone()), json!(ctor_name)]);
            for arg in args {
                current = ctor("App", vec![current, encode_type_json(arg)?]);
            }
            Ok(current)
        }
        // Note: Exp::Con (anonymous Sum constructor) is intentionally
        // not yet encoded — chain-resident axioms reference declared
        // inductives via Exp::InductiveCtor; anonymous Sum ctors don't
        // arise in axiom statements today. Add when a consumer needs it.
        other => Err(EncodeError::NotATypeLevelExp(format!("{other:?}"))),
    }
}

fn ctor(name: &str, args: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "ctor": name,
        "args": args,
    })
}

/// A `Patt::Var(name)` becomes the binder name; `Patt::Unit` encodes
/// as the empty string (decoded back to `Patt::Unit`); pattern bindings
/// `Patt::Pair(...)` are not supported in type expressions (see D47 §3.6).
fn binder_name(p: &Patt) -> String {
    match p {
        Patt::Var(n) => n.clone(),
        Patt::Unit => String::new(),
        Patt::Pair(_, _) => String::new(),
    }
}

/// Errors raised when a chain-resident `eigentt:TypeExpr` value cannot
/// be decoded back to an `Exp`.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// The value isn't a JSON-shaped chain inductive (`Value::Json`).
    MalformedValue(String),
    /// The `ctor` field is missing or not a string.
    MissingCtor,
    /// The `args` field is missing or not an array.
    MissingArgs,
    /// The ctor name isn't one of the 9 declared MiniTTType ctors.
    UnknownCtor(String),
    /// A ctor's args array has the wrong length for that ctor.
    WrongArgCount {
        ctor: &'static str,
        expected: usize,
        actual: usize,
    },
    /// A ctor arg had the wrong JSON shape (e.g., expected a string,
    /// got an integer).
    WrongArgShape {
        ctor: &'static str,
        slot: usize,
        details: String,
    },
    /// A `ConstRef`'s IRI couldn't be resolved in the supplied layer
    /// chain.
    UnresolvedConstRef(Iri),
    /// A `ConstRef` resolved to a resource whose primary class isn't
    /// one of the type-former classes (Class / DataType /
    /// InductiveType / CodataType).
    ConstRefWrongClass { iri: Iri, found_classes: Vec<Iri> },
    /// An `App` was decoded with a head that doesn't admit applications
    /// (e.g., a fully-applied EigonClass that takes no arguments).
    AppOnNonParametric(String),
    /// A `CtorApp` referenced a ctor name that doesn't exist on the
    /// resolved inductive type. (D48 / eigenius#71)
    CtorAppUnknownCtor {
        decl_iri: Iri,
        ctor_name: String,
        available: Vec<String>,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::MalformedValue(s) => write!(f, "malformed eigentt:TypeExpr value: {s}"),
            DecodeError::MissingCtor => write!(f, "eigentt:TypeExpr value missing `ctor` field"),
            DecodeError::MissingArgs => write!(f, "eigentt:TypeExpr value missing `args` field"),
            DecodeError::UnknownCtor(c) => write!(f, "unknown eigentt:TypeExpr ctor: `{c}`"),
            DecodeError::WrongArgCount {
                ctor,
                expected,
                actual,
            } => write!(
                f,
                "eigentt:TypeExpr ctor `{ctor}` expects {expected} arg(s), got {actual}"
            ),
            DecodeError::WrongArgShape {
                ctor,
                slot,
                details,
            } => write!(f, "eigentt:TypeExpr ctor `{ctor}` arg {slot}: {details}"),
            DecodeError::UnresolvedConstRef(iri) => {
                write!(f, "ConstRef references unresolved IRI: {iri}")
            }
            DecodeError::ConstRefWrongClass { iri, found_classes } => write!(
                f,
                "ConstRef IRI {iri} resolves to a resource of class {found_classes:?} \
                 (expected Class, DataType, InductiveType, or CodataType)"
            ),
            DecodeError::AppOnNonParametric(s) => {
                write!(f, "App spine applied to non-parametric head: {s}")
            }
            DecodeError::CtorAppUnknownCtor {
                decl_iri,
                ctor_name,
                available,
            } => write!(
                f,
                "CtorApp references unknown ctor `{ctor_name}` on inductive {decl_iri}; \
                 available ctors: {available:?}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode a chain-resident `eigentt:TypeExpr` value back to an
/// EigenTT `Exp`.
///
/// `App`-spines whose head is a `ConstRef` pointing to a parametric
/// `InductiveType` or `CodataType` are folded into a single
/// `Exp::InductiveType(decl, args)` / `Exp::CodataType(decl, args)`
/// node, matching the encoder's currying convention (D47 §3.1).
pub fn decode_type(value: &Value, layer: &Layer) -> Result<Exp, DecodeError> {
    let json = match value {
        Value::Json(j) => j,
        other => {
            return Err(DecodeError::MalformedValue(format!(
                "expected Value::Json, got {other:?}"
            )));
        }
    };
    decode_type_json(json, layer)
}

fn decode_type_json(v: &serde_json::Value, layer: &Layer) -> Result<Exp, DecodeError> {
    let obj = v
        .as_object()
        .ok_or_else(|| DecodeError::MalformedValue(format!("expected object, got {v:?}")))?;
    let ctor = obj
        .get("ctor")
        .and_then(|c| c.as_str())
        .ok_or(DecodeError::MissingCtor)?;
    let args = obj
        .get("args")
        .and_then(|a| a.as_array())
        .ok_or(DecodeError::MissingArgs)?;
    match ctor {
        "Sort" => {
            expect_arg_count("Sort", 1, args)?;
            let level = args[0]
                .as_i64()
                .ok_or_else(|| wrong_shape("Sort", 0, "expected integer"))?;
            Ok(Exp::Sort(level as usize))
        }
        "Var" => {
            expect_arg_count("Var", 1, args)?;
            let name = arg_string("Var", 0, &args[0])?;
            Ok(Exp::Var(name))
        }
        "One" => {
            expect_arg_count("One", 0, args)?;
            Ok(Exp::One)
        }
        "Pi" => {
            expect_arg_count("Pi", 3, args)?;
            let name = arg_string("Pi", 0, &args[0])?;
            let dom = decode_type_json(&args[1], layer)?;
            let body = decode_type_json(&args[2], layer)?;
            let patt = if name.is_empty() {
                Patt::Unit
            } else {
                Patt::Var(name)
            };
            Ok(Exp::Pi(patt, Box::new(dom), Box::new(body)))
        }
        "Sig" => {
            expect_arg_count("Sig", 3, args)?;
            let name = arg_string("Sig", 0, &args[0])?;
            let dom = decode_type_json(&args[1], layer)?;
            let body = decode_type_json(&args[2], layer)?;
            let patt = if name.is_empty() {
                Patt::Unit
            } else {
                Patt::Var(name)
            };
            Ok(Exp::Sig(patt, Box::new(dom), Box::new(body)))
        }
        "Lam" => {
            expect_arg_count("Lam", 3, args)?;
            let name = arg_string("Lam", 0, &args[0])?;
            // The dom annotation is decoded for round-trip-fidelity validation
            // but discarded — Exp::Lam doesn't carry a type slot.
            let _dom = decode_type_json(&args[1], layer)?;
            let body = decode_type_json(&args[2], layer)?;
            let patt = if name.is_empty() {
                Patt::Unit
            } else {
                Patt::Var(name)
            };
            Ok(Exp::Lam(patt, Box::new(body)))
        }
        "Id" => {
            expect_arg_count("Id", 3, args)?;
            let ty = decode_type_json(&args[0], layer)?;
            let lhs = decode_type_json(&args[1], layer)?;
            let rhs = decode_type_json(&args[2], layer)?;
            Ok(Exp::Id(Box::new(ty), Box::new(lhs), Box::new(rhs)))
        }
        "App" => {
            expect_arg_count("App", 2, args)?;
            let head = decode_type_json(&args[0], layer)?;
            let arg = decode_type_json(&args[1], layer)?;
            // Spine folding: if head is an InductiveType / CodataType /
            // InductiveCtor, append arg to its args list. Otherwise
            // produce a plain App.
            match head {
                Exp::InductiveType(decl, mut existing) => {
                    existing.push(arg);
                    Ok(Exp::InductiveType(decl, existing))
                }
                Exp::CodataType(decl, mut existing) => {
                    existing.push(arg);
                    Ok(Exp::CodataType(decl, existing))
                }
                Exp::InductiveCtor(decl, name, mut existing) => {
                    // D48 / eigenius#71: CtorApp via App-currying. The
                    // bottom of the App spine is `CtorApp(D, c)`
                    // (decoded to `Exp::InductiveCtor(decl, c, [])`);
                    // each enclosing App appends an arg.
                    existing.push(arg);
                    Ok(Exp::InductiveCtor(decl, name, existing))
                }
                // EigonClass / EigonPrimitive are nullary type-formers;
                // applying them via App is malformed input.
                head @ (Exp::EigonClass(_) | Exp::EigonPrimitive(_)) => Err(
                    DecodeError::AppOnNonParametric(format!("{head:?} cannot be applied")),
                ),
                other => Ok(Exp::App(Box::new(other), Box::new(arg))),
            }
        }
        "ConstRef" => {
            expect_arg_count("ConstRef", 1, args)?;
            let iri_str = arg_string("ConstRef", 0, &args[0])?;
            let iri = Iri::parse(&iri_str).map_err(|e| {
                wrong_shape("ConstRef", 0, &format!("invalid IRI `{iri_str}`: {e}"))
            })?;
            resolve_const_ref(iri, layer)
        }

        // ── D48 / eigenius#71 — term-level value decoding ─────────
        "UnitVal" => {
            expect_arg_count("UnitVal", 0, args)?;
            Ok(Exp::Unit)
        }
        "Pair" => {
            expect_arg_count("Pair", 2, args)?;
            let fst = decode_type_json(&args[0], layer)?;
            let snd = decode_type_json(&args[1], layer)?;
            Ok(Exp::Pair(Box::new(fst), Box::new(snd)))
        }
        "CtorApp" => {
            expect_arg_count("CtorApp", 2, args)?;
            let decl_iri_str = arg_string("CtorApp", 0, &args[0])?;
            let ctor_name = arg_string("CtorApp", 1, &args[1])?;
            let decl_iri = Iri::parse(&decl_iri_str).map_err(|e| {
                wrong_shape(
                    "CtorApp",
                    0,
                    &format!("invalid decl IRI `{decl_iri_str}`: {e}"),
                )
            })?;
            // Resolve the decl IRI through the layer chain, then
            // verify the named ctor exists on it. Multi-arg invocations
            // are layered via App on the decode side (see the "App" arm
            // above, which folds args into Exp::InductiveCtor's args
            // vec); CtorApp produces the nullary base.
            let decl = resolve_inductive_decl_for_ctor(&decl_iri, &ctor_name, layer)?;
            Ok(Exp::InductiveCtor(decl, ctor_name, Vec::new()))
        }
        lit @ ("LitInt" | "LitString" | "LitFloat") => {
            // Ontology declares these for forward compatibility with
            // FormulaTerm-style numerical institutions, but EigenTT's
            // Exp doesn't yet have literal variants. Decoding them
            // would require AST additions to Exp; punt until a real
            // consumer of literals lands in the kernel.
            Err(DecodeError::UnknownCtor(format!(
                "{lit} literal decoding requires EigenTT Exp to add literal variants — not yet implemented"
            )))
        }

        other => Err(DecodeError::UnknownCtor(other.to_string())),
    }
}

fn resolve_inductive_decl_for_ctor(
    decl_iri: &Iri,
    ctor_name: &str,
    layer: &Layer,
) -> Result<std::sync::Arc<crate::nbe::term::InductiveDecl>, DecodeError> {
    let resource = layer
        .resolve(decl_iri)
        .ok_or_else(|| DecodeError::UnresolvedConstRef(decl_iri.clone()))?;
    use crate::ontology::well_known as wk;
    if !resource.is_instance_of(&wk::iri(wk::INDUCTIVE_TYPE)) {
        return Err(DecodeError::ConstRefWrongClass {
            iri: decl_iri.clone(),
            found_classes: resource.is_a().to_vec(),
        });
    }
    let val = crate::program::ground::resolve_inductive_type(decl_iri, &resource, layer).map_err(
        |e| DecodeError::ConstRefWrongClass {
            iri: decl_iri.clone(),
            found_classes: vec![wk::iri(&format!("resolution error: {e}"))],
        },
    )?;
    let decl = match val {
        crate::nbe::val::Val::InductiveType { decl, .. } => decl,
        other => {
            return Err(DecodeError::ConstRefWrongClass {
                iri: decl_iri.clone(),
                found_classes: vec![wk::iri(&format!("unexpected resolution: {other:?}"))],
            });
        }
    };
    if !decl.ctors.iter().any(|c| c.name == ctor_name) {
        return Err(DecodeError::CtorAppUnknownCtor {
            decl_iri: decl_iri.clone(),
            ctor_name: ctor_name.to_string(),
            available: decl.ctors.iter().map(|c| c.name.clone()).collect(),
        });
    }
    Ok(decl)
}

fn resolve_const_ref(iri: Iri, layer: &Layer) -> Result<Exp, DecodeError> {
    let resource = layer
        .resolve(&iri)
        .ok_or_else(|| DecodeError::UnresolvedConstRef(iri.clone()))?;
    use crate::ontology::well_known as wk;
    let class_iris: Vec<Iri> = resource.is_a().to_vec();
    let class_iri = wk::iri(wk::CLASS);
    let datatype_iri = wk::iri(wk::DATA_TYPE);
    let inductive_iri = wk::iri(wk::INDUCTIVE_TYPE);
    let codata_iri = wk::iri(wk::CODATA_TYPE);
    if class_iris.contains(&class_iri) {
        Ok(Exp::EigonClass(iri))
    } else if class_iris.contains(&datatype_iri) {
        Err(DecodeError::ConstRefWrongClass {
            iri,
            found_classes: class_iris,
        })
        // Once EigonPrimitive encoding is implemented (PrimitiveType↔IRI
        // table), this branch produces Exp::EigonPrimitive(...).
    } else if class_iris.contains(&inductive_iri) {
        let val = crate::program::ground::resolve_inductive_type(&iri, &resource, layer).map_err(
            |e| DecodeError::ConstRefWrongClass {
                iri: iri.clone(),
                found_classes: vec![
                    Iri::parse(&format!("resolution error: {e}")).unwrap_or(iri.clone())
                ],
            },
        )?;
        match val {
            crate::nbe::val::Val::InductiveType { decl, .. } => {
                Ok(Exp::InductiveType(decl, Vec::new()))
            }
            other => Err(DecodeError::ConstRefWrongClass {
                iri,
                found_classes: vec![Iri::parse(&format!("unexpected resolution: {other:?}"))
                    .unwrap_or_else(|_| Iri::parse("urn:_:unknown").unwrap())],
            }),
        }
    } else if class_iris.contains(&codata_iri) {
        // Codata decl resolution lives under ground.rs as resolve_codata_type
        // (it's similar in shape). For now, leave as a stub error if no
        // axiom under test needs it — extend when a use case arrives.
        Err(DecodeError::ConstRefWrongClass {
            iri,
            found_classes: class_iris,
        })
    } else {
        Err(DecodeError::ConstRefWrongClass {
            iri,
            found_classes: class_iris,
        })
    }
}

fn expect_arg_count(
    ctor: &'static str,
    expected: usize,
    args: &[serde_json::Value],
) -> Result<(), DecodeError> {
    if args.len() != expected {
        Err(DecodeError::WrongArgCount {
            ctor,
            expected,
            actual: args.len(),
        })
    } else {
        Ok(())
    }
}

fn arg_string(
    ctor: &'static str,
    slot: usize,
    v: &serde_json::Value,
) -> Result<String, DecodeError> {
    v.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| wrong_shape(ctor, slot, "expected string"))
}

fn wrong_shape(ctor: &'static str, slot: usize, details: &str) -> DecodeError {
    DecodeError::WrongArgShape {
        ctor,
        slot,
        details: details.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::{InductiveCtorDecl, InductiveDecl};
    use std::sync::Arc;

    fn ctor_obj(name: &str, args: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"ctor": name, "args": args})
    }

    #[test]
    fn encodes_sort() {
        let v = encode_type(&Exp::Sort(0)).unwrap();
        assert_eq!(v, Value::Json(ctor_obj("Sort", vec![json!(0)])));
    }

    #[test]
    fn encodes_var() {
        let v = encode_type(&Exp::Var("P".to_string())).unwrap();
        assert_eq!(v, Value::Json(ctor_obj("Var", vec![json!("P")])));
    }

    #[test]
    fn encodes_one() {
        let v = encode_type(&Exp::One).unwrap();
        assert_eq!(v, Value::Json(ctor_obj("One", vec![])));
    }

    #[test]
    fn encodes_arrow_as_pi_with_empty_binder() {
        // 1 → 1 desugars to Pi(_, 1, 1)
        let exp = Exp::Arrow(Box::new(Exp::One), Box::new(Exp::One));
        let v = encode_type(&exp).unwrap();
        let one = ctor_obj("One", vec![]);
        assert_eq!(
            v,
            Value::Json(ctor_obj("Pi", vec![json!(""), one.clone(), one],))
        );
    }

    #[test]
    fn encodes_id_in_prop() {
        let exp = Exp::Id(
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
            Box::new(Exp::Var("y".to_string())),
        );
        let v = encode_type(&exp).unwrap();
        let one = ctor_obj("One", vec![]);
        let vx = ctor_obj("Var", vec![json!("x")]);
        let vy = ctor_obj("Var", vec![json!("y")]);
        assert_eq!(v, Value::Json(ctor_obj("Id", vec![one, vx, vy])));
    }

    #[test]
    fn encodes_propext_shape() {
        // propext : ∀ {P : Prop} {Q : Prop}, ((P → Q) × (Q → P)) → Id Prop P Q
        // Built from D47 §3.2's worked example.
        let p_var = || Exp::Var("P".to_string());
        let q_var = || Exp::Var("Q".to_string());
        let prop = || Exp::Sort(0);
        let p_to_q = Exp::Arrow(Box::new(p_var()), Box::new(q_var()));
        let q_to_p = Exp::Arrow(Box::new(q_var()), Box::new(p_var()));
        let iff = Exp::Times(Box::new(p_to_q), Box::new(q_to_p));
        let id_prop = Exp::Id(Box::new(prop()), Box::new(p_var()), Box::new(q_var()));
        let inner_arrow = Exp::Arrow(Box::new(iff), Box::new(id_prop));
        let outer_q = Exp::Pi(
            Patt::Var("Q".to_string()),
            Box::new(prop()),
            Box::new(inner_arrow),
        );
        let propext = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(prop()),
            Box::new(outer_q),
        );
        // Just verify the round-trip succeeds and produces a Pi-headed tree.
        let v = encode_type(&propext).unwrap();
        let Value::Json(j) = v else {
            panic!("expected Json")
        };
        assert_eq!(j["ctor"], "Pi");
        assert_eq!(j["args"][0], json!("P"));
    }

    #[test]
    fn rejects_lam_without_annotation() {
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let err = encode_type(&lam).unwrap_err();
        assert!(matches!(err, EncodeError::LamWithoutAnnotation));
    }

    #[test]
    fn rejects_non_type_level_exp() {
        // Refl is a term-level form, not a type. Should be rejected.
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let err = encode_type(&refl).unwrap_err();
        assert!(matches!(err, EncodeError::NotATypeLevelExp(_)));
    }

    // ---------- decoder tests ----------

    fn empty_layer() -> std::sync::Arc<Layer> {
        std::sync::Arc::new(
            crate::layer::LayerBuilder::new("decoder-test-empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        )
    }

    fn bootstrap_head() -> std::sync::Arc<Layer> {
        std::sync::Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head())
    }

    #[test]
    fn decodes_sort() {
        let v = encode_type(&Exp::Sort(2)).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, Exp::Sort(2));
    }

    #[test]
    fn decodes_pi_with_named_binder() {
        let exp = Exp::Pi(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
        );
        let v = encode_type(&exp).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, exp);
    }

    #[test]
    fn decodes_arrow_round_trips_as_pi_unit() {
        let exp = Exp::Arrow(Box::new(Exp::One), Box::new(Exp::One));
        let v = encode_type(&exp).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        // Round-trips to the desugared Pi shape per D47 §4.3.
        assert_eq!(
            decoded,
            Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::One))
        );
    }

    #[test]
    fn decodes_id_round_trip() {
        let exp = Exp::Id(
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
            Box::new(Exp::Var("y".to_string())),
        );
        let v = encode_type(&exp).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, exp);
    }

    #[test]
    fn decodes_propext_round_trip() {
        // Re-build propext, encode it, decode it, compare structurally
        // (modulo Arrow→Pi desugaring).
        let p_var = || Exp::Var("P".to_string());
        let q_var = || Exp::Var("Q".to_string());
        let prop = || Exp::Sort(0);
        let p_to_q = Exp::Arrow(Box::new(p_var()), Box::new(q_var()));
        let q_to_p = Exp::Arrow(Box::new(q_var()), Box::new(p_var()));
        let iff = Exp::Times(Box::new(p_to_q), Box::new(q_to_p));
        let id_prop = Exp::Id(Box::new(prop()), Box::new(p_var()), Box::new(q_var()));
        let inner_arrow = Exp::Arrow(Box::new(iff), Box::new(id_prop));
        let outer_q = Exp::Pi(
            Patt::Var("Q".to_string()),
            Box::new(prop()),
            Box::new(inner_arrow),
        );
        let propext = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(prop()),
            Box::new(outer_q),
        );
        let v = encode_type(&propext).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        // The decoded form is the desugared Pi/Sig version of the input.
        // For this round-trip, encode the desugared form and compare values.
        let v2 = encode_type(&decoded).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn decoder_rejects_unknown_ctor() {
        let bad = Value::Json(json!({"ctor": "Nonsense", "args": []}));
        let err = decode_type(&bad, &empty_layer()).unwrap_err();
        assert!(matches!(err, DecodeError::UnknownCtor(c) if c == "Nonsense"));
    }

    #[test]
    fn decoder_rejects_wrong_arg_count() {
        let bad = Value::Json(json!({"ctor": "Sort", "args": []}));
        let err = decode_type(&bad, &empty_layer()).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::WrongArgCount {
                ctor: "Sort",
                expected: 1,
                actual: 0,
            }
        ));
    }

    #[test]
    fn decoder_rejects_unresolved_constref() {
        let bad = Value::Json(json!({
            "ctor": "ConstRef",
            "args": ["urn:eigenius:nonexistent:Foo"]
        }));
        let err = decode_type(&bad, &empty_layer()).unwrap_err();
        assert!(matches!(err, DecodeError::UnresolvedConstRef(_)));
    }

    #[test]
    fn decoder_resolves_constref_to_eigon_class() {
        // urn:eigenius:core:Class is an is_a-of-Class resource in the
        // core ontology.
        let head = bootstrap_head();
        let v = Value::Json(json!({
            "ctor": "ConstRef",
            "args": ["urn:eigenius:core:Class"]
        }));
        let decoded = decode_type(&v, &head).unwrap();
        match decoded {
            Exp::EigonClass(iri) => {
                assert_eq!(iri.as_str(), "urn:eigenius:core:Class");
            }
            other => panic!("expected EigonClass, got {other:?}"),
        }
    }

    #[test]
    fn d48_round_trip_indexed_inductive_application_with_type_indices() {
        // D48 Phase I: an indexed inductive application like
        // `IxClassFamily A Set` (param: A, index: a type) round-trips
        // through encode → decode via App-currying. The encoder
        // produces the App spine; Phase B's eval split (`params ++
        // indices` based on `decl.params.len()`) handles the runtime
        // semantics.
        //
        // Limitation (documented): index values that are term-level
        // (literals, constructor applications) aren't yet encodable
        // by D47 — see D47 §3.5 "no literals". An axiom or theorem
        // statement referencing e.g. `Vec Nat 0` would need D47
        // extended with literal/ctor encoding. This test exercises
        // the type-level-index case which IS supported.
        let ix_decl = Arc::new(InductiveDecl {
            name: "urn:_:IxClassFamily".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            // Index telescope's type is Sort(1) — indices are types
            // themselves (e.g., the index says "what type am I
            // indexed by"). This keeps the test purely type-level.
            indices: vec![(Patt::Unit, Exp::Sort(1))],
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        // `IxClassFamily Some Other` — both param and index are
        // EigonClass IRIs (type-level).
        let app_form = Exp::InductiveType(
            ix_decl,
            vec![
                Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:Some").unwrap()),
                Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:Other").unwrap()),
            ],
        );
        let encoded = encode_type(&app_form).expect("encode indexed inductive");
        let Value::Json(j) = encoded else {
            panic!("expected Value::Json");
        };
        assert_eq!(j["ctor"], "App", "outermost should be App-curried");
        // Walk the App spine to verify the structure: 2 App layers
        // (one per param + index) bottoming at ConstRef(IxClassFamily).
        let mut cursor = &j;
        let mut depth = 0;
        while cursor["ctor"] == "App" {
            cursor = &cursor["args"][0];
            depth += 1;
        }
        assert!(
            cursor["ctor"] == "ConstRef" && cursor["args"][0] == "urn:_:IxClassFamily",
            "App spine should bottom out at ConstRef(urn:_:IxClassFamily); got {cursor}"
        );
        assert_eq!(
            depth, 2,
            "two App layers (one per param + index): got {depth}"
        );
    }

    #[test]
    fn encodes_applied_inductive_via_app_currying() {
        // InductiveType(List, [Nat]) — encoded as App(ConstRef(List), ConstRef(Nat))
        // via currying. We use synthetic decls with names that read as IRIs.
        let nat_decl = Arc::new(InductiveDecl {
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let list_decl = Arc::new(InductiveDecl {
            name: "urn:_:List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::InductiveType(
                    Arc::new(InductiveDecl {
                        name: "urn:_:List".to_string(),
                        params: Vec::new(),
                        indices: Vec::new(),
                        sort: Exp::Sort(1),
                        ctors: Vec::new(),
                    }),
                    vec![Exp::Var("A".to_string())],
                ),
            }],
        });
        let nat = Exp::InductiveType(nat_decl, Vec::new());
        let list_nat = Exp::InductiveType(list_decl, vec![nat]);
        let v = encode_type(&list_nat).unwrap();

        let const_nat = ctor_obj("ConstRef", vec![json!("urn:_:Nat")]);
        let const_list = ctor_obj("ConstRef", vec![json!("urn:_:List")]);
        let expected = ctor_obj("App", vec![const_list, const_nat]);
        assert_eq!(v, Value::Json(expected));
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 / eigenius#71 — term-level encoding (UnitVal, Pair, CtorApp)
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn encodes_unit_value() {
        let v = encode_type(&Exp::Unit).unwrap();
        assert_eq!(v, Value::Json(ctor_obj("UnitVal", vec![])));
    }

    #[test]
    fn encodes_pair_value() {
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let v = encode_type(&pair).unwrap();
        let unit = ctor_obj("UnitVal", vec![]);
        assert_eq!(v, Value::Json(ctor_obj("Pair", vec![unit.clone(), unit])));
    }

    #[test]
    fn encodes_nullary_inductive_ctor() {
        // Nat.zero — encoded as CtorApp(urn:_:Nat, zero) with no args.
        let nat_decl = Arc::new(InductiveDecl {
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "zero".to_string(),
                typ: Exp::Sort(1),
            }],
        });
        let zero = Exp::InductiveCtor(nat_decl, "zero".to_string(), Vec::new());
        let v = encode_type(&zero).unwrap();
        assert_eq!(
            v,
            Value::Json(ctor_obj("CtorApp", vec![json!("urn:_:Nat"), json!("zero")]))
        );
    }

    #[test]
    fn encodes_unary_inductive_ctor_via_app_currying() {
        // Nat.succ(x) — encoded as App(CtorApp(urn:_:Nat, succ), Var(x))
        let nat_decl = Arc::new(InductiveDecl {
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "succ".to_string(),
                typ: Exp::Sort(1),
            }],
        });
        let succ_x = Exp::InductiveCtor(
            nat_decl,
            "succ".to_string(),
            vec![Exp::Var("x".to_string())],
        );
        let v = encode_type(&succ_x).unwrap();
        let ctor_app = ctor_obj("CtorApp", vec![json!("urn:_:Nat"), json!("succ")]);
        let var_x = ctor_obj("Var", vec![json!("x")]);
        assert_eq!(v, Value::Json(ctor_obj("App", vec![ctor_app, var_x])));
    }

    #[test]
    fn unit_value_round_trips_via_decode() {
        let v = encode_type(&Exp::Unit).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, Exp::Unit);
    }

    #[test]
    fn pair_value_round_trips_via_decode() {
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let v = encode_type(&pair).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, pair);
    }

    #[test]
    fn encodes_indexed_inductive_with_ctor_app_index() {
        // The motivating "AssayShape 3"-style case (D48 / #71 closeout):
        // an indexed inductive whose index is a *value* built from
        // a ctor application (e.g., `succ (succ (succ zero))`).
        //
        // Build:
        //   - Nat-like inductive with zero/succ ctors
        //   - AssayShape : Nat → Set indexed inductive
        //   - The value `AssayShape (succ zero)` as an Exp
        //
        // The encoder must produce a nested App spine:
        //   App(ConstRef(AssayShape), App(CtorApp(Nat, succ), CtorApp(Nat, zero)))
        let nat_decl = Arc::new(InductiveDecl {
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "zero".to_string(),
                    typ: Exp::Sort(1),
                },
                InductiveCtorDecl {
                    name: "succ".to_string(),
                    typ: Exp::Sort(1),
                },
            ],
        });
        let assay_decl = Arc::new(InductiveDecl {
            name: "urn:_:AssayShape".to_string(),
            params: Vec::new(),
            indices: vec![(
                Patt::Var("n".to_string()),
                Exp::InductiveType(nat_decl.clone(), Vec::new()),
            )],
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let zero = Exp::InductiveCtor(nat_decl.clone(), "zero".to_string(), Vec::new());
        let succ_zero = Exp::InductiveCtor(nat_decl, "succ".to_string(), vec![zero]);
        let assay_succ_zero = Exp::InductiveType(assay_decl, vec![succ_zero]);

        let encoded = encode_type(&assay_succ_zero).expect("encode AssayShape (succ zero)");
        let Value::Json(j) = encoded else {
            panic!("expected Value::Json");
        };

        // Walk the outer App to verify shape:
        //   App(ConstRef(AssayShape), App(CtorApp(Nat, succ), CtorApp(Nat, zero)))
        assert_eq!(j["ctor"], "App");
        assert_eq!(j["args"][0]["ctor"], "ConstRef");
        assert_eq!(j["args"][0]["args"][0], "urn:_:AssayShape");
        assert_eq!(j["args"][1]["ctor"], "App");
        assert_eq!(j["args"][1]["args"][0]["ctor"], "CtorApp");
        assert_eq!(j["args"][1]["args"][0]["args"][0], "urn:_:Nat");
        assert_eq!(j["args"][1]["args"][0]["args"][1], "succ");
        assert_eq!(j["args"][1]["args"][1]["ctor"], "CtorApp");
        assert_eq!(j["args"][1]["args"][1]["args"][0], "urn:_:Nat");
        assert_eq!(j["args"][1]["args"][1]["args"][1], "zero");
    }
}
