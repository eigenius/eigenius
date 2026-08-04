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

//! **The declared half of the encoding** — which domain proposition each sentence is taken to
//! warrant, and on whose authority.
//!
//! The parser decides what a sentence *means*. It cannot decide that the meaning warrants a claim in
//! domain vocabulary; that is a human judgement, so it is read from a file that names its author and
//! its grounds, and it commits as a Declared bridge
//! ([`eigenius_reasoning::grade::BridgedClaimGrader`]).

use std::collections::BTreeMap;
use std::path::Path;

/// One row: a sentence, and the domain proposition it is declared to warrant.
#[derive(Clone, Debug)]
pub struct ClaimSpec {
    pub sentence: String,
    /// The domain predicate, e.g. `urn:eigenius:benchmark:onco:RequiresActivity`.
    pub predicate: String,
    /// Its `core:string` arguments, in order.
    pub args: Vec<String>,
    /// The `reasoning:subject_iri` index for the sentence.
    pub subject_iri: String,
    /// The authority the lift rests on.
    pub declared_by: String,
    /// Why the lift is warranted.
    pub rationale: String,
}

/// Load `sentence <TAB> predicate <TAB> arg,arg <TAB> subject_iri <TAB> declared_by <TAB> rationale`.
/// `#` comments and blank lines are ignored; a duplicate sentence is an error rather than a silent
/// overwrite.
pub fn load_claims(path: &Path) -> std::io::Result<BTreeMap<String, ClaimSpec>> {
    let bad = |msg: String| std::io::Error::new(std::io::ErrorKind::InvalidData, msg);
    let text = std::fs::read_to_string(path)?;
    let mut out = BTreeMap::new();
    for (lineno, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 6 {
            return Err(bad(format!(
                "{}:{}: expected 6 tab-separated columns (sentence, predicate, args, subject_iri, \
                 declared_by, rationale), got {}",
                path.display(),
                lineno + 1,
                c.len()
            )));
        }
        let sentence = c[0].trim().to_string();
        let prior = out.insert(
            sentence.clone(),
            ClaimSpec {
                sentence,
                predicate: c[1].trim().to_string(),
                args: c[2]
                    .split(',')
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect(),
                subject_iri: c[3].trim().to_string(),
                declared_by: c[4].trim().to_string(),
                rationale: c[5].trim().to_string(),
            },
        );
        if prior.is_some() {
            return Err(bad(format!(
                "{}:{}: a second claim for the same sentence — one of them would be silently dropped",
                path.display(),
                lineno + 1
            )));
        }
    }
    Ok(out)
}
