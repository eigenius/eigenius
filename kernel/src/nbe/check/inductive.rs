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

//! Inductive-type checking: strict-positivity companions (large
//! elimination, D46 §7 / D48 Phase H), indexed-ctor validation,
//! constructor/recursor/match checking (Phase 11b, D19, D48).
//! Split from `check.rs`; nanoda precedent (`inductive.rs` apart
//! from `tc.rs`).

use super::conv::{is_propositional_in_ctx, is_syntactically_propositional_type};
use super::subtype_of_with_hyps;
use super::witness::try_synthesize_chain_witness;
use super::{check, check_infer, CheckCtx, CheckError};
use crate::nbe::env::{gen_val, Rho};
use crate::nbe::eval::EvalCtx;
use crate::nbe::readback::readback_val;
use crate::nbe::recursor::derive_minor_types;
use crate::nbe::term::{Exp, InductiveDecl, Patt};
use crate::nbe::val::{Clos, Val};
use std::sync::Arc;

/// Singleton-elimination admissibility test for a Prop-typed inductive
/// declaration (D46 §7). Returns true iff a Prop-typed inductive may be
/// eliminated into a non-Prop result type (large elimination).
///
/// **Case A** — zero constructors: large elim is vacuously safe (no Prop
/// inhabitant exists to smuggle information across the Prop/Type boundary).
/// Examples: `False`, `Asserts(iri)`.
///
/// **Case B** — exactly one constructor, *each* of whose non-parameter
/// arguments is itself propositional. This restriction prevents Hurkens-
/// style information leakage. EigenTT lacks indexed inductive families
/// (issue #22), so the variant of case B that admits "arg appears in the
/// conclusion" does not apply here — every non-Prop ctor argument fails
/// the test.
///
/// Any other shape (≥ 2 ctors, or 1 ctor with a non-Prop argument that
/// doesn't appear in the conclusion) returns false, restricting motives
/// of the corresponding recursor / match to Prop.
pub fn large_elim_admitted(decl: &InductiveDecl) -> bool {
    if decl.ctors.is_empty() {
        return true;
    }
    if decl.ctors.len() != 1 {
        return false;
    }
    ctor_args_pass_singleton_b(&decl.ctors[0].typ, decl.params.len(), decl.indices.len())
}

/// Singleton-elim Case B check (D46 §7) for a single-constructor
/// inductive. Walks the ctor's Π-telescope past the parameter prefix;
/// each non-parameter argument must be either:
///
/// - syntactically propositional (inhabits Prop), **or**
/// - **be** one of the conclusion's index expressions — the index is
///   the argument variable itself (D48 Phase H, strict reading; closes
///   finding F-4 of docs/notes/nbe-reorganization-analysis.md).
///
/// The second clause is what admits e.g. `Eq A x y` whose ctor
/// `refl(a) : Eq A a a` has `a` appearing in both index positions —
/// large elim is admissible because the eliminator can reconstruct
/// `a` from the indices of the inductive type.
///
/// For non-indexed decls (`num_indices == 0`), the second clause is
/// vacuous and the check is equivalent to "all args are propositional"
/// — preserving pre-D48 behavior.
fn ctor_args_pass_singleton_b(ctor_typ: &Exp, num_params: usize, num_indices: usize) -> bool {
    // Walk the telescope; collect each non-param arg's (binder name,
    // type). Anonymous binders get an empty name (which never matches
    // a Var lookup, so they can only pass the test if propositional).
    let mut current = ctor_typ;
    let mut remaining_params = num_params;
    let mut non_param_args: Vec<(String, &Exp)> = Vec::new();
    loop {
        match current {
            Exp::Pi(patt, dom, body) => {
                if remaining_params > 0 {
                    remaining_params -= 1;
                } else {
                    let name = match patt {
                        Patt::Var(n) => n.clone(),
                        _ => String::new(),
                    };
                    non_param_args.push((name, dom));
                }
                current = body;
            }
            Exp::SizedPi { body, .. } => {
                // SizedPi binders may appear in the parameter prefix
                // (size-indexed inductives). Skip those; reject any
                // SizedPi appearing as a regular ctor argument since
                // sizes are not propositional and don't constitute
                // "appearing in conclusion" for Case B.
                if remaining_params == 0 {
                    return false;
                }
                remaining_params -= 1;
                current = body;
            }
            _ => break,
        }
    }
    // Extract the conclusion's index expressions (trailing
    // `num_indices` args of the `Exp::InductiveType(_, all_args)`).
    let index_exps: Vec<&Exp> = match current {
        Exp::InductiveType(_, all_args) if all_args.len() >= num_params + num_indices => {
            all_args[num_params..].iter().collect()
        }
        _ => Vec::new(),
    };
    // Each non-param arg must be propositional OR *be* one of the
    // conclusion's index expressions — syntactically `Var(name)` of an
    // unshadowed binder. Membership, not mere mention: the eliminator
    // must recover the arg from the type's indices (D46 §7 Case B;
    // nanoda_lib `large_elim_test_aux`). An index that only mentions
    // the arg (e.g. `f(n)`) does not determine it, and admitting it
    // would let large elimination distinguish proofs that D46 proof
    // irrelevance makes definitionally equal.
    for (i, (name, typ)) in non_param_args.iter().enumerate() {
        let propositional = is_syntactically_propositional_type(typ);
        // A later binder with the same name shadows this one —
        // `Var(name)` in the conclusion then refers to the later arg.
        let shadowed = non_param_args[i + 1..]
            .iter()
            .any(|(later, _)| later == name);
        let in_indices = !name.is_empty()
            && !shadowed
            && index_exps
                .iter()
                .any(|e| matches!(e, Exp::Var(n) if n == name));
        if !propositional && !in_indices {
            return false;
        }
    }
    true
}

/// One binder in a constructor telescope after the parameter prefix
/// has been stripped.
///
/// `Value` is an ordinary Π binder `(p : T)`; `Size` is a bounded
/// size Π binder `{p < upper}` (expressed as `Exp::SizedPi`). The
/// distinction matters because size args are verified against the
/// upper bound via [`crate::nbe::sized::size_lt_with_hyps`] and
/// introduce a hypothesis into the TSO when destructured.
#[derive(Debug, Clone)]
enum CtorArg {
    Value { patt: Patt, typ: Exp },
    Size { patt: Patt, upper: Exp },
}

