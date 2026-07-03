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
//! turns each binding into the two resources a chained, document-scoped lexicon layer needs:
//!
//! 1. a **named individual** — an *instance* of the grounded concept class (so a bare `MSI` can denote
//!    a referring entity, not just the class); and
//! 2. a **`cat_np` `lexicon:LexicalEntry`** — bare `MSI` is a proper-noun name of that individual.
//!
//! It is the fix for the #1 CNL-v2 parsing gap: a bare domain abbreviation imported as a `cat_n` common
//! noun cannot be an argument NP (see `d63-cnl-v2-parsing-diagnosis.md`); the injected `cat_np` entry
//! recovers it, with no parser/grammar change (the "add, not shadow" form).
//!
//! Unlike the WordNet/UMLS importers (which render ESL *text* that is compiled at load), these are
//! built **directly as in-memory [`Resource`]s** — the load path takes CBOR/Eigon-JSON resources, so
//! there is no reason to round-trip through ESL. The category term is encoded with
//! [`encode_type`](crate::program::eigentt_type_mirror::encode_type), the same D47 encoding ESL emits.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::category::resolve_inductive;
use crate::layer::{normalize_value, resolve_active_value_indexes, Layer};
use crate::nbe::term::Exp;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::ontology::Iri;
use crate::program::eigentt_type_mirror::encode_type;

// ── Stage A: abbreviation-definition extraction (Schwartz & Hearst 2003) ─────────
//
// Deterministic, high-precision extraction of `Long Form (SHORT)` definitions — the parenthetical
// pattern our biomedical corpus introduces its abbreviations with (`microsatellite instability
// (MSI)`). This is the deterministic-first half of Stage A; the LLM fallback (for non-parenthetical
// definitions) is a later orchestration step. Reference: A. S. Schwartz & M. A. Hearst, "A Simple
// Algorithm for Identifying Abbreviation Definitions in Biomedical Text," Pacific Symposium on
// Biocomputing 2003 (verify the exact identifier before citing as a load-bearing `.bib` anchor).

/// One extracted abbreviation definition: the surface short form, the **minimal** long form that
/// defines it (Schwartz-Hearst), and the full candidate `context` window before the paren. The
/// context lets grounding retry a **fuller** long form when the minimal one doesn't match a lexicon
/// surface string (e.g. `MMR`'s minimal `mismatch repair` vs the lexicon's `DNA mismatch repair`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbbrDef {
    pub short_form: String,
    pub long_form: String,
    pub context: String,
}

/// A candidate short form (the parenthetical) is admissible per Schwartz-Hearst: 2–10 chars, at most
/// two tokens, first char alphanumeric, and at least one letter (so `(1c)` / `(a, b)` are rejected).
fn is_valid_short_form(s: &str) -> bool {
    let n = s.chars().count();
    if !(2..=10).contains(&n) {
        return false;
    }
    if s.split_whitespace().count() > 2 {
        return false;
    }
    let first_alnum = s.chars().next().is_some_and(|c| c.is_alphanumeric());
    first_alnum && s.chars().any(|c| c.is_alphabetic())
}

/// The core Schwartz-Hearst test: find the shortest suffix of `long` whose characters contain those of
/// `short` as an **ordered subsequence**, scanned right-to-left, with the short form's FIRST char
/// constrained to a word start in `long`. `None` if no such match (⇒ not a definition).
fn find_best_long_form(short: &str, long: &str) -> Option<String> {
    let s: Vec<char> = short.chars().collect();
    let l: Vec<char> = long.chars().collect();
    if s.is_empty() || l.is_empty() {
        return None;
    }
    let mut s_index = s.len() as isize - 1;
    let mut l_index = l.len() as isize - 1;
    while s_index >= 0 {
        let curr = s[s_index as usize].to_ascii_lowercase();
        if !curr.is_alphanumeric() {
            s_index -= 1;
            continue;
        }
        // Move left until `long[l_index]` matches `curr` AND — for the first short char — its left
        // neighbour is a word boundary (so the abbreviation's first letter starts a word).
        while (l_index >= 0 && l[l_index as usize].to_ascii_lowercase() != curr)
            || (s_index == 0 && l_index > 0 && l[(l_index - 1) as usize].is_alphanumeric())
        {
            l_index -= 1;
        }
        if l_index < 0 {
            return None;
        }
        l_index -= 1;
        s_index -= 1;
    }
    // The long form begins at the word-initial char the first short char matched (`l_index + 1`).
    let start = (l_index + 1).max(0) as usize;
    let long_form: String = l[start..].iter().collect();
    let long_form = long_form.trim().to_string();
    (!long_form.is_empty()).then_some(long_form)
}

