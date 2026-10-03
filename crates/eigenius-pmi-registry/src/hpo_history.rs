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

//! The HP-label check (D99 §7): a registry row stores an HP ID and the label it was coded with,
//! and the two can disagree. The label is checked against the term in the loaded release (its
//! label, then its synonyms), then against the term's labels in earlier releases.
//!
//! The release history is needed because HPO drops former labels: HP:0003236 was "Elevated
//! circulating creatine kinase concentration" from 2021-06-08 to 2026-02-16, and the 2026-06-06
//! rename kept no synonym. `scripts/provision-hpo-history.sh` fetches the releases.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

/// A term as the loaded release has it.
#[derive(Debug, Clone)]
pub struct Term {
    pub label: String,
    /// `(scope, text)`, scope as OBO Graphs writes it (`hasExactSynonym`, `hasRelatedSynonym`, …).
    pub synonyms: Vec<(String, String)>,
    pub deprecated: bool,
}

/// One earlier release: each term's label in it.
#[derive(Debug, Clone)]
pub struct Release {
    /// The release date, `YYYY-MM-DD`, from the file's `data-version` header.
    pub tag: String,
    pub labels: HashMap<String, String>,
}

/// The loaded release and the earlier ones, oldest first.
#[derive(Debug)]
pub struct HpoHistory {
    pub version: String,
    terms: HashMap<String, Term>,
    /// Normalized label or synonym → the terms carrying it in the loaded release.
    by_text: HashMap<String, Vec<String>>,
    releases: Vec<Release>,
}

/// What a registry row's label is, for its HP ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The term's label in the loaded release.
    Label,
    /// One of the term's synonyms in the loaded release. `former` is the span of earlier releases
    /// where it was the label, if any.
    Synonym {
        scope: String,
        former: Option<(String, String)>,
    },
    /// The term's label in earlier releases (first and last), neither label nor synonym now.
    /// Passes; reported as stale.
    FormerLabel { first: String, last: String },
    /// The ID is not a live term of the loaded release.
    NotATerm,
    /// The label is not the term's in any release. `named` lists the loaded release's terms that
    /// carry it as label or synonym.
    Mismatch { current: String, named: Vec<String> },
}

impl Verdict {
    /// Whether a row with this verdict is converted. A mismatch or a dead ID is held back until
    /// someone declares which half of the row is right.
    pub fn passes(&self) -> bool {
        matches!(
            self,
            Verdict::Label | Verdict::Synonym { .. } | Verdict::FormerLabel { .. }
        )
    }
}

fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `http://purl.obolibrary.org/obo/HP_0000103` → `HP:0000103`.
fn curie(iri: &str) -> Option<String> {
    iri.strip_prefix("http://purl.obolibrary.org/obo/HP_")
        .map(|local| format!("HP:{local}"))
}

impl HpoHistory {
    /// The loaded release from its OBO Graphs JSON, and every `*.obo` release under `history`.
    pub fn load(hp_json: &Path, history: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(hp_json).map_err(|e| format!("{}: {e}", hp_json.display()))?;
        let mut releases = Vec::new();
        let entries =
            std::fs::read_dir(history).map_err(|e| format!("{}: {e}", history.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|x| x == "obo") {
                let obo = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                releases.push(parse_obo(&obo).map_err(|e| format!("{}: {e}", path.display()))?);
            }
        }
        Self::from_parts(&text, releases)
    }

