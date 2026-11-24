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

/// One sense-resolved reading pin: surface text and exact normalized `sem` term from a `correct`
/// row in `reading-adjudications.tsv` (D103 §3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingPin {
    pub sentence: String,
    pub sem: String,
    pub note: String,
}

/// Load sense-resolved `correct` reading pins from a D63 `reading-adjudications.tsv` file:
/// `sentence <TAB> sem <TAB> verdict [<TAB> evidence [<TAB> basis]]`.
///
/// Rows with verdict other than `correct` (`wrong`, `uncertain`) are ignored. Two `correct` rows
/// for the same sentence with differing `sem` strings return `InvalidData` (fail-closed against
/// ledger contradictions).
pub fn load_reading_pins(path: &Path) -> std::io::Result<BTreeMap<String, ReadingPin>> {
    let text = std::fs::read_to_string(path)?;
    let mut out: BTreeMap<String, ReadingPin> = BTreeMap::new();
    for (line_no, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(sentence), Some(sem), Some(verdict)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        if verdict.trim() != "correct" {
            continue;
        }
        let sentence = sentence.trim().to_string();
        let sem = sem.trim().to_string();
        let evidence = cols.next().unwrap_or("").trim();
        let basis = cols.next().unwrap_or("").trim();
        let note = match (evidence.is_empty(), basis.is_empty()) {
            (true, true) => String::new(),
            (false, true) => evidence.to_string(),
            (true, false) => basis.to_string(),
            (false, false) => format!("{evidence} | {basis}"),
        };
        if let Some(prev) = out.get(&sentence) {
            if prev.sem != sem {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "{}:{}: contradictory `correct` reading pin for sentence «{sentence}»",
                        path.display(),
                        line_no + 1
                    ),
                ));
            }
            continue;
        }
        out.insert(
            sentence.clone(),
            ReadingPin {
                sentence,
                sem,
                note,
            },
        );
    }
    Ok(out)
}

/// Load a D102 unit-anchor map: `ordinal <TAB> discourse_unit_iri` (1-based CNL line ordinal to
/// pre-existing chain `enc:DiscourseUnit` IRI). Comments (`#`), blank lines, and an optional
/// non-numeric header row are ignored.
pub fn load_doc_unit_map(path: &Path) -> std::io::Result<BTreeMap<usize, String>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = BTreeMap::new();
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut cols = trimmed.split('\t');
        let (Some(ord_s), Some(unit_iri)) = (cols.next(), cols.next()) else {
            continue;
        };
        let Ok(ord) = ord_s.trim().parse::<usize>() else {
            if idx == 0 {
                continue; // header row
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: expected 1-based integer ordinal, got `{ord_s}`",
                    path.display(),
                    idx + 1
                ),
            ));
        };
        out.insert(ord, unit_iri.trim().to_string());
    }
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_reading_pins_keeps_only_correct_and_rejects_contradictions() {
        let dir = std::env::temp_dir();
        let ok_path = dir.join("eigenius_test_reading_pins_ok.tsv");
        std::fs::write(
            &ok_path,
            "# comment\n\
             A gene mutates.\tsem_a\tcorrect\tunit 1\thuman\n\
             A cell divides.\tsem_b\tuncertain\tunit 2\n\
             A cell divides.\tsem_c\twrong\tunit 2\n",
        )
        .unwrap();
        let pins = load_reading_pins(&ok_path).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins["A gene mutates."].sem, "sem_a");
        assert_eq!(pins["A gene mutates."].note, "unit 1 | human");

        let bad_path = dir.join("eigenius_test_reading_pins_bad.tsv");
        std::fs::write(
            &bad_path,
            "A gene mutates.\tsem_a\tcorrect\tunit 1\n\
             A gene mutates.\tsem_other\tcorrect\tunit 1\n",
        )
        .unwrap();
        assert!(load_reading_pins(&bad_path).is_err());
    }
}