/// A found long form is admissible if it is non-empty and no longer than `min(|SF|+5, |SF|·2)` words
/// (the Schwartz-Hearst length bound: a definition can't be arbitrarily longer than the abbreviation).
fn is_valid_long_form(short: &str, long: &str) -> bool {
    let sf_len = short.chars().count();
    let max_words = (sf_len + 5).min(sf_len * 2);
    let wc = long.split_whitespace().count();
    (1..=max_words).contains(&wc)
}

/// Extract `Long Form (SHORT)` abbreviation definitions from raw document text (Schwartz-Hearst).
/// Runs on the **raw** text — upstream of `strip_bracketed_asides`, which would drop the `(SHORT)`
/// parenthetical — so the binding is captured even though the body sentence later loses the paren.
/// First-seen wins per short form (deduped, case-insensitively).
pub fn extract_abbreviations(text: &str) -> Vec<AbbrDef> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '(' {
            i += 1;
            continue;
        }
        let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == ')') else {
            break;
        };
        let inner: String = chars[i + 1..close].iter().collect();
        let short = inner.trim();
        if is_valid_short_form(short) {
            // Candidate long form: the `min(|SF|+5, |SF|·2)` words immediately preceding the `(`.
            let pre: String = chars[..i].iter().collect();
            let sf_len = short.chars().count();
            let max_words = (sf_len + 5).min(sf_len * 2);
            let words: Vec<&str> = pre.split_whitespace().collect();
            let take = words.len().saturating_sub(max_words);
            let candidate = words[take..].join(" ");
            if let Some(long) = find_best_long_form(short, &candidate) {
                if is_valid_long_form(short, &long) && seen.insert(short.to_lowercase()) {
                    out.push(AbbrDef {
                        short_form: short.to_string(),
                        long_form: long,
                        context: candidate,
                    });
                }
            }
        }
        i = close + 1;
    }
    out
}

// ── Stage A: grounding (long form → an existing concept, retrieve-first) ─────────

