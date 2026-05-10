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

//! D32 §6 cross-institution probe.
//!
//! Demonstrates the load-bearing claim of D32: a chain-side
//! `formulas:FormulaTerm` value can be handed to *any* numerical
//! institution without transformation. The Symbolics handler decodes
//! it for `Symbolics.simplify`; the IntervalArithmetic handler
//! decodes the *same shape* for interval extension. The comorphism
//! between the two is the identity function on `FormulaTerm`.
//!
//! Concrete dispatch:
//!
//! 1. Build a chain with the IntervalArithmetic ontology + the
//!    Symbolics ontology committed (both share the embedded
//!    `formulas:` layer the kernel bootstraps).
//! 2. Generate a Julia mirror covering `BoundedBy` + `SymbolicExpression`
//!    (the closure pulls `FormulaTerm` and the operator catalog
//!    transitively per D32 §3.6).
//! 3. Bake the EigeniusIntervals handler package — extended at
//!    Phase 19d.1 with `compute_bounds(expr, domain)` that
//!    interval-extends a FormulaTerm without going through any
//!    Symbolics-specific path.
//! 4. Dispatch `compute_bounds(SymbolicExpression(sin(x) + 0.5),
//!    BoundedBy(_, 0.0, π/2))` and verify the returned interval
//!    bounds the function's range.
//!
//! Skipped on hosts without buildah / Docker / a pullable Julia
//! base image — same gating as the substrate-side capstone.

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_julia::JuliaLanguageRuntime;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const INTERVALS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-ontology.eigon.json"
);
const SYMBOLICS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
);
// Phase 19f.1: symbolics ontology now references jump:VariableBound and
// jump:Constraint via SymbolicsToJuMPInput, so the JuMP ontology must
// be in the chain pool too.
const JUMP_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");
const INTERVALS_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/intervals/EigeniusIntervals/Project.toml");
const INTERVALS_HANDLER_SOURCE_JL: &str = include_str!(
    "../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl"
);
const FORMULAS_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/formulas/formulas-ontology.json");

const BOUNDED_BY_CLASS_IRI: &str = "urn:eigenius:intervals:BoundedBy";
const SYMBOLIC_EXPRESSION_CLASS_IRI: &str = "urn:eigenius:symbolics:SymbolicExpression";
const ENV_IRI: &str = "urn:eigenius:test:cross:env";
const SIGNATURE_IRI: &str = "urn:eigenius:test:cross:signatures:compute_bounds";
const HANDLER_METHOD_NAME: &str = "compute_bounds";
const HANDLER_PACKAGE_NAME: &str = "EigeniusIntervals";
const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

// ─── Chain fixture ──────────────────────────────────────────────────────

/// In-memory chain that pools resources from three ontologies:
/// intervals (BoundedBy), symbolics (SymbolicExpression), and the
/// formulas: layer (FormulaTerm + operator catalog). The mirror
/// generator's closure walker resolves transitive references through
/// this single accessor.
struct CrossChain {
    resources: HashMap<Iri, Resource>,
}

impl CrossChain {
    fn new() -> Self {
        let mut resources = HashMap::new();
        for json in [
            INTERVALS_ONTOLOGY_JSON,
            SYMBOLICS_ONTOLOGY_JSON,
            JUMP_ONTOLOGY_JSON,
            FORMULAS_ONTOLOGY_JSON,
        ] {
            for r in eigon_json::parse_document(json).expect("ontology must parse") {
                if let Some(id) = r.id() {
                    resources.insert(id.clone(), r);
                }
            }
        }
        Self { resources }
    }
}

impl ChainAccessor for CrossChain {
    fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
        self.resources.get(target).cloned()
    }
    fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
        true
    }
    fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
        true
    }
}

// ─── Mirror + handler-package builders ──────────────────────────────────

fn build_cross_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = CrossChain::new();
    let layer = iri("urn:eigenius:test:cross:layer");
    let seed = vec![
        iri(BOUNDED_BY_CLASS_IRI),
        iri(SYMBOLIC_EXPRESSION_CLASS_IRI),
    ];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("cross mirror generation");
    mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"))
}

