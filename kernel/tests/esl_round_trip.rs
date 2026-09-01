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

//! `compile(print(t))` is α-equal to `t`, over every D47 term in the committed demo artifacts.
//!
//! The comparator is [`alpha_canonicalize_proposition_json`] — the SAME normalisation the witness
//! index hashes to decide whether a chain-resident witness discharges a proposition. So a term that
//! passes here does not merely "look similar": it is the identical witness as far as the commit
//! gate is concerned.
//!
//! The corpus is the parser's own output (`claims-*.json`, produced by the DCG over English prose)
//! plus the hand-authored rule and certificate terms. Those exercise the forms that motivated the
//! surface work — `Sig` under `Fst` from every definite description, curried `App` spines,
//! `CtorApp` justification spines, and the synthesised `UnitVal` certificate slot.

use std::collections::BTreeMap;

use eigenius_kernel::esl;
use eigenius_kernel::esl::print::{is_d47_term, print_type_expr, print_value_term, Namespaces};
use eigenius_kernel::layer::Layer;
use eigenius_kernel::witness::alpha_canonicalize_proposition_json;
use serde_json::Value;

/// The demo's committed chain artifacts, relative to the kernel crate root (cargo's CWD for an
/// integration test). These are ESL — the demo loads them directly — so the test compiles each
/// one and round-trips the terms the compiler produced. That checks the committed source, not a
/// separate JSON copy of it.
const CORPUS: &[&str] = &[
    "../demo/prose-to-formulas-v2/claims-intact.esl",
    "../demo/prose-to-formulas-v2/claims-edited.esl",
    "../demo/prose-to-formulas-v2/inference.esl",
    // D66 replaced the generated shape rules and per-sentence bridges with transparent
    // definitions, so `rules.esl` and `bridges.esl` no longer exist. The definitions and the
    // quantified literature rule take their place in this corpus.
    "../demo/prose-to-formulas-v2/onco-typed.esl",
    "../demo/prose-to-formulas-v2/literature-rules.esl",
    // Repointed from `demo/prose-to-formulas/` to v2 on 2026-08-17: v1 was retired and its files
    // deleted (a sense-erased skeleton pin cannot break a sense-only tie, so it was
    // inventory-dependent — see that demo's README).
    //
    // COVERAGE NOTE — one artifact had no v2 counterpart and is GONE: `rule-general.esl`, whose term
    // nested THREE quantifiers over mixed domains (`forall v0:Set. forall v1:Set. forall
    // v2:lexicon:Entity. …`). What remains is shallower: `literature-rules.esl` has a single
    // `forall (m : Set)` and `inference.esl` two. So quantified terms are still round-tripped, but
    // DEEP nesting is not. If the printer/parser regresses on nested binders this corpus will no
    // longer catch it — worth a purpose-built fixture rather than relying on a demo artifact.
];

/// A D47 node: an object carrying `ctor` + `args`.
fn is_term(v: &Value) -> bool {
    v.get("ctor").and_then(Value::as_str).is_some() && v.get("args").is_some()
}

/// Every term-valued property in a document, keyed `<resource-id> :: <property>`.
fn terms_in(doc: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let rs = match doc {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    for r in rs {
        let Some(o) = r.as_object() else { continue };
        let id = o.get("@id").and_then(Value::as_str).unwrap_or("<anon>");
        for (k, v) in o {
            if is_term(v) {
                out.insert(format!("{id} :: {k}"), v.clone());
            }
        }
    }
    out
}

/// Print `term`, compile the result, and return the term the compiler produced.
///
/// The probe is a RESOURCE PROPERTY, not an `axiom` — that is where these terms actually live, and
/// it is the only context in which a bare inductive constructor (`app(...)`, `declared(...)`)
/// resolves. An axiom statement is a *type*; a certificate is a *term*, so it would not be
/// well-typed there regardless.
fn print_then_compile(term: &Value, prop_ns: &str, layer: &Layer) -> Result<Value, String> {
    let mut ns = Namespaces::new();
    // A `type_expr(...)` wrapper is what makes the slot a D47 TYPE; the value dialect goes in
    // bare. Printing one as the other would compile to a different encoding entirely.
    let body = if is_d47_term(term) {
        format!(
            "type_expr(\n{}\n    )",
            print_type_expr(term, &mut ns).map_err(|e| format!("print: {e}"))?
        )
    } else {
        print_value_term(term, &mut ns, prop_ns).map_err(|e| format!("print: {e}"))?
    };
    let src = format!(
        "{}namespace rt = \"urn:eigenius:roundtrip\";\n\nresource rt:probe : rt:Probe {{\n    \
         rt:term = {body};\n}}\n",
        ns.preamble()
    );
    // Against a layer, not bare: constructor short names resolve through the chain's ctor table
    // (`collect_ctors_from_layer`), which is where `justification:Certificate`'s ctors live. This is
    // also how decompiled ESL is meant to be reloaded.
    let resources = esl::compile_against_layer(&src, layer).map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        format!("recompile: {}\n--- source ---\n{src}", msgs.join("; "))
    })?;
    let iri = eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:roundtrip:term")
        .expect("well-formed IRI");
    for r in &resources {
        if let Some(eigenius_kernel::ontology::resource::Value::Json(j)) = r.get(&iri) {
            return Ok(j.clone());
        }
    }
    Err(format!(
        "no rt:term in recompiled output\n--- source ---\n{src}"
    ))
}

