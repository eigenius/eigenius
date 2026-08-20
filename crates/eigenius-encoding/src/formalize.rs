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

//! **Encoding → artifact**, independent of how the run was driven (D71 §7, slice 5a).
//!
//! Everything downstream of `DocumentPipeline::encode` is the same work whether a CLI, an RPC, a
//! notebook cell or an MCP tool asked for it: map each sentence's [`SentenceOutcome`] to an
//! emission record under the run's selection authority, fail closed (or record a `CutItem` under
//! `partial`), and emit the artifact. Only the INPUTS differ — file paths on one side, request
//! fields on the other.
//!
//! This module is that shared half. It was lifted out of the `prose-to-esl` driver rather than
//! reimplemented for the served path, because the alternative is two emitters that agree until they
//! do not: the CLI's artifacts are the demo's committed, byte-compared fixtures, and a served run
//! that emitted a slightly different shape would be discovered by nothing.

use std::collections::BTreeMap;

use eigenius_kernel::dcg::pipeline::DocumentEncoding;
use eigenius_kernel::dcg::skeleton::skeleton_of;
use eigenius_kernel::dcg::SentenceOutcome;
use eigenius_kernel::ontology::resource::Resource;

use crate::emit::{
    emit_document, CutReason, CutSentence, DocumentMeta, ParsedSentence, SentenceSelection,
};
use crate::select::Pin;

/// What a run produced: the artifact and the counts a driver reports.
pub struct Artifact {
    /// Eigon-JSON. A driver that wants ESL prints it back through the ESL printer.
    pub json: String,
    pub encoded: usize,
    pub cut: usize,
    pub glossary: usize,
}

/// Everything the emission half needs, as VALUES — no paths, no clap, no snapshot.
pub struct EmissionInputs<'a> {
    /// The source text the units were segmented from — spans are `find`ed in it.
    pub doc: &'a str,
    pub encoding: &'a DocumentEncoding,
    /// The claim clusters the in-loop lander built, keyed by claim IRI. Emitting THESE rather than
    /// rebuilding is not an optimization: a landed claim carries its discourse kind as a second
    /// `is_a`, and a rebuilt cluster has none, so an anaphor resolved to it stops type-checking.
    pub landed: &'a BTreeMap<String, (Resource, Resource)>,
    /// The declared selection arm. `None` means the computed arm chose (or abstained).
    pub pins: Option<&'a BTreeMap<String, Pin>>,
    pub binding_authority: Option<&'a str>,
    /// Record non-encoding units as `CutItem`s instead of aborting (D67 §5).
    pub partial: bool,
    pub meta: DocumentMeta<'a>,
}

