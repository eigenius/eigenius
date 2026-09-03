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

//! D74 — EigenTT `Prop` → nanoda `Expr`, built in the checker's own arena.
//!
//! The claim carries the proposition (`reflection:canonical_proposition`, a D47-encoded
//! `eigentt:Term`). This module manufactures the Lean statement *from that*, so the proof is
//! bound to the claim because the goal was made from the claim. Nothing is recovered from Lean;
//! D40's inverse direction is what this replaces.
//!
//! # Why the target is a nanoda `Expr` and not Lean source
//!
//! No source is emitted and no Lake build runs. The export is already parsed by nanoda in
//! [`crate::checker`], so both sides land as `ExprPtr` in one `TcCtx` arena and the comparison
//! is nanoda's own definitional equality — `α`-renaming, `δ`-unfolding and `η` handled by the
//! checker we already trust rather than reimplemented here.
//!
//! # Names are resolved, not constructed
//!
//! D74 §3.3 requires a total, injective, stable IRI → `Name` map agreed by both the externalizer
//! and whatever produced the export. The mangling half is
//! [`eigenius_lean_runtime::mirror_gen::lean_name`] — the single authority D30's emitter also
//! calls, so the two agree by construction.
//!
//! Turning that string into a `NamePtr` is done by **looking it up in the export**, not by
//! building it. Two reasons, one forced and one better:
//!
//! - Forced: nanoda's public API cannot build a multi-component `Name` from runtime strings.
//!   `TcCtx::str` needs a `StringPtr` and `alloc_string` is `pub(crate)`; `str1_owned` only
//!   builds a single component off `Anon`. String literals hit the same wall from the other
//!   side: `mk_string_lit_quick` is `pub` but takes `CowStr`, which is not, so `LitString`
//!   interns through `str1_owned` and reads the `StringPtr` back off the name. `LitInt` is
//!   unaffected — `mk_nat_lit_quick` takes a `BigUint`, which is public.
//! - Better: a constant the export does not declare cannot be `def_eq` to anything in it, so
//!   construction would only defer the failure to a comparison that cannot say naming was the
//!   cause. Resolving up front gives [`ExternalizeError::UnknownConstant`], which names the
//!   class and the Lean name it looked for. That is the early coverage refusal D74 §3.3 had
//!   assigned to the mirror before §6.3.1 deleted `mirror_iri`.
//!
//! # Refusal is typed and total
//!
//! The `Exp` match is exhaustive, so the compiler — not a table in a document — is what
//! guarantees every variant is classified. A proposition outside the fragment fails loudly with
//! the variant named; the alternative, translating "close enough", proves a different theorem
//! soundly.

use std::collections::BTreeMap;

use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::level::Level;
use eigenius_kernel::nbe::term::{Exp, Patt};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_lean_runtime::mirror_gen::lean_name;
use nanoda_lib::util::{ExprPtr, LevelPtr, NamePtr, TcCtx};

