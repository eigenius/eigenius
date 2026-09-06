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

use std::collections::{BTreeMap, HashMap};

use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::level::Level;
use eigenius_kernel::nbe::term::{Exp, Patt};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_lean_runtime::mirror_gen::lean_name;
use nanoda_lib::tc::TypeChecker;
use nanoda_lib::util::{ExprPtr, LevelPtr, NamePtr, TcCtx};

/// Why a proposition could not be externalized.
///
/// Every arm names the construct and, where there is one, the sub-term — never a silent
/// approximation.
// No `Eq`: `NonFiniteFloat` carries an `f64`, and NaN is not equal to itself. `PartialEq` is
// what the tests compare with anyway.
#[derive(Debug, Clone, PartialEq)]
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
    /// A constant whose universe arity the caller cannot supply.
    ///
    /// nanoda's `subst_expr_levels` ASSERTS that a `Const`'s level list matches the
    /// declaration's `uparams` arity — a mismatch panics inside `def_eq` rather than returning
    /// `false`. Refused here so it comes back as a verdict.
    UniverseArityMismatch {
        lean_name: String,
        /// Universe arguments the declaration takes.
        expected: usize,
        /// Universe arguments available to supply — the target declaration's own.
        available: usize,
    },
    /// A non-finite float — NaN or ±∞ (D74 §4.8).
    ///
    /// The literal encoding is a decimal round-trip and these have no decimal form. Refused
    /// rather than approximated.
    NonFiniteFloat(f64),
    /// A `Fst`/`Snd` whose scrutinee does not have a `Subtype` type.
    ///
    /// EigenTT's Σ maps to `Subtype` (§4.2), so its projections eliminate one. A scrutinee that
    /// infers to anything else is a term the fragment cannot place.
    NotASubtype {
        /// `"Fst"` or `"Snd"`.
        form: &'static str,
    },
    /// A universe parameter the target declaration does not declare (D74 §6.5).
    ///
    /// `def_eq` compares levels, so a `Param` naming something outside the target's `uparams`
    /// compares one parameter against a different one and fails with nothing to say that
    /// universes were the cause. Refused here so the diagnostic names the parameter.
    UnknownUniverseParam {
        param: String,
        /// The parameters the target declaration does declare.
        declared: Vec<String>,
    },
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
            Self::UniverseArityMismatch {
                lean_name,
                expected,
                available,
            } => write!(
                f,
                "`{lean_name}` takes {expected} universe argument(s) and the target declaration \
                 supplies {available}; a universe-polymorphic constant can only be spelled with \
                 the target's own parameters"
            ),
            Self::NonFiniteFloat(v) => write!(
                f,
                "float literal `{v}` is not finite, so it has no decimal form to encode"
            ),
            Self::NotASubtype { form } => write!(
                f,
                "`{form}` projects a term whose type is not a `Subtype`, so there is no \
                 elimination to build"
            ),
            Self::UnknownUniverseParam { param, declared } => write!(
                f,
                "universe parameter `{param}` is not one the target declaration declares \
                 ({})",
                if declared.is_empty() {
                    "it declares none".to_string()
                } else {
                    declared.join(", ")
                }
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

/// The binders in scope, innermost last, each paired with the FREE variable standing for it.
///
/// D74 §3.1 maps EigenTT's named variables onto Lean's de Bruijn indices, and the obvious way to
/// do that is to emit `mk_var(depth - 1 - position)` while descending. This does not, and the
/// reason is §4.6: nanoda's `infer` rejects loose bound variables, so a term built that way
/// cannot be inferred under a binder — which is exactly what reconstructing `Subtype.val`'s
/// implicits needs.
///
/// nanoda's own answer is locally nameless: descend by turning the binder into a free variable
/// (`mk_dbj_level`), work on the open term, then `abstr` it closed. This does the same, so a
/// sub-term is inferrable at any depth.
struct Binders<'t>(Vec<(String, ExprPtr<'t>)>);

impl<'t> Binders<'t> {
    /// The free variable standing for `name`, innermost first.
    fn lookup(&self, name: &str) -> Option<ExprPtr<'t>> {
        self.0
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
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
/// export's declared names, which is where a `Const` is resolved rather than built; `uparams`
/// are the target declaration's universe parameters, which a `Level::Param` must name (§6.5).
pub fn externalize<'x, 't: 'x, 'p: 't>(
    exp: &Exp,
    tc: &mut TypeChecker<'x, 't, 'p>,
    names: &NameTable<'t>,
    layer: &Layer,
    uparams: &[String],
    arities: &HashMap<NamePtr<'t>, usize>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let mut binders = Binders(Vec::new());
    let mut cx = Cx {
        names,
        layer,
        uparams,
        arities,
    };
    go(exp, tc, &mut cx, &mut binders)
}

/// What externalization needs besides the arena, bundled so adding a lookup does not re-thread
/// every recursive call.
struct Cx<'a, 't> {
    names: &'a NameTable<'t>,
    layer: &'a Layer,
    uparams: &'a [String],
    /// Universe arity per declared name — see [`const_levels`].
    arities: &'a HashMap<NamePtr<'t>, usize>,
}

