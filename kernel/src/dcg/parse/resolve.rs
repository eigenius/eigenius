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
//! **Open-parse resolution** (D64): binding the referent HOLES an open parse carries — a pronoun or
//! possessor left a fresh free variable at seed time — to concrete antecedents, and closing the parse.
//!
//! The parser's job ends at a felicitous but HOLE-BEARING term ([`OpenParse`], produced by the felicity
//! gate); this is the stage that hands those holes to a [`Proposer`] (the untrusted LLM resolver lives
//! in `super::super::resolver_llm`), substitutes the accepted bindings, and re-checks. The kernel stays
//! the oracle: a proposal is only ever *offered*, and a binding that does not type-check is rejected.

use super::*;

use crate::dcg::pretty::pretty_term;
use crate::dcg::reading_ranker::{
    DocumentContext, PriorSelection, ReadingCandidate, ReadingRanker,
};
use crate::dcg::skeleton::skeleton_of;
use crate::dcg::verbalize::{unit_sense_names, verbalize, Vb};

impl Parser {
    /// Resolve an [`OpenParse`] by substituting each hole with a proposed antecedent and
    /// **re-gating** through the kernel (D64 §4 — the trusted half of anaphora resolution; the
    /// untrusted proposer only ever *suggests* antecedents). `bindings` maps a hole's
    /// [`HoleInfo::var`] to its antecedent term (e.g. `EigonResource`/`EigonClass` for a chain
    /// entity). Each hole is bound to the antecedent's *value* during evaluation, so the
    /// resulting normal form is **closed**; it is then checked to inhabit `⟦cat⟧`. Returns the
    /// resolved closed [`Item`] iff every hole is bound and the closed term type-checks — a
    /// type-mismatched antecedent (e.g. a `Gene` where the predicate needs a `CellLine`) makes
    /// the check fail and yields `None`, exactly the kernel veto that keeps the LLM from having
    /// the last word. A leftover (unbound) hole likewise fails closed.
    pub fn resolve_open(&self, open: &OpenParse, bindings: &[(String, Exp)]) -> Option<Item> {
        // The open sem is `λ(h₀:T₀)…(hₙ:Tₙ). body` (D64 — a parametric proposition). Resolution is
        // APPLICATION: apply each hole's antecedent in binder order (`holes[0]` is the outermost binder),
        // then β-reduce. A hole with no binding ⇒ `None` (fail closed); the re-gate (empty Γ) rejects an
        // antecedent that itself still carries a free variable.
        let mut term = open.item.sem().clone();
        for hole in &open.holes {
            let ante = bindings
                .iter()
                .find(|(v, _)| *v == hole.var)
                .map(|(_, a)| a)?;
            term = Exp::App(Box::new(term), Box::new(ante.clone()));
        }
        let nf = readback_val(0, &eval(&term, &Rho::Nil).ok()?);
        let expected = denote_cat(open.item.cat()).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        // Closed re-gate: empty Γ, so any leftover hole is an unbound variable ⇒ fail closed.
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.grammar.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item::from_parts(
            open.item.cat().clone(),
            nf,
            open.item.prov(),
            open.item.cost(),
        ))
    }

    /// Resolve **every** hole of an [`OpenParse`] via an (untrusted) [`Proposer`], substituting
    /// and re-gating through the kernel (D64 §4, the resolve loop). For each hole the proposer
    /// is asked, given the sentence and the in-scope `candidates`, for a **ranked** list of
    /// antecedent IRIs; the loop searches those assignments (depth-first, bounded by the
    /// proposer's list lengths) and returns the first whole-parse assignment the kernel re-gates
    /// to a closed `Prop`. **Fail-closed**: a hole the proposer leaves empty, or whose every
    /// candidate the kernel vetoes (type mismatch), yields `None` — no committed parse. The
    /// proposer never decides felicity; [`Self::resolve_open`] (the kernel) does.
    pub fn resolve_with(
        &self,
        open: &OpenParse,
        sentence: &str,
        candidates: &[Candidate],
        proposer: &dyn Proposer,
    ) -> Option<Item> {
        let mut ranked: Vec<Vec<Exp>> = Vec::with_capacity(open.holes.len());
        for hole in &open.holes {
            let picks = proposer.propose(&ProposeCtx {
                sentence,
                hole,
                candidates,
            });
            let antes: Vec<Exp> = picks
                .iter()
                .filter_map(|iri| self.antecedent_exp(iri))
                .collect();
            if antes.is_empty() {
                return None; // unresolvable / unknown antecedent ⇒ fail closed
            }
            ranked.push(antes);
        }
        self.search_resolve(open, &ranked, &mut Vec::new())
    }

    /// Depth-first search over per-hole ranked antecedents: assign one antecedent per hole, then
    /// re-gate the whole assignment via [`Self::resolve_open`]; the first that type-checks closed
    /// wins, and a kernel veto backtracks to the next candidate (the trust boundary driving
    /// retry). Bounded by the proposer's list lengths.
    fn search_resolve(
        &self,
        open: &OpenParse,
        ranked: &[Vec<Exp>],
        acc: &mut Vec<(String, Exp)>,
    ) -> Option<Item> {
        let i = acc.len();
        if i == ranked.len() {
            return self.resolve_open(open, acc);
        }
        for ante in &ranked[i] {
            acc.push((open.holes[i].var.clone(), ante.clone()));
            if let Some(it) = self.search_resolve(open, ranked, acc) {
                return Some(it);
            }
            acc.pop();
        }
        None
    }

    /// The antecedent term for a chain-entity IRI: an `EigonResource` (named entity), `EigonClass`
    /// (a class), or `EigonAxiom`, per the entity's kind. `None` if the IRI does not resolve in
    /// the chain (so a hallucinated antecedent fails closed before re-gating).
    fn antecedent_exp(&self, iri: &Iri) -> Option<Exp> {
        self.grammar.layer.resolve(iri)?;
        Some(super::super::lexicon::resolve_sem(&self.grammar.layer, iri))
    }

    /// **Stage C — the discourse resolve loop** (D64 §4, `docs/design/d64-llm-anaphora-resolution.md`).
    /// Parse the document's `sentences` IN ORDER, threading a growing candidate set of antecedents. For
    /// each sentence: parse; if the best full parse is already CLOSED keep it; if it is OPEN (carries
    /// `EntityRef` referent holes — a pronoun / "these X"), resolve every hole against the in-scope
    /// `candidates` via [`Self::resolve_with`] (the untrusted `proposer` suggests, the kernel re-gates);
    /// a gap or unresolvable hole yields `None` (**fail-closed**). Then harvest the resolved sentence's
    /// referenced named entities into the candidate set — **most-recent-first** — for later sentences.
    /// Returns one resolved (closed) [`Item`] per input sentence.
    ///
    /// This is the piece D64 §4 leaves to the caller: the resolver primitives already exist, but nothing
    /// assembled candidates or threaded the discourse. The `proposer` is impl-agnostic — a deterministic
    /// mock in tests, the live `AnthropicProposer` (`use-llm`) end to end, or the orchestrator bridge
    /// (Phase 2). Recency is the only salience signal we model; the proposer does the ranking (§4). First
    /// cut: candidate surfaces are the entity IRI local names (a readable label is a later refinement),
    /// and only PRIOR-discourse entities are candidates (intra-sentential binding is a refinement).
    ///
    /// **Reading selection** (D63 `docs/notes/d63-reading-selection.md`): when a sentence has SEVERAL
    /// closed readings and a `ranker` is installed, the ranker chooses one — in document context (the
    /// surrounding text + the glosses of prior selections) — and the sentence becomes `Encoded` with a
    /// [`SelectionOutcome`] audit record. No ranker, or a ranker abstention, keeps the fail-open
    /// `Ambiguous` outcome. There is no kernel veto on this choice (every candidate type-checks); the
    /// audit record + the offline faithfulness gate are the controls.
    pub fn resolve_document(
        &self,
        sentences: &[&str],
        lemmatizer: &dyn Lemmatizer,
        proposer: &dyn Proposer,
        ranker: Option<&dyn ReadingRanker>,
    ) -> Vec<SentenceResolution> {
        let document = sentences.join(" ");
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut prior: Vec<PriorSelection> = Vec::new();
        let mut out = Vec::with_capacity(sentences.len());
        for (ordinal, s) in sentences.iter().enumerate() {
            let (mut closed, open) = self.parse_open(s, lemmatizer);
            let mut selection = None;
            let outcome = if closed.len() == 1 {
                SentenceOutcome::Encoded(closed.pop().expect("len==1"))
            } else if closed.len() > 1 {
                match ranker
                    .and_then(|r| self.select_reading(r, &document, s, lemmatizer, &prior, &closed))
                {
                    Some((idx, sel)) => {
                        let item = closed.remove(idx);
                        selection = Some(sel);
                        SentenceOutcome::Encoded(item)
                    }
                    None => SentenceOutcome::Ambiguous(closed),
                }
            } else if let Some(o) = open.first() {
                // OPEN: try to resolve its referent holes against the discourse; unresolvable ⇒ stays open.
                match self.resolve_with(o, s, &candidates, proposer) {
                    Some(item) => SentenceOutcome::Encoded(item),
                    None => SentenceOutcome::Open(o.clone()),
                }
            } else {
                SentenceOutcome::Gap
            };
            // Thread the discourse: the chosen reading's gloss joins the ranker's context for the later
            // sentences (sequential consistency), and its named entities join the anaphora candidate
            // set (most-recent-first). Gloss threading only runs with a ranker installed — without one
            // nothing consumes it.
            if ranker.is_some() {
                if let SentenceOutcome::Encoded(item) = &outcome {
                    let gloss = match &selection {
                        Some(sel) => sel.chosen_gloss.clone(),
                        None => self.reading_gloss(s, lemmatizer, item),
                    };
                    prior.push(PriorSelection { ordinal, gloss });
                }
            }
            let harvest = match &outcome {
                SentenceOutcome::Encoded(item) => Some(item.sem()),
                SentenceOutcome::Ambiguous(items) => items.first().map(Item::sem),
                _ => None,
            };
            if let Some(sem) = harvest {
                let mut fresh = entity_candidates(sem);
                fresh.append(&mut candidates);
                candidates = fresh;
            }
            out.push(SentenceResolution { outcome, selection });
        }
        out
    }

    /// Present a sentence's surviving readings to the (untrusted) `ranker` — grouped by skeleton,
    /// glossed by the shared verbaliser ([`crate::dcg::verbalize`]) — and map its choice back to an
    /// index into `closed`. `None` = abstain (no selection, or a malformed reply — an out-of-range
    /// index abstains rather than panicking; the ranker is untrusted input).
    ///
    /// Public because the measurement harness drives it directly (per-unit, with its own parse
    /// config and document order) while [`Self::resolve_document`] drives it in the pipeline —
    /// ONE presentation function for both, so the gate measures exactly what the pipeline runs.
    pub fn select_reading(
        &self,
        ranker: &dyn ReadingRanker,
        document: &str,
        sentence: &str,
        lemmatizer: &dyn Lemmatizer,
        prior: &[PriorSelection],
        closed: &[Item],
    ) -> Option<(usize, SelectionOutcome)> {
        let names = unit_sense_names(sentence, self, lemmatizer, &self.grammar.layer);
        let vb = Vb {
            names: &names,
            layer: &self.grammar.layer,
        };
        let skels: Vec<String> = closed.iter().map(|it| skeleton_of(it.sem())).collect();
        // Present GROUPED BY SKELETON (the stable sort keeps the forest's cost order within a
        // group), so structural alternatives sit side by side for the ranker.
        let mut order: Vec<usize> = (0..closed.len()).collect();
        order.sort_by(|&a, &b| skels[a].cmp(&skels[b]));
        let cands: Vec<ReadingCandidate> = order
            .iter()
            .map(|&i| ReadingCandidate {
                skeleton: skels[i].clone(),
                gloss: verbalize(closed[i].sem(), &vb),
                sem: pretty_term(closed[i].sem()),
            })
            .collect();
        let ctx = DocumentContext {
            document,
            sentence,
            prior_selections: prior,
        };
        let sel = ranker.select(&ctx, &cands)?;
        let chosen = *order.get(sel.chosen)?;
        let outcome = SelectionOutcome {
            chosen_skeleton: cands[sel.chosen].skeleton.clone(),
            chosen_gloss: cands[sel.chosen].gloss.clone(),
            rationale: sel.rationale,
            runner_up_skeletons: sel
                .runners_up
                .iter()
                .filter_map(|&i| cands.get(i))
                .map(|c| c.skeleton.clone())
                .collect(),
            candidates: cands.len(),
        };
        Some((chosen, outcome))
    }

    /// The chosen reading's gloss for the ranker's prior-selection context (a uniquely-encoded or
    /// anaphora-resolved sentence doesn't go through [`Self::select_reading`], so its gloss is
    /// computed here).
    fn reading_gloss(&self, sentence: &str, lemmatizer: &dyn Lemmatizer, item: &Item) -> String {
        let names = unit_sense_names(sentence, self, lemmatizer, &self.grammar.layer);
        verbalize(
            item.sem(),
            &Vb {
                names: &names,
                layer: &self.grammar.layer,
            },
        )
    }
}

