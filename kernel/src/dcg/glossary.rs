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

//! **Document glossary emission** (D63 Phase 1 — `docs/notes/d63-document-preprocessing-scope.md`).
//! The abbreviation-definition preprocessor (Stage A) extracts `ABBR → grounded concept` bindings from
//! a document (`microsatellite instability (MSI)` → `MSI`, grounded to `umlscui:C0920269`). This module
//! turns each binding into the **one** `lexicon:LexicalEntry` a chained, document-scoped lexicon layer
//! needs: the abbreviation is an **alias** of its grounded concept and carries that concept's own
//! lexical category (the *abbreviation/alias model*, `crates/eigenius-umls/src/convert.rs`).
//!
//! Keyed on the grounded concept's ontological kind (D62 named-individual typing):
//!
//! 1. a **class / phenomenon** (a UMLS CUI, `microsatellite instability`) → a **common noun**
//!    `cat_n(concept, Num)` whose `sem` IS the class. A bare argument comes from the general
//!    bare-plural/bare-mass shift; a prenominal classifier (`MSI cell lines`) from `compound_kind`.
//!    The number class is inherited from the long form's **head noun**: a *mass* head (`microsatellite
//!    instability` → `instability`, uncountable in the WordNet countability lexicon) licenses the
//!    bare-singular-mass subject; otherwise `num_any` (a count noun that needs a determiner). This is
//!    the intended home the UMLS importer defers bare-argument abbreviations to.
//! 2. a **named individual** (an HGNC gene symbol like `WRN`, imported as an instance) → a proper-noun
//!    `cat_np(sty, sg)` alias naming the SAME instance (no new individual is minted).
//!
//! There is no parser/grammar change — the `mass`/`num_any` shifts and `compound_kind` already exist
//! (`lexicon:Num::mass`, `bare_mass_nps`, D62 CNL). It is the "add, not shadow" form.
//!
//! Unlike the WordNet/UMLS importers (which render ESL *text* that is compiled at load), these are
//! built **directly as in-memory [`Resource`]s** — the load path takes CBOR/Eigon-JSON resources, so
//! there is no reason to round-trip through ESL. The category term is encoded with
//! [`encode_type`](crate::program::eigentt_type_mirror::encode_type), the same D47 encoding ESL emits.

use super::abbrev::{
    extract_abbreviations_with, AbbrDef, AbbreviationProposer, NoAbbreviationProposer,
};
use std::collections::BTreeSet;
use std::sync::Arc;

use super::augment::{LexicalBinding, LexiconAugmentation, Provenance, ResolutionMethod};
use super::category::{denote_cat, inductive_iri, is_adjective_cat};
use super::named_entity::extract_named_entities_with;
use crate::layer::{
    normalize_value, resolve_active_value_indexes, Layer, LayerBuilder, LayerStorage,
};
use crate::nbe::term::Exp;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::ontology::Iri;
use crate::program::eigentt_type_mirror::{decode_type, encode_type};

// ── Stage A: abbreviation-definition extraction (Schwartz & Hearst 2003) ─────────
//
// Deterministic, high-precision extraction of `Long Form (SHORT)` definitions — the parenthetical
// pattern our biomedical corpus introduces its abbreviations with (`microsatellite instability
// (MSI)`). This is the deterministic-first half of Stage A; the LLM fallback (for non-parenthetical
// definitions) is a later orchestration step. Reference: A. S. Schwartz & M. A. Hearst, "A Simple
// Algorithm for Identifying Abbreviation Definitions in Biomedical Text," Pacific Symposium on
// Biocomputing 2003 (verify the exact identifier before citing as a load-bearing `.bib` anchor).

// ── Stage A: LLM fallback (non-parenthetical definitions) ────────────────────────
//
// Schwartz-Hearst only catches `Long Form (SHORT)` parentheticals. Definitions introduced without a
// paren ("MSI stands for microsatellite instability"; "we refer to … as MSI") need a proposer. Like
// the sense reranker (`sense_ranker.rs`), the proposer is **untrusted**: it suggests `(short, long)`
// pairs which are validated ([`extract_abbreviations_with`] — the short form must actually occur in
// the text, rejecting hallucinations) and then flow through the SAME ground → emit → kernel-gate path
// as the deterministic ones, so a plausible-but-wrong proposal is caught downstream. Algorithm-in-Rust
// first (behind `use-llm`); the orchestrator refactor comes after the algorithm is validated.

// ── Stage A: grounding (long form → an existing concept, retrieve-first) ─────────

