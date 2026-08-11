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

//! **Reading selection — two arms, both fail-closed.**
//!
//! - [`select_pinned`] — the DECLARED arm: reads the human-verified skeleton out of a pin file
//!   (the `experiments/parsing/expected-readings.tsv` format) and keeps the one reading whose
//!   sense-erased skeleton equals it. No match is an error (the pin is stale or the grammar
//!   moved); *several* matches is also an error (they differ only in sense — a skeleton pin
//!   cannot adjudicate that).
//! - [`select_ranked`] — the COMPUTED arm (`d63-reading-selection.md`): the kernel's
//!   `Parser::select_reading` presents the readings to a [`ReadingRanker`] (in this pipeline, a
//!   replayed recording — artifact generation stays deterministic) in document context. An
//!   abstention — including a replay miss — is an error: nothing is emitted for an unchosen
//!   sentence.
//!
//! Neither arm has a kernel veto (every candidate reading type-checks); the pin arm's authority
//! is the human, the ranked arm's controls are the recorded decision (emitted as the
//! `enc:DecisionPoint`) and the reading-adjudication ledger.

use std::collections::BTreeMap;
use std::path::Path;

use eigenius_kernel::dcg::item::Item;
use eigenius_kernel::dcg::skeleton::skeleton_of;
use eigenius_kernel::dcg::{Lemmatizer, Parser, PriorSelection, ReadingRanker, SelectionOutcome};

/// One pinned sentence: its surface text and the sense-erased skeleton of the verified reading.
#[derive(Clone, Debug)]
pub struct Pin {
    pub sentence: String,
    pub skeleton: String,
    /// The pin file's note column — the human's verification record. Carried through to the emitted
    /// resource so the chain records *why* this reading was the one taken.
    pub note: String,
}

/// Why selection failed. Both variants carry the candidate skeletons, because a stale pin is
/// diagnosed by looking at what the parser actually produced.
#[derive(Debug)]
pub enum SelectError {
    NoPin {
        sentence: String,
    },
    NoMatch {
        sentence: String,
        pin: String,
        got: Vec<String>,
    },
    Ambiguous {
        sentence: String,
        pin: String,
        n: usize,
    },
    /// The ranked arm's ranker abstained (for a replayed recording: a key MISS — the document,
    /// forest, glosses, or an upstream selection changed under it).
    Abstained {
        sentence: String,
        candidates: usize,
    },
}

impl std::fmt::Display for SelectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPin { sentence } => write!(
                f,
                "no pinned reading for «{sentence}» — add one to the pin file (a reading nobody \
                 verified must not be encoded)"
            ),
            Self::NoMatch { sentence, pin, got } => {
                writeln!(
                    f,
                    "«{sentence}»: the pinned reading is NOT among the {} the parser produced.\n  \
                     pin: {pin}",
                    got.len()
                )?;
                for g in got {
                    writeln!(f, "  got: {g}")?;
                }
                Ok(())
            }
            Self::Ambiguous { sentence, pin, n } => write!(
                f,
                "«{sentence}»: {n} readings share the pinned skeleton — they differ only in sense, \
                 and choosing between them is not this crate's call.\n  pin: {pin}"
            ),
            Self::Abstained {
                sentence,
                candidates,
            } => write!(
                f,
                "«{sentence}»: the reading ranker ABSTAINED ({candidates} candidates) — nothing is \
                 emitted for an unchosen sentence. For a replayed recording this is a key MISS: \
                 the document, forest, glosses, or an upstream selection changed under it; \
                 re-record the draw (scripts/measure-parse-rate.sh --selections <new-file>) and \
                 adjudicate it, or pin the sentence."
            ),
        }
    }
}

/// Load a pin file: `sentence <TAB> skeleton [<TAB> note]`, `#` comments, blank lines ignored.
pub fn load_pins(path: &Path) -> std::io::Result<BTreeMap<String, Pin>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(sentence), Some(skeleton)) = (cols.next(), cols.next()) else {
            continue;
        };
        let sentence = sentence.trim().to_string();
        out.insert(
            sentence.clone(),
            Pin {
                sentence,
                skeleton: skeleton.trim().to_string(),
                note: cols.next().unwrap_or("").trim().to_string(),
            },
        );
    }
    Ok(out)
}

/// The one reading whose skeleton equals the pin. See the module docs for why both "none" and
/// "several" are errors.
pub fn select_pinned<'a, 'p>(
    sentence: &str,
    readings: &'a [Item],
    pins: &'p BTreeMap<String, Pin>,
) -> Result<(&'a Item, &'p Pin), SelectError> {
    let Some(pin) = pins.get(sentence) else {
        return Err(SelectError::NoPin {
            sentence: sentence.to_string(),
        });
    };
    let matched: Vec<&Item> = readings
        .iter()
        .filter(|it| skeleton_of(it.sem()) == pin.skeleton)
        .collect();
    match matched.len() {
        0 => {
            let mut got: Vec<String> = readings.iter().map(|it| skeleton_of(it.sem())).collect();
            got.sort();
            got.dedup();
            Err(SelectError::NoMatch {
                sentence: sentence.to_string(),
                pin: pin.skeleton.clone(),
                got,
            })
        }
        1 => Ok((matched[0], pin)),
        n => Err(SelectError::Ambiguous {
            sentence: sentence.to_string(),
            pin: pin.skeleton.clone(),
            n,
        }),
    }
}

/// The COMPUTED arm: put the readings to the `ranker` through the kernel's ONE presentation
/// function (`Parser::select_reading` — the same candidates, grouping, and context the
/// measurement harness and the discourse loop run). Returns the chosen index into `readings`
/// plus the audit record. An abstention fails closed — see [`SelectError::Abstained`].
#[allow(clippy::too_many_arguments)]
pub fn select_ranked(
    parser: &Parser,
    ranker: &dyn ReadingRanker,
    document: &str,
    sentence: &str,
    lemmatizer: &dyn Lemmatizer,
    prior: &[PriorSelection],
    readings: &[Item],
) -> Result<(usize, SelectionOutcome), SelectError> {
    match parser.select_reading(ranker, document, sentence, lemmatizer, prior, readings) {
        Some((idx, sel)) => Ok((idx, sel)),
        None => Err(SelectError::Abstained {
            sentence: sentence.to_string(),
            candidates: readings.len(),
        }),
    }
}