/// The outcome of encoding one sentence — the classified result of [`Parser::resolve_document`]
/// (and the document pipeline). Fail-closed: a sentence that cannot be encoded is `Open` or `Gap`, never
/// a silently-dropped or wrong closed parse.
#[derive(Clone)]
pub enum SentenceOutcome {
    /// A single closed, resolved proposition — the encoded knowledge (`item.sem()` is the `Prop`).
    Encoded(Item),
    /// Multiple closed parses: the sentence parses but carries unresolved sense/structural ambiguity.
    Ambiguous(Vec<Item>),
    /// Parsed but carries an unresolved referent hole — the anaphora proposer found no antecedent.
    Open(OpenParse),
    /// No parse — an OOV token, or an all-known-tokens grammar gap.
    Gap,
}

/// One sentence's result from the discourse loop: the classified outcome plus, when a reading
/// ranker chose among several surviving readings, the selection audit record.
#[derive(Clone)]
pub struct SentenceResolution {
    pub outcome: SentenceOutcome,
    /// `Some` iff a [`ReadingRanker`] selection collapsed an ambiguous forest to the `Encoded`
    /// reading. Uniquely-encoded, resolved-open, and abstained sentences carry `None`.
    pub selection: Option<SelectionOutcome>,
}

/// The audit record of an automated reading selection (`docs/notes/d63-reading-selection.md` §3):
/// what was chosen, why, and what it beat. Downstream emission records it as the claim's
/// `enc:DecisionPoint`. There is no kernel veto on the choice — this record and the offline
/// faithfulness gate are the controls.
#[derive(Clone, Debug)]
pub struct SelectionOutcome {
    /// The chosen reading's sense-erased skeleton — the key the pins and the adjudication ledger
    /// are written in.
    pub chosen_skeleton: String,
    /// The chosen reading's verbalised gloss.
    pub chosen_gloss: String,
    /// The ranker's stated reason, verbatim.
    pub rationale: String,
    /// The remaining candidates' skeletons in the ranker's preference order.
    pub runner_up_skeletons: Vec<String>,
    /// How many readings competed.
    pub candidates: usize,
}

