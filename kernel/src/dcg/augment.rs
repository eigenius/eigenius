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

use super::abbrev::{extract_abbreviations_with, AbbreviationProposer};
use super::glossary::{
    abbreviation_resources, glossary_resources, ground_abbreviation, AbbreviationBinding,
};
use super::lemmatizer::Lemmatizer;
use super::parse::Parser;
use super::segment::segment_sentences;
use super::segment::tokenize;

const LEXICAL_ENTRY: &str = "urn:eigenius:lexicon:LexicalEntry";

/// The **expected syntactic category** (coarse POS) of an OOV surface — the query-side signal that makes
/// description grounding POS-aware (`docs/notes/d63-lexicon-augmentation.md` §6a, the (B) step). It
/// selects which concept *kind* is an eligible grounding target (nominal → a class/instance;
/// verb/adjective → a predicate `eigentt:Axiom`) and which lexical category the alias is minted with.
/// Coarse on purpose: the axiom's own arrow supplies verb arity (intransitive vs transitive), so the
/// proposer need only name the part of speech.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedCat {
    /// A referring nominal — grounds to a class/instance, minted `cat_n`. The default (a `LexiconBacked`
    /// run with no live proposer degrades to this — the nominal-only (A) behaviour).
    Nominal,
    /// A verb — grounds to a predicate `eigentt:Axiom`, minted with the verb category its arrow implies.
    Verb,
    /// A predicative adjective — grounds to a predicate `eigentt:Axiom`, minted `S[adj]\NP`.
    Adjective,
}

/// The **untrusted** part-of-speech proposer for an OOV surface (§6a, the (B) step): given the surface
/// and its document context, propose the [`ExpectedCat`] the grammar expects there — the query-side
/// category the resolver matches concept hits against. Same "propose, kernel gates" contract as
/// [`AbbreviationProposer`](super::glossary::AbbreviationProposer) / the anaphora
/// [`Proposer`](super::parse::Proposer): a wrong proposal only mis-selects a candidate, which the
/// felicity gate then rejects — it never commits an ill-typed alias. [`NominalCategoryProposer`] is the
/// deterministic default; the live `AnthropicCategoryProposer` (`use-llm`) reads the sentence.
pub trait CategoryProposer {
    /// Propose the OOV's expected category, or `None` to abstain (→ the resolver's nominal default).
    fn propose_category(&self, surface: &str, context: &str) -> Option<ExpectedCat>;
}

/// The default, deterministic [`CategoryProposer`]: always [`ExpectedCat::Nominal`]. With it,
/// `LexiconBacked` grounding stays nominal-only — the (A) behaviour — with no LLM dependency.
pub struct NominalCategoryProposer;

impl CategoryProposer for NominalCategoryProposer {
    fn propose_category(&self, _surface: &str, _context: &str) -> Option<ExpectedCat> {
        Some(ExpectedCat::Nominal)
    }
}

