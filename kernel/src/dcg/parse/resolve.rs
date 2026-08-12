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
use crate::dcg::verbalize::{resource_label, unit_sense_names, verbalize, Vb};

/// Cap on FULL re-gates ([`Parser::resolve_open`] calls) per [`Parser::resolve_with`] search —
/// the kernel's self-protection against an over-proposing (untrusted) proposer: per-hole vetoes
/// prune linearly before the search, but a multi-hole parse whose whole-term re-gate keeps
/// failing would otherwise walk the full assignment cross-product at one term-sized
/// eval+check each. Exhaustion fails CLOSED (the parse stays open). 64 mirrors the chart's
/// cell-beam scale: the winner is expected within the first few ranked assignments.
const MAX_REGATE_ATTEMPTS: usize = 64;

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
            // **The hole-type veto, enforced HERE.** β-reduction ERASES the Π-binder's type
            // annotation, and after substitution the antecedent only meets the body's own
            // argument types (typically the wide `Entity` of a verb slot) — so the whole-term
            // check below cannot see a restrictor. Checking `antecedent : Tᵢ` before applying is
            // what makes a demonstrative's restrictor typing real ("these findings" resolves
            // only to findings). Subsumption comes with it: the checker's intensional
            // resource-inhabits-class rule walks `Layer::is_subclass_of`, so a SUBCLASS-typed
            // antecedent is accepted. (Found by the slice-2 veto test: without this check, the
            // Gene-for-CellLine antecedent resolved — the veto did not exist.)
            if !self.hole_accepts(hole, ante) {
                return None;
            }
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
    /// and re-gating through the kernel (D64 §4, the resolve loop). Per hole, the in-scope
    /// `candidates` are FIRST filtered by the RESTRICTOR VETO ([`Self::hole_accepts`]) — the
    /// veto is a per-(hole, candidate) fact, so it runs LINEARLY here, never inside the
    /// assignment cross-product (an all-candidates proposer on a two-hole parse otherwise
    /// re-checks every pair; the first close-out run spent 50 min there) — and only the
    /// type-passing subset is PRESENTED to the proposer (plan §2.4: the LLM never wastes prompt
    /// tokens ranking candidates the kernel would veto; its indices refer to the presented
    /// list). The depth-first search over the proposer-ranked assignments then re-gates whole
    /// parses, first success wins, capped at [`MAX_REGATE_ATTEMPTS`] full re-gates — the kernel
    /// self-protects against an over-proposing proposer (the proposer is untrusted input;
    /// "bounded by its list lengths" is not a bound the kernel may rely on). **Fail-closed**
    /// everywhere: a hole with every candidate vetoed, a proposer that returns no ranking, and
    /// a search that exhausts its budget all yield `None` — no committed parse. The proposer
    /// never decides felicity; [`Self::resolve_open`] (the kernel) does.
    pub fn resolve_with(
        &self,
        open: &OpenParse,
        doc: &DocumentContext,
        candidates: &[Candidate],
        proposer: &dyn Proposer,
    ) -> Option<Item> {
        let mut ranked: Vec<Vec<Exp>> = Vec::with_capacity(open.holes.len());
        for hole in &open.holes {
            // The type pre-filter: (candidate, antecedent term) pairs the veto admits for THIS hole.
            let fits: Vec<(&Candidate, Exp)> = candidates
                .iter()
                .filter_map(|c| {
                    let ante = self.antecedent_exp(c)?;
                    self.hole_accepts(hole, &ante).then_some((c, ante))
                })
                .collect();
            if fits.is_empty() {
                return None; // every candidate vetoed ⇒ fail closed
            }
            let presented: Vec<Candidate> = fits.iter().map(|(c, _)| (*c).clone()).collect();
            let proposal = proposer.propose(&ProposeCtx {
                doc,
                hole,
                candidates: &presented,
            });
            let antes: Vec<Exp> = proposal
                .ranked
                .iter()
                .filter_map(|&i| fits.get(i)) // out-of-range ⇒ ignored (untrusted input)
                .map(|(_, a)| a.clone())
                .collect();
            if antes.is_empty() {
                return None; // proposer declined every presented candidate ⇒ fail closed
            }
            ranked.push(antes);
        }
        let mut budget = MAX_REGATE_ATTEMPTS;
        self.search_resolve(open, &ranked, &mut Vec::new(), &mut budget)
    }

    /// Does `ante` inhabit the hole's declared type? The RESTRICTOR VETO as a standalone
    /// per-(hole, antecedent) fact — [`Self::resolve_open`] enforces the same check per binding
    /// (the soundness authority for direct callers); [`Self::resolve_with`] uses it to filter
    /// each hole's candidates BEFORE the assignment search.
    fn hole_accepts(&self, hole: &HoleInfo, ante: &Exp) -> bool {
        let Ok(ty_val) = eval(&hole.ty, &Rho::Nil) else {
            return false;
        };
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.grammar.layer));
        check(&mut ctx, ante, &ty_val).is_ok()
    }

    /// Depth-first search over per-hole ranked antecedents: assign one antecedent per hole, then
    /// re-gate the whole assignment via [`Self::resolve_open`]; the first that type-checks closed
    /// wins, and a kernel veto backtracks to the next candidate (the trust boundary driving
    /// retry). `budget` caps the total number of full re-gates (fail closed on exhaustion).
    fn search_resolve(
        &self,
        open: &OpenParse,
        ranked: &[Vec<Exp>],
        acc: &mut Vec<(String, Exp)>,
        budget: &mut usize,
    ) -> Option<Item> {
        let i = acc.len();
        if i == ranked.len() {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            return self.resolve_open(open, acc);
        }
        for ante in &ranked[i] {
            if *budget == 0 {
                return None;
            }
            acc.push((open.holes[i].var.clone(), ante.clone()));
            if let Some(it) = self.search_resolve(open, ranked, acc, budget) {
                return Some(it);
            }
            acc.pop();
        }
        None
    }

    /// The antecedent term for a proposed [`Candidate`]: an individual resolves through the
    /// chain to its sem (`EigonResource`/`EigonClass`/`EigonAxiom`, per the entity's kind —
    /// `None` if the IRI does not resolve, so a stale candidate fails closed before re-gating);
    /// a kind IS its harvested `kind_of(…)` term.
    fn antecedent_exp(&self, cand: &Candidate) -> Option<Exp> {
        match cand {
            Candidate::Individual { iri, .. } => {
                self.grammar.layer.resolve(iri)?;
                Some(super::super::lexicon::resolve_sem(&self.grammar.layer, iri))
            }
            Candidate::Kind { term, .. } => Some(term.clone()),
        }
    }

    /// **Stage C — the discourse resolve loop** (D64 §4, `docs/design/d64-llm-anaphora-resolution.md`).
    /// Parse the document's `sentences` IN ORDER, threading a growing candidate set of antecedents.
    ///
    /// **Pooled competition** (plan §2.2): each sentence's reading pool is its CLOSED readings ∪
    /// the open readings whose holes RESOLVE against the discourse — every open parse is tried via
    /// [`Self::resolve_with`] (the untrusted `proposer` suggests, the kernel re-gates), so a closed
    /// reading no longer silently kills an anaphoric one, and a reading whose holes cannot resolve
    /// drops out (the kernel veto acting as a selection filter). A pool of one is `Encoded`; a pool
    /// of several goes to the `ranker` (below); an empty pool is `Open` (holes but no resolution —
    /// **fail-closed**, never a wrong closed parse) or `Gap`. Then harvest the sentence's discourse
    /// referents — named entities AND kinds ([`Self::discourse_candidates`]) — into the candidate
    /// set, **most-recent-first**, for later sentences.
    ///
    /// This is the piece D64 §4 leaves to the caller: the resolver primitives already exist, but nothing
    /// assembled candidates or threaded the discourse. The `proposer` is impl-agnostic — a deterministic
    /// mock in tests, the live `AnthropicProposer` (`use-llm`) end to end, or the orchestrator bridge
    /// (Phase 2). Recency is the only salience signal we model; the proposer does the ranking (§4).
    /// Only PRIOR-discourse referents are candidates (intra-sentential binding is a refinement).
    ///
    /// **Reading selection** (D63 `docs/notes/d63-reading-selection.md`): when the pool holds SEVERAL
    /// readings and a `ranker` is installed, the ranker chooses one — in document context (the
    /// surrounding text + the glosses of prior selections) — and the sentence becomes `Encoded` with a
    /// [`SelectionOutcome`] audit record. No ranker, or a ranker abstention, keeps the fail-open
    /// `Ambiguous` outcome. There is no kernel veto on this choice (every pooled candidate
    /// type-checks — the resolved-open ones through the re-gate); the audit record + the offline
    /// faithfulness gate are the controls.
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
            let (closed, open) = self.parse_open(s, lemmatizer);
            // Pool = closed ∪ resolved-open, deduplicated by sem: two open parses resolving to
            // the same closed proposition (or duplicating a closed reading) are ONE reading.
            let mut pool = closed;
            let mut seen: BTreeSet<String> = pool.iter().map(|it| pretty_term(it.sem())).collect();
            {
                // The proposer gets the SAME document context as the reading ranker (§2.4):
                // surrounding text + prior selections. Scoped: it borrows `prior`, which the
                // gloss threading below appends to.
                let doc_ctx = DocumentContext {
                    document: &document,
                    sentence: s,
                    prior_selections: &prior,
                };
                for o in &open {
                    if let Some(item) = self.resolve_with(o, &doc_ctx, &candidates, proposer) {
                        if seen.insert(pretty_term(item.sem())) {
                            pool.push(item);
                        }
                    }
                }
            }
            let mut selection = None;
            let outcome = if pool.len() == 1 {
                SentenceOutcome::Encoded(pool.pop().expect("len==1"))
            } else if pool.len() > 1 {
                match ranker
                    .and_then(|r| self.select_reading(r, &document, s, lemmatizer, &prior, &pool))
                {
                    Some((idx, sel)) => {
                        let item = pool.remove(idx);
                        selection = Some(sel);
                        SentenceOutcome::Encoded(item)
                    }
                    None => SentenceOutcome::Ambiguous(pool),
                }
            } else if let Some(o) = open.first() {
                SentenceOutcome::Open(o.clone())
            } else {
                SentenceOutcome::Gap
            };
            // Thread the discourse: the chosen reading's gloss joins the document context of the
            // later sentences (sequential consistency). Threaded UNCONDITIONALLY since §2.4 —
            // the anaphora proposer consumes prior selections too, not just the ranker.
            if let SentenceOutcome::Encoded(item) = &outcome {
                let gloss = match &selection {
                    Some(sel) => sel.chosen_gloss.clone(),
                    None => self.reading_gloss(s, lemmatizer, item),
                };
                prior.push(PriorSelection { ordinal, gloss });
            }
            let harvest = match &outcome {
                SentenceOutcome::Encoded(item) => Some(item.sem()),
                SentenceOutcome::Ambiguous(items) => items.first().map(Item::sem),
                _ => None,
            };
            if let Some(sem) = harvest {
                let mut fresh = self.discourse_candidates(s, lemmatizer, sem);
                // Most-recent-first WITHOUT duplicates: a referent re-mentioned now moves to the
                // front (recency is the salience signal); its older entry is dropped.
                let fresh_keys: BTreeSet<String> = fresh.iter().map(Candidate::key).collect();
                candidates.retain(|c| !fresh_keys.contains(&c.key()));
                fresh.append(&mut candidates);
                candidates = fresh;
            }
            out.push(SentenceResolution { outcome, selection });
        }
        out
    }

    /// The discourse-antecedent candidates one resolved sem contributes (D64 §4, plan §2.3): every
    /// `EigonResource` — a committed named entity — and every CLOSED `ontology:kind_of(…)` subterm
    /// — a kind this sentence makes discourse-referent ("…genome-scale shRNA **libraries**" →
    /// "These libraries…") — in first-mention order, deduplicated. Surfaces are READABLE: an
    /// individual's layer label ([`resource_label`], falling back to the IRI local name), a kind's
    /// verbalized gloss over this sentence's own sense names. A `kind_of` subterm mentioning a
    /// variable bound outside it is skipped — it is not a self-standing referent (and could never
    /// re-gate closed; the closedness walk is a pre-filter, the empty-Γ re-gate stays the
    /// authority). Landed CLAIMS as antecedents ("These findings…") are pending Stage 3's
    /// incremental landing — a unit whose referent is a prior claim stays honestly `Open`.
    fn discourse_candidates(
        &self,
        sentence: &str,
        lemmatizer: &dyn Lemmatizer,
        sem: &Exp,
    ) -> Vec<Candidate> {
        let names = unit_sense_names(sentence, self, lemmatizer, &self.grammar.layer);
        let vb = Vb {
            names: &names,
            layer: &self.grammar.layer,
        };
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        self.walk_candidates(sem, &vb, &mut out, &mut seen);
        out
    }

    fn walk_candidates(
        &self,
        e: &Exp,
        vb: &Vb,
        out: &mut Vec<Candidate>,
        seen: &mut BTreeSet<String>,
    ) {
        if let Exp::App(f, k) = e {
            if matches!(f.as_ref(), Exp::EigonAxiom(i) if i.as_str() == "urn:eigenius:ontology:kind_of")
                && closed_under(e, &mut Vec::new())
            {
                let cand = Candidate::Kind {
                    term: e.clone(),
                    surface: verbalize(e, vb),
                };
                if seen.insert(cand.key()) {
                    out.push(cand);
                }
                // Keep walking INSIDE the kind: a refined kind's restrictor mentions further
                // referents ("data sets **for genes**…" — the inner kind is anaphorable too).
                self.walk_candidates(k, vb, out, seen);
                return;
            }
        }
        match e {
            Exp::EigonResource(res) => {
                if let Some(iri) = res.id() {
                    let surface = resource_label(iri, &self.grammar.layer).unwrap_or_else(|| {
                        iri.as_str().rsplit(':').next().unwrap_or("").to_string()
                    });
                    let cand = Candidate::Individual {
                        iri: iri.clone(),
                        surface,
                    };
                    if seen.insert(cand.key()) {
                        out.push(cand);
                    }
                }
            }
            Exp::App(a, b)
            | Exp::Arrow(a, b)
            | Exp::Times(a, b)
            | Exp::Pair(a, b)
            | Exp::Pi(_, a, b)
            | Exp::Sig(_, a, b)
            | Exp::Ann(a, b) => {
                self.walk_candidates(a, vb, out, seen);
                self.walk_candidates(b, vb, out, seen);
            }
            Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) => self.walk_candidates(b, vb, out, seen),
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
                for a in args {
                    self.walk_candidates(a, vb, out, seen);
                }
            }
            _ => {}
        }
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
            chosen_sem: cands[sel.chosen].sem.clone(),
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
    /// computed here). Public for drivers that thread the discourse themselves (the encoding
    /// pipeline's ranked arm) — the SAME gloss function keeps their replay keys compatible.
    pub fn reading_gloss(
        &self,
        sentence: &str,
        lemmatizer: &dyn Lemmatizer,
        item: &Item,
    ) -> String {
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
    /// Multiple readings survive — closed parses and/or discourse-resolved open parses (the §2.2
    /// pool): the sentence parses but carries unresolved sense/structural/referential ambiguity.
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
    /// The chosen reading's sense-erased skeleton — the structure key (the grammar-faithfulness
    /// pins and the skeleton adjudication ledger are written in it).
    pub chosen_skeleton: String,
    /// The chosen reading's pretty-printed sem — the READING's identity (structure + senses),
    /// the key the reading-level adjudication ledger is written in.
    pub chosen_sem: String,
    /// The chosen reading's verbalised gloss.
    pub chosen_gloss: String,
    /// The ranker's stated reason, verbatim.
    pub rationale: String,
    /// The remaining candidates' skeletons in the ranker's preference order.
    pub runner_up_skeletons: Vec<String>,
    /// How many readings competed.
    pub candidates: usize,
}

