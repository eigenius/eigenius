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

//! Why HGVS variant notation never reaches the named-individual route (survey §4 step 2).
//!
//! `Ile380Thr` is OOV in 20 of the XIAP report's units and blocks every one of them, yet the
//! recognizer that mints doc-local named individuals — the route that closed unit 4's gap in step
//! 4 — never offers it. The measurement said the recurrence signal should have fired (20 ≫ 2), so
//! this pins WHY it does not, with a permissive head predicate so nothing is attributed to the
//! lexicon.
//!
//! Three independent reasons, each sufficient on its own. Recorded as tests because the next
//! person to read "recurrence + non-adjective head" will expect `Ile380Thr` to qualify.

use eigenius_kernel::dcg::named_entity::extract_named_entities_with;

/// Every head admitted, so only orthography and apposition can reject anything.
fn permissive(_head: &str) -> bool {
    true
}

#[test]
fn variant_notation_fails_the_title_case_shape() {
    // `name_token` admits ALL-CAPS (`DRIVE`) or Title-case with NO interior capital — the rule
    // that deliberately rejects `mRNA` and `HeLa`. HGVS three-letter codes are exactly that shape:
    // `Ile380Thr` carries an interior `T`, so it is not a name token at all, and no amount of
    // recurrence can admit it.
    let text = "The residue Ile380Thr recurs. The residue Ile380Thr recurs again.";
    let got = extract_named_entities_with(text, permissive);
    assert!(
        !got.iter().any(|n| n.name.contains("Ile380Thr")),
        "interior capital rejects the HGVS shape, got {got:?}"
    );

    // The control: the same frame with a Title-case name IS recognized, so the test is not
    // vacuous — the apposition machinery works and the shape is what rejects the variant.
    let ok = extract_named_entities_with(
        "The project Achilles recurs. The project Achilles recurs again.",
        permissive,
    );
    assert!(
        ok.iter().any(|n| n.name == "Achilles"),
        "the control must be recognized or this test proves nothing, got {ok:?}"
    );
}

#[test]
fn the_hgvs_prefix_fails_the_first_character_test() {
    // `p.Ile380Thr` — the form the report uses in tables and quotations — begins lowercase, so it
    // is rejected before the shape test is even reached.
    let text = "The variant p.Ile380Thr recurs. The variant p.Ile380Thr recurs again.";
    let got = extract_named_entities_with(text, permissive);
    assert!(
        !got.iter().any(|n| n.name.contains("Ile380Thr")),
        "a lowercase-initial name is not a name token, got {got:?}"
    );
}

#[test]
fn the_documents_real_contexts_offer_no_noun_head() {
    // Even granting the shape, apposition needs a common-noun HEAD immediately before the name.
    // These are the actual contexts, by frequency, from the 450-unit corpus: a determiner, a
    // preposition, a comparative, or a VERB. None is a noun head, so the route could not fire here
    // even if HGVS were Title-case.
    for frame in [
        "an Ile380Thr edit",
        "for Ile380Thr and",
        "of Ile380Thr in",
        "than Ile380Thr .",
        "prefers Ile380Thr over",
        "carrying Ile380Thr is",
    ] {
        let text = format!("{frame}. {frame}.");
        let got = extract_named_entities_with(&text, permissive);
        assert!(
            !got.iter().any(|n| n.name.contains("Ile380Thr")),
            "«{frame}» offers no noun head, got {got:?}"
        );
    }
}
