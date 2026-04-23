//! Mini-TT bidirectional type checker.
//!
//! Ported from `Main.hs` lines 289-378 in the Mini-TT reference.
//! Uses NbE (eval + readback) for type equality checking.

use crate::layer::Layer;
use crate::nbe::env::{gen_val, lookup_gamma, up_gamma, Gamma, Rho};
use crate::nbe::eval::eval;
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
/// (`references/nanoda_lib/src/tc.rs`): a single struct carrying
/// mutable state (cache) plus immutable environment through all checker
/// calls. The cache is scoped per type-check invocation — fresh per
/// call, no cross-check invalidation needed.
pub struct CheckCtx {
    pub rho: Rho,
    pub gamma: Gamma,
    /// Optional layer for ontology resolution. `None` is the "pure"
    /// case used by tests that don't touch EigonClass resolution.
    pub layer: Option<Arc<Layer>>,
    /// Per-check memoization of resolved class types, keyed by class IRI string.
    type_cache: BTreeMap<String, Val>,
}

impl CheckCtx {
    /// Create a new context with no layer access (pure mode).
    pub fn new(rho: Rho, gamma: Gamma) -> Self {
        Self {
            rho,
            gamma,
            layer: None,
            type_cache: BTreeMap::new(),
        }
    }

    /// Create a new context with layer access for ontology resolution.
    pub fn with_layer(rho: Rho, gamma: Gamma, layer: Arc<Layer>) -> Self {
        Self {
            rho,
            gamma,
            layer: Some(layer),
            type_cache: BTreeMap::new(),
        }
    }

    /// Extend the context with a new variable binding (for entering binders).
    /// Shares the layer and type_cache with the parent context.
    fn extend(&self, patt: &Patt, typ: &Val, val: &Val) -> Result<CheckCtx, String> {
        let gamma1 = up_gamma(&self.gamma, patt, typ, val)?;
        let rho1 = self.rho.clone().extend(patt.clone(), val.clone());
        Ok(CheckCtx {
            rho: rho1,
            gamma: gamma1,
            layer: self.layer.clone(),
            type_cache: self.type_cache.clone(),
        })
    }

    /// Resolve an EigonClass IRI to a Mini-TT Sigma type, with caching.
    fn resolve_class_cached(&mut self, iri: &Iri) -> Result<Val, String> {
        let layer = self.layer.as_ref().ok_or_else(|| {
            format!(
                "cannot resolve class '{}' — no layer access in pure check mode",
                iri
            )
        })?;
        let key = iri.as_str().to_string();
        if let Some(cached) = self.type_cache.get(&key) {
            return Ok(cached.clone());
        }
        let v = crate::program::ground::resolve_class_type(iri, layer)?;
        self.type_cache.insert(key, v.clone());
        Ok(v)
    }
}

/// Check that a declaration is well-typed, returning the extended type context.
///
/// Port of `checkD` from the reference.
pub fn check_decl(ctx: &mut CheckCtx, decl: &Decl) -> Result<Gamma, String> {
    match decl {
        Decl::Def(patt, typ, body) => {
            // Check that the type is well-formed
            check_type(ctx, typ)?;
            let t = eval(typ, &ctx.rho).map_err(|e| e.to_string())?;
            // Check that the body has the declared type
            check(ctx, body, &t)?;
            // Extend the type context
            up_gamma(
                &ctx.gamma,
                patt,
                &t,
                &eval(body, &ctx.rho).map_err(|e| e.to_string())?,
            )
        }
        Decl::Drec(patt, typ, body) => {
            // Known subtlety (issue #13 item 3): The body is type-checked
            // under a generic binding (gen_val) so the checker sees an
            // opaque variable, not the real recursive value. When the real
            // value is substituted (UpDec below), neutrals that previously
            // blocked may reduce to something incompatible. Mini-TT
            // mitigates this via the guardedness check for codata; full
            // termination checking is deferred to Phase 11a.
            //
            // Check that the type is well-formed
            check_type(ctx, typ)?;
            let t = eval(typ, &ctx.rho).map_err(|e| e.to_string())?;
            let gen = gen_val(&ctx.rho);
            // Extend context with the recursive variable and check body
            let mut inner = ctx.extend(patt, &t, &gen)?;
            check(&mut inner, body, &t)?;
            // Guardedness: if the recursive body constructs a corecord,
            // verify every corecursive reference appears under a
            // constructor/lambda/app — not at the bare head of an
            // observation. D11 §3 "productivity."
            let mut forbidden: std::collections::HashSet<&str> = std::collections::HashSet::new();
            collect_pattern_names(patt, &mut forbidden);
            check_guarded(body, &forbidden)?;
            // Re-evaluate with the recursive binding
            let v = eval(body, &Rho::UpDec(Box::new(ctx.rho.clone()), decl.clone()))
                .map_err(|e| e.to_string())?;
            up_gamma(&ctx.gamma, patt, &t, &v)
        }
    }
}

