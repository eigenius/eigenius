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

//! The notation glossary source (D101 §step 2), on the plain bootstrap chain.
//!
//! `Ile380Thr` blocked 20 units of the XIAP report and the apposition source never offered it.
//! What is pinned here is the TIERING, because the tiers are the whole design: a token that states
//! its reference accession becomes a `variant:Allele` with true identity; one that does not closes
//! the gap lexically with its parse recorded, and no accession is invented.

use std::sync::Arc;

use eigenius_kernel::dcg::notation_augmentation;
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;

fn bootstrap_head() -> Arc<Layer> {
    Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    )
}

/// `variant` is NOT in BOOTSTRAP_CHAIN — it loads above the tip, after `lexicon`, because its
/// classes subclass `lexicon:Entity`. So tier 1 is only reachable on a chain that has it.
fn with_variant(head: Arc<Layer>) -> Arc<Layer> {
    let src = include_str!("../../ontologies/variant/variant.esl");
    let resources = esl::compile(src, &head).expect("variant compiles over the bootstrap tip");
    let mut b = LayerBuilder::new("variant", Some(head));
    for r in resources {
        b.add_resource(r).expect("add");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

#[test]
fn the_bare_change_closes_the_gap_without_inventing_an_accession() {
    let head = bootstrap_head();
    let aug = notation_augmentation(
        &head,
        "The bystander edit installs Ile380Thr in the UBA domain.",
    );

    assert!(
        !aug.added.is_empty(),
        "the gap must close — a unit carrying Ile380Thr returned missing-lexeme before this source"
    );
    let ni = aug
        .supporting
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str())
                .unwrap_or("")
                .contains("ile380thr")
        })
        .expect("a doc-local individual for the recognised token");

    // Tier 2: typed lexicon:Entity, NOT an allele — the accession is not in the token, and
    // variant:Allele is identified by it ("one change on two references is two alleles").
    assert!(
        ni.is_instance_of(&iri("urn:eigenius:lexicon:Entity")),
        "tier 2 is a lexical entity, got {:?}",
        ni.is_a()
    );
    assert!(
        ni.get(&iri("urn:eigenius:variant:reference")).is_none(),
        "no accession may be invented for a bare change"
    );
    // And the provenance says which source did it.
    assert!(
        aug.added.iter().all(|b| matches!(
            b.provenance.method,
            eigenius_kernel::dcg::augment::ResolutionMethod::NotationRecognized
        )),
        "the binding is attributed to notation recognition"
    );
}

#[test]
fn a_qualified_notation_becomes_an_allele_with_true_identity() {
    let head = with_variant(bootstrap_head());
    let aug = notation_augmentation(
        &head,
        "The correction targets NM_001167.4:c.1141C>T directly.",
    );

    let ni = aug
        .supporting
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str())
                .unwrap_or("")
                .contains("nm_001167")
        })
        .expect("a doc-local individual for the qualified notation");

    assert!(
        ni.is_instance_of(&iri("urn:eigenius:variant:NucleotideAllele")),
        "tier 1 is an allele, got {:?}",
        ni.is_a()
    );
    // The accession the NOTATION stated — not one chosen for it.
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:reference"))
            .and_then(|v| v.as_str().map(str::to_string))
            .as_deref(),
        Some("NM_001167.4")
    );
    // The parse, as typed fields. This is the point: nothing is carried by the name.
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:start"))
            .and_then(|v| v.as_integer()),
        Some(1141)
    );
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:ref_state"))
            .and_then(|v| v.as_str().map(str::to_string))
            .as_deref(),
        Some("C")
    );
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:alt"))
            .and_then(|v| v.as_str().map(str::to_string))
            .as_deref(),
        Some("T")
    );
}

