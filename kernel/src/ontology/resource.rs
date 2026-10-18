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

//! Resource and Value types for Eigenius.
//!
//! Everything in Eigenius is a Resource — classes, properties, data types,
//! formats, and instance data are all represented uniformly. A Resource
//! has an optional IRI identity and a set of property values.

use crate::ontology::iri::Iri;

/// A property value in the Eigon data model.
///
/// Values are typed according to the property definition's `data_type`.
/// The JSON-level representation determines which variant is used.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// UTF-8 string value.
    String(String),
    /// Signed integer in the 53-bit safe range.
    Integer(i64),
    /// 64-bit IEEE 754 floating-point number.
    Float(f64),
    /// Boolean true/false.
    Boolean(bool),
    /// Embedded resource (no `@id`).
    Embedded(Box<Resource>),
    /// Ordered array of values (resource_array or value_array).
    Array(Vec<Value>),
    /// Opaque JSON value, not validated by the ontology.
    Json(serde_json::Value),
}

impl Value {
    /// A value that REFERENCES a resource, from a parsed IRI.
    ///
    /// A reference is a string — the shape both codecs produce and both preserve. This
    /// constructor exists so that intent is visible where a resource is built in Rust with an
    /// `Iri` already in hand, which is the one thing the retired `Value::ResourceRef` variant
    /// was genuinely good for. It differs from that variant in the way that matters: it makes
    /// no claim a reader could depend on, because the result is an ordinary `Value::String`.
    pub fn iri(i: &Iri) -> Value {
        Value::String(i.as_str().to_string())
    }

    /// Returns the value as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the value as an integer, if it is one.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the value as a float, if it is one.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the value as a boolean, if it is one.
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// The value as a parsed IRI. `None` when it is not a string, or not a valid IRI.
    ///
    /// A reference is a string that parses; whether it is *meant* as a reference is the
    /// property's `data_type`, which this method does not consult and its callers do.
    ///
    /// There was an `as_iri_str` beside this one, whose doc said to prefer it over `as_str`
    /// when walking a resource-typed property. It read `ResourceRef` as well as `String`, and
    /// once that variant was retired (D85 §6.2) the two were the same match arm — a synonym
    /// whose documentation claimed a distinction that no longer existed. Its 47 callers read
    /// `as_str` now; a reader wanting the parsed form uses this.
    pub fn as_iri(&self) -> Option<Iri> {
        match self {
            Value::String(s) => Iri::parse(s).ok(),
            _ => None,
        }
    }

    /// Returns the value as an embedded resource, if it is one.
    pub fn as_embedded(&self) -> Option<&Resource> {
        match self {
            Value::Embedded(r) => Some(r),
            _ => None,
        }
    }

    /// Returns the value as an array, if it is one.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Extracts resource reference IRIs from an array value.
    /// The distinction between a string literal and a resource reference is made by the
    /// property's `data_type`, not at parse time, so this reads any IRI-parseable string.
    /// Non-string elements are silently skipped.
    pub fn as_iri_array(&self) -> Vec<Iri> {
        match self {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Iri::parse(s).ok(),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }
}

/// A resource in the Eigon data model.
///
/// Resources are the universal data unit. Everything — classes, properties,
/// data types, formats, and instance data — is a Resource. Top-level resources
/// have an `@id` (IRI identity). Embedded resources have no `@id` and exist
/// only as property values of their parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    /// IRI identity. `None` for embedded resources.
    id: Option<Iri>,
    /// Property values indexed by property IRI, in IRI order (canonical hashing depends on it).
    properties: PropertyMap,
}

impl Resource {
    /// Create a new top-level resource with the given IRI.
    pub fn new(id: Iri) -> Self {
        Self {
            id: Some(id),
            properties: PropertyMap::new(),
        }
    }

    /// Create a new embedded resource (no `@id`).
    pub fn new_embedded() -> Self {
        Self {
            id: None,
            properties: PropertyMap::new(),
        }
    }

