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
//! values conforming to the `urn:eigenius:eigentt:Term` inductive
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
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use std::collections::BTreeMap;

/// `eigentt:Judgement` — a `holds(logic, term, type)` value, not a term.
const JUDGEMENT_IRI: &str = "urn:eigenius:eigentt:Judgement";

/// Encoding errors raised when an `Exp` cannot be expressed in the
/// chain-mirrored type-fragment language.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodeError {
    /// The given `Exp` variant is not a type-level form (its term-level
    /// content cannot appear in a closed type expression).
    NotATypeLevelExp(String),
    /// The chain does not declare something the codec needs: `eigentt:Term` or `core:Level`
    /// itself, or a constructor the codec emits that the declaration does not have.
    Undeclared(String),
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
            EncodeError::Undeclared(s) => write!(f, "{s}"),
            EncodeError::LamWithoutAnnotation => write!(
                f,
                "type-level Lam encountered without binder-type annotation in context"
            ),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Encode an EigenTT type expression as a chain-resident
/// `eigentt:Term` value.
///
/// The output is a [`Value::Embedded`] resource whose `is_a` names the constructor's class
/// and which carries each argument under that class's property (D85 §6.1). The validator at
/// commit time checks it as a resource: its class against the slot, its arity against the
/// class's `requires`, and each argument against the property that declares it.
///
/// Multi-arg type-former references (e.g., `InductiveType(List, [Nat])`)
/// are encoded by App currying — `App(ConstRef(List), ConstRef(Nat))` —
/// per D47 §3.1.
pub fn encode_type(exp: &Exp, names: &CodecNames) -> Result<Value, EncodeError> {
    encode_term(exp, names)
}

/// The argument names of every constructor this codec writes, read from the chain.
///
/// The codec encodes into exactly two inductives — `eigentt:Term` and `core:Level` — and their
/// constructor names do not collide, so one flat table serves both. Reading the names here
/// rather than hard-coding them keeps the declaration the only place a constructor's arguments
/// are named; eigenius#218 is what a second copy costs when the two drift.
#[derive(Debug, Default, Clone)]
pub struct CodecNames {
    /// `<inductive>-<ctor>` → (inductive IRI, argument names in declaration order). Keyed by
    /// the CLASS, not the constructor's short name: two inductives may share one.
    by_class: BTreeMap<String, (String, Vec<String>)>,
}

impl CodecNames {
    /// Read the table from a chain: EVERY inductive it declares, keyed by
    /// `<inductive>-<ctor>`.
    ///
    /// It carried only `eigentt:Term` and `core:Level` while a constructor application of any
    /// other inductive was App-curried over `CtorApp` — three inductives sufficed because the
    /// spine was written in the term language. `Exp::InductiveCtor(I, c, args)` now writes a
    /// value of `I`'s constructor `c` (D85 §6.1), so the encoder needs `I`'s argument names,
    /// whichever `I` is.
    ///
    /// A chain declaring none of them yields an EMPTY table rather than an error: a source
    /// that encodes no term is a legitimate compile, and the failure belongs where a term
    /// would actually be built.
    pub fn from_layer(layer: &crate::layer::Layer) -> Self {
        let mut by_class = BTreeMap::new();
        for ind in crate::layer::resolve_typed_resources(layer, &[wk::INDUCTIVE_TYPE]) {
            let Some(ind_iri) = ind.id().cloned() else {
                continue;
            };
            let Some(table) = crate::layer::ctor_classes::arg_names_of(layer, &ind_iri) else {
                continue;
            };
            for (ctor, args) in table {
                by_class.insert(
                    crate::layer::ctor_classes::class_iri(ind_iri.as_str(), &ctor),
                    (ind_iri.as_str().to_string(), args),
                );
            }
        }
        Self { by_class }
    }

    /// The argument names of `ctor` on a NAMED inductive, for a caller that knows which one
    /// it means. `lookup` keys on the constructor name alone, which is enough for the term
    /// language but not for a value of some other inductive.
    /// Build from a class table — `<inductive>-<ctor>` → argument names — as the ESL compiler
    /// already holds, chain-resident declarations MERGED with the ones in the file being
    /// compiled. A file declares an inductive and then writes values of it in the same breath,
    /// so a table read from the parent chain alone cannot encode them.
    pub fn from_class_table(table: &BTreeMap<String, Vec<String>>) -> Self {
        let by_class = table
            .iter()
            .filter_map(|(class, args)| {
                let (ind, _) = class.rsplit_once('-')?;
                Some((class.clone(), (ind.to_string(), args.clone())))
            })
            .collect();
        Self { by_class }
    }