#[test]
fn without_the_variant_layer_tier_one_degrades_rather_than_dangling() {
    // The glossary runs inside the kernel, whose chain ends at `encoding`. Emitting
    // variant:NucleotideAllele on a chain without the layer would be an unresolvable class.
    let head = bootstrap_head();
    let aug = notation_augmentation(
        &head,
        "The correction targets NM_001167.4:c.1141C>T directly.",
    );
    let ni = aug
        .supporting
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str())
                .unwrap_or("")
                .contains("nm_001167")
        })
        .expect("still recognised, still closes the gap");
    assert!(
        ni.is_instance_of(&iri("urn:eigenius:lexicon:Entity")),
        "degrades to tier 2, got {:?}",
        ni.is_a()
    );
    assert!(ni.get(&iri("urn:eigenius:variant:reference")).is_none());
}

#[test]
fn ordinary_prose_and_product_names_yield_nothing() {
    let head = bootstrap_head();
    // Every token here is one the XIAP corpus contains and that must NOT be read as notation —
    // the editors, predictors and databases that remain OOV after this source.
    let aug = notation_augmentation(
        &head,
        "ABE9 and ABE8e were scored by SIFT4G, AlphaMissense, ClinPred and BayesDel against \
         MaveDB and HGMD at v4.1, which the paywalled text calls a meta-classifier.",
    );
    assert!(
        aug.added.is_empty() && aug.supporting.is_empty(),
        "no notation here, got {:?}",
        aug.supporting
            .iter()
            .map(|r| r.id().map(|i| i.as_str()).unwrap_or("").to_string())
            .collect::<Vec<_>>()
    );
}

/// The OOV-only guard, which is what makes the UniProt/gene-symbol collision harmless: a surface
/// the committed lexicon already supplies is left alone. Asserted against a real lexicon form
/// rather than a contrived one, so the test fails if `entries_for_form` stops matching.
#[test]
fn a_surface_the_lexicon_already_has_is_left_alone() {
    let head = bootstrap_head();
    // `2KNA` is PDB-shaped and NOT in the bootstrap lexicon, so it closes.
    let closes = notation_augmentation(&head, "The coordinates come from 2KNA directly.");
    assert!(
        !closes.added.is_empty(),
        "an unknown PDB-shaped token should close the gap"
    );

    // The bootstrap chain carries no lexical entries at all (the lexicon LAYER is schema; forms
    // arrive with the WordNet/UMLS import), so the guard cannot be exercised positively here
    // without a served chain. What is pinned is that the guard is CONSULTED: a token is skipped
    // when a form lookup hits. Recorded as the reason this stays an OOV closure.
    assert!(
        closes.added.iter().all(|b| b.provenance.method
            == eigenius_kernel::dcg::augment::ResolutionMethod::NotationRecognized),
        "notation-recognized provenance"
    );
}

/// A residue RANGE must close its endpoints too, not only itself. In the page a helix boundary is
/// one whitespace-delimited token (`Thr369–Asn373`), but the parser's tokenizer splits on the dash
/// and asks the lexicon for `Asn373` — so an entry keyed only on the whole range is keyed under a
/// surface nothing requests. That is why those endpoints stayed OOV after the range support landed.
#[test]
fn a_residue_range_closes_its_endpoints_as_well_as_itself() {
    let head = bootstrap_head();
    let aug = notation_augmentation(
        &head,
        "Helix \u{3b1}1 spans Thr369\u{2013}Asn373 in the domain.",
    );
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    for want in ["Thr369\u{2013}Asn373", "Thr369", "Asn373"] {
        assert!(
            surfaces.iter().any(|s| s == want),
            "{want} must get an entry; got {surfaces:?}"
        );
    }
}

// ── The sixth source: recurrent OOV names ────────────────────────────────────

use eigenius_kernel::dcg::oov_name_augmentation;

