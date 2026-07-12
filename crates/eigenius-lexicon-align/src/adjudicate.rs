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

//! **The adjudicator** — does one concept underlie both glosses?
//!
//! The one thing a threshold cannot do. Gloss overlap identifies the *obvious* duplicates
//! (`abbreviation`: "A shortened form of a word or phrase." ≡ "a shortened form of a word or
//! phrase"), but **94% of candidate pairs have overlap ≤ 0.25**, and that bucket mixes genuinely
//! different concepts with the same concept worded differently. Only a reader can tell them apart.
//!
//! **The LLM is untrusted, and treated as such:**
//! - Its verdicts are **recorded** (`alignment.jsonl`), never taken live at build time — the lexicon
//!   must be reproducible. (`temperature: 0` is *not* deterministic: two runs of the sense reranker
//!   differed on 5% of decisions, measured 2026-07-11.)
//! - It is **scored against the 294 gold pairs first**. If it cannot recover pairs whose glosses are
//!   near-identical, it cannot be trusted on the hard ones, and the approach fails here rather than
//!   three stages downstream.
//! - Its output is a **proposal**. The gate that decides whether an alignment is *good* is the parse
//!   measurement: `grammar-gap` must stay 0. A wrong merge destroys a reading, and that shows up.

use serde::{Deserialize, Serialize};

use crate::Candidate;

/// One adjudicated pair.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verdict {
    pub cui: String,
    pub offset: String,
    pub surface: String,
    /// `true` ⇒ the two denote the same concept and should be unified.
    pub same: bool,
    /// The model's confidence, 0–1. Used to set the merge threshold, not to decide the verdict.
    pub confidence: f32,
    /// One line: why. Kept so a bad merge can be traced to its reasoning.
    pub reason: String,
}

/// What the model returns for one batch — one row per pair, in the order given.
#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchReply {
    pub verdicts: Vec<Row>,
}

#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Row {
    /// The pair's index in the batch (0-based) — so a reordered or short reply is caught.
    pub index: usize,
    /// Do the two glosses define the SAME concept?
    pub same: bool,
    /// 0.0–1.0.
    pub confidence: f32,
    /// One short sentence.
    pub reason: String,
}

/// The prompt for one batch of candidate pairs.
#[cfg(feature = "use-llm")]
pub fn prompt(batch: &[&Candidate]) -> String {
    let mut p = String::from(
        "You are aligning two lexicons: WordNet (general English) and UMLS (biomedical).\n\
         Each item below is one WORD, with a UMLS definition and a WordNet definition.\n\n\
         For each item decide: do the two definitions define the SAME CONCEPT?\n\n\
         Rules:\n\
         - SAME means a speaker would take them to be the same thing, so one entry in a lexicon \
           could serve for both. Different wording is fine — judge the meaning, not the phrasing.\n\
         - DIFFERENT means they are distinct senses of a shared spelling (a `lead` that is a metal \
           vs a `lead` that is a clue), or one is a specialisation the other is not.\n\
         - When the UMLS sense is a narrow clinical reading of a broad ordinary word, that is \
           DIFFERENT.\n\
         - Some UMLS concepts have NO definition. Judge them from the name, the semantic type and \
           the atoms. An atom marked `(attribute)`, `(qualifier value)` or `(finding)` tells you \
           what KIND of thing the concept is.\n\
         - REJECT metadata artefacts: a concept that names a DISCIPLINE, a CODE, an ANSWER, a \
           DOCUMENT SECTION or a data-entry qualifier is not the ordinary word it is spelled like. \
           `Specialty Type - cancer` is the medical field, not the disease — that is DIFFERENT.\n\
         - If you cannot tell, answer same=false with low confidence. A wrong merge destroys a \
           reading; a missed merge only leaves things as they are. Prefer to miss.\n\n",
    );
    for (i, c) in batch.iter().enumerate() {
        // **51% of UMLS concepts have no definition** — including ordinary English abstract nouns
        // like `Deficiency`, whose surface IS a WordNet lemma. Requiring a gloss excluded them all.
        // For those, describe the concept by what UMLS *does* record: its preferred name, its
        // semantic type, and its atoms. The fully-specified names are the load-bearing part —
        // `Deficiency (attribute)`, `Deficient (qualifier value)` say what KIND of thing it is,
        // which is how a real merge is told apart from a metadata artefact such as
        // `Specialty Type - cancer` (a *discipline*, not the disease).
        let umls = if c.umls_gloss.trim().is_empty() {
            format!(
                "name \"{}\"; semantic type: {}; atoms: {}   [no definition in UMLS]",
                c.umls_name,
                c.tuis.join(", "),
                c.umls_atoms.join("; ")
            )
        } else {
            format!(
                "{} (semantic type: {})",
                c.umls_gloss.trim(),
                c.tuis.join(", ")
            )
        };
        p.push_str(&format!(
            "[{i}] word: {}\n  UMLS: {}\n  WordNet: {}\n\n",
            c.surface,
            umls,
            c.wn_gloss.trim(),
        ));
    }
    p.push_str("Return one verdict per item, with its index.");
    p
}

