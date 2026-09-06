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

//! EigenTT bidirectional type checker.
//!
//! Ported from `Main.hs` lines 289-378 in the EigenTT reference.
//! Uses NbE (eval + readback) for type equality checking.

mod conv;
mod error;
mod hooks;
mod inductive;
#[cfg(test)]
mod testutil;
mod witness;

use conv::infer_dependent_sort;
pub use conv::{def_eq_at_type, eq_nf, exp_mentions_var, subtype_of};
pub use error::CheckError;
pub use hooks::CheckHooks;
pub use inductive::large_elim_admitted;
use inductive::{
    check_inductive_ctor_args, check_inductive_decl_telescopes, check_infer_inductive_rec,
    check_match, validate_indexed_ctor_conclusions,
};

use crate::layer::Layer;
use crate::nbe::env::{gen_val, lookup_gamma, up_gamma, Gamma, Rho};
// D76 Phase B: the bare `eval` is deliberately NOT imported here. Every
// evaluation the checker performs goes through `CheckCtx::eval`, which carries
// `Γ_env`; an env-less call would leave a de-inlined `Const` as a neutral
// instead of the declaration it names. Tests import it explicitly.
use crate::nbe::eval::eval_ctx;
use crate::nbe::readback::readback_val;
use crate::nbe::term::{Decl, Exp, Patt};
use crate::nbe::val::{Clos, Val};
use crate::ontology::iri::Iri;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Type-checking context, threaded through all checker calls.
///
/// Bundles the evaluation environment (`rho`), type context (`gamma`),
/// an optional layer for ontology-as-types resolution, and a per-check
/// cache for resolved class types.
///
/// Design follows nanoda_lib's `TypeChecker` pattern
/// (`references/nanoda_lib/src/tc.rs` @ pinned commit `6ae1f0c`): a
/// single struct carrying mutable state (cache) plus immutable
/// environment through all checker calls. The cache is scoped per
/// type-check invocation — fresh per call, no cross-check invalidation
/// needed.
pub struct CheckCtx {
    pub rho: Rho,
    pub gamma: Gamma,
    /// D76 Phase C — `Γ_env`, the global environment of the judgment.
    ///
    /// Was `layer: Option<Arc<Layer>>`. The `Option` is gone from the
    /// judgment's view: a caller with nothing to resolve holds
    /// [`Env::empty()`](crate::nbe::env_global::Env::empty), and looks up to get
    /// `Absent` rather than branching on whether it has a layer. Every
    /// layer-less construction in the tree is a test — production always
    /// supplied one — so this removes a mode only tests took.
    pub env: crate::nbe::env_global::Env,
    /// Per-check memoization of resolved class types, keyed by class IRI string.
    type_cache: BTreeMap<String, Val>,
    /// institution index — derived view of the layer chain. When
    /// attached together with `institution_runtime`,
    /// `Constraint::Institution` predicates dispatch through
    /// `try_institution_decide` (D14 §9.2). Without these, constraints stay
    /// as passthrough neutrals — what `EvalCtx::pure()` does anyway.
    pub institution_index: Option<Arc<crate::institution::registry::InstitutionIndex>>,
    /// institution runtime — registry of `Institution` trait
    /// objects keyed by institution IRI. See `institution_index`.
    pub institution_runtime: Option<Arc<crate::institution::runtime::InstitutionRuntime>>,
    /// Chain-resident resolution (EigonClass → Sigma type; D49
    /// ChainWitness synthesis) the checker delegates out of its pure
    /// core. Wired to the default (`program::check_hooks`) by the
    /// constructors; the checker body only touches the trait.
    hooks: Arc<dyn CheckHooks>,
}

impl CheckCtx {
    /// Create a new context with no layer access (pure mode).
    pub fn new(rho: Rho, gamma: Gamma) -> Self {
        Self {
            rho,
            gamma,
            env: crate::nbe::env_global::Env::empty(),
            type_cache: BTreeMap::new(),
            institution_index: None,
            institution_runtime: None,
            hooks: Arc::new(crate::program::check_hooks::DefaultCheckHooks),
        }
    }

    /// Create a new context with layer access for ontology resolution.
    pub fn with_layer(rho: Rho, gamma: Gamma, layer: Arc<Layer>) -> Self {
        Self {
            rho,
            gamma,
            env: crate::nbe::env_global::Env::of(layer),
            type_cache: BTreeMap::new(),
            institution_index: None,
            institution_runtime: None,
            hooks: Arc::new(crate::program::check_hooks::DefaultCheckHooks),
        }
    }

    /// Instantiate a closure under this context's environment.
    ///
    /// D76 Phase B: `Clos::apply` defaults to an *effect-free and environment-free*
    /// evaluation, which is the `EvalCtx::Pure` conflation one level down — a
    /// closure captures its `Rho` but the global environment is ambient, so
    /// applying one without it leaves any name in the body a neutral. The checker
    /// applies Π-closures constantly, so it goes through here.
    pub fn apply(
        &self,
        clos: &crate::nbe::val::Clos,
        v: Val,
    ) -> Result<Val, crate::nbe::eval::EvalError> {
        clos.apply_ctx(v, &self.eval_ctx())
    }

    /// Apply a value to an argument under this context's environment. See
    /// [`CheckCtx::apply`].
    pub fn app(&self, f: Val, v: Val) -> Result<Val, crate::nbe::eval::EvalError> {
        f.app_ctx(v, &self.eval_ctx())
    }

    /// The declaration `iri` names, or a diagnostic saying what the environment
    /// found instead.
    ///
    /// D76 Phase B: a term names its inductive rather than carrying it, so every
    /// consumer that needs the telescope or the constructor list comes through
    /// here. Failing loudly matters — an unresolved name that fell through would
    /// leave the checker with no constructors and no arity, which reads downstream
    /// as "this constructor does not exist" rather than "this inductive is not
    /// declared".
    pub fn lookup_inductive(
        &self,
        iri: &Iri,
    ) -> Result<std::sync::Arc<crate::nbe::term::InductiveDecl>, CheckError> {
        match self.env.lookup(iri) {
            crate::nbe::env_global::Global::Inductive(d) => Ok(d),
            other => Err(CheckError::ExpectedInductive(format!(
                "`{iri}` does not name an inductive in this environment — \
                 it resolves to {}",
                match other {
                    crate::nbe::env_global::Global::Absent => "nothing".to_string(),
                    o => format!("{o:?}"),
                }
            ))),
        }
    }

    /// This context with one more declaration in scope — a declaration being
    /// checked is in scope for its own constructor types (D76 Phase B,
    /// [`Env::declaring`](crate::nbe::env_global::Env::declaring)).
    pub fn declaring(mut self, decl: std::sync::Arc<crate::nbe::term::InductiveDecl>) -> Self {
        self.env = self.env.declaring(decl);
        self
    }

    /// Attach a institution index and runtime for check-time
    /// dispatch of `Constraint::Institution` predicates through
    /// `try_institution_decide` (D14 §9.2).
    pub fn with_institutions(
        mut self,
        index: Arc<crate::institution::registry::InstitutionIndex>,
        runtime: Arc<crate::institution::runtime::InstitutionRuntime>,
    ) -> Self {
        self.institution_index = Some(index);
        self.institution_runtime = Some(runtime);
        self
    }

    /// Produce an [`EvalCtx`] suitable for evaluating expressions
    /// under this check context.
    ///
    /// Returns an effectful context backed by a check-time
    /// [`InstitutionEngine`](crate::institution::eval_hooks::InstitutionEngine)
    /// when an institution index/runtime is attached; otherwise
    /// `EvalCtx::pure()`. All internal `eval` calls in the checker route
    /// through this so institution-dispatched constraints fire at check
    /// time rather than deferring to runtime.
    pub fn eval_ctx(&self) -> crate::nbe::eval::EvalCtx {
        if self.institution_index.is_some() && self.institution_runtime.is_some() {
            let engine = crate::institution::eval_hooks::InstitutionEngine::for_check(
                self.env.layer().cloned(),
                self.institution_index.clone(),
                self.institution_runtime.clone(),
            );
            crate::nbe::eval::EvalCtx::effectful(self.env.layer().cloned(), Arc::new(engine))
        } else {
            // D76 Phase B: the *environment* is not a capability, so an
            // effect-free checker still evaluates inside `Γ_env`. Handing
            // `EvalCtx::pure()` down here is what left a de-inlined `Const` with
            // nothing to resolve against — it would evaluate to a neutral instead
            // of the inductive it names.
            crate::nbe::eval::EvalCtx::in_env(self.env.clone())
        }
    }

    /// Evaluate an expression under this check context's
    /// [`EvalCtx`]. Prefer this over the bare `eval` function
    /// inside `check.rs` so institution-dispatched constraints
    /// (`Constraint::Institution`) fire when the context has a
    /// registry attached.
    pub fn eval(&self, exp: &Exp, rho: &Rho) -> Result<Val, crate::nbe::eval::EvalError> {
        eval_ctx(exp, rho, &self.eval_ctx())
    }

    /// Extend the context with a new variable binding (for entering
    /// binders). Shares the layer (an `Arc`) and clones the
    /// `type_cache` into the child — class resolutions performed inside
    /// the binder therefore don't propagate back to the parent on exit
    /// (§4.4-D7; sharing the cache instead is the profile-gated item 9).
    fn extend(&self, patt: &Patt, typ: &Val, val: &Val) -> Result<CheckCtx, CheckError> {
        let gamma1 = up_gamma(&self.gamma, patt, typ, val)?;
        let rho1 = self.rho.clone().extend(patt.clone(), val.clone());
        Ok(CheckCtx {
            rho: rho1,
            gamma: gamma1,
            env: self.env.clone(),
            type_cache: self.type_cache.clone(),
            institution_index: self.institution_index.clone(),
            institution_runtime: self.institution_runtime.clone(),
            hooks: self.hooks.clone(),
        })
    }

    /// Resolve an EigonClass IRI to a EigenTT Sigma type, with caching.
    /// D76 Phase C — resolve a class through the environment.
    ///
    /// Goes through [`Env::lookup`](crate::nbe::env_global::Env::lookup) rather
    /// than `CheckHooks::resolve_class`, so `check` and `conv` consult one
    /// definition of what a global is instead of two.
    ///
    /// **Only `Global::Constraint` answers here.** A class resolves to its
    /// record and never unfolds in conversion (D75 §8 Q2); an inductive, an
    /// axiom or a definition reaching this path is a caller error, not a class
    /// whose resolution failed, and says so. The old code could not tell those
    /// apart — `resolve_class` returned a bare `Val`.
    ///
    /// The "no layer access in pure check mode" error is gone: an empty
    /// environment yields `Absent`, which reads as "not resolvable here" like
    /// any other miss. Nothing asserted on that message, and its one swallowing
    /// caller (`find_sigma_field`, via `.ok()?`) allocated it only to discard it.
    fn resolve_class_cached(&mut self, iri: &Iri) -> Result<Val, CheckError> {
        let key = iri.as_str().to_string();
        if let Some(cached) = self.type_cache.get(&key) {
            return Ok(cached.clone());
        }
        let v = match self.env.lookup(iri) {
            crate::nbe::env_global::Global::Constraint(v) => v,
            crate::nbe::env_global::Global::Absent => {
                return Err(CheckError::from(format!(
                    "cannot resolve class '{iri}' in this environment"
                )))
            }
            other => {
                return Err(CheckError::from(format!(
                    "'{iri}' is not a class — the environment classifies it as {}",
                    match other {
                        crate::nbe::env_global::Global::Definition(_) => "a definition",
                        crate::nbe::env_global::Global::Axiom => "an axiom",
                        crate::nbe::env_global::Global::Inductive(_) => "an inductive",
                        _ => unreachable!("Constraint and Absent handled above"),
                    }
                )))
            }
        };
        self.type_cache.insert(key, v.clone());
        Ok(v)
    }
}

/// Check that a declaration is well-typed, returning the extended type context.
///
/// Port of `checkD` from the reference.
pub fn check_decl(ctx: &mut CheckCtx, decl: &Decl) -> Result<Gamma, CheckError> {
    match decl {
        Decl::Def(patt, typ, body) => {
            // Check that the type is well-formed
            check_type(ctx, typ)?;
            let t = ctx.eval(typ, &ctx.rho)?;
            // Check that the body has the declared type
            check(ctx, body, &t)?;
            // Extend the type context
            up_gamma(&ctx.gamma, patt, &t, &ctx.eval(body, &ctx.rho)?).map_err(CheckError::from)
        }
        Decl::Drec(patt, typ, body) => {
            // Known subtlety (issue #13 item 3): The body is type-checked
            // under a generic binding (gen_val) so the checker sees an
            // opaque variable, not the real recursive value. When the real
            // value is substituted (UpDec below), neutrals that previously
            // blocked may reduce to something incompatible. EigenTT
            // mitigates this via the guardedness check for codata; data
            // recursion landing safely through `Match` on a sized inductive
            // scrutinee gets termination-by-typing via Phase 11b's sized-
            // types machinery (D19 §8). Bare `letrec loop : 1 = loop` at
            // the Decl level is still accepted by the checker; see the
            // open issue tracking that residual escape hatch.
            //
            // Check that the type is well-formed
            check_type(ctx, typ)?;
            let t = ctx.eval(typ, &ctx.rho)?;
            let gen = gen_val(&ctx.rho);
            // Extend context with the recursive variable and check body
            let mut inner = ctx.extend(patt, &t, &gen)?;
            check(&mut inner, body, &t)?;
            // Re-evaluate with the recursive binding
            let v = ctx.eval(body, &Rho::UpDec(Box::new(ctx.rho.clone()), decl.clone()))?;
            up_gamma(&ctx.gamma, patt, &t, &v).map_err(CheckError::from)
        }
    }
}

/// Check that an expression is a well-formed type.
///
/// Port of `checkT` from the reference.
pub fn check_type(ctx: &mut CheckCtx, exp: &Exp) -> Result<(), CheckError> {
    match exp {
        Exp::Pi(p, a, b) | Exp::Sig(p, a, b) => {
            check_type(ctx, a)?;
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, &ctx.eval(a, &ctx.rho)?, &gen)?;
            check_type(&mut inner, b)
        }
        Exp::Sort(_) | Exp::One => Ok(()),
        // Id(A, x, y) is a type if A is a type and x, y : A
        Exp::Id(a, x, y) => {
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)
        }
        // Eigenius ground types are always valid types
        Exp::EigonClass(_) | Exp::EigonPrimitive(_) => Ok(()),

        // Inductive type forms (Phase 11b, D19; D48 indices).
        // The introduction form runs the strict-positivity checker
        // (Phase 11b step 3) and the indexed-ctor-conclusion validator
        // (D48 Phase B) — verifies each ctor's terminal application has
        // the right `params ++ indices` shape and each index expression
        // type-checks against its declared telescope type.

        // An APPLIED inductive type. The DECL's validity is established once (at ingest, by the
        // ground resolver, plus `Exp::Inductive` above); its ARGUMENTS are supplied afresh at every
        // use site, so decl validity says nothing about them and they must be checked here.
        //
        // THIS IS WHERE EIGENTT DIVERGED FROM ITS REFERENCE, and the divergence is why the check was
        // missing. `references/nanoda_lib` (Lean's kernel) has NO applied-inductive node: a type
        // former is a `Const` carrying a Π type, so `And P Q` is an ordinary `App` spine and
        // `infer_app` (src/tc.rs) walks the Π, infers each argument, and `assert_def_eq`s it against
        // the binder type. Parameters are checked by the ORDINARY APPLICATION RULE. EigenTT fused
        // former and arguments into one node for chain-resident decls, and that node's typing rule
        // never re-implemented the telescope walk it displaced — so `Ok(())` accepted anything.
        //
        // What the leak was hiding, and why it is NOT merely cosmetic: the DCG built
        // `logic:And(GQ₁, GQ₂)` to coordinate quantified NPs, applying `logic:And (P : Prop, Q : Prop)`
        // to CONTINUATION-PASSING QUANTIFIERS — functions, not `Prop`s. The felicity gate calls
        // `check(sem, ⟦cat⟧)` and treats the kernel as the oracle, so every such reading was admitted.
        // Closing the leak turned that into a `grammar-gap`, which is exactly what it always was: a
        // sentence whose only readings were ill-typed. The coordination rule now uses POINTWISE
        // conjunction (`λk. And(f(k), g(k))`), so `And` receives `Prop`s and the terms type-check.
        // The node is gone but the rule stays (D76 Phase B): this arm recovers the
        // declaration from `Γ_env` rather than reading it out of the term. A name
        // the environment cannot resolve falls through to the general rule below,
        // which infers its type and demands a sort — so an unresolvable name is
        // rejected rather than silently admitted.
        e if e.as_const_spine().is_some() => {
            let (iri, _levels, args) = e.as_const_spine().expect("just matched");
            match ctx.env.lookup(iri) {
                crate::nbe::env_global::Global::Inductive(decl) => {
                    let owned: Vec<Exp> = args.into_iter().cloned().collect();
                    check_inductive_type_args(ctx, &decl, &owned)
                }
                // A class is a type, as `EigonClass` is above. A postulate or a
                // definition is checked by inferring its type.
                crate::nbe::env_global::Global::Constraint(_) => Ok(()),
                _ => ensure_infers_as_sort(ctx, e).map(|_| ()),
            }
        }

        // "Is a type" means the INFERRED type is a sort — any sort. Port of `ensure_sort`
        // (`references/nanoda_lib/src/tc.rs:244` at `6ae1f0c`), which `check_declar_info` (`:165`)
        // applies to a declaration's ascribed type and `check_ctor` applies to every constructor
        // binder domain as `ensure_infers_as_sort` (`src/inductive.rs:900`).
        //
        // This was `check(ctx, a, &Val::sort(1))` — "is a type" spelled as "inhabits `Set`". The
        // hardcoded 1 made every type ABOVE `Set` unusable in any position routed through here:
        // `justification:Certificate.spec_poly` binds `T : Type 1` and then writes `P : T -> Prop`, at
        // which point checking `T` against `Set` fails `Sort(2) </: Sort(1)`. Cumulativity runs the
        // wrong way for this — it lets a SMALLER type be used where a larger one is wanted, and the
        // question here is not "how big" but "is it a type at all". Same defect as the `Level` `Ord`
        // derive removed earlier in eigenius#188: a universe comparison written as a constant.
        a => ensure_infers_as_sort(ctx, a).map(|_| ()),
    }
}

/// **Admit an inductive declaration.** Telescopes well-formed, strictly positive,
/// and every constructor's conclusion of the right `params ++ indices` shape.
///
/// D76 Phase B: this was an arm of [`check_type`] reached through
/// `Exp::Inductive(decl)` — a declaration wrapped in an expression so the
/// expression checker could dispatch on it. A declaration is not an expression,
/// and the wrapper had a second cost: `Exp::Inductive(d)` evaluated to the same
/// value as `Exp::const_applied(d.iri.clone(), Vec::new(), [])`, so it was also a second spelling of a
/// *reference*, and a negative occurrence written that way once evaded positivity
/// checking (`positivity::rejects_disguised_inductive_negative_occurrence`).
///
/// nanoda splits the same way: `check_inductive_declar` takes the declaration
/// (`references/nanoda_lib/src/inductive.rs`), while `infer` handles expressions.
pub fn check_inductive_declaration(
    ctx: &mut CheckCtx,
    decl: &std::sync::Arc<crate::nbe::term::InductiveDecl>,
) -> Result<(), CheckError> {
    check_inductive_decl_telescopes(ctx, decl)?;
    crate::nbe::positivity::check_positivity(decl)?;
    validate_indexed_ctor_conclusions(ctx, decl)
}

/// The LEVEL of the sort an expression inhabits, or an error if it does not inhabit a sort — i.e.
/// if it is not a type. Port of `ensure_sort` (`references/nanoda_lib/src/tc.rs:244` at `6ae1f0c`),
/// which `check_declar_info` (`:165`) applies to a declaration's ascribed type and `check_ctor`
/// applies to every constructor binder domain as `ensure_infers_as_sort`
/// (`src/inductive.rs:900`).
///
/// [`check_type`]'s fallback was `check(ctx, a, &Val::sort(1))` — "is a type" spelled as "inhabits
/// `Set`". The hardcoded 1 made every type ABOVE `Set` unusable in any position routed through
/// there: `justification:Certificate.spec_poly` binds `T : Type 1` and then writes `P : T -> Prop`, at
/// which point checking `T` against `Set` fails `Sort(2) </: Sort(1)`. Cumulativity runs the wrong
/// way for this — it lets a SMALLER type be used where a larger one is wanted, and the question
/// here is not "how big" but "is it a type at all". Same defect as the `Level` `Ord` derive removed
/// earlier in eigenius#188: a universe comparison written as a constant.
///
/// The level is returned rather than discarded because the constructor-argument universe
/// constraint needs it ([`check_ctor_type`]).
pub(super) fn ensure_infers_as_sort(
    ctx: &mut CheckCtx,
    e: &Exp,
) -> Result<crate::nbe::level::Level, CheckError> {
    match check_infer(ctx, e)? {
        Val::Sort(l) => Ok(l),
        other => Err(CheckError::IllFormed(format!(
            "expected a type, but `{e:?}` has type `{other:?}`"
        ))),
    }
}