/// Every `lexicon:LexicalEntry` whose surface `form` matches (deduped by resource IRI, in index
/// order). Index-driven (a value-index probe) on the served chain; an eager resource scan only for
/// small in-memory layers with no active index (mirrors `Parser::build`'s fallback, so it never
/// scans the 7.6M-resource served lexicon).
fn entries_for_form(layer: &Arc<Layer>, form: &str) -> Vec<Arc<Resource>> {
    let (Ok(form_prop), Ok(entry_class)) = (
        Iri::parse("urn:eigenius:lexicon:form"),
        Iri::parse("urn:eigenius:lexicon:LexicalEntry"),
    ) else {
        return Vec::new();
    };
    let mut out: Vec<Arc<Resource>> = Vec::new();
    let mut seen: BTreeSet<Iri> = BTreeSet::new();

    if let Some(active) = resolve_active_value_indexes(layer)
        .into_iter()
        .find(|a| a.target_property == form_prop)
    {
        let key = normalize_value(&active.normalizer, form);
        for hit in layer.storage().value_index.lookup(&active.iri, &key) {
            let Ok((subject, _defining)) = hit else {
                continue;
            };
            let Some(r) = layer.resolve(&subject) else {
                continue;
            };
            // Shadow safety: a LexicalEntry whose form still normalizes to the queried key.
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            let Some(Value::String(f)) = r.get(&form_prop) else {
                continue;
            };
            if normalize_value(&active.normalizer, f) != key {
                continue;
            }
            if seen.insert(subject.clone()) {
                out.push(r);
            }
        }
    } else {
        let key = form.trim().to_lowercase();
        for (id, r) in layer.iter_all_resources() {
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            if let Some(Value::String(f)) = r.get(&form_prop) {
                if f.trim().to_lowercase() == key && seen.insert(id.clone()) {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// Every concept a surface `form` denotes in the chain — the `sem` of each matching
/// `lexicon:LexicalEntry` (deduped, in index order; a `cat_n` common-noun entry's `sem` IS the
/// concept class).
fn concepts_for_form(layer: &Arc<Layer>, form: &str) -> Vec<Iri> {
    let Ok(sem_prop) = Iri::parse("urn:eigenius:lexicon:sem") else {
        return Vec::new();
    };
    let read_sem = |r: &Resource| match r.get(&sem_prop) {
        Some(Value::String(s)) => Iri::parse(s).ok(),
        _ => None,
    };
    let mut out: Vec<Iri> = Vec::new();
    let mut seen: BTreeSet<Iri> = BTreeSet::new();
    for r in entries_for_form(layer, form) {
        if let Some(iri) = read_sem(&r) {
            if seen.insert(iri.clone()) {
                out.push(iri);
            }
        }
    }
    out
}

/// Ground a single long form to a concept (retrieve-first): the first concept the phrase denotes.
/// Prefer [`ground_abbreviation`] when a short form is available (it disambiguates and widens).
pub fn ground_long_form(layer: &Arc<Layer>, long_form: &str) -> Option<Iri> {
    concepts_for_form(layer, long_form).into_iter().next()
}

/// Ground an abbreviation to a concept, **ranked** (the grounding-precision + recall fixes):
///
/// - **Recall (point 2a):** try the minimal `long` form, then progressively fuller variants drawn from
///   `context` (the window before the paren) — so `MMR`'s minimal `mismatch repair` still grounds via
///   the lexicon's `DNA mismatch repair`.
/// - **Precision (point 1):** among the concepts the matched long form denotes, prefer the one that
///   ALSO carries the SHORT form as a surface string — the abbreviation cross-check. This picks
///   `microsatellite instability` = `C0920269` (which also has the atom `MSI`) over `C0796369`
///   (…Stability Assessment, which does not).
///
/// `None` on a genuine miss (no long-form variant matches any lexeme).
pub fn ground_abbreviation(
    layer: &Arc<Layer>,
    short: &str,
    long: &str,
    context: &str,
) -> Option<Iri> {
    // Long-form candidates: the minimal form first, then growing left toward the full context window.
    let ctx_words: Vec<&str> = context.split_whitespace().collect();
    let long_wc = long.split_whitespace().count().max(1);
    let mut candidates = vec![long.to_string()];
    for wc in (long_wc + 1)..=ctx_words.len() {
        candidates.push(ctx_words[ctx_words.len() - wc..].join(" "));
    }
    let long_concepts = candidates
        .iter()
        .map(|c| concepts_for_form(layer, c))
        .find(|cs| !cs.is_empty())?;

    // Abbreviation cross-check: prefer a concept that also carries the short form; else the first.
    let short_concepts: BTreeSet<Iri> = concepts_for_form(layer, short).into_iter().collect();
    long_concepts
        .iter()
        .find(|c| short_concepts.contains(*c))
        .cloned()
        .or_else(|| long_concepts.into_iter().next())
}

/// One abbreviation binding from Stage-A extraction: the surface `abbr`, the `long_form` it was
/// defined as (its head noun's countability sets the emitted number class), the `concept_iri` it is
/// grounded to (already resolvable in the chain — a UMLS CUI class or named individual, or a fresh
/// document-local class on a grounding miss), and the `doc_ns` IRI stem the emitted resource is minted
/// under (e.g. `"urn:eigenius:doc:<docid>"`, per-document so distinct documents don't collide).
pub struct AbbreviationBinding<'a> {
    pub abbr: &'a str,
    pub long_form: &'a str,
    pub concept_iri: &'a str,
    pub doc_ns: &'a str,
}

/// IRI-local-safe form of a surface abbreviation (lower-cased, non-alphanumerics → `_`).
fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Is the single-word `lemma` a mass/uncountable noun in `layer` — i.e. does it carry a
/// `cat_n(_, mass)` lexical entry (the general countability lexicon's shape, or the demo's
/// `e_instability`)? An abbreviation inherits its long form's head-noun countability, so a `mass`
/// head (`microsatellite instability` → `instability`) licenses the bare-singular-mass subject.
fn form_is_mass(layer: &Arc<Layer>, lemma: &str) -> bool {
    entries_for_form(layer, lemma)
        .iter()
        .filter_map(|r| entry_cat(layer, r))
        .any(|cat| cat_is_mass(&cat))
}

/// Decode a `lexicon:LexicalEntry`'s `cat` property back to a category `Exp` (the D47 type mirror).
fn entry_cat(layer: &Arc<Layer>, entry: &Resource) -> Option<Exp> {
    let cat_prop = Iri::parse("urn:eigenius:lexicon:cat").ok()?;
    decode_type(entry.get(&cat_prop)?, layer).ok()
}

/// True iff `cat` is `cat_n(_, mass)` — a mass/uncountable common noun.
fn cat_is_mass(cat: &Exp) -> bool {
    matches!(cat, Exp::InductiveCtor(_, name, args)
        if name == "cat_n" && args.len() == 2
            && matches!(&args[1], Exp::InductiveCtor(_, num, _) if num == "mass"))
}

/// The class(es) a resource is a *direct instance of* — its `is_a` targets excluding `core:Class`
/// itself. For a UMLS named individual (`resource umlscui:C : umlssty:T`) this is its semantic-type
/// class(es); for a class node (`is_a = [core:Class]`) it is empty.
///
/// A reference target is an IRI string, read through an accessor as [`Resource::is_instance_of`]
/// does. This was once two shapes, and matching only the in-memory one silently dropped a
/// persisted named individual's type, misclassifying it as a bare class (→ a common-noun alias
/// instead of the proper-noun one). That is the bug the accessor discipline prevents.
fn instance_type_classes(r: &Resource) -> Vec<Iri> {
    let (Ok(is_a), Ok(class)) = (Iri::parse(wk::IS_A), Iri::parse(wk::CLASS)) else {
        return Vec::new();
    };
    match r.get(&is_a) {
        Some(Value::Array(vs)) => vs
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Iri::parse(s).ok(),
                _ => None,
            })
            .filter(|iri| *iri != class)
            .collect(),
        _ => Vec::new(),
    }
}

/// Build the document-scoped lexical entry for one abbreviation binding — the abbreviation as an
/// **alias** of its grounded concept, carrying that concept's own lexical category (the alias model).
/// Returns the single entry to `add_resource` into a chained doc layer, or `None` if the
/// `lexicon:Cat`/`Num` decls or the concept IRI don't resolve against `layer`.
///
/// Keyed on the concept's ontological kind (D62): a **class/phenomenon** → a common noun
/// `cat_n(concept, mass|num_any)`, `sem` = the class (bare argument via the bare-plural/-mass shift;
/// prenominal classifier via `compound_kind`); a **named individual** → a proper noun
/// `cat_np(sty, sg)`, `sem` = the SAME instance (no new individual is minted). The `mass` vs `num_any`
/// choice is inherited from the long form's head-noun countability. The entry passes the felicity gate
/// at commit (fail-closed on a bad grounding).
pub fn abbreviation_resources(
    layer: &Arc<Layer>,
    binding: &AbbreviationBinding,
) -> Option<Vec<Resource>> {
    let cat_decl = inductive_iri(layer, "urn:eigenius:lexicon:Cat")?;
    let num_decl = inductive_iri(layer, "urn:eigenius:lexicon:Num")?;
    let concept = Iri::parse(binding.concept_iri).ok()?;
    let class_iri = Iri::parse(wk::CLASS).ok()?;

    // Ontological kind of the grounded concept: a named individual (an instance of some domain class)
    // vs a class/phenomenon (`is_a core:Class`). A bare/unresolved concept IRI defaults to class — the
    // grounding-miss shape (a fresh `doc:class_*` rooted at Entity, minted by `glossary_resources`).
    let concept_res = layer.resolve(&concept);
    let type_classes = concept_res
        .as_ref()
        .map(|r| instance_type_classes(r))
        .unwrap_or_default();
    let is_individual = concept_res
        .as_ref()
        .map(|r| !r.is_instance_of(&class_iri) && !type_classes.is_empty())
        .unwrap_or(false);

    // Build the (cat, sem) for the abbreviation as an alias of the concept. `sem_type = ⟦cat⟧`
    // (`denote_cat`) so the felicity gate's `type_eq(denote_cat(cat), sem_type)` holds by construction.
    let (cat, sem) = if is_individual {
        // Proper-noun alias naming the SAME instance; cat_np's type is the concept's semantic-type class.
        let sty = Exp::EigonClass(type_classes[0].clone());
        let sg = Exp::InductiveCtor(num_decl, "sg".into(), Vec::new());
        let cat = Exp::InductiveCtor(cat_decl, "cat_np".into(), vec![sty, sg]);
        (cat, Value::iri(&concept.clone()))
    } else {
        // Common-noun alias whose sem IS the class; number class from the long form's head noun.
        let head = binding
            .long_form
            .split_whitespace()
            .last()
            .unwrap_or(binding.long_form);
        let num_name = if form_is_mass(layer, head) {
            "mass"
        } else {
            "num_any"
        };
        let num = Exp::InductiveCtor(num_decl, num_name.into(), Vec::new());
        let concept_ty = Exp::EigonClass(concept.clone());
        let cat = Exp::InductiveCtor(cat_decl, "cat_n".into(), vec![concept_ty, num]);
        (cat, Value::iri(&concept.clone()))
    };
    let codec = crate::program::eigentt_type_mirror::CodecNames::from_layer(layer);
    let cat_val = encode_type(&cat, &codec).ok()?;
    let sem_type_val = encode_type(&denote_cat(&cat).ok()?, &codec).ok()?;

    let key = slug(binding.abbr);
    let e_iri = Iri::parse(&format!("{}:e_{key}", binding.doc_ns)).ok()?;
    let p = |s: &str| Iri::parse(s).expect("valid well-known iri");

    let mut e = Resource::new(e_iri);
    e.set(
        p(wk::IS_A),
        Value::Array(vec![Value::String(
            p("urn:eigenius:lexicon:LexicalEntry").as_str().to_string(),
        )]),
    );
    e.set(
        p("urn:eigenius:lexicon:form"),
        Value::String(binding.abbr.to_string()),
    );
    e.set(p("urn:eigenius:lexicon:cat"), cat_val);
    e.set(p("urn:eigenius:lexicon:sem"), sem);
    e.set(p("urn:eigenius:lexicon:sem_type"), sem_type_val);
    e.set(
        p("urn:eigenius:lexicon:sense"),
        Value::String(format!("doc:{key}")),
    );

    Some(vec![e])
}

/// The full document glossary for a set of extracted definitions: for each, **ground** the
/// abbreviation ([`ground_abbreviation`]) and **emit** its alias `lexicon:LexicalEntry`
/// ([`abbreviation_resources`]); on a grounding **miss**, mint a fresh document-local class
/// `doc:class_<abbr> : lexicon:Entity` and bind to it (§7-3) — so the abbreviation still parses
/// (ungrounded but Entity-typed) rather than being dropped. Returns every resource to commit into the
/// document's chained glossary layer.
pub fn glossary_resources(layer: &Arc<Layer>, defs: &[AbbrDef]) -> Vec<Resource> {
    let mut out = Vec::new();
    for d in defs {
        let mut extra = Vec::new();
        let concept_iri = match ground_abbreviation(layer, &d.short_form, &d.long_form, &d.context)
        {
            Some(c) => c.to_string(),
            None => {
                // Grounding miss → a fresh doc-local class rooted at Entity (Declared, ungrounded).
                let fresh = format!("urn:eigenius:doc:class_{}", slug(&d.short_form));
                if let Ok(ci) = Iri::parse(&fresh) {
                    let p = |s: &str| Iri::parse(s).expect("valid well-known iri");
                    let mut cls = Resource::new(ci);
                    cls.set(p(wk::IS_A), Value::Array(vec![Value::iri(&p(wk::CLASS))]));
                    cls.set(
                        p(wk::PARENT_CLASSES),
                        Value::Array(vec![Value::String(
                            p("urn:eigenius:lexicon:Entity").as_str().to_string(),
                        )]),
                    );
                    cls.set(
                        p(wk::DESCRIPTION),
                        Value::String(format!(
                            "Fresh document-local class for the ungrounded abbreviation {} ({:?}). \
                             Minted by the abbreviation-definition preprocessor — no matching concept \
                             found (§7-3).",
                            d.short_form, d.long_form
                        )),
                    );
                    extra.push(cls);
                }
                fresh
            }
        };
        let binding = AbbreviationBinding {
            abbr: &d.short_form,
            long_form: &d.long_form,
            concept_iri: &concept_iri,
            doc_ns: "urn:eigenius:doc",
        };
        if let Some(rs) = abbreviation_resources(layer, &binding) {
            out.append(&mut extra);
            out.extend(rs);
        }
    }
    out
}

// ── Stage A: named-entity source (D63 `d63-named-entity-glossary-source.md`) ──────
//
// Deterministic apposition NER (`super::named_entity`) → doc-local **named individuals**. The recognizer
// needs the common-noun head test ([`is_common_noun`], injected); emission mints a head-typed individual
// and emits its `cat_np` proper-noun alias (the individual arm of [`abbreviation_resources`]), packaged
// as [`LexicalBinding`]s for the in-memory augment overlay — NOT a persistent doc layer (the overlay is
// the lighter, already-sweep-chained path; §3d).

/// Does `form` have a **common-noun** (`cat_n`) lexical entry in the chain?
pub fn is_common_noun(layer: &Arc<Layer>, form: &str) -> bool {
    entries_for_form(layer, form)
        .iter()
        .filter_map(|r| entry_cat(layer, r))
        .any(|c| matches!(&c, Exp::InductiveCtor(_, n, _) if n == "cat_n"))
}

/// Does `form` have an **adjective** (`S[adj]\NP`) lexical entry in the chain?
pub fn is_adjective(layer: &Arc<Layer>, form: &str) -> bool {
    entries_for_form(layer, form)
        .iter()
        .filter_map(|r| entry_cat(layer, r))
        .any(|c| is_adjective_cat(&c))
}

/// The apposition **head admissibility** test: a noun that is NOT (also) an adjective. Just "is a common
/// noun" does not discriminate — in the served lexicon nearly every surface has *some* noun sense — so
/// the load-bearing filter is rejecting attributive adjectives ("somatic MMR", "other DNA"). Verb heads
/// ("identified WRN") are rejected by the recognizer's recurrence requirement, not here (the noun/verb
/// homonym "project" must stay admissible).
pub fn is_apposition_head(layer: &Arc<Layer>, form: &str) -> bool {
    is_common_noun(layer, form) && !is_adjective(layer, form)
}

/// The concept the first **common-noun** (`cat_n`) entry for `form` denotes — the head noun's class,
/// used to TYPE a named individual minted under it (§3b head-typing). `None` if `form` has no common-noun
/// entry with a resolvable `sem`.
fn common_noun_concept(layer: &Arc<Layer>, form: &str) -> Option<Iri> {
    let sem_prop = Iri::parse("urn:eigenius:lexicon:sem").ok()?;
    for r in entries_for_form(layer, form) {
        let is_cat_n = entry_cat(layer, &r)
            .is_some_and(|c| matches!(&c, Exp::InductiveCtor(_, n, _) if n == "cat_n"));
        if !is_cat_n {
            continue;
        }
        if let Some(Value::String(s)) = r.get(&sem_prop) {
            return Iri::parse(s).ok();
        }
    }
    None
}

/// **Named-entity augmentation** — recognize `<head> <Name>` appositions in `document`
/// ([`extract_named_entities_with`], head-test = [`is_apposition_head`], admitted on ≥2 occurrences) and
/// emit each as a doc-local **named individual**: mint `urn:eigenius:doc:ni_<slug>` typed at the HEAD
/// noun's class ([`common_noun_concept`], else `lexicon:Entity`), and emit its `cat_np(head_type, sg)`
/// proper-noun alias (the individual arm of [`abbreviation_resources`], resolved over a throwaway
/// in-memory chain carrying the minted individual — `resolve` only, no index build). Returns a
/// [`LexiconAugmentation`] (entries in `added`, minted individuals in `supporting`) to MERGE into the
/// document's augmentation — the same in-memory overlay the OOV grounding uses, so no persistent layer is
/// committed.
pub fn named_entity_augmentation(base: &Arc<Layer>, document: &str) -> LexiconAugmentation {
    let Ok(entry_class) = Iri::parse("urn:eigenius:lexicon:LexicalEntry") else {
        return LexiconAugmentation::default();
    };
    let entity = Iri::parse("urn:eigenius:lexicon:Entity").ok();

    let names = extract_named_entities_with(document, |w| is_apposition_head(base, w));
    let mut added = Vec::new();
    let mut supporting = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for ne in names {
        let key = slug(&ne.surface);
        if !seen.insert(key.clone()) {
            continue;
        }
        // Head-typed individual: the concept the head common noun denotes, else lexicon:Entity.
        let Some(head_type) = common_noun_concept(base, &ne.head).or_else(|| entity.clone()) else {
            continue;
        };
        let Ok(ni_iri) = Iri::parse(&format!("urn:eigenius:doc:ni_{key}")) else {
            continue;
        };
        let p = |s: &str| Iri::parse(s).expect("valid well-known iri");
        let mut ni = Resource::new(ni_iri.clone());
        ni.set(
            p(wk::IS_A),
            Value::Array(vec![Value::iri(&head_type.clone())]),
        );
        ni.set(
            p(wk::DESCRIPTION),
            Value::String(format!(
                "Doc-local named individual (apposition NER, D63): {:?}.",
                ne.surface
            )),
        );

        // Emit the cat_np alias, resolving the individual over a throwaway in-memory chain (resolve only —
        // NO LexicalIndex::build, so no parent materialisation).
        let mut b = LayerBuilder::new("ne-emit", Some(Arc::clone(base)));
        if b.add_resource(ni.clone()).is_err() {
            continue;
        }
        let tmp = Arc::new(b.build(LayerStorage::in_memory()));
        let binding = AbbreviationBinding {
            abbr: &ne.surface,
            long_form: &ne.surface,
            concept_iri: ni_iri.as_str(),
            doc_ns: "urn:eigenius:doc",
        };
        let Some(rs) = abbreviation_resources(&tmp, &binding) else {
            continue;
        };
        let (entries, extra): (Vec<Resource>, Vec<Resource>) =
            rs.into_iter().partition(|r| r.is_instance_of(&entry_class));
        if entries.is_empty() {
            continue;
        }
        supporting.push(ni);
        supporting.extend(extra);
        for proposed in entries {
            added.push(LexicalBinding {
                proposed,
                provenance: Provenance {
                    surface: ne.surface.clone(),
                    long_form: None,
                    context: ne.surface.clone(),
                    method: ResolutionMethod::NameRecognized,
                    grounded_to: None,
                    confidence: None,
                },
            });
        }
    }

    LexiconAugmentation {
        added,
        supporting,
        missing_oov: Vec::new(),
    }
}

/// **Stage A → the document glossary** — the Stage-A→B seam of the D63 preprocessing pipeline
/// (`docs/notes/d63-document-preprocessing-scope.md`). Extract every abbreviation definition from a whole
/// `document` (deterministic Schwartz-Hearst ∪ the untrusted LLM `proposer`, fail-closed), ground each
/// against `base`, and emit the alias `lexicon:LexicalEntry` resources (+ any fresh doc-local classes)
/// for a chained, document-scoped lexicon layer.
///
/// The caller **commits** the returned resources onto `base` — with the storage of its choice
/// (`LayerStorage::with_persistent` on the served/persisted path so the value index populates lazily and
/// the parse resolves lazily, §7-2; `in_memory` for a small demo) — and builds a `Parser` over the
/// result to parse the document's body sentences (Stage B). Every emitted binding still passes the
/// felicity gate at commit, so a mis-extracted abbreviation is rejected, not silently used.
pub fn document_glossary_resources_with(
    base: &Arc<Layer>,
    document: &str,
    proposer: &dyn AbbreviationProposer,
) -> Vec<Resource> {
    glossary_resources(base, &extract_abbreviations_with(document, proposer))
}

/// [`document_glossary_resources_with`] using only the deterministic Schwartz-Hearst extractor (no LLM).
pub fn document_glossary_resources(base: &Arc<Layer>, document: &str) -> Vec<Resource> {
    document_glossary_resources_with(base, document, &NoAbbreviationProposer)
}

// ───────────────────── live Anthropic abbreviation proposer (use-llm feature) ─────────────────────

#[cfg(feature = "use-llm")]
mod anthropic {
    use super::{AbbrDef, AbbreviationProposer};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// The model's structured reply: the abbreviation definitions it found in the text.
    #[derive(Deserialize, JsonSchema)]
    struct AbbrevReply {
        /// One entry per abbreviation/acronym definition present in the text.
        definitions: Vec<AbbrevPair>,
    }
    #[derive(Deserialize, JsonSchema)]
    struct AbbrevPair {
        /// The abbreviation / short form EXACTLY as it appears in the text (e.g. "MSI").
        short_form: String,
        /// The full form it is defined as standing for (e.g. "microsatellite instability").
        long_form: String,
    }

    /// An [`AbbreviationProposer`] backed by Anthropic Claude via the direct tool-use client
    /// ([`crate::dcg::anthropic_client`]). It proposes definitions the Schwartz-Hearst parenthetical
    /// extractor misses (non-parenthetical introductions). On any error it proposes nothing, so the
    /// deterministic extraction stands alone.
    pub struct AnthropicAbbreviationProposer {
        api_key: String,
        model: crate::dcg::anthropic_client::ModelConfig,
    }

    impl AnthropicAbbreviationProposer {
        pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
            Self::with_config(
                api_key,
                crate::dcg::anthropic_client::ModelConfig::with_model(model),
            )
        }

        /// Build with an explicit [`ModelConfig`] — how a formalization run selects the model it
        /// wants, and what a recorded draw names as the answerer (D71 §7.1 / §9).
        pub fn with_config(
            api_key: impl Into<String>,
            model: crate::dcg::anthropic_client::ModelConfig,
        ) -> Self {
            Self {
                api_key: api_key.into(),
                model,
            }
        }

        /// From `$ANTHROPIC_API_KEY`, defaulting to a fast model. `None` if the key is unset.
        pub fn from_env() -> Option<Self> {
            Self::from_env_with(Default::default())
        }

        /// From `$ANTHROPIC_API_KEY` with an explicit [`ModelConfig`]. The formalization service
        /// threads one config to every proposer in a run, so a draw's recorded model is the run's,
        /// not a per-seam default (D71 §7.1 / §9).
        pub fn from_env_with(cfg: crate::dcg::anthropic_client::ModelConfig) -> Option<Self> {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self::with_config(k, cfg.clone()))
        }

        fn ask(&self, instructions: &str) -> Option<AbbrevReply> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            match rt.block_on(crate::dcg::anthropic_client::anthropic_structured::<
                AbbrevReply,
            >(&self.api_key, &self.model, instructions))
            {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("anthropic abbreviation-proposer error: {e}");
                    None
                }
            }
        }
    }

    impl AbbreviationProposer for AnthropicAbbreviationProposer {
        fn propose(&self, text: &str) -> Vec<AbbrDef> {
            let prompt = format!(
                "Find every abbreviation/acronym DEFINITION in the text below — each place a short \
                 form is introduced as standing for a longer form, INCLUDING non-parenthetical \
                 introductions (e.g. \"X stands for …\", \"we refer to … as X\", \"…, abbreviated X\"). \
                 Return `definitions`: for each, the `short_form` exactly as written and the \
                 `long_form` it stands for. Do NOT invent abbreviations that are not defined in the \
                 text; return an empty list if there are none.\n\nText:\n{text}"
            );
            let Some(reply) = self.ask(&prompt) else {
                return Vec::new();
            };
            reply
                .definitions
                .into_iter()
                .map(|p| AbbrDef {
                    short_form: p.short_form.trim().to_string(),
                    // The proposer returns the full long form directly, so the grounding context IS
                    // the long form (no widening needed, unlike the Schwartz-Hearst minimal form).
                    context: p.long_form.trim().to_string(),
                    long_form: p.long_form.trim().to_string(),
                })
                .collect()
        }
    }
}

