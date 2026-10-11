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

//! The recognizer over the ACTUAL residual-OOV set of the XIAP document model, not hand-picked
//! cases — hand-picked positives tell you the pattern matches what you wrote it for, and say
//! nothing about what else it swallows. Reads the token list from `NOTATION_OOV_TYPES` (a
//! derivative of gitignored material), so it skips when unset.
//!
//!     NOTATION_OOV_TYPES=/path/oov-types.txt \
//!       cargo test -p eigenius-notation --test over_the_real_oov_set -- --ignored --nocapture

use eigenius_notation::{recognize, Notation};

#[test]
#[ignore = "reads a derivative of gitignored material; set NOTATION_OOV_TYPES"]
fn it_classifies_the_real_oov_set() {
    let Ok(path) = std::env::var("NOTATION_OOV_TYPES") else {
        eprintln!("SKIP: NOTATION_OOV_TYPES unset");
        return;
    };
    let src = std::fs::read_to_string(&path).expect("token list readable");
    let toks: Vec<&str> = src.lines().filter(|l| !l.trim().is_empty()).collect();

    let mut by_class: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    let mut unrecognised = Vec::new();
    for t in &toks {
        match recognize(t) {
            Some(n) => by_class
                .entry(match n {
                    Notation::ProteinChange(_) => "ProteinChange",
                    Notation::CodingChange(_) => "CodingChange",
                    Notation::ResiduePosition { .. } => "ResiduePosition",
                    Notation::ResidueRange { .. } => "ResidueRange",
                    Notation::UniProt(_) => "UniProt",
                    Notation::PdbEntry(_) => "PdbEntry",
                    Notation::DbSnp(_) => "DbSnp",
                    Notation::ClinVar(_) => "ClinVar",
                    Notation::SequenceRecord(_) => "SequenceRecord",
                    Notation::GenomicCoordinate(_) => "GenomicCoordinate",
                })
                .or_default()
                .push(t),
            None => unrecognised.push(*t),
        }
    }
    let hit: usize = by_class.values().map(Vec::len).sum();
    eprintln!("\n{hit} of {} OOV types recognised\n", toks.len());
    for (k, v) in &by_class {
        eprintln!("  {k:<18} {:>3}   {}", v.len(), v.join(", "));
    }
    eprintln!("\n  unrecognised ({}):", unrecognised.len());
    for chunk in unrecognised.chunks(6) {
        eprintln!("    {}", chunk.join(", "));
    }
}