    /// The loaded release from OBO Graphs JSON text, with the earlier releases in any order.
    pub fn from_parts(hp_json: &str, mut releases: Vec<Release>) -> Result<Self, String> {
        let doc: Value = serde_json::from_str(hp_json).map_err(|e| e.to_string())?;
        let graph = &doc["graphs"][0];
        let version = graph["meta"]["version"]
            .as_str()
            .and_then(|v| v.split("/releases/").nth(1))
            .and_then(|v| v.split('/').next())
            .ok_or("the OBO Graphs file names no release version")?
            .to_string();
        let mut terms = HashMap::new();
        let mut by_text: HashMap<String, Vec<String>> = HashMap::new();
        for node in graph["nodes"].as_array().ok_or("no nodes")? {
            let Some(id) = node["id"].as_str().and_then(curie) else {
                continue;
            };
            if node["type"].as_str() != Some("CLASS") {
                continue;
            }
            let Some(label) = node["lbl"].as_str() else {
                continue;
            };
            let meta = &node["meta"];
            let synonyms: Vec<(String, String)> = meta["synonyms"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    Some((
                        s["pred"].as_str()?.to_string(),
                        s["val"].as_str()?.to_string(),
                    ))
                })
                .collect();
            let deprecated = meta["deprecated"].as_bool().unwrap_or(false);
            if !deprecated {
                for text in std::iter::once(label).chain(synonyms.iter().map(|(_, t)| t.as_str())) {
                    let ids = by_text.entry(normalize(text)).or_default();
                    if !ids.contains(&id) {
                        ids.push(id.clone());
                    }
                }
            }
            terms.insert(
                id,
                Term {
                    label: label.to_string(),
                    synonyms,
                    deprecated,
                },
            );
        }
        releases.sort_by(|a, b| a.tag.cmp(&b.tag));
        Ok(HpoHistory {
            version,
            terms,
            by_text,
            releases,
        })
    }

    pub fn term(&self, id: &str) -> Option<&Term> {
        self.terms.get(id)
    }

    pub fn releases(&self) -> &[Release] {
        &self.releases
    }

    /// The first and last earlier release in which `label` was `id`'s label.
    fn former_span(&self, id: &str, label: &str) -> Option<(String, String)> {
        let want = normalize(label);
        let mut span: Option<(String, String)> = None;
        for r in &self.releases {
            if r.labels.get(id).is_some_and(|l| normalize(l) == want) {
                match &mut span {
                    Some((_, last)) => *last = r.tag.clone(),
                    None => span = Some((r.tag.clone(), r.tag.clone())),
                }
            }
        }
        span
    }

    /// Check a row's label against its HP ID.
    pub fn check(&self, id: &str, label: &str) -> Verdict {
        let Some(term) = self.terms.get(id).filter(|t| !t.deprecated) else {
            return Verdict::NotATerm;
        };
        let want = normalize(label);
        if normalize(&term.label) == want {
            return Verdict::Label;
        }
        if let Some((scope, _)) = term.synonyms.iter().find(|(_, t)| normalize(t) == want) {
            return Verdict::Synonym {
                scope: scope.clone(),
                former: self.former_span(id, label),
            };
        }
        if let Some((first, last)) = self.former_span(id, label) {
            return Verdict::FormerLabel { first, last };
        }
        Verdict::Mismatch {
            current: term.label.clone(),
            named: self.by_text.get(&want).cloned().unwrap_or_default(),
        }
    }
}