/// Build the EigeniusIntervals handler-package Resource. Same shape
/// the IntervalArithmetic e2e test uses, but the handler's source
/// now carries `compute_bounds` alongside `validate_bounded_by`
/// (Phase 19d.1).
fn build_handler_package() -> Resource {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }
    let mut r = Resource::new(iri(
        "urn:eigenius:test:cross:handler-package:EigeniusIntervals",
    ));
    r.set(
        iri("urn:eigenius:runtime:package_name"),
        Value::String(HANDLER_PACKAGE_NAME.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:manifest"),
        Value::String(INTERVALS_HANDLER_PROJECT_TOML.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:source_tree"),
        Value::Json(serde_json::json!([{
            "path": "src/EigeniusIntervals.jl",
            "content_base64": encode(INTERVALS_HANDLER_SOURCE_JL.as_bytes()),
        }])),
    );
    r
}

// ─── Input CBOR builders ────────────────────────────────────────────────

/// `sin(x) + 0.5` as a chain-shaped FormulaTerm tree.
fn sin_x_plus_half_term() -> serde_json::Value {
    serde_json::json!({
        "ctor": "App",
        "args": [
            {
                "ctor": "App",
                "args": [
                    {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                    {
                        "ctor": "App",
                        "args": [
                            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:sin"]},
                            {"ctor": "Var", "args": ["x"]}
                        ]
                    }
                ]
            },
            {"ctor": "LitFloat", "args": [0.5]}
        ]
    })
}

/// Build a `SymbolicExpression(term=sin(x)+0.5)` Eigon-CBOR payload
/// the worker decodes via the generated mirror.
fn build_symbolic_expression_cbor() -> Vec<u8> {
    let mut r = Resource::new(iri("urn:eigenius:test:cross:expr"));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(SYMBOLIC_EXPRESSION_CLASS_IRI))]),
    );
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("sin_x_plus_half".into()),
    );
    r.set(
        iri("urn:eigenius:symbolics:term"),
        Value::Json(sin_x_plus_half_term()),
    );
    eigon_cbor::serialize_resource(&r)
}

/// Build a `BoundedBy(value, lower, upper)` Eigon-CBOR payload —
/// repurposed here as the domain for `compute_bounds`. The worker
/// decodes it via the same `decode_BoundedBy` codec the
/// IntervalArithmetic AutoOnLoad gate uses.
fn build_domain_cbor(value: f64, lower: f64, upper: f64) -> Vec<u8> {
    let mut r = Resource::new(iri("urn:eigenius:test:cross:domain"));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(BOUNDED_BY_CLASS_IRI))]),
    );
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("domain".into()),
    );
    r.set(iri("urn:eigenius:intervals:value"), Value::Float(value));
    r.set(iri("urn:eigenius:intervals:lower"), Value::Float(lower));
    r.set(iri("urn:eigenius:intervals:upper"), Value::Float(upper));
    eigon_cbor::serialize_resource(&r)
}

fn build_setup_argument(source: &str) -> Vec<u8> {
    let mut arg = Resource::new_embedded();
    arg.set(
        iri("urn:eigenius:runtime:language"),
        Value::String("julia".to_string()),
    );
    arg.set(
        iri("urn:eigenius:runtime:source"),
        Value::String(source.to_string()),
    );
    eigon_cbor::serialize_resource(&arg)
}

// ─── Environment / skip gates ───────────────────────────────────────────

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-cross-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn julia_project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("julia")
        .join("runtime-worker")
        .canonicalize()
        .expect("julia/runtime-worker/ must exist relative to eigenius-julia's Cargo.toml")
}

fn is_docker_available() -> bool {
    Path::new("/var/run/docker.sock").exists()
}

fn ensure_base_image_pinned() -> Result<String, String> {
    let pull = std::process::Command::new("docker")
        .args(["pull", "--quiet", BASE_IMAGE_TAG])
        .output()
        .map_err(|e| format!("`docker pull` failed: {e}"))?;
    if !pull.status.success() {
        return Err(format!(
            "docker pull {BASE_IMAGE_TAG} exited {}: {}",
            pull.status,
            String::from_utf8_lossy(&pull.stderr)
        ));
    }
    let inspect = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            BASE_IMAGE_TAG,
        ])
        .output()
        .map_err(|e| format!("`docker image inspect` failed: {e}"))?;
    if !inspect.status.success() {
        return Err(format!(
            "docker image inspect failed: {}",
            String::from_utf8_lossy(&inspect.stderr)
        ));
    }
    let pinned = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
    if !pinned.contains('@') {
        return Err(format!(
            "expected RepoDigests entry like `julia@sha256:...`, got `{pinned}`"
        ));
    }
    Ok(if pinned.contains('/') {
        pinned
    } else {
        format!("docker.io/library/{pinned}")
    })
}