    /// Returns the resource's IRI identity, or `None` for embedded resources.
    pub fn id(&self) -> Option<&Iri> {
        self.id.as_ref()
    }

    /// Promote an embedded resource to a top-level resource by
    /// assigning an `@id`, or rebrand an existing top-level resource.
    /// Pass `None` to demote a top-level resource to embedded.
    pub fn set_id(&mut self, id: Option<Iri>) {
        self.id = id;
    }

    /// Returns true if this is a top-level resource (has an `@id`).
    pub fn is_top_level(&self) -> bool {
        self.id.is_some()
    }

    /// Get a property value by property IRI.
    pub fn get(&self, property: &Iri) -> Option<&Value> {
        self.properties.get(property)
    }

    /// Set a property value.
    pub fn set(&mut self, property: Iri, value: Value) {
        self.properties.insert(property, value);
    }

    /// Remove a property value, returning it if present.
    pub fn remove(&mut self, property: &Iri) -> Option<Value> {
        self.properties.remove(property)
    }

    /// Returns true if the resource has the given property.
    pub fn has(&self, property: &Iri) -> bool {
        self.properties.contains_key(property)
    }

    /// Returns all properties, in IRI order.
    pub fn properties(&self) -> &PropertyMap {
        &self.properties
    }

    /// Returns the `is_a` class IRIs for this resource.
    ///
    /// Reads the `urn:eigenius:core:is_a` property and extracts
    /// all resource reference IRIs from the array value.
    pub fn is_a(&self) -> Vec<Iri> {
        let is_a_iri = match Iri::parse(crate::ontology::well_known::IS_A) {
            Ok(iri) => iri,
            Err(_) => return vec![],
        };
        match self.properties.get(&is_a_iri) {
            Some(value) => value.as_iri_array(),
            None => vec![],
        }
    }

    /// Returns true if this resource is an instance of the given class.
    pub fn is_instance_of(&self, class_iri: &Iri) -> bool {
        self.is_a().iter().any(|c| c == class_iri)
    }

    /// Returns an iterator over all property IRIs on this resource.
    pub fn property_iris(&self) -> impl Iterator<Item = &Iri> {
        self.properties.keys()
    }
}

/// A resource's properties, kept sorted by property IRI.
///
/// A sorted `Vec`, not a `BTreeMap`: iteration is in IRI order either way, which canonical hashing
/// and both codecs depend on, but a `BTreeMap` allocates its leaf node at its full eleven-slot
/// capacity, ~630 bytes for an embedded type-expression node holding two or three properties. A
/// parsed WordNet entry carries ~15 such nodes, and their leaves were ~70% of its ~13.5 KB
/// (docs/notes/reseed-oom-memory-investigation.md). Lookup scans forward up to
/// [`LINEAR_SCAN_MAX`] entries and binary-searches above it; an insert in order, which is how the
/// codecs and the ESL compiler build a resource, is a push.
#[derive(Clone, PartialEq, Default)]
pub struct PropertyMap {
    entries: Vec<(Iri, Value)>,
}

/// The most entries a lookup scans forward rather than binary-searching.
///
/// Measured by `benches/resources.rs`, group `search`, on keys sharing a 30-byte prefix: at 3
/// entries a hit takes 3.6 ns scanning against 7.8 ns searching, at 16 entries 12.9 against 16.7,
/// and the two are even at 24. A scan's comparisons come out `Less` until the last, which the
/// branch predictor learns; each step of a binary search goes either way. Lexicon nodes hold at
/// most 8 properties, 72% of them 3.
const LINEAR_SCAN_MAX: usize = 16;