#[cfg(feature = "use-llm")]
pub use anthropic::AnthropicAbbreviationProposer;

// ── Stage E: notation recognition — the FIFTH extraction source (D101 §step 2) ────
//
// `Ile380Thr` blocked 20 units of the XIAP report and recurs 29 times, and the apposition source
// never offered it: an interior capital is not Title-case, `p.Ile380Thr` does not begin uppercase,
// and the document never puts a noun head in front of either (`an`, `for`, `of`, `than`, a table
// pipe, or a verb). Recurrence was never consulted — candidacy failed first — so no threshold
// change could have reached it. Loosening the shape to admit it would also admit `mRNA` and
// `HeLa`, which that rule exists to reject.
//
// A notation is a different kind of evidence. `eigenius_notation::recognize` reads published
// grammars, so there is nothing to tune and nothing to guard.
//
// WHAT A RECOGNISED TOKEN BECOMES, in two tiers, because `variant:Allele` is identified by its
// reference ACCESSION ("one change on two references is two alleles, related by congruence, not
// identity" — variant.esl):
//
//   1. The accession is IN the token (`NM_001167.4:c.1141C>T`) → a `variant:Allele` with complete,
//      true identity, carrying start/end/ref_state/alt/hgvs as typed fields.
//   2. The accession is absent (`Ile380Thr`, 25 of this document's 29 distinct changes) → a
//      doc-local `lexicon:Entity` individual that closes the gap LEXICALLY, with the parse recorded
//      on it. No allele is minted, because the accession would have to be invented, and the
//      document names FIVE protein references (P98170 plus four orthologues). Binding one is a
//      judgement with a warrant — `justification:Declared` — and that belongs to the encoding
//      step, which has the document's context. A glossary closes gaps; it does not make claims.
//
// Either way the parse is on the resource, never in the name: `ni_ile380thr` as an opaque string
// would put the meaning back where the ad-hoc-predicate failure puts it.