/// Check that an expression is a well-formed type.
///
/// Port of `checkT` from the reference.
pub fn check_type(ctx: &mut CheckCtx, exp: &Exp) -> Result<(), String> {
    match exp {
        Exp::Pi(p, a, b) | Exp::Sig(p, a, b) => {
            check_type(ctx, a)?;
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, &eval(a, &ctx.rho).map_err(|e| e.to_string())?, &gen)?;
            check_type(&mut inner, b)
        }
        Exp::Set | Exp::One | Exp::Type(_) => Ok(()),
        // Id(A, x, y) is a type if A is a type and x, y : A
        Exp::Id(a, x, y) => {
            check_type(ctx, a)?;
            let a_val = eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)
        }
        // Eigenius ground types are always valid types
        Exp::EigonClass(_) | Exp::EigonPrimitive(_) => Ok(()),

        // Codata type declaration: each observation's type must be a type.
        // Observation names must be distinct.
        Exp::Codata(observations) => {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for obs in observations {
                if !seen.insert(obs.name.as_str()) {
                    return Err(format!(
                        "duplicate observation name in codata type: '{}'",
                        obs.name
                    ));
                }
                check_type(ctx, &obs.typ)?;
            }
            Ok(())
        }

        a => check(ctx, a, &Val::Set),
    }
}