    /// Build a value of `inductive`'s constructor `ctor` from already-encoded arguments.
    ///
    /// The public face of the layout authority: a producer outside this module — the Lean
    /// chain mirror, say — says which constructor it means and hands over the arguments, and
    /// the names and arity come from the declaration this table read.
    /// Build the value a `{ctor, args}` literal denotes, arguments and all.
    ///
    /// A tagged literal reads well in source, so several producers write one and hand it here
    /// rather than assembling resources by hand. `prefer` is the inductives to resolve each
    /// constructor against, in order — a term producer passes `eigentt:Term` and `core:Level`,
    /// whose constructors are the term language and share names with other inductives
    /// (`Zero` belongs to `core:Level` and to `lean:LeanLevel`). A constructor none of them
    /// declares resolves against the whole chain, and a name more than one inductive declares
    /// is an error rather than a guess.
    ///
    /// A node with no `ctor` is a leaf: a string, number, boolean, or a list of nodes.
    pub fn value_of_tagged(
        &self,
        prefer: &[&str],
        tagged: &serde_json::Value,
    ) -> Result<Value, EncodeError> {
        let Some(ctor) = tagged.get("ctor").and_then(serde_json::Value::as_str) else {
            return Ok(match tagged {
                serde_json::Value::String(s) => Value::String(s.clone()),
                serde_json::Value::Bool(b) => Value::Boolean(*b),
                serde_json::Value::Array(a) => Value::Array(
                    a.iter()
                        .map(|e| self.value_of_tagged(prefer, e))
                        .collect::<Result<_, _>>()?,
                ),
                serde_json::Value::Number(n) => match n.as_i64() {
                    Some(i) => Value::Integer(i),
                    None => Value::Float(n.as_f64().unwrap_or_default()),
                },
                other => {
                    return Err(EncodeError::Undeclared(format!(
                        "a tagged literal has no leaf form for {other}"
                    )))
                }
            });
        };
        let empty = Vec::new();
        let args: Vec<Value> = tagged
            .get("args")
            .and_then(serde_json::Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .map(|a| self.value_of_tagged(prefer, a))
            .collect::<Result<_, _>>()?;
        let inductive = match prefer.iter().find(|p| self.lookup_declares(p, ctor)) {
            Some(p) => (*p).to_string(),
            None => self.find(ctor)?.to_string(),
        };
        self.value(&inductive, ctor, args)
    }

    pub fn value(
        &self,
        inductive: &str,
        ctor: &str,
        args: Vec<Value>,
    ) -> Result<Value, EncodeError> {
        let (inductive, names) = self.lookup_in(inductive, ctor)?;
        if names.len() != args.len() {
            return Err(EncodeError::Undeclared(format!(
                "`{ctor}` of `{inductive}` takes {} argument(s), got {}",
                names.len(),
                args.len()
            )));
        }
        Ok(crate::layer::ctor_classes::value_resource(
            inductive, ctor, names, &args,
        ))
    }

    /// Find the one inductive declaring `ctor`, for a caller that has only the name.
    ///
    /// Ambiguity is an error, not a guess: `App` belongs to `eigentt:Term`,
    /// `justification:Term` AND `formulas:FormulaTerm`, and picking one silently is how a
    /// value ends up stating a class its slot does not admit.
    /// Does `inductive` declare `ctor`?
    pub fn lookup_declares(&self, inductive: &str, ctor: &str) -> bool {
        self.by_class
            .contains_key(&crate::layer::ctor_classes::class_iri(inductive, ctor))
    }

    pub fn find(&self, ctor: &str) -> Result<&str, EncodeError> {
        let mut hits = self
            .by_class
            .iter()
            .filter(|(class, _)| class.rsplit_once('-').is_some_and(|(_, c)| c == ctor))
            .map(|(_, (ind, _))| ind.as_str());
        let Some(first) = hits.next() else {
            return Err(EncodeError::Undeclared(format!(
                "no inductive in this chain declares a constructor `{ctor}`"
            )));
        };
        let rest: Vec<&str> = hits.collect();
        if rest.is_empty() {
            Ok(first)
        } else {
            Err(EncodeError::Undeclared(format!(
                "`{ctor}` is declared by more than one inductive ({first}, {}) — name the one \
                 you mean",
                rest.join(", ")
            )))
        }
    }

    fn lookup_in(&self, inductive: &str, ctor: &str) -> Result<(&str, &[String]), EncodeError> {
        let class = crate::layer::ctor_classes::class_iri(inductive, ctor);
        match self.by_class.get(&class) {
            Some((ind, args)) => Ok((ind.as_str(), args.as_slice())),
            None => Err(EncodeError::Undeclared(format!(
                "`{inductive}` does not declare a constructor `{ctor}` in this layer chain"
            ))),
        }
    }
}

/// Encode a Lambda chain `λ (x_1 : T_1) … (x_n : T_n). body` with the
/// per-binder type annotations supplied separately, since `Exp::Lam`
/// itself doesn't carry them. Each `(patt, dom)` pair becomes a `Lam`
/// ctor in the chain JSON shape; the dom annotation is decoded-and-
/// discarded by the kernel decoder (D47 §3) but is preserved here for
/// round-trip fidelity. Used by the ESL compiler when emitting motives
/// for `match … returning fun (i : T) => body` (eigenius#72 Layer 3).
pub fn encode_lam_chain(
    binders: &[(Patt, Exp)],
    body: &Exp,
    names: &CodecNames,
) -> Result<Value, EncodeError> {
    let mut acc = encode_term(body, names)?;
    for (patt, dom) in binders.iter().rev() {
        let dom = encode_term(dom, names)?;
        acc = term(
            names,
            "Lam",
            vec![Value::String(binder_name(patt)), dom, acc],
        )?;
    }
    Ok(acc)
}

/// Encode an `Exp` as a chain-resident value (D85 §6.1).
///
/// **One representation.** Each arm names its constructor and its arguments; the argument
/// NAMES come from the inductive's declaration through `names`, and `ctor_classes::value_resource`
/// is the only thing that knows how a value is laid out. There is no intermediate tagged tree
/// and so no translation: `Exp::InductiveCtor(I, c, args)` is a value of `I`'s constructor `c`,
/// written directly, rather than App-curried over a `CtorApp` because JSON could not spell a
/// constructor with named arguments.
pub(crate) fn encode_term(exp: &Exp, names: &CodecNames) -> Result<Value, EncodeError> {
    let enc = |e: &Exp| encode_term(e, names);
    match exp {
        Exp::Sort(n) => term(names, "Sort", vec![encode_level(n, names)?]),
        Exp::Var(name) => term(names, "Var", vec![Value::String(name.clone())]),
        Exp::App(h, a) => term(names, "App", vec![enc(h)?, enc(a)?]),
        Exp::Ann(e, ty) => term(names, "Ann", vec![enc(e)?, enc(ty)?]),
        Exp::Pi(p, dom, body) => term(
            names,
            "Pi",
            vec![Value::String(binder_name(p)), enc(dom)?, enc(body)?],
        ),
        Exp::Sig(p, dom, body) => term(
            names,
            "Sig",
            vec![Value::String(binder_name(p)), enc(dom)?, enc(body)?],
        ),
        Exp::Record(fields) => {
            let mut items = Vec::with_capacity(fields.len());
            for (iri, patt, ty) in fields {
                items.push(Value::Array(vec![
                    Value::String(iri.as_str().to_string()),
                    Value::String(binder_name(patt)),
                    enc(ty)?,
                ]));
            }
            term(names, "Record", vec![Value::Array(items)])
        }
        Exp::Refine(carrier, classes) => term(
            names,
            "Refine",
            vec![
                enc(carrier)?,
                Value::Array(
                    classes
                        .iter()
                        .map(|i| Value::String(i.as_str().to_string()))
                        .collect(),
                ),
            ],
        ),
        Exp::Arrow(a, b) => enc(&Exp::Pi(Patt::Unit, a.clone(), b.clone())),
        Exp::Times(a, b) => enc(&Exp::Sig(Patt::Unit, a.clone(), b.clone())),
        Exp::Lam(_, _) => Err(EncodeError::LamWithoutAnnotation),
        Exp::One => term(names, "One", vec![]),
        Exp::Id(ty, x, y) => term(names, "Id", vec![enc(ty)?, enc(x)?, enc(y)?]),
        Exp::EigonClass(iri) | Exp::EigonAxiom(iri) => const_ref(names, iri.as_str(), &[]),
        Exp::EigonResource(res) => {
            let iri = res.id().ok_or_else(|| {
                EncodeError::NotATypeLevelExp("an EigonResource without an @id".to_string())
            })?;
            const_ref(names, iri.as_str(), &[])
        }
        Exp::EigonPrimitive(prim) => {
            use crate::nbe::term::PrimitiveType;
            let iri_str = match prim {
                PrimitiveType::String => wk::STRING,
                PrimitiveType::Integer => wk::INTEGER,
                PrimitiveType::Float => wk::FLOAT,
                PrimitiveType::Boolean => wk::BOOLEAN,
                PrimitiveType::Json => wk::JSON,
            };
            const_ref(names, iri_str, &[])
        }
        Exp::Const(iri, levels) => const_ref(names, iri.as_str(), levels),
        Exp::Unit => term(names, "UnitVal", vec![]),
        Exp::Pair(a, b) => term(names, "Pair", vec![enc(a)?, enc(b)?]),
        Exp::Fst(p) => term(names, "Fst", vec![enc(p)?]),
        Exp::Snd(p) => term(names, "Snd", vec![enc(p)?]),
        // **Inside a term, a constructor application is a TERM.** `CtorApp` names the
        // constructor and `App` applies it, which is what `eigentt:Term` declares and what
        // `Term-App-arg`'s `class_types: [eigentt:Term]` admits.
        //
        // This is not a second representation of a constructor value. THE SLOT'S DECLARED
        // TYPE decides the shape: a slot typed `eigentt:Judgement` holds a `Judgement-holds`
        // value (`encode_judgement`), an ESL value at a slot typed by its own inductive holds
        // that inductive's constructor class (`Compiler::ctor_application`), and a subterm of
        // a term is a term. Each position has ONE shape; none of them is converted into
        // another.
        Exp::InductiveCtor(iri, ctor_name, args) => {
            let mut acc = term(
                names,
                "CtorApp",
                vec![
                    Value::String(iri.as_str().to_string()),
                    Value::String(ctor_name.clone()),
                ],
            )?;
            for arg in args {
                acc = term(names, "App", vec![acc, enc(arg)?])?;
            }
            Ok(acc)
        }
        // D87 §4.2 — the checked-proof reference. Its own ctor, not `ConstRef`: `ConstRef`
        // resolves by the target's class and yields `EigonClass` / `EigonAxiom` / `Const`, none
        // of which a `lean:LeanProofPayload` instance is, and reusing it would put "asserted
        // without proof" and "checked by nanoda" in one wire form.
        Exp::Checked(iri) => term(
            names,
            "Checked",
            vec![Value::String(iri.as_str().to_string())],
        ),
        Exp::LitString(s) => term(names, "LitString", vec![Value::String(s.clone())]),
        Exp::LitInt(n) => term(names, "LitInt", vec![Value::Integer(*n)]),
        Exp::LitFloat(f) => term(names, "LitFloat", vec![Value::Float(*f)]),
        Exp::LitBool(b) => term(names, "LitBool", vec![Value::Boolean(*b)]),
        other => Err(EncodeError::NotATypeLevelExp(format!("{other:?}"))),
    }
}

/// One `eigentt:Term` value.
fn term(names: &CodecNames, ctor: &str, args: Vec<Value>) -> Result<Value, EncodeError> {
    let (inductive, arg_names) = names.lookup_in(wk::EIGENTT_TERM, ctor)?;
    if arg_names.len() != args.len() {
        return Err(EncodeError::Undeclared(format!(
            "the codec emits `{ctor}` with {} argument(s); its declaration has {}",
            args.len(),
            arg_names.len()
        )));
    }
    Ok(crate::layer::ctor_classes::value_resource(
        inductive, ctor, arg_names, &args,
    ))
}

/// `ConstRef(iri, levels)` — a reference, at the universe levels it is instantiated at.
fn const_ref(
    names: &CodecNames,
    iri: &str,
    levels: &[crate::nbe::level::Level],
) -> Result<Value, EncodeError> {
    let levels: Result<Vec<Value>, EncodeError> =
        levels.iter().map(|l| encode_level(l, names)).collect();
    term(
        names,
        "ConstRef",
        vec![Value::String(iri.to_string()), Value::Array(levels?)],
    )
}

/// Encode a universe level as a `core:Level` value (eigenius#188).
///
/// The chain ctor took a bare integer until slice 4; it now takes a `Level`, so a `Max`,
/// `IMax` or `Param` survives the round trip instead of being unrepresentable. Numerals
/// encode as the `Succ`-chain they are — `Set` is `Succ(Zero)` — which is more verbose than
/// `1` and is the price of one ctor able to carry every level rather than one declaration
/// per rung.
pub(crate) fn encode_level(
    l: &crate::nbe::level::Level,
    names: &CodecNames,
) -> Result<Value, EncodeError> {
    use crate::nbe::level::Level;
    let lvl = |ctor: &str, args: Vec<Value>| -> Result<Value, EncodeError> {
        let (inductive, arg_names) = names.lookup_in(wk::LEVEL, ctor)?;
        Ok(crate::layer::ctor_classes::value_resource(
            inductive, ctor, arg_names, &args,
        ))
    };
    match l {
        Level::Zero => lvl("Zero", vec![]),
        Level::Succ(a) => lvl("Succ", vec![encode_level(a, names)?]),
        Level::Max(a, b) => lvl(
            "Max",
            vec![encode_level(a, names)?, encode_level(b, names)?],
        ),
        Level::IMax(a, b) => lvl(
            "IMax",
            vec![encode_level(a, names)?, encode_level(b, names)?],
        ),
        Level::Param(n) => lvl("Param", vec![Value::String(n.clone())]),
    }
}

/// Decode an `eigentt:Level` value tree.
///
/// **There is no legacy arm for the pre-eigenius#188 bare integer, deliberately.** One was written
/// and removed: retyping `Sort`'s argument moves the bootstrap manifest, every persisted store
/// then fails to resume with `ManifestDrift`, and the reseed that answers it rewrites the chain
/// from source with this encoder. So no term in the old form can ever reach this function — the
/// arm was a compatibility layer for a state that cannot occur.
/// Decode a `core:Level` value, in either shape.
///
/// The sibling of [`decode_type`] for levels: a tagged dict decodes directly, a value resource
/// (D85 §1) is translated to the tagged form first. Needs the layer for the same reason
/// `decode_type` does — the constructor is a class the layer derived, and the ARGUMENT ORDER
/// comes from that constructor's declaration rather than from the value.
pub fn decode_level(
    value: &Value,
    _layer: &Layer,
) -> Result<crate::nbe::level::Level, DecodeError> {
    decode_level_value(value)
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

/// Errors raised when a chain-resident `eigentt:Term` value cannot
/// be decoded back to an `Exp`.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// The value isn't a well-formed inductive value resource.
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
            DecodeError::MalformedValue(s) => write!(f, "malformed eigentt:Term value: {s}"),
            DecodeError::MissingCtor => write!(f, "eigentt:Term value missing `ctor` field"),
            DecodeError::MissingArgs => write!(f, "eigentt:Term value missing `args` field"),
            DecodeError::UnknownCtor(c) => write!(f, "unknown eigentt:Term ctor: `{c}`"),
            DecodeError::WrongArgCount {
                ctor,
                expected,
                actual,
            } => write!(
                f,
                "eigentt:Term ctor `{ctor}` expects {expected} arg(s), got {actual}"
            ),
            DecodeError::WrongArgShape {
                ctor,
                slot,
                details,
            } => write!(f, "eigentt:Term ctor `{ctor}` arg {slot}: {details}"),
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

/// Context threaded through the recursive decoder: the layer for `ConstRef`
/// resolution.
///
/// **It used to carry a self-reference stub too** — an `Arc<InductiveDecl>` for
/// the in-construction inductive, so that a `ConstRef` naming it short-circuited
/// instead of calling `resolve_inductive_type` and looping unboundedly while
/// decoding that inductive's own constructor types. D76 Phase B removes the need:
/// a reference decodes to `Exp::Const`/`Exp::InductiveCtor` carrying the IRI, so
/// nothing is resolved here and there is nothing to recurse into.
#[derive(Clone, Copy)]
struct DecodeCtx<'a> {
    layer: &'a Layer,
}

/// Decode a chain-resident `eigentt:Term` value back to an
/// EigenTT `Exp`.
///
/// An `App` spine over a `ConstRef` decodes to the same spine over an
/// `Exp::Const` — the wire's currying convention (D47 §3.1) and the term's are the
/// same shape since D76 Phase B, so no folding happens here any more.
pub fn decode_type(value: &Value, layer: &Layer) -> Result<Exp, DecodeError> {
    let ctx = DecodeCtx { layer };
    match value {
        Value::Embedded(r) => decode_value(r, &ctx),
        other => Err(DecodeError::MalformedValue(format!(
            "expected a value resource, got {other:?}"
        ))),
    }
}

/// One argument of a constructor, decoded as a term.
fn decode_arg(v: &Value, ctx: &DecodeCtx<'_>) -> Result<Exp, DecodeError> {
    match v {
        Value::Embedded(r) => decode_value(r, ctx),
        other => Err(DecodeError::MalformedValue(format!(
            "expected a term argument, got {other:?}"
        ))),
    }
}

/// A `core:Level` value.
fn decode_level_value(v: &Value) -> Result<crate::nbe::level::Level, DecodeError> {
    use crate::nbe::level::Level;
    let bad = |m: String| DecodeError::MalformedValue(m);
    let Value::Embedded(r) = v else {
        return Err(bad(format!("expected a level value, got {v:?}")));
    };
    let class = r
        .is_a()
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| bad("a level value must name its constructor's class".into()))?;
    let ctor = class
        .rsplit_once('-')
        .map(|(_, c)| c.to_string())
        .ok_or_else(|| bad(format!("`{class}` is not `<inductive>-<ctor>`")))?;
    let arg = |name: &str| -> Result<Value, DecodeError> {
        Iri::parse(&format!("{class}-{name}"))
            .ok()
            .and_then(|k| r.get(&k).cloned())
            .ok_or_else(|| bad(format!("`{ctor}` is missing argument `{name}`")))
    };
    Ok(match ctor.as_str() {
        "Zero" => Level::Zero,
        "Succ" => Level::Succ(Box::new(decode_level_value(&arg("base")?)?)),
        "Max" => Level::Max(
            Box::new(decode_level_value(&arg("left")?)?),
            Box::new(decode_level_value(&arg("right")?)?),
        ),
        "IMax" => Level::IMax(
            Box::new(decode_level_value(&arg("left")?)?),
            Box::new(decode_level_value(&arg("right")?)?),
        ),
        "Param" => Level::Param(
            arg("name")?
                .as_str()
                .ok_or_else(|| bad("`Param`'s name must be a string".into()))?
                .to_string(),
        ),
        other => return Err(bad(format!("`{other}` is not a `core:Level` constructor"))),
    })
}