impl<'t> Cx<'_, 't> {
    fn arity_of(&self, name: NamePtr<'t>) -> usize {
        self.arities.get(&name).copied().unwrap_or(0)
    }
}

fn go<'x, 't: 'x, 'p: 't>(
    exp: &Exp,
    tc: &mut TypeChecker<'x, 't, 'p>,
    cx: &mut Cx<'_, 't>,
    binders: &mut Binders<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let outside = |variant: &'static str, note: &'static str| {
        Err(ExternalizeError::OutsideFragment { variant, note })
    };

    match exp {
        // ─── in the fragment ────────────────────────────────────────────────────────────

        // D74 §3.2 — the universes line up exactly, no shift. D46's `Sort(0)` is Lean's `Prop`.
        Exp::Sort(l) => {
            let level = externalize_level(l, tc.ctx, cx)?;
            Ok(tc.ctx.mk_sort(level))
        }

        Exp::Var(name) => match binders.lookup(name) {
            Some(fvar) => Ok(fvar),
            None => Err(ExternalizeError::UnboundVar(name.clone())),
        },

        // D86 — the numeric primitive core. A fully applied chain relation over `core:float`
        // becomes the Lean relation it denotes, rather than a `Const` under D74 §3.3's mangling
        // that the export does not declare. Partial application falls through to the generic
        // path and fails with `UnknownConstant`, which names both the IRI and the Lean name.
        Exp::App(f, x) => {
            if let Exp::App(head, a) = &**f {
                if let Exp::EigonAxiom(iri) = &**head {
                    if let Some(rel) = NumericRel::for_iri(iri.as_str()) {
                        return numeric_core(rel, a, x, tc, cx, binders);
                    }
                }
            }
            let f = go(f, tc, cx, binders)?;
            let x = go(x, tc, cx, binders)?;
            Ok(tc.ctx.mk_app(f, x))
        }

        Exp::Pi(p, dom, body) => {
            let dom = go(dom, tc, cx, binders)?;
            under_binder(tc, cx, binders, binder_name(p), dom, body, |c, n, d, b| {
                c.mk_pi(n, default_binder_style(), d, b)
            })
        }

        // `Arrow` is a non-dependent `Pi`; the binder is unused, so nothing is pushed and the
        // body's indices are unaffected.
        Exp::Arrow(dom, cod) => {
            let dom = go(dom, tc, cx, binders)?;
            under_binder(tc, cx, binders, "_".to_string(), dom, cod, |c, n, d, b| {
                c.mk_pi(n, default_binder_style(), d, b)
            })
        }

        // D74 §3.3 — a reference to a chain-resident declaration. `Const` replaced
        // `InductiveType(decl, args)` in D76 Phase B1, so it is this variant the §4 table's
        // `InductiveType` row now describes.
        Exp::Const(iri, levels) => {
            let name = resolve_chain_const(iri, cx)?;
            let ls: Result<Vec<LevelPtr<'t>>, _> = levels
                .iter()
                .map(|l| externalize_level(l, tc.ctx, cx))
                .collect();
            let ls = tc.ctx.alloc_levels_slice(&ls?);
            Ok(tc.ctx.mk_const(name, ls))
        }

        Exp::EigonClass(iri) | Exp::EigonAxiom(iri) => {
            let name = resolve_chain_const(iri, cx)?;
            let ls = const_levels(tc.ctx, cx, name, iri.as_str())?;
            Ok(tc.ctx.mk_const(name, ls))
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
            let n = tc.ctx.str1_owned(s.clone());
            let sp = match tc.ctx.read_name(n) {
                nanoda_lib::name::Name::Str(_, sp, _) => sp,
                _ => unreachable!("`str1_owned` builds a `Name::Str`"),
            };
            tc.ctx
                .mk_string_lit(sp)
                .ok_or(ExternalizeError::MissingLeanConstant(
                    "the string literal extension (Config::string_extension)",
                ))
        }

        // D74 §4 — negative is refused, because Lean's `NatLit` holds a `BigUint`. Synthesising
        // `Int.negSucc` would be a different term than the one authored.
        Exp::LitInt(n) if *n < 0 => Err(ExternalizeError::NegativeIntLiteral(*n)),
        Exp::LitInt(n) => tc
            .ctx
            .mk_nat_lit_quick(num_bigint::BigUint::from(*n as u64))
            .ok_or(ExternalizeError::MissingLeanConstant(
                "the nat literal extension (Config::nat_extension)",
            )),

        Exp::LitBool(b) => {
            let n = if *b { "Bool.true" } else { "Bool.false" };
            let name = cx
                .names
                .get(n)
                .ok_or(ExternalizeError::MissingLeanConstant(
                    "Bool.true / Bool.false",
                ))?;
            let empty = tc.ctx.alloc_levels_slice(&[]);
            Ok(tc.ctx.mk_const(name, empty))
        }

        // D46's unit type and its inhabitant.
        Exp::One => lean_const(tc.ctx, cx, "PUnit"),
        Exp::Unit => lean_const(tc.ctx, cx, "PUnit.unit"),

        Exp::Id(ty, x, y) => {
            let eq = lean_const(tc.ctx, cx, "Eq")?;
            let ty = go(ty, tc, cx, binders)?;
            let x = go(x, tc, cx, binders)?;
            let y = go(y, tc, cx, binders)?;
            let e = tc.ctx.mk_app(eq, ty);
            let e = tc.ctx.mk_app(e, x);
            Ok(tc.ctx.mk_app(e, y))
        }

        Exp::Refl(x) => {
            let rfl = lean_const(tc.ctx, cx, "rfl")?;
            let x = go(x, tc, cx, binders)?;
            Ok(tc.ctx.mk_app(rfl, x))
        }

        Exp::EigonPrimitive(p) => {
            use eigenius_kernel::nbe::term::PrimitiveType;
            let n =
                match p {
                    PrimitiveType::String => "String",
                    PrimitiveType::Integer => "Int",
                    PrimitiveType::Boolean => "Bool",
                    PrimitiveType::Float => "Float",
                    // `Float` has the same problem `LitFloat` has, and `Json` is a chain-side
                    // carrier with no Lean image at all.
                    PrimitiveType::Json => {
                        return outside(
                            "EigonPrimitive(Json)",
                            "a chain-side carrier type with no \
                         Lean image",
                        )
                    }
                    // `Iri` is a REFINEMENT of `String` (D88 §3), and refusing it is a decision about
                    // the TCB, not about expressibility.
                    //
                    // The shape exists on both sides. Lean has URI types — `Std.Http.URI` with
                    // `URI.parse?` and `URI.Query` in the networking stack, and the simpler
                    // `System.Uri` (`pathToUri`, `fileUriToPath?`) that LSP uses — and this file
                    // already builds refinements through [`subtype_of`], so `Subtype String isIri` is
                    // writable here.
                    //
                    // What blocks it is what that mapping WOULD BE. Each formal comorphism is in the
                    // TCB (see the D86 note below), so emitting `Subtype String p` asserts that our
                    // `Iri::parse` and Lean's `p` accept the same strings. Neither available `p`
                    // supports that assertion: `Std.Http.URI` is HTTP-oriented while our IRIs are
                    // `urn:` URNs, `System.Uri` is `file://`-oriented, and RFC 3987 IRIs admit
                    // non-ASCII that an RFC 3986 URI parser does not. Lean's own URI logic is still
                    // split between `Init.System.Uri` and `Std.Http`, with consolidation into one
                    // RFC 3986-compliant `Std.URI` an open RFC (#13922).
                    //
                    // So: refuse, and name what lifts the refusal — a general `Std.URI` whose accepted
                    // set we can state agrees with `Iri::parse`, at which point this becomes a
                    // `subtype_of` call and a TCB entry that can be reviewed on its merits.
                    PrimitiveType::Iri => return outside(
                        "EigonPrimitive(Iri)",
                        "a refinement of String; Lean's URI types (Std.Http.URI, System.Uri) are \
                         each narrower than an RFC 3987 IRI, so asserting they agree with \
                         Iri::parse would be an unsupportable TCB entry. Pending a general Std.URI",
                    ),
                };
            lean_const(tc.ctx, cx, n)
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
        // D87 §4.2 — a reference to a proof an external checker verified. It is refused here for
        // the reason the former exists: it names the artifact nanoda examined, and an artifact has
        // no Lean counterpart to translate INTO. Externalization manufactures a Lean GOAL from a
        // chain proposition; a `Checked` is evidence, not a proposition, so it can only appear as
        // `holds`'s `term` argument, never inside the `type` this walks.
        Exp::Checked(_) => outside(
            "Checked",
            "a reference to an externally checked proof is evidence, not a proposition — it names \
             the artifact the checker examined and has no Lean form",
        ),

        Exp::Lam(_, _) => outside(
            "Lam",
            "Mini-TT's lambda carries no domain and Lean's requires one that `def_eq` compares; \
             supplying it means making externalization bidirectional",
        ),

        // D74 §4.2 — `Subtype`, not `Sigma`. Measured: every Σ the DCG formalizer builds is
        // `Σ x : <class at Set>. <predicate at Prop>` — the restriction is always an application
        // of a relation declared `… -> Prop` (`ontology:compound_kind : Entity -> Set -> Prop`)
        // or `logic:And` over two of them. `Subtype : {α : Sort u} → (α → Prop) → Sort (max 1 u)`
        // is exactly that signature, and D30 already emits it for refinement-constrained fields
        // (`{ x : Float // 0.0 ≤ x }`), so it needs no new mirror vocabulary.
        //
        // `Exists` is ruled out independently: it has no projections — `Exists.elim` eliminates
        // into `Prop` only — so `Fst`/`Snd` would have no image at all.
        //
        // The predicate is a lambda built here, so §4.4's missing-domain problem does not arise:
        // the domain is the Σ's own, in hand.
        Exp::Sig(p, dom, body) => {
            let a = go(dom, tc, cx, binders)?;
            let pred = under_binder(tc, cx, binders, binder_name(p), a, body, |c, n, d, b| {
                c.mk_lambda(n, default_binder_style(), d, b)
            })?;
            subtype_of(tc.ctx, cx, a, pred)
        }

        // A non-dependent `Sig`; the binder is unused, so nothing is pushed.
        Exp::Times(dom, body) => {
            let a = go(dom, tc, cx, binders)?;
            let pred = under_binder(tc, cx, binders, "_".to_string(), a, body, |c, n, d, b| {
                c.mk_lambda(n, default_binder_style(), d, b)
            })?;
            subtype_of(tc.ctx, cx, a, pred)
        }

        // The introduction and elimination forms stay refused, and NOT for want of a decision.
        // `Subtype.mk : {α} → {p} → (val : α) → p val → Subtype p`, `Subtype.val : {α} → {p} →
        // Subtype p → α`, and `Subtype.property` likewise: each takes two implicit arguments that
        // a fully-elaborated export carries explicitly. `Pair(a, b)` and `Fst(e)` do not carry
        // them — recovering `α` and `p` needs the sub-term's TYPE, which is §4.4's bidirectional
        // problem in another costume.
        Exp::Pair(_, _) => outside(
            "Pair",
            "`Subtype.mk` takes the type and predicate as implicit arguments, which this form \
             does not carry; recovering them needs the term's type (§4.4)",
        ),
        // The eliminations. `Subtype.val : {α} → {p} → Subtype p → α` takes its type and
        // predicate implicitly, and a fully-elaborated export carries them explicitly — so they
        // must be supplied. `Fst(e)` does not carry them, but they are RECOVERABLE: the
        // externalized scrutinee is a well-formed term, so inferring its type gives `Subtype α p`
        // and destructuring that spine gives both. See `subtype_indices`.
        // The eliminations, at any depth. `Subtype.val : {α} → {p} → Subtype p → α` takes its
        // type and predicate implicitly and an elaborated export carries them explicitly, so they
        // must be supplied — and they are RECOVERABLE by inferring the scrutinee's type. That
        // works under a binder only because the body is built locally nameless: the scrutinee is
        // closed, so `infer` accepts it (§4.6).
        Exp::Fst(e) => {
            let scrutinee = go(e, tc, cx, binders)?;
            let (a, pred) = subtype_indices(tc, scrutinee, "Fst")?;
            subtype_projection(tc.ctx, cx, "Subtype.val", a, pred, scrutinee)
        }
        Exp::Snd(e) => {
            let scrutinee = go(e, tc, cx, binders)?;
            let (a, pred) = subtype_indices(tc, scrutinee, "Snd")?;
            subtype_projection(tc.ctx, cx, "Subtype.property", a, pred, scrutinee)
        }

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

        // D74 §4.8. Lean's `Expr` has no float node: `0.0` is
        // `@OfScientific.ofScientific.{0} Float instOfScientificFloat (nat_lit m) Bool.true
        // (nat_lit e)`, meaning `m * 10^(-e)` — a DECIMAL scientific form, not binary. So a
        // literal is built, not emitted.
        //
        // Exactness rests on the shortest-round-trip decimal: Rust's `{:e}` produces the shortest
        // decimal that reads back as the same f64, and Lean's `instOfScientificFloat` rounds it
        // to the same bits. Verified by `rfl` in the kernel against `0.1`, `1e300` and
        // `3.141592653589793`, so this is a reproduction of the value rather than an
        // approximation of it — the distinction §5 turns on.
        Exp::LitFloat(f) => float_literal(*f, tc.ctx, cx),

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

/// Build a binder by descending under a FREE variable, then closing it.
///
/// The locally-nameless discipline nanoda uses internally (§4.6). `mk_dbj_level` makes the
/// binder a free variable so the body is a closed term that `infer` accepts; `abstr` turns it
/// back into a bound occurrence once the body is built. Building the body with a loose bvar
/// instead would be simpler and would make every sub-term uninferrable.
fn under_binder<'x, 't: 'x, 'p: 't, F>(
    tc: &mut TypeChecker<'x, 't, 'p>,
    cx: &mut Cx<'_, 't>,
    binders: &mut Binders<'t>,
    name: String,
    domain: ExprPtr<'t>,
    body: &Exp,
    close: F,
) -> Result<ExprPtr<'t>, ExternalizeError>
where
    F: FnOnce(&mut TcCtx<'t, 'p>, NamePtr<'t>, ExprPtr<'t>, ExprPtr<'t>) -> ExprPtr<'t>,
{
    let n = tc.ctx.str1_owned(name.clone());
    let fvar = tc.ctx.mk_dbj_level(n, default_binder_style(), domain);
    binders.0.push((name, fvar));
    let built = go(body, tc, cx, binders);
    binders.0.pop();
    let built = built?;
    let closed = tc.ctx.abstr(built, &[fvar]);
    Ok(close(tc.ctx, n, domain, closed))
}

/// Lean's default binder style — an explicit binder. Externalized statements carry no
/// implicit/instance binders, because the claim's proposition has no notion of them.
fn default_binder_style() -> nanoda_lib::expr::BinderStyle {
    nanoda_lib::expr::BinderStyle::Default
}

/// The `α` and predicate a `Subtype`-typed term is indexed by, recovered by inference.
///
/// This is the elaborator's job, done with a checker's tools. nanoda never faces it: it reads
/// exports in which Lean's elaborator has already made every implicit explicit. Externalizing
/// puts us on the other side of that, so the implicits have to come from somewhere — and for an
/// ELIMINATION they can be inferred, because the scrutinee is already a well-formed term whose
/// type is `Subtype α p`.
///
/// `TypeChecker::infer` is `pub(crate)`, but `is_proof` is public and returns
/// `(is_prop, infer(e))` — its second component is the inferred type. `TcCtx::with_tc` scopes the
/// checker so the arena is free again afterwards.
fn subtype_indices<'x, 't: 'x, 'p: 't>(
    tc: &mut TypeChecker<'x, 't, 'p>,
    scrutinee: ExprPtr<'t>,
    form: &'static str,
) -> Result<(ExprPtr<'t>, ExprPtr<'t>), ExternalizeError> {
    // `TypeChecker::infer` is `pub(crate)`; `is_proof` is public and returns `(is_prop, infer(e))`.
    let inferred = tc.is_proof(scrutinee).1;
    // The type may be an unreduced application; `whnf` exposes the `Subtype` head.
    let ty = tc.whnf(inferred);
    let Some((_, name, _, args)) = tc.ctx.unfold_const_apps(ty) else {
        return Err(ExternalizeError::NotASubtype { form });
    };
    if render_name(tc.ctx, name) != "Subtype" || args.len() != 2 {
        return Err(ExternalizeError::NotASubtype { form });
    }
    Ok((args[0], args[1]))
}

/// `Subtype.val α p e` / `Subtype.property α p e` — an elimination with its implicits supplied.
fn subtype_projection<'t, 'p: 't>(
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
    which: &'static str,
    a: ExprPtr<'t>,
    pred: ExprPtr<'t>,
    scrutinee: ExprPtr<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let name = cx
        .names
        .get(which)
        .ok_or(ExternalizeError::MissingLeanConstant(which))?;
    let one = {
        let z = ctx.zero();
        ctx.succ(z)
    };
    let levels = ctx.alloc_levels_slice(&[one]);
    let head = ctx.mk_const(name, levels);
    let e = ctx.mk_app(head, a);
    let e = ctx.mk_app(e, pred);
    Ok(ctx.mk_app(e, scrutinee))
}