/// Check that an expression has a given type (checking mode).
///
/// Port of `check` from the reference.
pub fn check(ctx: &mut CheckCtx, exp: &Exp, typ: &Val) -> Result<(), String> {
    match (exp, typ) {
        // Lambda against Pi type
        (Exp::Lam(p, e), Val::Pi(t, g)) => {
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, t, &gen)?;
            check(&mut inner, e, &g.apply(gen).map_err(|e| e.to_string())?)
        }

        // Pair against Sigma type
        (Exp::Pair(e1, e2), Val::Sig(t, g)) => {
            check(ctx, e1, t)?;
            check(
                ctx,
                e2,
                &g.apply(eval(e1, &ctx.rho).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            )
        }

        // Constructor against Sum type
        (Exp::Con(c, e), Val::Data(cases, rho1)) => {
            let a = cases
                .iter()
                .find(|(name, _)| name == c)
                .map(|(_, typ)| typ)
                .ok_or_else(|| format!("constructor {c} not in sum type"))?;
            check(ctx, e, &eval(a, rho1).map_err(|e| e.to_string())?)
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
                return Err(format!(
                    "case branches {:?} do not match sum type {:?}",
                    branch_names, case_names
                ));
            }
            for (branch, (c, a)) in branches.iter().zip(cases.iter()) {
                let a_val = eval(a, rho1).map_err(|e| e.to_string())?;
                let g_c = Clos {
                    patt: Patt::Var("__case_arg".to_string()),
                    body: Exp::App(
                        Box::new(readback_val(ctx.rho.len(), &Val::Lam(g.clone()))),
                        Box::new(Exp::Con(
                            c.clone(),
                            Box::new(Exp::Var("__case_arg".to_string())),
                        )),
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
        (Exp::One, Val::Set) => Ok(()),

        // Pi type against Set
        (Exp::Pi(p, a, b), Val::Set) | (Exp::Sig(p, a, b), Val::Set) => {
            check(ctx, a, &Val::Set)?;
            let gen = gen_val(&ctx.rho);
            let mut inner = ctx.extend(p, &eval(a, &ctx.rho).map_err(|e| e.to_string())?, &gen)?;
            check(&mut inner, b, &Val::Set)
        }

        // Sum type against Set
        (Exp::Data(summands), Val::Set) => {
            for s in summands {
                check(ctx, &s.typ, &Val::Set)?;
            }
            Ok(())
        }

        // Declaration
        (Exp::Dec(d, e), t) => {
            let gamma1 = check_decl(ctx, d)?;
            let mut inner = CheckCtx {
                rho: Rho::UpDec(Box::new(ctx.rho.clone()), d.clone()),
                gamma: gamma1,
                layer: ctx.layer.clone(),
                type_cache: ctx.type_cache.clone(),
            };
            check(&mut inner, e, t)
        }

        // refl(a) : Id(A, a, a) — check that x and y are both a
        (Exp::Refl(a), Val::Id(typ, x, y)) => {
            check(ctx, a, typ)?;
            let a_val = eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            eq_nf(ctx.rho.len(), x, &a_val)?;
            eq_nf(ctx.rho.len(), y, &a_val)
        }

        // Id(A, x, y) : Set
        (Exp::Id(a, x, y), Val::Set) => {
            check(ctx, a, &Val::Set)?;
            let a_val = eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)
        }

        // Universe hierarchy: Type(n) : Type(n+1) prevents impredicativity.
        // Self-referential meta-claims (e.g. a level-1 trace referencing
        // level-1) are blocked at resource ingestion by the universe
        // stratification validator (Rule 13), not in the term checker.
        (Exp::Type(n), Val::Type(m)) if *n + 1 == *m => Ok(()),
        // Type(n) : Set (Set is the top universe for backward compatibility)
        (Exp::Type(_), Val::Set) => Ok(()),
        // Set : Type(1)
        (Exp::Set, Val::Type(1)) => Ok(()),

        // EigonClass/EigonPrimitive are ground types at level 0 but
        // inhabit all higher universes (cumulative).
        (Exp::EigonClass(_), Val::Set) | (Exp::EigonPrimitive(_), Val::Set) => Ok(()),
        (Exp::EigonClass(_), Val::Type(_)) | (Exp::EigonPrimitive(_), Val::Type(_)) => Ok(()),

        // Codata type formation: codata { ... } : Set
        (Exp::Codata(_), Val::Set) => check_type(ctx, exp),
        (Exp::Codata(_), Val::Type(_)) => check_type(ctx, exp),

        // Corecord against a codata type: each field's body must have
        // the corresponding observation's type, and every declared
        // observation must be covered.
        (Exp::CoRecord(fields), Val::Codata(observations, rho1)) => {
            let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            let obs_names: Vec<&str> = observations.iter().map(|(n, _)| n.as_str()).collect();
            if field_names != obs_names {
                return Err(format!(
                    "corecord fields {:?} do not match codata observations {:?}",
                    field_names, obs_names
                ));
            }
            for (field, (_, obs_typ)) in fields.iter().zip(observations.iter()) {
                let t = eval(obs_typ, rho1).map_err(|e| e.to_string())?;
                check(ctx, &field.body, &t)?;
            }
            Ok(())
        }

        // Fallthrough: infer type and compare
        (e, t) => {
            let t1 = check_infer(ctx, e)?;
            eq_nf(ctx.rho.len(), t, &t1)
        }
    }
}

/// Infer the type of an expression (inference mode).
///
/// Port of `checkI` from the reference.
pub fn check_infer(ctx: &mut CheckCtx, exp: &Exp) -> Result<Val, String> {
    match exp {
        Exp::Var(x) => lookup_gamma(&ctx.gamma, x),

        Exp::App(e1, e2) => {
            let t1 = check_infer(ctx, e1)?;
            let (t, g) = ext_pi(&t1)?;
            check(ctx, e2, &t)?;
            Ok(g.apply(eval(e2, &ctx.rho).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?)
        }

        Exp::Fst(e) => {
            let t = check_infer(ctx, e)?;
            let (t1, _) = ext_sig(&t)?;
            Ok(t1)
        }

        Exp::Snd(e) => {
            let t = check_infer(ctx, e)?;
            let (_, g) = ext_sig(&t)?;
            Ok(g.apply(
                eval(e, &ctx.rho)
                    .map_err(|e| e.to_string())?
                    .vfst()
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?)
        }

        // Eigenius: property/observation access type inference.
        //
        // ESL's `.name` syntax unifies two operations:
        // - property access on resources / Sigma-typed values
        // - observation on codata-typed values
        // We dispatch on the inferred type of the target.
        Exp::PropAccess(e, prop) => {
            let t = check_infer(ctx, e)?;
            let prop_name = prop.local_name();

            // Codata observation — same lookup that Exp::Observe does.
            if let Val::Codata(observations, rho1) = &t {
                for (name, typ) in observations {
                    if name == prop_name {
                        return eval(typ, rho1).map_err(|e| e.to_string());
                    }
                }
                return Err(format!(
                    "observation '{}' not found in codata type {:?}",
                    prop_name,
                    readback_val(ctx.rho.len(), &t)
                ));
            }

            // Fall back to the existing Sigma / resource behaviour.
            find_sigma_field(ctx, &t, prop_name).ok_or_else(|| {
                format!(
                    "property '{}' not found in type {:?}",
                    prop,
                    readback_val(ctx.rho.len(), &t)
                )
            })
        }

        // Codata observation type inference: e.obs has type T where
        // `obs : T` appears in the inferred codata type of e.
        Exp::Observe(e, obs) => {
            let t = check_infer(ctx, e)?;
            match &t {
                Val::Codata(observations, rho1) => {
                    for (name, typ) in observations {
                        if name == obs {
                            return eval(typ, rho1).map_err(|e| e.to_string());
                        }
                    }
                    Err(format!(
                        "observation '{}' not found in codata type {:?}",
                        obs,
                        readback_val(ctx.rho.len(), &t)
                    ))
                }
                other => Err(format!(
                    "observation target is not a codata value: {:?}",
                    readback_val(ctx.rho.len(), other)
                )),
            }
        }

        // --- Eigenius extension: 7 inference rules (D18 §6, issue #12 item 2) ---

        // Construct(class_iri, fields): check each field against the class's
        // Sigma chain and return EigonClass(class_iri).
        Exp::Construct(class_iri, fields) => {
            let class_type = ctx
                .resolve_class_cached(class_iri)
                .map_err(|e| format!("cannot infer Construct type for '{}': {}", class_iri, e))?;
            // Check each field against the resolved class type
            let mut remaining = class_type;
            for (prop_iri, field_exp) in fields {
                let field_type = find_sigma_field(ctx, &remaining, prop_iri.local_name())
                    .ok_or_else(|| {
                        format!("property '{}' not found in class '{}'", prop_iri, class_iri)
                    })?;
                check(ctx, field_exp, &field_type)?;
                // Advance through the Sigma chain
                remaining = advance_sigma(&remaining, prop_iri.local_name(), field_exp, &ctx.rho);
            }
            Ok(Val::EigonClass(class_iri.clone()))
        }

        // EigonResource(r): infer class from r.is_a().first()
        Exp::EigonResource(r) => {
            let classes = r.is_a();
            let class_iri = classes
                .first()
                .ok_or_else(|| "EigonResource has no is_a class".to_string())?;
            Ok(Val::EigonClass(class_iri.clone()))
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
            let a_val = eval(a, &ctx.rho).map_err(|e| e.to_string())?;
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
            let v_val = eval(v, &ctx.rho).map_err(|e| e.to_string())?;
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
            let a_val = eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            let x_val = eval(x, &ctx.rho).map_err(|e| e.to_string())?;
            let y_val = eval(y, &ctx.rho).map_err(|e| e.to_string())?;
            Ok(Val::Id(Box::new(a_val), Box::new(x_val), Box::new(y_val)))
        }

        // IdJ([A, C, d, x, y, p]): Martin-Löf J eliminator.
        // Per D18 §6.4, require an explicit motive C and return C(x, y, p).
        // Lean handles this via recursor reduction; we use a direct J-rule
        // since Mini-TT doesn't have a recursor framework.
        Exp::IdJ(args) => {
            let [ref a, ref _c, ref d, ref x, ref y, ref p] = **args;
            // A must be a type
            check_type(ctx, a)?;
            let a_val = eval(a, &ctx.rho).map_err(|e| e.to_string())?;
            // x, y : A
            check(ctx, x, &a_val)?;
            check(ctx, y, &a_val)?;
            let x_val = eval(x, &ctx.rho).map_err(|e| e.to_string())?;
            let y_val = eval(y, &ctx.rho).map_err(|e| e.to_string())?;
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
                Val::Pi(_, g) => g.apply(x_val).map_err(|e| e.to_string()),
                _ => Ok(Val::Set), // conservative fallback
            }
        }

        e => Err(format!("cannot infer type of: {e:?}")),
    }
}

/// Check type equality by normalization.
///
/// Port of `eqNf` from the reference: normalize both sides
/// and compare syntactically.
pub fn eq_nf(level: usize, v1: &Val, v2: &Val) -> Result<(), String> {
    let e1 = readback_val(level, v1);
    let e2 = readback_val(level, v2);
    if e1 == e2 {
        Ok(())
    } else {
        Err(format!("type mismatch: {e1:?} ≠ {e2:?}"))
    }
}

/// Collect the variable names bound by a pattern.
fn collect_pattern_names<'a>(p: &'a Patt, out: &mut std::collections::HashSet<&'a str>) {
    match p {
        Patt::Var(n) => {
            out.insert(n.as_str());
        }
        Patt::Pair(p1, p2) => {
            collect_pattern_names(p1, out);
            collect_pattern_names(p2, out);
        }
        Patt::Unit => {}
    }
}

/// If `exp` reduces syntactically to a forbidden variable through a
/// chain of observations and projections, return that variable's name.
/// Used by the guardedness check to detect unguarded corecursive
/// references at the head of an `Observe`.
///
/// This intentionally stops at `App` / `Lam` / `CoRecord` / constructor
/// boundaries — crossing any of those makes the reference guarded.
fn has_forbidden_head<'a>(
    exp: &'a Exp,
    forbidden: &std::collections::HashSet<&str>,
) -> Option<&'a str> {
    match exp {
        Exp::Var(x) if forbidden.contains(x.as_str()) => Some(x.as_str()),
        Exp::Observe(inner, _) => has_forbidden_head(inner, forbidden),
        Exp::Fst(inner) | Exp::Snd(inner) => has_forbidden_head(inner, forbidden),
        _ => None,
    }
}