/// Is the `variant` layer on this chain? It is NOT in `BOOTSTRAP_CHAIN` — it loads above the tip,
/// after `lexicon` (its classes subclass `lexicon:Entity`) — so the glossary must detect it and
/// degrade to tier 2 for everything rather than emit classes the chain cannot resolve.
fn variant_layer_present(base: &Arc<Layer>) -> bool {
    Iri::parse("urn:eigenius:variant:Allele")
        .ok()
        .and_then(|i| base.resolve(&i))
        .is_some()
}

/// The tokens of `document`, bare of surrounding punctuation that is not part of a notation.
/// `(Rigaud` → `Rigaud`; `p.Ile380Thr,` → `p.Ile380Thr`; `NM_001167.4:c.1141C>T` is left whole.
///
/// A dash-joined token yields BOTH the whole and its parts. `Thr369–Asn373` is one
/// whitespace-delimited token here, but the PARSER's tokenizer splits on the dash and asks the
/// lexicon for `Asn373` — so an entry keyed only on the whole range is keyed under a surface
/// nothing ever requests. That mismatch is why those endpoints stayed OOV after the range support
/// landed: recognising the range was necessary and not sufficient. Extra candidates cost nothing,
/// because `recognize` refuses whatever is not a notation (`ABE9-SpRY`, `Gyrd-Hansen`).
fn notation_tokens(document: &str) -> Vec<&str> {
    fn bare(t: &str) -> &str {
        t.trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | '`' | '*' | '|' | '«' | '»'
            ) || matches!(c, ',' | ';' | '!' | '?')
        })
        .trim_end_matches('.')
    }
    // Commas too: the parser's tokenizer splits `chrX:123,900,532` and asks the lexicon for
    // `chrX:123`, the single biggest remaining blocker at 11 units. Same mismatch as the dash.
    const DASHES: [char; 4] = ['-', '\u{2013}', '\u{2014}', ','];
    let mut out = Vec::new();
    for t in whitespace_tokens(document) {
        out.push(t);
        if t.contains(DASHES) {
            for part in t.split(DASHES) {
                let p = bare(part);
                if !p.is_empty() && p != t {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// The whitespace tokens alone, bare of surrounding punctuation — WITHOUT the dash parts.
///
/// The split is not cosmetic. Dash parts exist so a residue range closes the endpoints the
/// parser's tokenizer asks for, and [`notation_tokens`]'s consumer filters them through
/// `recognize`, which refuses a non-notation. A consumer with no such filter must not see them:
/// the name source accepts any recurring capitalised token, and on the dash parts it minted
/// `Gyrd` and `Hansen` as separate entities beside `Gyrd-Hansen`.
fn whitespace_tokens(document: &str) -> Vec<&str> {
    fn bare(t: &str) -> &str {
        t.trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | '`' | '*' | '|' | '«' | '»'
            ) || matches!(c, ',' | ';' | '!' | '?')
        })
        .trim_end_matches('.')
    }
    document
        .split_whitespace()
        .map(bare)
        .filter(|t| !t.is_empty())
        .collect()
}

/// **Notation augmentation** — recognize published notations in `document`
/// ([`eigenius_notation::recognize`]) and emit each as a doc-local individual with a `cat_np`
/// proper-noun alias, so a unit carrying `Ile380Thr` reaches the grammar instead of returning
/// `missing-lexeme` before it is parsed.
///
/// Tiered as the module comment above describes: a token carrying its reference accession becomes a
/// `variant:Allele`; everything else closes the gap lexically with its parse recorded. Falls back to
/// tier 2 for all tokens when the `variant` layer is absent from `base`.
pub fn notation_augmentation(base: &Arc<Layer>, document: &str) -> LexiconAugmentation {
    use eigenius_notation::{recognize, Notation};

    let Ok(entry_class) = Iri::parse("urn:eigenius:lexicon:LexicalEntry") else {
        return LexiconAugmentation::default();
    };
    let Some(entity) = Iri::parse("urn:eigenius:lexicon:Entity").ok() else {
        return LexiconAugmentation::default();
    };
    let has_variant = variant_layer_present(base);
    let p = |s: &str| Iri::parse(s).expect("valid well-known iri");

    let mut added = Vec::new();
    let mut supporting = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for tok in notation_tokens(document) {
        let Some(n) = recognize(tok) else { continue };
        let key = slug(tok);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        // OOV ONLY, and this guard is what makes that claim true rather than asserted. The
        // published UniProt accession pattern also admits gene symbols shaped `P2RY12`, and the
        // mitigation for that is precisely this: a surface the committed lexicon already supplies
        // is left alone, so a known gene symbol never reaches the recognizer's verdict. It is also
        // the "add, not shadow" rule the other sources follow — a second entry for a form the
        // lexicon has is a duplicate, not a closure.
        if !entries_for_form(base, tok).is_empty() {
            continue;
        }
        let Ok(ni_iri) = Iri::parse(&format!("urn:eigenius:doc:ni_{key}")) else {
            continue;
        };

        // Tier 1 needs the accession the notation itself states. `recognize` returns the parse, not
        // the qualifier, so the token is re-read for one: `<accession>:<c|g|p|n|m>.<change>`.
        let qualified = tok
            .split_once(':')
            .filter(|(acc, rest)| {
                !acc.is_empty()
                    && rest.len() > 2
                    && rest.as_bytes()[1] == b'.'
                    && matches!(rest.as_bytes()[0], b'c' | b'g' | b'p' | b'n' | b'm')
            })
            .map(|(acc, _)| acc.to_string());

        let mut ni = Resource::new(ni_iri.clone());
        let (class, note) = match (&n, has_variant, &qualified) {
            (Notation::ProteinChange(c), true, Some(acc)) => {
                ni.set(
                    p("urn:eigenius:variant:reference"),
                    Value::String(acc.clone()),
                );
                ni.set(
                    p("urn:eigenius:variant:start"),
                    Value::Integer(c.start as i64),
                );
                ni.set(p("urn:eigenius:variant:end"), Value::Integer(c.end as i64));
                ni.set(p("urn:eigenius:variant:alt"), Value::String(c.alt.clone()));
                if let Some(r) = &c.ref_state {
                    ni.set(
                        p("urn:eigenius:variant:ref_state"),
                        Value::String(r.clone()),
                    );
                }
                ni.set(
                    p("urn:eigenius:variant:hgvs"),
                    Value::String(tok.to_string()),
                );
                (
                    "urn:eigenius:variant:ProteinAllele",
                    "a protein allele, reference stated by the notation",
                )
            }
            (Notation::CodingChange(c), true, Some(acc)) => {
                ni.set(
                    p("urn:eigenius:variant:reference"),
                    Value::String(acc.clone()),
                );
                ni.set(
                    p("urn:eigenius:variant:start"),
                    Value::Integer(c.start as i64),
                );
                ni.set(p("urn:eigenius:variant:end"), Value::Integer(c.end as i64));
                ni.set(p("urn:eigenius:variant:alt"), Value::String(c.alt.clone()));
                if let Some(r) = &c.ref_state {
                    ni.set(
                        p("urn:eigenius:variant:ref_state"),
                        Value::String(r.clone()),
                    );
                }
                ni.set(
                    p("urn:eigenius:variant:hgvs"),
                    Value::String(tok.to_string()),
                );
                (
                    "urn:eigenius:variant:NucleotideAllele",
                    "a nucleotide allele, reference stated by the notation",
                )
            }
            (Notation::ResiduePosition { residue, position }, true, Some(acc)) => {
                ni.set(
                    p("urn:eigenius:variant:reference"),
                    Value::String(acc.clone()),
                );
                ni.set(
                    p("urn:eigenius:variant:start"),
                    Value::Integer(*position as i64),
                );
                ni.set(
                    p("urn:eigenius:variant:end"),
                    Value::Integer(*position as i64),
                );
                ni.set(
                    p("urn:eigenius:variant:ref_state"),
                    Value::String(residue.clone()),
                );
                (
                    "urn:eigenius:variant:Location",
                    "a location, reference stated by the notation",
                )
            }
            (
                Notation::ResidueRange {
                    start_residue,
                    start,
                    end,
                    ..
                },
                true,
                Some(acc),
            ) => {
                ni.set(
                    p("urn:eigenius:variant:reference"),
                    Value::String(acc.clone()),
                );
                ni.set(
                    p("urn:eigenius:variant:start"),
                    Value::Integer(*start as i64),
                );
                ni.set(p("urn:eigenius:variant:end"), Value::Integer(*end as i64));
                ni.set(
                    p("urn:eigenius:variant:ref_state"),
                    Value::String(start_residue.clone()),
                );
                (
                    "urn:eigenius:variant:Location",
                    "an interval, reference stated by the notation",
                )
            }
            // Tier 2: the parse is certain, the accession is not. Recorded, not typed as an allele.
            _ => (entity.as_str(), describe(&n)),
        };
        let Ok(class_iri) = Iri::parse(class) else {
            continue;
        };
        ni.set(p(wk::IS_A), Value::Array(vec![Value::iri(&class_iri)]));
        ni.set(
            p(wk::DESCRIPTION),
            Value::String(format!(
                "Doc-local individual, notation recognized (D101): {tok:?} — {note}."
            )),
        );

        let mut b = LayerBuilder::new("notation-emit", Some(Arc::clone(base)));
        if b.add_resource(ni.clone()).is_err() {
            continue;
        }
        let tmp = Arc::new(b.build(LayerStorage::in_memory()));
        let binding = AbbreviationBinding {
            abbr: tok,
            long_form: tok,
            concept_iri: ni_iri.as_str(),
            doc_ns: "urn:eigenius:doc",
        };
        let Some(rs) = abbreviation_resources(&tmp, &binding) else {
            continue;
        };
        let (entries, extra): (Vec<Resource>, Vec<Resource>) =
            rs.into_iter().partition(|r| r.is_instance_of(&entry_class));
        if entries.is_empty() {
            continue;
        }
        supporting.push(ni);
        supporting.extend(extra);
        for proposed in entries {
            added.push(LexicalBinding {
                proposed,
                provenance: Provenance {
                    surface: tok.to_string(),
                    long_form: None,
                    context: tok.to_string(),
                    method: ResolutionMethod::NotationRecognized,
                    grounded_to: None,
                    confidence: None,
                },
            });
        }
    }

    LexiconAugmentation {
        added,
        supporting,
        missing_oov: Vec::new(),
    }
}

/// What the recognised notation is, for the individual's description — the parse in words, so the
/// resource says what it is without the IRI having to carry it.
fn describe(n: &eigenius_notation::Notation) -> &'static str {
    use eigenius_notation::Notation as N;
    match n {
        N::ProteinChange(_) => "a protein change; its reference accession is not in the notation",
        N::CodingChange(_) => "a coding change; its reference accession is not in the notation",
        N::ResiduePosition { .. } => {
            "a residue and position; its reference accession is not in the notation"
        }
        N::CodingPosition { .. } => {
            "a coding position; its reference accession is not in the notation"
        }
        N::ResidueRange { .. } => {
            "an interval between two residue positions; its reference accession is not in the \
             notation"
        }
        N::UniProt(_) => "a UniProt accession",
        N::PdbEntry(_) => "a Protein Data Bank entry",
        N::DbSnp(_) => "a dbSNP reference SNP",
        N::ClinVar(_) => "a ClinVar variation record",
        N::SequenceRecord(_) => "an Ensembl or RefSeq sequence record",
        N::GenomicCoordinate(_) => "a genomic coordinate",
    }
}