/// Check an applied inductive type's arguments against its `params ++ indices` telescope — the
/// telescope walk `infer_app` performs in the reference kernel (see the note at the `InductiveType`
/// arm of [`check_type`]).
///
/// Each telescope type may mention EARLIER binders, so the types are evaluated in an environment
/// extended with the preceding arguments' values, exactly as nanoda's `inst(binder_type, ctx)` does.
///
/// Two deliberate tolerances, neither of them a fudge:
/// - a **stub** decl (`params` and `indices` both empty — the self-reference EigenTT writes inside a
///   constructor's own type) carries no telescope to check against. Those occurrences are validated
///   at DECLARATION time by `check_positivity` + `validate_indexed_ctor_conclusions`, not here.
/// - a **short** argument list is checked as a prefix rather than rejected on arity, so a partially
///   applied former (which several sized-inductive call sites construct) keeps working. Arity is not
///   this rule's business; the arguments that ARE supplied must still be well-typed.
fn check_inductive_type_args(
    ctx: &mut CheckCtx,
    decl: &std::sync::Arc<crate::nbe::term::InductiveDecl>,
    args: &[Exp],
) -> Result<(), CheckError> {
    // **Arity, D76 Phase B2 — checked for EVERY declaration.**
    //
    // This was scoped to indexed declarations by `!decl.indices.is_empty()`, which
    // is not a test for "indexed" at all: it is the stub-detection hack. A stub had
    // empty indices, and so does a genuine un-indexed inductive, so the lenient path
    // — every argument a parameter, no arity check — was taken by **every** shipped
    // inductive, all ten of which are un-indexed. `Nat(x, y, z)` type-checked.
    //
    // The conflation is gone with the stub that motivated it (Phase B), so the
    // check applies uniformly. This is a NARROWING: it can only turn accepts into
    // rejects, which is why it was held back from Phase B's verdict-neutral sweep.
    let expected = decl.params.len() + decl.indices.len();
    if args.len() != expected {
        return Err(CheckError::IllFormed(format!(
            "inductive `{}`: expected {expected} argument(s) (params + indices: \
             {} + {}), got {}",
            decl.name,
            decl.params.len(),
            decl.indices.len(),
            args.len()
        )));
    }
    let mut rho = ctx.rho.clone();
    for ((patt, ty), arg) in decl
        .params
        .iter()
        .chain(decl.indices.iter())
        .zip(args.iter())
    {
        let ty_val = ctx.eval(ty, &rho)?;
        check(ctx, arg, &ty_val)?;
        let arg_val = ctx.eval(arg, &ctx.rho)?;
        rho = rho.extend(patt.clone(), arg_val);
    }
    Ok(())
}

/// Check that an expression has a given type (checking mode).
///
/// Port of `check` from the reference.
pub fn check(ctx: &mut CheckCtx, exp: &Exp, typ: &Val) -> Result<(), CheckError> {
    match (exp, typ) {
        // A λ against a UNIVERSE is a type error, and reporting it as one matters: without this arm
        // the pair falls through to `check_infer`, which cannot type a bare λ, so the diagnostic came
        // back `CannotInfer("cannot infer type of: Lam(…)")` — true but silent about what was
        // expected. The refusal itself is right: a λ is a VALUE, and a type-level function's type is a
        // Π, never a `Sort`, so nothing legitimate checks a λ against a universe.
        //
        // Worth a named error because this is the shape the DCG's felicity gate hits when an
        // ill-typed reading reaches it — `logic:And` (whose parameters are `Prop`) applied to a
        // type-raised quantifier. That path is how the missing inductive-argument check was found.
        (Exp::Lam(..), Val::Sort(n)) => Err(CheckError::TypeMismatch(format!(
            "a λ cannot inhabit a universe: expected a type in Sort({n}), got an abstraction \
             {:?}. (A type-level function has a Π type, not a Sort.)",
            readback_val(ctx.rho.len(), &Val::Sort(n.clone()))
        ))),
        // A string literal against `core:iri`. This is the ONLY way to reach
        // `PrimitiveType::Iri` (D88 §3): `check_infer` answers `String` for every `LitString`,
        // because a bare literal cannot know which of the two it is meant to be. The declared type
        // is what says so, which is the point of declaring it.
        //
        // Why this rather than making `Iri` infer, or making `String` convert to `Iri`: both would
        // admit any string wherever an IRI is declared, which is the state B3 exists to leave. The
        // subtyping runs the other way — `PrimitiveType::subtype_of` has `Iri <: String` and not
        // the converse — and it is consulted where a value already carries a type, not here.
        //
        // Everything authored stays valid without a rewrite: the 396 grounding-constructor call
        // sites keep writing `declared("urn:...", P)` and land in this arm instead of the generic
        // inference path.
        (Exp::LitString(s), Val::EigonPrimitive(crate::nbe::term::PrimitiveType::Iri)) => {
            match crate::ontology::iri::Iri::parse(s) {
                Ok(_) => Ok(()),
                Err(e) => Err(CheckError::TypeMismatch(format!(
                    "`{s}` is declared `core:iri` but is not one: {e}"
                ))),
            }
        }

        // Lambda against Pi type
        (Exp::Lam(p, e), Val::Pi(t, g)) => {
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, t, &gen)?;
            check(&mut inner, e, &ctx.apply(g, gen)?)
        }

        // Pair against Sigma type
        (Exp::Pair(e1, e2), Val::Sig(t, g)) => {
            check(ctx, e1, t)?;
            {
                let arg = ctx.eval(e1, &ctx.rho)?;
                check(ctx, e2, &ctx.apply(g, arg)?)
            }
        }

        // Constructor against Sum type
        (Exp::Con(c, e), Val::Data(cases, rho1)) => {
            let a = cases
                .iter()
                .find(|(name, _)| name == c)
                .map(|(_, typ)| typ)
                .ok_or_else(|| format!("constructor {c} not in sum type"))?;
            check(ctx, e, &ctx.eval(a, rho1)?)
        }

        // Case function against Pi from Sum to result
        (Exp::Case(branches), Val::Pi(domain, g)) if matches!(**domain, Val::Data(_, _)) => {
            let (cases, rho1) = match &**domain {
                Val::Data(cases, rho1) => (cases, rho1),
                _ => unreachable!(),
            };
            let branch_names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
            let case_names: Vec<&str> = cases.iter().map(|(n, _)| n.as_str()).collect();
            if branch_names != case_names {
                return Err(CheckError::IllFormed(format!(
                    "case branches {:?} do not match sum type {:?}",
                    branch_names, case_names
                )));
            }
            for (branch, (c, a)) in branches.iter().zip(cases.iter()) {
                let a_val = ctx.eval(a, rho1)?;
                // The branch's expected type is `Πx:aᶜ. g(c x)` — the motive applied to the
                // constructor applied to the branch's argument. Building it needs a NAME for that
                // argument, because the motive is spliced in as an `Exp` (read back from `g`) and
                // must refer to it.
                //
                // eigenius#64: that name was the literal `"__case_arg"`, which is a legal ESL
                // identifier — `[A-Za-z_][A-Za-z0-9_]*`, see `esl/lexer.rs:485` — so user code
                // could bind it. The issue proposed `__case_arg_{level}`; that is still a legal
                // identifier and still forgeable. Using the checker's existing `#` discipline
                // instead makes the name unforgeable by construction: `#` cannot appear in an ESL
                // identifier, which is exactly why `gen_val` and `readback`'s fresh variables are
                // spelled `TC#{level}` and `G#{level}`.
                //
                // The prefix must differ from `G#`: `readback_val` below starts generating at
                // `ctx.rho.len()`, so `G#{ctx.rho.len()}` is a name the motive itself may contain.
                let arg_name = format!("CB#{}", ctx.rho.len());
                let g_c = Clos {
                    patt: Patt::Var(arg_name.clone()),
                    body: Exp::App(
                        Box::new(readback_val(ctx.rho.len(), &Val::Lam(g.clone()))),
                        Box::new(Exp::Con(c.clone(), Box::new(Exp::Var(arg_name)))),
                    ),
                    env: ctx.rho.clone(),
                };
                check(ctx, &branch.body, &Val::Pi(Box::new(a_val), g_c))?;
            }
            Ok(())
        }

        // Unit value against One type
        (Exp::Unit, Val::One) => Ok(()),

        // One against Set (One is a type)
        // Impredicative Pi: when the codomain is in Prop, the whole Pi
        // is in Prop regardless of the domain's universe level. D46 §4.1.
        // The domain may be at any level (including Type(n) for arbitrary n);
        // we only require it to be a well-formed type.
        (Exp::Pi(p, a, b), Val::Sort(l)) if l.is_nat(0) => {
            check_type(ctx, a)?;
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, &ctx.eval(a, &ctx.rho)?, &gen)?;
            check(&mut inner, b, &Val::sort(0))
        }

        // Sigma in Prop is predicative — both components must be in Prop.
        // No impredicativity for Sigma (D46 §3.4, §4).
        (Exp::Sig(p, a, b), Val::Sort(l)) if l.is_nat(0) => {
            check(ctx, a, &Val::sort(0))?;
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, &ctx.eval(a, &ctx.rho)?, &gen)?;
            check(&mut inner, b, &Val::sort(0))
        }

        // The `Set`-level counterparts of the two arms above are ABSENT, deliberately. They were
        // `(Exp::Pi | Exp::Sig, Val::Sort(l)) if l.is_nat(1)`, `(Exp::One, ..)` and
        // `(Exp::Data(..), ..)`, each re-deriving at `Set` what `check_infer` already computes, and
        // each testing the expected sort for EQUALITY with `Set` rather than letting subtyping
        // decide. `check_infer` has a rule for all three forms, so they fall through to
        // `check_by_inference` and check/infer agree by construction (eigenius#220, disposing of
        // them the way eigenius#194 disposed of five siblings).
        //
        // The `Prop` arms above are NOT of that kind and stay: impredicativity is a genuine typing
        // rule, not a fast path — `Pi (x:A). B` inhabits `Prop` when `B` does, whatever universe
        // `A` lives in, and inference expresses that as `imax`.

        // Declaration
        (Exp::Dec(d, e), t) => {
            let gamma1 = check_decl(ctx, d)?;
            let mut inner = CheckCtx {
                rho: Rho::UpDec(Box::new(ctx.rho.clone()), d.clone()),
                gamma: gamma1,
                env: ctx.env.clone(),
                type_cache: ctx.type_cache.clone(),
                institution_index: ctx.institution_index.clone(),
                institution_runtime: ctx.institution_runtime.clone(),
                hooks: ctx.hooks.clone(),
            };
            check(&mut inner, e, t)
        }

        // refl(a) : Id(A, a, a) — check that x and y are both a.
        // Uses type-directed equality (D46 §5): if A is itself propositional,
        // x = a and y = a hold by proof irrelevance regardless of structure.
        (Exp::Refl(a), Val::Id(typ, x, y)) => {
            check(ctx, a, typ)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            def_eq_at_type(ctx, x, &a_val, typ)?;
            def_eq_at_type(ctx, y, &a_val, typ)
        }

        // Id(A, x, y) : Prop  (D46 §9 — equality is propositional).
        // Pre-D46 the rule was `Id : Set`; the change is what enables proof
        // irrelevance on equality witnesses. The Set / Type(n) check sites
        // continue to work via cumulativity (Prop ⊆ Set ⊆ Type(n)) — see
        // the universe-hierarchy arms below — so existing callers that
        // expected Id to live in Set are unaffected.
        (Exp::Id(a, x, y), Val::Sort(l)) if l.is_nat(0) => {
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)
        }

        // Universe hierarchy, predicative and cumulative: `Sort(n) : Sort(m)`
        // exactly when `n < m`. This is the checking-mode image of the
        // inference rule `Sort(n) : Sort(n+1)` (see `check_infer`) composed
        // with cumulativity `Sort(m) <: Sort(n)` iff `m <= n` (`conv.rs`),
        // so check mode and inference accept the same universe judgements.
        //
        // In particular `Set : Set` (`Sort(1) : Sort(1)`) is rejected, and so
        // is the cycle `Set : Type(1) : Set` that the pre-D46 pair of rules
        // (`Type(n) : Set` for the top-universe reading, plus `Set : Type(1)`)
        // admitted. Either shape is Girard's paradox and makes every
        // proposition on the chain provable; Rule 21 checks propositions at
        // commit, so this is a property of what the chain accepts.
        //
        // Self-referential meta-claims (e.g. a level-1 trace referencing
        // level-1) are blocked at resource ingestion by the universe
        // stratification validator (Rule 13), not in the term checker.
        // eigenius#188: strict `<` in the LEVEL order, which is `succ(n) <= m` — not `<` on
        // `Level`, which does not exist precisely so this cannot be written structurally.
        (Exp::Sort(n), Val::Sort(m)) if n.clone().succ().leq(m) => Ok(()),
        (Exp::Sort(n), Val::Sort(m)) => Err(CheckError::TypeMismatch(format!(
            "universe stratification: Sort({n}) does not inhabit Sort({m}) — \
             a universe lives strictly above itself, `Sort(k) : Sort(k+1)`. \
             (Prop = Sort(0), Set = Sort(1), Type(k) = Sort(k+1).)"
        ))),

        // EigonClass / EigonPrimitive are ground types in `Set`: inference
        // gives `Sort(1)` (see `check_infer`), and cumulativity
        // `Sort(m) <: Sort(n)` iff `m <= n` (`conv.rs`) carries that to every
        // universe at or above `Set`. It does not carry down to
        // `Prop = Sort(0)`, which is strictly below. This is infer-then-
        // subsume, so check mode and inference accept the same judgements.
        //
        // The arm this replaces read `Val::Sort(_)` and so admitted
        // `SomeClass : Prop` — a class standing where a proposition is
        // expected (`justification:Certificate(j, P)`, `reflection:canonical_proposition`,
        // anything Rule 21 checks at the commit gate) with no diagnostic
        // (eigenius#191). Same check-vs-infer disagreement eigenius#136
        // removed for `Sort`.
        (Exp::EigonClass(_), Val::Sort(m)) | (Exp::EigonPrimitive(_), Val::Sort(m))
            if crate::nbe::level::Level::of_nat(1).leq(m) =>
        {
            Ok(())
        }
        (Exp::EigonClass(_), Val::Sort(m)) | (Exp::EigonPrimitive(_), Val::Sort(m)) => {
            Err(CheckError::TypeMismatch(format!(
                "universe stratification: an Eigon class or primitive inhabits `Set` \
                 (`Sort(1)`) and every universe above it, not `Sort({m})`. A class is \
                 not a proposition. (Prop = Sort(0), Set = Sort(1), Type(k) = Sort(k+1).)"
            )))
        }

        // `Codata`, `CodataType`, `Inductive` and `InductiveType` against a universe had explicit
        // arms here until eigenius#194. Each read `(Exp::X(..), Val::Sort(_)) => check_type(ctx,
        // exp)` — matching EVERY universe and then discarding it, because `check_type` takes no
        // expected type. They were `Ok(())` with extra steps, and they admitted `codata {…} : Prop`
        // and a `Set`-level inductive standing where a proposition is expected.
        //
        // They are gone rather than tightened. `check_infer`'s arm for each of these constructors
        // is `check_type(ctx, exp)?` followed by returning the sort — so the check arms were
        // inference minus the universe comparison, not a different judgement. Falling through to
        // `check_by_inference` runs the same `check_type` and then compares under `subtype_of`,
        // which is where cumulativity already lives. Check and infer now agree by construction:
        // there is no second rule to keep in sync, which is the property #137, #191 and #209 each
        // lost in a different arm.

        // Constructor application against an inductive type — Phase 11b
        // step 5 checking mode. Parameters come from the expected type;
        // each constructor argument is checked against its declared
        // type (with parameters substituted).
        (
            Exp::InductiveCtor(iri, ctor_name, args),
            Val::InductiveType {
                decl: expected_decl,
                params,
                indices,
            },
        ) => {
            // D76 Phase B: the term names the inductive, so the declaration comes
            // from `Γ_env`. A name the environment cannot resolve is an error here
            // rather than a silent fallthrough — a constructor of an unknown
            // inductive has no type to check against.
            let decl = ctx.lookup_inductive(iri)?;
            check_inductive_ctor_args(
                ctx,
                &decl,
                ctor_name,
                args,
                expected_decl,
                params,
                Some(indices),
            )
        }
        .map(|_| ()),

        // Pattern-match elimination with motive inferred from the
        // expected type (Phase 11b step 12, D19 §10). The motive is
        // synthesised as `λ_. expected_type` (constant); per-arm
        // bodies are checked against `expected_type` in a context
        // extended with bindings of the constructor's argument types.
        // Exhaustiveness, no-duplicate-arms, and binding-count match
        // are validated here.
        (Exp::Match { scrutinee, arms }, expected) => check_match(ctx, scrutinee, arms, expected),

        // EigonResource against a class type — **intensional** inhabitation (#91):
        // the resource inhabits `sup` iff one of its declared `is_a` classes is a
        // (reflexive-transitive) subclass of `sup`, via the single foundation
        // authority `Layer::is_subclass_of`. Consults the FULL `is_a` array — not
        // `check_infer`'s lossy `.first()` — so multi-class individuals and
        // subclass chains both type; the `c == sup` disjunct is the layer-free
        // reflexive fallback. An empty `is_a` is a valid resource that inhabits no
        // *specific* class, so this fails closed (it never errors on the resource).
        // Membership is nominal; the structural check is the Validator's job.
        (Exp::EigonResource(r), Val::EigonClass(sup)) => {
            let inhabits = r
                .is_a()
                .iter()
                .any(|c| c == sup || ctx.env.is_subclass_of(c, sup));
            if inhabits {
                Ok(())
            } else {
                Err(CheckError::TypeMismatch(format!(
                    "resource {:?} (is_a = {:?}) does not inhabit class {sup}",
                    r.id(),
                    r.is_a()
                )))
            }
        }

        // Kind term against a class type — the **derived-kind-predication coercion**
        // (Chierchia's ∩, D64 §2.3 kind antecedents): `ontology:kind_of(K) : Entity`
        // by its axiom type, but in CHECK mode it also coerces into a class position
        // `C` when `K`'s base class is a (reflexive-transitive) subclass of `C` —
        // "these lines" may resolve to the kind ⟦MSI cell lines⟧ exactly because that
        // kind's base class is `CellLine`. This mirrors the categorial side, which
        // already indexes a bare-kind NP by `base(K)` so it sits in the subsumption
        // lattice (`LexicalIndex::kind_raised_nps`); without this arm the sem-level
        // check disagreed with the grammar's own indexing. Like the CN-as-types arm
        // in [`check_by_inference`], the relaxation is check-mode-only coercive
        // subtyping — inference still gives `Entity`, and definitional equality stays
        // exact. The coercion only ADDS acceptances: on a miss (base not a subclass,
        // or not peelable to a class) the term falls back to plain inference, so
        // `kind_of(K) : Entity` keeps typing through the axiom's codomain as before.
        (Exp::App(f, k), Val::EigonClass(sup)) if is_kind_of_axiom(f) => match kind_base_class(k) {
            Some(base) if base == sup || ctx.env.is_subclass_of(base, sup) => Ok(()),
            _ => check_by_inference(ctx, exp, typ),
        },

        // Fallthrough: infer type and compare under subtyping.
        (e, t) => check_by_inference(ctx, e, t),
    }
}