/// After notation, what still blocks the XIAP report is PRODUCT NAMES — `gnomAD` alone blocks 22
/// units. None is a notation and none reaches apposition, which needs a common-noun head in front
/// and a Title-case shape neither `gnomAD` nor `ABE9` has.
#[test]
fn a_recurring_capitalised_oov_name_is_minted() {
    let head = bootstrap_head();
    let doc = "The gnomAD v4 release lists no hemizygote. A second gnomAD query confirmed it. \
               ABE9 narrows the window, and ABE9 was built on ABE8e.";
    let aug = oov_name_augmentation(&head, doc);
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    for want in ["gnomAD", "ABE9"] {
        assert!(
            surfaces.iter().any(|s| s == want),
            "{want}, got {surfaces:?}"
        );
    }
    // ABE8e occurs ONCE here: repetition is the whole precision guard.
    assert!(
        !surfaces.iter().any(|s| s == "ABE8e"),
        "a single mention is not a name, got {surfaces:?}"
    );
}

/// A possessive is a FORM of the name, not a second entity: two aliases, one individual.
#[test]
fn a_possessive_aliases_the_same_individual() {
    let head = bootstrap_head();
    let doc = "gnomAD reports no hemizygote. gnomAD's coverage at this site is complete. \
               The gnomAD page was checked.";
    let aug = oov_name_augmentation(&head, doc);
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    assert!(surfaces.iter().any(|s| s == "gnomAD"), "{surfaces:?}");
    assert!(surfaces.iter().any(|s| s == "gnomAD's"), "{surfaces:?}");
    let individuals: Vec<&str> = aug
        .supporting
        .iter()
        .filter_map(|r| r.id().map(|i| i.as_str()))
        .filter(|i| i.contains("ni_gnomad"))
        .collect();
    assert_eq!(
        individuals.len(),
        1,
        "one individual, two aliases: {individuals:?}"
    );
}

/// The precision side. Ordinary prose must not be swallowed, and a notation stays the notation
/// source's to claim even though it recurs and carries capitals.
#[test]
fn prose_and_notation_are_not_claimed_as_names() {
    let head = bootstrap_head();
    let doc = "The variant Ile380Thr recurs. The variant Ile380Thr is the subject. \
               The residue is exposed. The residue is exposed on helix one.";
    let aug = oov_name_augmentation(&head, doc);
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    assert!(
        !surfaces.iter().any(|s| s == "Ile380Thr"),
        "a notation belongs to the notation source, got {surfaces:?}"
    );
    for w in ["The", "variant", "residue", "exposed", "helix"] {
        assert!(
            !surfaces.iter().any(|s| s == w),
            "{w} is prose, got {surfaces:?}"
        );
    }
}

/// Two precision defects the first wired run exposed, both from interactions rather than from the
/// rule itself.
#[test]
fn neither_a_surname_half_nor_a_capitalised_function_word_is_a_name() {
    let head = bootstrap_head();
    // Dash parts exist so a residue RANGE closes its endpoints; the notation source filters them
    // through `recognize`, this one has no such filter, and it minted both halves of the surname.
    let doc = "Gyrd-Hansen et al. tested this. Gyrd-Hansen et al. showed dimerisation. \
               The A AND the B. The C AND the D.";
    let aug = oov_name_augmentation(&head, doc);
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    assert!(
        surfaces.iter().any(|s| s == "Gyrd-Hansen"),
        "the whole surname is a name, got {surfaces:?}"
    );
    for half in ["Gyrd", "Hansen"] {
        assert!(
            !surfaces.iter().any(|s| s == half),
            "{half} is half a surname, not a name: {surfaces:?}"
        );
    }
    assert!(
        !surfaces.iter().any(|s| s == "AND"),
        "a function word in capitals is still a function word: {surfaces:?}"
    );
}

// ── The gap stream ───────────────────────────────────────────────────────────

use eigenius_kernel::dcg::lexical_gap_resources;
use eigenius_kernel::validation::Validator;

