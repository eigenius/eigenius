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
//! **The felicity gate** — the kernel as the oracle (D62 §2 stage 5).
//!
//! A full-span parse is only a *candidate*: the composition rules built a term from the categories, but
//! nothing yet says the assembled sem is well-typed. This stage decides. It evaluates the sem (NbE),
//! reads back the normal form, and `check`s it against the type the category denotes (`⟦cat⟧`) — so a
//! parse is admitted iff the KERNEL types it. Nothing else in the pipeline is trusted to judge that.
//!
//! Two outcomes survive: a CLOSED parse (a hole-free proposition), and an [`OpenParse`] — felicitous,
//! but still carrying referent holes (a pronoun / possessor), which `super::resolve` then binds. An
//! infelicitous candidate is simply dropped: an empty forest is a first-class answer, not an error.

use super::*;

impl Parser {
    /// Normalize `it.sem()` (NbE β-reduction → a normal form) and keep the item —
    /// carrying the reduced sem — only if the kernel confirms it **inhabits `⟦cat⟧`**:
    /// `Prop` for a declarative `S`, `T → Prop` for a wh-question `Q(T)`. Uses
    /// check-mode (not `check_infer`) so a wh-question's answer-property *lambda* —
    /// which `check_infer` cannot synthesize — is checked against its expected Π/→.
    pub(super) fn reduced_felicitous(&self, it: &Item) -> Option<Item> {
        let expected = denote_cat(it.cat()).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        let nf = felicity_readback(&eval(it.sem(), &Rho::Nil).ok()?)?;
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.grammar.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item::from_parts(it.cat().clone(), nf, it.prov(), it.cost()))
    }

    /// Build-then-subsume (D3, `docs/notes/d63-nominal-modification-normal-form.md` §8; Eisner 1996's
    /// exact restricted-grammar fallback): drop a closed reading whose sem is **definitionally equal**
    /// to one already kept. [`Self::reduced_felicitous`] / [`Self::classify_felicitous`] have already
    /// normalized every sem to its NbE normal form, so equal *meaning* is now equal *structure* — this
    /// collapses spurious ambiguity (different derivations, one reading) and, being an equality, **never
    /// drops a distinct reading** (the rare luxury the typed kernel affords). Uses structural `Exp`
    /// equality on the FULL IRIs — not the lossy [`super::super::pretty_term`], which shortens an IRI to its
    /// local segment and could false-merge two distinct senses. O(n²) over the pre-cap forest, which the
    /// felicity gate has already bounded to the classify-candidate count.
    pub(super) fn subsume_duplicates(forest: &mut Vec<Item>) {
        let mut out: Vec<Item> = Vec::with_capacity(forest.len());
        for it in forest.drain(..) {
            if !out
                .iter()
                .any(|k| k.cat() == it.cat() && k.sem() == it.sem())
            {
                out.push(it);
            }
        }
        *forest = out;
    }

    /// Classify a full-span candidate as a CLOSED felicitous parse or an OPEN one (D64), or reject it.
    /// Generalizes [`Self::reduced_felicitous`] to hole-bearing sems: at seed time each hole is a fresh
    /// free variable, so eval binds it to a generic neutral (else Pure `eval` errors `UnboundVariable`)
    /// to reach the normal form. For an OPEN parse the normal form's holes are then ABSTRACTED into a
    /// closed function `λ(hᵢ:Tᵢ). nf`, checked against `Π(hᵢ:Tᵢ). ⟦cat⟧` (see [`OpenParse`]) — the same
    /// verification as a Γ-bound `nf`, but now the sem is a self-contained term and resolution is
    /// application. `hole_specs` carries every candidate hole `(base name, type, kind)`; a candidate
    /// mentions only the subset it actually carries — currently `EntityRef` holes (`Entity`, in argument
    /// position: a pronoun/possessor referent → D64). `Neut::Gen(0, base)` reads back as `Var("{base}0")`,
    /// so the binder + reported hole name use that readback form. With no holes present this is exactly
    /// `reduced_felicitous` — the closed path is unchanged.
    pub(super) fn classify_felicitous(
        &self,
        it: &Item,
        hole_specs: &[(String, Exp, HoleKind)],
    ) -> Option<FelicitousOutcome> {
        // Holes carried by this parse (tested on the raw, pre-reduction sem).
        let present: Vec<&(String, Exp, HoleKind)> = hole_specs
            .iter()
            .filter(|(base, _, _)| exp_mentions_var(it.sem(), base))
            .collect();
        let expected = denote_cat(it.cat()).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        // Evaluate the assembled sem with each freshened hole base bound to a generic neutral
        // (else Pure eval errors on the free var). `Neut::Gen(0, base)` reads back as
        // `Var("{base}0")`, so the holes in the normal form carry that suffixed name.
        let mut eval_rho = Rho::Nil;
        for (base, _, _) in &present {
            eval_rho =
                eval_rho.extend(Patt::Var(base.clone()), Val::Nt(Neut::Gen(0, base.clone())));
        }
        // STEP-TIMING instrumentation (set `EIGENIUS_PARSE_DEBUG=1`): each step is flushed BEFORE
        // it runs, so the last line printed before an OOM/SIGKILL names the exploding step
        // (eval / readback / check) — the felicity gate is the witnessed full-lexicon blow-up site.
        let dbg = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();
        if dbg {
            eprintln!("    [felicity] eval start");
        }
        let evaled = eval(it.sem(), &eval_rho).ok()?;
        if dbg {
            eprintln!("    [felicity] readback start");
        }
        let nf = felicity_readback(&evaled)?;
        // The holes carried by this parse — each a typed parameter (readback-named `{base}0`).
        let infos: Vec<HoleInfo> = present
            .iter()
            .map(|(base, ty_exp, kind)| HoleInfo {
                var: format!("{base}0"),
                ty: (*ty_exp).clone(),
                kind: (*kind).clone(),
            })
            .collect();
        if dbg {
            eprintln!("    [felicity] check start");
        }
        if infos.is_empty() {
            // CLOSED: `nf` is a hole-free `Prop`; check it directly against ⟦cat⟧.
            let mut ctx =
                CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.grammar.layer));
            if let Err(e) = check(&mut ctx, &nf, &expected_val) {
                // `EIGENIUS_TRACE_GATE=1` — WHY a full-span candidate was refused. The gate is the
                // last stage, so a silent `.ok()?` here turns any type error into an unexplained
                // grammar-gap; recovering the reason otherwise means bisecting the derivation by hand.
                if std::env::var("EIGENIUS_TRACE_GATE").is_ok() {
                    eprintln!("  !! GATE REFUSED prov={:?} err={e:?}", it.prov());
                    if let Some(w) = conn_over_function(&nf) {
                        eprintln!("     {w}");
                    }
                }
                return None;
            }
            let item = Item::from_parts(it.cat().clone(), nf, it.prov(), it.cost());
            return Some(FelicitousOutcome::Closed(item));
        }
        // OPEN (D64): ABSTRACT each hole as a typed parameter, so the sem is a CLOSED function
        // `λ(h₀:T₀)…(hₙ:Tₙ). nf : Π(h₀:T₀)…(hₙ:Tₙ). ⟦cat⟧` — a *parametric* proposition, not a term
        // with free variables. Resolution is then plain APPLICATION ([`Parser::resolve_open`]). Fold
        // innermost-first so `holes[0]` is the OUTERMOST binder (the application order). The gate checks
        // the abstraction against the Π-type (empty Γ — the binders type the holes), which is the same
        // verification as the old Γ-bound `nf`, now inside the type theory.
        let mut abstracted = nf;
        let mut pi_ty = expected;
        for info in infos.iter().rev() {
            abstracted = Exp::Lam(Patt::Var(info.var.clone()), Box::new(abstracted));
            pi_ty = Exp::Pi(
                Patt::Var(info.var.clone()),
                Box::new(info.ty.clone()),
                Box::new(pi_ty),
            );
        }
        let pi_val = eval(&pi_ty, &Rho::Nil).ok()?;
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.grammar.layer));
        check(&mut ctx, &abstracted, &pi_val).ok()?;
        let item = Item::from_parts(it.cat().clone(), abstracted, it.prov(), it.cost());
        Some(FelicitousOutcome::Open(OpenParse { item, holes: infos }))
    }
}