/// Map a document's encoding to its artifact.
pub fn emit_from_encoding(inputs: &EmissionInputs<'_>) -> Result<Artifact, String> {
    // Map each sentence's outcome to the emission record. Default: fail-closed on anything that
    // did not encode under the chosen authority. Under `--partial`: the non-encoding lands as a
    // `CutSentence` (DiscourseUnit + CutItem) and the run continues — the artifact states what
    // did not encode (D67 §5).
    let mut parsed: Vec<ParsedSentence> = Vec::new();
    let mut cuts: Vec<CutSentence> = Vec::new();
    let cut = |cuts: &mut Vec<CutSentence>, n: usize, se_text: &str, reason: CutReason| {
        let label = match &reason {
            CutReason::Ambiguous { readings } => format!("ambiguous ({readings} readings)"),
            CutReason::Unresolved { holes } => format!("unresolved ({holes} hole(s))"),
            CutReason::NoParse { oov } if !oov.is_empty() => format!("no parse (OOV: {oov:?})"),
            CutReason::NoParse { .. } => "no parse (grammar)".to_string(),
        };
        eprintln!("  [{n}] CUT — {label} — {}", se_text.trim());
        let start = inputs.doc.find(se_text).unwrap_or(0);
        cuts.push(CutSentence {
            ordinal: n,
            text: se_text.to_string(),
            span: (start, start + se_text.len()),
            reason,
        });
    };
    for (i, se) in inputs.encoding.sentences.iter().enumerate() {
        let n = i + 1;
        let text = se.text.trim();
        let item = match &se.outcome {
            SentenceOutcome::Encoded(item) => item,
            SentenceOutcome::Ambiguous(pool) => {
                if inputs.partial {
                    cut(
                        &mut cuts,
                        n,
                        &se.text,
                        CutReason::Ambiguous {
                            readings: pool.len(),
                        },
                    );
                    continue;
                }
                let skels: Vec<String> = pool.iter().map(|it| skeleton_of(it.sem())).collect();
                return Err(match &inputs.pins {
                    Some(pins) => match pins.get(text) {
                        None => format!("sentence {n} «{text}»: no pin, {} readings", pool.len()),
                        Some(pin) => {
                            let hits = skels.iter().filter(|s| **s == pin.skeleton).count();
                            if hits == 0 {
                                format!(
                                    "sentence {n} «{text}»: the pinned skeleton matches none of \
                                     the {} readings\n  pinned: {}\n  forest:\n    {}",
                                    pool.len(),
                                    pin.skeleton,
                                    skels.join("\n    ")
                                )
                            } else {
                                format!(
                                    "sentence {n} «{text}»: the pinned skeleton matches {hits} \
                                     readings — a sense-level tie a skeleton pin cannot break \
                                     (fail-closed)",
                                )
                            }
                        }
                    },
                    None => format!(
                        "sentence {n} «{text}»: the selection replay abstained or missed \
                         ({} readings) — the recording does not answer this question",
                        pool.len()
                    ),
                });
            }
            SentenceOutcome::Open(o) => {
                if inputs.partial {
                    cut(
                        &mut cuts,
                        n,
                        &se.text,
                        CutReason::Unresolved {
                            holes: o.holes.len(),
                        },
                    );
                    continue;
                }
                return Err(format!(
                    "sentence {n} «{text}»: {} unresolved referent hole(s) — provide --proposals \
                     with a recorded draw that resolves them, or pick prose without anaphora",
                    o.holes.len()
                ));
            }
            SentenceOutcome::Gap => {
                if inputs.partial {
                    // Classify: residual Stage-A OOV surfaces occurring in this sentence make it
                    // a vocabulary cut; none makes it a grammar cut.
                    let oov: Vec<String> = inputs
                        .encoding
                        .augmentation
                        .missing_oov
                        .iter()
                        .map(|g| g.surface.clone())
                        .filter(|s| contains_word(&se.text, s))
                        .collect();
                    cut(&mut cuts, n, &se.text, CutReason::NoParse { oov });
                    continue;
                }
                return Err(format!(
                    "sentence {n} «{text}»: no parse — a grammar gap or out-of-vocabulary tokens"
                ));
            }
        };
        // The emission's selection record. Under pins, verify the encoded reading IS the pinned
        // one even when it was the sole survivor (the ranker only fires on pools > 1). A pin
        // CONTRADICTION stays fatal even under --partial (pin drift, not a coverage gap); a
        // missing pin for a sole survivor is tolerated under --partial (no choice existed).
        let selection = match &inputs.pins {
            Some(pins) => match pins.get(text) {
                None if inputs.partial => SentenceSelection::Sole,
                None => return Err(format!("sentence {n} «{text}»: no pin")),
                Some(pin) => {
                    let sk = skeleton_of(item.sem());
                    if sk != pin.skeleton {
                        return Err(format!(
                            "sentence {n} «{text}»: the encoded reading is not the pinned one\n  \
                             pinned: {}\n  got:    {sk}",
                            pin.skeleton
                        ));
                    }
                    SentenceSelection::Pinned(pin)
                }
            },
            None => match &se.selection {
                Some(sel) => SentenceSelection::Ranked(sel),
                None => SentenceSelection::Sole,
            },
        };
        let candidates = se.selection.as_ref().map(|s| s.candidates).unwrap_or(1);
        eprintln!(
            "  [{n}] encoded (of {candidates} reading(s)){} — {text}",
            if se.resolution.is_some() {
                " [anaphora resolved]"
            } else {
                ""
            }
        );
        let start = inputs.doc.find(se.text.as_str()).unwrap_or(0);
        parsed.push(ParsedSentence {
            ordinal: n,
            text: se.text.clone(),
            span: (start, start + se.text.len()),
            item,
            candidates,
            selection,
            bindings: se
                .resolution
                .as_ref()
                .map(|r| r.bindings.clone())
                .unwrap_or_default(),
            binding_authority: inputs.binding_authority,
            cluster: inputs
                .landed
                .get(&format!("{}:claim_{n}", inputs.meta.ns))
                .cloned(),
        });
    }

    // Stage-A glossary resources go into the artifact — the entries that grounded the parse
    // (a claim's proposition may reference a doc-glossary-only concept; without them the
    // artifact does not load on a chain that lacks the doc branch).
    let glossary = inputs.encoding.augmentation.resources();
    if !glossary.is_empty() {
        eprintln!("glossary: {} Stage-A resource(s) emitted", glossary.len());
    }
    let json = emit_document(&inputs.meta, &glossary, &parsed, &cuts).map_err(|e| e.to_string())?;
    Ok(Artifact {
        json,
        encoded: parsed.len(),
        cut: cuts.len(),
        glossary: glossary.len(),
    })
}

/// Does `sentence` contain `word` as a whole token (case-insensitive)? Attributing a residual
/// OOV surface to a sentence by substring would credit «then» to «strengthen» — the artifact's
/// cut reason has to name surfaces the sentence actually contains. Alphanumerics bound a token;
/// a hyphen does not (`Cas9-mediated` is one Stage-A surface).
fn contains_word(sentence: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let hay: Vec<char> = sentence.to_lowercase().chars().collect();
    let needle: Vec<char> = word.to_lowercase().chars().collect();
    let bounded = |c: Option<&char>| c.is_none_or(|c| !c.is_alphanumeric());
    hay.windows(needle.len()).enumerate().any(|(i, w)| {
        w == needle.as_slice()
            && bounded(i.checked_sub(1).and_then(|p| hay.get(p)))
            && bounded(hay.get(i + needle.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::contains_word;

    #[test]
    fn oov_attribution_is_token_bounded() {
        assert!(contains_word("We then evaluated MSI.", "then"));
        assert!(!contains_word("Chromatin can strengthen it.", "then"));
        assert!(contains_word(
            "CRISPR–Cas9-mediated knockout",
            "Cas9-mediated"
        ));
        assert!(contains_word("essential in vitro and in vivo", "VITRO"));
        assert!(!contains_word("nitrovitrogen", "vitro"));
        assert!(!contains_word("anything", ""));
    }
}