/// The constructor a value states, and its arguments in DECLARATION order.
///
/// The one read of an inductive value: `is_a` names the constructor's class, the class names
/// its inductive, and the inductive's `core:ctors` gives the argument names and their order.
/// Everything that consumes a value goes through here — the decoder, the printer, the
/// institutions — so there is one description of how a value is taken apart.
pub fn ctor_and_args<'a>(
    r: &'a Resource,
    layer: &Layer,
) -> Result<(String, Vec<&'a Value>), DecodeError> {
    use crate::ontology::well_known as wk;
    let bad = |m: String| DecodeError::MalformedValue(m);

    let class_iri = r.is_a().first().cloned().ok_or_else(|| {
        bad("a value resource must name its constructor's class in `is_a`".to_string())
    })?;
    let class = layer.resolve(&class_iri).ok_or_else(|| {
        bad(format!(
            "`is_a` names `{class_iri}`, which does not resolve"
        ))
    })?;

    let inductive_iri = class
        .get(&wk::iri(wk::PARENT_CLASSES))
        .and_then(|v| v.as_iri_array().first().cloned())
        .ok_or_else(|| {
            bad(format!(
                "`{class_iri}` is not a constructor class — no `subclass_of`"
            ))
        })?;
    let ctor_name = class_iri
        .as_str()
        .strip_prefix(&format!("{inductive_iri}-"))
        .ok_or_else(|| {
            bad(format!(
                "`{class_iri}` is not named `{inductive_iri}-<ctor>`"
            ))
        })?
        .to_string();

    let inductive = layer
        .resolve(&inductive_iri)
        .ok_or_else(|| bad(format!("`{inductive_iri}` does not resolve")))?;
    let ctor = match inductive.get(&wk::iri(wk::CTORS)) {
        Some(Value::Array(cs)) => cs.iter().find_map(|c| match c {
            Value::Embedded(d)
                if d.get(&wk::iri(wk::CTOR_NAME)).and_then(|v| v.as_str())
                    == Some(ctor_name.as_str()) =>
            {
                Some(d.clone())
            }
            _ => None,
        }),
        _ => None,
    }
    .ok_or_else(|| bad(format!("`{inductive_iri}` declares no ctor `{ctor_name}`")))?;

    let mut args = Vec::new();
    if let Some(Value::Array(arg_types)) = ctor.get(&wk::iri(wk::ARG_TYPES)) {
        for (i, at) in arg_types.iter().enumerate() {
            let arg_name = match at {
                Value::Embedded(a) => a
                    .get(&wk::iri(wk::ARG_NAME))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("arg_{i}")),
                _ => format!("arg_{i}"),
            };
            let prop = Iri::parse(&format!("{class_iri}-{arg_name}"))
                .map_err(|e| bad(format!("bad derived property IRI: {e}")))?;
            let v = r.get(&prop).ok_or_else(|| {
                bad(format!(
                    "value of `{ctor_name}` is missing argument `{arg_name}`"
                ))
            })?;
            args.push(v);
        }
    }
    Ok((ctor_name, args))
}

/// The constructor view as JSON: `{ctor, args}`, for consumers written against that shape.
pub fn ctor_view(r: &Resource, layer: &Layer) -> Result<serde_json::Value, DecodeError> {
    let (ctor_name, args) = ctor_and_args(r, layer)?;
    let args: Result<Vec<serde_json::Value>, DecodeError> =
        args.iter().map(|a| arg_value_to_json(a, layer)).collect();
    Ok(serde_json::json!({ "ctor": ctor_name, "args": args? }))
}

/// One argument of a value resource, as the `{ctor, args}` view expects it.
///
/// A nested inductive value recurses; a primitive passes through.
fn arg_value_to_json(v: &Value, layer: &Layer) -> Result<serde_json::Value, DecodeError> {
    Ok(match v {
        Value::Embedded(r) => ctor_view(r, layer)?,
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Integer(i) => serde_json::Value::from(*i),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|x| arg_value_to_json(x, layer))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        other => {
            return Err(DecodeError::MalformedValue(format!(
                "argument value has no tagged-dict form: {other:?}"
            )))
        }
    })
}