/// Is `e` closed under the binders WITHIN it (no variable bound outside)? The harvest
/// pre-filter for kind candidates — a subterm mentioning an enclosing binder's variable is not a
/// self-standing discourse referent. Unhandled `Exp` variants pass (the empty-Γ re-gate in
/// [`Parser::resolve_open`] remains the authority on closedness).
fn closed_under(e: &Exp, bound: &mut Vec<String>) -> bool {
    fn bind(p: &Patt, bound: &mut Vec<String>) -> usize {
        match p {
            Patt::Var(n) => {
                bound.push(n.clone());
                1
            }
            Patt::Pair(a, b) => bind(a, bound) + bind(b, bound),
            Patt::Unit => 0,
        }
    }
    match e {
        Exp::Var(v) => bound.iter().any(|b| b == v),
        Exp::Lam(p, b) => {
            let n = bind(p, bound);
            let ok = closed_under(b, bound);
            bound.truncate(bound.len() - n);
            ok
        }
        Exp::Pi(p, a, b) | Exp::Sig(p, a, b) => {
            let ok_a = closed_under(a, bound);
            let n = bind(p, bound);
            let ok_b = closed_under(b, bound);
            bound.truncate(bound.len() - n);
            ok_a && ok_b
        }
        Exp::App(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) | Exp::Ann(a, b) => {
            closed_under(a, bound) && closed_under(b, bound)
        }
        Exp::Fst(x) | Exp::Snd(x) => closed_under(x, bound),
        Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
            args.iter().all(|a| closed_under(a, bound))
        }
        _ => true,
    }
}