/// Peel a constructor's Π-telescope past the parameter prefix,
/// returning the remaining binders as `CtorArg`s plus the residual
/// (final) result-type expression.
///
/// Accepts both `Exp::Pi` and `Exp::SizedPi` at non-parameter
/// positions. Parameter positions are always `Exp::Pi` by
/// construction — size parameters have type `SizeSort` but the
/// binder itself is a plain Pi, so `params_to_skip` only ever
/// applies to `Pi`.
/// Validate (D48 Phase B) every ctor's terminal application against the
/// declaration's index telescope.
///
/// For each ctor:
/// 1. Peel the Π-telescope past the parameter prefix, collecting the
///    constructor's value arguments.
/// 2. The terminal residual must be `Exp::InductiveType(d, args)` with
///    `d.name == decl.name` (positivity already checks this) and
///    `args.len() == decl.params.len() + decl.indices.len()`.
/// 3. The last `decl.indices.len()` args are the ctor's index expressions.
///    Each must type-check against the corresponding declared index type
///    (with the parameter prefix substituted), evaluated under a context
///    extended with the param binders and the ctor's non-param args.
///
/// Pre-D48 (non-indexed) declarations have `decl.indices.is_empty()`
/// and this validator is a near-no-op — it only verifies the conclusion
/// arg count equals `decl.params.len()`, which positivity's existing
/// `check_result_type` does not enforce.
pub(super) fn validate_indexed_ctor_conclusions(
    ctx: &mut CheckCtx,
    decl: &InductiveDecl,
) -> Result<(), CheckError> {
    let n_params = decl.params.len();
    let n_indices = decl.indices.len();
    let expected_args = n_params + n_indices;

    for ctor in &decl.ctors {
        // Peel the telescope to get non-param args + the conclusion.
        let (ctor_args, residual) = peel_ctor_telescope(&ctor.typ, n_params);

        // The conclusion must be an InductiveType application of `decl`
        // with the right arg count. Positivity already verified the name
        // matches; we add the arg-count check here.
        let conclusion_args = match residual {
            Exp::InductiveType(d, args) if d.iri == decl.iri => args,
            _ => {
                return Err(CheckError::IllFormed(format!(
                    "constructor `{}.{}`: conclusion must be `{}(...)` — \
                     positivity check should have caught this",
                    decl.name, ctor.name, decl.name
                )));
            }
        };
        if conclusion_args.len() != expected_args {
            return Err(CheckError::IllFormed(format!(
                "constructor `{}.{}`: conclusion `{}(...)` has {} arg(s) \
                 but `{}` declares {} param(s) + {} index/indices = {} total",
                decl.name,
                ctor.name,
                decl.name,
                conclusion_args.len(),
                decl.name,
                n_params,
                n_indices,
                expected_args
            )));
        }

        if n_indices == 0 {
            // Non-indexed decl — no index expressions to type-check.
            // Continue to the next ctor.
            continue;
        }

        // Type-check each index expression against the declared index
        // telescope type. The context is extended with:
        //   (a) the parameter prefix binders (so the index telescope
        //       types may refer to them),
        //   (b) the ctor's non-param value arguments (so index
        //       expressions may refer to them, like `n+1` in
        //       `cons : (n : Nat) → A → Vec A n → Vec A (n+1)`).
        let mut inner_ctx = ctx_with_param_and_arg_binders(ctx, decl, &ctor_args)?;

        // The conclusion's index args sit at conclusion_args[n_params..].
        let index_args = &conclusion_args[n_params..];

        // The declared index telescope's types reference earlier indices
        // in scope; for now D48 v1 supports non-dependent index telescopes
        // (each index's type doesn't reference earlier indices). Walk the
        // telescope and check each index expression.
        for (i, (_idx_patt, idx_type_exp)) in decl.indices.iter().enumerate() {
            let idx_type_val = inner_ctx
                .eval(idx_type_exp, &inner_ctx.rho.clone())
                .map_err(|e| {
                    format!(
                        "constructor `{}.{}`: index #{i} type evaluation failed: {e}",
                        decl.name, ctor.name
                    )
                })?;
            check(&mut inner_ctx, &index_args[i], &idx_type_val).map_err(|e| {
                format!(
                    "constructor `{}.{}`: index #{i} expression doesn't match \
                     declared index telescope type: {e}",
                    decl.name, ctor.name
                )
            })?;
        }
    }

    Ok(())
}

/// Build a CheckCtx extended with the inductive's parameter binders
/// and then the ctor's non-param value arguments. Used by
/// `validate_indexed_ctor_conclusions` so index expressions in a ctor
/// conclusion may refer to both the params and the ctor's value args.
///
/// Size binders (`CtorArg::Size`) bind a variable of type `SizeSort`
/// without a TSO hypothesis — sufficient for type-checking index
/// expressions that mention the size, though such expressions are
/// uncommon in D48 v1.
fn ctx_with_param_and_arg_binders(
    ctx: &CheckCtx,
    decl: &InductiveDecl,
    ctor_args: &[CtorArg],
) -> Result<CheckCtx, CheckError> {
    // Walk the parameter prefix, then the ctor's value/size args,
    // chaining `extend` to produce successive contexts.
    //
    // Note: `extend` returns by value, so we hold each intermediate
    // ctx via `current` (Option) and replace it as we go. We avoid
    // cloning the entire ctx — `extend` already does the right shared-
    // Arc copies for layer / type_cache / size_tso.
    let mut current: Option<CheckCtx> = None;

    for (patt, type_exp) in &decl.params {
        let c: &CheckCtx = current.as_ref().unwrap_or(ctx);
        let typ_val = c.eval(type_exp, &c.rho.clone()).map_err(|e| {
            format!(
                "parameter `{patt:?}` of inductive `{}`: type evaluation failed: {e}",
                decl.name
            )
        })?;
        let gen = gen_val(&c.rho);
        current = Some(c.extend(patt, &typ_val, &gen)?);
    }
    for arg in ctor_args {
        let c: &CheckCtx = current.as_ref().unwrap_or(ctx);
        match arg {
            CtorArg::Value { patt, typ } => {
                let typ_val = c
                    .eval(typ, &c.rho.clone())
                    .map_err(|e| format!("ctor arg `{patt:?}`: type evaluation failed: {e}"))?;
                let gen = gen_val(&c.rho);
                current = Some(c.extend(patt, &typ_val, &gen)?);
            }
            CtorArg::Size { patt, .. } => {
                let gen = gen_val(&c.rho);
                current = Some(c.extend(patt, &Val::SizeSort, &gen)?);
            }
        }
    }
    // If neither the param prefix nor the ctor args extended the ctx
    // (a parameter-less, argument-less ctor), fall back to a fresh
    // child of the outer ctx via a no-op extend on Patt::Unit.
    Ok(current.unwrap_or_else(|| {
        ctx.extend(&Patt::Unit, &Val::One, &Val::Unit)
            .expect("Unit/One extend cannot fail")
    }))
}

fn peel_ctor_telescope(ctor_typ: &Exp, params_to_skip: usize) -> (Vec<CtorArg>, &Exp) {
    let mut args: Vec<CtorArg> = Vec::new();
    let mut remaining = params_to_skip;
    let mut current = ctor_typ;
    loop {
        match current {
            Exp::Pi(patt, dom, body) => {
                if remaining > 0 {
                    remaining -= 1;
                } else {
                    args.push(CtorArg::Value {
                        patt: patt.clone(),
                        typ: (**dom).clone(),
                    });
                }
                current = body;
            }
            Exp::SizedPi { patt, upper, body } => {
                // Size binders appear only after the param prefix.
                args.push(CtorArg::Size {
                    patt: patt.clone(),
                    upper: (**upper).clone(),
                });
                current = body;
            }
            _ => break,
        }
    }
    (args, current)
}

