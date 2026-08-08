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

//! **Reading selection — declared, not solved.**
//!
//! Structural disambiguation (D62 S4) is open work: the reference page runs 60 of 62 units
//! ambiguous. So this crate does not *decide* which reading is right. It reads the human-verified
//! skeleton out of a pin file (the format of `experiments/parsing/expected-readings.tsv`) and keeps
//! the readings whose sense-erased skeleton equals it.
//!
//! **Fail closed, both ways.** No match is an error — the pin is stale or the grammar moved, and
//! either way the demo must not encode a reading nobody verified. *Several* matches is also an
//! error: readings sharing a skeleton differ only in sense, and picking one arbitrarily would be
//! exactly the silent, unwitnessed choice this crate exists to avoid.

use std::collections::BTreeMap;
use std::path::Path;

use eigenius_kernel::dcg::item::Item;
use eigenius_kernel::dcg::skeleton::skeleton_of;

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