/// A `Float` literal, as the `OfScientific` application Lean elaborates one to (D74 §4.8).
///
/// `@OfScientific.ofScientific.{0} Float instOfScientificFloat (nat_lit m) (b : Bool)
/// (nat_lit e)` denotes `m * 10^(-e)` when `b`, `m * 10^e` otherwise — DECIMAL scientific, not
/// binary. Negative values wrap in `@Neg.neg.{0} Float instNegFloat`, which is what Lean itself
/// emits for `-1.5`.
///
/// **Why this is exact.** `{:e}` gives the shortest decimal that reads back as the same `f64`,
/// and `instOfScientificFloat` rounds that decimal to the same bits. Confirmed by `rfl` in the
/// kernel for `0.1`, `1e300` and `3.141592653589793`. Anything weaker would be an approximation
/// of the authored value, which is §5's failure mode wearing a numeric hat.
///
/// Non-finite values are refused: NaN and ±∞ have no decimal form, so there is nothing to
/// round-trip.
fn float_literal<'t, 'p: 't>(
    v: f64,
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    use num_bigint::BigUint;
    use std::str::FromStr;

    if !v.is_finite() {
        return Err(ExternalizeError::NonFiniteFloat(v));
    }

    // `{:e}` -> "<digits>[.<digits>]e<exp>", shortest round-trip, sign on the front.
    let rendered = format!("{:e}", v);
    let negative = rendered.starts_with('-');
    let body = rendered.trim_start_matches('-');
    let (significand, exp10) = body
        .split_once('e')
        .expect("`{:e}` always emits an exponent");
    let exp10: i64 = exp10.parse().expect("`{:e}` emits a decimal exponent");

    // Fold the decimal point into the exponent: `3.14e0` is `314 * 10^-2`.
    let (digits, fraction_len) = match significand.split_once('.') {
        Some((int, frac)) => (format!("{int}{frac}"), frac.len() as i64),
        None => (significand.to_string(), 0),
    };
    let mantissa = BigUint::from_str(&digits).expect("`{:e}` emits decimal digits");
    let scale = exp10 - fraction_len;

    // `@OfScientific.ofScientific.{0}` — the level is fixed by the type it builds, `Float :
    // Type 0`, not by the enclosing declaration's parameters. `lean_const` takes the target's,
    // which is right for a polymorphic constant standing at the target's universe and wrong here.
    let of_scientific = lean_const_at(ctx, cx, "OfScientific.ofScientific", 0)?;
    let float_ty = lean_const(ctx, cx, "Float")?;
    let inst = lean_const(ctx, cx, "instOfScientificFloat")?;
    let m = ctx
        .mk_nat_lit_quick(mantissa)
        .ok_or(ExternalizeError::MissingLeanConstant(
            "the nat literal extension (Config::nat_extension)",
        ))?;
    // The Bool says "negate the exponent", so a NEGATIVE scale is `true`.
    let sign = lean_const(ctx, cx, if scale < 0 { "Bool.true" } else { "Bool.false" })?;
    let e = ctx
        .mk_nat_lit_quick(BigUint::from(scale.unsigned_abs()))
        .ok_or(ExternalizeError::MissingLeanConstant(
            "the nat literal extension (Config::nat_extension)",
        ))?;

    let mut lit = ctx.mk_app(of_scientific, float_ty);
    for arg in [inst, m, sign, e] {
        lit = ctx.mk_app(lit, arg);
    }

    if !negative {
        return Ok(lit);
    }
    let neg = lean_const_at(ctx, cx, "Neg.neg", 0)?;
    let inst_neg = lean_const(ctx, cx, "instNegFloat")?;
    let float_ty = lean_const(ctx, cx, "Float")?;
    let e = ctx.mk_app(neg, float_ty);
    let e = ctx.mk_app(e, inst_neg);
    Ok(ctx.mk_app(e, lit))
}