/// Every concept a surface `form` denotes in the chain — the `sem` of each matching
/// `lexicon:LexicalEntry` (deduped, in index order; a `cat_n` common-noun entry's `sem` IS the
/// concept class). Index-driven (a value-index probe) on the served chain; an eager resource scan
/// only for small in-memory layers with no active index (mirrors `LexicalIndex::build`'s fallback, so
/// it never scans the 7.6M-resource served lexicon).
fn concepts_for_form(layer: &Arc<Layer>, form: &str) -> Vec<Iri> {
    let (Ok(form_prop), Ok(sem_prop), Ok(entry_class)) = (
        Iri::parse("urn:eigenius:lexicon:form"),
        Iri::parse("urn:eigenius:lexicon:sem"),
        Iri::parse("urn:eigenius:lexicon:LexicalEntry"),
    ) else {
        return Vec::new();
    };
    let read_sem = |r: &Resource| match r.get(&sem_prop) {
        Some(Value::ResourceRef(iri)) => Some(iri.clone()),
        Some(Value::String(s)) => Iri::parse(s).ok(),
        _ => None,
    };
    let mut out: Vec<Iri> = Vec::new();
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
            if let Some(iri) = read_sem(&r) {
                if seen.insert(iri.clone()) {
                    out.push(iri);
                }
            }
        }
    } else {
        let key = form.trim().to_lowercase();
        for (_id, r) in layer.iter_all_resources() {
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            if let Some(Value::String(f)) = r.get(&form_prop) {
                if f.trim().to_lowercase() == key {
                    if let Some(iri) = read_sem(r.as_ref()) {
                        if seen.insert(iri.clone()) {
                            out.push(iri);
                        }
                    }
                }
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

/// One abbreviation binding from Stage-A extraction: the surface `abbr`, the `concept_iri` it is
/// grounded to (a class already resolvable in the chain — a UMLS CUI, or a fresh document-local class
/// on a grounding miss), and the `doc_ns` IRI stem the emitted resources are minted under (e.g.
/// `"urn:eigenius:doc:<docid>"`, per-document so distinct documents don't collide).
pub struct AbbreviationBinding<'a> {
    pub abbr: &'a str,
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

/// Build the document-scoped resources for one abbreviation binding — a named individual + its
/// `cat_np(concept, sg)` lexical entry. Returns the pair to `add_resource` into a chained doc layer,
/// or `None` if the `lexicon:Cat`/`Num` decls or the concept IRI don't resolve against `layer`.
///
/// Mirrors the UMLS importer's named-individual shape (`crates/eigenius-umls/src/convert.rs`), but as
/// direct structures: `sem` is the individual, `sem_type` is the concept, `cat` is the encoded
/// `cat_np`. The resources still pass the felicity gate at commit (fail-closed on a bad grounding).
pub fn abbreviation_resources(
    layer: &Arc<Layer>,
    binding: &AbbreviationBinding,
) -> Option<Vec<Resource>> {
    let cat_decl = resolve_inductive(layer, "urn:eigenius:lexicon:Cat")?;
    let num_decl = resolve_inductive(layer, "urn:eigenius:lexicon:Num")?;
    let concept = Iri::parse(binding.concept_iri).ok()?;

    // The category `cat_np(<concept>, sg)` and its denotation `<concept>` (the sem_type), encoded to
    // resource `Value`s exactly as ESL's `type_expr( … )` would (D47).
    let concept_ty = Exp::EigonClass(concept.clone());
    let sg = Exp::InductiveCtor(num_decl, "sg".into(), Vec::new());
    let cat_np = Exp::InductiveCtor(cat_decl, "cat_np".into(), vec![concept_ty.clone(), sg]);
    let cat_val = encode_type(&cat_np).ok()?;
    let sem_type_val = encode_type(&concept_ty).ok()?;

    let key = slug(binding.abbr);
    let ni_iri = Iri::parse(&format!("{}:ni_{key}", binding.doc_ns)).ok()?;
    let e_iri = Iri::parse(&format!("{}:e_{key}", binding.doc_ns)).ok()?;
    let p = |s: &str| Iri::parse(s).expect("valid well-known iri");
    let is_a = p(wk::IS_A);

    // (1) named individual: `<doc:ni_msi> : <concept>`.
    let mut ni = Resource::new(ni_iri.clone());
    ni.set(
        is_a.clone(),
        Value::Array(vec![Value::ResourceRef(concept)]),
    );
    ni.set(
        p(wk::DESCRIPTION),
        Value::String(format!(
            "Document-local named individual for the abbreviation {} (grounded to {}). Injected by \
             the abbreviation-definition preprocessor (D63 Phase 1).",
            binding.abbr, binding.concept_iri
        )),
    );

    // (2) lexical entry: bare `<abbr>` is a `cat_np` name of that individual.
    let mut e = Resource::new(e_iri);
    e.set(
        is_a,
        Value::Array(vec![Value::ResourceRef(p(
            "urn:eigenius:lexicon:LexicalEntry",
        ))]),
    );
    e.set(
        p("urn:eigenius:lexicon:form"),
        Value::String(binding.abbr.to_string()),
    );
    e.set(p("urn:eigenius:lexicon:cat"), cat_val);
    e.set(p("urn:eigenius:lexicon:sem"), Value::ResourceRef(ni_iri));
    e.set(p("urn:eigenius:lexicon:sem_type"), sem_type_val);
    e.set(
        p("urn:eigenius:lexicon:sense"),
        Value::String(format!("doc:{key}")),
    );
    e.set(
        p("urn:eigenius:lexicon:grade"),
        Value::ResourceRef(p("urn:eigenius:reflection:epistemic:declared")),
    );

    Some(vec![ni, e])
}

/// The full document glossary for a set of extracted definitions: for each, **ground** the
/// abbreviation ([`ground_abbreviation`]) and **emit** its named individual + `cat_np` entry; on a
/// grounding **miss**, mint a fresh document-local class `doc:class_<abbr> : lexicon:Entity` and bind
/// to it (§7-3) — so the abbreviation still parses (ungrounded but Entity-typed) rather than being
/// dropped. Returns every resource to commit into the document's chained glossary layer.
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
                    cls.set(
                        p(wk::IS_A),
                        Value::Array(vec![Value::ResourceRef(p(wk::CLASS))]),
                    );
                    cls.set(
                        p(wk::PARENT_CLASSES),
                        Value::Array(vec![Value::ResourceRef(p("urn:eigenius:lexicon:Entity"))]),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn defs(text: &str) -> Vec<(String, String)> {
        extract_abbreviations(text)
            .into_iter()
            .map(|d| (d.short_form, d.long_form))
            .collect()
    }

    #[test]
    fn extracts_the_wrn_definitions() {
        // The real WRN first-page introductions (the ones the CNL-v2 rewrite dropped, §4b).
        assert_eq!(
            defs("cancers with microsatellite instability (MSI), which results from deficient DNA mismatch repair"),
            vec![("MSI".to_string(), "microsatellite instability".to_string())],
        );
        // `MMR` = **M**is**M**atch **R**epair — both M's come from `mismatch` (positions 0 and 3), so
        // Schwartz-Hearst returns the MINIMAL long form `mismatch repair`, correctly dropping the
        // unnecessary `DNA` modifier (the abbreviation doesn't need it).
        assert_eq!(
            defs("defects in DNA mismatch repair (MMR) promote a hypermutable state"),
            vec![("MMR".to_string(), "mismatch repair".to_string())],
        );
        // `MSI` matches a non-word-initial S and I (both from "instability") — the subsequence match,
        // not first-letters, is what makes this work.
        assert_eq!(
            find_best_long_form("MSI", "microsatellite instability").as_deref(),
            Some("microsatellite instability"),
        );
    }

    #[test]
    fn rejects_non_definitions() {
        // A figure/table reference is not an abbreviation definition: no matching long form.
        assert!(defs("we analysed the data (Fig. 1c)").is_empty());
        // A parenthetical aside whose chars don't subsequence-match the preceding text.
        assert!(defs("the result was clear (see below)").is_empty());
        // An over-long "short form" (>2 tokens) is not an abbreviation candidate.
        assert!(defs("the process (a slow and careful one) matters").is_empty());
    }

    #[test]
    fn short_form_validity() {
        assert!(is_valid_short_form("MSI"));
        assert!(is_valid_short_form("PARP-1"));
        assert!(!is_valid_short_form("a")); // too short
        assert!(!is_valid_short_form("123")); // no letter
        assert!(!is_valid_short_form("one two three")); // >2 tokens
    }

    #[test]
    fn dedups_repeated_definitions_first_seen_wins() {
        let text = "microsatellite instability (MSI) is common; later, MSI (microsatellite instability) recurs";
        assert_eq!(
            defs(text),
            vec![("MSI".to_string(), "microsatellite instability".to_string())],
        );
    }
}
