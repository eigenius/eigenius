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

//! P1.3 end-to-end: the Eigon ↔ R marshalling, exercised through a real R
//! worker (LocalServiceSpawner). An input `Resource` carrying typed value
//! arrays is dispatched; the R script decodes the columns with
//! `r_eigon_f64_array` / `r_eigon_str_array`, computes a base-R statistic
//! (no extra packages — runs anywhere R is present), and encodes an Eigon
//! a resource via the `r_eigon_begin/add_class/set_f64/finish`
//! builder. `run_script` parses that CBOR back into the `RunOutcome`
//! output. This proves the full wrapped-R recompute shape the P5 lme4
//! xenograft recompute uses — minus lme4, so it's green in this sandbox.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_r::RLanguageRuntime;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::service::LocalServiceSpawner;

fn cdylib_path() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let profile = exe.parent().and_then(|d| d.parent()).expect("profile dir");
    profile.join("libeigenius_r_worker.so")
}

fn driver_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eigenius-r-worker/r/EigeniusRWorker.R")
}

fn rscript_available() -> bool {
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

const X: &str = "urn:eigenius:test:x";
const G: &str = "urn:eigenius:test:g";
const MEAN: &str = "urn:eigenius:test:mean";
const PVAL: &str = "urn:eigenius:test:p";

/// The R script: decode two columns, compute a base-R group t-test +
/// overall mean, and encode a resource. (`as.character` so `g`
/// round-trips as strings; `t.test(x ~ factor(g))` needs no packages.)
const SCRIPT: &str = r#"
in0 <- eigenius_inputs[[1]]
x <- .Call("r_eigon_f64_array", in0, "urn:eigenius:test:x")
g <- .Call("r_eigon_str_array", in0, "urn:eigenius:test:g")
m <- mean(x)
p <- t.test(x ~ factor(g))$p.value
b <- .Call("r_eigon_begin", "urn:eigenius:test:result")
.Call("r_eigon_add_class", b, "urn:eigenius:core:Resource")
.Call("r_eigon_set_f64", b, "urn:eigenius:test:mean", m)
.Call("r_eigon_set_f64", b, "urn:eigenius:test:p", p)
if (p < 0.05) {
  .Call("r_eigon_set_proposition", b, "urn:eigenius:test:GroupsDiffer", c("x", "g"))
}
.Call("r_eigon_finish", b)
"#;

#[test]
fn eigon_r_marshalling_round_trip() {
    if !rscript_available() {
        eprintln!("skipping eigon_r_marshalling_round_trip: Rscript unavailable");
        return;
    }
    let cdylib = cdylib_path();
    assert!(cdylib.exists(), "cdylib not built at {}", cdylib.display());

    let depot = tempfile::tempdir().expect("depot");
    let spawner = Arc::new(LocalServiceSpawner::new(depot.path().to_path_buf()));
    let runtime = RLanguageRuntime::new(spawner, driver_path(), cdylib, depot.path().to_path_buf());

    // Input table: x = two clearly-separated groups a/b, so the t-test is
    // tiny-p and the mean is exact.
    let mut input = Resource::new(Iri::parse("urn:eigenius:test:table").unwrap());
    let xs = [1.0, 2.0, 3.0, 11.0, 12.0, 13.0];
    input.set(
        Iri::parse(X).unwrap(),
        Value::Array(xs.iter().map(|v| Value::Float(*v)).collect()),
    );
    input.set(
        Iri::parse(G).unwrap(),
        Value::Array(
            ["a", "a", "a", "b", "b", "b"]
                .iter()
                .map(|s| Value::String(s.to_string()))
                .collect(),
        ),
    );
    let env = Resource::new(Iri::parse("urn:eigenius:test:renv").unwrap());

    let script = {
        let mut r = Resource::new(Iri::parse("urn:eigenius:test:rscript").unwrap());
        r.set(
            Iri::parse("urn:eigenius:runtime:source").unwrap(),
            Value::String(SCRIPT.to_string()),
        );
        r
    };

    let outcome = runtime
        .run_script(&env, &script, &[input])
        .expect("marshalling round-trip dispatch succeeds");

    // The output is the parsed Eigon resource the script built.
    let get_f64 = |iri: &str| match outcome.output.get(&Iri::parse(iri).unwrap()) {
        Some(Value::Float(f)) => *f,
        other => panic!("property {iri} not a Float: {other:?}"),
    };
    // mean([1,2,3,11,12,13]) = 7.0 exactly — the f64 array decoded correctly.
    assert!(
        (get_f64(MEAN) - 7.0).abs() < 1e-9,
        "mean = {}",
        get_f64(MEAN)
    );
    // The group t-test is highly significant (groups a/b are far apart) —
    // proves the string column decoded + drove the model.
    assert!(get_f64(PVAL) < 1e-3, "p = {}", get_f64(PVAL));
    // The is_a the builder set is on the parsed resource.
    let is_a = outcome
        .output
        .get(&Iri::parse("urn:eigenius:core:is_a").unwrap());
    let has_class = matches!(is_a, Some(Value::Array(a))
        if a.iter().any(|v| matches!(v, Value::String(s) if s == "urn:eigenius:core:Resource")));
    // The class is incidental — this pins that `r_eigon_add_class` round-trips at
    // all. It named `reflection:DerivedResource` until P5 (2/n) removed that class from
    // the ontology, at which point the assertion held while the resource it described
    // would have been refused at commit.
    assert!(has_class, "output is_a missing core:Resource: {is_a:?}");

    // The proposition the script set (groups differ → p < 0.05) round-trips as the App-spine term
    // the reasoning institution consumes: App(App(ConstRef(GroupsDiffer), LitString(x)),
    // LitString(g)).
    //
    // It arrives in the D85 §6.1 shape — an inductive value is a RESOURCE whose `is_a` names the
    // constructor's class and whose arguments are named properties. It was a `Value::Json` tagged
    // dict, `{"ctor": …, "args": […]}`, until D85 replaced that encoding; the assertion here kept
    // expecting the old form and had been failing since, on `main` as well as on any branch.
    let prop = outcome
        .output
        .get(&Iri::parse("urn:eigenius:eigentt:proposition").unwrap());
    let Some(Value::Embedded(term)) = prop else {
        panic!("proposition is an embedded inductive value, got: {prop:?}")
    };

    /// The constructor class of an inductive value, short-named.
    fn ctor_of(r: &Resource) -> String {
        let is_a = r
            .get(&Iri::parse("urn:eigenius:core:is_a").unwrap())
            .expect("an inductive value names its constructor class");
        let Value::Array(v) = is_a else {
            panic!("is_a is an array, got {is_a:?}")
        };
        match v.first().expect("is_a is non-empty") {
            Value::String(s) => s.rsplit(':').next().expect("non-empty IRI").to_string(),
            other => panic!("is_a entry is a string, got {other:?}"),
        }
    }
    fn field<'a>(r: &'a Resource, name: &str) -> &'a Value {
        r.get(&Iri::parse(&format!("urn:eigenius:eigentt:{name}")).unwrap())
            .unwrap_or_else(|| panic!("inductive value carries `{name}`"))
    }
    fn embedded<'a>(v: &'a Value, what: &str) -> &'a Resource {
        match v {
            Value::Embedded(r) => r,
            other => panic!("{what} is an embedded value, got {other:?}"),
        }
    }
    fn lit_string(r: &Resource) -> &str {
        assert_eq!(ctor_of(r), "Term-LitString");
        match field(r, "Term-LitString-value") {
            Value::String(s) => s,
            other => panic!("LitString carries a string, got {other:?}"),
        }
    }

    // outer: App(<inner>, LitString("g"))
    assert_eq!(ctor_of(term), "Term-App", "the spine's outer node");
    assert_eq!(
        lit_string(embedded(field(term, "Term-App-arg"), "outer arg")),
        "g"
    );

    // inner: App(ConstRef(GroupsDiffer), LitString("x"))
    let inner = embedded(field(term, "Term-App-head"), "outer head");
    assert_eq!(ctor_of(inner), "Term-App", "the spine's inner node");
    assert_eq!(
        lit_string(embedded(field(inner, "Term-App-arg"), "inner arg")),
        "x"
    );

    let head = embedded(field(inner, "Term-App-head"), "inner head");
    assert_eq!(ctor_of(head), "Term-ConstRef");
    assert_eq!(
        field(head, "Term-ConstRef-iri"),
        &Value::String("urn:eigenius:test:GroupsDiffer".to_string()),
        "the spine's head names the declared predicate"
    );
}
