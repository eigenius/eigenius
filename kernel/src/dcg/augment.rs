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

//! **Document lexicon augmentation** (D63, `docs/notes/d63-lexicon-augmentation.md`) — the data model +
//! the deterministic `DocumentOnly` transducer that generalize Stage A's abbreviation glossary into
//! "resolve every lexical gap to a grounded, typed entry, exposing the augmentation as a first-class,
//! composable value."
//!
//! A [`LexicalBinding`] **wraps a proposed, un-committed `lexicon:LexicalEntry`** (the same type the parser
//! seeds) plus [`Provenance`] — how it was produced and how far to trust it. It is *not* a rival to the
//! committed entry; it is the proposal envelope in propose → gate → commit, and running the pipeline
//! **harvests** these as candidate permanent lexicon additions. A detected OOV that no proposal closes is a
//! [`Gap`] (a fail-closed finding, never a silent drop). [`LexiconAugmentation`] is the transducer's exposed
//! state: `added` (the harvest) + `missing_oov` (the residual).
//!
//! Phase 1 (here) implements the `DocumentOnly` source (the document's own abbreviation definitions +
//! the OOV pre-pass). `LexiconBacked` (text-retrieval grounding) and `LlmBacked` (synthesis) are Phase 2/3.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::layer::{resolve_active_text_indexes, Layer};
use crate::ontology::resource::{Resource, Value};
use crate::ontology::Iri;
use crate::query::text::analyzer::registry::analyzer_for;
use crate::query::text::search::run_text_search;

use super::glossary::{
    abbreviation_resources, extract_abbreviations_with, glossary_resources, ground_abbreviation,
    AbbreviationBinding, AbbreviationProposer,
};
use super::lemmatizer::Lemmatizer;
use super::lookup::{tokenize, LexicalIndex};

const LEXICAL_ENTRY: &str = "urn:eigenius:lexicon:LexicalEntry";

/// How a proposed lexical entry was resolved — a **trust signal** on the binding, most-trusted first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionMethod {
    /// The document itself defined it (Schwartz-Hearst / a definitional pattern). Deterministic.
    DefinitionExtracted,
    /// A retrieval hit against the committed lexicon (the form / description text index) grounded it.
    RetrievalGrounded,
    /// An LLM synthesized a provisional type/grounding from a retrieved definition. Lowest trust.
    LlmSynthesized,
}

/// The provenance envelope on a [`LexicalBinding`]: how the wrapped entry was produced + how far to trust
/// it (`docs/notes/d63-lexicon-augmentation.md` §3). Carried on the proposal, not on the committed entry.
#[derive(Clone, Debug)]
pub struct Provenance {
    /// The surface the gap was found under (pre-normalization).
    pub surface: String,
    /// The intra-document definition — `Some` for an abbreviation, `None` for a bare OOV term.
    pub long_form: Option<String>,
    /// The source window (grounding retries + audit).
    pub context: String,
    /// How the entry was resolved — the trust signal driving the promotion filter.
    pub method: ResolutionMethod,
    /// The ontology concept the entry aliases, when grounding succeeded (`None` ⇒ ungrounded / minted class).
    pub grounded_to: Option<Iri>,
    /// Retrieval / LLM confidence, when applicable.
    pub confidence: Option<f32>,
}

/// A proposed, un-committed `lexicon:LexicalEntry` + its [`Provenance`] — the unit the pipeline harvests
/// and the kernel gates before committing (§3). Wraps the committed type; it does not rival it.
#[derive(Clone, Debug)]
pub struct LexicalBinding {
    pub proposed: Resource,
    pub provenance: Provenance,
}

/// A detected OOV surface that **no proposal closed** — a fail-closed finding, never silently dropped (§7).
#[derive(Clone, Debug)]
pub struct Gap {
    pub surface: String,
    pub context: String,
    /// The resolution methods attempted (empty in `DocumentOnly` — nothing beyond abbreviation extraction).
    pub tried: Vec<ResolutionMethod>,
}