/// Why a proposition could not be externalized.
///
/// Every arm names the construct and, where there is one, the sub-term — never a silent
/// approximation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalizeError {
    /// The proposition uses a construct outside D74 §4's fragment.
    OutsideFragment {
        /// The `Exp` variant, as written in `kernel/src/nbe/term.rs`.
        variant: &'static str,
        /// Why it is out, in terms a reader can act on.
        note: &'static str,
    },
    /// A chain constant whose Lean name the export does not declare.
    ///
    /// Either the proposition mentions a class the mirror the proof was compiled against did
    /// not cover, or the mirror has moved since. Both are the same repair: regenerate the
    /// mirror and rebuild the proof.
    UnknownConstant {
        /// The chain IRI.
        iri: String,
        /// The Lean name D30's mangling gives it — what was looked for.
        lean_name: String,
    },
    /// A chain IRI that does not resolve, or resolves to something with no `core:short_name`.
    /// The Lean name cannot be spelled at all.
    Unnameable { iri: String, reason: &'static str },
    /// `LitInt` below zero. Lean's `NatLit` holds a `BigUint`; a negative literal has no image
    /// and inventing `Int.negSucc` would be a different term than the one authored.
    NegativeIntLiteral(i64),
    /// A `Var` with no enclosing binder. EigenTT names its variables and Lean uses de Bruijn
    /// indices, so an unbound name has no index to become.
    UnboundVar(String),
    /// A Lean-side constant the fragment needs (`Eq`, `PUnit`, `Bool.true`, …) that the export
    /// does not declare. Distinct from [`Self::UnknownConstant`], which is about chain classes.
    MissingLeanConstant(&'static str),
}

impl std::fmt::Display for ExternalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideFragment { variant, note } => {
                write!(f, "`{variant}` is outside the D74 §4 fragment: {note}")
            }
            Self::UnknownConstant { iri, lean_name } => write!(
                f,
                "the export declares no `{lean_name}` (for `{iri}`) — the proof was compiled \
                 against a mirror that does not cover this class, or the mirror has moved"
            ),
            Self::Unnameable { iri, reason } => {
                write!(f, "`{iri}` has no Lean name: {reason}")
            }
            Self::NegativeIntLiteral(n) => write!(
                f,
                "integer literal {n} is negative; Lean's `NatLit` holds a BigUint"
            ),
            Self::UnboundVar(v) => write!(f, "variable `{v}` is not bound by any enclosing binder"),
            Self::MissingLeanConstant(n) => write!(
                f,
                "the export does not declare `{n}`, which this proposition needs"
            ),
        }
    }
}

impl std::error::Error for ExternalizeError {}

/// Every `Name` the export declares, keyed by its dotted rendering.
///
/// Built once per externalization. The export's `declars` map is keyed by `NamePtr`, which is
/// an interning handle rather than a string, so finding a name by its spelling means rendering
/// each one — the same linear scan `check_proof` already does to locate `target_name`.
pub struct NameTable<'t> {
    by_rendering: BTreeMap<String, NamePtr<'t>>,
}

impl<'t> NameTable<'t> {
    /// Render every declared name in `ctx`'s export.
    pub fn build<'p: 't>(ctx: &TcCtx<'t, 'p>, declared: &[NamePtr<'t>]) -> Self {
        let mut by_rendering = BTreeMap::new();
        for n in declared {
            by_rendering.insert(render_name(ctx, *n), *n);
        }
        Self { by_rendering }
    }

    fn get(&self, rendered: &str) -> Option<NamePtr<'t>> {
        self.by_rendering.get(rendered).copied()
    }
}

/// Render a nanoda `Name` to its dotted form — `Foo.bar.42`.
pub fn render_name<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, name: NamePtr<'t>) -> String {
    use nanoda_lib::name::Name;
    let mut parts: Vec<String> = Vec::new();
    let mut cur = name;
    loop {
        match ctx.read_name(cur) {
            Name::Anon => break,
            Name::Str(prefix, suffix, _) => {
                parts.push(ctx.read_string(suffix).as_ref().to_string());
                cur = prefix;
            }
            Name::Num(prefix, suffix, _) => {
                parts.push(suffix.to_string());
                cur = prefix;
            }
        }
    }
    parts.reverse();
    parts.join(".")
}

/// The binders in scope, innermost last. EigenTT names its variables; Lean counts them
/// (D74 §3.1), so a `Var` becomes `depth - 1 - position`.
struct Binders(Vec<String>);

impl Binders {
    fn index_of(&self, name: &str) -> Option<u16> {
        self.0
            .iter()
            .rposition(|b| b == name)
            .map(|pos| (self.0.len() - 1 - pos) as u16)
    }
}