/// Decode a value resource into an `Exp`.
///
/// Reads the constructor and its arguments through `ctor_and_args` — the one read of an
/// inductive value — and dispatches. Arguments arrive as `Value`, not JSON: a resource is
/// destructured directly, so nothing is serialised on the way in and no number loses its type.
fn decode_value(r: &Resource, ctx: &DecodeCtx<'_>) -> Result<Exp, DecodeError> {
    let (ctor, args) = ctor_and_args(r, ctx.layer)?;
    let ctor = ctor.as_str();
    let args: &[&Value] = &args;
    match ctor {
        "Sort" => {
            expect_arg_count("Sort", 1, args)?;
            Ok(Exp::Sort(decode_level_value(args[0])?))
        }
        "Var" => {
            expect_arg_count("Var", 1, args)?;
            let name = arg_string("Var", 0, args[0])?;
            Ok(Exp::Var(name))
        }
        "Ann" => {
            // Type annotation `(e : T)` — the bidirectional mode switch.
            expect_arg_count("Ann", 2, args)?;
            let e = decode_arg(args[0], ctx)?;
            let t = decode_arg(args[1], ctx)?;
            Ok(Exp::Ann(Box::new(e), Box::new(t)))
        }
        "One" => {
            expect_arg_count("One", 0, args)?;
            Ok(Exp::One)
        }
        "Pi" => {
            expect_arg_count("Pi", 3, args)?;
            let name = arg_string("Pi", 0, args[0])?;
            let dom = decode_arg(args[1], ctx)?;
            let body = decode_arg(args[2], ctx)?;
            let patt = if name.is_empty() {
                Patt::Unit
            } else {
                Patt::Var(name)
            };
            Ok(Exp::Pi(patt, Box::new(dom), Box::new(body)))
        }
        "Sig" => {
            expect_arg_count("Sig", 3, args)?;
            let name = arg_string("Sig", 0, args[0])?;
            let dom = decode_arg(args[1], ctx)?;
            let body = decode_arg(args[2], ctx)?;
            let patt = if name.is_empty() {
                Patt::Unit
            } else {
                Patt::Var(name)
            };
            Ok(Exp::Sig(patt, Box::new(dom), Box::new(body)))
        }
        "Record" => {
            expect_arg_count("Record", 1, args)?;
            let items = args[0]
                .as_array()
                .ok_or_else(|| DecodeError::WrongArgShape {
                    ctor: "Record",
                    slot: 0,
                    details: "expected an array of fields".into(),
                })?;
            let mut fields = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let triple = item.as_array().filter(|t| t.len() == 3).ok_or_else(|| {
                    DecodeError::WrongArgShape {
                        ctor: "Record",
                        slot: i,
                        details: "field must be [iri, binder, type]".into(),
                    }
                })?;
                let iri_str = triple[0]
                    .as_str()
                    .ok_or_else(|| DecodeError::WrongArgShape {
                        ctor: "Record",
                        slot: i,
                        details: "iri must be a string".into(),
                    })?;
                let iri = crate::ontology::iri::Iri::parse(iri_str).map_err(|e| {
                    DecodeError::WrongArgShape {
                        ctor: "Record",
                        slot: i,
                        details: format!("bad iri `{iri_str}`: {e}"),
                    }
                })?;
                let name = triple[1]
                    .as_str()
                    .ok_or_else(|| DecodeError::WrongArgShape {
                        ctor: "Record",
                        slot: i,
                        details: "binder must be a string".into(),
                    })?;
                let patt = if name.is_empty() {
                    Patt::Unit
                } else {
                    Patt::Var(name.to_string())
                };
                fields.push((iri, patt, decode_arg(&triple[2], ctx)?));
            }
            // Rebuild through the canonicalising constructor rather than
            // trusting the wire order (D78 §1).
            // Rebuilding through the canonicalising constructor also rejects
            // cycles and duplicates that the wire form could carry.
            Exp::record(fields).map_err(|e| DecodeError::WrongArgShape {
                ctor: "Record",
                slot: 0,
                details: e.to_string(),
            })
        }
        "Refine" => {
            expect_arg_count("Refine", 2, args)?;
            let carrier = decode_arg(args[0], ctx)?;
            let names = args[1]
                .as_array()
                .ok_or_else(|| DecodeError::WrongArgShape {
                    ctor: "Refine",
                    slot: 1,
                    details: "expected an array of class IRIs".into(),
                })?;
            let mut classes = std::collections::BTreeSet::new();
            for (i, n) in names.iter().enumerate() {
                let s = n.as_str().ok_or_else(|| DecodeError::WrongArgShape {
                    ctor: "Refine",
                    slot: 1,
                    details: format!("class {i} must be a string"),
                })?;
                classes.insert(crate::ontology::iri::Iri::parse(s).map_err(|e| {
                    DecodeError::WrongArgShape {
                        ctor: "Refine",
                        slot: 1,
                        details: format!("bad class iri `{s}`: {e}"),
                    }
                })?);
            }
            // `Refine(R, ∅) = R` — one representation, matching eval.
            if classes.is_empty() {
                Ok(carrier)
            } else {
                Ok(Exp::Refine(Box::new(carrier), classes))
            }
        }
        "Lam" => {
            expect_arg_count("Lam", 3, args)?;
            let name = arg_string("Lam", 0, args[0])?;
            // The dom annotation is decoded for round-trip-fidelity validation
            // but discarded — Exp::Lam doesn't carry a type slot.
            let _dom = decode_arg(args[1], ctx)?;
            let body = decode_arg(args[2], ctx)?;
            let patt = if name.is_empty() {
                Patt::Unit
            } else {
                Patt::Var(name)
            };
            Ok(Exp::Lam(patt, Box::new(body)))
        }
        "Id" => {
            expect_arg_count("Id", 3, args)?;
            let ty = decode_arg(args[0], ctx)?;
            let lhs = decode_arg(args[1], ctx)?;
            let rhs = decode_arg(args[2], ctx)?;
            Ok(Exp::Id(Box::new(ty), Box::new(lhs), Box::new(rhs)))
        }
        "App" => {
            expect_arg_count("App", 2, args)?;
            let head = decode_arg(args[0], ctx)?;
            let arg = decode_arg(args[1], ctx)?;
            // D66: the head resolved from a transparent `eigentt:Definition`, so it is that
            // definition's lambda chain. Peel and substitute instead of building an `App` — the
            // redex is never formed, so the result is normal (§2.4).
            if is_definition_head(args[0], ctx) {
                return peel_and_substitute(head, arg);
            }
            // Spine folding: if head is an InductiveType / CodataType /
            // InductiveCtor, append arg to its args list. Otherwise
            // produce a plain App.
            match head {
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
            // One or two arguments: the IRI, and optionally its level arguments.
            // See the encode arm — the second is emitted only when non-empty, so a
            // monomorphic reference is byte-identical to what shipped before
            // eigenius#188's residual.
            if args.len() != 1 && args.len() != 2 {
                return Err(DecodeError::WrongArgCount {
                    ctor: "ConstRef",
                    expected: 1,
                    actual: args.len(),
                });
            }
            let iri_str = arg_string("ConstRef", 0, args[0])?;
            let iri = Iri::parse(&iri_str).map_err(|e| {
                wrong_shape("ConstRef", 0, &format!("invalid IRI `{iri_str}`: {e}"))
            })?;
            let levels: Vec<crate::nbe::level::Level> = match args.get(1) {
                None => Vec::new(),
                Some(Value::Array(ls)) => ls
                    .iter()
                    .map(decode_level_value)
                    .collect::<Result<Vec<_>, _>>()?,
                Some(other) => {
                    return Err(wrong_shape(
                        "ConstRef",
                        1,
                        &format!("level arguments must be an array, got {other:?}"),
                    ))
                }
            };
            if levels.is_empty() {
                resolve_const_ref(iri, ctx)
            } else {
                // A level-carrying reference names its declaration and keeps its
                // arguments; it does not unfold here. Resolution and instantiation
                // happen in `Γ_env`, which is the whole point of D76 — the decoder
                // deciding it would be the inline-the-environment antipattern with
                // levels attached.
                Ok(Exp::Const(iri, levels))
            }
        }

        // ── D48 / eigenius#71 — term-level value decoding ─────────
        "UnitVal" => {
            expect_arg_count("UnitVal", 0, args)?;
            Ok(Exp::Unit)
        }
        "Pair" => {
            expect_arg_count("Pair", 2, args)?;
            let fst = decode_arg(args[0], ctx)?;
            let snd = decode_arg(args[1], ctx)?;
            Ok(Exp::Pair(Box::new(fst), Box::new(snd)))
        }
        "Fst" => {
            expect_arg_count("Fst", 1, args)?;
            Ok(Exp::Fst(Box::new(decode_arg(args[0], ctx)?)))
        }
        "Snd" => {
            expect_arg_count("Snd", 1, args)?;
            Ok(Exp::Snd(Box::new(decode_arg(args[0], ctx)?)))
        }
        "CtorApp" => {
            expect_arg_count("CtorApp", 2, args)?;
            let decl_iri_str = arg_string("CtorApp", 0, args[0])?;
            let ctor_name = arg_string("CtorApp", 1, args[1])?;
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
            //
            // **D76 Phase B — no resolution here.** This called
            // `resolve_inductive_decl_for_ctor`, which decoded the ENTIRE target
            // inductive, every constructor type of it, to fill a declaration slot
            // the node no longer has; and short-circuited to the self-reference stub
            // when the target was the inductive being assembled, because otherwise
            // it recursed unboundedly. It also verified the constructor name — which
            // the type checker does anyway (`check_ctor_unknown_name`), and does with
            // the environment in hand rather than at decode time.
            Ok(Exp::InductiveCtor(decl_iri, ctor_name, Vec::new()))
        }
        // eigenius#71 — literal primitive values. The matching encode
        // arms emit `{"ctor": "LitString", "args": [<json-value>]}`
        // etc.; here we extract the JSON primitive and wrap it in the
        // corresponding `Exp::Lit*` variant. The arg is taken
        // positionally per the codec's standard ctor shape (args[0]
        // is the literal payload); the JSON type of the payload
        // determines which arm we landed in, but we still type-check
        // it defensively in case a malformed payload reached commit
        // (the validator at canonical_proposition emission time should
        // already have caught this — D49 Phase 5 — but the codec is
        // the last line of defence).
        // D87 §4.2. Decoding is total, as it must be — the ctor is on the chain, so a value
        // carrying it has to read back. What refuses a hand-authored one is `check`, which has no
        // proof of the proposition and will not manufacture one.
        "Checked" => {
            expect_arg_count("Checked", 1, args)?;
            let iri_str = arg_string("Checked", 0, args[0])?;
            let iri = Iri::parse(&iri_str)
                .map_err(|e| wrong_shape("Checked", 0, &format!("invalid IRI `{iri_str}`: {e}")))?;
            Ok(Exp::Checked(iri))
        }
        "LitString" => {
            expect_arg_count("LitString", 1, args)?;
            let s = args[0]
                .as_str()
                .ok_or_else(|| {
                    DecodeError::MalformedValue(format!(
                        "LitString arg must be a JSON string, got {:?}",
                        args[0]
                    ))
                })?
                .to_string();
            Ok(Exp::LitString(s))
        }
        "LitInt" => {
            expect_arg_count("LitInt", 1, args)?;
            let n = args[0].as_integer().ok_or_else(|| {
                DecodeError::MalformedValue(format!(
                    "LitInt arg must be a JSON integer, got {:?}",
                    args[0]
                ))
            })?;
            Ok(Exp::LitInt(n))
        }
        "LitFloat" => {
            expect_arg_count("LitFloat", 1, args)?;
            let f = args[0].as_float().ok_or_else(|| {
                DecodeError::MalformedValue(format!(
                    "LitFloat arg must be a JSON number, got {:?}",
                    args[0]
                ))
            })?;
            Ok(Exp::LitFloat(f))
        }
        "LitBool" => {
            expect_arg_count("LitBool", 1, args)?;
            let b = args[0].as_boolean().ok_or_else(|| {
                DecodeError::MalformedValue(format!(
                    "LitBool arg must be a JSON boolean, got {:?}",
                    args[0]
                ))
            })?;
            Ok(Exp::LitBool(b))
        }

        other => Err(DecodeError::UnknownCtor(other.to_string())),
    }
}

