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

//! The invariant `dcg::closed_class` states about itself:
//!
//! > Dropping them cannot make a word unknown (the bootstrap covers it).
//!
//! Both importers withhold every content entry on these surfaces, so a surface listed here and NOT
//! covered by the bootstrap is a word the whole pipeline can no longer see — a silent corpus-wide
//! gap, invisible until a document uses it. The list and `ontologies/lexicon/closed-class.esl` are
//! two halves of one claim and had drifted in both directions; this pins the direction that costs
//! coverage.
//!
//! "Covered" is deliberately `has_token`, not "has a `lexicon:form` entry": `and`/`or` are consumed
//! by the parser's coordination rule and `any`/`then` are reserved by the grammar, so none of them
//! has an entry and all of them are known.

use std::sync::Arc;

use eigenius_kernel::dcg::closed_class::is_closed_class_surface;
use eigenius_kernel::dcg::parse::Parser;
use eigenius_kernel::dcg::Identity;

/// Every surface the closed class claims, as the two importers see it. Kept here rather than
/// exported so the module's lists stay private; `is_closed_class_surface` is the contract.
const CLAIMED: &[&str] = &[
    // prepositions
    "for", "from", "into", "as", "with", "on", "at", "by", "of", "in", "than", "within", "upon",
    "onto", "unto", "after", "against", "per", "to", "via", "without",
    // coordinating conjunctions + subordinator
    "and", "or", "but", "nor", "if", // auxiliaries
    "has", "had", // determiners and quantifiers
    "some", "each", "every", "all", "any", "no", "several", "many", "few", "fewer", "most", "both",
    // demonstratives
    "this", "that", "these", "those", // copula
    "be", "is", "are", "was", "were", "am", "been",
];

#[test]
fn the_claimed_list_matches_the_predicate() {
    for s in CLAIMED {
        assert!(
            is_closed_class_surface(s),
            "{s} is in this test's list but `is_closed_class_surface` says no — the lists drifted"
        );
    }
}

/// The invariant itself: a bootstrap-only parser knows every surface the closed class withholds.
#[test]
fn closed_class_surfaces_stay_known() {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let parser = Parser::build(Arc::clone(ctx.head()));
    let lem = Identity;
    let mut unknown: Vec<&str> = CLAIMED
        .iter()
        .copied()
        .filter(|s| !parser.has_token(s, &lem))
        .collect();
    unknown.sort_unstable();
    // Pinned, not papered over. Each of these is withheld from both importers and has no bootstrap
    // reading, so in a real run it reaches the parser as OOV and the page augmentation grounds it as
    // though it were an unseen domain term — measured on the XIAP certification log for all three.
    // Each needs a semantic decision rather than a copied entry, which is why none is guessed here:
    //
    //   any   a free-choice / negative-polarity determiner. `exists_sem` (what `some` uses) is wrong
    //         under negation: "not any" is none, not "not some".
    //   nor   denies BOTH conjuncts. The reserved table offers `rk_coord_and`/`rk_coord_or`, and
    //         mapping `nor` to either asserts something the sentence denies; it wants its own kind.
    //   been  a past participle, so it needs a participle category, not the finite copula's.
    //
    // `am` and `then` were the two that needed no decision and are fixed: `am` is the copula
    // paradigm's missing first-person form, and `then` left the withholding list because it is an
    // adverb WordNet supplies.
    const KNOWN_GAPS: &[&str] = &["any", "been", "nor"];
    assert_eq!(
        unknown, KNOWN_GAPS,
        "the set of closed-class surfaces with no reading changed; withheld from every importer and \
         unknown to the bootstrap means no reading remains"
    );
}