/// What a hole dispatches to once resolved (the carrier's resolver tag — D64). Currently the single
/// `EntityRef` (pronoun/possessive referents → the D64 anaphora resolver), an *internal-resolution*
/// hole. (The `Quantification` variant — a bare plural's deferred determiner — was removed with the
/// kind-predication reshape Phase B, since bare plural/mass now commit to `kind_of(t)`; `ProofObligation`
/// for factive presuppositions is a planned future arm.) The carrier types each hole per its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleKind {
    /// An unresolved entity referent (a pronoun / possessor), resolved by APPLYING a chain antecedent
    /// to its parameter and re-gating. First-order, `Entity`-typed, in argument position.
    EntityRef,
}

/// One typed parameter of an [`OpenParse`]'s abstraction: the binder name (`var`) standing for an
/// unresolved referent, the EigenTT type it must inhabit (Slice 1: `Entity`), and its resolver
/// [`HoleKind`]. A `Proposer` consumes it (to filter/rank antecedents); [`Parser::resolve_open`]
/// applies the chosen antecedent to it.
#[derive(Clone, Debug)]
pub struct HoleInfo {
    pub var: String,
    pub ty: Exp,
    pub kind: HoleKind,
}

/// An **open** parse (D64): a felicitous full-span `S` whose sem is a PARAMETRIC proposition —
/// `item.sem()` is the closed function `λ(h₀:T₀)…(hₙ:Tₙ). body : Π(h₀:T₀)…(hₙ:Tₙ). ⟦cat⟧`, abstracting
/// each unresolved referent as a typed parameter (`holes[0]` = the outermost binder). Each [`HoleInfo`]
/// names one such parameter (type + resolver [`HoleKind`]); [`Parser::resolve_open`] closes the parse
/// by APPLYING a chain antecedent to each parameter and re-gating. The sem is a well-typed EigenTT term
/// — just a function, not yet a `Prop`; it becomes a `Prop` once the parameters are applied.
#[derive(Clone)]
pub struct OpenParse {
    pub item: Item,
    pub holes: Vec<HoleInfo>,
}

