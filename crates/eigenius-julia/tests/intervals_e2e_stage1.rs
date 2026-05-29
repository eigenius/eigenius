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

//! Phase 19a.6 e2e — stage 1 (chain-side install lifecycle).
//!
//! Drives the D31 install pipeline for the IntervalArithmetic
//! institution end-to-end *up to* the actual `DispatchExternal` RPC:
//!
//! 1. Bootstrap a kernel chain.
//! 2. Commit the BoundedBy ontology layer
//!    (`julia/institutions/intervals/declarations/intervals-ontology.eigon.json`).
//! 3. Run `JuliaMirrorGenerator` against the BoundedBy class, lift
//!    the result through `mirror_to_resource`, and commit the
//!    `RuntimePackageMirror` resource.
//! 4. Build a `RuntimeEnvironment` resource (with a placeholder
//!    `image_digest`) referencing the mirror, and commit it.
//! 5. Commit the institution declarations layer
//!    (`Institution` + `QueryClass` + `RuntimeMethodSignature` from
//!    `intervals-institution.eigon.json`).
//! 6. Assert that
//!    [`crate::capability::registration::validate_external_institution_chain`]
//!    produces a clean `ExternalInstitutionPlan` for the IntervalArithmetic
//!    institution — the env IRI resolves, the image digest is
//!    forwarded, the QueryClass's `query_handler` resolves to the
//!    handler signature, and the worker dispatch metadata
//!    (`method_name`, `language`) match the declarations.
//!
//! This stage doesn't fire the AutoOnLoad gate — that's stage 2,
//! which adds a `DockerSpawner` + a built env image and commits a
//! `BoundedBy` instance to drive the round-trip. Stage 1 pins the
//! chain-side wiring so stage 2 starts from a known-good install.

use std::sync::Arc;

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::capability::registration::validate_external_institution_chain;
use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::registry::{InstitutionIndex, RuntimeKind};
use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::{Layer, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;

// ─── Paths to the institution's source-of-truth artifacts ──────────────

const ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-ontology.eigon.json"
);

const INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-institution.eigon.json"
);

// Phase 19d.2 added `BoundsRequest.expr: SymbolicExpression` to the
// intervals ontology, so the symbolics ontology must be committed
// before intervals can validate.
const SYMBOLICS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
);
// Phase 19f.1 added `SymbolicsToJuMPInput` to the symbolics ontology,
// referencing `jump:VariableBound` and `jump:Constraint` via
// class_types — the JuMP ontology must be on the chain before
// symbolics validates.
const JUMP_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");

// ─── Pinned IRIs ───────────────────────────────────────────────────────
//
// These match what the `.eigon.json` declarations carry; pinned here
// (rather than re-parsed) so a typo on either side surfaces as an
// assertion failure with a clear message.

const BOUNDED_BY_CLASS_IRI: &str = "urn:eigenius:intervals:BoundedBy";
const INSTITUTION_IRI: &str = "urn:eigenius:institutions:intervals";
const QUERY_CLASS_IRI: &str = "urn:eigenius:intervals:query_classes:bounded_by_validity";
const SIGNATURE_IRI: &str = "urn:eigenius:intervals:signatures:validate_bounded_by";
const ENV_IRI: &str = "urn:eigenius:intervals:env:v1";

/// Placeholder content-addressed digest stamped on the env. Stage 1
/// doesn't run the worker — the digest just rides through validate /
/// register code paths so a real digest swap in stage 2 doesn't
/// surface a structural mismatch.
const PLACEHOLDER_IMAGE_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

// ─── Helpers ───────────────────────────────────────────────────────────

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

/// Stage a JSON document onto `ctx`'s working layer, then route the
/// commit through [`commit_layer_default`] (the D41 single-layer-commit
/// surface) and advance `ctx.head` to the new layer.
fn commit_layer(
    ctx: &mut ExecutionContext,
    backend: &MemoryPersistentBackend,
    json: &str,
    name: &str,
) -> Arc<Layer> {
    let resources = eigon_json::parse_document(json).expect("Eigon-JSON must parse");
    for r in resources {
        ctx.add_resource(r).expect("add_resource");
    }
    let working = ctx.take_working(name).expect("take_working");
    let layer = commit_layer_default(working, ctx.storage().clone(), backend)
        .expect("commit_layer_default");
    ctx.advance_head(Arc::clone(&layer), name)
        .expect("advance_head");
    layer
}

/// `ChainAccessor` impl backed by an in-process [`Layer`]. The mirror
/// generator only needs `resolve`; the boundary-check methods can be
/// no-ops here because the e2e walks chains it just built itself —
/// no compositionality questions to answer.
struct LayerChain {
    layer: Arc<Layer>,
}

