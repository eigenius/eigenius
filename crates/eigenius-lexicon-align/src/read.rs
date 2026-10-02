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

//! Reading committed UMLS lexical entries **from the chain**, for the alignment emitters. The
//! committed resource is the truth: an emitter passes its properties through and changes only the
//! concept it denotes.

use eigenius_kernel::layer::Layer;
use eigenius_kernel::ontology::{Iri, Resource, Value};

/// The `num` argument of a `cat_n(umlscui:<CUI>, num)` category, or `None` if the category is not
/// that shape — which is how a **named individual** (`cat_np(umlssty:<TUI>, sg)`) is excluded: it is
/// an instance, not a class, and pointing it at a class would be a type error.
///
/// The number is READ from the category — the constructor of `lexicon:Num` it applies — rather than
/// matched against a list. A list (`num_any`, `mass`, `sg`, `pl`) missed D70's `name`, so every
/// named-condition entry read as "not a `cat_n`" and was left unaligned.
pub fn cat_n_num(cat: &Value, cui: &str, layer: &Layer) -> Option<String> {
    // Reads the value in EITHER shape. It matched only `Value::Json`, and a `lexicon:cat` is a
    // value resource since D85 §5 step 4 — so every category read as "not a `cat_n`" and every
    // entry was counted a named individual. The emitter reported 40 405 skips, a plausible
    // number, and wrote an empty layer; the kernel then panicked on the empty commit batch.
    let j = match cat {
        Value::Json(j) => j.clone(),
        Value::Embedded(r) => {
            eigenius_kernel::program::eigentt_type_mirror::ctor_view(r, layer).ok()?
        }
        _ => return None,
    };
    let s = j.to_string();
    if !s.contains("\"cat_n\"") {
        return None; // cat_np (named individual) or anything else — skip.
    }
    if !s.contains(&format!("urn:eigenius:umlscui:{cui}")) {
        return None; // the category does not index THIS concept — do not touch it.
    }
    num_ctor(&j)
}

/// The name of the `lexicon:Num` constructor applied anywhere in a category's constructor view.
fn num_ctor(j: &serde_json::Value) -> Option<String> {
    const NUM: &str = "urn:eigenius:lexicon:Num";
    match j {
        serde_json::Value::Object(o) => {
            let args = o.get("args").and_then(|a| a.as_array());
            if o.get("ctor").and_then(|c| c.as_str()) == Some("CtorApp") {
                if let Some(args) = args {
                    if args.first().and_then(|d| d.as_str()) == Some(NUM) {
                        return args.get(1).and_then(|n| n.as_str()).map(str::to_string);
                    }
                }
            }
            o.values().find_map(num_ctor)
        }
        serde_json::Value::Array(a) => a.iter().find_map(num_ctor),
        _ => None,
    }
}

/// A string-valued property, if present.
pub fn as_str(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// An IRI in the ESL form the layer header declares (`urn:eigenius:lexicon:umls` → `lexicon:umls`).
pub fn qname(iri: &str) -> String {
    match iri.strip_prefix("urn:eigenius:lexicon:") {
        Some(local) => format!("lexicon:{local}"),
        None => iri.to_string(),
    }
}

/// Every committed lexical entry of a UMLS concept: `e_<CUI>_<i><suffix>` for each suffix the
/// importer mints ([`eigenius_umls::convert::ENTRY_SUFFIXES`] — the plain entry, the `_mass` and the
/// D70 `_name` variants), for `i` up from 0. Indices can have gaps (an importer skips a surface), so
/// the probe stops after `gap` consecutive misses, or at `max_index`.
pub fn entries_of(
    head: &Layer,
    cui: &str,
    max_index: usize,
    gap: usize,
) -> Vec<(String, Resource)> {
    let mut out = Vec::new();
    let mut misses = 0usize;
    for i in 0..max_index {
        if misses > gap {
            break;
        }
        let mut found = false;
        for suffix in eigenius_umls::convert::ENTRY_SUFFIXES {
            let iri_s = format!("urn:eigenius:umlscui:e_{cui}_{i}{suffix}");
            let Ok(iri) = Iri::parse(&iri_s) else {
                continue;
            };
            if let Some(r) = head.resolve(&iri) {
                found = true;
                out.push((iri_s, (*r).clone()));
            }
        }
        misses = if found { 0 } else { misses + 1 };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::num_ctor;

    fn cat_n(num: &str) -> serde_json::Value {
        // `cat_n(umlscui:C1, <num>)` as the constructor view renders the D47 term.
        serde_json::json!({"ctor": "App", "args": [
            {"ctor": "App", "args": [
                {"ctor": "CtorApp", "args": ["urn:eigenius:lexicon:Cat", "cat_n"]},
                {"ctor": "ConstRef", "args": ["urn:eigenius:umlscui:C1", []]}
            ]},
            {"ctor": "CtorApp", "args": ["urn:eigenius:lexicon:Num", num]}
        ]})
    }

    /// Every number the lexicon declares is read, D70's `name` included — the list this replaced
    /// stopped at `pl`.
    #[test]
    fn the_number_is_read_from_the_category() {
        for num in ["num_any", "mass", "sg", "pl", "name"] {
            assert_eq!(num_ctor(&cat_n(num)).as_deref(), Some(num));
        }
    }

    #[test]
    fn a_constructor_of_another_inductive_is_not_a_number() {
        let v =
            serde_json::json!({"ctor": "CtorApp", "args": ["urn:eigenius:lexicon:Cat", "cat_n"]});
        assert_eq!(num_ctor(&v), None);
    }
}