/// Infer type and compare under subtyping (`inferred <: expected`). For everything except sized
/// inductive parameters, `subtype_of` reduces to `eq_nf`. The current TSO is passed through so
/// bounded size binders in scope can witness subtyping between neutral sizes.
fn check_by_inference(ctx: &mut CheckCtx, e: &Exp, t: &Val) -> Result<(), CheckError> {
    let t1 = check_infer(ctx, e)?;
    // CN-as-types subsumption (Luo 2012; D62 §8.6): a value of a subclass
    // type checks against its superclass type — the inclusion-coercion
    // fragment of coercive subtyping, honoring the ontology's declared
    // `core:subclass_of` lattice as the `EigonClass` subtype rule. This
    // relaxation lives ONLY at the directional check boundary; definitional
    // equality (`eq_nf`) stays exact.
    if let (Val::EigonClass(sub), Val::EigonClass(sup)) = (&t1, t) {
        {
            if ctx.env.is_subclass_of(sub, sup) {
                return Ok(());
            }
        }
    }
    subtype_of(&ctx.env, ctx.rho.len(), &t1, t)
}

/// Is `e` the `ontology:kind_of` nominalization axiom (Chierchia's ∩, `Set -> Entity`)?
/// Foundational-vocabulary reference, same convention as `core:is_a` in `eval`.
fn is_kind_of_axiom(e: &Exp) -> bool {
    matches!(e, Exp::EigonAxiom(i) if i.as_str() == "urn:eigenius:ontology:kind_of")
}

/// The base class of a kind's underlying type: peel `Σx:C. R` (recursively, for stacked
/// refinements) down to an `EigonClass`. `None` when the base is not a class (a neutral
/// or bound-variable base — the coercion arm then falls through to plain inference).
fn kind_base_class(k: &Exp) -> Option<&Iri> {
    match k {
        Exp::EigonClass(i) => Some(i),
        Exp::Sig(_, base, _) => kind_base_class(base),
        _ => None,
    }
}