/// Type-check a constructor application's arguments and **return the type it constructs** — the
/// ctor's declared result evaluated under the bound arguments.
///
/// `expected_indices` is `None` in *inference* mode (no expected type to check against) and
/// `Some(idx)` in *checking* mode. This mirrors Lean: `nanoda_lib`'s `infer_app` walks the head's
/// Pi telescope instantiating each argument and returns `inst(fun, ctx)`, with no expected type and
/// no index unification — a constructor there is an ordinary `Const`, so its result type, indices
/// included, simply falls out of substitution. Eigenius keeps constructors as a compound
/// `Exp::InductiveCtor` node, so the equivalent instantiation is `actual_result` below.
///
/// Before this returned a type, the `Exp::InductiveCtor` inference arm passed empty expected
/// indices and answered `indices: []`, which made **every indexed inductive's constructor
/// un-inferable** (`index arity mismatch (actual has N, expected has 0)`) and would have answered
/// with the wrong type had it passed. That is not a corner case: `reasoning:JustifiedBy` is
/// indexed, so no `reasoning:certificate` could pass validation Rule 21 at commit — including the
/// WRN case study's own `chain/04-phase1-recompute-conclusions.esl` (found 2026-08-03).
pub(super) fn check_inductive_ctor_args(
    ctx: &mut CheckCtx,
    decl: &Arc<InductiveDecl>,
    ctor_name: &str,
    args: &[Exp],
    expected_decl: &Arc<InductiveDecl>,
    params: &[Val],
    expected_indices: Option<&[Val]>,
) -> Result<Val, CheckError> {
    if decl.name != expected_decl.name {
        return Err(CheckError::TypeMismatch(format!(
            "InductiveCtor: constructor of `{}` does not match expected inductive `{}`",
            decl.name, expected_decl.name
        )));
    }
    let ctor_idx = decl
        .ctors
        .iter()
        .position(|c| c.name == ctor_name)
        .ok_or_else(|| {
            format!(
                "InductiveCtor: no constructor `{ctor_name}` in `{}`",
                decl.name
            )
        })?;
    let ctor = &decl.ctors[ctor_idx];

    let (arg_specs, current) = peel_ctor_telescope(&ctor.typ, decl.params.len());

    // Permitted arity shapes:
    //
    //   args.len() == arg_specs.len()  — fully specified by the user
    //   args.len() <  arg_specs.len()  — trailing `ChainWitness`-typed
    //                                    slots elided in the surface
    //                                    form. The synthesize hook
    //                                    (`try_synthesize_chain_witness`)
    //                                    populates each missing slot
    //                                    from the layer's witness
    //                                    index. Non-ChainWitness gaps
    //                                    error below.
    //   args.len() >  arg_specs.len()  — error (too many args)
    //
    // The elision is what lets ESL authors write
    // `declared(iri, P)` instead of `declared(iri, P, <sentinel>)`.
    // The synthesize hook never reads the user's expression at a
    // ChainWitness slot, so eliding it is equivalent to providing a
    // sentinel — but with no boilerplate at the call site.
    if args.len() > arg_specs.len() {
        return Err(CheckError::IllFormed(format!(
            "InductiveCtor `{}.{ctor_name}` expects {} args, got {}",
            decl.name,
            arg_specs.len(),
            args.len()
        )));
    }

    // Internal env for evaluating expected types: starts with params
    // bound, then accumulates each checked arg's value.
    let mut arg_env = Rho::Nil;
    for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
        arg_env = arg_env.extend(patt.clone(), val.clone());
    }
    for (i, spec) in arg_specs.iter().enumerate() {
        let user_arg = args.get(i);
        match spec {
            CtorArg::Value { patt, typ } => {
                let arg_typ_val = ctx.eval(typ, &arg_env)?;

                // D49 Phase 6 hook — when the expected arg type is a
                // ChainWitness predicate (`IsDeclaredAs` / `IsObservedAs`
                // / `IsDerivedAs` / `IsVerifiedAs`), synthesize the
                // witness from the layer's witness index rather than
                // type-checking the user's arg. ChainWitness predicates
                // have zero constructors — the user can't construct an
                // inhabitant — so kernel-side synthesis IS the type-
                // checking step here. The user's `arg_exp` (if any) at
                // this position is ignored by design.
                let arg_val = match try_synthesize_chain_witness(ctx, &arg_typ_val)? {
                    Some(witness_val) => witness_val,
                    None => {
                        let arg_exp = user_arg.ok_or_else(|| {
                            format!(
                                "InductiveCtor `{}.{ctor_name}`: arg {i} is missing and \
                                 its expected type is not a ChainWitness predicate. Only \
                                 trailing ChainWitness-typed slots may be elided in the \
                                 surface form.",
                                decl.name
                            )
                        })?;
                        check(ctx, arg_exp, &arg_typ_val)?;
                        ctx.eval(arg_exp, &ctx.rho)?
                    }
                };
                arg_env = arg_env.extend(patt.clone(), arg_val);
            }
            CtorArg::Size { patt, upper } => {
                let arg_exp = user_arg.ok_or_else(|| {
                    format!(
                        "InductiveCtor `{}.{ctor_name}`: sized arg {i} cannot be elided",
                        decl.name
                    )
                })?;
                // Bounded size arg: user's expression must be a
                // size value strictly below the upper bound
                // (evaluated in `arg_env` so it can reference the
                // inductive's size parameter).
                check(ctx, arg_exp, &Val::SizeSort)?;
                let upper_val = ctx.eval(upper, &arg_env)?;
                let arg_val = ctx.eval(arg_exp, &ctx.rho)?;
                if !crate::nbe::sized::size_lt_with_hyps(&arg_val, &upper_val, &ctx.size_tso) {
                    return Err(CheckError::IllFormed(format!(
                        "InductiveCtor `{}.{ctor_name}`: size argument {:?} is not \
                         strictly below upper bound {:?}",
                        decl.name,
                        readback_val(ctx.rho.len(), &arg_val),
                        readback_val(ctx.rho.len(), &upper_val),
                    )));
                }
                arg_env = arg_env.extend(patt.clone(), arg_val);
            }
        }
    }

    // Verify the constructor's declared result type matches the
    // expected inductive type (up to subtyping).
    //
    // For a plain inductive like `cons : Π A:Set. A → List A → List A`
    // this is always trivial — after param binding, `List A` evaluates
    // to `List(A_applied)` which equals the expected type on the nose.
    //
    // For sized inductives it actually bites. A constructor whose
    // declared result is `SizedNat (↑ i)` produces a value whose size
    // is `↑ i_applied`; if the expected size is `i_applied` this check
    // now catches the mismatch (strict-order violation `↑ i ≰ i`).
    // Without this check a buggy constructor declaration of the form
    // `foo : Π p:P. OtherInductive` or `foo : ... → SizedNat (↑ i)`
    // used at `SizedNat i` would pass silently.
    // The constructed type: the ctor's declared result under the bound arguments. This is the
    // answer in inference mode, and the left-hand side of the comparison in checking mode.
    let actual_result = ctx.eval(current, &arg_env)?;
    let Some(expected_indices) = expected_indices else {
        // INFERENCE — nothing to compare against, so neither the result-type subtype check nor the
        // index unification below applies (Lean does neither when inferring). Answer with the type
        // the constructor actually builds, indices and all.
        //
        // Tie the knot first. A ctor's declared result type refers to its own inductive through a
        // SELF-REFERENCE placeholder — the same IRI and name, but built with `ctors: []` (see e.g.
        // `simple_vec_decl` in the check tests). Handing that back verbatim yields a type whose
        // constructor list is empty, and a later `match` on the value then fails with
        // "match arm references unknown constructor". Substitute the full declaration, keeping the
        // indices that were just recovered.
        return Ok(match actual_result {
            Val::InductiveType {
                decl: found,
                params,
                indices,
            } if found.name == decl.name => Val::InductiveType {
                decl: decl.clone(),
                params,
                indices,
            },
            other => other,
        });
    };
    let expected_result = Val::InductiveType {
        decl: expected_decl.clone(),
        params: params.to_vec(),
        indices: expected_indices.to_vec(),
    };
    subtype_of_with_hyps(
        ctx.rho.len(),
        &actual_result,
        &expected_result,
        &ctx.size_tso,
    )
    .map_err(|err| {
        CheckError::TypeMismatch(format!(
            "InductiveCtor `{}.{ctor_name}`: result type mismatch ({err})",
            decl.name
        ))
    })?;

    // D48 Phase D — index unification. `subtype_of_with_hyps`
    // (inductive-param case) only iterates the parameter telescope; it
    // ignores `indices`. For indexed inductives (`decl.indices` non-empty),
    // explicitly unify each actual conclusion index against the
    // corresponding expected index. Failures are reported as
    // "index mismatch" with the structured unification error.
    if !decl.indices.is_empty() {
        let (actual_indices, expected_indices_for_unify): (&[Val], &[Val]) =
            match (&actual_result, &expected_result) {
                (
                    Val::InductiveType { indices: a_idx, .. },
                    Val::InductiveType { indices: e_idx, .. },
                ) => (a_idx.as_slice(), e_idx.as_slice()),
                _ => {
                    unreachable!("actual/expected built above must be Val::InductiveType variants")
                }
            };
        if actual_indices.len() != expected_indices_for_unify.len() {
            return Err(CheckError::IllFormed(format!(
                "InductiveCtor `{}.{ctor_name}`: index arity mismatch \
                 (actual has {}, expected has {})",
                decl.name,
                actual_indices.len(),
                expected_indices_for_unify.len()
            )));
        }
        // Phase D uses a fresh per-call MetaCtx — EigenTT doesn't yet
        // have implicit-arg syntax that would create metas surviving
        // outside ctor checking. Phase F (motive inference) will
        // thread a longer-lived MetaCtx through.
        let mut mctx = crate::nbe::unify::MetaCtx::new();
        for (i, (actual, expected)) in actual_indices
            .iter()
            .zip(expected_indices_for_unify.iter())
            .enumerate()
        {
            crate::nbe::unify::unify(ctx.rho.len(), actual, expected, &mut mctx).map_err(|e| {
                CheckError::TypeMismatch(format!(
                    "InductiveCtor `{}.{ctor_name}`: index #{i} mismatch: {e}",
                    decl.name
                ))
            })?;
        }
    }

    Ok(actual_result)
}