/// The binder name a pattern introduces.
///
/// `Patt::Pair` destructures, which the fragment does not admit — it can only appear under a
/// `Sig`/`Times` binder, and those are refused before a pattern is reached. `Unit` is Lean's
/// unnamed binder, and carries a name only for readability.
fn binder_name(p: &Patt) -> String {
    match p {
        Patt::Var(n) => n.to_string(),
        Patt::Unit => "_".to_string(),
        Patt::Pair(_, _) => "_pair".to_string(),
    }
}

/// Externalize `exp` into `ctx`'s arena.
///
/// `layer` resolves a chain IRI's `core:short_name`, which D30's mangling needs; `names` is the
/// export's declared names, which is where a `Const` is resolved rather than built.
pub fn externalize<'t, 'p: 't>(
    exp: &Exp,
    ctx: &mut TcCtx<'t, 'p>,
    names: &NameTable<'t>,
    layer: &Layer,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let mut binders = Binders(Vec::new());
    go(exp, ctx, names, layer, &mut binders)
}

fn go<'t, 'p: 't>(
    exp: &Exp,
    ctx: &mut TcCtx<'t, 'p>,
    names: &NameTable<'t>,
    layer: &Layer,
    binders: &mut Binders,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let outside = |variant: &'static str, note: &'static str| {
        Err(ExternalizeError::OutsideFragment { variant, note })
    };

    match exp {
        // ─── in the fragment ────────────────────────────────────────────────────────────

        // D74 §3.2 — the universes line up exactly, no shift. D46's `Sort(0)` is Lean's `Prop`.
        Exp::Sort(l) => {
            let level = externalize_level(l, ctx)?;
            Ok(ctx.mk_sort(level))
        }

        Exp::Var(name) => match binders.index_of(name) {
            Some(idx) => Ok(ctx.mk_var(idx)),
            None => Err(ExternalizeError::UnboundVar(name.clone())),
        },

        Exp::App(f, x) => {
            let f = go(f, ctx, names, layer, binders)?;
            let x = go(x, ctx, names, layer, binders)?;
            Ok(ctx.mk_app(f, x))
        }

        Exp::Pi(p, dom, body) => {
            let dom = go(dom, ctx, names, layer, binders)?;
            let name = binder_name(p);
            let n = ctx.str1_owned(name.clone());
            binders.0.push(name);
            let body = go(body, ctx, names, layer, binders);
            binders.0.pop();
            Ok(ctx.mk_pi(n, default_binder_style(), dom, body?))
        }

        // `Arrow` is a non-dependent `Pi`; the binder is unused, so nothing is pushed and the
        // body's indices are unaffected.
        Exp::Arrow(dom, cod) => {
            let dom = go(dom, ctx, names, layer, binders)?;
            let n = ctx.str1_owned("_".to_string());
            binders.0.push("_".to_string());
            let cod = go(cod, ctx, names, layer, binders);
            binders.0.pop();
            Ok(ctx.mk_pi(n, default_binder_style(), dom, cod?))
        }

        // D74 §3.3 — a reference to a chain-resident declaration. `Const` replaced
        // `InductiveType(decl, args)` in D76 Phase B1, so it is this variant the §4 table's
        // `InductiveType` row now describes.
        Exp::Const(iri, levels) => {
            let name = resolve_chain_const(iri, ctx, names, layer)?;
            let ls: Result<Vec<LevelPtr<'t>>, _> =
                levels.iter().map(|l| externalize_level(l, ctx)).collect();
            let ls = ctx.alloc_levels_slice(&ls?);
            Ok(ctx.mk_const(name, ls))
        }

        Exp::EigonClass(iri) | Exp::EigonAxiom(iri) => {
            let name = resolve_chain_const(iri, ctx, names, layer)?;
            let empty = ctx.alloc_levels_slice(&[]);
            Ok(ctx.mk_const(name, empty))
        }

        // Both literal constructors return `None` when the corresponding parser extension is
        // off. `check_proof` turns both on, so `None` here means the caller built the `TcCtx`
        // some other way — a wiring error, not a proposition the fragment excludes.
        // `mk_string_lit_quick` is `pub` but takes `CowStr`, which is `pub(crate)`, so it is
        // not callable from here; `mk_string_lit` needs a `StringPtr` and `alloc_string` is
        // `pub(crate)` too. Interning a one-component name is the public route to a
        // `StringPtr` — `str1_owned` allocates the string internally, and reading the name back
        // recovers the pointer it allocated.
        Exp::LitString(s) => {
            let n = ctx.str1_owned(s.clone());
            let sp = match ctx.read_name(n) {
                nanoda_lib::name::Name::Str(_, sp, _) => sp,
                _ => unreachable!("`str1_owned` builds a `Name::Str`"),
            };
            ctx.mk_string_lit(sp)
                .ok_or(ExternalizeError::MissingLeanConstant(
                    "the string literal extension (Config::string_extension)",
                ))
        }

        // D74 §4 — negative is refused, because Lean's `NatLit` holds a `BigUint`. Synthesising
        // `Int.negSucc` would be a different term than the one authored.
        Exp::LitInt(n) if *n < 0 => Err(ExternalizeError::NegativeIntLiteral(*n)),
        Exp::LitInt(n) => ctx
            .mk_nat_lit_quick(num_bigint::BigUint::from(*n as u64))
            .ok_or(ExternalizeError::MissingLeanConstant(
                "the nat literal extension (Config::nat_extension)",
            )),

        Exp::LitBool(b) => {
            let n = if *b { "Bool.true" } else { "Bool.false" };
            let name = names.get(n).ok_or(ExternalizeError::MissingLeanConstant(
                "Bool.true / Bool.false",
            ))?;
            let empty = ctx.alloc_levels_slice(&[]);
            Ok(ctx.mk_const(name, empty))
        }

        // D46's unit type and its inhabitant.
        Exp::One => lean_const(ctx, names, "PUnit"),
        Exp::Unit => lean_const(ctx, names, "PUnit.unit"),

        Exp::Id(ty, x, y) => {
            let eq = lean_const(ctx, names, "Eq")?;
            let ty = go(ty, ctx, names, layer, binders)?;
            let x = go(x, ctx, names, layer, binders)?;
            let y = go(y, ctx, names, layer, binders)?;
            let e = ctx.mk_app(eq, ty);
            let e = ctx.mk_app(e, x);
            Ok(ctx.mk_app(e, y))
        }

        Exp::Refl(x) => {
            let rfl = lean_const(ctx, names, "rfl")?;
            let x = go(x, ctx, names, layer, binders)?;
            Ok(ctx.mk_app(rfl, x))
        }

        Exp::EigonPrimitive(p) => {
            use eigenius_kernel::nbe::term::PrimitiveType;
            let n = match p {
                PrimitiveType::String => "String",
                PrimitiveType::Integer => "Int",
                PrimitiveType::Boolean => "Bool",
                // `Float` has the same problem `LitFloat` has, and `Json` is a chain-side
                // carrier with no Lean image at all.
                PrimitiveType::Float => {
                    return outside(
                        "EigonPrimitive(Float)",
                        "Lean has no `Float` vs `Real` \
                         decision in v1 — see `LitFloat`",
                    )
                }
                PrimitiveType::Json => {
                    return outside(
                        "EigonPrimitive(Json)",
                        "a chain-side carrier type with no \
                         Lean image",
                    )
                }
            };
            lean_const(ctx, names, n)
        }

        // ─── outside the fragment ───────────────────────────────────────────────────────
        //
        // Each refusal names the construct. D74 §4 lists the reasons; the `match` is what makes
        // the list total, so a variant added to `Exp` breaks this file rather than falling
        // through to a wrong translation.
        // `Lam` is Mini-TT's UNANNOTATED lambda, inherited with the rest of this AST from the
        // Coquand et al. reference implementation. It carries no domain because Mini-TT is
        // bidirectional: a lambda is only ever CHECKED against a known `Pi`, which supplies one.
        // `check_infer` has no `Lam` arm at all — `kernel/src/nbe/check/mod.rs` pins that as "not
        // inferable" — and `(Exp::Lam(..), Val::Sort(n))` is an explicit error, so a λ cannot BE
        // a proposition; it can only appear as an argument inside one, where the applied
        // function's type determines its domain.
        //
        // Lean's `Lambda` requires a domain and `def_eq` compares it — `def_eq_binder_aux` runs
        // `if self.def_eq(t1, t2) { … } else { return false }`. So there is no placeholder that
        // works: a wrong domain is a wrong term, not an invisible one.
        //
        // Admitting it means making this function bidirectional, threading the expected type down
        // so a `Lam` under an application gets its domain from the function. Measured before
        // refusing: of the 102 committed `canonical_proposition` values in the tree, zero contain
        // a `Lam`, so v1 gives up nothing that exists.
        Exp::Lam(_, _) => outside(
            "Lam",
            "Mini-TT's lambda carries no domain and Lean's requires one that `def_eq` compares; \
             supplying it means making externalization bidirectional",
        ),

        Exp::Sig(_, _, _) => outside(
            "Sig",
            "Lean's `Sigma` is library, not primitive — admitting it means pinning WHICH Sigma, \
             which is a decision of its own",
        ),
        Exp::Times(_, _) => outside("Times", "a non-dependent `Sig`; same decision"),
        Exp::Pair(_, _) => outside("Pair", "the introduction form for `Sig`"),
        Exp::Fst(_) => outside("Fst", "an elimination form for `Sig`"),
        Exp::Snd(_) => outside("Snd", "an elimination form for `Sig`"),

        Exp::Record(_) => outside(
            "Record",
            "a D78 record type — a resource's own shape, not a proposition about one",
        ),
        Exp::Refine(_, _) => outside(
            "Refine",
            "a D78 record with its class constraints — resource-level, like `Record`",
        ),

        Exp::Map(_, _) => outside("Map", "computation, not proposition"),
        Exp::Reduce(_, _, _) => outside("Reduce", "computation, not proposition"),
        Exp::NativeDecide(_, _) => outside("NativeDecide", "computation, not proposition"),
        Exp::DecEq(_, _, _) => outside("DecEq", "computation, not proposition"),

        Exp::Template(_, _) => outside("Template", "resource-level"),
        Exp::PropAccess(_, _) => outside(
            "PropAccess",
            "projects a field off a resource VALUE; a proposition about a value rather than a \
             class is outside the fragment",
        ),
        Exp::Construct(_, _) => outside("Construct", "builds a resource value; resource-level"),
        Exp::EigonResource(_) => outside(
            "EigonResource",
            "names a resource rather than its class; resource-level",
        ),

        Exp::LitFloat(_) => outside(
            "LitFloat",
            "Lean has no float literal, and a proposition over reals needs a `Float` vs `Real` \
             decision v1 does not make",
        ),

        Exp::Data(_) => outside(
            "Data",
            "a declaration form the codec does not emit into a proposition slot",
        ),
        Exp::Case(_) => outside("Case", "an eliminator; surface form, not a proposition"),
        Exp::Dec(_, _) => outside("Dec", "a let/letrec binding; surface form"),
        Exp::Ann(_, _) => outside("Ann", "a type ascription; surface form"),
        Exp::Con(_, _) => outside(
            "Con",
            "a constructor application whose inductive is implicit; use `InductiveCtor`",
        ),
        // D74 §4.3. Not a naming question: D30 v1 emits a `structure` per mirrored CLASS and no
        // inductives at all (`mirror_gen/mod.rs:603` — "those land with D30 v1.1"), so the
        // inductive is absent from the export and so are its constructors. When they land the
        // name is `<inductive>.<ctor>`, Lean's convention and the only candidate — D85's derived
        // ctor class is `{inductive}-{ctor}`, and `-` is not a Lean identifier character.
        Exp::InductiveCtor(_, _, _) => outside(
            "InductiveCtor",
            "D30 v1 mirrors classes as `structure`s and emits no inductives, so a constructor \
             has no constant in the export to denote (D30 v1.1 / Phase 20b)",
        ),
        Exp::InductiveRec { .. } => outside(
            "InductiveRec",
            "a recursor application — elimination, not a proposition; the same line `Case` and \
             `IdJ` fall on",
        ),
        Exp::Match { .. } => outside("Match", "an eliminator; surface form, not a proposition"),
        Exp::InstitutionInvoke { .. } => outside(
            "InstitutionInvoke",
            "dispatches a comorphism — an effect, and one whose result is not determined by the \
             proposition alone",
        ),
        Exp::IdJ(_) => outside(
            "IdJ",
            "the J eliminator — `Id` and `Refl` are in the fragment, elimination is not",
        ),
    }
}