/// One release's `hp.obo`: its date and each `[Term]`'s label.
pub fn parse_obo(text: &str) -> Result<Release, String> {
    let mut tag = None;
    let mut labels = HashMap::new();
    let mut in_term = false;
    let mut id: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.starts_with('[') {
            in_term = line == "[Term]";
            id = None;
            continue;
        }
        if tag.is_none() {
            if let Some(v) = line.strip_prefix("data-version: ") {
                tag = v.rsplit('/').next().map(str::to_string);
            }
        }
        if !in_term {
            continue;
        }
        if let Some(v) = line.strip_prefix("id: ") {
            id = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("name: ") {
            if let Some(id) = &id {
                labels.insert(id.clone(), v.to_string());
            }
        }
    }
    let tag = tag.ok_or("no data-version header")?;
    Ok(Release { tag, labels })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &str = r#"{ "graphs": [ {
        "meta": { "version": "http://purl.obolibrary.org/obo/hp/releases/2026-09-01/hp.json" },
        "nodes": [
          { "id": "http://purl.obolibrary.org/obo/HP_0000103", "lbl": "Polyuria", "type": "CLASS" },
          { "id": "http://purl.obolibrary.org/obo/HP_0002353", "lbl": "EEG abnormality", "type": "CLASS",
            "meta": { "synonyms": [ { "pred": "hasExactSynonym", "val": "Abnormal EEG" } ] } },
          { "id": "http://purl.obolibrary.org/obo/HP_0003236", "lbl": "Elevated circulating creatine kinase activity", "type": "CLASS" },
          { "id": "http://purl.obolibrary.org/obo/HP_0004906", "lbl": "Hypernatremic dehydration", "type": "CLASS" },
          { "id": "http://purl.obolibrary.org/obo/HP_0004918", "lbl": "Hyperchloremic metabolic acidosis", "type": "CLASS" },
          { "id": "http://purl.obolibrary.org/obo/HP_0000001", "lbl": "All", "type": "CLASS",
            "meta": { "deprecated": true } }
        ] } ] }"#;

    fn obo(tag: &str, terms: &[(&str, &str)]) -> String {
        let mut s = format!("format-version: 1.2\ndata-version: hp/releases/{tag}\n\n");
        for (id, name) in terms {
            s.push_str(&format!("[Term]\nid: {id}\nname: {name}\n\n"));
        }
        s.push_str("[Typedef]\nid: part_of\nname: part of\n");
        s
    }

    fn history() -> HpoHistory {
        let ck = "HP:0003236";
        let releases = vec![
            parse_obo(&obo(
                "2021-06-08",
                &[(ck, "Elevated circulating creatine kinase concentration")],
            ))
            .unwrap(),
            parse_obo(&obo(
                "2018-03-08",
                &[(ck, "Elevated serum creatine phosphokinase")],
            ))
            .unwrap(),
            parse_obo(&obo(
                "2026-02-16",
                &[(ck, "Elevated circulating creatine kinase concentration")],
            ))
            .unwrap(),
        ];
        HpoHistory::from_parts(CURRENT, releases).unwrap()
    }

    #[test]
    fn a_release_is_dated_by_its_header_and_typedefs_are_not_terms() {
        let r = parse_obo(&obo("2018-03-08", &[("HP:0000103", "Polyuria")])).unwrap();
        assert_eq!(r.tag, "2018-03-08");
        assert_eq!(r.labels.len(), 1);
        let old =
            parse_obo("data-version: releases/2018-03-08\n[Term]\nid: HP:1\nname: x\n").unwrap();
        assert_eq!(old.tag, "2018-03-08");
    }

    #[test]
    fn the_loaded_label_and_a_synonym_pass() {
        let h = history();
        assert_eq!(h.version, "2026-09-01");
        assert_eq!(h.check("HP:0000103", "polyuria"), Verdict::Label);
        assert!(matches!(
            h.check("HP:0002353", "Abnormal EEG"),
            Verdict::Synonym { former: None, .. }
        ));
    }

    #[test]
    fn a_dropped_former_label_passes_as_stale() {
        let v = history().check(
            "HP:0003236",
            "Elevated circulating creatine kinase concentration",
        );
        assert_eq!(
            v,
            Verdict::FormerLabel {
                first: "2021-06-08".into(),
                last: "2026-02-16".into()
            }
        );
        assert!(v.passes());
    }

    #[test]
    fn another_terms_label_is_held_back_and_names_that_term() {
        let v = history().check("HP:0004918", "Hypernatremic dehydration");
        assert_eq!(
            v,
            Verdict::Mismatch {
                current: "Hyperchloremic metabolic acidosis".into(),
                named: vec!["HP:0004906".into()]
            }
        );
        assert!(!v.passes());
    }

    #[test]
    fn a_dead_or_unknown_id_is_held_back() {
        let h = history();
        assert_eq!(h.check("HP:0000001", "All"), Verdict::NotATerm);
        assert_eq!(h.check("HP:9999999", "Polyuria"), Verdict::NotATerm);
    }
}
