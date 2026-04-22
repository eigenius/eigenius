//! Phase 9a end-to-end durability test.
//!
//! Exercises the full SEED → install → shutdown → RESUME → dispatch
//! cycle against a real RocksDB instance. The test:
//!
//! 1. Opens a fresh RocksStore in a tempdir.
//! 2. Builds an `EigeniusService` with the persistent backend (SEED path).
//! 3. Installs the `wasm-ordering-institution` fixture via the Load RPC.
//! 4. Dispatches a `ConvergenceQuery` via FiberQuery and checks the result.
//! 5. Drops the service and the store (RocksDB is single-writer).
//! 6. Re-opens a new RocksStore at the same path (RESUME path).
//! 7. Rehydrates WASM institutions from the persisted chain.
//! 8. Repeats the FiberQuery and verifies the same result — proving the
//!    institution is live again without a re-install.
//!
//! This is the automated counterpart to the manual smoke test recorded
//! when Phase 9a landed. See D13 (Durable Kernel State) and #15.

use std::sync::Arc;

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{
    FiberQueryRequest, HealthRequest, InspectRequest, ListInstitutionsRequest, LoadRequest,
};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

const INSTITUTION_IRI: &str = "urn:eigenius:test:wasm:ordering";
const REFINEMENT_CLASS: &str = "urn:eigenius:test:wasm:Refinement";
const CONVERGENCE_QUERY_CLASS: &str = "urn:eigenius:test:wasm:ConvergenceQuery";
const CONVERGED_PROP: &str = "urn:eigenius:test:wasm:converged";
const FIXTURE: &[u8] =
    include_bytes!("../../../kernel/tests/fixtures/eigenius_wasm_ordering_institution.wasm");

/// Standard base64 (RFC 4648) encoder — mirrors `encode_base64` in the CLI.
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(ALPHA[(b0 >> 2) as usize] as char);
        out.push(ALPHA[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHA[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHA[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Build the JSON resource the CLI's `capability install --kind institution`
/// path sends over the Load RPC.
fn institution_install_json() -> String {
    let b64 = encode_base64(FIXTURE);
    serde_json::json!({
        "@id": INSTITUTION_IRI,
        "urn:eigenius:core:is_a": ["urn:eigenius:institution:Institution"],
        "urn:eigenius:institution:institution_iri": INSTITUTION_IRI,
        "urn:eigenius:institution:institution_name": "ordering",
        "urn:eigenius:institution:implementation": "wasm",
        "urn:eigenius:institution:wasm_binary": b64,
    })
    .to_string()
}

/// Build a ConvergenceQuery resource (tolerance=0.01, latest_delta=0.005).
/// The institution should report converged=true for these values.
fn convergence_query_json() -> String {
    serde_json::json!({
        "@id": "urn:eigenius:test:dur:query-1",
        "urn:eigenius:core:is_a": [CONVERGENCE_QUERY_CLASS],
        "urn:eigenius:test:wasm:tolerance": 0.01,
        "urn:eigenius:test:wasm:latest_delta": 0.005,
    })
    .to_string()
}

/// Parse a FiberQuery result (CBOR) and extract the `converged` boolean.
fn converged_from_result(cbor: &[u8]) -> bool {
    // The institution returns an anonymous result resource (no @id), so
    // we must use the lenient parser.
    let resource = eigon_cbor::parse_resource_lenient(cbor).expect("parse fiber result");
    let prop = Iri::parse(CONVERGED_PROP).unwrap();
    match resource.get(&prop) {
        Some(Value::Boolean(b)) => *b,
        other => panic!("expected boolean '{CONVERGED_PROP}', got {other:?}"),
    }
}

/// Install the ordering institution and dispatch one ConvergenceQuery.
/// Asserts that the query reports convergence.
async fn install_and_dispatch(service: &EigeniusService) {
    let resp = service
        .load(Request::new(LoadRequest {
            resources: institution_install_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
        }))
        .await
        .expect("load rpc")
        .into_inner();
    assert!(resp.success, "install failed: {:?}", resp.errors);

    // The #15 fix means the declared class should be queryable immediately.
    let inspect = service
        .inspect(Request::new(InspectRequest {
            at_layer: String::new(),
            iri: REFINEMENT_CLASS.to_string(),
        }))
        .await
        .expect("inspect rpc")
        .into_inner();
    assert!(
        inspect.found,
        "Refinement class missing after install — #15 regression"
    );

    let fq = service
        .fiber_query(Request::new(FiberQueryRequest {
            institution_iri: INSTITUTION_IRI.to_string(),
            query: convergence_query_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
        }))
        .await
        .expect("fiber_query rpc")
        .into_inner();
    assert!(fq.success, "fiber_query failed: {}", fq.error);
    assert!(
        converged_from_result(&fq.result),
        "expected converged=true for |0.005| <= 0.01"
    );
}

/// Dispatch one ConvergenceQuery. Also verifies that `list_institutions`
/// sees the rehydrated registration and `inspect` still finds the
/// institution-declared class.
async fn dispatch_only(service: &EigeniusService) {
    let insts = service
        .list_institutions(Request::new(ListInstitutionsRequest {
            at_layer: String::new(),
        }))
        .await
        .expect("list rpc")
        .into_inner();
    assert!(
        insts.institutions.iter().any(|i| i.iri == INSTITUTION_IRI),
        "institution missing after restart — rehydration regression"
    );

    let inspect = service
        .inspect(Request::new(InspectRequest {
            at_layer: String::new(),
            iri: REFINEMENT_CLASS.to_string(),
        }))
        .await
        .expect("inspect rpc")
        .into_inner();
    assert!(
        inspect.found,
        "Refinement class missing after restart — persistence regression"
    );

    let fq = service
        .fiber_query(Request::new(FiberQueryRequest {
            institution_iri: INSTITUTION_IRI.to_string(),
            query: convergence_query_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
        }))
        .await
        .expect("fiber_query rpc")
        .into_inner();
    assert!(fq.success, "fiber_query failed after restart: {}", fq.error);
    assert!(
        converged_from_result(&fq.result),
        "expected converged=true after restart"
    );
}

#[tokio::test]
async fn install_survives_kernel_restart() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().to_owned();

    // ---- Round 1: SEED + install ---------------------------------------
    {
        let store = Arc::new(RocksStore::open(&db_path).expect("open rocks"));
        let backend: Arc<dyn PersistentBackend> = store;

        let service = EigeniusService::with_persistent_backend(
            eigenius_kernel::program::component::ComponentRegistry::default(),
            Arc::clone(&backend),
        )
        .expect("build service");

        // Sanity — SEED path wrote something to the backend.
        let health = service
            .health(Request::new(HealthRequest {}))
            .await
            .expect("health")
            .into_inner();
        assert!(health.healthy);
        assert!(health.resource_count > 0);

        install_and_dispatch(&service).await;
        // `service` and `backend` drop here — the RocksDB lock is released.
    }

    // ---- Round 2: RESUME + re-dispatch --------------------------------
    {
        let store = Arc::new(RocksStore::open(&db_path).expect("re-open rocks"));
        let backend: Arc<dyn PersistentBackend> = store;

        let service = EigeniusService::with_persistent_backend(
            eigenius_kernel::program::component::ComponentRegistry::default(),
            Arc::clone(&backend),
        )
        .expect("rebuild service");

        // Walk the persisted chain and re-register every WASM institution
        // we find — this is what `start_server` does at boot.
        let errors = service.rehydrate_wasm_from_chain().await;
        assert!(
            errors.is_empty(),
            "rehydration errors after restart: {errors:?}"
        );

        dispatch_only(&service).await;
    }
}
