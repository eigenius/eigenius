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

    /// Classify a full-span candidate as a CLOSED felicitous parse or an OPEN one carrying
    /// unresolved holes (D64), or reject it. Generalizes [`Self::reduced_felicitous`] to
    /// hole-bearing sems: each hole is a free variable, so it is bound in `rho` to a generic neutral
    /// (else Pure `eval` errors `UnboundVariable`) and in `gamma` to **its own type** so `check`
    /// types it. `hole_specs` carries every candidate hole `(base name, type, kind)`; a candidate
    /// mentions only the subset it actually carries — currently `EntityRef` holes (`Entity`, in
    /// argument position: a pronoun/possessor referent → D64). `Neut::Gen(0, base)` reads back as
    /// `Var("{base}0")`, so the gamma key and reported hole name use that readback form. With no holes
    /// present this is exactly `reduced_felicitous` (empty `rho`/`gamma`) — the closed path is unchanged.
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
        // Check the normal form under a context binding each (readback-named) hole in BOTH
        // `rho` (a neutral value — `check` evaluates subterms, which would otherwise error on the
        // free var) and `gamma` (its **own** type — `Entity` for a referent, the GQ type for a
        // quantification hole). The carried `HoleInfo` reports each hole's type + kind.
        let mut chk_rho = Rho::Nil;
        let mut gamma: Gamma = Vec::new();
        let mut infos: Vec<HoleInfo> = Vec::new();
        for (base, ty_exp, kind) in &present {
            let name = format!("{base}0");
            chk_rho = chk_rho.extend(Patt::Var(name.clone()), Val::Nt(Neut::Gen(0, name.clone())));
            gamma.push((name.clone(), eval(ty_exp, &Rho::Nil).ok()?));
            infos.push(HoleInfo {
                var: name,
                ty: (*ty_exp).clone(),
                kind: (*kind).clone(),
            });
        }
        let mut ctx = CheckCtx::with_layer(chk_rho, gamma, Arc::clone(&self.grammar.layer));
        if dbg {
            eprintln!("    [felicity] check start");
        }
        check(&mut ctx, &nf, &expected_val).ok()?;
        let item = Item::from_parts(it.cat().clone(), nf, it.prov(), it.cost());
        if infos.is_empty() {
            Some(FelicitousOutcome::Closed(item))
        } else {
            Some(FelicitousOutcome::Open(OpenParse { item, holes: infos }))
        }
    }
}

/// What a hole dispatches to once resolved (the carrier's resolver tag — D64). Currently the single
/// `EntityRef` (pronoun/possessive referents → the D64 anaphora resolver), an *internal-resolution*
/// hole. (The `Quantification` variant — a bare plural's deferred determiner — was removed with the
/// kind-predication reshape Phase B, since bare plural/mass now commit to `kind_of(t)`; `ProofObligation`
/// for factive presuppositions is a planned future arm.) The carrier types each hole per its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleKind {
    /// An unresolved entity referent (a pronoun / possessor), resolved by substituting a chain
    /// antecedent and re-gating. First-order, `Entity`-typed, in argument position.
    EntityRef,
}

/// One referent hole in an [`OpenParse`]: the free variable standing in the sem, the EigenTT
/// type it must inhabit (Slice 1: `Entity`), and its resolver [`HoleKind`]. This is what a
/// `Proposer` consumes (to filter/rank antecedents) and what [`Parser::resolve_open`]
/// fills.
#[derive(Clone, Debug)]
pub struct HoleInfo {
    pub var: String,
    pub ty: Exp,
    pub kind: HoleKind,
}

/// An **open** parse (D64): a felicitous full-span `S` whose sem still carries unresolved
/// referent holes (free variables). Each [`HoleInfo`] is a slot the D64 resolver fills (by
/// substituting a chain antecedent + re-gating — [`Parser::resolve_open`]). The kernel
/// type-checked `item.sem()` with each hole bound to its type; it is NOT a closed final parse.
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
