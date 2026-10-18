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

//! Tier-1 data-extraction pin (D50 §9.1) — the mechanical link from the raw
//! checksummed slices to the SampleSet arrays inlined in
//! `wrn-phase1-recompute-plans.esl`.
//!
//! Shells out to the committed canonical extractor
//! (`experiments/publications/wrn-helicase/extract/extract_samplesets.py
//! --check`), which enforces each slice's sha256, re-derives every SampleSet
//! from the pinned slice + column + filter, and diffs against the inlined
//! values — failing on any drift.
//!
//! `#[ignore]` by default: the slices are gitignored (~235 MB; see
//! `data/MANIFEST.md`), so this runs only where they are present —
//! `cargo test -p eigenius-statistics -- --ignored`. It skips gracefully
//! (does not fail) when the slices or `python3` are absent, and fails only
//! on an actual drift between the inlined arrays and the raw data.

use std::path::PathBuf;
use std::process::Command;

#[test]
#[ignore = "requires the gitignored data slices (~235 MB); run with --ignored where present"]
fn inlined_samplesets_reproduce_from_pinned_slices() {
    let wrn = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/publications/wrn-helicase");
    let script = wrn.join("extract/extract_samplesets.py");
    let slices = wrn.join("data/slices");

    if !slices.is_dir() {
        eprintln!(
            "SKIP: data slices not present at {} — fetch per data/MANIFEST.md to run this pin",
            slices.display()
        );
        return;
    }

    let output = match Command::new("python3").arg(&script).arg("--check").output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP: could not run python3 ({e}) — needed for the extraction pin");
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "extraction pin failed — inlined SampleSet arrays drifted from the pinned slices, \
         or a slice sha256 changed.\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    print!("{stdout}");
}