impl eigenius_runtime_substrate::chain::ChainAccessor for LayerChain {
    fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
        self.layer.resolve(target).map(|arc| (*arc).clone())
    }
    fn is_ancestor_or_equal(&self, _anchor: &Iri, _candidate: &Iri) -> bool {
        true
    }
    fn class_unchanged_between(
        &self,
        _mirror_layer: &Iri,
        _claim_layer: &Iri,
        _class_iri: &Iri,
    ) -> bool {
        true
    }
}

// ─── The test ──────────────────────────────────────────────────────────

#[test]
fn intervals_install_lifecycle_produces_clean_external_institution_plan() {
    // Stage 1 walks the install pipeline in process and pins the
    // resulting plan against the declared metadata.

    // 1. Bootstrap with a memory-backed `PersistentBackend` so commits
    //    route through `commit_layer_default` (D41 Phase G).
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // 1.4 JuMP ontology — must precede symbolics because
    //     `SymbolicsToJuMPInput` references jump:VariableBound and
    //     jump:Constraint via class_types (Phase 19f.1).
    commit_layer(&mut ctx, &backend, JUMP_ONTOLOGY_JSON, "jump_ontology");

    // 1.5 Symbolics ontology — must precede intervals because
    //     `BoundsRequest.expr` references SymbolicExpression
    //     (Phase 19d.2).
    commit_layer(
        &mut ctx,
        &backend,
        SYMBOLICS_ONTOLOGY_JSON,
        "symbolics_ontology",
    );

    // 2. Ontology layer — the BoundedBy class + value/lower/upper
    //    properties.
    let ontology_layer = commit_layer(&mut ctx, &backend, ONTOLOGY_JSON, "intervals_ontology");
    assert!(
        ontology_layer.resolve(&iri(BOUNDED_BY_CLASS_IRI)).is_some(),
        "BoundedBy class must resolve on the committed ontology layer"
    );

    // 3. Mirror layer — generate the typed Julia mirror against the
    //    BoundedBy class on the just-committed ontology layer, then
    //    lift the generator output into a `RuntimePackageMirror`
    //    resource via `mirror_to_resource`.
    let chain = LayerChain {
        layer: Arc::clone(&ontology_layer),
    };
    // The mirror's `source_layer` is a per-environment IRI the
    // operator chooses (see `eigenius mirror create --layer`); the
    // chain accessor's `resolve` looks up resources independently of
    // the layer IRI value, so any well-formed IRI works for stage 1.
    let source_layer_iri = iri("urn:eigenius:test:intervals:layer");
    let seed = [iri(BOUNDED_BY_CLASS_IRI)];
    let request = eigenius_runtime_substrate::mirror_generator::MirrorGenerationRequest {
        source_layer: &source_layer_iri,
        seed_classes: &seed,
        chain: &chain,
    };
    let generator = JuliaMirrorGenerator::new();
    let output = eigenius_runtime_substrate::mirror_generator::MirrorGenerator::generate(
        &generator, &request,
    )
    .expect("mirror generation");
    assert!(
        output
            .mirrored_classes
            .iter()
            .any(|i| i.as_str() == BOUNDED_BY_CLASS_IRI),
        "mirror closure must include BoundedBy"
    );
    let mirror_resource = mirror_to_resource(
        &generator,
        &output,
        &source_layer_iri,
        Some("2026-05-05T00:00:00.000Z"),
    );
    let mirror_iri = mirror_resource
        .id()
        .expect("mirror resource carries @id")
        .clone();
    ctx.add_resource(mirror_resource)
        .expect("commit mirror Resource");
    let mirror_working = ctx.take_working("intervals_mirror").expect("take_working");
    let mirror_layer =
        commit_layer_default(mirror_working, ctx.storage().clone(), backend.as_ref())
            .expect("commit mirror layer");
    ctx.advance_head(Arc::clone(&mirror_layer), "intervals_mirror")
        .expect("advance_head");
    let _mirror_layer = mirror_layer;

    // 4. Env layer — RuntimeEnvironment carrying language, image
    //    digest (placeholder), the mirror reference, runtime version,
    //    lockfile (verbatim), and lifecycle. `Service` lifecycle
    //    because the institution dispatch path goes through
    //    `LanguageRuntime::call_method` which rejects Job envs at
    //    the boundary check (D26 §5.3.1).
    let env = build_env_resource(&mirror_iri);
    ctx.add_resource(env).expect("commit env");
    let env_working = ctx.take_working("intervals_env").expect("take_working");
    let env_layer = commit_layer_default(env_working, ctx.storage().clone(), backend.as_ref())
        .expect("commit env layer");
    ctx.advance_head(Arc::clone(&env_layer), "intervals_env")
        .expect("advance_head");
    let _env_layer = env_layer;

    // 5. Institution layer — Institution + QueryClass +
    //    RuntimeMethodSignature.
    let institution_layer = commit_layer(
        &mut ctx,
        &backend,
        INSTITUTION_JSON,
        "intervals_institution",
    );
    assert!(
        institution_layer.resolve(&iri(INSTITUTION_IRI)).is_some(),
        "Institution must resolve on the committed institution layer"
    );

    // 6. Build the index from the head and assert
    //    validate_external_institution_chain produces exactly one
    //    plan with the metadata we declared.
    let head = Arc::clone(ctx.head());
    let (index, parse_errors) = InstitutionIndex::from_layer(&head);
    assert!(parse_errors.is_empty(), "{parse_errors:?}");

    let inst_entry = index
        .institution(&iri(INSTITUTION_IRI))
        .expect("Institution must be indexed");
    assert_eq!(inst_entry.runtime, Some(RuntimeKind::External));
    assert_eq!(
        inst_entry.requires_environment.as_ref().map(|i| i.as_str()),
        Some(ENV_IRI)
    );

    let (plans, errors) = validate_external_institution_chain(&head, &index);
    assert!(
        errors.is_empty(),
        "validate_external_institution_chain must accept the install: {errors:?}"
    );
    assert_eq!(plans.len(), 1, "expected exactly one plan; got {plans:?}");
    let plan = &plans[0];
    assert_eq!(plan.institution_iri.as_str(), INSTITUTION_IRI);
    assert_eq!(plan.env_iri.as_str(), ENV_IRI);
    assert_eq!(plan.image_digest, PLACEHOLDER_IMAGE_DIGEST);
    assert_eq!(plan.language, "julia");

    // The QueryClass's `query_handler` IRI is the lookup key the
    // kernel will use when routing through `Institution::query` —
    // pin it resolves to the right `(method_name, signature_iri)`
    // pair.
    let handler = plan
        .handlers
        .get(&iri(SIGNATURE_IRI))
        .expect("handler for signature_iri must be in the plan");
    assert_eq!(handler.method_name, "validate_bounded_by");
    assert_eq!(handler.signature_iri.as_str(), SIGNATURE_IRI);

    // Cross-check that the QueryClass's pointer back to the
    // institution + signature matches the plan's handler key.
    let qc = index
        .query_class(&iri(QUERY_CLASS_IRI))
        .expect("QueryClass indexed");
    assert_eq!(qc.institution_ref.as_str(), INSTITUTION_IRI);
    assert_eq!(qc.query_handler.as_str(), SIGNATURE_IRI);
    assert_eq!(qc.query_class.as_str(), BOUNDED_BY_CLASS_IRI);
}

