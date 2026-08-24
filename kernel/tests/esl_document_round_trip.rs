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

//! **DECLARATION-level round trip: `compile(print(d))` is `d`** (eigenius#217).
//!
//! `esl_round_trip.rs` pins TERMS — every D47 expression in the committed artifacts. It found the
//! surface defects eigenius#188 turned up, and it cannot see this one, because nothing in it ever
//! decompiles a DECLARATION. `eigenius decompile` emitted exactly one form, `resource X : C { … }`,
//! for every resource including a `core:InductiveType`; the output was valid ESL that recompiled
//! into a different thing, since it went through the resource path rather than `compile_data`.
//!
//! A printer that emits text which reparses into something else is precisely what a round-trip
//! test exists to catch. This file is the missing half of that suite.

use eigenius_kernel::esl;
use eigenius_kernel::ontology::eigon_json;
use serde_json::Value;

/// Compile ESL source and return its resources as Eigon-JSON, keyed by `@id`.
fn compile_to_json(src: &str) -> std::collections::BTreeMap<String, Value> {
    let resources = esl::compile(src).unwrap_or_else(|e| panic!("source must compile: {e:?}"));
    resources
        .iter()
        .filter_map(|r| {
            let v = eigon_json::serialize_resource(r);
            let id = v.get("@id")?.as_str()?.to_string();
            Some((id, v))
        })
        .collect()
}

/// Decompile one resource and recompile it, returning the resulting JSON.
fn round_trip(json: &Value) -> std::collections::BTreeMap<String, Value> {
    let printed = esl::print::print_document(&Value::Array(vec![json.clone()]))
        .unwrap_or_else(|e| panic!("decompile must succeed: {e:?}"));
    compile_to_json(&printed)
}

/// The declaration forms a chain actually carries, each in its own right.
///
/// `data` is the one eigenius#217 is about; the others are here so a future change that fixes
/// `data` by breaking a sibling is caught in the same file.
fn cases() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "non-parametric data",
            "urn:eigenius:ex:Nat",
            r#"
            namespace ex = "urn:eigenius:ex";
            data ex:Nat {
                zero,
                succ(ex:Nat),
            }
            "#,
        ),
        (
            "parametric data",
            "urn:eigenius:ex:List",
            r#"
            namespace ex = "urn:eigenius:ex";
            data ex:List(A : Set) {
                nil,
                cons(A, ex:List(A)),
            }
            "#,
        ),
        (
            "data at Prop with a typed ctor",
            "urn:eigenius:ex:And",
            r#"
            namespace ex = "urn:eigenius:ex";
            data ex:And(P : Prop, Q : Prop) : Prop {
                intro : forall (P : Prop, Q : Prop) => P -> Q -> ex:And(P, Q),
            }
            "#,
        ),
        (
            "indexed data",
            "urn:eigenius:ex:Eq",
            r#"
            namespace ex = "urn:eigenius:ex";
            data ex:Eq(A : Set) : A -> A -> Prop {
                refl : forall (A : Set, a : A) => ex:Eq(A, a, a),
            }
            "#,
        ),
        (
            "data with a description and named arguments (eigenius#221)",
            "urn:eigenius:ex:Tree",
            r#"
            namespace ex = "urn:eigenius:ex";
            data ex:Tree(A : Set) {
                description = "a binary tree, \"quoted\" for escaping";
                leaf,
                node(left : ex:Tree(A), value : A, right : ex:Tree(A)),
            }
            "#,
        ),
        (
            "embedded resource and opaque JSON as property values (eigenius#222)",
            "urn:eigenius:ex:reading",
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:ex";
            resource ex:reading : core:Class {
                ex:bound = { core:is_a = [ex:Interval]; ex:lo = 0.5; ex:hi = -2.0; };
                ex:solver_output = json({ "k_elim": 0.5, "path": [1, -2.0, true, null], "n": 3 });
            }
            "#,
        ),
        (
            "plain class (the control — this form already round-trips)",
            "urn:eigenius:ex:Dog",
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:ex";
            class ex:Dog {
                description = "a dog";
            }
            "#,
        ),
    ]
}