/// Type-check an `Exp::InductiveRec` application and return its result
/// type `motive(major)`.
pub(super) fn check_infer_inductive_rec(
    ctx: &mut CheckCtx,
    decl: &Arc<InductiveDecl>,
    motive: &Exp,
    minors: &[Exp],
    major: &Exp,
) -> Result<Val, CheckError> {
    // 1. Major must inhabit the inductive being eliminated.
    let major_typ = check_infer(ctx, major)?;
    let (major_decl, params) = match &major_typ {
        Val::InductiveType {
            decl: d,
            params: p,
            indices: _,
        } => (d.clone(), p.clone()),
        other => {
            return Err(CheckError::ExpectedInductive(format!(
                "InductiveRec on `{}`: major has type {:?}, expected an inductive type",
                decl.name,
                readback_val(ctx.rho.len(), other)
            )));
        }
    };
    if major_decl.name != decl.name {
        return Err(CheckError::TypeMismatch(format!(
            "InductiveRec: declaration mismatch — recursor for `{}`, major has type `{}`",
            decl.name, major_decl.name
        )));
    }

    // 2. Motive : I(params) → Sort(<codomain>).
    //    For non-Prop inductives, codomain is Sort(2) — any sort body
    //    is admitted via cumulativity (Set, Type(n) all inhabit Sort(2)).
    //    For Prop inductives, singleton-elim (D46 §7) gates large elim:
    //    if `large_elim_admitted(decl)` then any sort is permitted;
    //    otherwise the motive must return Prop (Sort(0)).
    let codomain_sort = if matches!(decl.sort, Exp::Sort(0)) && !large_elim_admitted(decl) {
        Exp::Sort(0)
    } else {
        Exp::Sort(2)
    };
    let motive_dom = Val::InductiveType {
        decl: decl.clone(),
        params: params.clone(),
        indices: Vec::new(),
    };
    let motive_typ = Val::Pi(
        Box::new(motive_dom),
        Clos::new(Patt::Unit, codomain_sort, Rho::Nil),
    );
    check(ctx, motive, &motive_typ).map_err(|e| {
        if matches!(decl.sort, Exp::Sort(0)) && !large_elim_admitted(decl) {
            CheckError::IllFormed(format!(
                "singleton-elim violation: recursor on `{}` (a Prop with {} \
                 ctor{}, failing the singleton test) requires a Prop-valued \
                 motive; got: {e}",
                decl.name,
                decl.ctors.len(),
                if decl.ctors.len() == 1 { "" } else { "s" }
            ))
        } else {
            e
        }
    })?;

    // 3. Minors: one per constructor, each against its derived type.
    if minors.len() != decl.ctors.len() {
        return Err(CheckError::IllFormed(format!(
            "InductiveRec on `{}`: expected {} minors (one per constructor), got {}",
            decl.name,
            decl.ctors.len(),
            minors.len()
        )));
    }
    let motive_val = ctx.eval(motive, &ctx.rho)?;
    let expected_minor_types = derive_minor_types(decl, &params, &motive_val, &EvalCtx::Pure)?;
    for (minor, expected_typ) in minors.iter().zip(expected_minor_types.iter()) {
        check(ctx, minor, expected_typ)?;
    }

    // 4. Result: motive(major).
    let major_val = ctx.eval(major, &ctx.rho)?;
    motive_val.app(major_val).map_err(CheckError::from)
}