/// `urn:eigenius:eigentt:Definition` — the class decode unfolds (D66).
fn definition_class_iri() -> Iri {
    Iri::parse("urn:eigenius:eigentt:Definition").expect("valid IRI")
}
fn definition_body_iri() -> Iri {
    Iri::parse("urn:eigenius:eigentt:definition_body").expect("valid IRI")
}
fn definition_opaque_iri() -> Iri {
    Iri::parse("urn:eigenius:eigentt:definition_opaque").expect("valid IRI")
}

/// Absent means transparent — the common case, so it is the default.
fn definition_is_opaque(resource: &crate::ontology::resource::Resource) -> bool {
    matches!(
        resource.get(&definition_opaque_iri()),
        Some(crate::ontology::resource::Value::Boolean(true))
    )
}

/// Is this decoded head a transparent definition's body — i.e. a lambda chain the App spine should
/// peel rather than apply?
///
/// Peeling is gated on the head having come from a `Definition`, not on it merely *being* a `Lam`.
/// Reducing any `App(Lam, _)` at decode would change the hash of every stored proposition that
/// happens to contain a redex, which is a separate decision from this feature (D66 §2.4 specifies
/// the narrower rule).
fn is_definition_head(head: &Value, ctx: &DecodeCtx<'_>) -> bool {
    let mut cursor = head;
    loop {
        let Value::Embedded(r) = cursor else {
            return false;
        };
        let Ok((ctor, args)) = ctor_and_args(r, ctx.layer) else {
            return false;
        };
        match ctor.as_str() {
            "App" => {
                let Some(next) = args.first() else {
                    return false;
                };
                cursor = next;
            }
            "ConstRef" => {
                let Some(iri_str) = args.first().and_then(|v| v.as_str()) else {
                    return false;
                };
                let Ok(iri) = Iri::parse(iri_str) else {
                    return false;
                };
                return ctx
                    .layer
                    .resolve(&iri)
                    .filter(|r| r.is_a().contains(&definition_class_iri()))
                    .is_some_and(|r| !definition_is_opaque(&r));
            }
            _ => return false,
        }
    }
}

/// Peel one leading `Lam` off `head` and substitute `arg` into its body (D66 §2.4 / D8).
///
/// No redex is formed, so the result stays normal — which is what lets both ends of the witness key
/// hash the same term without evaluating (D9). Under-application simply stops early: three binders
/// applied to one argument leaves a two-binder `Lam`, still normal.
fn peel_and_substitute(head: Exp, arg: Exp) -> Result<Exp, DecodeError> {
    match head {
        Exp::Lam(crate::nbe::term::Patt::Var(name), body) => {
            crate::nbe::subst::subst(&body, name.as_str(), &arg).map_err(|e| {
                DecodeError::AppOnNonParametric(format!("definition instantiation: {e}"))
            })
        }
        // A wildcard binder discards its argument.
        Exp::Lam(_, body) => Ok(*body),
        other => Ok(Exp::App(Box::new(other), Box::new(arg))),
    }
}

fn resolve_const_ref(iri: Iri, ctx: &DecodeCtx<'_>) -> Result<Exp, DecodeError> {
    // D76 Phase B — no self-reference short-circuit is needed any more. It existed
    // because resolving a `ConstRef` produced a *declaration*, so decoding a ctor
    // body that mentioned its own inductive re-entered `resolve_inductive_type`
    // and recursed unboundedly; the stub was the base case. A reference now
    // resolves to `Exp::Const(iri, …)`, which names the declaration without
    // decoding it, so there is nothing to recurse into.
    // Primitive IRIs short-circuit to `Exp::EigonPrimitive` ahead of the
    // layer lookup. The five core primitive `DataType` resources resolve
    // to the corresponding primitive enum value; without this, the
    // datatype-rejection branch below would refuse them. Mirrors the
    // same mapping in `ground::decode_arg_type`.
    use crate::nbe::term::PrimitiveType;
    use crate::ontology::well_known as wk;
    match iri.as_str() {
        wk::STRING => return Ok(Exp::EigonPrimitive(PrimitiveType::String)),
        wk::INTEGER => return Ok(Exp::EigonPrimitive(PrimitiveType::Integer)),
        wk::FLOAT => return Ok(Exp::EigonPrimitive(PrimitiveType::Float)),
        wk::BOOLEAN => return Ok(Exp::EigonPrimitive(PrimitiveType::Boolean)),
        wk::JSON => return Ok(Exp::EigonPrimitive(PrimitiveType::Json)),
        _ => {}
    }
    let resource = ctx
        .layer
        .resolve(&iri)
        .ok_or_else(|| DecodeError::UnresolvedConstRef(iri.clone()))?;
    let class_iris: Vec<Iri> = resource.is_a().to_vec();
    let class_iri = wk::iri(wk::CLASS);
    let datatype_iri = wk::iri(wk::DATA_TYPE);
    let inductive_iri = wk::iri(wk::INDUCTIVE_TYPE);
    // D46 §10 axiom IRI — an opaque chain-resident `eigentt:Axiom`
    // resource. Its registered type is looked up at `check_infer`
    // time via the layer's cached `axiom_env`; here we just emit the
    // reference so the decoded `Exp` carries the IRI through.
    let axiom_iri = Iri::parse("urn:eigenius:eigentt:Axiom")
        .expect("urn:eigenius:eigentt:Axiom is a valid IRI");
    if class_iris.contains(&axiom_iri) {
        return Ok(Exp::EigonAxiom(iri));
    }
    // D66 — a chain-resident `eigentt:Definition`. A TRANSPARENT one decodes to its stored body
    // (a lambda chain); the enclosing App spine then peels and substitutes, so a use lands on a
    // normal term without forming a redex. An OPAQUE one behaves exactly like an axiom: rigid,
    // never unfolded, identity is the folded name (#95 / D66 D9 carve-out).
    if class_iris.contains(&definition_class_iri()) {
        if definition_is_opaque(&resource) {
            return Ok(Exp::EigonAxiom(iri));
        }
        let body = resource.get(&definition_body_iri()).ok_or_else(|| {
            DecodeError::ConstRefWrongClass {
                iri: iri.clone(),
                found_classes: class_iris.clone(),
            }
        })?;
        return decode_type(body, ctx.layer);
    }
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
        // The name, not the declaration (D76 Phase B). This used to decode the
        // whole thing — params, indices, every constructor type — to drop it into
        // a slot the term no longer has. The type checker resolves it through
        // `Γ_env` when it needs it, and memoizes.
        Ok(Exp::Const(iri, Vec::new()))
    } else {
        // A resolved resource that is none of axiom/class/inductive/datatype — a plain
        // term-level *individual* (an `Entity` value). The dual of the encode-side `EigonResource →
        // ConstRef` arm: a proposition may reference a named individual (`hela`). A misuse in a *type*
        // position is caught downstream by the type-checker — the same deferral `EigonClass`/
        // `EigonAxiom` already rely on — so the codec need not gate it here. (`resolve` above already
        // rejected an unresolved IRI as `UnresolvedConstRef`, so `resource` is a real chain resource.)
        Ok(Exp::EigonResource(Box::new(resource.as_ref().clone())))
    }
}