impl PropertyMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Where `key` is (`Ok`) or would be inserted (`Err`).
    fn position(&self, key: &Iri) -> Result<usize, usize> {
        if self.entries.len() > LINEAR_SCAN_MAX {
            return self.entries.binary_search_by(|(k, _)| k.cmp(key));
        }
        for (i, (k, _)) in self.entries.iter().enumerate() {
            match k.cmp(key) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => return Ok(i),
                std::cmp::Ordering::Greater => return Err(i),
            }
        }
        Err(self.entries.len())
    }

    pub fn get(&self, key: &Iri) -> Option<&Value> {
        self.position(key).ok().map(|i| &self.entries[i].1)
    }

    /// Set `key` to `value`, returning the value it replaces.
    pub fn insert(&mut self, key: Iri, value: Value) -> Option<Value> {
        if self.entries.last().is_none_or(|(last, _)| *last < key) {
            self.entries.push((key, value));
            return None;
        }
        match self.position(&key) {
            Ok(i) => Some(std::mem::replace(&mut self.entries[i].1, value)),
            Err(i) => {
                self.entries.insert(i, (key, value));
                None
            }
        }
    }

    pub fn remove(&mut self, key: &Iri) -> Option<Value> {
        self.position(key).ok().map(|i| self.entries.remove(i).1)
    }

    pub fn contains_key(&self, key: &Iri) -> bool {
        self.position(key).is_ok()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The properties in IRI order.
    pub fn iter(&self) -> Iter<'_> {
        Iter(self.entries.iter())
    }

    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &Iri> + ExactSizeIterator + Clone {
        self.entries.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Value> + ExactSizeIterator + Clone {
        self.entries.iter().map(|(_, v)| v)
    }
}