/// `Subtype α pred` — the Lean image of an EigenTT `Sig` (D74 §4.2).
///
/// `Subtype` binds ONE universe parameter: the one its domain sits at. EigenTT's Σ is predicative
/// (`nbe/check/mod.rs`: "Sigma in Prop is predicative — both components must be in Prop"), so a
/// Σ quantifying over a class cannot inhabit `Prop`; its domain is a class at `Set`, which D30
/// emits as a Lean `structure` — a `Type`, i.e. `Sort 1`. The level is therefore `1`, spelled
/// `succ zero`, and NOT taken from the target's parameters: which universe `Subtype` sits at is
/// fixed by its domain, not by the enclosing declaration.
fn subtype_of<'t, 'p: 't>(
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
    domain: ExprPtr<'t>,
    predicate: ExprPtr<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let name = cx
        .names
        .get("Subtype")
        .ok_or(ExternalizeError::MissingLeanConstant("Subtype"))?;
    let one = {
        let z = ctx.zero();
        ctx.succ(z)
    };
    let levels = ctx.alloc_levels_slice(&[one]);
    let head = ctx.mk_const(name, levels);
    let applied = ctx.mk_app(head, domain);
    Ok(ctx.mk_app(applied, predicate))
}

/// D86 §2 — the numeric primitive core: chain relations over `core:float` and the Lean
/// relations they denote.
///
/// **Why the correspondence lives here and not on the chain.** Each formal comorphism is in the
/// TCB (`docs/guides/esl/09-institutions.md` §9.11.2), so stating it as a property on the chain
/// axiom would make a TCB entry authorable by committing a resource — the self-nomination shape
/// eigenius#23 deleted `epistemic_status` for. In Rust it is reviewed like the rest of the TCB
/// (D74, D30, `nanoda_lib`) and cannot be extended from the chain. D86 §5's "generate the Lean
/// side from the chain" argument does not apply: that guards two hand-written sides against
/// drift, and this is a fixed set of two asserted relations.
///
/// **Two are asserted; three are built from them.** `Le` and `Eq` are the correspondence — each
/// entering the TCB — and `Ge`, `Gt` and `Lt` are derived, so widening the surface from one
/// ordering relation to four costs the TCB nothing (D86 §3.2).
///
/// **`Eq` is IEEE equality, not Lean's `Eq`.** Lean's propositional `Eq` on `Float` is
/// structural — same bits — so it separates `0.0` from `-0.0`, and no measurement claim means
/// that (D86 §3.3). The chain name carries the IEEE-ness on its face for the same reason.
///
/// **The drift this cannot hide.** If a chain relation is added and this table is not, the
/// externalizer falls through to D74 §3.3's mangling and emits a `Const` the export does not
/// declare; `checker.rs` sets `unknown_pp_declar_hard_error: true`, so nanoda refuses. Agreement
/// by construction would be stronger, but the weaker property still admits no wrong answer, only
/// a refused one (eigenius#236).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NumericRel {
    /// `@LE.le.{0} Float instLEFloat x y` — asserted.
    Le,
    /// `(x == y) = true`, over `instBEqFloat` — asserted.
    Eq,
    /// `le(y, x)`.
    Ge,
    /// `lt(y, x)`.
    Gt,
    /// `le(x, y) ∧ ¬eq(x, y)`. The conjunct is what makes `<` come out FALSE at signed zero,
    /// where `≤` and IEEE `==` both hold (D86 §3.2).
    Lt,
}