// ── Stage F: recurrent OOV names — the SIXTH extraction source ───────────────────
//
// After notation, the units still blocked are blocked by PRODUCT NAMES: `gnomAD` 22 units, `ABE9`
// 11, `SIFT4G` 6, `AlphaFold` / `AlphaMissense` / `ClinPred` / `ABE8e` 5 each. None is a notation —
// there is no grammar to read — and none reaches apposition, which needs a common-noun head in
// front and a Title-case shape neither `gnomAD` nor `ABE9` has.
//
// The signal is orthography plus repetition, which apposition explicitly rejects as too weak:
// "in a 7.6M-entry lexicon almost every surface has *some* noun sense, so `is a common noun` does
// not discriminate". That objection does not reach HERE, because this source only ever sees
// surfaces the lexicon has NO sense for. The OOV gate is what makes the weak signal admissible,
// and it is why this is a separate source with its own provenance rather than a loosened
// apposition rule.

/// Does `t` look like a NAME the document keeps using — a capital somewhere, letters and digits
/// only, and nothing that belongs to another source?
fn is_oov_name(t: &str) -> bool {
    let core = t.trim_end_matches("'s").trim_end_matches("\u{2019}s");
    // A function word in capitals is still a function word. `AND` was minted as a named
    // individual: repetition and a capital are not enough, and the closed-class layer already
    // owns the answer (case-insensitive, whole-surface).
    if super::closed_class::is_closed_class_surface(core) {
        return false;
    }
    core.len() >= 2
        && core.starts_with(|c: char| c.is_ascii_alphabetic())
        && core.chars().any(|c| c.is_ascii_uppercase())
        && core
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
        // A notation is the notation source's to claim, not this one's.
        && eigenius_notation::recognize(core).is_none()
}

