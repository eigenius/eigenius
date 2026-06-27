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

//! Stratification checker for EigenQL DEFINE rules.
//!
//! Builds a dependency graph from DEFINE rules, detects negation cycles,
//! and computes strata ordering for evaluation.

use crate::query::ast::*;
use crate::query::error::QueryError;
use std::collections::{BTreeMap, BTreeSet};

/// A stratum: a group of relations that can be evaluated together.
#[derive(Debug)]
pub struct Stratum {
    pub relations: Vec<String>,
    pub order: usize,
}

/// Check stratification of DEFINE rules and compute evaluation order.
///
/// Returns strata in evaluation order, or an error if a negation cycle exists.
pub fn stratify(definitions: &[RuleDefinition]) -> Result<Vec<Stratum>, QueryError> {
    if definitions.is_empty() {
        return Ok(vec![]);
    }

    // Collect all defined relation names
    let relation_names: BTreeSet<String> = definitions.iter().map(|d| d.name.clone()).collect();

    // Build dependency graph: for each relation, which relations it references
    // positively and negatively
    let mut pos_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut neg_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for def in definitions {
        let pos = pos_deps.entry(def.name.clone()).or_default();
        let neg = neg_deps.entry(def.name.clone()).or_default();

        for pattern in def.body.patterns() {
            if let Some(Name::ShortName(ref class_name)) = pattern.class {
                if relation_names.contains(class_name) {
                    if pattern.negated {
                        neg.insert(class_name.clone());
                    } else {
                        pos.insert(class_name.clone());
                    }
                }
            }
        }
    }

    // Check for negation cycles using DFS
    // A negation cycle exists if there's a cycle in the dependency graph
    // that passes through at least one negative edge
    for relation in &relation_names {
        if has_negation_cycle(
            relation,
            relation,
            false,
            &pos_deps,
            &neg_deps,
            &mut BTreeSet::new(),
        ) {
            return Err(QueryError::stratification(format!(
                "negation cycle detected involving relation '{relation}'"
            )));
        }
    }

    // Compute strata via topological sort considering negative edges
    // Relations connected only by positive edges share a stratum.
    // A negative edge forces the negated relation into a lower stratum.
    let mut stratum_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut changed = true;

    // Initialize all relations to stratum 0
    for name in &relation_names {
        stratum_map.insert(name.clone(), 0);
    }

    // Iterate until stable
    while changed {
        changed = false;
        for name in &relation_names {
            let current = stratum_map[name];

            // Positive dependencies: must be in the same or lower stratum
            if let Some(deps) = pos_deps.get(name) {
                for dep in deps {
                    if let Some(&dep_stratum) = stratum_map.get(dep) {
                        if dep_stratum > current {
                            stratum_map.insert(name.clone(), dep_stratum);
                            changed = true;
                        }
                    }
                }
            }

            // Negative dependencies: negated relation must be in a strictly lower stratum
            if let Some(deps) = neg_deps.get(name) {
                for dep in deps {
                    if let Some(&dep_stratum) = stratum_map.get(dep) {
                        if dep_stratum >= stratum_map[name] {
                            stratum_map.insert(name.clone(), dep_stratum + 1);
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Group relations by stratum and sort
    let mut strata_groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (name, stratum) in &stratum_map {
        strata_groups
            .entry(*stratum)
            .or_default()
            .push(name.clone());
    }

    let strata: Vec<Stratum> = strata_groups
        .into_iter()
        .map(|(order, relations)| Stratum { relations, order })
        .collect();

    Ok(strata)
}

/// DFS to detect negation cycles.
fn has_negation_cycle(
    start: &str,
    current: &str,
    seen_negation: bool,
    pos_deps: &BTreeMap<String, BTreeSet<String>>,
    neg_deps: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if !visited.insert(current.to_string()) {
        // We've reached a node we already visited
        return current == start && seen_negation;
    }

    // Follow positive edges
    if let Some(deps) = pos_deps.get(current) {
        for dep in deps {
            if dep == start && seen_negation {
                return true;
            }
            if has_negation_cycle(start, dep, seen_negation, pos_deps, neg_deps, visited) {
                return true;
            }
        }
    }

    // Follow negative edges
    if let Some(deps) = neg_deps.get(current) {
        for dep in deps {
            if dep == start {
                return true; // Any cycle through a negative edge is bad
            }
            if has_negation_cycle(start, dep, true, pos_deps, neg_deps, visited) {
                return true;
            }
        }
    }

    visited.remove(current);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_def(name: &str, patterns: Vec<(&str, bool)>) -> RuleDefinition {
        RuleDefinition {
            name: name.to_string(),
            variables: vec![Variable::new("x")],
            body: MatchPart {
                using: vec![],
                using_institutions: vec![],
                using_namespaces: vec![],
                clauses: patterns
                    .into_iter()
                    .map(|(class, negated)| {
                        Clause::Pattern(Pattern {
                            subject: Variable::new("x"),
                            class: Some(Name::ShortName(class.to_string())),
                            properties: vec![],
                            negated,
                        })
                    })
                    .collect(),
                conditions: vec![],
            },
        }
    }

    #[test]
    fn no_definitions() {
        let strata = stratify(&[]).unwrap();
        assert!(strata.is_empty());
    }

    #[test]
    fn non_recursive_single_rule() {
        let defs = vec![make_def("A", vec![])];
        let strata = stratify(&defs).unwrap();
        assert_eq!(strata.len(), 1);
        assert_eq!(strata[0].relations, vec!["A"]);
    }

    #[test]
    fn self_recursive_no_negation() {
        // A depends on A positively — fine
        let defs = vec![make_def("A", vec![("A", false)])];
        let strata = stratify(&defs).unwrap();
        assert_eq!(strata.len(), 1);
    }

    #[test]
    fn valid_stratification() {
        // HasParent depends on nothing
        // Orphan negates HasParent — valid (two strata)
        let defs = vec![
            make_def("HasParent", vec![]),
            make_def("Orphan", vec![("HasParent", true)]),
        ];
        let strata = stratify(&defs).unwrap();
        assert!(strata.len() >= 2);
        // HasParent should be in a lower stratum than Orphan
        let hp_stratum = strata
            .iter()
            .find(|s| s.relations.contains(&"HasParent".to_string()))
            .unwrap()
            .order;
        let orphan_stratum = strata
            .iter()
            .find(|s| s.relations.contains(&"Orphan".to_string()))
            .unwrap()
            .order;
        assert!(hp_stratum < orphan_stratum);
    }

    #[test]
    fn negation_cycle_rejected() {
        // A negates B, B negates A — cycle!
        let defs = vec![
            make_def("A", vec![("B", true)]),
            make_def("B", vec![("A", true)]),
        ];
        let result = stratify(&defs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("negation cycle"));
    }

    #[test]
    fn positive_cycle_ok() {
        // A depends on B, B depends on A — fine (mutual recursion without negation)
        let defs = vec![
            make_def("A", vec![("B", false)]),
            make_def("B", vec![("A", false)]),
        ];
        let result = stratify(&defs);
        assert!(result.is_ok());
    }

    #[test]
    fn complex_valid_stratification() {
        // Base: no deps
        // Middle: depends on Base (positive)
        // Top: negates Middle
        let defs = vec![
            make_def("Base", vec![]),
            make_def("Middle", vec![("Base", false)]),
            make_def("Top", vec![("Middle", true)]),
        ];
        let strata = stratify(&defs).unwrap();
        let base_s = strata
            .iter()
            .find(|s| s.relations.contains(&"Base".to_string()))
            .unwrap()
            .order;
        let mid_s = strata
            .iter()
            .find(|s| s.relations.contains(&"Middle".to_string()))
            .unwrap()
            .order;
        let top_s = strata
            .iter()
            .find(|s| s.relations.contains(&"Top".to_string()))
            .unwrap()
            .order;
        assert!(base_s <= mid_s);
        assert!(mid_s < top_s);
    }
}