/// Type-check `match scrutinee { arm₁; arm₂; … }` against an expected
/// result type (Phase 11b step 12, D19 §10).
///
/// 1. Infer the scrutinee's type — must be `Val::InductiveType { decl, params }`.
/// 2. Validate exhaustiveness (every constructor in `decl` has an arm)
///    and no duplicate arms.
/// 3. For each arm, build a context extended with bindings for the
///    constructor's positional arguments (with parameters substituted),
///    then check the arm body against `expected_type`. Binding count
///    must match the constructor's arity.
///
/// The motive synthesised by this check is the constant function
/// `λ_. expected_type`, so each arm body is checked at the same type.
/// Dependent motives (where the result type varies with the matched
/// constructor) need explicit annotation via `Exp::InductiveRec` and
/// are not handled by this path.
pub(super) fn check_match(
    ctx: &mut CheckCtx,
    scrutinee: &Exp,
    arms: &[crate::nbe::term::MatchArm],
    expected: &Val,
) -> Result<(), CheckError> {
    use std::collections::BTreeMap;

    let scrutinee_type = check_infer(ctx, scrutinee)?;
    let (decl, params, scrutinee_indices) = match &scrutinee_type {
        Val::InductiveType {
            decl,
            params,
            indices,
        } => (decl.clone(), params.clone(), indices.clone()),
        other => {
            return Err(CheckError::ExpectedInductive(format!(
                "match scrutinee has type {:?}, expected an inductive type",
                readback_val(ctx.rho.len(), other)
            )));
        }
    };

    let mut arms_by_ctor: BTreeMap<&str, &crate::nbe::term::MatchArm> = BTreeMap::new();
    for arm in arms {
        if arms_by_ctor.insert(arm.ctor_name.as_str(), arm).is_some() {
            return Err(CheckError::IllFormed(format!(
                "duplicate match arm for `{}.{}`",
                decl.name, arm.ctor_name
            )));
        }
    }
    for ctor_name in arms_by_ctor.keys() {
        if !decl.ctors.iter().any(|c| &c.name == ctor_name) {
            return Err(CheckError::IllFormed(format!(
                "match arm references unknown constructor `{}.{ctor_name}`",
                decl.name
            )));
        }
    }

    // Singleton-elim (D46 §7): a Prop-typed inductive that fails the
    // singleton test cannot be matched into a non-Prop result type.
    if matches!(decl.sort, Exp::Sort(0))
        && !large_elim_admitted(&decl)
        && !is_propositional_in_ctx(ctx, expected)?
    {
        return Err(CheckError::IllFormed(format!(
            "singleton-elim violation: match on `{}` (a Prop with {} \
             ctor{}, failing the singleton test) requires a Prop-valued \
             result type",
            decl.name,
            decl.ctors.len(),
            if decl.ctors.len() == 1 { "" } else { "s" }
        )));
    }

    for ctor in &decl.ctors {
        let arm = arms_by_ctor.get(ctor.name.as_str()).ok_or_else(|| {
            format!(
                "non-exhaustive match: missing case for `{}.{}`",
                decl.name, ctor.name
            )
        })?;

        // Extract this constructor's argument types (after the
        // parameter prefix) from its Π-telescope. Supports both
        // ordinary `Pi` binders and bounded-size `SizedPi` binders;
        // size binders become rigid hypotheses in the arm's TSO.
        let (arg_specs, _ctor_result) = peel_ctor_telescope(&ctor.typ, decl.params.len());

        if arm.bindings.len() != arg_specs.len() {
            return Err(CheckError::IllFormed(format!(
                "match arm `{}.{}` expects {} bindings, got {}",
                decl.name,
                ctor.name,
                arg_specs.len(),
                arm.bindings.len()
            )));
        }

        // Build the arm's check context: start from the outer ctx,
        // bind parameters for evaluating arg types, then extend with
        // each binding (bound to a fresh generic value of the
        // corresponding arg type).
        let mut arg_env = Rho::Nil;
        for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
            arg_env = arg_env.extend(patt.clone(), val.clone());
        }
        let mut arm_ctx = CheckCtx {
            rho: ctx.rho.clone(),
            gamma: ctx.gamma.clone(),
            layer: ctx.layer.clone(),
            type_cache: ctx.type_cache.clone(),
            size_tso: ctx.size_tso.clone(),
            institution_index: ctx.institution_index.clone(),
            institution_runtime: ctx.institution_runtime.clone(),
            hooks: ctx.hooks.clone(),
        };
        for (spec, binding) in arg_specs.iter().zip(arm.bindings.iter()) {
            match spec {
                CtorArg::Value { patt, typ } => {
                    let arg_typ_val = ctx.eval(typ, &arg_env)?;
                    let gen = gen_val(&arm_ctx.rho);
                    arm_ctx = arm_ctx.extend(binding, &arg_typ_val, &gen)?;
                    arg_env = arg_env.extend(patt.clone(), gen);
                }
                CtorArg::Size { patt, upper } => {
                    // The constructor's bounded size binder exposes
                    // the predecessor size in the arm's scope, with
                    // `bound_size < upper` available as a TSO
                    // hypothesis. This is what lets a recursive call
                    // on the destructured sub-value type-check at a
                    // strictly-smaller size — i.e. termination via
                    // pattern-match on a sized inductive.
                    let upper_val = ctx.eval(upper, &arg_env)?;
                    let new_level = arm_ctx.rho.len();
                    let gen = gen_val(&arm_ctx.rho);
                    arm_ctx = arm_ctx.extend(binding, &Val::SizeSort, &gen)?;
                    match &upper_val {
                        Val::SizeInf => {
                            // `{j < ∞}` in a ctor adds no hypothesis
                            // — anything is below ∞ structurally.
                        }
                        Val::Nt(crate::nbe::val::Neut::Gen(upper_level, _)) => {
                            arm_ctx
                                .size_tso
                                .insert(new_level as u32, 1, *upper_level as u32);
                        }
                        _ => {
                            return Err(CheckError::IllFormed(format!(
                                "match arm `{}.{}`: constructor's bounded size binder upper \
                                 must be rigid or ∞, got {:?}",
                                decl.name,
                                ctor.name,
                                readback_val(ctx.rho.len(), &upper_val),
                            )));
                        }
                    }
                    arg_env = arg_env.extend(patt.clone(), gen);
                }
            }
        }

        // D48 Phase F — index-coherence check.
        //
        // For an indexed decl, this arm's ctor produces a conclusion
        // `D(params)(ctor_idx_1, …, ctor_idx_m)` where each `ctor_idx_k`
        // is an expression that may reference the ctor's value
        // arguments. Evaluate these under `arg_env` (which has the
        // params and the arm's bindings bound) and unify each against
        // the scrutinee's corresponding index value. If unification
        // fails, this arm is unreachable per the scrutinee's index
        // shape — the user wrote (e.g.) a `nil` arm on `Vec A 1`.
        //
        // For non-indexed decls (`decl.indices.is_empty()`), this is a
        // no-op — scrutinee_indices is empty and the loop body never
        // runs.
        if !decl.indices.is_empty() {
            // Evaluate ctor's conclusion. _ctor_result was discarded
            // above; re-peel to get it.
            let (_arg_specs_recheck, ctor_result) =
                peel_ctor_telescope(&ctor.typ, decl.params.len());
            let actual_conclusion = arm_ctx.eval(ctor_result, &arg_env)?;
            let actual_indices: &[Val] = match &actual_conclusion {
                Val::InductiveType { indices, .. } => indices.as_slice(),
                _ => {
                    return Err(CheckError::IllFormed(format!(
                        "match arm `{}.{}`: ctor conclusion did not evaluate \
                         to an inductive type",
                        decl.name, ctor.name
                    )));
                }
            };
            if actual_indices.len() != scrutinee_indices.len() {
                return Err(CheckError::IllFormed(format!(
                    "match arm `{}.{}`: index arity mismatch \
                     (ctor produces {}, scrutinee has {})",
                    decl.name,
                    ctor.name,
                    actual_indices.len(),
                    scrutinee_indices.len()
                )));
            }
            let mut mctx = crate::nbe::unify::MetaCtx::new();
            for (i, (actual, expected_idx)) in actual_indices
                .iter()
                .zip(scrutinee_indices.iter())
                .enumerate()
            {
                crate::nbe::unify::unify(arm_ctx.rho.len(), actual, expected_idx, &mut mctx)
                    .map_err(|e| {
                        format!(
                            "match arm `{}.{}` is unreachable: ctor's index #{i} \
                         doesn't match scrutinee's index ({e}). If this arm \
                         should be reachable under a dependent motive, use \
                         `Exp::InductiveRec` with an explicit `returning T` \
                         annotation.",
                            decl.name, ctor.name
                        )
                    })?;
            }
        }

        check(&mut arm_ctx, &arm.body, expected)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::check::testutil::*;
    use crate::nbe::check::*;
    // ---------- D46 §7 — singleton-elim tests ----------

    fn mk_prop_decl(
        name: &str,
        ctors: Vec<crate::nbe::term::InductiveCtorDecl>,
    ) -> crate::nbe::term::InductiveDecl {
        crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
            name: name.to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(0),
            ctors,
        }
    }

    #[test]
    fn large_elim_zero_ctors_admitted() {
        // False : Prop with zero ctors — Case A.
        let decl = mk_prop_decl("False", Vec::new());
        assert!(large_elim_admitted(&decl));
    }

    #[test]
    fn large_elim_multi_ctor_rejected() {
        // Multi-ctor Prop — Case B requires exactly one ctor; rejected.
        let decl = mk_prop_decl(
            "Either2",
            vec![
                crate::nbe::term::InductiveCtorDecl {
                    name: "left".to_string(),
                    typ: Exp::EigonClass(
                        crate::ontology::iri::Iri::parse("urn:_:Either2").unwrap(),
                    ),
                },
                crate::nbe::term::InductiveCtorDecl {
                    name: "right".to_string(),
                    typ: Exp::EigonClass(
                        crate::ontology::iri::Iri::parse("urn:_:Either2").unwrap(),
                    ),
                },
            ],
        );
        assert!(!large_elim_admitted(&decl));
    }

    #[test]
    fn large_elim_single_ctor_all_prop_args_admitted() {
        // SingleProp { mk : Id(1, (), ()) → SingleProp } — ctor arg is Id (Prop).
        let id_arg = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let conclusion =
            Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:SingleProp").unwrap());
        let ctor_typ = Exp::Pi(Patt::Unit, Box::new(id_arg), Box::new(conclusion));
        let decl = mk_prop_decl(
            "SingleProp",
            vec![crate::nbe::term::InductiveCtorDecl {
                name: "mk".to_string(),
                typ: ctor_typ,
            }],
        );
        assert!(large_elim_admitted(&decl));
    }

    #[test]
    fn large_elim_single_ctor_with_non_prop_arg_rejected() {
        // BadProp { mk : 1 → BadProp } — ctor arg is `1 : Set`, not in Prop.
        let conclusion =
            Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:BadProp").unwrap());
        let ctor_typ = Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(conclusion));
        let decl = mk_prop_decl(
            "BadProp",
            vec![crate::nbe::term::InductiveCtorDecl {
                name: "mk".to_string(),
                typ: ctor_typ,
            }],
        );
        assert!(!large_elim_admitted(&decl));
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 Phase H — singleton-elim Case B "arg appears in conclusion"
    // ──────────────────────────────────────────────────────────────────

    /// `Eq A x y` (the canonical motivating case for D48 Phase H's
    /// extension to singleton-elim Case B). Indexed by two values of
    /// type A; single ctor `refl(a) : Eq A a a` has `a` appearing in
    /// both index positions.
    ///
    /// Built as a Prop-sorted indexed inductive with one param (A : Set)
    /// and two indices of type A (both unbound type-parameter
    /// references — but for the singleton-elim test we just need the
    /// shape, so the index telescope uses `Exp::Var("A")` referring to
    /// the param).
    fn eq_decl() -> std::sync::Arc<crate::nbe::term::InductiveDecl> {
        // Self-ref for the ctor's conclusion.
        let self_ref = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Eq").unwrap(),
            name: "Eq".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: vec![
                (Patt::Var("x".to_string()), Exp::Var("A".to_string())),
                (Patt::Var("y".to_string()), Exp::Var("A".to_string())),
            ],
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        });
        // refl(a) : Eq A a a — conclusion supplies `a` in both indices.
        let conclusion = Exp::InductiveType(
            self_ref.clone(),
            vec![
                Exp::Var("A".to_string()),
                Exp::Var("a".to_string()),
                Exp::Var("a".to_string()),
            ],
        );
        let ctor_typ = Exp::Pi(
            Patt::Var("A".to_string()),
            Box::new(Exp::Sort(1)),
            Box::new(Exp::Pi(
                Patt::Var("a".to_string()),
                Box::new(Exp::Var("A".to_string())),
                Box::new(conclusion),
            )),
        );
        std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Eq").unwrap(),
            name: "Eq".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: vec![
                (Patt::Var("x".to_string()), Exp::Var("A".to_string())),
                (Patt::Var("y".to_string()), Exp::Var("A".to_string())),
            ],
            sort: Exp::Sort(0),
            ctors: vec![crate::nbe::term::InductiveCtorDecl {
                name: "refl".to_string(),
                typ: ctor_typ,
            }],
        })
    }

    #[test]
    fn d48_singleton_elim_admits_eq_via_indices_in_conclusion() {
        // `Eq`'s `refl(a)` has a non-Prop arg `a : A` that appears in
        // both conclusion indices. Pre-D48 this failed singleton-elim
        // Case B (no indices => "appears in conclusion" was vacuous).
        // With D48 Phase H, the extended Case B admits it.
        let decl = eq_decl();
        assert!(
            large_elim_admitted(&decl),
            "Eq must admit large elim under D48 Phase H — refl's `a` arg appears in indices"
        );
    }

    #[test]
    fn d48_singleton_elim_still_rejects_arg_not_in_conclusion() {
        // A synthetic Prop-sorted indexed inductive whose single ctor
        // takes a non-Prop arg that does NOT appear in the conclusion's
        // index expressions. Even with the Phase H extension, this
        // should still be rejected.
        let self_ref = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:BadIxProp").unwrap(),
            name: "BadIxProp".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        });
        // Conclusion: BadIxProp () — the index is the constant `()`,
        // not mentioning any ctor arg.
        let conclusion = Exp::InductiveType(self_ref.clone(), vec![Exp::Unit]);
        // Ctor: takes a non-Prop arg `_:1` (Unit type, in Set) that
        // doesn't appear in conclusion.
        let ctor_typ = Exp::Pi(
            Patt::Var("smuggled".to_string()),
            Box::new(Exp::One),
            Box::new(conclusion),
        );
        let decl = crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:BadIxProp").unwrap(),
            name: "BadIxProp".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(0),
            ctors: vec![crate::nbe::term::InductiveCtorDecl {
                name: "smuggle".to_string(),
                typ: ctor_typ,
            }],
        };
        assert!(
            !large_elim_admitted(&decl),
            "BadIxProp must NOT admit large elim — the non-Prop arg doesn't appear in indices"
        );
    }

    #[test]
    fn d48_singleton_elim_unchanged_for_non_indexed_props() {
        // Without indices, the Phase H extension is vacuous — the
        // pre-D46 behavior holds: every non-param arg must be
        // syntactically propositional.
        // (Re-asserts the existing single-ctor-with-Id-arg case
        // to catch any Phase H regression.)
        let id_arg = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let conclusion =
            Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:SingleProp").unwrap());
        let ctor_typ = Exp::Pi(Patt::Unit, Box::new(id_arg), Box::new(conclusion));
        let decl = mk_prop_decl(
            "SingleProp",
            vec![crate::nbe::term::InductiveCtorDecl {
                name: "mk".to_string(),
                typ: ctor_typ,
            }],
        );
        assert!(large_elim_admitted(&decl));
    }

    /// Closes finding F-4 (port-fidelity analysis,
    /// docs/notes/nbe-reorganization-analysis.md §4): singleton-elim
    /// Case B requires each non-Prop ctor arg to *be* one of the
    /// conclusion's indices (set membership, matching nanoda's
    /// `large_elim_test_aux` @ f58f2f6) — an index that merely
    /// *mentions* the arg does not determine it, so large elim is not
    /// admitted.
    #[test]
    fn singleton_elim_rejects_index_that_only_mentions_arg() {
        // P : 1 → Prop with ctor `mk : (n : 1) → P (n, ())` — the index
        // expression `(n, ())` mentions `n` but is not `n` itself.
        let self_ref = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:MentionsIx").unwrap(),
            name: "MentionsIx".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        });
        let index_exp = Exp::Pair(Box::new(Exp::Var("n".to_string())), Box::new(Exp::Unit));
        let conclusion = Exp::InductiveType(self_ref, vec![index_exp]);
        let ctor_typ = Exp::Pi(
            Patt::Var("n".to_string()),
            Box::new(Exp::One), // non-propositional per is_syntactically_propositional_type
            Box::new(conclusion),
        );
        let decl = crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:MentionsIx").unwrap(),
            name: "MentionsIx".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(0),
            ctors: vec![crate::nbe::term::InductiveCtorDecl {
                name: "mk".to_string(),
                typ: ctor_typ,
            }],
        };
        assert!(
            !large_elim_admitted(&decl),
            "an index that merely mentions the arg must not admit large elim"
        );
    }

    /// F-4 companion: an arg whose name is rebound by a later binder is
    /// not recoverable through `Var(name)` — the conclusion index
    /// refers to the later binder.
    #[test]
    fn singleton_elim_rejects_shadowed_arg_reference() {
        // P : 1 → Prop with ctor `mk : (n : 1) → (n : Id(1,(),())) → P n`
        // — the index `n` refers to the SECOND (propositional) binder;
        // the first, non-Prop `n` is shadowed and unrecoverable.
        let self_ref = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:ShadowIx").unwrap(),
            name: "ShadowIx".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        });
        let conclusion = Exp::InductiveType(self_ref, vec![Exp::Var("n".to_string())]);
        let id_typ = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let ctor_typ = Exp::Pi(
            Patt::Var("n".to_string()),
            Box::new(Exp::One), // non-Prop, shadowed below
            Box::new(Exp::Pi(
                Patt::Var("n".to_string()),
                Box::new(id_typ), // propositional
                Box::new(conclusion),
            )),
        );
        let decl = crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:ShadowIx").unwrap(),
            name: "ShadowIx".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(0),
            ctors: vec![crate::nbe::term::InductiveCtorDecl {
                name: "mk".to_string(),
                typ: ctor_typ,
            }],
        };
        assert!(
            !large_elim_admitted(&decl),
            "a shadowed arg is not recoverable from the indices"
        );
    }

    /// Closes finding F-3 (port-fidelity analysis,
    /// docs/notes/nbe-reorganization-analysis.md §4): a constructor
    /// conclusion that instantiates the block parameter to something
    /// other than the parameter itself is rejected at declaration
    /// checking, matching nanoda's `check_ctor` → `is_valid_ind_app`.
    #[test]
    fn rejects_nonuniform_conclusion_params() {
        // Q(A : Set) { mk : Q(1) } — conclusion `Q(1)`, not `Q(A)`.
        let s = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Q").unwrap(),
            name: "Q".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let decl = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Q").unwrap(),
            name: "Q".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![crate::nbe::term::InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(Exp::InductiveType(s, vec![Exp::One])),
                ),
            }],
        });
        let mut ctx = CheckCtx::new(Rho::Nil, Vec::new());
        let err = check_type(&mut ctx, &Exp::Inductive(decl))
            .expect_err("non-uniform conclusion params")
            .to_string();
        assert!(err.contains("parameters through unchanged"), "got: {err}");
    }

    #[test]
    fn large_elim_does_not_apply_to_non_prop_inductives() {
        // A Set-sorted inductive isn't subject to the singleton restriction
        // at all — large_elim_admitted is only consulted for Prop decls.
        // Smoke-test the function returns sensibly regardless.
        let set_decl = crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
            name: "Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                crate::nbe::term::InductiveCtorDecl {
                    name: "zero".to_string(),
                    typ: Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap()),
                },
                crate::nbe::term::InductiveCtorDecl {
                    name: "succ".to_string(),
                    typ: Exp::Pi(
                        Patt::Unit,
                        Box::new(Exp::EigonClass(
                            crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap(),
                        )),
                        Box::new(Exp::EigonClass(
                            crate::ontology::iri::Iri::parse("urn:_:Nat").unwrap(),
                        )),
                    ),
                },
            ],
        };
        // For a non-Prop inductive the singleton test is not load-bearing,
        // but the algorithm still runs correctly: Nat has 2 ctors, so the
        // test returns false (as it would for any 2-ctor Prop).
        assert!(!large_elim_admitted(&set_decl));
    }

    // --- Inductive type checking (Phase 11b step 5) ---

    use crate::nbe::term::InductiveCtorDecl;

    fn nat_succ_exp(decl: &Arc<InductiveDecl>, n: Exp) -> Exp {
        Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![n])
    }

    /// Constant `λ_. Set` motive — applied to anything yields `Set`.
    fn const_set_motive_exp() -> Exp {
        Exp::Lam(Patt::Unit, Box::new(Exp::Sort(1)))
    }

    #[test]
    fn check_ctor_zero_against_nat_type() {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check(&mut c, &nat_zero_exp(&nat), &nat_ty).expect("zero : Nat");
    }

    #[test]
    fn check_ctor_succ_zero_against_nat_type() {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        let exp = nat_succ_exp(&nat, nat_zero_exp(&nat));
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        check(&mut c, &exp, &nat_ty).expect("succ zero : Nat");
    }

    #[test]
    fn check_ctor_arg_type_mismatch() {
        // succ Set should fail because Set : Type, not Nat.
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        let bogus = Exp::InductiveCtor(nat.clone(), "succ".to_string(), vec![Exp::Sort(1)]);
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        assert!(check(&mut c, &bogus, &nat_ty).is_err());
    }

    #[test]
    fn check_ctor_unknown_constructor_name() {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        let bogus = Exp::InductiveCtor(nat.clone(), "two".to_string(), Vec::new());
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let err = check(&mut c, &bogus, &nat_ty).unwrap_err().to_string();
        assert!(err.contains("no constructor"), "unexpected: {err}");
    }

    #[test]
    fn check_ctor_wrong_decl_against_other_inductive() {
        // Construct a Bool decl, then try to type-check Bool's True against Nat.
        let nat = nat_decl();
        let bs = ind_self_ref("Bool");
        let bool_ty_exp = Exp::InductiveType(bs, Vec::new());
        let bool_decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Bool").unwrap(),
            name: "Bool".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "True".to_string(),
                typ: bool_ty_exp,
            }],
        });
        let true_exp = Exp::InductiveCtor(bool_decl, "True".to_string(), Vec::new());
        let nat_ty = Val::InductiveType {
            decl: nat,
            params: Vec::new(),
            indices: Vec::new(),
        };
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let err = check(&mut c, &true_exp, &nat_ty).unwrap_err().to_string();
        assert!(err.contains("does not match"), "unexpected: {err}");
    }

    #[test]
    fn infer_ctor_succeeds_for_non_parametric_inductive() {
        // Nat has no params → inference returns InductiveType{Nat, []}.
        let nat = nat_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let typ = check_infer(&mut c, &nat_zero_exp(&nat)).expect("infer Nat.zero");
        match typ {
            Val::InductiveType {
                decl,
                params,
                indices: _,
            } => {
                assert_eq!(decl.name, "Nat");
                assert!(params.is_empty());
            }
            other => panic!("expected InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn infer_ctor_fails_for_parametric_inductive() {
        let s = ind_self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list_decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:List").unwrap(),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(list_ty),
                ),
            }],
        });
        let nil_exp = Exp::InductiveCtor(list_decl, "nil".to_string(), Vec::new());
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let err = check_infer(&mut c, &nil_exp).unwrap_err().to_string();
        assert!(err.contains("checking mode"), "unexpected: {err}");
    }

    /// Build a `CheckCtx` with `n : Nat` bound (gamma + rho).
    fn ctx_with_nat_var() -> (Arc<InductiveDecl>, CheckCtx) {
        let nat = nat_decl();
        let nat_ty = Val::InductiveType {
            decl: nat.clone(),
            params: Vec::new(),
            indices: Vec::new(),
        };
        let nat_val = Val::InductiveVal {
            decl: nat.clone(),
            ctor_name: "zero".to_string(),
            args: Vec::new(),
        };
        let gamma: Gamma = vec![("n".to_string(), nat_ty)];
        let rho = Rho::Nil.extend(Patt::Var("n".to_string()), nat_val);
        (nat, CheckCtx::new(rho, gamma))
    }

    #[test]
    fn infer_rec_well_typed() {
        // Nat.rec (λ_. Set) Nat (λ_n. λ_ih. Nat) n   (motive constant Set)
        // Motive : Nat → Set, zero minor : Set, succ minor : Nat → Set → Set,
        // result type: Set.
        let (nat, mut c) = ctx_with_nat_var();
        let nat_ty_exp = Exp::InductiveType(nat.clone(), Vec::new());
        let succ_minor = Exp::Lam(
            Patt::Unit,
            Box::new(Exp::Lam(Patt::Unit, Box::new(nat_ty_exp.clone()))),
        );
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![nat_ty_exp, succ_minor],
            major: Box::new(Exp::Var("n".to_string())),
        };
        let typ = check_infer(&mut c, &exp).expect("Nat.rec well-typed");
        assert!(matches!(typ, Val::Sort(1)), "expected Set, got {typ:?}");
    }

    #[test]
    fn infer_rec_wrong_minor_count() {
        let (nat, mut c) = ctx_with_nat_var();
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![Exp::InductiveType(nat_decl(), Vec::new())], // only 1 minor, needs 2
            major: Box::new(Exp::Var("n".to_string())),
        };
        let err = check_infer(&mut c, &exp).unwrap_err().to_string();
        assert!(err.contains("expected 2 minors"), "unexpected: {err}");
    }

    #[test]
    fn infer_rec_minor_type_mismatch() {
        // Wrong type for the zero minor — supply Unit instead of a Set.
        let (nat, mut c) = ctx_with_nat_var();
        let nat_ty_exp = Exp::InductiveType(nat.clone(), Vec::new());
        let succ_minor = Exp::Lam(
            Patt::Unit,
            Box::new(Exp::Lam(Patt::Unit, Box::new(nat_ty_exp))),
        );
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![Exp::Unit, succ_minor],
            major: Box::new(Exp::Var("n".to_string())),
        };
        assert!(check_infer(&mut c, &exp).is_err());
    }

    #[test]
    fn infer_rec_major_wrong_type() {
        // Major has type 1 (One), not Nat — must fail with the inductive-type message.
        let nat = nat_decl();
        let nat_ty_exp = Exp::InductiveType(nat.clone(), Vec::new());
        let succ_minor = Exp::Lam(
            Patt::Unit,
            Box::new(Exp::Lam(Patt::Unit, Box::new(nat_ty_exp.clone()))),
        );
        let exp = Exp::InductiveRec {
            decl: nat,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![nat_ty_exp, succ_minor],
            major: Box::new(Exp::Var("u".to_string())),
        };
        let gamma: Gamma = vec![("u".to_string(), Val::One)];
        let rho = Rho::Nil.extend(Patt::Var("u".to_string()), Val::Unit);
        let mut c = CheckCtx::new(rho, gamma);
        let err = check_infer(&mut c, &exp).unwrap_err().to_string();
        assert!(
            err.contains("expected an inductive type"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn infer_rec_decl_mismatch() {
        // n : Nat but recursor uses Bool decl.
        let (_nat, mut c) = ctx_with_nat_var();
        let bs = ind_self_ref("Bool");
        let bool_ty = Exp::InductiveType(bs, Vec::new());
        let bool_decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Bool").unwrap(),
            name: "Bool".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "True".to_string(),
                    typ: bool_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "False".to_string(),
                    typ: bool_ty.clone(),
                },
            ],
        });
        let exp = Exp::InductiveRec {
            decl: bool_decl,
            motive: Box::new(const_set_motive_exp()),
            minors: vec![bool_ty.clone(), bool_ty],
            major: Box::new(Exp::Var("n".to_string())),
        };
        let err = check_infer(&mut c, &exp).unwrap_err().to_string();
        assert!(err.contains("declaration mismatch"), "unexpected: {err}");
    }

    // --- Sized inductive termination via Match (Phase 11b step 15g) ---
    //
    // A proper sized Nat whose `succ` constructor uses `SizedPi` for
    // its predecessor size, so pattern-matching on `succ(j, n)`
    // introduces `j < i` as a TSO hypothesis in the arm — the
    // hypothesis that lets recursive calls on `n` type-check as
    // strictly-decreasing.

    fn sized_nat_with_sized_pi_decl() -> Arc<InductiveDecl> {
        // SizedNatP(i : SizeSort) with
        //   zero : Π i:SizeSort. SizedNatP i
        //   succ : Π i:SizeSort. {j < i}. SizedNatP j → SizedNatP i
        let self_ref = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:SizedNatP").unwrap(),
            name: "SizedNatP".to_string(),
            params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let snat_i = Exp::InductiveType(self_ref.clone(), vec![Exp::Var("i".to_string())]);
        let snat_j = Exp::InductiveType(self_ref, vec![Exp::Var("j".to_string())]);
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:SizedNatP").unwrap(),
            name: "SizedNatP".to_string(),
            params: vec![(Patt::Var("i".to_string()), Exp::SizeSort)],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "zero".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("i".to_string()),
                        Box::new(Exp::SizeSort),
                        Box::new(snat_i.clone()),
                    ),
                },
                InductiveCtorDecl {
                    name: "succ".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("i".to_string()),
                        Box::new(Exp::SizeSort),
                        Box::new(Exp::SizedPi {
                            patt: Patt::Var("j".to_string()),
                            upper: Box::new(Exp::Var("i".to_string())),
                            body: Box::new(Exp::Pi(Patt::Unit, Box::new(snat_j), Box::new(snat_i))),
                        }),
                    ),
                },
            ],
        })
    }

    #[test]
    fn sized_nat_p_succ_at_inf_with_equal_predecessor() {
        // Under expected type `SizedNatP ∞`, check
        // `succ(size=∞, n=zero)`. The outer param `i=∞` is provided
        // by the expected type; user supplies only the non-param
        // args (size + value). size_lt(∞, ∞) holds via ∞-absorption.
        let decl = sized_nat_with_sized_pi_decl();
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let succ_inf =
            Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![Exp::SizeInf, zero]);
        let ty = Val::InductiveType {
            decl,
            params: vec![Val::SizeInf],
            indices: Vec::new(),
        };
        check(&mut c, &succ_inf, &ty).expect("succ(∞, zero) : SizedNatP ∞");
    }

    #[test]
    fn sized_nat_p_succ_with_non_decreasing_size_rejected() {
        // Under `i : SizeSort` and expected `SizedNatP i`, the
        // expression `succ(size=i, n=zero)` must be rejected: the
        // predecessor size `i` is not strictly below the outer `i`.
        let decl = sized_nat_with_sized_pi_decl();
        let (mut c, i_val) = ctx_with_size_var("i");
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let bad = Exp::InductiveCtor(
            decl.clone(),
            "succ".to_string(),
            vec![Exp::Var("i".to_string()), zero],
        );
        let ty = Val::InductiveType {
            decl,
            params: vec![i_val],
            indices: Vec::new(),
        };
        let err = check(&mut c, &bad, &ty).unwrap_err().to_string();
        assert!(
            err.contains("not strictly below"),
            "expected size-bound error, got: {err}"
        );
    }

    #[test]
    fn sized_nat_p_match_arm_sees_hypothesis() {
        // The key termination-by-typing test.
        //
        // Given `i : SizeSort` and `x : SizedNatP(i)`, match on x.
        // In the `succ(j, n)` arm:
        //   - `j : SizeSort` is a fresh rigid with TSO `j < i`
        //   - `n : SizedNatP(j)` (strictly smaller inductive)
        //
        // The arm body checks `n : SizedNatP(i)` — which requires
        // `SizedNatP(j) <: SizedNatP(i)`, i.e. `j ≤ i`. From the
        // TSO hypothesis `j < i`, subtyping derives `j ≤ i`. ✓
        //
        // Without the hypothesis, this subtyping fails.
        let decl = sized_nat_with_sized_pi_decl();
        let (c, i_val) = ctx_with_size_var("i");

        let snatp_i = Val::InductiveType {
            decl: decl.clone(),
            params: vec![i_val.clone()],
            indices: Vec::new(),
        };
        let x_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("x".to_string()), &snatp_i, &x_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        // match x { zero => x; succ(j, n) => n }
        // Expected type: SizedNatP(i). Both arms must produce that.
        // succ arm bindings are (j, n) — the non-param ctor args.
        // `j : SizeSort` gets TSO hypothesis `j < i`; `n : SizedNatP(j)`.
        // The arm body is `n`, which under subtyping lifts into
        // SizedNatP(i) via the hypothesis.
        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("x".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "zero".to_string(),
                    bindings: vec![],
                    body: Exp::Var("x".to_string()),
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "succ".to_string(),
                    bindings: vec![Patt::Var("j".to_string()), Patt::Var("n".to_string())],
                    body: Exp::Var("n".to_string()),
                },
            ],
        };
        check(&mut c2, &match_exp, &snatp_i)
            .expect("match arm with succ(j, n) uses hypothesis j < i to lift n into SizedNatP(i)");
    }

    #[test]
    fn sized_nat_p_match_arm_without_hypothesis_usage_still_typechecks() {
        // The OLD `sized_nat_decl` (plain Pi, no SizedPi) gives
        // `succ` a single non-param arg of type `SizedNat(i)` —
        // i.e. the predecessor shares the outer size, no decrease.
        // Matching still type-checks trivially: the `n` binding in
        // `succ(n)` has type SizedNat(i) = expected. This doesn't
        // exercise hypothesis entailment (there's no SizedPi in the
        // ctor) but verifies the old path still works after the
        // refactor that introduced `CtorArg`.
        let decl = sized_nat_decl();
        let (c, i_val) = ctx_with_size_var("i");

        let snat_i = Val::InductiveType {
            decl: decl.clone(),
            params: vec![i_val],
            indices: Vec::new(),
        };
        let x_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("x".to_string()), &snat_i, &x_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let match_exp = Exp::Match {
            scrutinee: Box::new(Exp::Var("x".to_string())),
            arms: vec![
                crate::nbe::term::MatchArm {
                    ctor_name: "zero".to_string(),
                    bindings: vec![],
                    body: Exp::Var("x".to_string()),
                },
                crate::nbe::term::MatchArm {
                    ctor_name: "succ".to_string(),
                    bindings: vec![Patt::Var("n".to_string())],
                    body: Exp::Var("n".to_string()),
                },
            ],
        };
        check(&mut c2, &match_exp, &snat_i).expect("old-style sized Nat match still works");
    }
}