/// **Recurrent-OOV-name augmentation** — the sixth source. A surface the lexicon cannot supply,
/// carrying a capital and occurring at least `MIN_OCCURRENCES` times, becomes a doc-local named
/// individual with a `cat_np` alias. A possessive (`gnomAD's`) is emitted as a second alias of the
/// SAME individual, not as an entity of its own.
pub fn oov_name_augmentation(base: &Arc<Layer>, document: &str) -> LexiconAugmentation {
    /// Repetition is the whole precision guard, so one mention is not a name here.
    const MIN_OCCURRENCES: usize = 2;

    let Ok(entry_class) = Iri::parse("urn:eigenius:lexicon:LexicalEntry") else {
        return LexiconAugmentation::default();
    };
    let Some(entity) = Iri::parse("urn:eigenius:lexicon:Entity").ok() else {
        return LexiconAugmentation::default();
    };

    let toks: Vec<&str> = whitespace_tokens(document);
    let mut freq: std::collections::BTreeMap<&str, usize> = Default::default();
    for t in &toks {
        *freq.entry(*t).or_default() += 1;
    }

    let mut added = Vec::new();
    let mut supporting = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for t in &toks {
        let t = *t;
        if !is_oov_name(t) || freq.get(t).copied().unwrap_or(0) < MIN_OCCURRENCES {
            continue;
        }
        let core = t.trim_end_matches("'s").trim_end_matches("\u{2019}s");
        let key = slug(core);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        if !entries_for_form(base, core).is_empty() {
            continue;
        }
        let Ok(ni_iri) = Iri::parse(&format!("urn:eigenius:doc:ni_{key}")) else {
            continue;
        };
        let p = |s: &str| Iri::parse(s).expect("valid well-known iri");
        let mut ni = Resource::new(ni_iri.clone());
        ni.set(p(wk::IS_A), Value::Array(vec![Value::iri(&entity)]));
        ni.set(
            p(wk::DESCRIPTION),
            Value::String(format!(
                "Doc-local named individual (recurrent OOV name): {core:?}, used {} times and \
                 absent from the lexicon.",
                freq.get(t).copied().unwrap_or(0)
            )),
        );
        let mut b = LayerBuilder::new("oov-name-emit", Some(Arc::clone(base)));
        if b.add_resource(ni.clone()).is_err() {
            continue;
        }
        let tmp = Arc::new(b.build(LayerStorage::in_memory()));

        // The bare name, and the possessive when the document writes one — two aliases, ONE
        // individual: `gnomAD's` is a form of gnomAD, not a second database.
        let mut surfaces = vec![core.to_string()];
        for suffix in ["'s", "\u{2019}s"] {
            let poss = format!("{core}{suffix}");
            if document.contains(&poss) {
                surfaces.push(poss);
            }
        }
        let mut minted = false;
        for surface in surfaces {
            let binding = AbbreviationBinding {
                abbr: &surface,
                long_form: &surface,
                concept_iri: ni_iri.as_str(),
                doc_ns: "urn:eigenius:doc",
            };
            let Some(rs) = abbreviation_resources(&tmp, &binding) else {
                continue;
            };
            let (entries, extra): (Vec<Resource>, Vec<Resource>) =
                rs.into_iter().partition(|r| r.is_instance_of(&entry_class));
            if entries.is_empty() {
                continue;
            }
            minted = true;
            supporting.extend(extra);
            for proposed in entries {
                added.push(LexicalBinding {
                    proposed,
                    provenance: Provenance {
                        surface: surface.clone(),
                        long_form: None,
                        context: core.to_string(),
                        method: ResolutionMethod::RecurrentOovName,
                        grounded_to: None,
                        confidence: None,
                    },
                });
            }
        }
        if minted {
            supporting.push(ni);
        }
    }

    LexiconAugmentation {
        added,
        supporting,
        missing_oov: Vec::new(),
    }
}