/// A candidate antecedent for anaphora resolution (D64 §4, plan §2.3), with its READABLE surface
/// form for the proposer to rank against. The resolver assembles these from the discourse context
/// ([`Parser::resolve_document`]); the (untrusted) [`Proposer`] selects among them by index, and
/// the kernel re-gates. A third referent kind — a LANDED CLAIM ("These findings…") — is pending
/// Stage 3's incremental landing: its resolution semantics (claim-resource typing, plural/group
/// reference to a SET of claims) are undecided, so no variant exists yet and such units stay
/// honestly `Open`.
#[derive(Clone, Debug)]
pub enum Candidate {
    /// A committed named entity — an in-scope chain individual.
    Individual { iri: Iri, surface: String },
    /// A kind made discourse-referent by a prior sentence — a closed `ontology:kind_of(…)` term
    /// (possibly Σ-refined: ⟦MSI cell lines⟧). The kernel's derived-kind-predication coercion
    /// lets it inhabit a restrictor-typed hole iff its base class subsumes into the restrictor.
    Kind { term: Exp, surface: String },
}

impl Candidate {
    /// The surface form presented to the proposer.
    pub fn surface(&self) -> &str {
        match self {
            Candidate::Individual { surface, .. } | Candidate::Kind { surface, .. } => surface,
        }
    }

