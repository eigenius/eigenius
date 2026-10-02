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
    for n in ["num_any", "mass", "sg", "pl"] {
        if s.contains(&format!("\"{n}\"")) {
            return Some(n.to_string());
        }
    }
    None
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

/// Every committed lexical entry of a UMLS concept: `e_<CUI>_<i>` and its additive `_mass`
/// variant, for `i` up from 0. Indices can have gaps (an importer skips a surface), so the probe
/// stops after `gap` consecutive misses, or at `max_index`.
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
        for suffix in ["", "_mass"] {
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