fn expect_arg_count(
    ctor: &'static str,
    expected: usize,
    args: &[&Value],
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

fn arg_string(ctor: &'static str, slot: usize, v: &Value) -> Result<String, DecodeError> {
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

/// Build a `justification:Certificate(j, P)` TYPE from encoded indices.
///
/// The inverse of [`certificate_indices`]. An indexed inductive applied to its
/// indices encodes as nested `App`s over a `ConstRef` head.
pub fn certificate_type(j: &Value, p: &Value, names: &CodecNames) -> Result<Value, EncodeError> {
    let head = const_ref(names, "urn:eigenius:justification:Certificate", &[])?;
    let one = term(names, "App", vec![head, j.clone()])?;
    term(names, "App", vec![one, p.clone()])
}

/// Build an `eigentt:Judgement` value — `holds(logic, term, type)` — from an
/// `eigentt:Logic` individual and two encoded terms.
///
/// The inverse of [`decode_judgement`]. A constructor application encodes as
/// `App`s folded over a `CtorApp` base, which is the D47 shape for any
/// chain-declared inductive.
pub fn encode_judgement(
    logic_iri: &str,
    term: &Value,
    typ: &Value,
    names: &CodecNames,
) -> Result<Value, EncodeError> {
    // `holds(logic, term, typ)`, App-curried. `term` and `typ` arrive ALREADY encoded, so this
    // one assembles value resources directly rather than going through the tagged form —
    // there is no tagged tree to convert, only two encoded operands to apply.
    // `logic` is declared `eigentt:Logic` — a REFERENCE to the logic individual, not a term —
    // so it is the IRI, not a `ConstRef` around it. In the tagged form the distinction had
    // nowhere to live; the derived property carries `class_types: [eigentt:Logic]` and Rule 8
    // checks it.
    let logic = Value::String(logic_iri.to_string());
    let (inductive, arg_names) = names.lookup_in(JUDGEMENT_IRI, "holds")?;
    Ok(crate::layer::ctor_classes::value_resource(
        inductive,
        "holds",
        arg_names,
        &[logic, term.clone(), typ.clone()],
    ))
}

/// The three fields of a committed `eigentt:Judgement` value: the logic whose
/// checker ran, the term it checked, and the type it checked against.
#[derive(Debug, Clone)]
pub struct Judgement {
    /// IRI of the `eigentt:Logic` individual naming the checker.
    pub logic: Iri,
    /// The checked term.
    pub term: Exp,
    /// The type it was checked against.
    pub typ: Exp,
}

/// Decode a stored `eigentt:Judgement` value into its three fields.
///
/// A judgement is `holds(logic, term, type)` — an ordinary constructor
/// application, so it decodes through [`decode_type`] like any other term and
/// this only names the parts.
pub fn decode_judgement(value: &Value, layer: &Layer) -> Result<Judgement, DecodeError> {
    // A judgement VALUE names its own constructor (D85 §6.1) and its three arguments are
    // properties, so it is read here rather than folded through the term language and matched
    // back out of an `App` spine.
    if let Value::Embedded(r) = value {
        let holds = crate::layer::ctor_classes::class_iri(JUDGEMENT_IRI, "holds");
        if r.is_a().iter().any(|i| i.as_str() == holds) {
            let arg = |n: &str| {
                Iri::parse(&crate::layer::ctor_classes::arg_property_iri(&holds, n))
                    .ok()
                    .and_then(|k| r.get(&k).cloned())
            };
            let logic_v = arg("logic").ok_or_else(|| {
                DecodeError::MalformedValue("a judgement is missing `logic`".into())
            })?;
            let logic = logic_v
                .as_str()
                .and_then(|s| Iri::parse(s).ok())
                .ok_or_else(|| {
                    DecodeError::MalformedValue(format!(
                        "a judgement's `logic` must be an IRI reference, got {logic_v:?}"
                    ))
                })?;
            let term_v = arg("term").ok_or_else(|| {
                DecodeError::MalformedValue("a judgement is missing `term`".into())
            })?;
            let type_v = arg("type").ok_or_else(|| {
                DecodeError::MalformedValue("a judgement is missing `type`".into())
            })?;
            return Ok(Judgement {
                logic,
                term: decode_type(&term_v, layer)?,
                typ: decode_type(&type_v, layer)?,
            });
        }
    }
    let exp = decode_type(value, layer)?;
    match &exp {
        Exp::InductiveCtor(_, name, args) if name.as_str() == "holds" && args.len() == 3 => {
            let logic = match &args[0] {
                // An `eigentt:Logic` inhabitant is a RESOURCE, so a reference
                // to one decodes to `EigonResource` carrying the whole record —
                // not to a `Const`. That is a consequence of Logic being a
                // class with individuals rather than an inductive with nullary
                // constructors, and it is the shape this has to read.
                Exp::EigonResource(r) => match r.id() {
                    Some(iri) => iri.clone(),
                    None => {
                        return Err(DecodeError::MalformedValue(
                            "a judgement's logic names an embedded resource with no @id"
                                .to_string(),
                        ))
                    }
                },
                Exp::Const(iri, _) | Exp::EigonClass(iri) => iri.clone(),
                Exp::InductiveCtor(iri, _, _) => iri.clone(),
                other => {
                    return Err(DecodeError::MalformedValue(format!(
                        "a judgement's logic must name an eigentt:Logic individual, got {other:?}"
                    )))
                }
            };
            Ok(Judgement {
                logic,
                term: args[1].clone(),
                typ: args[2].clone(),
            })
        }
        other => Err(DecodeError::MalformedValue(format!(
            "expected a judgement `holds(logic, term, type)`, got {other:?}"
        ))),
    }
}

/// Project the two indices out of a `justification:Certificate(j, P)` type.
///
/// A certificate type is the indexed inductive applied to its two indices, so
/// it reaches here as `App(App(Const(Certificate), j), P)` — the shape D76
/// Phase B leaves for a type former applied to arguments.
///
/// This is what lets a conclusion's proposition be recovered from its
/// judgement rather than stored in a second slot. The emit and check sides
/// must agree on the result: the witness index hashes `P` projected out here,
/// while a citing certificate's `verified(iri, P)` supplies `P` directly, and
/// a mismatch does not error — it silently fails to admit the witness.
pub fn certificate_indices(typ: &Exp) -> Option<(&Exp, &Exp)> {
    let (inner, p) = match typ {
        Exp::App(f, a) => (f.as_ref(), a.as_ref()),
        _ => return None,
    };
    let (head, j) = match inner {
        Exp::App(f, a) => (f.as_ref(), a.as_ref()),
        _ => return None,
    };
    let names_certificate = matches!(
        head,
        Exp::Const(iri, _) | Exp::EigonClass(iri) | Exp::EigonAxiom(iri)
            if iri.as_str() == "urn:eigenius:justification:Certificate"
    );
    names_certificate.then_some((j, p))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    /// Materialise a tagged literal as the value resources it denotes — a FIXTURE builder.
    ///
    /// These tests describe terms as `{"ctor": …, "args": […]}` because that reads well in a
    /// literal. The values themselves are resources (D85 §6.1), so the literal is built out
    /// through the declaration, which means a fixture cannot name a constructor or an arity
    /// the chain does not have.
    pub(super) fn value_of(tagged: &serde_json::Value) -> Value {
        let names = crate::testing::codec_names();
        let Some(ctor) = tagged.get("ctor").and_then(serde_json::Value::as_str) else {
            return match tagged {
                serde_json::Value::String(s) => Value::String(s.clone()),
                serde_json::Value::Bool(b) => Value::Boolean(*b),
                serde_json::Value::Array(a) => Value::Array(a.iter().map(value_of).collect()),
                serde_json::Value::Number(n) => match n.as_i64() {
                    Some(i) => Value::Integer(i),
                    None => Value::Float(n.as_f64().unwrap_or_default()),
                },
                other => Value::Json(other.clone()),
            };
        };
        let empty = Vec::new();
        let args: Vec<Value> = tagged
            .get("args")
            .and_then(serde_json::Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .map(value_of)
            .collect();
        let inductive = if ["Zero", "Succ", "Max", "IMax", "Param"].contains(&ctor) {
            wk::LEVEL
        } else {
            wk::EIGENTT_TERM
        };
        names
            .value(inductive, ctor, args)
            .unwrap_or_else(|e| panic!("fixture names a constructor the chain lacks: {e}"))
    }

    /// Encode, then project to `{ctor, args}` — for assertions written as `j["ctor"]`.
    ///
    /// The encoder produces value resources; [`ctor_view`] reads one back. It is a test
    /// convenience, not a second encoding.
    fn tagged(exp: &Exp) -> Result<serde_json::Value, EncodeError> {
        let v = encode_type(exp, crate::testing::codec_names())?;
        match &v {
            Value::Embedded(r) => Ok(ctor_view(r, crate::testing::term_chain())
                .expect("a freshly encoded value projects")),
            other => Ok(serde_json::json!(format!("{other:?}"))),
        }
    }

    use super::*;
    use crate::nbe::term::{InductiveCtorDecl, InductiveDecl};
    use std::sync::Arc;

    fn ctor_obj(name: &str, args: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"ctor": name, "args": args})
    }

    #[test]
    fn encodes_sort() {
        // eigenius#188: `Sort`'s argument is an `eigentt:Level` tree, not a numeral. `Prop` is
        // `Zero`; `Set` is `Succ(Zero)`.
        let v = tagged(&Exp::sort(0)).unwrap();
        assert_eq!(v, ctor_obj("Sort", vec![ctor_obj("Zero", vec![])]));
        let v = tagged(&Exp::sort(1)).unwrap();
        assert_eq!(
            v,
            ctor_obj(
                "Sort",
                vec![ctor_obj("Succ", vec![ctor_obj("Zero", vec![])])]
            )
        );
    }

    /// **A polymorphic level survives the round trip** — the point of eigenius#188. Under the
    /// numeral encoding a `Max`, `IMax` or `Param` was simply unrepresentable on the chain.
    #[test]
    fn round_trips_a_polymorphic_level() {
        use crate::nbe::level::Level;
        let l = Level::IMax(
            Box::new(Level::Param("u".to_string())),
            Box::new(Level::Max(
                Box::new(Level::Param("v".to_string())),
                Box::new(Level::of_nat(1)),
            )),
        );
        let layer = empty_layer();
        let encoded = encode_type(&Exp::Sort(l.clone()), crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&encoded, &layer).unwrap();
        assert_eq!(decoded, Exp::Sort(l));
    }

    // --- eigenius#71 / D49 — literal Exp variants ---

    #[test]
    fn encodes_lit_string() {
        let v = tagged(&Exp::LitString("urn:eigenius:example:thing".to_string())).unwrap();
        assert_eq!(
            v,
            ctor_obj("LitString", vec![json!("urn:eigenius:example:thing")])
        );
    }

    #[test]
    fn encodes_lit_int() {
        let v = tagged(&Exp::LitInt(42)).unwrap();
        assert_eq!(v, ctor_obj("LitInt", vec![json!(42)]));
    }

    #[test]
    fn encodes_lit_float() {
        let v = tagged(&Exp::LitFloat(1.5)).unwrap();
        assert_eq!(v, ctor_obj("LitFloat", vec![json!(1.5)]));
    }

    #[test]
    fn encodes_lit_bool() {
        let v = tagged(&Exp::LitBool(true)).unwrap();
        assert_eq!(v, ctor_obj("LitBool", vec![json!(true)]));
    }

    #[test]
    fn lit_string_roundtrip() {
        // No layer chain needed for literals — they decode pure-locally,
        // never touching the chain.
        let layer = empty_layer();
        let original = Exp::LitString("urn:eigenius:example:thing".to_string());
        let encoded = encode_type(&original, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&encoded, &layer).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn ann_roundtrip() {
        // `(P : Prop)` — the bidirectional annotation round-trips through D47.
        let layer = empty_layer();
        let original = Exp::Ann(Box::new(Exp::Var("P".to_string())), Box::new(Exp::sort(0)));
        let encoded = encode_type(&original, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&encoded, &layer).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn lit_int_roundtrip() {
        let layer = empty_layer();
        let original = Exp::LitInt(-42);
        let encoded = encode_type(&original, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&encoded, &layer).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn lit_float_roundtrip() {
        let layer = empty_layer();
        let original = Exp::LitFloat(1.25);
        let encoded = encode_type(&original, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&encoded, &layer).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn lit_bool_roundtrip() {
        let layer = empty_layer();
        for original in [Exp::LitBool(true), Exp::LitBool(false)] {
            let encoded = encode_type(&original, crate::testing::codec_names()).unwrap();
            let decoded = decode_type(&encoded, &layer).unwrap();
            assert_eq!(decoded, original);
        }
    }

    /// eigenius#142 — `LitBool` is purely ADDITIVE to the codec. Terms
    /// already persisted (no version field exists to bump) still decode
    /// to the same `Exp`, and the arg is type-checked like the others:
    /// a JSON string in the slot is rejected, not silently coerced.
    #[test]
    fn lit_bool_is_additive_and_typed() {
        let layer = empty_layer();
        for pre_existing in [
            ctor_obj("LitInt", vec![json!(42)]),
            ctor_obj("LitString", vec![json!("s")]),
            ctor_obj("LitFloat", vec![json!(1.5)]),
            ctor_obj(
                "Sort",
                vec![ctor_obj("Succ", vec![ctor_obj("Zero", vec![])])],
            ),
            ctor_obj("UnitVal", vec![]),
        ] {
            decode_type(&value_of(&pre_existing.clone()), &layer)
                .unwrap_or_else(|e| panic!("{pre_existing} no longer decodes: {e}"));
        }
        let malformed = ctor_obj("LitBool", vec![json!("true")]);
        match decode_type(&value_of(&malformed), &layer) {
            Err(DecodeError::MalformedValue(msg)) => assert!(msg.contains("LitBool"), "{msg}"),
            other => panic!("expected MalformedValue, got {other:?}"),
        }
    }

    #[test]
    fn lit_string_decode_rejects_non_string_arg() {
        let layer = empty_layer();
        // Authored-by-hand malformed payload: LitString with an int arg.
        let malformed = ctor_obj("LitString", vec![json!(42)]);
        let result = decode_type(&value_of(&malformed), &layer);
        assert!(result.is_err(), "LitString with int arg must reject");
        match result.unwrap_err() {
            DecodeError::MalformedValue(msg) => {
                assert!(msg.contains("LitString"), "diagnostic: {msg}");
            }
            other => panic!("expected MalformedValue, got {other:?}"),
        }
    }

    #[test]
    fn encodes_var() {
        let v = tagged(&Exp::Var("P".to_string())).unwrap();
        assert_eq!(v, ctor_obj("Var", vec![json!("P")]));
    }

    #[test]
    fn encodes_one() {
        let v = tagged(&Exp::One).unwrap();
        assert_eq!(v, ctor_obj("One", vec![]));
    }

    #[test]
    fn encodes_arrow_as_pi_with_empty_binder() {
        // 1 → 1 desugars to Pi(_, 1, 1)
        let exp = Exp::Arrow(Box::new(Exp::One), Box::new(Exp::One));
        let v = tagged(&exp).unwrap();
        let one = ctor_obj("One", vec![]);
        assert_eq!(v, ctor_obj("Pi", vec![json!(""), one.clone(), one],));
    }

    #[test]
    fn encodes_id_in_prop() {
        let exp = Exp::Id(
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
            Box::new(Exp::Var("y".to_string())),
        );
        let v = tagged(&exp).unwrap();
        let one = ctor_obj("One", vec![]);
        let vx = ctor_obj("Var", vec![json!("x")]);
        let vy = ctor_obj("Var", vec![json!("y")]);
        assert_eq!(v, ctor_obj("Id", vec![one, vx, vy]));
    }

    #[test]
    fn encodes_propext_shape() {
        // propext : ∀ {P : Prop} {Q : Prop}, ((P → Q) × (Q → P)) → Id Prop P Q
        // Built from D47 §3.2's worked example.
        let p_var = || Exp::Var("P".to_string());
        let q_var = || Exp::Var("Q".to_string());
        let prop = || Exp::sort(0);
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
        let v = tagged(&propext).unwrap();
        let j = v;
        assert_eq!(j["ctor"], "Pi");
        assert_eq!(j["args"][0], json!("P"));
    }

    #[test]
    fn rejects_lam_without_annotation() {
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let err = encode_type(&lam, crate::testing::codec_names()).unwrap_err();
        assert!(matches!(err, EncodeError::LamWithoutAnnotation));
    }

    #[test]
    fn rejects_non_type_level_exp() {
        // Refl is a term-level form, not a type. Should be rejected.
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let err = encode_type(&refl, crate::testing::codec_names()).unwrap_err();
        assert!(matches!(err, EncodeError::NotATypeLevelExp(_)));
    }

    // ---------- decoder tests ----------

    /// The chain a decode needs.
    ///
    /// It was a genuinely empty root layer while terms were opaque JSON. A term is now a
    /// resource whose `is_a` names its constructor's class (D85 §6.1), and that class is
    /// DERIVED from `eigentt:Term`'s declaration — so a layer without the declaration cannot
    /// decode any term at all, and every one of these round-trips would fail on
    /// "`Term-Ann` does not resolve" rather than on anything the test is about.
    pub(super) fn empty_layer() -> std::sync::Arc<Layer> {
        std::sync::Arc::clone(crate::testing::term_chain())
    }

    fn bootstrap_head() -> std::sync::Arc<Layer> {
        std::sync::Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head())
    }

    #[test]
    fn decodes_sort() {
        let v = encode_type(&Exp::sort(2), crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, Exp::sort(2));
    }

    #[test]
    fn decodes_pi_with_named_binder() {
        let exp = Exp::Pi(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
        );
        let v = encode_type(&exp, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, exp);
    }

    #[test]
    fn decodes_arrow_round_trips_as_pi_unit() {
        let exp = Exp::Arrow(Box::new(Exp::One), Box::new(Exp::One));
        let v = encode_type(&exp, crate::testing::codec_names()).unwrap();
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
        let v = encode_type(&exp, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, exp);
    }

    /// D87 §4.2 — the checked-proof reference survives the codec as itself.
    ///
    /// The property that matters is that it does not come back as an `EigonAxiom`. Both would
    /// round-trip through a `ConstRef`, and a chain that could not tell them apart is the
    /// conflation the former was added to prevent: `Declared(a)` and `Verified(a)` would name the
    /// same resource with the authored justification term as the only discriminator.
    #[test]
    fn a_checked_proof_reference_round_trips_as_itself() {
        let payload = Iri::parse("urn:eigenius:demo:lean:proof_payload").unwrap();
        let exp = Exp::Checked(payload.clone());
        let v = encode_type(&exp, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, exp);
        assert_ne!(
            decoded,
            Exp::EigonAxiom(payload),
            "a checked-proof reference must not decode as an axiom — the whole point of the \
             former is that the chain can tell `asserted without proof` from `checked by nanoda`"
        );
    }

    #[test]
    fn decodes_propext_round_trip() {
        // Re-build propext, encode it, decode it, compare structurally
        // (modulo Arrow→Pi desugaring).
        let p_var = || Exp::Var("P".to_string());
        let q_var = || Exp::Var("Q".to_string());
        let prop = || Exp::sort(0);
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
        let v = encode_type(&propext, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        // The decoded form is the desugared Pi/Sig version of the input.
        // For this round-trip, encode the desugared form and compare values.
        let v2 = encode_type(&decoded, crate::testing::codec_names()).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    /// **The rejection moved from decode to construction.** A term used to be a tagged dict,
    /// which could name any constructor at all, so the decoder had to refuse the ones
    /// `eigentt:Term` does not declare. A value states its constructor's CLASS (D85 §6.1), and
    /// there is no class for `Nonsense` — the term cannot be built in the first place.
    fn an_undeclared_ctor_cannot_be_built() {
        let err = crate::testing::codec_names()
            .value(wk::EIGENTT_TERM, "Nonsense", vec![])
            .expect_err("`eigentt:Term` declares no `Nonsense`");
        assert!(format!("{err}").contains("Nonsense"), "{err}");
    }

    #[test]
    /// Arity likewise: a value carries its arguments as NAMED properties, so a `Sort` with no
    /// level is not a term the builder will make.
    fn a_wrong_arity_cannot_be_built() {
        let err = crate::testing::codec_names()
            .value(wk::EIGENTT_TERM, "Sort", vec![])
            .expect_err("`Sort` takes one argument");
        assert!(
            format!("{err}").contains("takes 1 argument"),
            "expected an arity diagnostic, got {err}"
        );
    }

    #[test]
    fn decoder_rejects_unresolved_constref() {
        let bad = json!({
            "ctor": "ConstRef",
            "args": ["urn:eigenius:nonexistent:Foo", []]
        });
        let err = decode_type(&value_of(&bad.clone()), &empty_layer()).unwrap_err();
        assert!(matches!(err, DecodeError::UnresolvedConstRef(_)));
    }

    #[test]
    fn decoder_resolves_constref_to_eigon_class() {
        // urn:eigenius:core:Class is an is_a-of-Class resource in the
        // core ontology.
        let head = bootstrap_head();
        let v = json!({
            "ctor": "ConstRef",
            "args": ["urn:eigenius:core:Class", []]
        });
        let decoded = decode_type(&value_of(&v), &head).unwrap();
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
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:IxClassFamily").unwrap(),
            name: "urn:_:IxClassFamily".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            // Index telescope's type is Sort(1) — indices are types
            // themselves (e.g., the index says "what type am I
            // indexed by"). This keeps the test purely type-level.
            indices: vec![(Patt::Unit, Exp::sort(1))],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        // `IxClassFamily Some Other` — both param and index are
        // EigonClass IRIs (type-level).
        let app_form = Exp::const_applied(
            ix_decl.iri.clone(),
            Vec::new(),
            vec![
                Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:Some").unwrap()),
                Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:Other").unwrap()),
            ],
        );
        let encoded = tagged(&app_form).expect("encode indexed inductive");
        let j = encoded;
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
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap(),
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        let list_decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:List").unwrap(),
            name: "urn:_:List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "nil".to_string(),
                typ: Exp::const_applied(
                    Arc::new(InductiveDecl {
                        uparams: Vec::new(),
                        iri: crate::ontology::iri::Iri::parse("urn:_:List").unwrap(),
                        name: "urn:_:List".to_string(),
                        params: Vec::new(),
                        indices: Vec::new(),
                        sort: Exp::sort(1),
                        ctors: Vec::new(),
                    })
                    .iri
                    .clone(),
                    Vec::new(),
                    vec![Exp::Var("A".to_string())],
                ),
            }],
        });
        let nat = Exp::const_applied(nat_decl.iri.clone(), Vec::new(), Vec::new());
        let list_nat = Exp::const_applied(list_decl.iri.clone(), Vec::new(), vec![nat]);
        let v = tagged(&list_nat).unwrap();

        let const_nat = ctor_obj("ConstRef", vec![json!("urn:_:Nat"), json!([])]);
        let const_list = ctor_obj("ConstRef", vec![json!("urn:_:List"), json!([])]);
        let expected = ctor_obj("App", vec![const_list, const_nat]);
        assert_eq!(v, expected);
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 / eigenius#71 — term-level encoding (UnitVal, Pair, CtorApp)
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn encodes_unit_value() {
        let v = tagged(&Exp::Unit).unwrap();
        assert_eq!(v, ctor_obj("UnitVal", vec![]));
    }

    #[test]
    fn encodes_pair_value() {
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let v = tagged(&pair).unwrap();
        let unit = ctor_obj("UnitVal", vec![]);
        assert_eq!(v, ctor_obj("Pair", vec![unit.clone(), unit]));
    }

    #[test]
    fn encodes_nullary_inductive_ctor() {
        // Nat.zero — encoded as CtorApp(urn:_:Nat, zero) with no args.
        let nat_decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap(),
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "zero".to_string(),
                typ: Exp::sort(1),
            }],
        });
        let zero = Exp::InductiveCtor(nat_decl.iri.clone(), "zero".to_string(), Vec::new());
        let v = tagged(&zero).unwrap();
        assert_eq!(
            v,
            ctor_obj("CtorApp", vec![json!("urn:_:Nat"), json!("zero")])
        );
    }

    #[test]
    fn encodes_unary_inductive_ctor_via_app_currying() {
        // Nat.succ(x) — encoded as App(CtorApp(urn:_:Nat, succ), Var(x))
        let nat_decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap(),
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "succ".to_string(),
                typ: Exp::sort(1),
            }],
        });
        let succ_x = Exp::InductiveCtor(
            nat_decl.iri.clone(),
            "succ".to_string(),
            vec![Exp::Var("x".to_string())],
        );
        let v = tagged(&succ_x).unwrap();
        let ctor_app = ctor_obj("CtorApp", vec![json!("urn:_:Nat"), json!("succ")]);
        let var_x = ctor_obj("Var", vec![json!("x")]);
        assert_eq!(v, ctor_obj("App", vec![ctor_app, var_x]));
    }

    #[test]
    fn unit_value_round_trips_via_decode() {
        let v = encode_type(&Exp::Unit, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, Exp::Unit);
    }

    #[test]
    fn pair_value_round_trips_via_decode() {
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let v = encode_type(&pair, crate::testing::codec_names()).unwrap();
        let decoded = decode_type(&v, &empty_layer()).unwrap();
        assert_eq!(decoded, pair);
    }

    /// The DCG's definite description: `(Σx:One. x).1` stands in for `the(Σx:C. P(x)).1`, whose
    /// shape is what the D62 encoding pipeline has to commit. Before `Fst`/`Snd` were ctors of the
    /// fragment this failed with `NotATypeLevelExp`, so every parsed sentence carrying a definite NP
    /// was uncommittable.
    #[test]
    fn sigma_projections_round_trip_via_decode() {
        let sig = Exp::Sig(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
        );
        for proj in [
            Exp::Fst(Box::new(sig.clone())),
            Exp::Snd(Box::new(sig.clone())),
        ] {
            let v = encode_type(&proj, crate::testing::codec_names()).unwrap();
            assert_eq!(decode_type(&v, &empty_layer()).unwrap(), proj);
        }
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
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap(),
            name: "urn:_:Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "zero".to_string(),
                    typ: Exp::sort(1),
                },
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "succ".to_string(),
                    typ: Exp::sort(1),
                },
            ],
        });
        let assay_decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:_:AssayShape").unwrap(),
            name: "urn:_:AssayShape".to_string(),
            params: Vec::new(),
            indices: vec![(
                Patt::Var("n".to_string()),
                Exp::const_applied(nat_decl.iri.clone(), Vec::new(), Vec::new()),
            )],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        let zero = Exp::InductiveCtor(nat_decl.iri.clone(), "zero".to_string(), Vec::new());
        let succ_zero = Exp::InductiveCtor(nat_decl.iri.clone(), "succ".to_string(), vec![zero]);
        let assay_succ_zero =
            Exp::const_applied(assay_decl.iri.clone(), Vec::new(), vec![succ_zero]);

        let encoded = tagged(&assay_succ_zero).expect("encode AssayShape (succ zero)");
        let j = encoded;

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