/// The named-entity antecedent candidates a resolved sem references — every `EigonResource` IRI (a
/// committed named entity), as a [`Candidate`] whose surface is the IRI local name (the part after the
/// last `:`), in first-seen order. Used by [`Parser::resolve_document`] to build the discourse
/// candidate set. (Kinds / prior propositions as antecedents are a later refinement.)
fn entity_candidates(sem: &Exp) -> Vec<Candidate> {
    fn walk(e: &Exp, out: &mut Vec<Candidate>, seen: &mut BTreeSet<Iri>) {
        match e {
            Exp::EigonResource(res) => {
                if let Some(iri) = res.id() {
                    if seen.insert(iri.clone()) {
                        let surface = iri.as_str().rsplit(':').next().unwrap_or("").to_string();
                        out.push(Candidate {
                            iri: iri.clone(),
                            surface,
                        });
                    }
                }
            }
            Exp::App(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) => {
                walk(a, out, seen);
                walk(b, out, seen);
            }
            Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Ann(a, b) => {
                walk(a, out, seen);
                walk(b, out, seen);
            }
            Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) => walk(b, out, seen),
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
                for a in args {
                    walk(a, out, seen);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    walk(sem, &mut out, &mut seen);
    out
}

/// A candidate antecedent for anaphora resolution (D64 §4): an in-scope committed chain entity,
/// with its surface form for the proposer to rank against. The resolver assembles these from the
/// discourse context; the (untrusted) [`Proposer`] ranks/selects, and the kernel re-gates.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub iri: Iri,
    pub surface: String,
}

/// The context handed to a [`Proposer`] for one referent hole: the sentence, the hole (its type
/// + kind), and the in-scope candidate antecedents.
pub struct ProposeCtx<'a> {
    pub sentence: &'a str,
    pub hole: &'a HoleInfo,
    pub candidates: &'a [Candidate],
}

/// The **untrusted** anaphora proposer (D64 §4): given a hole and the in-scope candidates, return
/// a **ranked** list of antecedent IRIs (most-preferred first; empty ⇒ unresolvable). It only
/// *suggests*; the kernel re-gates every suggestion ([`Parser::resolve_open`]). Impls: a
/// deterministic mock (tests), a feature-gated live LLM client (`use-llm`), and the production
/// orchestrator bridge — all behind this one trait, so the algorithm is impl-agnostic.
pub trait Proposer {
    fn propose(&self, ctx: &ProposeCtx) -> Vec<Iri>;
}