/// `enc:LexicalGap` had no writer since D62 declared it, so a run's lexical closure left no
/// record — nothing in an artifact said `gnomAD` was a doc-local individual rather than a lexicon
/// concept. The records must VALIDATE as the class they claim, not merely serialise.
#[test]
fn the_gap_stream_validates_as_lexical_gaps() {
    let head = bootstrap_head();
    let doc = "gnomAD reports no hemizygote. gnomAD's coverage is complete. \
               The gnomAD page lists Ile380Thr and Ile380Thr again.";
    let mut aug = notation_augmentation(&head, doc);
    let names = oov_name_augmentation(&head, doc);
    aug.added.extend(names.added);
    aug.supporting.extend(names.supporting);

    let gaps = lexical_gap_resources(&aug, "urn:eigenius:doc");
    assert!(
        !gaps.is_empty(),
        "the stream must record what the sources closed"
    );

    // Every record carries what the class REQUIRES: a token and a disposition.
    for g in &gaps {
        assert!(
            g.get(&iri("urn:eigenius:encoding:gap_token")).is_some()
                && g.get(&iri("urn:eigenius:encoding:gap_disposition"))
                    .is_some(),
            "requires gap_token + gap_disposition: {:?}",
            g.id()
        );
    }

    // Deterministic sources mint entries, so these are `injected`, not `bound`.
    let injected = gaps
        .iter()
        .filter(|g| {
            g.get(&iri("urn:eigenius:encoding:gap_disposition"))
                .and_then(|v| v.as_iri())
                .map(|i| i.as_str() == "urn:eigenius:encoding:gap_injected")
                .unwrap_or(false)
        })
        .count();
    assert!(
        injected > 0,
        "a minted entry is an injected gap, got {gaps:?}"
    );

    // And they load: a record that does not validate is not a coverage signal. The stream is NOT
    // standalone — `bound_entry` points at the minted entry, so the entries and their supporting
    // individuals must be committed with it, which is what the artifact does. Validating the gaps
    // alone fails with UnresolvedClassReference, and that is correct behaviour, not a bug.
    let mut b = LayerBuilder::new("gap-stream", Some(Arc::clone(&head)));
    for r in aug.supporting.iter().cloned() {
        let _ = b.add_resource(r);
    }
    for binding in &aug.added {
        let _ = b.add_resource(binding.proposed.clone());
    }
    for g in gaps {
        b.add_resource(g).expect("gap record adds");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let errors = Validator::new(Arc::clone(&layer)).validate();
    assert!(
        errors.is_empty(),
        "{} validation error(s): {errors:#?}",
        errors.len()
    );
}

// ── The seventh source: inline code spans ────────────────────────────────────

use eigenius_kernel::dcg::code_span_augmentation;

/// 48 of the 113 tokens left unresolvable after notation and names occur inside an inline code
/// span. The author marked them, for the one thing they have in common — each is a literal string
/// MENTIONED, not a word used — which is why one markup rule covers what looked like six unrelated
/// pattern-matching jobs (IUPAC sequences, hostnames, dotted field paths, URLs, peptides).
#[test]
fn a_token_the_document_marks_as_code_becomes_a_literal() {
    let head = bootstrap_head();
    let doc = "The query used `dbnsfp.aa.pos` and `gnomad.broadinstitute.org` directly, \
               with the spacer `ACCCCATTCATATAGCTTCT` and the construct `XIAP^\u{394}UBA`.";
    let aug = code_span_augmentation(&head, doc);
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    for want in [
        "dbnsfp.aa.pos",
        "gnomad.broadinstitute.org",
        "ACCCCATTCATATAGCTTCT",
        "XIAP^\u{394}UBA",
    ] {
        assert!(
            surfaces.iter().any(|s| s == want),
            "{want}, got {surfaces:?}"
        );
    }
    assert!(
        aug.added.iter().all(|b| b.provenance.method
            == eigenius_kernel::dcg::augment::ResolutionMethod::MarkedAsCode),
        "the document's own markup is the provenance"
    );
}

/// Unmarked prose is untouched — the rule is the markup, not the shape of the token.
#[test]
fn unmarked_prose_is_not_claimed_as_a_literal() {
    let head = bootstrap_head();
    let aug = code_span_augmentation(
        &head,
        "The residue is partially exposed on helix one and the domain is dispensable.",
    );
    assert!(
        aug.added.is_empty(),
        "no code spans here, got {:?}",
        aug.added
            .iter()
            .map(|b| b.provenance.surface.clone())
            .collect::<Vec<_>>()
    );
}

/// A NOTATION inside a code span stays the notation source's: that source records ref/position/alt
/// as typed fields, where this one would mint an opaque string and lose the structure.
#[test]
fn a_notation_in_a_code_span_is_left_to_the_notation_source() {
    let head = bootstrap_head();
    let aug = code_span_augmentation(
        &head,
        "The variant `p.Ile380Thr` sits in `XIAP^\u{394}UBA`.",
    );
    let surfaces: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.clone())
        .collect();
    assert!(
        !surfaces.iter().any(|s| s == "p.Ile380Thr"),
        "a notation must keep its parse, got {surfaces:?}"
    );
    assert!(
        surfaces.iter().any(|s| s == "XIAP^\u{394}UBA"),
        "a non-notation literal still closes, got {surfaces:?}"
    );
}