#[cfg(test)]
mod record_codec {
    use super::*;
    use crate::nbe::term::{Exp, Patt};
    use crate::ontology::iri::Iri;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }
    fn plain(name: &str, binder: &str) -> (Iri, Patt, Exp) {
        (iri(name), Patt::Var(binder.into()), Exp::sort(1))
    }

    fn round_trip(e: &Exp) -> Exp {
        let layer = super::tests::empty_layer();
        let encoded = encode_type(e, crate::testing::codec_names()).expect("encode");
        decode_type(&encoded, &layer).expect("decode")
    }

    /// Decode a hand-written term. The literal is a readable description; the value it
    /// denotes is built through the declaration (see `tests::value_of`), so a fixture cannot
    /// name a constructor or an arity the chain does not have.
    fn decode_raw(j: serde_json::Value) -> Result<Exp, DecodeError> {
        let layer = super::tests::empty_layer();
        decode_type(&super::tests::value_of(&j), &layer)
    }

    #[test]
    fn a_record_round_trips() {
        let e = Exp::record(vec![plain("urn:t:a", "a"), plain("urn:t:b", "b")]).unwrap();
        assert_eq!(round_trip(&e), e);
    }

    #[test]
    fn an_empty_record_round_trips() {
        // The common case: 749 of 894 shipped classes have no `requires`.
        let e = Exp::record(vec![]).unwrap();
        assert_eq!(round_trip(&e), e);
    }

    #[test]
    fn a_dependent_record_round_trips() {
        let e = Exp::record(vec![
            plain("urn:t:a", "a"),
            // Not `Times`/`Arrow`: the encoder normalises those to `Sig`/`Pi`
            // (`:127-128`), so they would fail this round-trip for reasons that
            // have nothing to do with records.
            (
                iri("urn:t:b"),
                Patt::Var("b".into()),
                Exp::App(
                    Box::new(Exp::Var("F".into())),
                    Box::new(Exp::Var("a".into())),
                ),
            ),
        ])
        .unwrap();
        assert_eq!(round_trip(&e), e);
    }

    #[test]
    fn decode_recanonicalises_a_scrambled_wire_order() {
        // The wire form is not trusted: decoding rebuilds through
        // `Exp::record`, so a hand-written encoding cannot introduce a
        // non-canonical order (D78 §1).
        let scrambled = serde_json::json!({
            "ctor": "Record",
            "args": [[
                ["urn:t:z", "z", {"ctor": "Sort", "args": [{"ctor": "Succ", "args": [{"ctor": "Zero", "args": []}]}]}],
                ["urn:t:a", "a", {"ctor": "Sort", "args": [{"ctor": "Succ", "args": [{"ctor": "Zero", "args": []}]}]}]
            ]]
        });
        let decoded = decode_raw(scrambled).expect("decode");
        match decoded {
            Exp::Record(fs) => {
                let names: Vec<&str> = fs.iter().map(|(i, _, _)| i.as_str()).collect();
                assert_eq!(names, ["urn:t:a", "urn:t:z"], "decode must canonicalise");
            }
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_a_cycle_the_wire_form_carries() {
        let cyclic = serde_json::json!({
            "ctor": "Record",
            "args": [[
                ["urn:t:a", "a", {"ctor": "Var", "args": ["b"]}],
                ["urn:t:b", "b", {"ctor": "Var", "args": ["a"]}]
            ]]
        });
        let err = decode_raw(cyclic).unwrap_err();
        assert!(
            format!("{err:?}").contains("cycle"),
            "a cycle must not decode: {err:?}"
        );
    }
}
