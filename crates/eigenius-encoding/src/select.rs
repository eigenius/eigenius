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

//! **The pin file** — the DECLARED selection authority's data: `sentence <TAB> skeleton
//! [<TAB> note]`, the `experiments/parsing/expected-readings.tsv` format. Since D67 §3 the
//! selection itself runs inside the kernel's discourse loop (`PinReadingRanker` — the pins map
//! is handed to it; ties and misses surface as `Ambiguous` outcomes the CLI fails closed on),
//! so this module keeps only the format: the [`Pin`] record (whose note column the emission
//! carries onto the chain) and [`load_pins`]. The old `select_pinned`/`select_ranked` free
//! functions were the pre-pipeline private selection path and are gone with it.

use std::collections::BTreeMap;
use std::path::Path;

/// One pinned sentence: its surface text and the sense-erased skeleton of the verified reading.
#[derive(Clone, Debug)]
pub struct Pin {
    pub sentence: String,
    pub skeleton: String,
    /// The pin file's note column — the human's verification record. Carried through to the emitted
    /// resource so the chain records *why* this reading was the one taken.
    pub note: String,
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