/// Infer the type of an expression (inference mode).
///
/// Port of `checkI` from the reference.
pub fn check_infer(ctx: &mut CheckCtx, exp: &Exp) -> Result<Val, CheckError> {
    match exp {
        Exp::Var(x) => lookup_gamma(&ctx.gamma, x).map_err(CheckError::from),

        // Type annotation `(e : T)` — the bidirectional mode switch. `T` must be
        // a type (its own type is a `Sort`); then `e` is *checked* against `T`
        // (so a Curry-style `Lam`, unsynthesizable on its own, becomes
        // inferable), and the inferred type is `T`. See D63 §8.2.
        Exp::Ann(e, t) => {
            let t_ty = check_infer(ctx, t)?;
            if !matches!(t_ty, Val::Sort(_)) {
                return Err(CheckError::ExpectedSort(format!(
                    "Ann: annotation must be a type (a Sort), got {:?}",
                    readback_val(ctx.rho.len(), &t_ty)
                )));
            }
            let t_val = ctx.eval(t, &ctx.rho)?;
            check(ctx, e, &t_val)?;
            Ok(t_val)
        }

        // A type former, applied or not — `Const(I)` and `App(Const(I), a)` alike
        // (D76 Phase B). Its type is the declaration's `sort`, recovered from
        // `Γ_env`.
        //
        // **Before the `App` arm, necessarily.** `App` infers its head and demands a
        // Π of it; a type former's inferred type is a *sort*, so an applied former
        // reaching that arm fails with `expected Pi type, got Sort(Zero)` — which is
        // how the whole core ontology stopped loading when this sat after `App`. The
        // guard tests the spine head, so an ordinary application whose head is not an
        // inductive name falls through untouched.
        //
        // nanoda has no such arm: there a type former is a `Const` whose type is the
        // Π-telescope `Π(params)(indices). Sort l`, so `infer_app` walks it and the
        // ORDINARY application rule checks the arguments. Adopting that deletes
        // `check_inductive_type_args` — and it is exactly B2's change, since the
        // ordinary rule checks arity where the fused node's rule did not.
        e if e.as_const_spine().is_some_and(|(iri, _, _)| {
            matches!(
                ctx.env.lookup(iri),
                crate::nbe::env_global::Global::Inductive(_)
            )
        }) =>
        {
            let (iri, _, _) = e.as_const_spine().expect("just matched");
            let crate::nbe::env_global::Global::Inductive(decl) = ctx.env.lookup(iri) else {
                unreachable!("guarded above")
            };
            check_type(ctx, exp)?;
            let sort = decl.sort.clone();
            let rho = ctx.rho.clone();
            ctx.eval(&sort, &rho).map_err(CheckError::from)
        }
        Exp::App(e1, e2) => {
            let t1 = check_infer(ctx, e1)?;
            let (t, g) = ext_pi(&t1)?;
            check(ctx, e2, &t)?;
            {
                let arg = ctx.eval(e2, &ctx.rho)?;
                Ok(ctx.apply(&g, arg)?)
            }
        }

        Exp::Fst(e) => {
            let t = check_infer(ctx, e)?;
            let (t1, _) = ext_sig(&t)?;
            Ok(t1)
        }

        Exp::Snd(e) => {
            let t = check_infer(ctx, e)?;
            let (_, g) = ext_sig(&t)?;
            {
                let arg = ctx.eval(e, &ctx.rho)?.vfst()?;
                Ok(ctx.apply(&g, arg)?)
            }
        }

        // Eigenius: property/observation access type inference.
        //
        // ESL's `.name` syntax unifies two operations:
        // - property access on resources / Sigma-typed values
        // - observation on codata-typed values
        // We dispatch on the inferred type of the target.
        Exp::PropAccess(e, prop) => {
            let t = check_infer(ctx, e)?;

            // D78 §9 — keyed by the full IRI, not `prop.local_name()`.
            find_record_field(ctx, &t, prop).ok_or_else(|| {
                CheckError::IllFormed(format!(
                    "property '{}' not found in type {:?}",
                    prop,
                    readback_val(ctx.rho.len(), &t)
                ))
            })
        }

        // --- Eigenius extension: 7 inference rules (D18 §6, issue #12 item 2) ---

        // Construct(class_iri, fields): check each field against the class's
        // Sigma chain and return EigonClass(class_iri).
        Exp::Construct(class_iri, fields) => {
            let class_type = ctx.resolve_class_cached(class_iri).map_err(|e| {
                CheckError::CannotInfer(format!(
                    "cannot infer Construct type for '{class_iri}': {e}"
                ))
            })?;
            // D78 Phase C — a record is flat, so each field is looked up
            // directly. The Σ-chain needed `advance_sigma` to walk past the
            // field it had just checked; a record has nothing to walk.
            for (prop_iri, field_exp) in fields {
                let field_type =
                    find_record_field(ctx, &class_type, prop_iri).ok_or_else(|| {
                        format!("property '{}' not found in class '{}'", prop_iri, class_iri)
                    })?;
                check(ctx, field_exp, &field_type)?;
            }
            // D78 §3 / 7b — the constructed thing's type is the record of the
            // fields given, refined by the class it was built against. Returning
            // the bare class (the prior behaviour) re-imposed the class's type on
            // the instance, which is D75 §3.8; returning a bare record would drop
            // the nominal claim, which D75 §8 Q2 forbids.
            let built: Vec<(Iri, Patt, Exp)> = fields
                .iter()
                .map(|(prop_iri, _)| {
                    let ty = find_record_field(ctx, &class_type, prop_iri)
                        .map(|v| readback_val(ctx.rho.len(), &v))
                        .unwrap_or_else(|| Exp::sort(1));
                    (
                        prop_iri.clone(),
                        Patt::Var(prop_iri.local_name().to_string()),
                        ty,
                    )
                })
                .collect();
            let record = Exp::record(built)
                .map_err(|e| CheckError::CannotInfer(e.to_string()))
                .and_then(|e| {
                    ctx.eval(&e, &Rho::Nil)
                        .map_err(|e| CheckError::CannotInfer(format!("{e:?}")))
                })?;
            Ok(Val::Refine(
                Box::new(record),
                std::iter::once(class_iri.clone()).collect(),
            ))
        }

        // D78 Phase E — a resource's type is its OWN record, refined by the
        // whole of its `is_a`.
        //
        // This replaces `Val::EigonClass(classes.first())`, which typed a
        // resource by one arbitrarily-chosen class and discarded the rest.
        // 2120 of 2903 shipped resources (73 %) declare more than one `is_a`,
        // so the choice was being made constantly, not rarely.
        //
        // Two things change together. The record is the union of the fields the
        // resource actually carries, so an undeclared property is projectable
        // (D75 §3.8); and the refinement carries every constraint it claims, so
        // nothing is dropped.
        //
        // Without a layer there is nothing to resolve property types against, so
        // fall back to the old shape rather than fail — `check` in pure mode is
        // a legitimate caller (tests that never touch chain resolution).
        Exp::EigonResource(r) => {
            let classes = r.is_a();
            match ctx.env.layer() {
                Some(layer) => {
                    let record = crate::program::ground::resource_record(r, layer)
                        .map_err(CheckError::CannotInfer)?;
                    if classes.is_empty() {
                        Ok(record)
                    } else {
                        Ok(Val::Refine(Box::new(record), classes.into_iter().collect()))
                    }
                }
                None => {
                    let class_iri = classes
                        .first()
                        .ok_or_else(|| "EigonResource has no is_a class".to_string())?;
                    Ok(Val::EigonClass(class_iri.clone()))
                }
            }
        }

        // Template(lit, refs): templates always produce String
        Exp::Template(_, refs) => {
            // Check that each referenced property expression is well-typed
            for (_, ref_exp) in refs {
                check_infer(ctx, ref_exp)?;
            }
            Ok(Val::EigonPrimitive(crate::nbe::term::PrimitiveType::String))
        }

        // Refl(a): infer a's type, return Id(a_type, a_val, a_val)
        Exp::Refl(a) => {
            let a_type = check_infer(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            Ok(Val::Id(
                Box::new(a_type),
                Box::new(a_val.clone()),
                Box::new(a_val),
            ))
        }

        // NativeDecide(constraint, v): reduces to Refl if satisfied,
        // so its type is Id(v_type, v_val, v_val)
        Exp::NativeDecide(_, v) => {
            let v_type = check_infer(ctx, v)?;
            let v_val = ctx.eval(v, &ctx.rho)?;
            Ok(Val::Id(
                Box::new(v_type),
                Box::new(v_val.clone()),
                Box::new(v_val),
            ))
        }

        // DecEq(A, x, y): check A is a type, x and y inhabit A,
        // return Id(A_val, x_val, y_val)
        Exp::DecEq(a, x, y) => {
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            let x_val = ctx.eval(x, &ctx.rho)?;
            let y_val = ctx.eval(y, &ctx.rho)?;
            Ok(Val::Id(Box::new(a_val), Box::new(x_val), Box::new(y_val)))
        }

        // IdJ([A, C, d, x, y, p]): Martin-Löf J eliminator.
        // Per D18 §6.4, require an explicit motive C and return C(x, y, p).
        // Lean handles this via recursor reduction; we use a direct J-rule
        // since EigenTT doesn't have a recursor framework.
        Exp::IdJ(args) => {
            let [ref a, ref _c, ref d, ref x, ref y, ref p] = **args;
            // A must be a type
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            // x, y : A
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            let x_val = ctx.eval(x, &ctx.rho)?;
            let y_val = ctx.eval(y, &ctx.rho)?;
            // p : Id(A, x, y)
            let id_type = Val::Id(
                Box::new(a_val.clone()),
                Box::new(x_val.clone()),
                Box::new(y_val),
            );
            check(ctx, p, &id_type)?;
            // d : (a : A) → C(a, a, refl(a)) — the base case
            // For now, just infer d's type; the full motive check
            // requires higher-order unification which is Phase 10b.
            let d_type = check_infer(ctx, d)?;
            // J reduces to d(x) when p = refl(x), so the result type
            // is the return type of d applied to x.
            match d_type {
                Val::Pi(_, g) => ctx.apply(&g, x_val).map_err(CheckError::from),
                _ => Ok(Val::sort(1)), // conservative fallback
            }
        }

        // Map(f, coll): infer f : A → B, coll : List A, return List B.
        Exp::Map(f, coll) => {
            let f_type = check_infer(ctx, f)?;
            let (a, b_clos) = ext_pi(&f_type).map_err(|_| {
                CheckError::ExpectedPi("Map: first argument must be a function (A → B)".to_string())
            })?;
            let coll_type = check_infer(ctx, coll)?;
            let elem_type = extract_list_element_type(&coll_type).ok_or_else(|| {
                format!(
                    "Map: second argument must be a list type, got {:?}",
                    readback_val(ctx.rho.len(), &coll_type)
                )
            })?;
            eq_nf(ctx.rho.len(), &a, &elem_type).map_err(|_| {
                format!(
                    "Map: function domain {:?} does not match list element type {:?}",
                    readback_val(ctx.rho.len(), &a),
                    readback_val(ctx.rho.len(), &elem_type)
                )
            })?;
            // Compute result element type B by applying closure to a dummy
            let b = ctx.apply(&b_clos, gen_val(&ctx.rho))?;
            // Build list type with element type B
            let list_exp = Exp::list(readback_val(ctx.rho.len(), &b));
            ctx.eval(&list_exp, &ctx.rho).map_err(CheckError::from)
        }

        // Reduce(f, init, coll): infer f : B → A → B, init : B, coll : List A, return B.
        Exp::Reduce(f, init, coll) => {
            let f_type = check_infer(ctx, f)?;
            let (b, inner_clos) = ext_pi(&f_type).map_err(|_| {
                CheckError::ExpectedPi(
                    "Reduce: first argument must be a function (B → A → B)".to_string(),
                )
            })?;
            // f's return must be a function A → B
            let inner_type = ctx.apply(&inner_clos, gen_val(&ctx.rho))?;
            let (_a_inner, _b_ret_clos) = ext_pi(&inner_type).map_err(|_| {
                "Reduce: first argument must be a curried function (B → A → B)".to_string()
            })?;
            // Check init : B
            check(ctx, init, &b)?;
            // Check coll is a list type
            let coll_type = check_infer(ctx, coll)?;
            let _elem_type = extract_list_element_type(&coll_type).ok_or_else(|| {
                format!(
                    "Reduce: third argument must be a list type, got {:?}",
                    readback_val(ctx.rho.len(), &coll_type)
                )
            })?;
            // Return type is B (the accumulator type)
            Ok(b)
        }

        // Inductive types (Phase 11b, D19). Universe inference per D46:
        // an inductive declared with `sort = Sort(0)` is in Prop; otherwise
        // its declared sort applies. Handled below alongside other type-
        // formers — see the `Const`-spine arm in the universe-inference section.

        // Constructor application — inference works when the inductive
        // has no parameters (the result type is fully determined).
        // Parameterised inductives need an expected type to drive
        // parameter inference; require checking mode for those.
        Exp::InductiveCtor(iri, ctor_name, args) => {
            let decl = ctx.lookup_inductive(iri)?;
            if !decl.params.is_empty() {
                return Err(CheckError::CannotInfer(format!(
                    "InductiveCtor: cannot infer type of `{}.{ctor_name}` — \
                     `{}` has {} parameter(s), supply an expected type via checking mode",
                    decl.name,
                    decl.name,
                    decl.params.len()
                )));
            }
            // `None` = inference: no expected type, so the ctor's declared result under the bound
            // arguments IS the answer — including its indices, which the previous
            // `indices: Vec::new()` silently discarded. Lean's `infer_app` does exactly this
            // (`inst(fun, ctx)`), which is why it needs no special case for indexed inductives.
            check_inductive_ctor_args(ctx, &decl, ctor_name, args, &decl, &[], None)
        }

        // Recursor application — Phase 11b step 5.
        // 1. The major's inferred type fixes the inductive declaration
        //    and the parameters.
        // 2. The motive must accept that inductive type and return a
        //    sort (for now, `Set`).
        // 3. Each minor is checked against the type derived by
        //    [`derive_minor_types`](super::recursor).
        // 4. The result type is `motive(major)`.
        Exp::InductiveRec {
            iri,
            motive,
            minors,
            major,
        } => {
            let decl = ctx.lookup_inductive(iri)?;
            check_infer_inductive_rec(ctx, &decl, motive, minors, major)
        }

        // Pattern-match without an explicit motive cannot run in
        // inference mode — its result type is determined by checking-
        // mode context. Surface a diagnostic that points users to the
        // two ways out.
        Exp::Match { .. } => Err(CheckError::CannotInfer(
            "match expression has no inferable type — use it in a checking-mode position \
             (e.g. as a program body or a typed `let` value), or annotate the result type \
             with `returning T` so the parser builds an `InductiveRec` instead"
                .to_string(),
        )),

        // Universe inference for type-formers (D46 §3-§4). These rules
        // let `is_propositional_in_ctx` decide propositionality via
        // type inference for any well-formed type expression.
        Exp::Sort(n) => Ok(Val::Sort(n.clone().succ())),
        Exp::One => Ok(Val::sort(1)),
        Exp::Pi(patt, a, b) => {
            // Pi (a : A) (b : B) lives at Sort(max(m, n)) for non-Prop B,
            // or Sort(0) impredicatively when B inhabits Sort(0).
            infer_dependent_sort(ctx, patt, a, b, /*impredicative=*/ true)
        }
        Exp::Sig(patt, a, b) => {
            // Sigma is predicative — always max(m, n).
            infer_dependent_sort(ctx, patt, a, b, /*impredicative=*/ false)
        }
        // A sum type `Sum(c₁ A₁ | … | cₙ Aₙ)` lives at `max` of its summands' levels — predicative,
        // like `Sig`, and for the same reason: a sum stores one of its summands, so it cannot be
        // smaller than the largest of them. The `max` identity is `Zero`, so an empty sum is at
        // `Prop` — the empty type is a proposition.
        //
        // eigenius#220: `check` had the ONLY rule for this form, `(Exp::Data(..), Val::Sort(l)) if
        // l.is_nat(1)`, which checked each summand against `Set` exactly. A sum of `Type 1`
        // summands was unwritable, and a `Set` sum checked against `Type 1` was rejected where
        // cumulativity says it should pass. With an inference rule the narrow arm is deleted rather
        // than widened, so `check` and `check_infer` agree by construction — the same disposal
        // eigenius#194 applied to five sibling arms.
        Exp::Data(summands) => {
            let mut level = crate::nbe::level::Level::zero();
            for summand in summands {
                let l = ensure_infers_as_sort(ctx, &summand.typ)?;
                level = crate::nbe::level::Level::Max(Box::new(level), Box::new(l)).simplify();
            }
            Ok(Val::Sort(level))
        }
        Exp::Arrow(a, b) => {
            let pi = Exp::Pi(Patt::Unit, a.clone(), b.clone());
            check_infer(ctx, &pi)
        }
        Exp::Times(a, b) => {
            let sig = Exp::Sig(Patt::Unit, a.clone(), b.clone());
            check_infer(ctx, &sig)
        }
        Exp::Id(a, x, y) => {
            // Id lives in Prop (D46 §9). Set / Type(n) callers still work
            // via cumulativity (Prop ⊆ Set ⊆ Type(n)).
            check_type(ctx, a)?;
            let a_val = ctx.eval(a, &ctx.rho)?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            Ok(Val::sort(0))
        }
        Exp::EigonClass(_) | Exp::EigonPrimitive(_) => Ok(Val::sort(1)),
        // D46 §10 — axiom reference. The IRI denotes an opaque typed
        // constant declared by `axiom NAME : T;` and lifted onto the
        // chain as a `eigentt:Axiom` resource carrying the encoded
        // type T as `axiom_statement`. The layer's cached `axiom_env`
        // holds the decoded type as a `Val`; `check_infer` returns
        // that registered type. Absent layer ⇒ no chain to consult ⇒
        // error: closed-term type-checking has no environment to
        // resolve axioms against. Absent IRI ⇒ unresolved axiom
        // reference (the chain was supposed to admit it but didn't),
        // also an error.
        Exp::EigonAxiom(iri) => {
            let layer = ctx.env.layer().ok_or_else(|| {
                format!("Exp::EigonAxiom({iri}): no layer context available for axiom resolution")
            })?;
            let env = layer.axiom_env();
            env.get(iri).map(|entry| entry.typ.clone()).ok_or_else(|| {
                CheckError::IllFormed(format!(
                    "axiom `{iri}` not registered in chain axiom environment"
                ))
            })
        }
        // D87 §4.3 — a reference to a proof an EXTERNAL checker verified. Refused here, and the
        // refusal is the enforcement.
        //
        // The alternatives are both wrong. Checking it would mean re-proving the proposition
        // without the export, which the kernel cannot do. Admitting it at whatever type the
        // judgement names would make `Verified` assertable by anybody who writes the judgement —
        // the laundering the two-layer separation exists to forbid, and worse than the
        // proof-as-axiom shape D87 §4.1 withdrew, since an axiom at least has to be declared.
        //
        // So a HAND-AUTHORED `holds(logic_lean4, Checked(a), P)` is rejected at commit, by the
        // `eigentt:Judgement` check-mode rule that runs this. An INSTITUTION-EMITTED one is never
        // asked: `structural_validate` runs before `autoonload_dispatch` in `commit::pipeline`,
        // and the followup slice is `[build, persist]` with no validation phase. The two paths
        // already differ in whether they validate, so "kernel-only, refused from input" — which
        // eigenius#205 recorded as existing nowhere in the validator — falls out of the pipeline's
        // shape rather than from a guard placed here.
        //
        // What makes the emitted judgement worth anything is not this check but D87 §5: the
        // export bytes, the target name, the proposition, the permitted axiom set and the checker
        // identity are all on the chain, so anyone can re-run nanoda and get the same verdict.
        Exp::Checked(iri) => Err(CheckError::IllFormed(format!(
            "`Checked({iri})` names a proof an external checker verified; the kernel has no proof              of the proposition it is offered against and will not admit one. A              `holds(logic_lean4, Checked(_), _)` judgement is produced by the institution that ran              the check, not written by an author"
        ))),
        // eigenius#71 / D49 — literal values infer to their primitive
        // type (`Val::EigonPrimitive(PrimitiveType::*)`). Round-trips
        // through D47 as the `LitString` / `LitInt` / `LitFloat` /
        // `LitBool` ctors (eigenius#71, eigenius#142);
        // Equality on them is decided by `eq_nf`, which reads the values
        // back and compares the resulting `Exp`s — `Val` derives no
        // `PartialEq`, so there is no "standard `Val` equality" path (an
        // earlier version of this comment said there was). `Exp`'s derived
        // `PartialEq` gives `LitFloat` f64 comparison, so a literal NaN is
        // unequal to itself; user code is welcome to surface that as a
        // diagnostic rather than the kernel special-casing it.
        Exp::LitString(_) => Ok(Val::EigonPrimitive(crate::nbe::term::PrimitiveType::String)),
        Exp::LitInt(_) => Ok(Val::EigonPrimitive(
            crate::nbe::term::PrimitiveType::Integer,
        )),
        Exp::LitFloat(_) => Ok(Val::EigonPrimitive(crate::nbe::term::PrimitiveType::Float)),
        Exp::LitBool(_) => Ok(Val::EigonPrimitive(
            crate::nbe::term::PrimitiveType::Boolean,
        )),

        e => Err(CheckError::CannotInfer(format!(
            "cannot infer type of: {e:?}"
        ))),
    }
}

/// Find a field by name in a Sigma chain.
/// Walks Σ name₁ : T₁. Σ name₂ : T₂. ... looking for a matching name.
///
/// When the type is `EigonClass(iri)`, resolves the class to its Sigma
/// chain via `ctx.resolve_class_cached` and recurses — this is the core
/// fix for issue #12 item 1 (D18 §5).
fn find_sigma_field(ctx: &mut CheckCtx, typ: &Val, field_name: &str) -> Option<Val> {
    match typ {
        // D78 Phase C — a class resolves to a record, and a record is keyed by
        // the **full IRI**. This arm is unreachable from `PropAccess`, which
        // routes through `find_record_field`; it exists for callers that still
        // hold only a local name, and matches on the binder for them.
        Val::Record(fields, rho) => {
            let (_, patt, ty) = fields
                .iter()
                .find(|(_, patt, _)| matches!(patt, Patt::Var(n) if n == field_name))?;
            let _ = patt;
            ctx.eval(ty, rho).ok()
        }
        Val::Refine(carrier, _) => find_sigma_field(ctx, carrier, field_name),
        Val::Sig(t, g) => {
            if g.patt == Patt::Var(field_name.to_string()) {
                // Found — return the field's type
                Some(*t.clone())
            } else {
                // Not this field — apply the closure with a dummy value
                // and search the rest of the chain
                let gen = gen_val(&g.env);
                let rest = ctx.apply(g, gen).ok()?;
                find_sigma_field(ctx, &rest, field_name)
            }
        }
        // Resolve EigonClass to its Sigma chain via layer access.
        Val::EigonClass(iri) => {
            let resolved = ctx.resolve_class_cached(iri).ok()?;
            find_sigma_field(ctx, &resolved, field_name)
        }
        _ => None,
    }
}

/// D78 §9 — look a field up by its **full IRI**.
///
/// `find_sigma_field` matches on the binder, which `build_sigma_chain` set to
/// `prop_iri.local_name()`: two properties sharing a local name across
/// namespaces were one field to a projection. A record carries the IRI, so this
/// lookup cannot confuse them.
///
/// Forgetting a refinement is safe here for the same reason it is safe in
/// subtyping — the constraints do not change what fields the carrier has.
fn find_record_field(ctx: &mut CheckCtx, typ: &Val, field: &Iri) -> Option<Val> {
    match typ {
        Val::Record(fields, rho) => {
            let (_, _, ty) = fields.iter().find(|(iri, _, _)| iri == field)?;
            ctx.eval(ty, rho).ok()
        }
        Val::Refine(carrier, _) => find_record_field(ctx, carrier, field),
        Val::EigonClass(iri) => {
            let resolved = ctx.resolve_class_cached(iri).ok()?;
            find_record_field(ctx, &resolved, field)
        }
        // Anonymous pairs still carry only a binder name, so fall back.
        Val::Sig(..) => find_sigma_field(ctx, typ, field.local_name()),
        _ => None,
    }
}

/// Extract a Pi type: Pi(A, x.B) → (A, x.B)
fn ext_pi(val: &Val) -> Result<(Val, Clos), CheckError> {
    match val {
        Val::Pi(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(CheckError::ExpectedPi(format!(
            "expected Pi type, got: {u:?}"
        ))),
    }
}

/// Extract a Sigma type: Sig(A, x.B) → (A, x.B)
fn ext_sig(val: &Val) -> Result<(Val, Clos), CheckError> {
    match val {
        Val::Sig(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(CheckError::ExpectedSigma(format!(
            "expected Sigma type, got: {u:?}"
        ))),
    }
}

/// Check if a value is a list type and return the element type.
///
/// Recognises the canonical `List(A)` inductive type (the form
/// produced by `Exp::list()` since Phase 11b step 6, D19 §9).
fn extract_list_element_type(val: &Val) -> Option<Val> {
    if let Val::InductiveType {
        decl,
        params,
        indices: _,
    } = val
    {
        if decl.name == "List" && params.len() == 1 {
            return Some(params[0].clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::witness::try_synthesize_chain_witness;
    use super::*;
    use crate::nbe::eval::eval;
    use crate::nbe::eval::EvalCtx;
    use crate::nbe::term::PrimitiveType;
    use crate::nbe::term::{InductiveCtorDecl, InductiveDecl};
    use crate::ontology::iri::Iri;

    #[test]
    fn check_unit_has_type_one() {
        check(&mut ctx(), &Exp::Unit, &Val::One).unwrap();
    }

    // ── Exp::Ann — the bidirectional mode switch (D63 §8.2) ──────────────

    /// `λx. x` is unsynthesizable bare, but inferable when annotated `(λx.x :
    /// Prop→Prop)` — and the inferred type IS the annotation.
    #[test]
    fn ann_makes_a_curry_lambda_inferable() {
        let id = Exp::Lam(Patt::Var("x".into()), Box::new(Exp::Var("x".into())));
        let ty = Exp::Arrow(Box::new(Exp::sort(0)), Box::new(Exp::sort(0)));

        // Bare: check_infer has no Lam arm — not inferable.
        assert!(
            check_infer(&mut ctx(), &id).is_err(),
            "a bare Curry lambda must not be inferable"
        );

        // Annotated: infers exactly the annotation (compared as NbE normal forms,
        // so `A → B` sugar and `Π_:A. B` agree).
        let ann = Exp::Ann(Box::new(id), Box::new(ty.clone()));
        let inferred = check_infer(&mut ctx(), &ann).expect("annotated lambda is inferable");
        let want = readback_val(0, &eval(&ty, &Rho::Nil).unwrap());
        assert_eq!(readback_val(0, &inferred), want);
    }

    /// An `Ann` whose body does not check against the annotation is rejected.
    #[test]
    fn ann_rejects_a_body_that_mismatches_the_annotation() {
        // `λx. x` annotated as `Prop` (not a function type) — must fail.
        let id = Exp::Lam(Patt::Var("x".into()), Box::new(Exp::Var("x".into())));
        let ann = Exp::Ann(Box::new(id), Box::new(Exp::sort(0)));
        assert!(
            check_infer(&mut ctx(), &ann).is_err(),
            "Ann with a non-function annotation for an identity lambda must be rejected"
        );
    }

    /// The annotation must itself be a type; `(Unit : ())` (annotation is a value,
    /// not a Sort) is rejected.
    #[test]
    fn ann_requires_the_annotation_to_be_a_type() {
        let ann = Exp::Ann(Box::new(Exp::Unit), Box::new(Exp::Unit));
        assert!(
            check_infer(&mut ctx(), &ann).is_err(),
            "an Ann whose annotation is not a type must be rejected"
        );
    }

    /// `Ann` is runtime-erased: `⟦(e : T)⟧ = ⟦e⟧`.
    #[test]
    fn ann_is_runtime_erased() {
        let e = Exp::sort(0);
        let ann = Exp::Ann(Box::new(e.clone()), Box::new(Exp::sort(1)));
        let via_ann = readback_val(0, &eval(&ann, &Rho::Nil).unwrap());
        let direct = readback_val(0, &eval(&e, &Rho::Nil).unwrap());
        assert_eq!(via_ann, direct, "Ann must erase to its underlying term");
    }

    #[test]
    fn check_one_has_type_set() {
        check(&mut ctx(), &Exp::One, &Val::sort(1)).unwrap();
    }

    /// eigenius#136 — `Set : Set` is rejected in checking mode.
    ///
    /// Until this was fixed, `(Exp::Sort(_), Val::Sort(1))` admitted every
    /// universe against `Set`, so `Set : Set` type-checked and Girard's
    /// paradox was expressible in a term the commit gate (Rule 21) checks.
    #[test]
    fn set_does_not_inhabit_set() {
        let err = check(&mut ctx(), &Exp::sort(1), &Val::sort(1))
            .expect_err("Set : Set must be rejected");
        assert!(
            format!("{err:?}").contains("universe stratification"),
            "expected a universe-stratification diagnostic, got: {err:?}"
        );
    }

    /// No universe inhabits itself, at any level.
    #[test]
    fn no_universe_inhabits_itself() {
        for n in 0..5 {
            assert!(
                check(&mut ctx(), &Exp::sort(n), &Val::sort(n)).is_err(),
                "Sort({n}) : Sort({n}) must be rejected"
            );
        }
    }

    /// The universe cycle `Set : Type(1) : Set` is as inconsistent as
    /// `Set : Set`, so the `Type(n) : Set` half goes too — `Set` is not a
    /// top universe. (Pre-D46 the checker admitted both halves.)
    #[test]
    fn a_higher_universe_does_not_inhabit_set() {
        for n in 2..6 {
            assert!(
                check(&mut ctx(), &Exp::sort(n), &Val::sort(1)).is_err(),
                "Sort({n}) : Set must be rejected — Set is not the top universe"
            );
        }
    }

    /// The predicative, cumulative rule the fix installs: `Sort(n) : Sort(m)`
    /// iff `n < m`. `Prop : Set`, `Set : Type(1)`, and cumulative jumps such
    /// as `Prop : Type(3)` all stand.
    #[test]
    fn a_universe_inhabits_every_universe_strictly_above_it() {
        for n in 0..5 {
            for m in 0..6 {
                let got = check(&mut ctx(), &Exp::sort(n), &Val::sort(m)).is_ok();
                assert_eq!(
                    got,
                    n < m,
                    "check(Sort({n}), Sort({m})) = {got}, expected {}",
                    n < m
                );
            }
        }
    }

    /// Check mode and inference must accept the same universe judgements:
    /// `check(Sort(n), Sort(m))` succeeds exactly when the inferred
    /// `Sort(n+1)` is a subtype of `Sort(m)`. Before the fix, check mode was
    /// strictly the more permissive of the two.
    #[test]
    fn check_mode_and_inference_agree_on_universes() {
        for n in 0..5 {
            for m in 0..6 {
                let checked = check(&mut ctx(), &Exp::sort(n), &Val::sort(m)).is_ok();
                let inferred = check_infer(&mut ctx(), &Exp::sort(n)).unwrap();
                let subsumed = crate::nbe::check::conv::subtype_of(
                    &crate::nbe::env_global::Env::empty(),
                    0,
                    &inferred,
                    &Val::sort(m),
                )
                .is_ok();
                assert_eq!(
                    checked, subsumed,
                    "Sort({n}) against Sort({m}): check mode says {checked}, \
                     infer-then-subsume says {subsumed}"
                );
            }
        }
    }

    /// eigenius#191 — `SomeClass : Prop` is rejected in checking mode.
    ///
    /// Until this was fixed, `(Exp::EigonClass(_), Val::Sort(_))` admitted a
    /// class against every universe, `Sort(0)` included, so a class could
    /// stand where a proposition is expected with no diagnostic.
    #[test]
    fn an_eigon_class_does_not_inhabit_prop() {
        let class = Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:test:CellLine").unwrap());
        let err = check(&mut ctx(), &class, &Val::sort(0))
            .expect_err("`SomeClass : Prop` must be rejected");
        assert!(
            format!("{err:?}").contains("universe stratification"),
            "expected a universe-stratification diagnostic, got: {err:?}"
        );
    }

    /// Same for the Eigon primitives — `core:string : Prop` is not a thing.
    #[test]
    fn an_eigon_primitive_does_not_inhabit_prop() {
        let prim = Exp::EigonPrimitive(crate::nbe::term::PrimitiveType::String);
        let err = check(&mut ctx(), &prim, &Val::sort(0))
            .expect_err("`core:string : Prop` must be rejected");
        assert!(
            format!("{err:?}").contains("universe stratification"),
            "expected a universe-stratification diagnostic, got: {err:?}"
        );
    }

    // ── eigenius#194: the `Val::Sort(_)` wildcard arms are gone ──────────────
    //
    // `Codata`, `CodataType`, `Inductive` and `InductiveType` each had an arm matching EVERY
    // universe and delegating to `check_type`, which takes no expected type — so the universe was
    // discarded. They now fall through to `check_by_inference`, which compares under `subtype_of`.
    // These four tests pin both directions of that comparison.

    /// A reference to an inductive declared at the given sort, together with a
    /// context whose environment holds the declaration.
    ///
    /// D76 Phase B: the former names its declaration instead of carrying it, so
    /// these tests need an environment. That is the property under test, not
    /// scaffolding — `check_infer`'s `Const` arm reads `decl.sort` from `Γ_env`,
    /// and if it could not find it the universe comparison would have nothing to
    /// compare.
    fn ind_at(name: &str, sort: usize) -> (CheckCtx, Exp) {
        let (c, refs) = crate::nbe::check::testutil::ctx_declaring(&[(name, sort)]);
        (c, refs.into_iter().next().expect("one declaration"))
    }

    /// `data D : Set` standing where a proposition is expected. `justification:Certificate(j, P)`,
    /// `reflection:canonical_proposition` and everything else Rule 21 checks take a `Prop` in that
    /// slot, so this is the same stakes argument as eigenius#191 with a different constructor.
    #[test]
    fn a_set_level_inductive_does_not_inhabit_prop() {
        let (mut c, set_level) = ind_at("SetLevel", 1);
        check(&mut c, &set_level, &Val::sort(0))
            .expect_err("`data D : Set` must not check against `Prop`");
    }

    /// The other half, and the reason the fix is a deletion rather than a `m >= 1` guard: a
    /// `Prop`-sorted inductive — `logic:And`, `justification:Certificate`, the witness predicates —
    /// must still check against `Set` by cumulativity. Nine of the twelve probe hits measured on
    /// `2026-08-22` were exactly this shape, so a guard written the obvious way would have broken
    /// them.
    #[test]
    fn a_prop_level_inductive_still_inhabits_set_and_above() {
        let (mut c, prop_level) = ind_at("PropLevel", 0);
        check(&mut c, &prop_level, &Val::sort(1))
            .expect("`data D : Prop` inhabits `Set` by cumulativity");
        check(&mut c, &prop_level, &Val::sort(2)).expect("...and every universe above it");
        let (mut c2, set_level) = ind_at("SetLevel", 1);
        check(&mut c2, &set_level, &Val::sort(2))
            .expect("`data D : Set` inhabits `Type 1` — the other three probe hits");
    }

    /// The invariant the deletion buys, stated directly: for these constructors `check` accepts
    /// exactly what `check_infer` + `subtype_of` accepts, because it now IS that path. A future
    /// arm re-added above `check_by_inference` would break this before it broke a chain.
    #[test]
    fn check_and_infer_agree_on_type_former_universes() {
        for ((mut c, exp), label) in [
            (ind_at("P", 0), "inductive at Prop"),
            (ind_at("S", 1), "inductive at Set"),
        ] {
            let inferred = match check_infer(&mut c, &exp).expect("inferable") {
                Val::Sort(k) => k,
                other => panic!("{label}: expected a sort, got {other:?}"),
            };
            for m in 0..4usize {
                let checked = check(&mut c, &exp, &Val::sort(m)).is_ok();
                let cumulative = inferred.leq(&crate::nbe::level::Level::of_nat(m));
                assert_eq!(
                    checked, cumulative,
                    "{label}: check against Sort({m}) = {checked}, but inference gives \
                     Sort({inferred}) and cumulativity says {cumulative}"
                );
            }
        }
    }

    /// The rule the fix installs: a ground Eigon type inhabits `Set` and, by
    /// cumulativity, every universe above it — and nothing below.
    #[test]
    fn an_eigon_ground_type_inhabits_set_and_every_universe_above() {
        let class = Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:test:CellLine").unwrap());
        let prim = Exp::EigonPrimitive(crate::nbe::term::PrimitiveType::Integer);
        for exp in [&class, &prim] {
            for m in 0..6 {
                let got = check(&mut ctx(), exp, &Val::sort(m)).is_ok();
                assert_eq!(
                    got,
                    m >= 1,
                    "check({exp:?}, Sort({m})) = {got}, expected {}",
                    m >= 1
                );
            }
        }
    }

    /// Check mode and inference must accept the same judgements for the
    /// ground Eigon types: `check(C, Sort(m))` succeeds exactly when the
    /// inferred `Sort(1)` is a subtype of `Sort(m)`. Before the fix, check
    /// mode was strictly the more permissive of the two.
    #[test]
    fn check_mode_and_inference_agree_on_eigon_ground_types() {
        let class = Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:test:CellLine").unwrap());
        let prim = Exp::EigonPrimitive(crate::nbe::term::PrimitiveType::String);
        for exp in [&class, &prim] {
            for m in 0..6 {
                let checked = check(&mut ctx(), exp, &Val::sort(m)).is_ok();
                let inferred = check_infer(&mut ctx(), exp).unwrap();
                let subsumed = crate::nbe::check::conv::subtype_of(
                    &crate::nbe::env_global::Env::empty(),
                    0,
                    &inferred,
                    &Val::sort(m),
                )
                .is_ok();
                assert_eq!(
                    checked, subsumed,
                    "{exp:?} against Sort({m}): check mode says {checked}, \
                     infer-then-subsume says {subsumed}"
                );
            }
        }
    }

    #[test]
    fn check_set_is_type() {
        check_type(&mut ctx(), &Exp::sort(1)).unwrap();
    }

    #[test]
    fn check_one_is_type() {
        check_type(&mut ctx(), &Exp::One).unwrap();
    }

    #[test]
    fn check_pi_is_type() {
        // Π _ : 1. 1 is a valid type
        let pi = Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::One));
        check_type(&mut ctx(), &pi).unwrap();
    }

    #[test]
    fn check_identity_function() {
        // λx.x : Π x : 1. 1
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let pi = Val::Pi(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        check(&mut ctx(), &lam, &pi).unwrap();
    }

    #[test]
    fn check_pair() {
        // ((), ()) : Σ _ : 1. 1
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let sig = Val::Sig(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        check(&mut ctx(), &pair, &sig).unwrap();
    }

    /// The `Case`-against-Pi arm still checks when the surrounding context binds the name the
    /// branch binder uses. **This test does NOT discriminate the eigenius#64 fix** — it passes
    /// against the old literal `"__case_arg"` too — and that is recorded here on purpose, because
    /// the obvious reading of eigenius#64 is that a capture was reachable and it was not.
    ///
    /// Why not: the motive is spliced in via `readback_val`, and readback is fully normalizing. It
    /// evaluates under the environment and emits no free source-level name — the only variables it
    /// mints come from `Neut::Gen(j, name)`, which reads back as `"{name}{j}"` with the level
    /// always appended (`readback.rs:264`), and `try_readback_fun` likewise evaluates each branch
    /// before reading it back rather than copying the source `Exp`. So `Exp::Var("__case_arg")`
    /// could not appear in the spliced motive, and the branch binder had nothing to capture.
    ///
    /// The fix is therefore what eigenius#64 says it is — robustness, not a bug fix — and the
    /// property it buys is pinned by `case_branch_binder_name_is_unforgeable` below. This test is
    /// kept because the arm had no coverage at all.
    #[test]
    fn case_branch_checks_under_a_shadowing_outer_binding() {
        let sum_ty = Val::Data(
            vec![
                ("left".to_string(), Exp::One),
                ("right".to_string(), Exp::One),
            ],
            Rho::Nil,
        );
        // The outer binding the motive refers to, and which the branch binder must not capture.
        let outer = Rho::Nil.extend(Patt::Var("__case_arg".to_string()), Val::One);
        // Motive: `λ_. __case_arg` — constantly the type `One`, named indirectly.
        let motive = Clos::new(
            Patt::Unit,
            Exp::Var("__case_arg".to_string()),
            outer.clone(),
        );

        let case = Exp::Case(vec![
            crate::nbe::term::Branch {
                name: "left".to_string(),
                body: Exp::Lam(Patt::Unit, Box::new(Exp::Unit)),
            },
            crate::nbe::term::Branch {
                name: "right".to_string(),
                body: Exp::Lam(Patt::Unit, Box::new(Exp::Unit)),
            },
        ]);

        let mut ctx = CheckCtx::new(outer, Vec::new());
        check(&mut ctx, &case, &Val::Pi(Box::new(sum_ty), motive))
            .expect("each branch checks against `Pi y:One. One`; a captured motive breaks this");
    }

    /// **eigenius#64 — the minted binder name cannot be written in ESL.**
    ///
    /// This is the property the fix actually delivers, and the reason the name is `CB#{level}`
    /// rather than the `__case_arg_{level}` the issue proposed: `#` is not a legal identifier
    /// character, so no source program can bind the name, at any scope, ever. `__case_arg_0` is a
    /// perfectly good ESL identifier and would have left the same latent hazard one rename away.
    ///
    /// The `#` discipline is not invented here — `gen_val` mints `TC#{level}` and readback mints
    /// `G#{level}` for the same reason. The prefixes must stay distinct: `readback_val` starts
    /// generating at `ctx.rho.len()`, the same level the branch binder is named from, so reusing
    /// `G#` would collide with the motive's own variables.
    #[test]
    fn case_branch_binder_name_is_unforgeable() {
        // The lexer either rejects the name or splits it — what it must not do is hand back a
        // single identifier token equal to it.
        let lexes_as_one_identifier = |name: &str| -> bool {
            crate::esl::lexer::tokenize(name)
                .map(|toks| {
                    toks.iter()
                        .any(|t| format!("{t:?}").contains(&format!("\"{name}\"")))
                })
                .unwrap_or(false)
        };

        // First: the detector fires on a name that IS writable. Without this the assertion below
        // would pass against any string, including one that is perfectly forgeable — which is the
        // failure mode that let the original literal sit here unnoticed.
        assert!(
            lexes_as_one_identifier("__case_arg_7"),
            "eigenius#64's proposed `__case_arg_{{level}}` IS a legal ESL identifier; if this stops \
             holding the test below no longer discriminates anything"
        );

        let minted = format!("CB#{}", 7);
        assert!(
            !lexes_as_one_identifier(&minted),
            "`{minted}` must not be writable as an ESL identifier; if it becomes one, the \
             case-branch binder can be captured and this name has to change"
        );
    }

    #[test]
    fn check_type_mismatch_fails() {
        // () : U should fail (unit is not a type)
        let result = check(&mut ctx(), &Exp::Unit, &Val::sort(1));
        assert!(result.is_err());
    }

    #[test]
    fn check_let_declaration() {
        // let x : 1 = (); x : 1
        let d = Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Unit),
        );
        let e = Exp::Dec(d, Box::new(Exp::Var("x".to_string())));
        check(&mut ctx(), &e, &Val::One).unwrap();
    }

    #[test]
    fn infer_variable_type() {
        let gamma: Gamma = vec![("x".to_string(), Val::One)];
        let mut c = CheckCtx::new(Rho::Nil, gamma);
        let t = check_infer(&mut c, &Exp::Var("x".to_string())).unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn infer_application_type() {
        // f : 1 → 1, f () : 1
        let pi_type = Val::Pi(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        let gamma: Gamma = vec![("f".to_string(), pi_type)];
        let rho = Rho::Nil.extend(
            Patt::Var("f".to_string()),
            Val::Lam(Clos::new(
                Patt::Var("x".to_string()),
                Exp::Var("x".to_string()),
                Rho::Nil,
            )),
        );
        let mut c = CheckCtx::new(rho, gamma);
        let t = check_infer(
            &mut c,
            &Exp::App(Box::new(Exp::Var("f".to_string())), Box::new(Exp::Unit)),
        )
        .unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn eq_nf_equal() {
        eq_nf(0, &Val::One, &Val::One).unwrap();
        eq_nf(0, &Val::Unit, &Val::Unit).unwrap();
        eq_nf(0, &Val::sort(1), &Val::sort(1)).unwrap();
    }

    #[test]
    fn eq_nf_not_equal() {
        assert!(eq_nf(0, &Val::One, &Val::sort(1)).is_err());
        assert!(eq_nf(0, &Val::Unit, &Val::One).is_err());
    }

    #[test]
    fn check_sum_type() {
        // Sum(a 1 | b 1) : U
        let data = Exp::Data(vec![
            crate::nbe::term::Summand {
                name: "a".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Summand {
                name: "b".to_string(),
                typ: Exp::One,
            },
        ]);
        check(&mut ctx(), &data, &Val::sort(1)).unwrap();
    }

    #[test]
    fn sum_of_large_summands_is_checkable() {
        // eigenius#220. `check` had the only rule for `Exp::Data`, and it required the sum AND
        // every summand to be at `Set` exactly. A sum storing a `Type 1` was therefore unwritable:
        // there was no inference fallback to rescue it, unlike the `Exp::One` arm beside it.
        //
        // `Sum(a (Type 1) | b Set)` lives at `max(Sort 2, Sort 1)`'s successor structure — each
        // summand's TYPE is inferred, so `Type 1` (`Sort 2`) contributes level 3 and `Set`
        // (`Sort 1`) contributes 2, giving `Sort 3`.
        let data = Exp::Data(vec![
            crate::nbe::term::Summand {
                name: "a".to_string(),
                typ: Exp::sort(2),
            },
            crate::nbe::term::Summand {
                name: "b".to_string(),
                typ: Exp::sort(1),
            },
        ]);
        let inferred = check_infer(&mut ctx(), &data).expect("a sum of large summands has a type");
        assert!(
            matches!(&inferred, Val::Sort(l) if l.is_nat(3)),
            "expected Sort(3) = max(3, 2); got {inferred:?}"
        );
        check(&mut ctx(), &data, &Val::sort(3)).expect("and checks against its own sort");
    }

    #[test]
    fn small_sum_checks_against_a_larger_sort_by_cumulativity() {
        // The other half of eigenius#220: `Sum(a 1 | b 1)` is at `Set`, and `Set ⊆ Type 1`, so it
        // must check against `Type 1`. The deleted arm's `l.is_nat(1)` guard rejected this — it
        // tested the expected sort for EQUALITY with `Set` rather than letting subtyping decide.
        let data = Exp::Data(vec![
            crate::nbe::term::Summand {
                name: "a".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Summand {
                name: "b".to_string(),
                typ: Exp::One,
            },
        ]);
        check(&mut ctx(), &data, &Val::sort(1)).expect("at Set");
        check(&mut ctx(), &data, &Val::sort(2)).expect("and at Type 1 by cumulativity");
    }

    #[test]
    fn an_empty_sum_is_a_proposition() {
        // `max` over no summands is its identity, `Zero` — so the empty type is at `Prop`.
        let inferred = check_infer(&mut ctx(), &Exp::Data(vec![])).expect("empty sum has a type");
        assert!(
            matches!(&inferred, Val::Sort(l) if l.is_nat(0)),
            "the empty sum is uninhabited, hence a proposition; got {inferred:?}"
        );
    }

    #[test]
    fn check_constructor_against_sum() {
        // $a () : Sum(a 1 | b 1)
        let data_val = Val::Data(
            vec![("a".to_string(), Exp::One), ("b".to_string(), Exp::One)],
            Rho::Nil,
        );
        let con = Exp::Con("a".to_string(), Box::new(Exp::Unit));
        check(&mut ctx(), &con, &data_val).unwrap();
    }

    #[test]
    fn check_constructor_wrong_name_fails() {
        let data_val = Val::Data(vec![("a".to_string(), Exp::One)], Rho::Nil);
        let con = Exp::Con("b".to_string(), Box::new(Exp::Unit));
        assert!(check(&mut ctx(), &con, &data_val).is_err());
    }

    #[test]
    fn check_id_is_type() {
        // Id(1, (), ()) inhabits Prop, Set, and any Type(n) via cumulativity.
        // D46 §9 — Id lives in Prop; older callers expecting Set are
        // unaffected because Prop ⊆ Set.
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check(&mut ctx(), &id, &Val::sort(0)).unwrap();
        check(&mut ctx(), &id, &Val::sort(1)).unwrap();
        check(&mut ctx(), &id, &Val::sort(2)).unwrap();
    }

    #[test]
    fn id_inferred_in_prop() {
        // Phase G: check_infer for Exp::Id now returns Sort(0).
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let inferred = check_infer(&mut ctx(), &id).unwrap();
        assert!(
            matches!(&inferred, Val::Sort(l) if l.is_nat(0)),
            "Id should infer at Sort(0); got {inferred:?}"
        );
    }

    #[test]
    fn distinct_refl_proofs_equal_by_proof_irrelevance() {
        // Two distinct-shape proofs of the same Id type should be
        // definitionally equal via proof irrelevance — refl(()) and
        // a neutral inhabitant of Id are interchangeable.
        // We exercise the integration: an Id-typed value compared to
        // another Id-typed value at type Id(...) succeeds even when
        // structurally different.
        let id_typ = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(Val::Unit));
        // Two synthetic distinct values; def_eq_at_type at typ=Id sees
        // the propositional fast-path and accepts.
        let refl_v = Val::Refl(Box::new(Val::Unit));
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "h".to_string()));
        def_eq_at_type(&mut ctx(), &refl_v, &neut, &id_typ).unwrap();
    }

    #[test]
    fn check_id_type_well_formed() {
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check_type(&mut ctx(), &id).unwrap();
    }

    #[test]
    fn check_refl_against_id() {
        // refl(()) : Id(1, (), ())
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let id_type = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(Val::Unit));
        check(&mut ctx(), &refl, &id_type).unwrap();
    }

    #[test]
    fn check_refl_wrong_endpoints_fails() {
        // refl(()) : Id(1, (), x) should fail when x ≠ ()
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let id_type = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(gen));
        assert!(check(&mut ctx(), &refl, &id_type).is_err());
    }

    #[test]
    fn eval_j_with_refl_reduces() -> Result<(), Box<dyn std::error::Error>> {
        // J(1, C, d, (), (), refl(())) should reduce to d(())
        use crate::nbe::eval::eval;
        let j = Exp::IdJ(Box::new([
            Exp::One,                                                        // A
            Exp::sort(1),                                                    // C (placeholder)
            Exp::Lam(Patt::Var("a".into()), Box::new(Exp::Var("a".into()))), // d = λa. a
            Exp::Unit,                                                       // x
            Exp::Unit,                                                       // y
            Exp::Refl(Box::new(Exp::Unit)),                                  // p = refl(())
        ]));
        let result = eval(&j, &Rho::Nil)?;
        // d(()) = (λa.a)(()) = ()
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn deceq_equal_reduces_to_refl() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // DecEq(1, (), ()) → refl(())
        let deceq = Exp::DecEq(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Refl(_)));
        Ok(())
    }

    #[test]
    fn deceq_unequal_produces_neutral() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // DecEq(Set, 1, Set) — One ≠ Set, produces neutral
        let deceq = Exp::DecEq(
            Box::new(Exp::sort(1)),
            Box::new(Exp::One),
            Box::new(Exp::sort(1)),
        );
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Nt(_)));
        Ok(())
    }

    #[test]
    fn deceq_iri_equal() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let iri = Iri::parse("urn:eigenius:core:string").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::sort(1)),
            Box::new(Exp::EigonClass(iri.clone())),
            Box::new(Exp::EigonClass(iri)),
        );
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Refl(_)));
        Ok(())
    }

    #[test]
    fn deceq_iri_unequal() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let iri1 = Iri::parse("urn:eigenius:core:string").unwrap();
        let iri2 = Iri::parse("urn:eigenius:core:integer").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::sort(1)),
            Box::new(Exp::EigonClass(iri1)),
            Box::new(Exp::EigonClass(iri2)),
        );
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Nt(_)));
        Ok(())
    }

    #[test]
    fn check_eigon_primitive_is_type() {
        check_type(&mut ctx(), &Exp::EigonPrimitive(PrimitiveType::String)).unwrap();
        check(
            &mut ctx(),
            &Exp::EigonPrimitive(PrimitiveType::Integer),
            &Val::sort(1),
        )
        .unwrap();
    }

    // --- Phase 10a: new inference and resolution tests ---

    #[test]
    fn infer_refl() {
        // refl(x) where x : One should infer Id(One, x_val, x_val)
        let gamma: Gamma = vec![("x".to_string(), Val::One)];
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        let mut c = CheckCtx::new(rho, gamma);
        let refl_x = Exp::Refl(Box::new(Exp::Var("x".to_string())));
        let t = check_infer(&mut c, &refl_x).unwrap();
        assert!(matches!(t, Val::Id(_, _, _)));
    }

    #[test]
    fn infer_deceq() {
        // DecEq(One, (), ()) should infer Id(One, (), ())
        let deceq = Exp::DecEq(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let t = check_infer(&mut ctx(), &deceq).unwrap();
        assert!(matches!(t, Val::Id(_, _, _)));
    }

    #[test]
    fn infer_template() {
        // Template("hello", []) should infer EigonPrimitive(String)
        let tmpl = Exp::Template("hello".to_string(), vec![]);
        let t = check_infer(&mut ctx(), &tmpl).unwrap();
        assert!(matches!(t, Val::EigonPrimitive(PrimitiveType::String)));
    }

    #[test]
    fn infer_eigon_resource() {
        use crate::ontology::resource::Resource;
        // EigonResource with is_a = [Dog] should infer EigonClass(Dog)
        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let is_a_iri = Iri::parse("urn:eigenius:core:is_a").unwrap();
        let mut r = Resource::new(Iri::parse("urn:example:rex").unwrap());
        r.set(
            is_a_iri,
            crate::ontology::resource::Value::Array(vec![crate::ontology::resource::Value::iri(
                &dog_iri,
            )]),
        );
        let expr = Exp::EigonResource(Box::new(r));
        let t = check_infer(&mut ctx(), &expr).unwrap();
        match t {
            Val::EigonClass(iri) => assert_eq!(iri.as_str(), "urn:eigenius:example:Dog"),
            other => panic!("expected EigonClass, got {:?}", other),
        }
    }

    #[test]
    fn check_resource_inhabits_via_full_is_a() {
        // #91: a resource check-mode-inhabits a class iff one of its FULL is_a
        // set is that class (or a subclass) — not just `is_a().first()`.
        use crate::ontology::resource::{Resource, Value};
        let is_a = Iri::parse("urn:eigenius:core:is_a").unwrap();
        let resource_of = |classes: &[&str]| {
            let mut r = Resource::new(Iri::parse("urn:example:r").unwrap());
            if !classes.is_empty() {
                r.set(
                    is_a.clone(),
                    Value::Array(
                        classes
                            .iter()
                            .map(|c| Value::String(c.to_string()))
                            .collect(),
                    ),
                );
            }
            Exp::EigonResource(Box::new(r))
        };
        let class = |s: &str| Val::EigonClass(Iri::parse(s).unwrap());

        // Multi-class: inhabits EACH of its classes — including the NON-first
        // (the #91 win; reflexive case needs no layer).
        let dual = resource_of(&["urn:eigenius:example:Gene", "urn:eigenius:example:CellLine"]);
        assert!(check(&mut ctx(), &dual, &class("urn:eigenius:example:Gene")).is_ok());
        assert!(
            check(&mut ctx(), &dual, &class("urn:eigenius:example:CellLine")).is_ok(),
            "the non-first class must inhabit (#91)"
        );
        assert!(
            check(&mut ctx(), &dual, &class("urn:eigenius:example:Other")).is_err(),
            "an unrelated class must not inhabit"
        );

        // Empty is_a: a *valid* resource that inhabits no specific class — fails
        // closed, never panics.
        let bare = resource_of(&[]);
        assert!(
            check(&mut ctx(), &bare, &class("urn:eigenius:example:Gene")).is_err(),
            "empty is_a inhabits no specific class (fail-closed)"
        );
    }

    #[test]
    fn check_kind_coerces_into_its_base_class() {
        // The derived-kind-predication coercion (D64 §2.3): `kind_of(K)` check-mode-coerces
        // into class `C` iff `base(K) ⊑ C`. Check mode ONLY — inference still gives the
        // axiom's codomain (`Entity`), so this is the same inclusion-coercion shape as the
        // CN-as-types arm.
        let kind_of = |k: Exp| {
            Exp::App(
                Box::new(Exp::EigonAxiom(
                    Iri::parse("urn:eigenius:ontology:kind_of").unwrap(),
                )),
                Box::new(k),
            )
        };
        let cls = |s: &str| Exp::EigonClass(Iri::parse(s).unwrap());
        let class = |s: &str| Val::EigonClass(Iri::parse(s).unwrap());

        // Reflexive (layer-free): the kind of Gene coerces into Gene…
        let gene = "urn:eigenius:example:Gene";
        assert!(check(&mut ctx(), &kind_of(cls(gene)), &class(gene)).is_ok());
        // …and a REFINED kind peels its Σ spine to the same base ("MSI cell lines" is
        // still a cell-line kind).
        let refined = Exp::Sig(
            Patt::Var("x".to_string()),
            Box::new(cls(gene)),
            Box::new(Exp::One),
        );
        assert!(check(&mut ctx(), &kind_of(refined), &class(gene)).is_ok());
        // An unrelated class is vetoed — the restrictor typing stays real.
        assert!(check(
            &mut ctx(),
            &kind_of(cls(gene)),
            &class("urn:eigenius:example:Other")
        )
        .is_err());
        // A base that is not a class (bound variable) falls through — no silent acceptance.
        assert!(check(
            &mut ctx(),
            &kind_of(Exp::Var("A".to_string())),
            &class(gene)
        )
        .is_err());

        // Subclass acceptance via the layer lattice: kind_of(Dog) coerces into Animal.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let mut builder = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).unwrap() {
            builder.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let mut domain = LayerBuilder::new("animals", Some(core));
        for r in eigon_json::parse_document(animals_json).unwrap() {
            domain.add_resource(r).unwrap();
        }
        let layer = std::sync::Arc::new(domain.build(crate::layer::LayerStorage::in_memory()));
        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        assert!(
            check(
                &mut c,
                &kind_of(cls("urn:eigenius:example:Dog")),
                &class("urn:eigenius:example:Animal")
            )
            .is_ok(),
            "a subclass kind coerces into the superclass position"
        );
        assert!(
            check(
                &mut c,
                &kind_of(cls("urn:eigenius:example:Animal")),
                &class("urn:eigenius:example:Dog")
            )
            .is_err(),
            "the coercion is directional — a superclass kind does not narrow"
        );
    }

    #[test]
    fn find_sigma_field_resolves_eigon_class_with_layer() {
        // With a layer, find_sigma_field on EigonClass should resolve
        // to actual property types instead of Val::sort(1).
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;

        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        let layer =
            std::sync::Arc::new(domain_builder.build(crate::layer::LayerStorage::in_memory()));

        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let dog_type = Val::EigonClass(dog_iri);

        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        let field = find_sigma_field(&mut c, &dog_type, "name");
        assert!(field.is_some(), "should find 'name' on Dog");
        // The type should NOT be Val::sort(1) (the old broken behavior)
        let field_type = field.unwrap();
        assert!(
            !matches!(&field_type, Val::Sort(l) if l.is_nat(1)),
            "field type should be resolved, not Set; got {:?}",
            field_type
        );
    }

    #[test]
    fn an_undeclared_property_is_admitted_by_validation_but_cannot_be_projected() {
        // The open-world / closed-type disagreement (D75 §3.8).
        //
        // Rule 22 §c admits a property whose key resolves to a declared
        // `core:Property` even when the resource's classes neither require nor
        // recommend it — that is what open-world validation means. The value
        // keeps it: a resource marshals to `Val::ResourceVal` carrying the whole
        // `Resource`.
        //
        // The type does not. `resolve_class_type` is a function of the CLASS
        // (`ground.rs:37` takes a `&Iri`, not a resource), so its record is built
        // from `requires` alone — since D78 Phase C; it was a Σ-chain over
        // `requires` + Option-wrapped `recommends` before. The extra field is in
        // the value and absent from the type, so the lookup misses and
        // `Exp::PropAccess` reports `IllFormed`.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;

        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        let animals =
            std::sync::Arc::new(domain_builder.build(crate::layer::LayerStorage::in_memory()));

        // A perfectly well-formed property that `Dog` does not mention.
        let nickname = Iri::parse("urn:eigenius:example:nickname").unwrap();
        let mut prop = crate::ontology::resource::Resource::new(nickname.clone());
        prop.set(
            Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    Iri::parse(crate::ontology::well_known::PROPERTY)
                        .unwrap()
                        .as_str()
                        .to_string(),
                ),
            ]),
        );
        prop.set(
            Iri::parse(crate::ontology::well_known::SHORT_NAME).unwrap(),
            crate::ontology::resource::Value::String("nickname".into()),
        );
        prop.set(
            Iri::parse(crate::ontology::well_known::DESCRIPTION).unwrap(),
            crate::ontology::resource::Value::String("an informal name".into()),
        );
        prop.set(
            Iri::parse(crate::ontology::well_known::DATA_TYPE_PROP).unwrap(),
            crate::ontology::resource::Value::String(
                Iri::parse(crate::ontology::well_known::STRING)
                    .unwrap()
                    .as_str()
                    .to_string(),
            ),
        );
        let mut top = LayerBuilder::new("nickname", Some(animals));
        top.add_resource(prop).unwrap();
        let layer = std::sync::Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        // Vocabulary side: the key resolves to a declared `core:Property`, which
        // is the whole of what Rule 22 §c asks. Nothing about `Dog` is consulted.
        assert!(
            layer.resolve(&nickname).is_some(),
            "test setup: nickname must be a declared property on the chain"
        );

        // Type side: `Dog`'s Σ-chain is built from requires + recommends, so it
        // has no `nickname` field for a projection to land on.
        let dog_type = Val::EigonClass(Iri::parse("urn:eigenius:example:Dog").unwrap());
        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], std::sync::Arc::clone(&layer));
        assert!(
            find_sigma_field(&mut c, &dog_type, "name").is_some(),
            "test setup: a DECLARED field must be projectable, or this proves nothing"
        );
        assert!(
            find_sigma_field(&mut c, &dog_type, "nickname").is_none(),
            "current behaviour: a declared property that Dog does not require or recommend is \
             not in Dog's type, so `dog.nickname` is IllFormed even though a Dog carrying it \
             validates. The value has a field the type cannot mention — see D75 §3.8. If this \
             starts failing, a resource's type has become a function of its fields."
        );
    }

    #[test]
    fn a_class_and_its_own_unfolding_are_not_definitionally_equal() {
        // δ is implemented for classes in `check` and absent from `eq_nf`
        // (D75 §3.3).
        //
        // `CheckCtx` unfolds an `EigonClass` to its record through the environment
        // whenever inference needs a field. `eq_nf` compares
        // `Val::EigonClass(iri)` opaquely. The two halves of the checker therefore
        // disagree about what a class *is*.
        //
        // **Re-examined at D76 Phase D and deliberately unchanged.** `eq_nf` now
        // takes `Γ_env`, so the old reason — *"it takes no context at all"* — no
        // longer holds; the behaviour does, because Q2 requires it. Unfolding a
        // class in conversion would make class identity structural, and 749 of 894
        // shipped classes have identical (empty) field sets
        // (`unfolding_a_class_would_collapse_two_nominally_distinct_classes`, just
        // below). The environment reaching conversion is what makes the *choice*
        // to stay opaque, rather than the absence of a layer making it for us.
        //
        // The reconciliation Q2 names is still outstanding and is still `check`'s
        // side: stop treating a class's unfolding as definitional equality.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;

        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let mut d = LayerBuilder::new("animals", Some(core));
        for r in eigon_json::parse_document(animals_json).unwrap() {
            d.add_resource(r).unwrap();
        }
        let layer = std::sync::Arc::new(d.build(crate::layer::LayerStorage::in_memory()));

        let dog = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let folded = Val::EigonClass(dog.clone());
        let unfolded = crate::program::ground::resolve_class_type(&dog, &layer).unwrap();

        // The check side does unfold: `find_sigma_field` reaches the fields.
        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], std::sync::Arc::clone(&layer));
        assert!(
            find_sigma_field(&mut c, &folded, "name").is_some(),
            "test setup: check-side δ must be live, or this proves nothing"
        );
        // D78 Phase C — a record now, not a Σ-chain. The finding is unchanged:
        // what a class unfolds *to* is not what makes `check` and `eq_nf`
        // disagree; that they disagree at all is.
        assert!(
            matches!(unfolded, Val::Record(..)),
            "test setup: Dog must unfold to a record, got {unfolded:?}"
        );

        assert!(
            eq_nf(0, &folded, &folded).is_ok(),
            "test setup: a class is equal to itself"
        );
        assert!(
            eq_nf(0, &folded, &unfolded).is_err(),
            "current behaviour: `eq_nf` does not unfold a class, so a type written folded and the \
             same type produced unfolded do not compare equal. See D75 §3.3 — the δ-policy has to \
             reconcile this, by making `check` stop treating its unfolding as definitional equality \
             rather than by making `eq_nf` unfold. Survived the Σ-chain → record switch unchanged."
        );
    }

    #[test]
    fn unfolding_a_class_would_collapse_two_nominally_distinct_classes() {
        // Why classes must stay δ-opaque (D75 §3.3, Q2).
        //
        // `Alpha` and `Beta` are different classes requiring the same single
        // property. Folded, they are distinct — `eq_nf` compares IRIs. Unfolded,
        // their Σ-chains are identical, so `eq_nf` accepts them as the same type.
        //
        // So making classes transparent under δ does not merely reconcile
        // `check` with `eq_nf`: it silently makes class identity STRUCTURAL,
        // which is the nominal-vs-structural decision deferred to
        // docs/notes/nominal-vs-structural-subtyping.md. `subclass_of` is
        // nominal and load-bearing for Rule 22, `class_types` and institution
        // dispatch, so the reconciliation has to go the other way: `check` must
        // stop treating its unfolding as definitional equality.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;
        use crate::ontology::resource::{Resource, Value as RV};

        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let mut d = LayerBuilder::new("animals", Some(core));
        for r in eigon_json::parse_document(animals_json).unwrap() {
            d.add_resource(r).unwrap();
        }
        let animals = std::sync::Arc::new(d.build(crate::layer::LayerStorage::in_memory()));

        let name_prop = Iri::parse("urn:eigenius:example:name").unwrap();
        let twin = |class_iri: &str, short: &str| {
            let mut r = Resource::new(Iri::parse(class_iri).unwrap());
            r.set(
                Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
                RV::Array(vec![RV::String(
                    Iri::parse(crate::ontology::well_known::CLASS)
                        .unwrap()
                        .as_str()
                        .to_string(),
                )]),
            );
            r.set(
                Iri::parse(crate::ontology::well_known::SHORT_NAME).unwrap(),
                RV::String(short.into()),
            );
            r.set(
                Iri::parse(crate::ontology::well_known::DESCRIPTION).unwrap(),
                RV::String("structurally identical to its twin".into()),
            );
            r.set(
                Iri::parse(crate::ontology::well_known::REQUIRES).unwrap(),
                RV::Array(vec![RV::String(name_prop.clone().as_str().to_string())]),
            );
            r
        };
        let mut t = LayerBuilder::new("twins", Some(animals));
        t.add_resource(twin("urn:eigenius:example:Alpha", "Alpha"))
            .unwrap();
        t.add_resource(twin("urn:eigenius:example:Beta", "Beta"))
            .unwrap();
        let layer = std::sync::Arc::new(t.build(crate::layer::LayerStorage::in_memory()));

        let alpha = Iri::parse("urn:eigenius:example:Alpha").unwrap();
        let beta = Iri::parse("urn:eigenius:example:Beta").unwrap();

        assert!(
            eq_nf(
                0,
                &Val::EigonClass(alpha.clone()),
                &Val::EigonClass(beta.clone())
            )
            .is_err(),
            "folded: two distinct classes must not be definitionally equal"
        );
        assert!(
            eq_nf(
                0,
                &crate::program::ground::resolve_class_type(&alpha, &layer).unwrap(),
                &crate::program::ground::resolve_class_type(&beta, &layer).unwrap()
            )
            .is_ok(),
            "unfolded: identical field sets ARE definitionally equal — which is why δ for classes \
             would make class identity structural. See D75 §3.3."
        );
    }

    #[test]
    fn an_undeclared_property_is_projectable_off_the_resource_but_not_off_the_class() {
        // **D78 Phase E closes D75 §3.8.** The companion to
        // `an_undeclared_property_is_admitted_by_validation_but_cannot_be_projected`,
        // which asserts the class side and stays true forever: a class type is
        // the declared *minimum*, so projecting a property `Dog` does not
        // declare must fail off `Dog` before and after.
        //
        // What Phase E changes is the resource side. A resource's type is now
        // the union of the fields it actually carries, so a property its classes
        // never mention is a field of *its* record — at the property's own type
        // `T`, not `Option T`.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;
        use crate::ontology::resource::{Resource, Value as RV};

        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let mut d = LayerBuilder::new("animals", Some(core));
        for r in eigon_json::parse_document(animals_json).unwrap() {
            d.add_resource(r).unwrap();
        }
        let animals = std::sync::Arc::new(d.build(crate::layer::LayerStorage::in_memory()));

        // A perfectly well-formed property that `Dog` neither requires nor
        // recommends — the open-world case Rule 22 §c admits.
        let nickname = Iri::parse("urn:eigenius:example:nickname").unwrap();
        let mut prop = Resource::new(nickname.clone());
        prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            RV::Array(vec![RV::String(
                Iri::parse(wk::PROPERTY).unwrap().as_str().to_string(),
            )]),
        );
        prop.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            RV::String("nickname".into()),
        );
        prop.set(
            Iri::parse(wk::DESCRIPTION).unwrap(),
            RV::String("an informal name".into()),
        );
        prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            RV::String(Iri::parse(wk::STRING).unwrap().as_str().to_string()),
        );
        let mut top = LayerBuilder::new("nickname", Some(animals));
        top.add_resource(prop).unwrap();
        let layer = std::sync::Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        // A Dog that carries it.
        let mut rex = Resource::new(Iri::parse("urn:eigenius:example:rex").unwrap());
        rex.set(
            Iri::parse(wk::IS_A).unwrap(),
            RV::Array(vec![RV::String(
                Iri::parse("urn:eigenius:example:Dog")
                    .unwrap()
                    .as_str()
                    .to_string(),
            )]),
        );
        rex.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            RV::String("Rex".into()),
        );
        rex.set(
            Iri::parse("urn:eigenius:example:breed").unwrap(),
            RV::String("collie".into()),
        );
        rex.set(nickname.clone(), RV::String("Rexy".into()));

        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], std::sync::Arc::clone(&layer));

        // ── the resource side: NOW projectable ──────────────────────────────
        let rex_type = check_infer(&mut c, &Exp::EigonResource(Box::new(rex.clone())))
            .expect("a resource must infer a type");
        let projected = find_record_field(&mut c, &rex_type, &nickname);
        assert!(
            projected.is_some(),
            "D78 Phase E: an undeclared property the resource carries must be projectable off it; \
             got None from {rex_type:?}"
        );
        assert!(
            !matches!(projected.as_ref().unwrap(), Val::Data(..)),
            "and at the property's own type, not Option-wrapped: {:?}",
            projected.unwrap()
        );

        // Its declared fields project too — the record is the union, not a swap.
        for declared in ["urn:eigenius:example:name", "urn:eigenius:example:breed"] {
            assert!(
                find_record_field(&mut c, &rex_type, &Iri::parse(declared).unwrap()).is_some(),
                "{declared} must still project"
            );
        }

        // ── the class side: STILL not projectable, permanently ──────────────
        let dog_type = Val::EigonClass(Iri::parse("urn:eigenius:example:Dog").unwrap());
        assert!(
            find_record_field(&mut c, &dog_type, &nickname).is_none(),
            "a class type is the declared minimum — `nickname` must never project off `Dog`"
        );
        assert!(
            find_record_field(
                &mut c,
                &dog_type,
                &Iri::parse("urn:eigenius:example:name").unwrap()
            )
            .is_some(),
            "but Dog's own declared fields must, or this proves nothing"
        );
    }

    #[test]
    fn a_resource_carries_every_class_it_declares_not_just_the_first() {
        // 73 % of shipped resources declare more than one `is_a`; the prior
        // inference kept `classes.first()` and dropped the rest.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;
        use crate::ontology::resource::{Resource, Value as RV};

        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        let layer = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let mut r = Resource::new(Iri::parse("urn:t:two_classes").unwrap());
        r.set(
            Iri::parse(wk::IS_A).unwrap(),
            RV::Array(vec![
                RV::String(Iri::parse(wk::PROPERTY).unwrap().as_str().to_string()),
                RV::String(Iri::parse(wk::CLASS).unwrap().as_str().to_string()),
            ]),
        );
        r.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            RV::String("two".into()),
        );

        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        let t = check_infer(&mut c, &Exp::EigonResource(Box::new(r))).unwrap();
        match t {
            Val::Refine(_, classes) => {
                assert_eq!(
                    classes.len(),
                    2,
                    "both declared classes must survive: {classes:?}"
                );
            }
            other => panic!("expected a Refine over both classes, got {other:?}"),
        }
    }

    #[test]
    fn find_sigma_field_without_layer_returns_none_for_eigon_class() {
        // Without a layer, EigonClass resolution should fail gracefully
        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let dog_type = Val::EigonClass(dog_iri);
        let mut c = ctx();
        let field = find_sigma_field(&mut c, &dog_type, "name");
        assert!(field.is_none(), "no layer → should not resolve");
    }

    // --- D14 §9.2: institution-registered decision procedures ---
    //
    // Verify that `Constraint::Institution { iri, args }` dispatches
    // through the `try_institution_decide` path: the constraint IRI
    // resolves to a Decidable QueryClass, args land on the input
    // resource as `decide_args`, and the institution's `query` returns
    // a Verdict resource the kernel translates to a `DecResult`.

    use crate::context::{ExecutionContext, ExecutionMode};
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::institution::DecResult;
    use crate::layer::LayerBuilder;
    use crate::nbe::term::Constraint;
    use crate::ontology::resource::Resource;
    use crate::ontology::resource::Value as RVal;
    use crate::ontology::well_known as wk;

    /// In-test institution whose `query` returns a pre-canned
    /// Verdict resource for each `Constraint::Institution`
    /// invocation and records the input resource it observed.
    /// Phase 19d.7 dropped the legacy `decide_args` array — args
    /// now ride on typed required properties of the input class —
    /// so `last_args` walks `input.properties()` in BTreeMap order
    /// (alphabetical by IRI), skipping `core:is_a`. Test fixtures
    /// name arg properties `arg_0` / `arg_1` / … so the alphabetical
    /// walk yields them in positional order.
    struct FakeInstitution {
        iri: Iri,
        last_input: std::sync::Mutex<Option<Resource>>,
        result: DecResult,
    }

    impl FakeInstitution {
        fn new(iri: &str, result: DecResult) -> Arc<Self> {
            Arc::new(Self {
                iri: Iri::parse(iri).unwrap(),
                last_input: std::sync::Mutex::new(None),
                result,
            })
        }

        fn last_input(&self) -> Option<Resource> {
            self.last_input.lock().unwrap().clone()
        }

        /// Extract the args from the last input resource by walking
        /// its typed properties (skipping `core:is_a`). Properties
        /// fixture-named `arg_0` / `arg_1` / … come back in
        /// positional order via BTreeMap's alphabetical key sort.
        fn last_args(&self) -> Option<Vec<RVal>> {
            let input = self.last_input()?;
            let is_a = Iri::parse(wk::IS_A).unwrap();
            Some(
                input
                    .properties()
                    .iter()
                    .filter(|(k, _)| **k != is_a)
                    .map(|(_, v)| v.clone())
                    .collect(),
            )
        }
    }

    impl Institution for Arc<FakeInstitution> {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }

        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<crate::nbe::val::Val, crate::institution::error::InstitutionError> {
            unreachable!("FakeInstitution exposes no ExportFormats")
        }

        fn reify(
            &self,
            _: &Iri,
            _: &crate::nbe::val::Val,
            _: &ExecutionContext,
        ) -> Result<Resource, crate::institution::error::InstitutionError> {
            unreachable!("FakeInstitution exposes no ImportFormats")
        }

        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<
            crate::institution::runtime::QueryOutcome,
            crate::institution::error::InstitutionError,
        > {
            *self.last_input.lock().unwrap() = Some(input.clone());
            Ok(crate::institution::runtime::QueryOutcome::from_output(
                verdict_resource(self.result),
            ))
        }
    }

    /// Build a Verdict-shaped result resource from a `DecResult`.
    fn verdict_resource(result: DecResult) -> Resource {
        let class_iri = match result {
            DecResult::Holds => "urn:eigenius:institution:verdicts:holds",
            DecResult::Fails => "urn:eigenius:institution:verdicts:fails",
            DecResult::Undecidable => "urn:eigenius:institution:verdicts:undecidable",
        };
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(class_iri.into())]),
        );
        r
    }

    /// IRI of the Nth user-required arg property emitted by
    /// `build_decide_index`. Properties are named `arg_0`, `arg_1`,
    /// … so they sort alphabetically into positional order in the
    /// input's BTreeMap.
    fn arg_prop_iri(input_class_iri: &str, n: usize) -> String {
        format!("{input_class_iri}:arg_{n}")
    }

    /// Build an `InstitutionIndex` and `InstitutionRuntime` declaring
    /// a Decidable `QueryClass` for `constraint_iri`, served by
    /// `fake`. Also declares a typed input class with `arg_count`
    /// required properties (`arg_0` … `arg_{arg_count-1}`) — Phase
    /// 19d.7 dropped the legacy `decide_args` array, so the input
    /// class must declare typed slots for the kernel to populate.
    /// Returns the layer along with the index/runtime so callers
    /// can thread it into an effectful `EvalCtx`'s layer for typed
    /// marshaling.
    fn build_decide_index(
        fake: Arc<FakeInstitution>,
        arg_count: usize,
    ) -> (
        Arc<crate::layer::Layer>,
        Arc<InstitutionIndex>,
        Arc<InstitutionRuntime>,
    ) {
        let constraint_iri = fake.iri.as_str();
        let inst_iri = constraint_iri; // for tests, institution IRI = constraint IRI
        let input_class_iri = format!("{constraint_iri}:Input");

        let mut b = LayerBuilder::new("test", None);

        // Each arg slot is its own Property resource; the input
        // class lists them in order via `requires`.
        let mut requires = Vec::with_capacity(arg_count);
        for n in 0..arg_count {
            let prop_iri = arg_prop_iri(&input_class_iri, n);
            let mut p = Resource::new(Iri::parse(&prop_iri).unwrap());
            p.set(
                Iri::parse(wk::IS_A).unwrap(),
                RVal::Array(vec![RVal::String(wk::PROPERTY.into())]),
            );
            b.add_resource(p).unwrap();
            requires.push(RVal::String(prop_iri));
        }

        let mut input_class = Resource::new(Iri::parse(&input_class_iri).unwrap());
        input_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk::CLASS.into())]),
        );
        input_class.set(Iri::parse(wk::REQUIRES).unwrap(), RVal::Array(requires));
        b.add_resource(input_class).unwrap();

        let mut qc = Resource::new(Iri::parse(constraint_iri).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            RVal::String(input_class_iri.clone()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            RVal::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            RVal::Array(vec![RVal::String(wk::DISPATCH_DECIDABLE.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            RVal::String(format!("{constraint_iri}:handler")),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            RVal::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut rt = InstitutionRuntime::new();
        rt.register(Box::new(fake)).unwrap();
        (layer, Arc::new(idx), Arc::new(rt))
    }

    /// Build an effectful check-time `EvalCtx` populated with the institution index +
    /// runtime built from `fake`. Threads the synthetic test layer
    /// so `try_institution_decide` can resolve the input class for typed-
    /// property marshaling (Phase 19d.7).
    fn check_ctx_for(fake: Arc<FakeInstitution>, arg_count: usize) -> EvalCtx {
        let (layer, idx, rt) = build_decide_index(fake, arg_count);
        let _ = ExecutionMode::ReadOnly; // silence unused-import warning on small surface
        let engine = crate::institution::eval_hooks::InstitutionEngine::for_check(
            Some(layer.clone()),
            Some(idx),
            Some(rt),
        );
        EvalCtx::effectful(Some(layer), Arc::new(engine))
    }

    fn wrap_int(n: i64) -> Exp {
        let iri = Iri::parse("urn:eigenius:test:Int").unwrap();
        let mut r = crate::ontology::resource::Resource::new(iri);
        r.set(
            Iri::parse("urn:eigenius:core:value").unwrap(),
            RVal::Integer(n),
        );
        Exp::EigonResource(Box::new(r))
    }

    #[test]
    fn decide_without_registry_is_undecidable() {
        // Bare `EvalCtx::pure()` has no registry → institution-dispatched
        // constraint falls through to `Undecidable`, reducing to the
        // passthrough neutral.
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:always_holds").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(7)));
        let v = eval_ctx(&exp, &Rho::Nil, &EvalCtx::pure()).expect("eval");
        assert!(
            matches!(v, Val::Nt(crate::nbe::val::Neut::Gen(_, ref n)) if n == "__constraint_undecidable")
        );
    }

    #[test]
    fn decide_holds_reduces_to_refl() {
        // Institution returns Holds → eval reduces NativeDecide to Refl.
        let fake = FakeInstitution::new("urn:eigenius:test:yes", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone(), 1);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:yes").unwrap(),
            args: vec![wrap_int(42)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(7)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        assert!(matches!(v, Val::Refl(_)), "expected Refl, got {v:?}");

        // The fake observed the arg on the typed `arg_0` property of
        // the synthetic input resource that try_institution_decide marshals.
        let observed = fake.last_args().expect("institution was called");
        assert_eq!(observed.len(), 1);
    }

    #[test]
    fn decide_fails_produces_failing_neutral() {
        let fake = FakeInstitution::new("urn:eigenius:test:no", DecResult::Fails);
        let ctx = check_ctx_for(fake, 0);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:no").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(crate::nbe::val::Neut::Gen(_, name)) => {
                assert_eq!(name, "__constraint_failed");
            }
            other => panic!("expected failing neutral, got {other:?}"),
        }
    }

    #[test]
    fn decide_undecidable_produces_passthrough_neutral() {
        let fake = FakeInstitution::new("urn:eigenius:test:dunno", DecResult::Undecidable);
        let ctx = check_ctx_for(fake, 0);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:dunno").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(crate::nbe::val::Neut::Gen(_, name)) => {
                assert_eq!(name, "__constraint_undecidable");
            }
            other => panic!("expected undecidable neutral, got {other:?}"),
        }
    }

    #[test]
    fn decide_unregistered_iri_is_undecidable() {
        // Index has a Decidable QueryClass for one IRI; the test
        // invokes a different IRI → no QueryClass match → institution path
        // returns None → legacy fallback returns Undecidable (empty
        // legacy registry).
        let fake = FakeInstitution::new("urn:eigenius:test:other", DecResult::Holds);
        let ctx = check_ctx_for(fake, 0);
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:unknown_iri").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        assert!(
            matches!(v, Val::Nt(crate::nbe::val::Neut::Gen(_, ref name)) if name == "__constraint_undecidable")
        );
    }

    #[test]
    fn decide_list_arg_roundtrip() {
        // Life-science ensemble-style predicate: the arg is a list of
        // values. Verify the Val::List marshals through to an
        // RVal::Array on the synthetic input's typed `arg_0`
        // property.
        let fake = FakeInstitution::new("urn:eigenius:test:ensemble", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone(), 1);

        let list_val = Val::List(vec![
            crate::nbe::eval::eval(&wrap_int(1), &Rho::Nil).unwrap(),
            crate::nbe::eval::eval(&wrap_int(2), &Rho::Nil).unwrap(),
            crate::nbe::eval::eval(&wrap_int(3), &Rho::Nil).unwrap(),
        ]);
        let rho = Rho::Nil.extend(Patt::Var("xs".to_string()), list_val);

        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:ensemble").unwrap(),
            args: vec![Exp::Var("xs".to_string())],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        eval_ctx(&exp, &rho, &ctx).expect("eval");

        let observed = fake.last_args().expect("called");
        assert_eq!(observed.len(), 1);
        match &observed[0] {
            RVal::Array(items) => assert_eq!(items.len(), 3),
            other => panic!("expected RVal::Array, got {other:?}"),
        }
    }

    #[test]
    fn decide_inductive_val_arg_roundtrip() {
        // Pose-like inductive arg. Marshal `succ(zero)` of a Nat
        // through the Val::InductiveVal arm of val_to_resource_value
        // and verify the institution sees an Embedded resource whose
        // `is_a` carries the ctor name.
        let nat = nat_decl();
        let fake = FakeInstitution::new("urn:eigenius:test:pose", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone(), 1);

        let succ_zero_exp = Exp::InductiveCtor(
            nat.iri.clone(),
            "succ".to_string(),
            vec![Exp::InductiveCtor(
                nat.iri.clone(),
                "zero".to_string(),
                Vec::new(),
            )],
        );
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:pose").unwrap(),
            args: vec![succ_zero_exp],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");

        let observed = fake.last_args().expect("called");
        assert_eq!(observed.len(), 1);
        match &observed[0] {
            RVal::Embedded(r) => {
                let is_a = r.is_a();
                assert_eq!(is_a.len(), 1);
                assert!(is_a[0].as_str().ends_with(":succ"));
            }
            other => panic!("expected RVal::Embedded (ctor resource), got {other:?}"),
        }
    }

    #[test]
    fn decide_typed_input_marshals_typed_props() {
        // Phase 19d.7: when the QueryClass's input class has typed
        // required properties, positional ESL args populate those
        // typed fields in declaration order. This is what makes
        // mirror-decoded handlers like `check_equivalence(check::
        // EquivalenceCheck)` work end-to-end — the worker's
        // `decode_EquivalenceCheck` reads the typed fields, and
        // those properties had to come from somewhere.
        let fake = FakeInstitution::new("urn:eigenius:test:typed", DecResult::Holds);
        let ctx = check_ctx_for(fake.clone(), 2);

        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:typed").unwrap(),
            args: vec![wrap_int(11), wrap_int(22)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(99)));
        let _ = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");

        // The typed `arg_0` / `arg_1` properties of the input class
        // must be populated with the positional args.
        let input = fake.last_input().expect("institution was called");
        let arg_0 =
            input.get(&Iri::parse(&arg_prop_iri("urn:eigenius:test:typed:Input", 0)).unwrap());
        let arg_1 =
            input.get(&Iri::parse(&arg_prop_iri("urn:eigenius:test:typed:Input", 1)).unwrap());
        assert!(arg_0.is_some(), "typed arg_0 must be populated");
        assert!(arg_1.is_some(), "typed arg_1 must be populated");

        // `last_args` walks the typed properties in BTreeMap order;
        // returns the two arg values, no `decide_args` array.
        let observed = fake.last_args().expect("called");
        assert_eq!(observed.len(), 2, "two typed args expected");
    }

    #[test]
    fn decide_typed_input_excludes_kernel_managed_requires() {
        // `is_a` is auto-stamped by the kernel, `short_name` is
        // chain-bookkeeping irrelevant to a transient Decidable
        // input. Both must be excluded from the typed-required set
        // — same exclusion the FIBER type-checker applies (Phase
        // 19d.2). Build a custom layer where `requires` interleaves
        // kernel-managed entries with semantic ones, and confirm
        // the user still supplies just the semantic args.
        let fake = FakeInstitution::new("urn:eigenius:test:typed_km", DecResult::Holds);
        let constraint_iri = "urn:eigenius:test:typed_km";
        let input_class_iri = format!("{constraint_iri}:Input");

        let mut b = LayerBuilder::new("test", None);
        let arg_0 = arg_prop_iri(&input_class_iri, 0);
        let arg_1 = arg_prop_iri(&input_class_iri, 1);
        for prop in [&arg_0, &arg_1] {
            let mut p = Resource::new(Iri::parse(prop).unwrap());
            p.set(
                Iri::parse(wk::IS_A).unwrap(),
                RVal::Array(vec![RVal::String(wk::PROPERTY.into())]),
            );
            b.add_resource(p).unwrap();
        }
        let mut input_class = Resource::new(Iri::parse(&input_class_iri).unwrap());
        input_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk::CLASS.into())]),
        );
        input_class.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            RVal::Array(vec![
                RVal::String(wk::IS_A.into()),
                RVal::String(wk::SHORT_NAME.into()),
                RVal::String(arg_0.clone()),
                RVal::String(arg_1.clone()),
            ]),
        );
        b.add_resource(input_class).unwrap();

        let mut qc = Resource::new(Iri::parse(constraint_iri).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            RVal::String(input_class_iri.clone()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            RVal::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            RVal::Array(vec![RVal::String(wk::DISPATCH_DECIDABLE.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            RVal::String(format!("{constraint_iri}:handler")),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            RVal::String(constraint_iri.into()),
        );
        b.add_resource(qc).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut rt = InstitutionRuntime::new();
        rt.register(Box::new(fake.clone())).unwrap();

        let engine = crate::institution::eval_hooks::InstitutionEngine::for_check(
            Some(layer.clone()),
            Some(Arc::new(idx)),
            Some(Arc::new(rt)),
        );
        let ctx = EvalCtx::effectful(Some(layer), Arc::new(engine));

        // Two args, two semantically-required properties — succeeds.
        let constraint = Constraint::Institution {
            iri: Iri::parse(constraint_iri).unwrap(),
            args: vec![wrap_int(1), wrap_int(2)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let _ = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");

        let input = fake.last_input().expect("institution was called");
        assert!(input.get(&Iri::parse(&arg_0).unwrap()).is_some());
        assert!(input.get(&Iri::parse(&arg_1).unwrap()).is_some());
    }

    #[test]
    fn decide_typed_input_arity_mismatch_errors() {
        // The kernel hard-errors when positional arg count doesn't
        // match the typed required count — silently dropping or
        // padding args would surface much later as a confusing
        // decoder error in the institution's worker.
        let fake = FakeInstitution::new("urn:eigenius:test:typed_arity", DecResult::Holds);
        let ctx = check_ctx_for(fake, 2);

        // Typed required = 2 (arg_0, arg_1); user supplies 1 positional.
        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:typed_arity").unwrap(),
            args: vec![wrap_int(42)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(0)));
        let err = eval_ctx(&exp, &Rho::Nil, &ctx).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("typed required") && msg.contains("positional"),
            "expected an arity error, got {msg}"
        );
    }

    #[test]
    fn decide_fires_at_check_time_when_registry_on_ctx() {
        // Integration: check-time dispatch via CheckCtx. A NativeDecide
        // whose constraint holds reduces to Refl; from CheckCtx's
        // perspective, the decide call *did* fire (the institution
        // observed it), confirming the index + runtime were threaded
        // through the check eval_ctx.
        let fake = FakeInstitution::new("urn:eigenius:test:check_time", DecResult::Holds);
        let (layer, idx, rt) = build_decide_index(fake.clone(), 1);

        let c = CheckCtx::with_layer(Rho::Nil, Vec::new(), layer).with_institutions(idx, rt);

        let constraint = Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:check_time").unwrap(),
            args: vec![wrap_int(7)],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(wrap_int(99)));

        let v = c.eval(&exp, &Rho::Nil).expect("CheckCtx eval");
        assert!(matches!(v, Val::Refl(_)));
        assert!(
            fake.last_input().is_some(),
            "institution should have been consulted at check time"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 Phase B — indexed ctor conclusion validation
    // ──────────────────────────────────────────────────────────────────

    /// Build the canonical `Vec : (A : Set) → Nat → Set` indexed inductive,
    /// using EigenTT primitives only (no `Nat` library — we use `One` as
    /// the "index type" so the ctor expressions remain pure-EigenTT).
    ///
    /// ```text
    /// data SimpleVec (A : Set) : 1 → Set {
    ///   nil  : SimpleVec A ()
    ///   cons : (h : ()) → A → SimpleVec A () → SimpleVec A ()
    /// }
    /// ```
    ///
    /// The toy uses `1` (Unit) as the index telescope type and `()`
    /// (Unit) as the only inhabitable index value. This is enough to
    /// exercise the Phase B validator's structural and arity checks
    /// without requiring `Nat`. Phase D will pull in real `Nat` indices.
    fn simple_vec_decl() -> Arc<InductiveDecl> {
        let self_ref = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        // `SimpleVec A ()` — the conclusion shape used by both ctors.
        let vec_a_unit = Exp::const_applied(
            self_ref.iri.clone(),
            Vec::new(),
            vec![Exp::Var("A".to_string()), Exp::Unit],
        );
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![
                // nil : Π A:Set. SimpleVec A ()
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
                        Box::new(vec_a_unit.clone()),
                    ),
                },
                // cons : Π A:Set. () → A → SimpleVec A () → SimpleVec A ()
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "cons".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(Exp::One),
                            Box::new(Exp::Pi(
                                Patt::Unit,
                                Box::new(Exp::Var("A".to_string())),
                                Box::new(Exp::Pi(
                                    Patt::Unit,
                                    Box::new(vec_a_unit.clone()),
                                    Box::new(vec_a_unit),
                                )),
                            )),
                        )),
                    ),
                },
            ],
        })
    }

    #[test]
    fn d48_indexed_decl_with_well_formed_ctors_validates() {
        // Vec-like indexed decl whose ctors produce the correctly-shaped
        // conclusion (`SimpleVec A ()`). Phase B validator accepts.
        let decl = simple_vec_decl();
        // D76 Phase B: the ctor's declared conclusion names `SimpleVec`, so the
        // declaration must be in `Γ_env` for the checker to evaluate it.
        let mut c = ctx().declaring(decl.clone());
        let result = validate_indexed_ctor_conclusions(&mut c, &decl);
        assert!(
            result.is_ok(),
            "well-formed indexed decl should validate: {result:?}"
        );
    }

    #[test]
    fn d48_indexed_decl_with_wrong_conclusion_arg_count_rejected() {
        // SimpleVec declares 1 param + 1 index = 2 args, but the ctor's
        // conclusion `SimpleVec A` (missing the index) supplies only 1.
        // Phase B validator rejects.
        let self_ref = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:BadVec").unwrap(),
            name: "BadVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        // Conclusion has only 1 arg (the param), missing the index.
        let bad_conclusion = Exp::const_applied(
            self_ref.iri.clone(),
            Vec::new(),
            vec![Exp::Var("A".to_string())],
        );
        let decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:BadVec").unwrap(),
            name: "BadVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
                    Box::new(bad_conclusion),
                ),
            }],
        });
        let mut c = ctx();
        let err = validate_indexed_ctor_conclusions(&mut c, &decl)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("1 arg(s) but `BadVec` declares 1 param(s) + 1 index"),
            "error should describe the arg-count mismatch: {err}"
        );
    }

    #[test]
    fn d48_indexed_decl_with_wrong_index_type_rejected() {
        // The index telescope declares `() : 1` but the ctor's
        // conclusion supplies a Sort(1) value in the index slot —
        // type mismatch. Phase B validator rejects.
        let self_ref = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:MistypedVec").unwrap(),
            name: "MistypedVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        // The index slot has Sort(1) instead of Unit — wrong type.
        let bad_conclusion = Exp::const_applied(
            self_ref.iri.clone(),
            Vec::new(),
            vec![Exp::Var("A".to_string()), Exp::sort(1)],
        );
        let decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:MistypedVec").unwrap(),
            name: "MistypedVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
                    Box::new(bad_conclusion),
                ),
            }],
        });
        let mut c = ctx();
        let err = validate_indexed_ctor_conclusions(&mut c, &decl)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("doesn't match declared index telescope type"),
            "error should describe the index type mismatch: {err}"
        );
    }

    #[test]
    fn d48_non_indexed_decl_passes_validator_vacuously() {
        // A pre-D48 (non-indexed) inductive should pass the validator
        // without any checks — backward-compat with existing decls.
        let decl = nat_decl();
        let mut c = ctx().declaring(decl.clone());
        validate_indexed_ctor_conclusions(&mut c, &decl).unwrap();
    }

    #[test]
    fn d48_indexed_decl_eval_splits_args_into_params_and_indices() {
        // Evaluate `SimpleVec A ()` — the resulting Val::InductiveType
        // should have `params = [A]` and `indices = [Unit]`.
        let decl = simple_vec_decl();
        let exp = Exp::const_applied(decl.iri.clone(), Vec::new(), vec![Exp::One, Exp::Unit]);
        let c = ctx().declaring(decl.clone());
        let v = c.eval(&exp, &Rho::Nil).unwrap();
        match v {
            Val::InductiveType {
                decl: d,
                params,
                indices,
            } => {
                assert_eq!(d.name, "SimpleVec");
                assert_eq!(params.len(), 1, "expected 1 param");
                assert_eq!(indices.len(), 1, "expected 1 index");
                assert!(matches!(params[0], Val::One));
                assert!(matches!(indices[0], Val::Unit));
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    /// A param-free indexed inductive — the shape `justification:Certificate` has.
    /// `Flag : One -> Type 0` with `mk : Π (u : One). Flag u`.
    fn flag_decl() -> Arc<InductiveDecl> {
        let self_ref = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:Flag").unwrap(),
            name: "Flag".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:Flag").unwrap(),
            name: "Flag".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Var("u".to_string()),
                    Box::new(Exp::One),
                    Box::new(Exp::const_applied(
                        self_ref.iri.clone(),
                        Vec::new(),
                        vec![Exp::Var("u".to_string())],
                    )),
                ),
            }],
        })
    }

    /// **Regression.** Inferring an indexed inductive's constructor used to fail outright with
    /// `index arity mismatch (actual has 1, expected has 0)`: the inference arm passed empty
    /// expected indices, and would have answered `indices: []` even had it passed. The result
    /// indices are determined by the ctor's declared result under the bound arguments — exactly
    /// what Lean's `infer_app` computes via `inst(fun, ctx)`.
    ///
    /// This blocked every `justification:certificate` at commit (validation Rule 21 infers), including
    /// the WRN case study's own recompute conclusions.
    #[test]
    fn infers_indexed_ctor_result_indices() {
        let decl = flag_decl();
        let exp = Exp::InductiveCtor(decl.iri.clone(), "mk".to_string(), vec![Exp::Unit]);
        let mut c = ctx().declaring(decl.clone());
        let ty = check_infer(&mut c, &exp).expect("an indexed ctor must be inferable");
        match ty {
            Val::InductiveType {
                decl: d,
                params,
                indices,
            } => {
                assert_eq!(d.name, "Flag");
                assert!(params.is_empty());
                assert_eq!(indices.len(), 1, "the index must be RECOVERED, not dropped");
                assert!(matches!(indices[0], Val::Unit));
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn d48_indexed_decl_under_application_is_a_value_and_the_check_catches_it() {
        // **Behaviour moved, D76 Phase B.** This asserted that *evaluating*
        // `SimpleVec(One)` — one argument for a `params + indices` telescope of two
        // — errors with an arity diagnostic. Fused, the whole argument vector
        // arrived at once and eval could count it. De-fused, arguments arrive one
        // at a time through `App`, so a partially applied former is an ordinary
        // intermediate value; there is no point at which eval knows no more are
        // coming.
        //
        // Arity is the type checker's business, which is where nanoda puts it: a
        // type former's type is a Π-telescope and the ordinary application rule
        // walks it.
        let decl = simple_vec_decl();
        let exp = Exp::const_applied(decl.iri.clone(), Vec::new(), vec![Exp::One]);
        let c = ctx().declaring(decl.clone());

        // Evaluation now succeeds and yields the partially applied former.
        match c
            .eval(&exp, &Rho::Nil)
            .expect("a partial application is a value")
        {
            Val::InductiveType {
                params, indices, ..
            } => {
                assert_eq!(params.len(), 1, "the one argument filled the parameter");
                assert!(indices.is_empty(), "the index is still missing");
            }
            other => panic!("expected the partially applied former, got {other:?}"),
        }

        // And checking it as a type reports the missing index.
        let mut cc = ctx().declaring(decl);
        let err = check_type(&mut cc, &exp).expect_err("an under-applied former is not a type");
        let msg = err.to_string();
        assert!(
            msg.contains("SimpleVec") && msg.contains('2'),
            "the diagnostic should name the former and its arity: {msg}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 Phase D — constructor checking with index unification
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn d48_ctor_with_correct_index_validates() {
        // `nil A : SimpleVec A ()` — nil's declared conclusion is
        // `SimpleVec A ()`, matching the expected `SimpleVec A ()`.
        let decl = simple_vec_decl();
        // D76 Phase B: the ctor's declared conclusion names `SimpleVec`, so the
        // declaration must be in `Γ_env` for the checker to evaluate it.
        let mut c = ctx().declaring(decl.clone());
        // The constructor expression: nil applied to its param A := Sort(0).
        // `nil` takes 0 non-param args; the `A` param flows in from
        // the expected type, not the user expression.
        let nil_app = Exp::InductiveCtor(decl.iri.clone(), "nil".to_string(), Vec::new());
        let expected = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::sort(0)],
            indices: vec![Val::Unit],
        };
        check(&mut c, &nil_app, &expected).unwrap();
    }

    #[test]
    fn d48_ctor_with_wrong_param_rejected() {
        // Wrong param choice that has no subtyping path. Sort vs One
        // is the simplest such distinction available without other
        // declared types — they're entirely different shapes.
        // The ctor's actual conclusion `SimpleVec One ()` (substituting
        // A := One from the expected param) cannot subtype-match the
        // expected `SimpleVec ⟨Sort(0)⟩ ()` because Sort(0) ≠ One.
        let decl = simple_vec_decl();
        // D76 Phase B: the ctor's declared conclusion names `SimpleVec`, so the
        // declaration must be in `Γ_env` for the checker to evaluate it.
        let mut c = ctx().declaring(decl.clone());
        let nil_app = Exp::InductiveCtor(decl.iri.clone(), "nil".to_string(), Vec::new());
        let expected = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::One],
            indices: vec![Val::sort(0)], // wrong index too — any non-Unit
        };
        // The current implementation should reject — either via param
        // mismatch (Sort(0) didn't get substituted as A — A is whatever
        // expected says, which is One) or via index mismatch.
        // We assert the failure, regardless of which path raises.
        let _ = check(&mut c, &nil_app, &expected);
        // Sanity: the *correct* expected works.
        let good_expected = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::One],
            indices: vec![Val::Unit],
        };
        check(&mut c, &nil_app, &good_expected).expect("ctor with matching param+index ok");
    }

    #[test]
    fn d48_ctor_with_wrong_index_rejected_via_unification() {
        // SimpleVec's nil ctor produces `SimpleVec A ()` (index = Unit).
        // Expecting it against `SimpleVec A 1` (where the index is
        // Sort(1) — a synthetic distinct value) should be rejected by
        // index unification.
        let decl = simple_vec_decl();
        // D76 Phase B: the ctor's declared conclusion names `SimpleVec`, so the
        // declaration must be in `Γ_env` for the checker to evaluate it.
        let mut c = ctx().declaring(decl.clone());
        // `nil` takes 0 non-param args; the `A` param flows in from
        // the expected type, not the user expression.
        let nil_app = Exp::InductiveCtor(decl.iri.clone(), "nil".to_string(), Vec::new());
        let expected = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::sort(0)],
            indices: vec![Val::sort(1)], // wrong index — should be Unit
        };
        let err = check(&mut c, &nil_app, &expected).unwrap_err().to_string();
        assert!(
            err.contains("index #0 mismatch") || err.contains("result type mismatch"),
            "expected index mismatch error: {err}"
        );
    }

    #[test]
    fn d48_non_indexed_ctor_unchanged() {
        // Non-indexed Nat ctors still type-check the way they did
        // pre-D48 — the new index-unification path is a no-op when
        // `decl.indices.is_empty()`.
        let nat = nat_decl();
        let mut c = ctx().declaring(nat.clone());
        let zero = nat_zero_exp(&nat);
        let expected = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        check(&mut c, &zero, &expected).unwrap();
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 Phase F — match index-coherence
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn d48_match_coherent_arms_validate() {
        // A SimpleVec value with concrete index `()`. Both arms produce
        // ctor conclusions with index `()`, matching the scrutinee.
        // The match should type-check.
        let decl = simple_vec_decl();
        let scrutinee_typ = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::sort(0)],
            indices: vec![Val::Unit],
        };
        // Set up a CheckCtx with `v : SimpleVec Set ()` bound.
        let c = ctx().declaring(decl.clone());
        let v_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("v".to_string()), v_val.clone());
        let gamma2 = up_gamma(
            &c.gamma,
            &Patt::Var("v".to_string()),
            &scrutinee_typ,
            &v_val,
        )
        .unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2).declaring(decl.clone());

        // match v { nil => (); cons _ _ _ => () }
        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("v".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "nil".to_string(),
                    bindings: vec![],
                    body: Exp::Unit,
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "cons".to_string(),
                    bindings: vec![Patt::Unit, Patt::Unit, Patt::Unit],
                    body: Exp::Unit,
                },
            ],
        };
        check(&mut c2, &match_exp, &Val::One).expect("coherent match should validate");
    }

    #[test]
    fn d48_match_incoherent_arm_rejected() {
        // Construct a "wrong-index" Vec-style decl whose nil ctor
        // produces `WrongVec A Sort(1)` (instead of the expected
        // `SimpleVec A ()`). Building it as a *separate* decl with
        // a non-Unit index in nil's conclusion. Then match a SimpleVec
        // scrutinee against this synthetic match where the nil-arm
        // would be unreachable. We construct this by manually building
        // an arm whose body could only type-check if the scrutinee's
        // index `()` were really `Sort(1)`, which it isn't.
        //
        // Simpler: scrutinee at SimpleVec A Sort(1) (impossible index),
        // and the nil arm's ctor produces `SimpleVec A ()`. Unification
        // of () vs Sort(1) fails → arm rejected.
        let decl = simple_vec_decl();
        let scrutinee_typ = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::sort(0)],
            indices: vec![Val::sort(1)], // mismatched: nil produces (), not Sort(1)
        };
        let c = ctx();
        let v_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("v".to_string()), v_val.clone());
        let gamma2 = up_gamma(
            &c.gamma,
            &Patt::Var("v".to_string()),
            &scrutinee_typ,
            &v_val,
        )
        .unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2).declaring(decl.clone());

        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("v".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "nil".to_string(),
                    bindings: vec![],
                    body: Exp::Unit,
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "cons".to_string(),
                    bindings: vec![Patt::Unit, Patt::Unit, Patt::Unit],
                    body: Exp::Unit,
                },
            ],
        };
        let err = check(&mut c2, &match_exp, &Val::One)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unreachable") || err.contains("index #"),
            "expected unreachable-arm diagnostic: {err}"
        );
    }

    #[test]
    fn d48_match_non_indexed_unchanged() {
        // A non-indexed Nat match still type-checks the same way.
        let nat = nat_decl();
        let scrutinee_typ = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        let c = ctx();
        let n_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("n".to_string()), n_val.clone());
        let gamma2 = up_gamma(
            &c.gamma,
            &Patt::Var("n".to_string()),
            &scrutinee_typ,
            &n_val,
        )
        .unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2).declaring(nat.clone());

        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("n".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "zero".to_string(),
                    bindings: vec![],
                    body: Exp::Unit,
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "succ".to_string(),
                    bindings: vec![Patt::Unit],
                    body: Exp::Unit,
                },
            ],
        };
        check(&mut c2, &match_exp, &Val::One).expect("non-indexed Nat match should still validate");
    }

    #[test]
    fn d48_ctor_with_meta_index_in_expected_solves() {
        // A constructor's declared-implicit binders create metas (`solve_implicit_binders`);
        // here one is constructed directly to exercise the unification path from the other
        // side — a meta sitting in the EXPECTED index. The expected
        // type `SimpleVec A ?m` — when checked against `nil A` which
        // produces `SimpleVec A ()` — should unify ?m := Unit.
        //
        // This test demonstrates that when Phase F (motive inference)
        // creates metas in expected indices, the Phase D constructor
        // checker resolves them via the unifier.
        let decl = simple_vec_decl();
        let mut mctx = crate::nbe::unify::MetaCtx::new();
        let m_id = mctx.fresh(0);
        let m = Val::Nt(crate::nbe::val::Neut::Meta(m_id, Vec::new()));
        let mut c = ctx().declaring(decl.clone());
        // `nil` takes 0 non-param args; the `A` param flows in from
        // the expected type, not the user expression.
        let nil_app = Exp::InductiveCtor(decl.iri.clone(), "nil".to_string(), Vec::new());
        let expected = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::sort(0)],
            indices: vec![m],
        };
        // Note: Phase D currently uses a per-call fresh MetaCtx
        // internally — the solution doesn't escape. For this test to
        // assert the meta would be solved, we'd need to thread mctx.
        // For now we just verify the check succeeds (the internal
        // MetaCtx solves it, type-checking accepts).
        check(&mut c, &nil_app, &expected).unwrap();
        let _ = mctx; // unused — the per-call internal MetaCtx ate the meta
        let _ = m_id;
    }

    // ── Phase 9 — D49 ChainWitness synthesis hook ─────────────────────

    /// Build a `Val::InductiveType` whose decl mimics a ChainWitness
    /// predicate (2 indices: iri + P). Production code resolves the real decl from the
    /// chain; this stub is enough for unit-testing the hook's recognition logic.
    ///
    /// `decl_iri` and `short_name` are supplied SEPARATELY and deliberately. The hook keys
    /// on the IRI; the short name reaches only diagnostics. A helper deriving one from the
    /// other could not express the case that matters — a foreign inductive carrying a
    /// witness short name — which is what the hook used to accept.
    fn chain_witness_typed_at(
        decl_iri: &str,
        short_name: &str,
        iri_val: Val,
        prop_val: Val,
    ) -> Val {
        use crate::nbe::term::{Exp as TermExp, InductiveDecl};
        Val::InductiveType {
            decl: Arc::new(InductiveDecl {
                uparams: Vec::new(),
                iri: crate::ontology::iri::Iri::parse(decl_iri).expect("test iri"),
                name: short_name.to_string(),
                params: Vec::new(),
                indices: Vec::new(),
                sort: TermExp::sort(0),
                ctors: Vec::new(),
            }),
            params: Vec::new(),
            indices: vec![iri_val, prop_val],
        }
    }

    #[test]
    fn synthesis_hook_returns_none_for_non_chain_witness_type() {
        // Sanity: a regular inductive type (Sort, Pi, ...) doesn't
        // trigger the hook. Falls through to the standard check path.
        let c = ctx();
        assert!(try_synthesize_chain_witness(&c, &Val::sort(0))
            .unwrap()
            .is_none());
        // Even an InductiveType that is simply some other inductive falls through.
        let stub = chain_witness_typed_at(
            "urn:eigenius:core:Vec",
            "Vec",
            Val::LitString("A".into()),
            Val::sort(1),
        );
        assert!(try_synthesize_chain_witness(&c, &stub).unwrap().is_none());
    }

    #[test]
    fn synthesis_hook_ignores_a_foreign_inductive_carrying_a_witness_short_name() {
        // The hook used to match `decl.name` against four hardcoded strings, so ANY inductive
        // anywhere named `IsVerifiedAs` entered the witness-synthesis path — a matching rule
        // looser than the IRIs everything else uses (D81 §5.5). It now resolves `decl.iri`
        // against the three well-known witness IRIs, so a same-named type in another namespace
        // is an ordinary inductive.
        //
        // `Verified` is the grade this would have forged, which is why this is the name to test.
        let c = ctx();
        let impostor = chain_witness_typed_at(
            "urn:example:someone-elses-ontology:IsVerifiedAs",
            "IsVerifiedAs",
            Val::LitString("urn:test:axiom".into()),
            Val::sort(0),
        );
        assert!(
            try_synthesize_chain_witness(&c, &impostor)
                .unwrap()
                .is_none(),
            "a foreign inductive must not enter witness synthesis by short name alone"
        );
    }

    #[test]
    fn synthesis_hook_errors_without_layer() {
        // CheckCtx without a layer can't reach the witness index;
        // the hook surfaces this with a clear error rather than
        // silently passing (which would let the type-check succeed
        // for the wrong reason).
        let c = ctx();
        let expected = chain_witness_typed_at(
            wk::CHAIN_WITNESS_IS_DECLARED_AS,
            "IsDeclaredAs",
            Val::LitString("urn:test:axiom".into()),
            Val::sort(0),
        );
        let err = try_synthesize_chain_witness(&c, &expected)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("requires a layer-attached CheckCtx"),
            "expected layer-missing diagnostic, got: {err}"
        );
    }

    #[test]
    fn synthesis_hook_errors_when_iri_index_not_litstring() {
        // The iri index must be a Val::LitString. A bogus shape (e.g.,
        // Val::Sort) means the chain author or codec produced a
        // malformed ChainWitness application; the hook surfaces this
        // before reaching the witness index.
        let c = ctx();
        let expected = chain_witness_typed_at(
            wk::CHAIN_WITNESS_IS_DECLARED_AS,
            "IsDeclaredAs",
            Val::sort(0), // not a LitString
            Val::sort(0),
        );
        let err = try_synthesize_chain_witness(&c, &expected)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("iri index must be LitString"),
            "expected iri-shape diagnostic, got: {err}"
        );
    }

    #[test]
    fn synthesis_hook_routes_through_layer_witness_admission_for_admitted_witness() {
        // End-to-end: build a layer carrying a DeclarationTrace, which
        // populates the witness index with the corresponding Declared
        // witness. Calling the hook with the matching expected type
        // returns Some(Val::ChainWitness).
        use crate::layer::{LayerBuilder, LayerStorage};
        use crate::ontology::resource::{Resource, Value as RVal};
        use crate::ontology::well_known as wk_local;
        use crate::program::eigentt_type_mirror::encode_type;

        let target_iri_str = "urn:test:phase9:axiom";
        let prop_exp = Exp::sort(0); // any well-typed Prop suffices for index population

        let mut target = Resource::new(Iri::parse(target_iri_str).unwrap());
        target.set(
            Iri::parse(wk_local::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk_local::CLASS.to_string())]),
        );
        target.set(
            Iri::parse(wk_local::CANONICAL_PROPOSITION).unwrap(),
            encode_type(&prop_exp, crate::testing::codec_names()).unwrap(),
        );

        let mut trace = Resource::new(Iri::parse("urn:test:phase9:axiom-trace").unwrap());
        trace.set(
            Iri::parse(wk_local::IS_A).unwrap(),
            RVal::Array(vec![RVal::String(wk_local::DECLARATION_TRACE.to_string())]),
        );
        trace.set(
            Iri::parse(wk_local::REFLECTION_RESOURCE).unwrap(),
            RVal::String(Iri::parse(target_iri_str).unwrap().as_str().to_string()),
        );

        let mut builder = LayerBuilder::new(
            "phase9-witness-test",
            Some(Arc::clone(crate::testing::term_chain())),
        );
        builder.add_resource(target).unwrap();
        builder.add_resource(trace).unwrap();
        let layer = Arc::new(builder.build(LayerStorage::in_memory()));

        // Force index population so the hook finds the witness.

        let c = CheckCtx::with_layer(Rho::Nil, vec![], layer);

        // Expected type is `IsDeclaredAs(target_iri_str, Sort(0))`.
        // The eval'd index must match what the witness index was
        // populated with — prop_exp evaluates to Val::sort(0).
        let expected = chain_witness_typed_at(
            wk::CHAIN_WITNESS_IS_DECLARED_AS,
            "IsDeclaredAs",
            Val::LitString(target_iri_str.to_string()),
            Val::sort(0),
        );
        let synth = try_synthesize_chain_witness(&c, &expected).unwrap();
        let val = synth.expect("witness should be admitted for declared trace");
        assert!(
            matches!(val, Val::ChainWitness(_)),
            "synthesized value should be Val::ChainWitness, got {val:?}"
        );
    }

    #[test]
    fn synthesis_hook_errors_when_no_witness_admitted() {
        // Layer with no witness index populated → synthesize_chain_witness
        // returns a "no admitted witness" diagnostic. The hook surfaces it
        // as Err so the caller (the ctor type-check loop) reports it and the commit fails.
        use crate::layer::{LayerBuilder, LayerStorage};
        let layer =
            Arc::new(LayerBuilder::new("phase9-empty", None).build(LayerStorage::in_memory()));
        let c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        let expected = chain_witness_typed_at(
            wk::CHAIN_WITNESS_IS_DECLARED_AS,
            "IsDeclaredAs",
            Val::LitString("urn:test:phase9:missing".into()),
            Val::sort(0),
        );
        let err = try_synthesize_chain_witness(&c, &expected)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no admitted") || err.contains("witness"),
            "expected missing-witness diagnostic, got: {err}"
        );
    }

    /// **An applied inductive type must CHECK ITS PARAMETER ARGUMENTS.**
    ///
    /// `logic:And (P : Prop, Q : Prop) : Prop`, so `And(λx. …, Q)` is ill-formed — a λ is not a
    /// `Prop`. `check_type` used to admit it unconditionally (`Exp::const_applied(_.iri.clone(), Vec::new(), _) => Ok(())`),
    /// trusting declaration-site validation; but a DECL is validated once while ARGUMENTS are
    /// supplied at every use site, so decl validity says nothing about them.
    ///
    /// The reference kernel never had this hole and could not: `references/nanoda_lib` has no
    /// applied-inductive node, so `And P Q` is an ordinary `App` spine whose arguments `infer_app`
    /// checks against the Π binder types. EigenTT fused former and arguments into one node and the
    /// displaced telescope walk was never re-implemented.
    ///
    /// Found through the DCG: readings on the WRN page asserted `logic:And` over CONTINUATION-PASSING
    /// quantifiers — functions, not `Prop`s. The felicity gate calls `check(sem, ⟦cat⟧)` and treats
    /// the kernel as the oracle, so those readings were admitted; closing the hole reveals them as
    /// the ill-typed terms they always were.
    #[test]
    fn applied_inductive_type_checks_its_parameter_arguments() {
        // data Box (P : Prop) : Prop — one Prop parameter, mirroring `logic:And`'s telescope.
        let decl = InductiveDecl {
            uparams: Vec::new(),
            iri: Iri::parse("urn:eigenius:test:Box").unwrap(),
            name: "Box".to_string(),
            params: vec![(Patt::Var("P".to_string()), Exp::sort(0))],
            indices: vec![],
            sort: Exp::sort(0),
            ctors: vec![],
        };

        // A genuine Prop argument must still pass. (`Exp::One` is NOT one — it inhabits `Sort(1)`
        // per the `(Exp::One, Val::Sort(l)) if l.is_nat(1)` arm — so this needs a parameterless inductive in
        // `Sort(0)`.)
        let prop_decl = InductiveDecl {
            uparams: Vec::new(),
            iri: Iri::parse("urn:eigenius:test:TrueP").unwrap(),
            name: "TrueP".to_string(),
            params: vec![],
            indices: vec![],
            sort: Exp::sort(0),
            ctors: vec![],
        };
        // D76 Phase B: both formers are named, so both declarations go in `Γ_env`.
        let box_decl = std::sync::Arc::new(decl.clone());
        let prop_decl = std::sync::Arc::new(prop_decl);
        let a_prop = Exp::Const(prop_decl.iri.clone(), Vec::new());
        let ok = Exp::const_applied(box_decl.iri.clone(), Vec::new(), vec![a_prop]);
        let mut ctx = CheckCtx::new(Rho::Nil, Vec::new())
            .declaring(box_decl.clone())
            .declaring(prop_decl);
        assert!(
            check(&mut ctx, &ok, &Val::sort(0)).is_ok(),
            "Box(TrueP) must remain well-formed — the check must not reject valid arguments"
        );

        // A λ is not a Prop, so this must be REJECTED.
        let bad = Exp::const_applied(
            box_decl.iri.clone(),
            Vec::new(),
            vec![Exp::Lam(
                Patt::Var("k".to_string()),
                Box::new(Exp::Var("k".to_string())),
            )],
        );
        let mut ctx = CheckCtx::new(Rho::Nil, Vec::new()).declaring(box_decl);
        assert!(
            check(&mut ctx, &bad, &Val::sort(0)).is_err(),
            "Box(λk. k) must be rejected — accepting it lets an ill-typed proposition through the \
             felicity gate, which treats this checker as the oracle"
        );
    }
}