/// Adjudicate one batch. `Err` on any transport/API/decode failure — the caller fails closed and
/// records nothing, rather than silently treating a failed call as "different".
#[cfg(feature = "use-llm")]
pub async fn adjudicate_batch(
    api_key: &str,
    model: &str,
    batch: &[&Candidate],
) -> Result<Vec<Verdict>, String> {
    let reply: BatchReply = eigenius_kernel::dcg::anthropic_client::anthropic_structured(
        api_key,
        model,
        &prompt(batch),
    )
    .await?;

    let mut out = Vec::with_capacity(batch.len());
    for r in reply.verdicts {
        let Some(c) = batch.get(r.index) else {
            return Err(format!(
                "model returned index {} for a batch of {}",
                r.index,
                batch.len()
            ));
        };
        out.push(Verdict {
            cui: c.cui.clone(),
            offset: c.offset.clone(),
            surface: c.surface.clone(),
            same: r.same,
            confidence: r.confidence.clamp(0.0, 1.0),
            reason: r.reason,
        });
    }
    Ok(out)
}

/// Score the adjudicator against the gold set — the check that decides whether it can be trusted.
///
/// **Recall is what matters here.** Every gold pair has near-identical glosses; a judge that cannot
/// call those the same concept is not a judge. A low score means stop.
pub fn score_against_gold(gold: &[&Candidate], verdicts: &[Verdict]) -> GoldScore {
    let by_key: std::collections::BTreeMap<(&str, &str), &Verdict> = verdicts
        .iter()
        .map(|v| ((v.cui.as_str(), v.offset.as_str()), v))
        .collect();
    let mut hit = 0usize;
    let mut miss: Vec<(&str, &str)> = Vec::new();
    for g in gold {
        match by_key.get(&(g.cui.as_str(), g.offset.as_str())) {
            Some(v) if v.same => hit += 1,
            Some(v) => miss.push((g.surface.as_str(), v.reason.as_str())),
            None => miss.push((g.surface.as_str(), "(not adjudicated)")),
        }
    }
    GoldScore {
        total: gold.len(),
        recovered: hit,
        missed: miss
            .iter()
            .map(|(s, r)| (s.to_string(), r.to_string()))
            .collect(),
    }
}

/// The adjudicator's score on the gold set.
#[derive(Debug)]
pub struct GoldScore {
    pub total: usize,
    pub recovered: usize,
    /// The gold pairs it called *different*, with its stated reason — read these; they are where the
    /// judge is wrong, or the gold set is.
    pub missed: Vec<(String, String)>,
}

impl GoldScore {
    pub fn recall(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.recovered as f32 / self.total as f32
        }
    }
}