#[test]
fn every_demo_term_round_trips_through_esl() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let layer = ctx.head();
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for path in CORPUS {
        let Ok(text) = std::fs::read_to_string(path) else {
            panic!("corpus file missing: {path} (run from the kernel crate root)");
        };
        let resources = esl::compile_against_layer(&text, layer)
            .unwrap_or_else(|e| panic!("{path} does not compile: {e:?}"));
        let doc = Value::Array(
            resources
                .iter()
                .map(eigenius_kernel::ontology::eigon_json::serialize_resource)
                .collect(),
        );
        for (label, term) in terms_in(&doc) {
            checked += 1;
            // The inductive a property's values inhabit is declared in the same ontology as the
            // property, so the property IRI minus its local name is the ctor namespace.
            let prop_ns = label
                .rsplit_once(" :: ")
                .and_then(|(_, p)| p.rsplit_once(':'))
                .map(|(ns, _)| ns.to_string())
                .unwrap_or_default();
            match print_then_compile(&term, &prop_ns, layer) {
                Err(e) => failures.push(format!("{path}\n  {label}\n  {e}")),
                Ok(back) => {
                    let a = alpha_canonicalize_proposition_json(&term);
                    let b = alpha_canonicalize_proposition_json(&back);
                    if a != b {
                        failures.push(format!(
                            "{path}\n  {label}\n  NOT alpha-equal after round trip\n  \
                             original: {a}\n  reparsed: {b}"
                        ));
                    }
                }
            }
        }
    }

    assert!(checked > 0, "corpus produced no terms — the fixtures moved");
    assert!(
        failures.is_empty(),
        "{}/{} terms failed to round-trip:\n\n{}",
        failures.len(),
        checked,
        failures.join("\n\n")
    );
    eprintln!("round-tripped {checked} D47 terms from the demo corpus");
}

/// eigenius#142 — a boolean literal survives ESL source → D47 term → printed ESL → D47 term.
///
/// The `LitBool` ctor was added to `eigentt:Term` for `program:Literal` booleans; without the
/// matching `true` / `false` surface in `parse_type_expr` the printer would emit source that does
/// not reparse, which is exactly what `print_then_compile` catches.
#[test]
fn boolean_literal_round_trips_through_esl() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let layer = ctx.head();

    for (literal, json) in [("true", true), ("false", false)] {
        let src = format!(
            "namespace rt = \"urn:eigenius:roundtrip\";\n\n\
             resource rt:probe : rt:Probe {{\n    rt:term = type_expr({literal});\n}}\n"
        );
        let resources = esl::compile_against_layer(&src, layer)
            .unwrap_or_else(|e| panic!("{literal} does not compile: {e:?}"));
        let iri = eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:roundtrip:term")
            .expect("well-formed IRI");
        let term = resources
            .iter()
            .find_map(|r| match r.get(&iri) {
                Some(eigenius_kernel::ontology::resource::Value::Json(j)) => Some(j.clone()),
                _ => None,
            })
            .expect("rt:term is a D47 term");

        assert_eq!(
            term,
            serde_json::json!({"ctor": "LitBool", "args": [json]}),
            "`{literal}` in a type position must encode as the LitBool ctor"
        );
        assert!(is_d47_term(&term), "LitBool must classify as a D47 ctor");
        assert_eq!(
            print_type_expr(&term, &mut Namespaces::new()).expect("prints"),
            literal
        );

        let back = print_then_compile(&term, "rt", layer).expect("reparses");
        assert_eq!(
            alpha_canonicalize_proposition_json(&term),
            alpha_canonicalize_proposition_json(&back),
            "`{literal}` changed across the round trip"
        );
    }
}

/// The printer must REFUSE a term it has no surface for rather than emit something lossy — a
/// decompiler that drops a subterm yields source compiling to a different chain object.
#[test]
fn refuses_a_ctor_with_no_esl_surface() {
    let pair = serde_json::json!({"ctor": "Pair", "args": [
        {"ctor": "UnitVal", "args": []},
        {"ctor": "UnitVal", "args": []},
    ]});
    let err = print_type_expr(&pair, &mut Namespaces::new()).expect_err("Pair has no surface");
    assert!(err.message.contains("no ESL surface"), "{err}");
}