    /// The candidate's stable identity — the dedup/recency key of the discourse candidate set,
    /// and the identity the proposal record/replay key is written in.
    pub fn key(&self) -> String {
        match self {
            Candidate::Individual { iri, .. } => format!("i:{}", iri.as_str()),
            Candidate::Kind { term, .. } => format!("k:{}", pretty_term(term)),
        }
    }
}

/// The context handed to a [`Proposer`] for one referent hole (plan §2.4): the full
/// [`DocumentContext`] — the SAME context the reading ranker gets (surrounding input text, the
/// target sentence, the glosses of prior sentences' selected readings), because "these lines"
/// is resolved by the surrounding prose, not the bare sentence — plus the hole (its restrictor
/// type + kind) and the candidate antecedents ALREADY FILTERED to the hole's type (the §2.4
/// pre-filter; `ctx.candidates` is the presented list the proposal's indices refer to). Number
/// features (sg/pl of the anaphor) are NOT yet carried — [`HoleInfo`] has no number; threading
/// it needs the felicity gate to capture cat features at freshening, a later increment.
pub struct ProposeCtx<'a> {
    pub doc: &'a DocumentContext<'a>,
    pub hole: &'a HoleInfo,
    pub candidates: &'a [Candidate],
}

/// A proposer's answer for one hole: candidate indices ranked most-preferred first (into the
/// presented `ctx.candidates`; empty ⇒ unresolvable; out-of-range indices are ignored), plus the
/// audit fields a live proposer supplies (recorded verbatim; `None` from deterministic impls).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Proposal {
    pub ranked: Vec<usize>,
    /// Why this ranking — one sentence, recorded into the run's proposal artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// The proposer's own confidence in its TOP pick, `0.0..=1.0`. Advisory: nothing gates on
    /// it yet; it is recorded so a later slice can calibrate abstention against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