/// How a proposed lexical entry was resolved — a **trust signal** on the binding, most-trusted first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionMethod {
    /// The document itself defined it (Schwartz-Hearst / a definitional pattern). Deterministic.
    DefinitionExtracted,
    /// A deterministic named-entity apposition (`<common-noun-head> <Name>`) recognized it as a doc-local
    /// named individual (D63 named-entity glossary source). Deterministic, same trust as extraction.
    NameRecognized,
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
    // these; `DocumentOnly` reports them as-is. Each gap carries the sentence it occurs in as `context` —
    // the window a `CategoryProposer` reads to infer the OOV's expected category (§6a, the (B) step).
    let index = Parser::build(Arc::clone(base));
    let sentences = segment_sentences(document);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut missing_oov = Vec::new();
    for tok in tokenize(document) {
        let t = tok.trim().to_lowercase();
        if t.is_empty() || known.contains(&t) || !seen.insert(t.clone()) {
            continue;
        }
        if !index.has_token(&t, lemmatizer) {
            let context = sentences
                .iter()
                .find(|s| s.to_lowercase().contains(&t))
                .cloned()
                .unwrap_or_default();
            missing_oov.push(Gap {
                surface: tok,
                context,
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
/// `lexicon:sem`), keeps only concepts whose kind matches the OOV's `expected` category (nominal ⇒
/// non-axiom; verb/adjective ⇒ an `eigentt:Axiom` — so a fuzzy form hit on a verb entry can't ground a
/// nominal OOV to a predicate, the (B) step), **sums BM25 score per concept**, and returns the top
/// concept + a rough confidence (its score share) — the disambiguation step. `None` if no form index is
/// active, no hit, or no hit resolves to a concept of the expected kind.
fn ground_via_form_index(
    head: &Arc<Layer>,
    surface: &str,
    expected: ExpectedCat,
) -> Option<(Iri, f32)> {
    let form_prop = Iri::parse("urn:eigenius:lexicon:form").ok()?;
    let sem_prop = Iri::parse("urn:eigenius:lexicon:sem").ok()?;
    let axiom_class = Iri::parse(AXIOM_CLASS).ok()?;
    let want_axiom = matches!(expected, ExpectedCat::Verb | ExpectedCat::Adjective);
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
    // `ResourceRef` (in-memory) or a `String` IRI (CBOR round-trip collapses it) — accept both. Skip a
    // concept whose kind (axiom vs not) doesn't match `expected` — POS coherence for the mint downstream.
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
        let is_axiom = head
            .resolve(&concept)
            .map(|r| r.is_instance_of(&axiom_class))
            .unwrap_or(false);
        if is_axiom != want_axiom {
            continue;
        }
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
/// (the gloss sits on the noun class / instance / axiom directly — no entry→`sem` hop). Hits are matched
/// to the OOV's `expected` category (the (B) step): a **nominal** OOV keeps only non-axiom concepts (a
/// class/instance); a **verb/adjective** OOV keeps only predicate `eigentt:Axiom` concepts — so a nominal
/// OOV never grounds to a predicate, nor a predicate OOV to a class. Eligibility is the resolver's call
/// (the index only retrieves); the kernel felicity gate backstops the minted alias. Returns the
/// top-scored eligible concept + confidence (its score share among eligible hits). `None` if no
/// description index is active, no hit, or no hit matches `expected`.
fn ground_via_description_index(
    head: &Arc<Layer>,
    surface: &str,
    expected: ExpectedCat,
) -> Option<(Iri, f32)> {
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

    // A description hit is the concept itself. Keep only hits whose kind matches `expected`: nominal ⇒
    // non-axiom (a class/instance); verb/adjective ⇒ an `eigentt:Axiom` (a predicate denotation). Rank
    // by score (one description ⇒ one hit per concept); confidence is the top hit's share of the total.
    let want_axiom = matches!(expected, ExpectedCat::Verb | ExpectedCat::Adjective);
    let mut best: Option<(Iri, f32)> = None;
    let mut total = 0.0f32;
    for h in &hits {
        let Some(concept) = head.resolve(&h.subject) else {
            continue;
        };
        if concept.is_instance_of(&axiom_class) != want_axiom {
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

/// Mint the document-scoped alias entry for an OOV `surface` grounded to a **predicate** concept (a
/// verb/adjective `eigentt:Axiom`). Reuses a committed **sibling** entry's categorial type — the
/// converter already built the correct verb/adjective `lexicon:cat` for that axiom's lemma forms, so
/// cloning one (swapping the surface) is exact, where reconstructing the cat here would duplicate that
/// logic and risk divergence. Siblings are found via the triple index (`scan_chain` over `lexicon:sem`
/// = the axiom — resource-typed, so indexed even after the persist String-collapse). Deterministic:
/// the first sibling by sorted IRI. `None` if the axiom has no committed entry to clone (fail-closed).
fn predicate_alias_resources(head: &Arc<Layer>, surface: &str, concept: &Iri) -> Option<Resource> {
    let sem_prop = Iri::parse("urn:eigenius:lexicon:sem").ok()?;
    let cat_prop = Iri::parse("urn:eigenius:lexicon:cat").ok()?;
    let sem_type_prop = Iri::parse("urn:eigenius:lexicon:sem_type").ok()?;
    let p = |s: &str| Iri::parse(s).expect("valid well-known iri");

    // Sibling entries: committed lexical entries whose `sem` IS this axiom (its WordNet lemma forms).
    let siblings = crate::layer::scan_chain(head, &sem_prop, concept);
    for sib_iri in &siblings {
        let Some(sib) = head.resolve(sib_iri) else {
            continue;
        };
        let (Some(cat), Some(sem_type)) = (sib.get(&cat_prop), sib.get(&sem_type_prop)) else {
            continue;
        };
        let key = surface
            .trim()
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "_");
        let e_iri = Iri::parse(&format!("urn:eigenius:doc:e_{key}")).ok()?;
        let mut e = Resource::new(e_iri);
        e.set(
            p("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(p(LEXICAL_ENTRY))]),
        );
        e.set(
            p("urn:eigenius:lexicon:form"),
            Value::String(surface.to_string()),
        );
        e.set(p("urn:eigenius:lexicon:cat"), cat.clone());
        e.set(
            p("urn:eigenius:lexicon:sem"),
            Value::ResourceRef(concept.clone()),
        );
        e.set(p("urn:eigenius:lexicon:sem_type"), sem_type.clone());
        e.set(
            p("urn:eigenius:lexicon:sense"),
            Value::String(format!("doc:{key}")),
        );
        e.set(
            p("urn:eigenius:lexicon:grade"),
            Value::ResourceRef(p("urn:eigenius:reflection:epistemic:declared")),
        );
        return Some(e);
    }
    None
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
    category_proposer: &dyn CategoryProposer,
    lemmatizer: &dyn Lemmatizer,
) -> LexiconAugmentation {
    let mut aug = augment_document_only(base, document, proposer, lemmatizer);
    let Ok(entry_class) = Iri::parse(LEXICAL_ENTRY) else {
        return aug;
    };
    let Ok(axiom_class) = Iri::parse(AXIOM_CLASS) else {
        return aug;
    };
    let mut still_missing = Vec::new();
    for gap in std::mem::take(&mut aug.missing_oov) {
        // The (untrusted) POS proposer names the OOV's expected category; the resolver matches concept
        // kinds against it — a nominal never grounds to a predicate, nor vice versa. No proposal ⇒ nominal
        // (the (A) default). §6a, the (B) step.
        let expected = category_proposer
            .propose_category(&gap.surface, &gap.context)
            .unwrap_or(ExpectedCat::Nominal);
        // Form index (primary surface→concept) → gloss index (secondary, definition mentions). Both are
        // now POS-aware: they return only a concept whose kind matches `expected`, so a nominal OOV
        // grounds to a class and a verb/adjective OOV to its axiom, on either path.
        let grounded = ground_via_form_index(base, &gap.surface, expected)
            .or_else(|| ground_via_description_index(base, &gap.surface, expected));
        let Some((concept, confidence)) = grounded else {
            still_missing.push(gap_tried(gap, ResolutionMethod::RetrievalGrounded));
            continue;
        };
        // Mint by the concept's KIND (fail-closed if it can't): a predicate `eigentt:Axiom` → clone a
        // committed sibling's verb/adjective cat; a class/instance → the nominal `cat_n`/`cat_np` alias
        // model. The kernel felicity gate re-checks at `add_resource`.
        let is_axiom = base
            .resolve(&concept)
            .map(|r| r.is_instance_of(&axiom_class))
            .unwrap_or(false);
        let entry = if is_axiom {
            predicate_alias_resources(base, &gap.surface, &concept)
        } else {
            let binding = AbbreviationBinding {
                abbr: gap.surface.as_str(),
                long_form: gap.surface.as_str(),
                concept_iri: concept.as_str(),
                doc_ns: "urn:eigenius:doc",
            };
            abbreviation_resources(base, &binding)
                .and_then(|rs| rs.into_iter().find(|r| r.is_instance_of(&entry_class)))
        };
        let Some(proposed) = entry else {
            still_missing.push(gap_tried(gap, ResolutionMethod::RetrievalGrounded));
            continue;
        };
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
    aug.missing_oov = still_missing;
    aug
}

// ───────────────────── live Anthropic POS proposer (use-llm feature) ─────────────────────

#[cfg(feature = "use-llm")]
mod category_llm {
    use super::{CategoryProposer, ExpectedCat};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// The model's structured reply: the part of speech of the target word as used in the sentence.
    #[derive(Deserialize, JsonSchema)]
    struct PosReply {
        /// The part of speech of the target word AS USED in the sentence — exactly one of
        /// "noun", "verb", or "adjective" (any nominal, including a proper noun, is "noun").
        part_of_speech: String,
    }

    /// A [`CategoryProposer`] backed by Anthropic Claude via the direct tool-use client
    /// ([`crate::dcg::anthropic_client`]) — the (B) source (§6a). It reads the OOV's sentence and names
    /// the part of speech the grammar expects there; the resolver matches concept kinds against it and
    /// the kernel felicity gate re-checks the minted alias. On any error it abstains (`None`), so the
    /// resolver falls back to its nominal default.
    pub struct AnthropicCategoryProposer {
        api_key: String,
        model: String,
    }

    impl AnthropicCategoryProposer {
        pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
            Self {
                api_key: api_key.into(),
                model: model.into(),
            }
        }

        /// From `$ANTHROPIC_API_KEY`, defaulting to a fast model. `None` if the key is unset.
        pub fn from_env() -> Option<Self> {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self::new(k, crate::dcg::anthropic_client::DEFAULT_MODEL))
        }

        fn ask(&self, instructions: &str) -> Option<PosReply> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            match rt.block_on(
                crate::dcg::anthropic_client::anthropic_structured::<PosReply>(
                    &self.api_key,
                    &self.model,
                    instructions,
                ),
            ) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("anthropic category-proposer error: {e}");
                    None
                }
            }
        }
    }

    impl CategoryProposer for AnthropicCategoryProposer {
        fn propose_category(&self, surface: &str, context: &str) -> Option<ExpectedCat> {
            let prompt = format!(
                "In the sentence below, what is the part of speech of the word \"{surface}\" AS USED \
                 there? Answer `part_of_speech` with exactly one of: \"noun\", \"verb\", \"adjective\". \
                 A proper noun, named entity, or any nominal is \"noun\".\n\nSentence:\n{context}"
            );
            let reply = self.ask(&prompt)?;
            match reply.part_of_speech.trim().to_lowercase().as_str() {
                "verb" => Some(ExpectedCat::Verb),
                "adjective" | "adj" => Some(ExpectedCat::Adjective),
                "noun" | "nominal" => Some(ExpectedCat::Nominal),
                _ => None,
            }
        }
    }
}

#[cfg(feature = "use-llm")]
pub use category_llm::AnthropicCategoryProposer;