/// Binder names the DCG generates (`G#0`) are not legal identifiers; renaming them must not
/// capture a free occurrence that happens to carry the replacement name.
#[test]
fn renamed_binders_do_not_capture() {
    // exists G#0 : Set => x0(G#0)   — `x0` is free, and is the printer's first choice of name.
    let term = serde_json::json!({"ctor": "Sig", "args": [
        "G#0",
        {"ctor": "Sort", "args": [level_tree(1)]},
        {"ctor": "App", "args": [{"ctor": "Var", "args": ["x0"]}, {"ctor": "Var", "args": ["G#0"]}]},
    ]});
    let printed = print_type_expr(&term, &mut Namespaces::new()).expect("prints");
    assert!(
        !printed.starts_with("exists x0 "),
        "renamed the binder onto a free name already in the term: {printed}"
    );
}

/// `Layout::Pretty` may only change whitespace. Every corpus term is printed both ways, and both
/// renderings must compile to the same term — otherwise the readable form on disk would commit
/// something other than what the flat form does.
#[test]
fn pretty_layout_changes_only_whitespace() {
    use eigenius_kernel::esl::print::{print_type_expr_with, Layout};

    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let layer = ctx.head();
    let mut checked = 0usize;
    let mut broke_a_line = 0usize;

    for path in CORPUS {
        let text = std::fs::read_to_string(path).expect("corpus file");
        let resources = esl::compile_against_layer(&text, layer).expect("corpus compiles");
        let doc = Value::Array(
            resources
                .iter()
                .map(eigenius_kernel::ontology::eigon_json::serialize_resource)
                .collect(),
        );
        for (label, term) in terms_in(&doc) {
            if !is_d47_term(&term) {
                continue;
            }
            let flat = print_type_expr(&term, &mut Namespaces::new()).expect("flat");
            let mut pns = Namespaces::new();
            let pretty = print_type_expr_with(&term, &mut pns, Layout::Pretty, 0).expect("pretty");
            if pretty.contains('\n') {
                broke_a_line += 1;
            }
            // Whitespace-insensitive equality is not enough — a break could land inside a token.
            // Compile both and compare the terms.
            let a = print_then_compile(&term, "", layer).expect("flat recompiles");
            let b = wrap_and_compile(&pretty, &pns, layer)
                .unwrap_or_else(|e| panic!("{label}: pretty form does not compile: {e}"));
            assert_eq!(
                alpha_canonicalize_proposition_json(&a),
                alpha_canonicalize_proposition_json(&b),
                "{label}: pretty and flat compile to different terms"
            );
            checked += 1;
            assert_ne!(flat, "", "{label}: empty rendering");
        }
    }
    assert!(checked > 0, "no D47 terms in the corpus");
    assert!(
        broke_a_line > 0,
        "no term was long enough to break — the pretty path went untested"
    );
    eprintln!("{checked} terms print identically flat and pretty ({broke_a_line} broke a line)");
}

/// `Type n` is the only undelimited multi-token form the printer emits, so it is the only one a
/// surrounding construct could split. Every position that could do so is checked here, at levels
/// on both sides of the `Prop` / `Set` / `Type n` spelling boundary.
///
/// **Regression.** The printer emitted `Type(1)`, which the parser rejects outright (`expected
/// non-negative integer level after `Type`, found LParen`) — so no term carrying a universe above
/// `Set` could be decompiled into source that reparses. `Prop` (`Sort 0`) and `Set` (`Sort 1`) are
/// spelled as single tokens and were unaffected, which is exactly why a corpus that only uses
/// `Succ^n(Zero)` as the encoder emits it (eigenius#188).
fn level_tree(n: u64) -> serde_json::Value {
    let mut v = serde_json::json!({"ctor": "Zero", "args": []});
    for _ in 0..n {
        v = serde_json::json!({"ctor": "Succ", "args": [v]});
    }
    v
}