/// The lexicon-augmentation transducer's exposed state (§6): the harvested proposals + the residual gaps.
/// `supporting` holds non-entry resources a binding references (e.g. a fresh doc-local class minted on a
/// grounding miss) that must be committed alongside the entries.
#[derive(Clone, Debug, Default)]
pub struct LexiconAugmentation {
    pub added: Vec<LexicalBinding>,
    pub supporting: Vec<Resource>,
    pub missing_oov: Vec<Gap>,
}

impl LexiconAugmentation {
    /// Every resource to commit into the document's chained lexicon layer: the proposed entries + the
    /// supporting resources (in that order).
    pub fn resources(&self) -> Vec<Resource> {
        self.added
            .iter()
            .map(|b| b.proposed.clone())
            .chain(self.supporting.iter().cloned())
            .collect()
    }

    /// No entries added and no gaps recorded.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.missing_oov.is_empty()
    }
}

/// Which sources may generate entries (§6). Phase 1 implements [`AugmentOptions::DocumentOnly`];
/// `LexiconBacked` (text-retrieval grounding, scoped by a `lexicon:LexiconProfile`) and `LlmBacked`
/// (synthesis) are Phase 2/3.
#[derive(Clone, Debug)]
pub enum AugmentOptions {
    DocumentOnly,
    LexiconBacked(Iri),
    LlmBacked,
}

/// The **`DocumentOnly`** augmentation (Phase 1): from the document's own abbreviation definitions build
/// grounded [`LexicalBinding`]s (method [`ResolutionMethod::DefinitionExtracted`]), and flag every remaining
/// OOV token as a [`Gap`]. Deterministic — no retrieval, no LLM. Generalizes the Stage-A abbreviation
/// glossary into the augmentation shape (§2/§3): the same `extract → ground → emit` tail, but wrapped as
/// proposals with provenance, plus the fail-closed OOV pre-pass.
pub fn augment_document_only(
    base: &Arc<Layer>,
    document: &str,
    proposer: &dyn AbbreviationProposer,
    lemmatizer: &dyn Lemmatizer,
) -> LexiconAugmentation {
    let Ok(entry_class) = Iri::parse(LEXICAL_ENTRY) else {
        return LexiconAugmentation::default();
    };

    // Stage A → proposals. For each extracted definition, ground it and emit its alias entry, then wrap
    // the entry as a binding (the fresh doc-local class on a grounding miss becomes a supporting resource).
    let defs = extract_abbreviations_with(document, proposer);
    let mut added = Vec::new();
    let mut supporting = Vec::new();
    let mut known: BTreeSet<String> = BTreeSet::new();
    for d in &defs {
        let grounded_to = ground_abbreviation(base, &d.short_form, &d.long_form, &d.context);
        let (entries, extra): (Vec<Resource>, Vec<Resource>) =
            glossary_resources(base, std::slice::from_ref(d))
                .into_iter()
                .partition(|r| r.is_instance_of(&entry_class));
        supporting.extend(extra);
        for proposed in entries {
            added.push(LexicalBinding {
                proposed,
                provenance: Provenance {
                    surface: d.short_form.clone(),
                    long_form: Some(d.long_form.clone()),
                    context: d.context.clone(),
                    method: ResolutionMethod::DefinitionExtracted,
                    grounded_to: grounded_to.clone(),
                    confidence: None,
                },
            });
        }
        known.insert(d.short_form.trim().to_lowercase());
    }

    // OOV pre-pass (fail-closed): every single token the base lexicon does not know — and that we did not
    // just add as an abbreviation — is a `Gap`. `LexiconBacked`/`LlmBacked` (Phase 2/3) would try to ground
    // these; `DocumentOnly` reports them as-is.
    let index = LexicalIndex::build(Arc::clone(base));
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut missing_oov = Vec::new();
    for tok in tokenize(document) {
        let t = tok.trim().to_lowercase();
        if t.is_empty() || known.contains(&t) || !seen.insert(t.clone()) {
            continue;
        }
        if !index.has_token(&t, lemmatizer) {
            missing_oov.push(Gap {
                surface: tok,
                context: String::new(),
                tried: Vec::new(),
            });
        }
    }

    LexiconAugmentation {
        added,
        supporting,
        missing_oov,
    }
}