impl Proposal {
    /// A ranking with no audit fields — the deterministic-impl convenience.
    pub fn ranked(ranked: Vec<usize>) -> Self {
        Self {
            ranked,
            rationale: None,
            confidence: None,
        }
    }
}

/// The **untrusted** anaphora proposer (D64 §4): given a hole and the presented candidates,
/// return a ranked [`Proposal`]. The proposer only ever *selects among the assembled candidates*
/// — it cannot introduce an antecedent of its own, which is what lets a kind (a term, not an
/// IRI) be a candidate at all — and the kernel re-gates every selection
/// ([`Parser::resolve_open`]). Impls: deterministic mocks (tests), the record/replay pair
/// (`crate::dcg::proposer_record`), a feature-gated live LLM client (`use-llm`), and the
/// production orchestrator bridge — all behind this one trait, so the algorithm is impl-agnostic.
pub trait Proposer {
    fn propose(&self, ctx: &ProposeCtx) -> Proposal;
}

impl<T: Proposer + ?Sized> Proposer for Box<T> {
    fn propose(&self, ctx: &ProposeCtx) -> Proposal {
        (**self).propose(ctx)
    }
}

impl<T: Proposer + ?Sized> Proposer for std::sync::Arc<T> {
    fn propose(&self, ctx: &ProposeCtx) -> Proposal {
        (**self).propose(ctx)
    }
}