/// those two did not catch it.
#[test]
fn sorts_round_trip_in_every_position() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let layer = ctx.head();
    let mut checked = 0usize;

    // 0 → `Prop`, 1 → `Set`, 2 and up → `Type n`. Both spellings, so the test would still hold if
    // the boundary moved.
    for level in [0u64, 1, 2, 7] {
        // eigenius#188: `Sort`'s argument is an `eigentt:Level` tree — `Set` is `Succ(Zero)`,
        // not `1`. Built here the way the encoder builds it, so the comparison below is against
        // the encoding a fresh compile produces. There is no numeral form to test: retyping the
        // ctor moved the manifest, and the reseed that forced re-encodes every term from source.
        let s = serde_json::json!({"ctor": "Sort", "args": [level_tree(level)]});
        let cases = [
            ("bare", s.clone()),
            (
                "arrow domain",
                serde_json::json!({"ctor": "Pi", "args": ["", s.clone(), s.clone()]}),
            ),
            (
                "App argument",
                serde_json::json!({"ctor": "App", "args": [
                    {"ctor": "ConstRef", "args": ["urn:eigenius:core:string"]},
                    s.clone(),
                ]}),
            ),
            (
                "binder domain",
                serde_json::json!({"ctor": "Sig", "args": [
                    "x", s.clone(), {"ctor": "Var", "args": ["x"]},
                ]}),
            ),
            (
                "under a projection",
                serde_json::json!({"ctor": "Fst", "args": [
                    {"ctor": "Sig", "args": ["x", s.clone(), {"ctor": "Var", "args": ["x"]}]},
                ]}),
            ),
        ];
        for (position, term) in cases {
            let label = format!("Sort {level} in {position}");
            let mut ns = Namespaces::new();
            let printed = print_type_expr(&term, &mut ns).expect("prints");
            let back = wrap_and_compile(&printed, &ns, layer)
                .unwrap_or_else(|e| panic!("{label}: printed `{printed}` does not compile: {e}"));
            assert_eq!(
                alpha_canonicalize_proposition_json(&term),
                alpha_canonicalize_proposition_json(&back),
                "{label}: printed `{printed}`, which compiles to a different term"
            );
            checked += 1;
        }
    }
    eprintln!("{checked} sort placements round-trip");
}

/// Compile an already-printed type-expression body in a resource-property slot.
fn wrap_and_compile(body: &str, ns: &Namespaces, layer: &Layer) -> Result<Value, String> {
    // The preamble comes from the same `Namespaces` the body was printed with — the printer
    // records exactly the aliases it emitted, so there is nothing to keep in sync by hand.
    let src = format!(
        "{}namespace rt = \"urn:eigenius:roundtrip\";\n\nresource rt:probe : rt:Probe {{\n    \
         rt:term = type_expr(\n{body}\n    );\n}}\n",
        ns.preamble()
    );
    let resources = esl::compile_against_layer(&src, layer).map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        format!("{}\n--- source ---\n{src}", msgs.join("; "))
    })?;
    let iri = eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:roundtrip:term")
        .expect("well-formed IRI");
    for r in &resources {
        if let Some(eigenius_kernel::ontology::resource::Value::Json(j)) = r.get(&iri) {
            return Ok(j.clone());
        }
    }
    Err("no rt:term".into())
}

/// **Polymorphic levels round-trip through ESL** (eigenius#188, N3 §3 as revised).
///
/// This test asserted the OPPOSITE until the surface syntax landed: that the printer refuses a
/// polymorphic level, recorded as a documented limitation. It was evidence the design was
/// incomplete — without syntax nothing can author a `Level::Param`, so `uparams` would have had
/// nothing to generalise and universe polymorphism would have been unreachable from the language.
///
/// The forms are Lean 4's: `Sort u`, `Sort (max u v)`, `Sort (imax u v)`, `Sort (u + 1)`.
#[test]
fn polymorphic_levels_round_trip_through_esl() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let layer = ctx.head();
    let param = |n: &str| serde_json::json!({"ctor": "Param", "args": [n]});
    let cases = [
        ("Sort u", param("u")),
        (
            "Sort (max u v)",
            serde_json::json!({"ctor": "Max", "args": [param("u"), param("v")]}),
        ),
        (
            "Sort (imax u v)",
            serde_json::json!({"ctor": "IMax", "args": [param("u"), param("v")]}),
        ),
        (
            "Sort (u + 2)",
            serde_json::json!({"ctor": "Succ", "args": [
                {"ctor": "Succ", "args": [param("u")]}
            ]}),
        ),
    ];
    for (expected_source, level) in cases {
        let term = serde_json::json!({"ctor": "Sort", "args": [level]});
        let mut ns = Namespaces::new();
        let printed = print_type_expr(&term, &mut ns).expect("a polymorphic level prints");
        assert_eq!(printed, expected_source, "surface spelling");
        let back = wrap_and_compile(&printed, &ns, layer)
            .unwrap_or_else(|e| panic!("`{printed}` must recompile: {e}"));
        assert_eq!(back, term, "`{printed}` must round-trip to the same level");
    }
}

/// The abbreviations stay abbreviations: a numeral level prints as `Prop` / `Set` / `Type k`,
/// never as the general `Sort n` form, so the 942 monomorphic uses in the tree print as written.
#[test]
fn numeral_levels_still_print_as_the_abbreviations() {
    for (n, expected) in [(0u64, "Prop"), (1, "Set"), (2, "Type 1"), (4, "Type 3")] {
        let term = serde_json::json!({"ctor": "Sort", "args": [level_tree(n)]});
        let printed = print_type_expr(&term, &mut Namespaces::new()).expect("prints");
        assert_eq!(printed, expected);
    }
}