/// Ground an OOV `surface` against the committed lexicon via the **form text index** (BM25/token) — the
/// primary `LexiconBacked` path (`docs/notes/d63-lexicon-augmentation.md` §6a). Runs the active
/// `core:TextIndex` over `lexicon:form`, maps each hit entry to the ontology concept it aliases (its
/// `lexicon:sem`), **sums BM25 score per concept**, and returns the top concept + a rough confidence
/// (its share of the total score) — the disambiguation step. `None` if no form index is active, the
/// query has no hits, or no hit resolves to a concept.
fn ground_via_form_index(head: &Arc<Layer>, surface: &str) -> Option<(Iri, f32)> {
    let form_prop = Iri::parse("urn:eigenius:lexicon:form").ok()?;
    let sem_prop = Iri::parse("urn:eigenius:lexicon:sem").ok()?;
    let active = resolve_active_text_indexes(head);
    let idx = active.iter().find(|a| a.target_property == form_prop)?;
    let analyzer = analyzer_for(&idx.analyzer)?;
    let hits = run_text_search(
        head,
        head.storage().text_index.as_ref(),
        &idx.iri,
        analyzer.as_ref(),
        surface,
    )
    .ok()?;

    // Aggregate BM25 score per concept the matched entries alias. `sem` survives persist as either a
    // `ResourceRef` (in-memory) or a `String` IRI (CBOR round-trip collapses it) — accept both.
    let mut by_concept: BTreeMap<Iri, f32> = BTreeMap::new();
    for h in &hits {
        let Some(entry) = head.resolve(&h.subject) else {
            continue;
        };
        let concept = match entry.get(&sem_prop) {
            Some(Value::ResourceRef(iri)) => iri.clone(),
            Some(Value::String(s)) => match Iri::parse(s) {
                Ok(i) => i,
                Err(_) => continue,
            },
            _ => continue,
        };
        *by_concept.entry(concept).or_default() += h.score;
    }
    if by_concept.is_empty() {
        return None;
    }
    let total: f32 = by_concept.values().sum();
    let (concept, top) = by_concept.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some((concept, if total > 0.0 { top / total } else { 0.0 }))
}

/// `core:description` (the concept-gloss index's target) and `eigentt:Axiom` (a *predicate*
/// denotation — a verb/adjective sense — which a nominal OOV must not ground to).
const DESCRIPTION: &str = "urn:eigenius:core:description";
const AXIOM_CLASS: &str = "urn:eigenius:eigentt:Axiom";

/// Ground `surface` against the committed lexicon's **concept `core:description` text index** (§6a
/// index c) — the SECONDARY recall path, tried when [`ground_via_form_index`] misses (a query term in a
/// *definition* but in no `lexicon:form`). Unlike the form path, a description hit **is** the concept
/// (the gloss sits on the noun class / instance / axiom directly — no entry→`sem` hop). Hits that are
/// verb/adjective **axioms** (`is_a` `eigentt:Axiom`) are dropped: an axiom is a predicate denotation,
/// and grounding a nominal OOV to a predicate would mint an incoherent alias. Eligibility is the
/// resolver's call (the index only retrieves); the kernel felicity gate backstops any residual
/// non-nominal concept when the alias is minted (`abbreviation_resources`). Returns the top-scored
/// eligible concept + confidence (its score share among eligible hits). `None` if no description index
/// is active, no hit, or no hit is an eligible concept.
fn ground_via_description_index(head: &Arc<Layer>, surface: &str) -> Option<(Iri, f32)> {
    let desc_prop = Iri::parse(DESCRIPTION).ok()?;
    let axiom_class = Iri::parse(AXIOM_CLASS).ok()?;
    let active = resolve_active_text_indexes(head);
    let idx = active.iter().find(|a| a.target_property == desc_prop)?;
    let analyzer = analyzer_for(&idx.analyzer)?;
    let hits = run_text_search(
        head,
        head.storage().text_index.as_ref(),
        &idx.iri,
        analyzer.as_ref(),
        surface,
    )
    .ok()?;

    // A description hit is the concept itself. Keep only eligible NOMINAL targets — drop predicate
    // axioms (verb/adjective senses). Rank by score (one description ⇒ one hit per concept); confidence
    // is the top hit's share of the eligible total.
    let mut best: Option<(Iri, f32)> = None;
    let mut total = 0.0f32;
    for h in &hits {
        let Some(concept) = head.resolve(&h.subject) else {
            continue;
        };
        if concept.is_instance_of(&axiom_class) {
            continue;
        }
        total += h.score;
        if best.as_ref().map(|(_, s)| h.score > *s).unwrap_or(true) {
            best = Some((h.subject.clone(), h.score));
        }
    }
    let (concept, top) = best?;
    Some((concept, if total > 0.0 { top / total } else { 0.0 }))
}