impl NumericRel {
    /// The whole table, short enough to read in full (D86 §6.1).
    fn for_iri(iri: &str) -> Option<Self> {
        match iri {
            "urn:eigenius:measurements:le" => Some(Self::Le),
            "urn:eigenius:measurements:float_ieee_eq" => Some(Self::Eq),
            "urn:eigenius:measurements:ge" => Some(Self::Ge),
            "urn:eigenius:measurements:gt" => Some(Self::Gt),
            "urn:eigenius:measurements:lt" => Some(Self::Lt),
            _ => None,
        }
    }
}

/// Build the Lean proposition a numeric-core relation denotes.
///
/// NaN needs no handling and gets none (D86 §3.4): both relations are non-reflexive on NaN in
/// Lean exactly as they are in IEEE 754, so `0.0 ≤ x` is simply false for a NaN `x` — the right
/// answer, since a NaN measurement does not satisfy a bound — and `Lt`'s derivation gives
/// `false ∧ ¬false = false`, which is IEEE's answer too.
fn numeric_core<'x, 't: 'x, 'p: 't>(
    rel: NumericRel,
    a: &Exp,
    b: &Exp,
    tc: &mut TypeChecker<'x, 't, 'p>,
    cx: &mut Cx<'_, 't>,
    binders: &mut Binders<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    match rel {
        NumericRel::Le => float_le(a, b, tc, cx, binders),
        NumericRel::Ge => float_le(b, a, tc, cx, binders),
        NumericRel::Eq => float_ieee_eq(a, b, tc, cx, binders),
        NumericRel::Lt => float_lt(a, b, tc, cx, binders),
        NumericRel::Gt => float_lt(b, a, tc, cx, binders),
    }
}