/// Later entries win, as repeated inserts into a map would.
impl FromIterator<(Iri, Value)> for PropertyMap {
    fn from_iter<I: IntoIterator<Item = (Iri, Value)>>(iter: I) -> Self {
        let mut map = PropertyMap::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

/// Printed as a map, as the `BTreeMap` it replaced was.
impl std::fmt::Debug for PropertyMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Iterator over a [`PropertyMap`]'s `(property, value)` pairs in IRI order.
#[derive(Clone)]
pub struct Iter<'a>(std::slice::Iter<'a, (Iri, Value)>);

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a Iri, &'a Value);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k, v))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k, v))
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl<'a> IntoIterator for &'a PropertyMap {
    type Item = (&'a Iri, &'a Value);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn top_level_resource() {
        let r = Resource::new(iri("urn:eigenius:example:alice"));
        assert!(r.is_top_level());
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:alice");
    }

    #[test]
    fn embedded_resource() {
        let r = Resource::new_embedded();
        assert!(!r.is_top_level());
        assert!(r.id().is_none());
    }

    #[test]
    fn set_and_get_property() {
        let mut r = Resource::new(iri("urn:eigenius:example:alice"));
        let prop = iri("urn:eigenius:example:name");
        r.set(prop.clone(), Value::String("Alice".to_string()));
        assert_eq!(r.get(&prop).unwrap().as_str(), Some("Alice"));
    }

    #[test]
    fn is_a_returns_class_iris() {
        let mut r = Resource::new(iri("urn:eigenius:example:rex"));
        let is_a = iri("urn:eigenius:core:is_a");
        r.set(
            is_a,
            Value::Array(vec![
                Value::String("urn:eigenius:example:Dog".to_string()),
                Value::String("urn:eigenius:example:Pet".to_string()),
            ]),
        );
        let classes = r.is_a();
        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].as_str(), "urn:eigenius:example:Dog");
        assert_eq!(classes[1].as_str(), "urn:eigenius:example:Pet");
    }

    #[test]
    fn is_instance_of() {
        let mut r = Resource::new(iri("urn:eigenius:example:rex"));
        let is_a = iri("urn:eigenius:core:is_a");
        r.set(
            is_a,
            Value::Array(vec![Value::String("urn:eigenius:example:Dog".to_string())]),
        );
        assert!(r.is_instance_of(&iri("urn:eigenius:example:Dog")));
        assert!(!r.is_instance_of(&iri("urn:eigenius:example:Cat")));
    }

    #[test]
    fn value_accessors() {
        assert_eq!(Value::String("hi".into()).as_str(), Some("hi"));
        assert_eq!(Value::Integer(42).as_integer(), Some(42));
        assert_eq!(Value::Float(2.72).as_float(), Some(2.72));
        assert_eq!(Value::Boolean(true).as_boolean(), Some(true));
        assert!(Value::iri(&iri("urn:a:b")).as_iri().is_some());
        assert!(Value::String("hi".into()).as_integer().is_none());
    }

    #[test]
    fn properties_are_ordered() {
        let mut r = Resource::new(iri("urn:eigenius:example:test"));
        r.set(iri("urn:z:prop"), Value::String("z".into()));
        r.set(iri("urn:a:prop"), Value::String("a".into()));
        r.set(iri("urn:m:prop"), Value::String("m".into()));

        let keys: Vec<&str> = r.property_iris().map(|i| i.as_str()).collect();
        assert_eq!(keys, vec!["urn:a:prop", "urn:m:prop", "urn:z:prop"]);
    }

    #[test]
    fn a_property_map_keeps_iri_order_whatever_the_insertion_order() {
        let mut m = PropertyMap::new();
        for k in ["urn:m:p", "urn:z:p", "urn:a:p", "urn:q:p"] {
            assert_eq!(m.insert(iri(k), Value::String(k.into())), None);
        }
        let keys: Vec<&str> = m.keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["urn:a:p", "urn:m:p", "urn:q:p", "urn:z:p"]);
        assert_eq!(
            m.get(&iri("urn:q:p")).and_then(Value::as_str),
            Some("urn:q:p")
        );
        assert!(m.get(&iri("urn:b:p")).is_none());
    }

    #[test]
    fn a_property_map_replaces_and_removes_like_a_map() {
        let mut m = PropertyMap::new();
        m.insert(iri("urn:a:p"), Value::Integer(1));
        m.insert(iri("urn:b:p"), Value::Integer(2));
        assert_eq!(
            m.insert(iri("urn:a:p"), Value::Integer(3)),
            Some(Value::Integer(1))
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m.remove(&iri("urn:a:p")), Some(Value::Integer(3)));
        assert_eq!(m.remove(&iri("urn:a:p")), None);
        assert!(!m.contains_key(&iri("urn:a:p")) && m.contains_key(&iri("urn:b:p")));
    }

    #[test]
    fn a_property_map_finds_the_same_keys_scanning_or_searching() {
        let key = |i: usize| iri(&format!("urn:p:{i:03}"));
        for n in 1..=LINEAR_SCAN_MAX + 8 {
            let mut m = PropertyMap::new();
            // Even keys, inserted last first, so every insert lands at the front.
            for i in (0..n).rev() {
                assert_eq!(m.insert(key(2 * i), Value::Integer(i as i64)), None);
            }
            assert!(
                m.keys().zip(m.keys().skip(1)).all(|(a, b)| a < b),
                "n = {n}"
            );
            for i in 0..n {
                assert_eq!(
                    m.get(&key(2 * i)),
                    Some(&Value::Integer(i as i64)),
                    "n = {n}"
                );
                assert!(m.get(&key(2 * i + 1)).is_none(), "n = {n}");
            }
            assert_eq!(
                m.remove(&key(2 * (n / 2))),
                Some(Value::Integer((n / 2) as i64))
            );
            assert!(
                !m.contains_key(&key(2 * (n / 2))) && m.len() == n - 1,
                "n = {n}"
            );
        }
    }

    #[test]
    fn property_maps_with_the_same_content_are_equal_and_print_as_maps() {
        let a: PropertyMap = [("urn:b:p", 2), ("urn:a:p", 1)]
            .into_iter()
            .map(|(k, n)| (iri(k), Value::Integer(n)))
            .collect();
        let b: PropertyMap = [("urn:a:p", 9), ("urn:b:p", 2), ("urn:a:p", 1)]
            .into_iter()
            .map(|(k, n)| (iri(k), Value::Integer(n)))
            .collect();
        assert_eq!(a, b);
        let printed = format!("{a:?}");
        assert!(
            printed.starts_with('{') && printed.contains("Integer(1)"),
            "{printed}"
        );
    }
}