/// Append a tried method to a gap (fail-closed provenance on the residual).
fn gap_tried(mut gap: Gap, method: ResolutionMethod) -> Gap {
    gap.tried.push(method);
    gap
}

/// The **`LexiconBacked`** augmentation (Phase 2, §6a): run [`augment_document_only`], then try to ground
/// each residual OOV `Gap` against the committed lexicon's text indexes — the **form** index first
/// ([`ground_via_form_index`], the primary surface→concept path), then, on a miss, the concept-gloss
/// **description** index ([`ground_via_description_index`], secondary recall). A grounded gap becomes a
/// `RetrievalGrounded` [`LexicalBinding`] — an alias entry naming the concept (the abbreviation alias
/// model, reused) — and moves from `missing_oov` to `added`; an un-grounded gap stays a `Gap` with
/// `RetrievalGrounded` recorded in `tried` (fail-closed). Requires an active `core:TextIndex` over
/// `lexicon:form` (and/or `core:description`) in `base`'s chain; without one it degrades to `DocumentOnly`.
pub fn augment_lexicon_backed(
    base: &Arc<Layer>,
    document: &str,
    proposer: &dyn AbbreviationProposer,
    lemmatizer: &dyn Lemmatizer,
) -> LexiconAugmentation {
    let mut aug = augment_document_only(base, document, proposer, lemmatizer);
    let Ok(entry_class) = Iri::parse(LEXICAL_ENTRY) else {
        return aug;
    };
    let mut still_missing = Vec::new();
    for gap in std::mem::take(&mut aug.missing_oov) {
        // Form index (primary) → concept-description index (secondary recall). Both yield a
        // (concept, confidence); the minting + kernel gate are identical downstream.
        let grounded = ground_via_form_index(base, &gap.surface)
            .or_else(|| ground_via_description_index(base, &gap.surface));
        let Some((concept, confidence)) = grounded else {
            still_missing.push(gap_tried(gap, ResolutionMethod::RetrievalGrounded));
            continue;
        };
        // Emit the alias entry naming the grounded concept (reuse the abbreviation alias model: the OOV
        // surface is its own "long form"). Fail-closed: if emission fails, keep the gap.
        let binding = AbbreviationBinding {
            abbr: gap.surface.as_str(),
            long_form: gap.surface.as_str(),
            concept_iri: concept.as_str(),
            doc_ns: "urn:eigenius:doc",
        };
        let Some(resources) = abbreviation_resources(base, &binding) else {
            still_missing.push(gap_tried(gap, ResolutionMethod::RetrievalGrounded));
            continue;
        };
        let (entries, extra): (Vec<Resource>, Vec<Resource>) = resources
            .into_iter()
            .partition(|r| r.is_instance_of(&entry_class));
        aug.supporting.extend(extra);
        for proposed in entries {
            aug.added.push(LexicalBinding {
                proposed,
                provenance: Provenance {
                    surface: gap.surface.clone(),
                    long_form: None,
                    context: gap.context.clone(),
                    method: ResolutionMethod::RetrievalGrounded,
                    grounded_to: Some(concept.clone()),
                    confidence: Some(confidence),
                },
            });
        }
    }
    aug.missing_oov = still_missing;
    aug
}