/// `@LE.le.{0} Float instLEFloat a b`. The level is fixed at 0 by `Float : Type 0`, whatever the
/// target declaration's own parameters are — the same reason [`lean_const_at`] exists.
fn float_le<'x, 't: 'x, 'p: 't>(
    a: &Exp,
    b: &Exp,
    tc: &mut TypeChecker<'x, 't, 'p>,
    cx: &mut Cx<'_, 't>,
    binders: &mut Binders<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let head = lean_const_at(tc.ctx, cx, "LE.le", 0)?;
    let float_ty = lean_const(tc.ctx, cx, "Float")?;
    let inst = lean_const(tc.ctx, cx, "instLEFloat")?;
    let a = go(a, tc, cx, binders)?;
    let b = go(b, tc, cx, binders)?;
    let e = tc.ctx.mk_app(head, float_ty);
    let e = tc.ctx.mk_app(e, inst);
    let e = tc.ctx.mk_app(e, a);
    Ok(tc.ctx.mk_app(e, b))
}

/// `@Eq.{1} Bool (@BEq.beq.{0} Float instBEqFloat a b) Bool.true` — D86 §6.2's chain-side
/// proposition `(a == b) = true`.
///
/// `Eq` sits at level 1 because it quantifies over `Sort u` and its argument is `Bool : Type 0`,
/// which is `Sort 1`. `BEq.beq` sits at 0 because it quantifies over `Type u` and its argument is
/// `Float : Type 0`.
///
/// **Not `Decidable` equality.** That decides `Eq`, and `Eq` on `Float` is structural, so a
/// `Decidable` equality delivers exactly the relation D86 §3.3 rejects.
fn float_ieee_eq<'x, 't: 'x, 'p: 't>(
    a: &Exp,
    b: &Exp,
    tc: &mut TypeChecker<'x, 't, 'p>,
    cx: &mut Cx<'_, 't>,
    binders: &mut Binders<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let beq = lean_const_at(tc.ctx, cx, "BEq.beq", 0)?;
    let float_ty = lean_const(tc.ctx, cx, "Float")?;
    let inst = lean_const(tc.ctx, cx, "instBEqFloat")?;
    let a = go(a, tc, cx, binders)?;
    let b = go(b, tc, cx, binders)?;
    let applied = tc.ctx.mk_app(beq, float_ty);
    let applied = tc.ctx.mk_app(applied, inst);
    let applied = tc.ctx.mk_app(applied, a);
    let applied = tc.ctx.mk_app(applied, b);

    let eq = lean_const_at(tc.ctx, cx, "Eq", 1)?;
    let bool_ty = lean_const(tc.ctx, cx, "Bool")?;
    let tt = lean_const(tc.ctx, cx, "Bool.true")?;
    let e = tc.ctx.mk_app(eq, bool_ty);
    let e = tc.ctx.mk_app(e, applied);
    Ok(tc.ctx.mk_app(e, tt))
}