/// Syntactic guardedness check for corecursive definitions (D11 §3).
///
/// A corecord definition `letrec x = ...` is guarded iff `x` (or any
/// mutually-bound name) never appears at the *head* of an
/// `Observe` expression within a field body — because doing so would
/// trigger immediate unfolding of the same corecord at the same layer,
/// producing no progress.
///
/// The check is syntactic and Agda-style. Productive patterns covered:
/// - `letrec nats(n) = corecord { head = n; tail = nats(n+1) }` — the
///   corecursive call is under `App`, which breaks the observation
///   chain; each observation produces a fresh corecord.
/// - `letrec ones = corecord { head = 1; tail = ones }` — a naked
///   reference at a field body is fine; observing `ones.tail.tail...`
///   re-returns the corecord value each time, with finite cost per
///   step.
///
/// Rejected:
/// - `letrec bad = corecord { head = bad.head; tail = ... }` — observing
///   `bad.head` requires evaluating `bad.head`, infinite loop.
///
/// Conservative approximation: syntactic guardedness cannot catch
/// cases where the loop goes through a function call (e.g. `broken(n).head`
/// where `broken` returns a corecord whose head body is
/// `broken(n).head`). Sized types would close that gap — out of scope
/// for v1. See D11 §3.4 and [eigenius#16][1].
///
/// [1]: https://github.com/eigenius/eigenius/issues/16
pub fn check_guarded(exp: &Exp, forbidden: &std::collections::HashSet<&str>) -> Result<(), String> {
    match exp {
        Exp::Observe(inner, obs) => {
            if let Some(name) = has_forbidden_head(inner, forbidden) {
                return Err(format!(
                    "unguarded corecursive reference: '{name}' is observed at field '{obs}' \
                     inside its own definition — this would loop at evaluation time. \
                     Put the recursive call under a function application or inside \
                     another constructor so that each observation makes progress."
                ));
            }
            check_guarded(inner, forbidden)
        }

        // Sub-expressions that need recursive checking.
        Exp::Lam(_, e) => check_guarded(e, forbidden),
        Exp::App(e1, e2) => {
            check_guarded(e1, forbidden)?;
            check_guarded(e2, forbidden)
        }
        Exp::Pair(e1, e2) => {
            check_guarded(e1, forbidden)?;
            check_guarded(e2, forbidden)
        }
        Exp::Con(_, e) => check_guarded(e, forbidden),
        Exp::Fst(e) | Exp::Snd(e) => check_guarded(e, forbidden),
        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) => {
            check_guarded(a, forbidden)?;
            check_guarded(b, forbidden)
        }
        Exp::Arrow(a, b) | Exp::Times(a, b) => {
            check_guarded(a, forbidden)?;
            check_guarded(b, forbidden)
        }
        Exp::Data(summands) => {
            for s in summands {
                check_guarded(&s.typ, forbidden)?;
            }
            Ok(())
        }
        Exp::Case(branches) => {
            for b in branches {
                check_guarded(&b.body, forbidden)?;
            }
            Ok(())
        }
        Exp::Dec(_, e) => check_guarded(e, forbidden),
        Exp::Id(a, x, y) => {
            check_guarded(a, forbidden)?;
            check_guarded(x, forbidden)?;
            check_guarded(y, forbidden)
        }
        Exp::Refl(a) => check_guarded(a, forbidden),
        Exp::IdJ(args) => {
            for a in args.iter() {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::NativeDecide(_, v) => check_guarded(v, forbidden),
        Exp::DecEq(a, x, y) => {
            check_guarded(a, forbidden)?;
            check_guarded(x, forbidden)?;
            check_guarded(y, forbidden)
        }
        Exp::PropAccess(e, _) => check_guarded(e, forbidden),
        Exp::Template(_, refs) => {
            for (_, t) in refs {
                check_guarded(t, forbidden)?;
            }
            Ok(())
        }
        Exp::Construct(_, fields) => {
            for (_, e) in fields {
                check_guarded(e, forbidden)?;
            }
            Ok(())
        }

        // Codata forms
        Exp::Codata(observations) => {
            for o in observations {
                check_guarded(&o.typ, forbidden)?;
            }
            Ok(())
        }
        Exp::CoRecord(fields) => {
            for f in fields {
                check_guarded(&f.body, forbidden)?;
            }
            Ok(())
        }

        // Leaves — no sub-expressions to check.
        Exp::Var(_)
        | Exp::Set
        | Exp::Type(_)
        | Exp::One
        | Exp::Unit
        | Exp::EigonClass(_)
        | Exp::EigonPrimitive(_)
        | Exp::EigonResource(_) => Ok(()),
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
        Val::Sig(t, g) => {
            if g.patt == Patt::Var(field_name.to_string()) {
                // Found — return the field's type
                Some(*t.clone())
            } else {
                // Not this field — apply the closure with a dummy value
                // and search the rest of the chain
                let gen = gen_val(&g.env);
                let rest = g.apply(gen).ok()?;
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

/// Advance past one field in a Sigma chain. After `find_sigma_field`
/// found `field_name`, this returns the rest of the Sigma: applies
/// the closure with the field's value and recurses.
fn advance_sigma(typ: &Val, field_name: &str, field_exp: &Exp, rho: &Rho) -> Val {
    match typ {
        Val::Sig(_, g) => {
            if g.patt == Patt::Var(field_name.to_string()) {
                match eval(field_exp, rho).and_then(|v| g.apply(v)) {
                    Ok(v) => v,
                    Err(_) => typ.clone(),
                }
            } else {
                let gen = gen_val(&g.env);
                match g.apply(gen) {
                    Ok(rest) => advance_sigma(&rest, field_name, field_exp, rho),
                    Err(_) => typ.clone(),
                }
            }
        }
        _ => typ.clone(),
    }
}

/// Extract a Pi type: Pi(A, x.B) → (A, x.B)
fn ext_pi(val: &Val) -> Result<(Val, Clos), String> {
    match val {
        Val::Pi(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(format!("expected Pi type, got: {u:?}")),
    }
}

/// Extract a Sigma type: Sig(A, x.B) → (A, x.B)
fn ext_sig(val: &Val) -> Result<(Val, Clos), String> {
    match val {
        Val::Sig(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(format!("expected Sigma type, got: {u:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;
    use crate::ontology::iri::Iri;

    fn ctx() -> CheckCtx {
        CheckCtx::new(Rho::Nil, vec![])
    }

    #[test]
    fn check_unit_has_type_one() {
        check(&mut ctx(), &Exp::Unit, &Val::One).unwrap();
    }

    #[test]
    fn check_one_has_type_set() {
        check(&mut ctx(), &Exp::One, &Val::Set).unwrap();
    }

    #[test]
    fn check_set_is_type() {
        check_type(&mut ctx(), &Exp::Set).unwrap();
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

    #[test]
    fn check_type_mismatch_fails() {
        // () : U should fail (unit is not a type)
        let result = check(&mut ctx(), &Exp::Unit, &Val::Set);
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
        eq_nf(0, &Val::Set, &Val::Set).unwrap();
    }

    #[test]
    fn eq_nf_not_equal() {
        assert!(eq_nf(0, &Val::One, &Val::Set).is_err());
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
        check(&mut ctx(), &data, &Val::Set).unwrap();
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
        // Id(1, (), ()) : Set
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check(&mut ctx(), &id, &Val::Set).unwrap();
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
            Exp::Set,                                                        // C (placeholder)
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
        let deceq = Exp::DecEq(Box::new(Exp::Set), Box::new(Exp::One), Box::new(Exp::Set));
        let result = eval(&deceq, &Rho::Nil)?;
        assert!(matches!(result, Val::Nt(_)));
        Ok(())
    }

    #[test]
    fn deceq_iri_equal() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let iri = Iri::parse("urn:eigenius:core:string").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::Set),
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
            Box::new(Exp::Set),
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
            &Val::Set,
        )
        .unwrap();
    }

    // --- Codata tests (D11, Phase 9b-i) ---

    fn pair_codata_type() -> Exp {
        // codata { fst : 1; snd : 1 }
        Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "fst".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "snd".to_string(),
                typ: Exp::One,
            },
        ])
    }

    fn unit_pair_corecord() -> Exp {
        // corecord { fst = (); snd = () }
        Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "fst".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "snd".to_string(),
                body: Exp::Unit,
            },
        ])
    }

    #[test]
    fn codata_type_is_a_type() {
        check_type(&mut ctx(), &pair_codata_type()).unwrap();
        check(&mut ctx(), &pair_codata_type(), &Val::Set).unwrap();
    }

    #[test]
    fn codata_duplicate_observation_rejected() {
        let bad = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "x".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "x".to_string(),
                typ: Exp::One,
            },
        ]);
        assert!(check_type(&mut ctx(), &bad).is_err());
    }

    #[test]
    fn corecord_checks_against_codata_type() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        check(&mut ctx(), &unit_pair_corecord(), &codata_typ)?;
        Ok(())
    }

    #[test]
    fn corecord_mismatched_fields_rejected() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        // Missing 'snd'
        let bad = Exp::CoRecord(vec![crate::nbe::term::CoField {
            name: "fst".to_string(),
            body: Exp::Unit,
        }]);
        assert!(check(&mut ctx(), &bad, &codata_typ).is_err());
        Ok(())
    }

    #[test]
    fn corecord_wrong_field_order_rejected() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        // Fields in wrong order
        let bad = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "snd".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "fst".to_string(),
                body: Exp::Unit,
            },
        ]);
        assert!(check(&mut ctx(), &bad, &codata_typ).is_err());
        Ok(())
    }

    #[test]
    fn observation_evaluates_to_field_body() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // corecord { fst = (); snd = () }.fst → ()
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "fst".to_string());
        let result = eval(&observe, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn observation_unknown_field_returns_err() {
        // vobserve now returns Err for unknown fields (issue #19)
        use crate::nbe::eval::eval;
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "missing".to_string());
        let result = eval(&observe, &Rho::Nil);
        assert!(result.is_err());
    }

    #[test]
    fn observation_type_inference() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::env::up_gamma;
        use crate::nbe::eval::eval;
        // Given x : codata { fst : 1; snd : 1 }, infer x.fst : 1
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let gamma = up_gamma(&vec![], &Patt::Var("x".to_string()), &codata_typ, &gen)?;
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), gen);
        let mut c = CheckCtx::new(rho, gamma);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let t = check_infer(&mut c, &observe)?;
        assert!(matches!(t, Val::One));
        Ok(())
    }

    #[test]
    fn observation_on_neutral_blocks() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // let x = <neutral>; x.fst should produce a Neut::Observe
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), neut);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let result = eval(&observe, &rho)?;
        assert!(matches!(
            result,
            Val::Nt(crate::nbe::val::Neut::Observe(_, _))
        ));
        Ok(())
    }

    #[test]
    fn stream_two_observations_advance() -> Result<(), Box<dyn std::error::Error>> {
        // letrec nats : Nat → codata { head : Nat; tail : codata { head : Nat; tail : ... } } = λn. corecord { head = n; tail = nats(n+1) }
        //
        // Simplified for testing: use Unit as the element type and
        // represent Nat as a chain of Con values. Observing head twice
        // should advance the stream.
        //
        // Stream type (same at every step, so we use a self-referential
        // type by using EigonPrimitive::Integer as a stand-in — type
        // checking is not the focus here; we just want to verify
        // evaluation and observation plumbing).
        use crate::nbe::eval::eval;
        use crate::nbe::term::PrimitiveType;

        // Build: λn. corecord { head = n; tail = f(n) }
        // where f is a free variable we'll instantiate via Rho.
        //
        // Instead of full recursion, verify two cases:
        //   corecord { head = (), tail = corecord { head = (), tail = <bottom> } }
        // and confirm that .tail.head returns ().
        let inner = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::EigonPrimitive(PrimitiveType::Integer), // placeholder "bottom"
            },
        ]);
        let outer = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: inner,
            },
        ]);
        // outer.tail.head → ()
        let expr = Exp::Observe(
            Box::new(Exp::Observe(Box::new(outer), "tail".to_string())),
            "head".to_string(),
        );
        let result = eval(&expr, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn recursive_stream_via_letrec() -> Result<(), Box<dyn std::error::Error>> {
        // letrec nats : codata { head : 1; tail : codata {...} } = corecord { head = (); tail = nats }
        // Observing nats.tail.tail.head should give ().
        use crate::nbe::eval::eval;

        // Self-referential codata type is tricky without proper type
        // theory; sidestep by using a simpler fixpoint test: the
        // evaluator should handle the corecursive reference via
        // Rho::UpDec.
        let corecord = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Var("nats".to_string()),
            },
        ]);
        // We don't need the type to check — just evaluate.
        let letrec = Exp::Dec(
            Decl::Drec(
                Patt::Var("nats".to_string()),
                Box::new(Exp::One), // placeholder type (not checked here)
                Box::new(corecord),
            ),
            // nats.tail.tail.head
            Box::new(Exp::Observe(
                Box::new(Exp::Observe(
                    Box::new(Exp::Observe(
                        Box::new(Exp::Var("nats".to_string())),
                        "tail".to_string(),
                    )),
                    "tail".to_string(),
                )),
                "head".to_string(),
            )),
        );
        let result = eval(&letrec, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    // --- Guardedness tests (D11 §3, Phase 9b-i) ---

    fn forbidden(names: &[&'static str]) -> std::collections::HashSet<&'static str> {
        names.iter().copied().collect()
    }

    #[test]
    fn guardedness_accepts_naked_corecursive_field_body() {
        // letrec ones = corecord { head = (); tail = ones }
        // The `tail` body is a naked reference to the corecursive name.
        // This is productive: observing tail returns the corecord,
        // subsequent observations are fresh steps.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Var("ones".to_string()),
            },
        ]);
        check_guarded(&body, &forbidden(&["ones"])).unwrap();
    }

    #[test]
    fn guardedness_accepts_corecursive_call_under_app() {
        // corecord { head = n; tail = nats(n+1) }
        // tail body is App(Var(nats), ...) — call under function
        // application is productive.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Var("n".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::App(Box::new(Exp::Var("nats".to_string())), Box::new(Exp::Unit)),
            },
        ]);
        check_guarded(&body, &forbidden(&["nats"])).unwrap();
    }

    #[test]
    fn guardedness_rejects_bare_corecursive_observation() {
        // corecord { head = bad.head; tail = ... }
        // Observing a corecord's own field inside its own body loops.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("bad".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        let err = check_guarded(&body, &forbidden(&["bad"])).unwrap_err();
        assert!(err.contains("unguarded"));
        assert!(err.contains("bad"));
    }

    #[test]
    fn guardedness_rejects_chained_corecursive_observation() {
        // bad.tail.head — chain of observations on corecursive name
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(
                    Box::new(Exp::Observe(
                        Box::new(Exp::Var("bad".to_string())),
                        "tail".to_string(),
                    )),
                    "head".to_string(),
                ),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        assert!(check_guarded(&body, &forbidden(&["bad"])).is_err());
    }

    #[test]
    fn guardedness_accepts_non_corecursive_letrec() {
        // letrec f = λx. f(x) — data recursion (not codata), no corecord.
        // Guardedness is a no-op here (data termination is a separate
        // concern; Mini-TT doesn't check it either).
        let body = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("x".to_string())),
            )),
        );
        check_guarded(&body, &forbidden(&["f"])).unwrap();
    }

    #[test]
    fn guardedness_accepts_observation_of_non_corecursive_ref() {
        // corecord { head = other.head; tail = () }
        // `other` is not a corecursive name here — observing it is fine.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("other".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        // Only `self` is forbidden; `other` is free.
        check_guarded(&body, &forbidden(&["self"])).unwrap();
    }

    #[test]
    fn guardedness_in_check_decl_rejects_bad_corecord() {
        // letrec bad : codata { head : 1; tail : 1 } = corecord { head = bad.head; tail = () }
        // The Drec pathway in check_decl now invokes check_guarded; this
        // should surface the unguarded reference.
        let codata_typ = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: Exp::One,
            },
        ]);
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("bad".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        let d = Decl::Drec(
            Patt::Var("bad".to_string()),
            Box::new(codata_typ),
            Box::new(body),
        );
        let err = check_decl(&mut ctx(), &d).unwrap_err();
        assert!(
            err.contains("unguarded"),
            "expected unguarded error, got: {err}"
        );
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
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(dog_iri.as_str().to_string()),
            ]),
        );
        let expr = Exp::EigonResource(Box::new(r));
        let t = check_infer(&mut ctx(), &expr).unwrap();
        match t {
            Val::EigonClass(iri) => assert_eq!(iri.as_str(), "urn:eigenius:example:Dog"),
            other => panic!("expected EigonClass, got {:?}", other),
        }
    }

    #[test]
    fn find_sigma_field_resolves_eigon_class_with_layer() {
        // With a layer, find_sigma_field on EigonClass should resolve
        // to actual property types instead of Val::Set.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(builder.build());

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        let layer = std::sync::Arc::new(domain_builder.build());

        let dog_iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let dog_type = Val::EigonClass(dog_iri);

        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        let field = find_sigma_field(&mut c, &dog_type, "name");
        assert!(field.is_some(), "should find 'name' on Dog");
        // The type should NOT be Val::Set (the old broken behavior)
        let field_type = field.unwrap();
        assert!(
            !matches!(field_type, Val::Set),
            "field type should be resolved, not Set; got {:?}",
            field_type
        );
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
}