/// Lean's default binder style — an explicit binder. Externalized statements carry no
/// implicit/instance binders, because the claim's proposition has no notion of them.
fn default_binder_style() -> nanoda_lib::expr::BinderStyle {
    nanoda_lib::expr::BinderStyle::Default
}

/// Resolve one of Lean's own constants by name.
fn lean_const<'t, 'p: 't>(
    ctx: &mut TcCtx<'t, 'p>,
    names: &NameTable<'t>,
    n: &'static str,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let name = names
        .get(n)
        .ok_or(ExternalizeError::MissingLeanConstant(n))?;
    let empty = ctx.alloc_levels_slice(&[]);
    Ok(ctx.mk_const(name, empty))
}

/// The `NamePtr` for a chain IRI, spelled by D30's mangling and resolved in the export.
fn resolve_chain_const<'t, 'p: 't>(
    iri: &Iri,
    _ctx: &mut TcCtx<'t, 'p>,
    names: &NameTable<'t>,
    layer: &Layer,
) -> Result<NamePtr<'t>, ExternalizeError> {
    let def = layer
        .resolve(iri)
        .ok_or_else(|| ExternalizeError::Unnameable {
            iri: iri.as_str().to_string(),
            reason: "does not resolve in the verification context's layer chain",
        })?;
    let short = def
        .get(&Iri::parse(wk::SHORT_NAME).expect("well-known IRI"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ExternalizeError::Unnameable {
            iri: iri.as_str().to_string(),
            reason: "carries no `core:short_name`, so D30's mangling cannot spell it",
        })?;
    let lean = lean_name::class_lean_name_absolute(iri, short);
    names.get(&lean).ok_or(ExternalizeError::UnknownConstant {
        iri: iri.as_str().to_string(),
        lean_name: lean,
    })
}

/// D74 §3.2 — EigenTT levels and Lean levels are the same lattice, so this is structural.
fn externalize_level<'t, 'p: 't>(
    l: &Level,
    ctx: &mut TcCtx<'t, 'p>,
) -> Result<LevelPtr<'t>, ExternalizeError> {
    Ok(match l {
        Level::Zero => ctx.zero(),
        Level::Succ(inner) => {
            let i = externalize_level(inner, ctx)?;
            ctx.succ(i)
        }
        Level::Max(a, b) => {
            let a = externalize_level(a, ctx)?;
            let b = externalize_level(b, ctx)?;
            ctx.max(a, b)
        }
        Level::IMax(a, b) => {
            let a = externalize_level(a, ctx)?;
            let b = externalize_level(b, ctx)?;
            ctx.imax(a, b)
        }
        Level::Param(n) => {
            let n = ctx.str1_owned(n.to_string());
            ctx.param(n)
        }
    })
}