/// `And (a ≤ b) (Not ((a == b) = true))` — D86 §3.2's derivation.
///
/// `And` and `Not` are `Prop -> Prop -> Prop` and `Prop -> Prop`, so neither takes a universe
/// argument.
fn float_lt<'x, 't: 'x, 'p: 't>(
    a: &Exp,
    b: &Exp,
    tc: &mut TypeChecker<'x, 't, 'p>,
    cx: &mut Cx<'_, 't>,
    binders: &mut Binders<'t>,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let le = float_le(a, b, tc, cx, binders)?;
    let eq = float_ieee_eq(a, b, tc, cx, binders)?;
    let not = lean_const(tc.ctx, cx, "Not")?;
    let neq = tc.ctx.mk_app(not, eq);
    let and = lean_const(tc.ctx, cx, "And")?;
    let e = tc.ctx.mk_app(and, le);
    Ok(tc.ctx.mk_app(e, neq))
}

/// One of Lean's own constants at an EXPLICIT universe level.
///
/// For a constant whose level is fixed by the type it operates on rather than by the enclosing
/// declaration — `@OfScientific.ofScientific.{0}` builds a `Float : Type 0` whatever the target
/// declares. [`lean_const`] takes the target's parameters, which is right when the constant
/// stands at the target's universe and wrong when it does not.
fn lean_const_at<'t, 'p: 't>(
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
    n: &'static str,
    level: usize,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let name = cx
        .names
        .get(n)
        .ok_or(ExternalizeError::MissingLeanConstant(n))?;
    let mut l = ctx.zero();
    for _ in 0..level {
        l = ctx.succ(l);
    }
    let levels = ctx.alloc_levels_slice(&[l]);
    Ok(ctx.mk_const(name, levels))
}