/// The outcome of classifying a full-span candidate (see [`Parser::classify_felicitous`]).
pub(super) enum FelicitousOutcome {
    Closed(Item),
    Open(OpenParse),
}

/// Readback for the **felicity oracle**. The gate evaluates UNTRUSTED candidate sems off the chart,
/// and a spurious derivation can produce a stuck application (e.g. a resource applied as a function —
/// witnessed for a named-individual subject under do-support/modal + a PP). `try_readback_val`
/// returns that as `Err` rather than panicking (`readback_val` asserts well-typedness and would
/// panic), so such a candidate is simply **not felicitous** — reject it (`None`). This is the
/// readback half of the fallibility `eval` already has; it replaced an earlier `catch_unwind` guard
/// that turned the panic into a rejection but still printed to stderr.
fn felicity_readback(val: &Val) -> Option<Exp> {
    try_readback_val(0, val).ok()
}

/// A complete clause root must be **finite**: `cat_s(_, fin | fin_any)`. A base /
/// infinitival clause (`cat_s(_, bse)` — the VP an auxiliary selects) is never a
/// standalone sentence (D63 §8.5, Slice 5a). Non-`cat_s` categories are not clauses.
pub(super) fn is_finite_clause(cat: &Exp) -> bool {
    match is_ctor(cat, "cat_s") {
        Some([_mood, fin]) => {
            matches!(fin, Exp::InductiveCtor(_, n, _) if n == "fin" || n == "fin_any")
        }
        _ => false,
    }
}

/// Find a **connective applied to a function** in a NORMALIZED sem — the (B) defect.
///
/// `logic:And`/`logic:Or` are declared `(P : Prop, Q : Prop) : Prop`, so an argument that is an
/// abstraction is ill-typed and can never reduce to a `Prop`.
///
/// This runs on `nf`, at the gate, because a CONSTRUCTION-TIME test cannot find it: every DCG site
/// builds the connective over `Exp::App(…)` terms and the λ only surfaces after β-normalization.
/// Instrumenting `generalized_coord`, `fold_conn` and `conjoin_canonical` for a syntactic
/// `Exp::Lam` argument gave 0 hits across the whole page — cap-only and under replay — for exactly
/// that reason. Reports the connective, the argument index and the binder, which with `it.prov()`
/// beside it names the construction.
fn conn_over_function(e: &Exp) -> Option<String> {
    if let Exp::InductiveType(d, args) = e {
        if matches!(
            d.iri.as_str(),
            "urn:eigenius:logic:And" | "urn:eigenius:logic:Or"
        ) {
            for (i, a) in args.iter().enumerate() {
                if let Exp::Lam(p, body) = a {
                    return Some(format!(
                        "CONN-OVER-FUNCTION: {}(arg#{i}) is λ{p:?}. {}",
                        d.name,
                        match body.as_ref() {
                            Exp::App(..) => "App(…)",
                            Exp::Lam(..) => "λ…",
                            Exp::InductiveType(d2, _) => d2.name.as_str(),
                            _ => "…",
                        }
                    ));
                }
            }
        }
    }
    Parser::sem_subterms(e)
        .into_iter()
        .find_map(conn_over_function)
}