// ─── Env Resource builder ──────────────────────────────────────────────

/// Build a structurally-valid `RuntimeEnvironment` carrying the
/// placeholder digest. The shape matches what `eigenius env create`
/// will produce in stage 2; v1 of `env create` only commits the env
/// metadata (not the actual image), so a placeholder digest is
/// representative of where stage 1 is in the lifecycle.
fn build_env_resource(mirror_iri: &Iri) -> Resource {
    let mut env = Resource::new(iri(ENV_IRI));
    env.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:runtime:RuntimeEnvironment",
        ))]),
    );
    env.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("intervals-env-v1".into()),
    );
    env.set(
        iri("urn:eigenius:runtime:language"),
        Value::String("julia".into()),
    );
    env.set(
        iri("urn:eigenius:runtime:image_digest"),
        Value::String(PLACEHOLDER_IMAGE_DIGEST.into()),
    );
    env.set(
        iri("urn:eigenius:runtime:runtime_version"),
        Value::String("1.10".into()),
    );
    // Lockfile bytes ride opaquely on the chain; the substrate parses
    // them at image-build time. Stage 1 just needs the property
    // present for structural validation.
    env.set(
        iri("urn:eigenius:runtime:lockfile"),
        Value::String("# stage 1 placeholder Manifest.toml\n".into()),
    );
    env.set(
        iri("urn:eigenius:runtime:lifecycle"),
        Value::ResourceRef(iri("urn:eigenius:runtime:lifecycle:Service")),
    );
    // Reference the mirror so the chain links the env to its typed
    // boundary surface; the property is recommended (D26 §5.3) so
    // omitting it would still pass structural validation, but the
    // stage-2 worker dispatch needs the link to resolve mirror codecs.
    env.set(
        iri("urn:eigenius:runtime:mirror_dependency"),
        Value::ResourceRef(mirror_iri.clone()),
    );
    env
}