/// Resolve one of Lean's own constants by name.
fn lean_const<'t, 'p: 't>(
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
    n: &'static str,
) -> Result<ExprPtr<'t>, ExternalizeError> {
    let name = cx
        .names
        .get(n)
        .ok_or(ExternalizeError::MissingLeanConstant(n))?;
    let levels = const_levels(ctx, cx, name, n)?;
    Ok(ctx.mk_const(name, levels))
}

/// The universe arguments for `name`, taken from the target declaration's parameters.
///
/// Lean's `PUnit`, `Eq` and `rfl` are universe-polymorphic; a D30-emitted class is not
/// (`structure Person where …` binds no parameters), so most constants take none. Supplying the
/// wrong count does not merely fail the comparison — `subst_expr_levels` asserts on it and
/// nanoda panics inside `def_eq`.
///
/// The target declaration's own parameters are the only source an outside-in translation has:
/// which universe a constant sits at is determined by the surrounding term, the same
/// information `Lam`'s domain needs (§4.4). Where the arities agree that is the right answer —
/// `PUnit.unit : PUnit.{u}` wants exactly the `u` its own declaration binds.
fn const_levels<'t, 'p: 't>(
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
    name: NamePtr<'t>,
    label: &str,
) -> Result<nanoda_lib::util::LevelsPtr<'t>, ExternalizeError> {
    let arity = cx.arity_of(name);
    if arity == 0 {
        return Ok(ctx.alloc_levels_slice(&[]));
    }
    if arity != cx.uparams.len() {
        return Err(ExternalizeError::UniverseArityMismatch {
            lean_name: label.to_string(),
            expected: arity,
            available: cx.uparams.len(),
        });
    }
    let ls: Vec<LevelPtr<'t>> = cx
        .uparams
        .iter()
        .map(|u| {
            let p = ctx.str1_owned(u.clone());
            ctx.param(p)
        })
        .collect();
    Ok(ctx.alloc_levels_slice(&ls))
}

/// The `NamePtr` for a chain IRI, spelled by D30's mangling and resolved in the export.
fn resolve_chain_const<'t>(iri: &Iri, cx: &Cx<'_, 't>) -> Result<NamePtr<'t>, ExternalizeError> {
    let def = cx
        .layer
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
    cx.names
        .get(&lean)
        .ok_or(ExternalizeError::UnknownConstant {
            iri: iri.as_str().to_string(),
            lean_name: lean,
        })
}

/// D74 §3.2 — EigenTT levels and Lean levels are the same lattice, so this is structural.
fn externalize_level<'t, 'p: 't>(
    l: &Level,
    ctx: &mut TcCtx<'t, 'p>,
    cx: &Cx<'_, 't>,
) -> Result<LevelPtr<'t>, ExternalizeError> {
    Ok(match l {
        Level::Zero => ctx.zero(),
        Level::Succ(inner) => {
            let i = externalize_level(inner, ctx, cx)?;
            ctx.succ(i)
        }
        Level::Max(a, b) => {
            let a = externalize_level(a, ctx, cx)?;
            let b = externalize_level(b, ctx, cx)?;
            ctx.max(a, b)
        }
        Level::IMax(a, b) => {
            let a = externalize_level(a, ctx, cx)?;
            let b = externalize_level(b, ctx, cx)?;
            ctx.imax(a, b)
        }
        // D74 §6.5 — a parameter the target declaration does not declare makes `def_eq` compare
        // one parameter against a different one, and fail without saying universes were why.
        Level::Param(n) => {
            let n = n.to_string();
            if !cx.uparams.contains(&n) {
                return Err(ExternalizeError::UnknownUniverseParam {
                    param: n,
                    declared: cx.uparams.to_vec(),
                });
            }
            let ptr = ctx.str1_owned(n);
            ctx.param(ptr)
        }
    })
}