// ── The gap stream (D62 §9) ──────────────────────────────────────────────────
//
// `enc:LexicalGap` has been declared since D62 and had NO WRITER: the class, its three
// dispositions and all four properties existed, and the only mention of it in the Rust was a
// bootstrap comment listing the class names. So a run's lexical closure left no record — the
// artifact could not say that `gnomAD` was a doc-local individual injected by the recurrent-name
// source rather than a lexicon concept, which is exactly the provenance a reviewer needs before
// trusting a claim built on it. D62 §9 calls the gap stream "a first-class coverage signal"; it
// was a signal nothing emitted.
//
// `ResolutionMethod` cannot ride along machine-readably: it lives on the PROPOSAL by design
// ("carried on the proposal, not on the committed entry"), and the encoding ontology has no
// property for it. It goes in `core:description` here, and a queryable property is the one
// ontology addition this would want — deliberately not taken, because it costs a reseed.

/// **The gap stream** — one `enc:LexicalGap` per surface an augmentation touched, with how it was
/// disposed: `gap_bound` where an existing concept was found, `gap_injected` where an entry was
/// minted, `gap_unresolvable` for what survived every source.
///
/// `enc:gap_unit` is left unset: an augmentation is built per DOCUMENT, so the unit a gap blocked
/// is not known at this seam. It is `recommends`, not `requires`, and inventing one would be worse
/// than omitting it.
///
/// NOT STANDALONE: `enc:bound_entry` points at the minted `lexicon:LexicalEntry`, so these records
/// must be committed alongside the augmentation's own entries and supporting individuals. On their
/// own they fail validation with `UnresolvedClassReference`, which is the reference working.
pub fn lexical_gap_resources(aug: &LexiconAugmentation, doc_ns: &str) -> Vec<Resource> {
    let p = |s: &str| Iri::parse(s).expect("valid well-known iri");
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    let mut emit = |surface: &str, disposition: &str, entry: Option<&Iri>, note: String| {
        let key = slug(surface);
        if key.is_empty() || !seen.insert(format!("{disposition}:{key}")) {
            return;
        }
        let Ok(iri) = Iri::parse(&format!("{doc_ns}:gap_{key}")) else {
            return;
        };
        let Ok(disp) = Iri::parse(disposition) else {
            return;
        };
        let mut r = Resource::new(iri);
        r.set(
            p(wk::IS_A),
            Value::Array(vec![Value::iri(&p("urn:eigenius:encoding:LexicalGap"))]),
        );
        r.set(
            p("urn:eigenius:encoding:gap_token"),
            Value::String(surface.to_string()),
        );
        r.set(
            p("urn:eigenius:encoding:gap_disposition"),
            Value::iri(&disp),
        );
        if let Some(e) = entry {
            r.set(p("urn:eigenius:encoding:bound_entry"), Value::iri(e));
        }
        r.set(p(wk::DESCRIPTION), Value::String(note));
        out.push(r);
    };

    for b in &aug.added {
        // Grounded to a concept the chain already had, or an entry minted for this document? That
        // is the bound/injected distinction, and it is the one a reader needs.
        let bound = b.provenance.grounded_to.is_some();
        let entry = b.proposed.id().cloned();
        emit(
            &b.provenance.surface,
            if bound {
                "urn:eigenius:encoding:gap_bound"
            } else {
                "urn:eigenius:encoding:gap_injected"
            },
            entry.as_ref(),
            format!(
                "Resolved by {:?}{}.",
                b.provenance.method,
                b.provenance
                    .grounded_to
                    .as_ref()
                    .map(|g| format!(", grounded to {g}"))
                    .unwrap_or_default()
            ),
        );
    }
    for g in &aug.missing_oov {
        emit(
            &g.surface,
            "urn:eigenius:encoding:gap_unresolvable",
            None,
            format!(
                "No target found by {} method(s): {:?}.",
                g.tried.len(),
                g.tried
            ),
        );
    }
    out
}