#[test]
fn every_declaration_form_round_trips_through_esl() {
    let mut failures = Vec::new();
    for (label, id, src) in cases() {
        let original = compile_to_json(src);
        let Some(before) = original.get(id) else {
            panic!("{label}: source did not produce `{id}` — fixture is wrong");
        };
        let after_doc = round_trip(before);
        match after_doc.get(id) {
            Some(after) if after == before => {}
            Some(after) => failures.push(format!(
                "{label} ({id}): recompiled to a DIFFERENT resource\n  before: {before}\n  after:  {after}"
            )),
            None => failures.push(format!(
                "{label} ({id}): recompiling the decompiled text produced no resource at that IRI \
                 (got {:?})",
                after_doc.keys().collect::<Vec<_>>()
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "decompile → recompile is not the identity:\n\n{}",
        failures.join("\n\n")
    );
}

/// **Every inductive on the shipped ontologies decompiles and recompiles to the same DECLARATION.**
///
/// The synthesised cases above pin the forms; this pins the actual declarations, which is what
/// eigenius#217 was about. Before the fix all five in `core` + `formulas` failed — not by
/// round-tripping to something else, but by failing to print at all: `core:ctors` holds embedded
/// `InductiveCtor` resources and `print_property_value` has no surface for those.
///
/// **The comparator is deliberately not byte identity, and the reason is a measured surface gap,
/// not a convenience.** Three kinds of difference are expected and none is a printer defect:
///
/// 1. **Compiler-added provenance.** Recompiling mints stable constructor `@id`s
///    (`core:Level:Zero`), adds `reflection:DeclaredResource` to `is_a`, and stamps
///    `reflection:declared_by`. The hand-authored JSON has none of these — it is under-specified
///    relative to compiler output, so the difference runs the other way from a loss.
///
/// `core:description` and `core:arg_name` were on this list until eigenius#221 added the surface
/// for both — `description = "…";` in the body and `succ(base : ex:Nat)` for a named argument. The
/// exclusions are gone and the test now asserts the stronger property, which is what closing a
/// language gap is supposed to buy.
#[test]
fn every_shipped_inductive_round_trips_through_esl() {
    const INDUCTIVE: &str = "urn:eigenius:core:InductiveType";
    /// Strip what ESL cannot express and what compiling legitimately adds.
    fn declaration_content(v: &Value) -> Value {
        let mut v = v.clone();
        fn scrub(v: &mut Value) {
            match v {
                Value::Object(o) => {
                    o.remove("@id");
                    o.remove("urn:eigenius:reflection:declared_by");
                    o.remove("urn:eigenius:core:is_a");
                    // The compiler writes explicit empty arrays where hand-authored JSON omits
                    // the key. Absent and empty mean the same thing for all three, so normalise.
                    for k in [
                        "urn:eigenius:core:type_args",
                        "urn:eigenius:core:type_params",
                        "urn:eigenius:core:indices",
                        "urn:eigenius:core:arg_types",
                    ] {
                        if o.get(k) == Some(&Value::Array(vec![])) {
                            o.remove(k);
                        }
                    }
                    for (_, x) in o.iter_mut() {
                        scrub(x);
                    }
                }
                Value::Array(a) => a.iter_mut().for_each(scrub),
                _ => {}
            }
        }
        scrub(&mut v);
        v
    }

    let mut seen = 0;
    let mut failures = Vec::new();

    for file in [
        "../ontologies/core/core-ontology.json",
        "../ontologies/formulas/formulas-ontology.json",
        "../ontologies/reflection/reflection-ontology.json",
    ] {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let resources = eigon_json::parse_document(&text)
            .unwrap_or_else(|e| panic!("{file} must parse: {e:?}"));
        for r in &resources {
            if !r.is_a().iter().any(|i| i.as_str() == INDUCTIVE) {
                continue;
            }
            seen += 1;
            let id = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
            let before = eigon_json::serialize_resource(r);
            let printed = match esl::print::print_document(&Value::Array(vec![before.clone()])) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(format!("{id}: does not decompile at all: {e:?}"));
                    continue;
                }
            };
            match esl::compile(&printed) {
                Ok(rs) => {
                    let after = rs
                        .iter()
                        .map(eigon_json::serialize_resource)
                        .find(|v| v.get("@id").and_then(Value::as_str) == Some(id.as_str()));
                    match after {
                        Some(a) if declaration_content(&a) == declaration_content(&before) => {}
                        Some(a) => failures.push(format!(
                            "{id}: recompiled to a DIFFERENT declaration\n  before: {}\n  after:  {}",
                            declaration_content(&before),
                            declaration_content(&a)
                        )),
                        None => failures.push(format!("{id}: recompiled without that IRI")),
                    }
                }
                Err(e) => failures.push(format!(
                    "{id}: decompiled text does not recompile: {e:?}\n--- source ---\n{printed}"
                )),
            }
        }
    }

    assert!(seen >= 5, "expected the shipped inductives; found {seen}");
    assert!(
        failures.is_empty(),
        "{} of {seen} shipped inductives do not round-trip:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// **Every shipped ontology DOCUMENT decompiles and recompiles.** (eigenius#222)
///
/// The per-resource tests above miss two whole classes of defect, because both are properties of
/// the emitted *document* rather than of any one resource:
///
/// - **the namespace alias.** `print_document` mints an alias per IRI prefix. `urn:eigenius:program`
///   yielded `program`, which is an ESL KEYWORD, so `program:Foo` never lexed as one `QualName`
///   token and the parser failed on the class-list colon. 79 resources, all in one file.
/// - **cross-resource references.** A resource compiled alone cannot see a `macro` declared beside
///   it; the document can.
///
/// Four documents still fail to PRINT, all for the same reason: an IRI whose local name is not a
/// legal ESL identifier (`let-program`, `identity-program`, `program-run`, `obo-foundry` — all
/// hyphenated). That is a language-level question about what a local name may be, tracked
/// separately; this test pins it as a known set rather than letting it drift.
#[test]
fn every_shipped_ontology_document_round_trips() {
    fn json_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                json_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "json") {
                out.push(p);
            }
        }
    }
    /// Documents whose IRIs ESL cannot spell. Shrinking this list is the point; growing it is a
    /// regression and must be argued for.
    const CANNOT_SPELL: &[&str] = &[
        "let-program.json",
        "simple-program.json",
        "notebook-ontology.json",
        "obo-meta-ontology.json",
    ];

    let mut files = Vec::new();
    json_files(std::path::Path::new("../ontologies"), &mut files);
    files.sort();
    assert!(
        files.len() > 20,
        "expected the shipped ontologies, found {}",
        files.len()
    );

    let mut ok = 0;
    let mut failures = Vec::new();
    for f in &files {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let Ok(resources) = eigon_json::parse_document(&text) else {
            continue;
        };
        let doc = Value::Array(
            resources
                .iter()
                .map(eigon_json::serialize_resource)
                .collect(),
        );
        let expected_unspellable = CANNOT_SPELL.contains(&name.as_str());
        match esl::print::print_document(&doc) {
            Err(e) => {
                if !expected_unspellable {
                    failures.push(format!("{name}: does not decompile: {}", e.message));
                }
            }
            Ok(src) => {
                assert!(
                    !expected_unspellable,
                    "{name} is on CANNOT_SPELL but now decompiles — remove it from the list"
                );
                match esl::compile(&src) {
                    Ok(_) => ok += 1,
                    Err(e) => failures.push(format!(
                        "{name}: decompiled text does not recompile: {:?}",
                        e.first().map(std::string::ToString::to_string)
                    )),
                }
            }
        }
    }
    assert!(
        ok > 25,
        "expected most documents to round-trip; only {ok} did"
    );
    assert!(
        failures.is_empty(),
        "{} document(s) fail:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}