fn skip_unless_full_environment() -> Option<String> {
    if !is_docker_available() {
        return Some("Docker socket unavailable".into());
    }
    if !eigenius_runtime_substrate::is_buildah_available() {
        return Some("buildah unavailable".into());
    }
    None
}

// ─── The probe ──────────────────────────────────────────────────────────

#[ignore = "heavy E2E: cross-institution Julia env image build."]
#[test]
fn cross_institution_typed_transfer_via_formula_term() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping cross-institution probe: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot("probe");
    let spawner = match eigenius_runtime_substrate::spawner::service::DockerServiceSpawner::new(
        eigenius_runtime_substrate::spawner::DockerSpawnerConfig::new(depot.clone()),
    ) {
        Ok(s) => std::sync::Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };

    let project_dir = julia_project_dir();
    let runtime = std::sync::Arc::new(JuliaLanguageRuntime::new(
        project_dir,
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    let mirror = build_cross_mirror();
    let handler_pkg = build_handler_package();
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest =
        match runtime_for_dispatch.build_environment_image(&env, &[handler_pkg], Some(&mirror)) {
            Ok(d) => d,
            Err(e) => {
                let _ = runtime.drain();
                let _ = std::fs::remove_dir_all(&depot);
                panic!("build_environment_image: {e}");
            }
        };

    let mut dispatcher = eigenius_runtime_substrate::facade::SubstrateDispatcher::new();
    dispatcher
        .register_language_runtime(runtime_for_dispatch)
        .expect("register julia runtime");

    // Bring the handler module into Main so `Core.eval(Main, fn_symbol)`
    // resolves `compute_bounds`. Same pattern as the existing
    // intervals e2e test.
    let setup = "begin; using EigeniusIntervals; nothing; end";
    if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &build_setup_argument(setup)) {
        let _ = runtime.drain();
        let _ = std::fs::remove_dir_all(&depot);
        panic!("setup script (using EigeniusIntervals) failed: {e:?}");
    }

    // Domain: `[0, π/2]`. Over this, `sin(x) + 0.5` ranges from 0.5
    // to 1.5 exactly. Interval arithmetic gives a slightly wider
    // bound (rounding); we assert containment, not point-equality.
    let inputs = vec![
        build_symbolic_expression_cbor(),
        build_domain_cbor(0.0, 0.0, std::f64::consts::FRAC_PI_2),
    ];
    let outcome = match dispatcher.dispatch_external_institution(
        "julia",
        ENV_IRI,
        digest.as_str(),
        HANDLER_METHOD_NAME,
        SIGNATURE_IRI,
        &inputs,
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("dispatch_external_institution: {e}");
        }
    };

    // The output is a BoundedBy resource whose [lower, upper] should
    // bracket [0.5, 1.5] — a rigorous interval-arithmetic enclosure
    // of `sin(x) + 0.5` over `[0, π/2]`.
    let output =
        eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output Resource");
    let lower = output
        .get(&iri("urn:eigenius:intervals:lower"))
        .and_then(Value::as_float)
        .expect("output carries lower");
    let upper = output
        .get(&iri("urn:eigenius:intervals:upper"))
        .and_then(Value::as_float)
        .expect("output carries upper");

    assert!(
        lower <= 0.5,
        "lower bound must enclose 0.5; got lower={lower}, upper={upper}"
    );
    assert!(
        upper >= 1.5,
        "upper bound must enclose 1.5; got lower={lower}, upper={upper}"
    );
    // Sanity: the interval should be tight enough that we're
    // demonstrating real interval-arithmetic, not Inf/-Inf escape.
    assert!(
        lower >= 0.0 && upper <= 2.0,
        "interval must be tight (within [0, 2]) — got [{lower}, {upper}]"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