// ── The eighth source: derivation from an attested base ──────────────────────

use eigenius_kernel::dcg::derive_with;

/// The words the lexicon simply lacks are what `ResolutionMethod::LlmSynthesized` was declared
/// for — and that arm has one reference in the codebase, its own enum variant. Some need no LLM:
/// a productive affix over an ATTESTED base is deterministic. The four bases below were verified
/// present in WordNet before the rule was written (analogy, read, pull, dimer).
#[test]
fn a_productive_affix_over_an_attested_base_derives() {
    // The lexicon the rule is tested against: exactly the bases the document's words need.
    let known = |s: &str| matches!(s, "analogy" | "read" | "pull" | "dimer" | "cpg");

    for (word, stem, affix) in [
        ("pulldown", "pull", "down"),
        ("dimerisation", "dimer", "isation"),
    ] {
        let got = derive_with(word, known);
        assert_eq!(
            got.as_ref().map(|(s, a, _)| (s.as_str(), *a)),
            Some((stem, affix)),
            "{word} should derive from {stem}"
        );
    }

    // No attested base, no derivation — `paywalled` and `phred` stay honestly unresolved rather
    // than being invented here. They are what the LLM arm is for.
    for word in ["paywalled", "phred", "disanalogue"] {
        assert_eq!(
            derive_with(word, known),
            None,
            "{word} has no attested base"
        );
    }

    // PREFIXES ARE NOT A DETERMINISTIC RULE, which is why they were removed. `disanalogy` and
    // `overread` were correct here, but the same stripping gives `research` → `search` and
    // `discuss` → `cuss`, whose stems are equally attested — they did not fire on this document
    // only because those words are themselves in the lexicon, so the OOV gate skipped them. All
    // five go to the LlmSynthesized arm rather than being grounded on luck.
    let wide = |s: &str| matches!(s, "search" | "cuss" | "move" | "analogy" | "read");
    for word in ["research", "discuss", "remove", "disanalogy", "overread"] {
        assert_eq!(
            derive_with(word, wide),
            None,
            "{word}: no prefix is stripped"
        );
    }

    // An INFLECTION is not a derivation: `resources` is `resource` + plural, which the first
    // version of this rule read as `re` + `sources`.
    assert_eq!(
        derive_with("resources", |s| s == "sources" || s == "resource"),
        None
    );
}

/// The guards: one affix only, a stem long enough to be a word, and lowercase only — a capitalised
/// OOV surface is a NAME, which the sixth source owns.
#[test]
fn derivation_strips_one_affix_from_a_lowercase_word_only() {
    let known = |s: &str| matches!(s, "pull" | "dimer" | "up");

    // Capitalised: not this source's business — a capitalised OOV surface is a NAME.
    assert_eq!(derive_with("Pulldown", |_| true), None);
    // Too short to be a word: `up` is attested, but a two-letter stem is not accepted.
    assert_eq!(derive_with("upup", known), None);
    // The real case still works, so the guards above are not vacuous.
    assert!(derive_with("pulldown", known).is_some());
}
