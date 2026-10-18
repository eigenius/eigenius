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

//! Class equivalence (D99 §11): the groups of classes that `core:EquivalentClasses` resources
//! declare equivalent, as the subsumption walk reads them.
//!
//! An equivalence is an INSTANCE naming classes already declared, so it is found through the triple
//! index — the `is_a` entry for `core:EquivalentClasses` — once per layer, and held in memory for
//! [`Layer::is_subclass_of`](super::Layer::is_subclass_of). Two equivalences that share a class are
//! one group: equivalence is transitive.
//!
//! It is consulted for SUBSUMPTION only. A class's definition — its record type, the properties it
//! inherits — comes from `subclass_of` alone and never reads this; that is what keeps an
//! equivalence from being the mutual-`subclass_of` cycle declaration ordering refuses.

use std::collections::{BTreeMap, BTreeSet};

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::well_known as wk;

/// The equivalence groups visible from one layer.
#[derive(Debug, Default)]
pub struct ClassEquivalences {
    group_of: BTreeMap<Iri, usize>,
    groups: Vec<BTreeSet<Iri>>,
}

impl ClassEquivalences {
    /// Read every `core:EquivalentClasses` on `head`'s chain and merge them into groups.
    pub fn build(head: &Layer) -> Self {
        let is_a = wk::iri(wk::IS_A);
        let class = wk::iri(wk::EQUIVALENT_CLASSES);
        let members_prop = wk::iri(wk::EQUIVALENT_CLASSES_MEMBERS);
        let mut out = ClassEquivalences::default();
        for subject in super::index::scan_chain(head, &is_a, &class) {
            let Some(r) = head.resolve(&subject) else {
                continue;
            };
            if !r.is_a().iter().any(|c| c == &class) {
                continue; // the index entry is stale against a redefinition
            }
            if let Some(v) = r.get(&members_prop) {
                out.merge(v.as_iri_array());
            }
        }
        out
    }

    /// Put `members` in one group, merging every group any of them is already in.
    fn merge(&mut self, members: Vec<Iri>) {
        if members.len() < 2 {
            return;
        }
        let mut joined: BTreeSet<Iri> = members.into_iter().collect();
        let touched: BTreeSet<usize> = joined
            .iter()
            .filter_map(|c| self.group_of.get(c).copied())
            .collect();
        for g in &touched {
            joined.extend(std::mem::take(&mut self.groups[*g]));
        }
        let id = self.groups.len();
        for c in &joined {
            self.group_of.insert(c.clone(), id);
        }
        self.groups.push(joined);
    }

    /// The classes declared equivalent to `class`, itself excluded. Empty for a class in no group.
    pub fn equivalents<'a>(&'a self, class: &'a Iri) -> impl Iterator<Item = &'a Iri> + 'a {
        self.group_of
            .get(class)
            .into_iter()
            .flat_map(move |g| self.groups[*g].iter())
            .filter(move |c| *c != class)
    }

    pub fn is_empty(&self) -> bool {
        self.group_of.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn overlapping_equivalences_are_one_group() {
        let mut e = ClassEquivalences::default();
        e.merge(vec![iri("urn:x:a"), iri("urn:x:b")]);
        e.merge(vec![iri("urn:x:c"), iri("urn:x:d")]);
        e.merge(vec![iri("urn:x:b"), iri("urn:x:c")]);
        let a = iri("urn:x:a");
        let of_a: BTreeSet<Iri> = e.equivalents(&a).cloned().collect();
        assert_eq!(
            of_a,
            BTreeSet::from([iri("urn:x:b"), iri("urn:x:c"), iri("urn:x:d")])
        );
        assert_eq!(e.equivalents(&iri("urn:x:z")).count(), 0);
    }
}
