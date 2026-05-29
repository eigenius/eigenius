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

//! Integration tests for the gRPC server.
//!
//! These tests start an actual gRPC server, connect a client,
//! and verify end-to-end behavior.

use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;
use eigenius_kernel::server::proto::*;
use eigenius_kernel::server::EigeniusService;
use std::time::{Duration, Instant};

/// Spin up a kernel gRPC server on an OS-assigned ephemeral port and
/// return its endpoint.
///
/// We bind a `TcpListener` ourselves to port 0, read back the assigned
/// port, then hand the already-bound listener to tonic via
/// `serve_with_incoming`. This avoids two failure modes the previous
/// `PORT_COUNTER` + `Server::serve(addr)` pattern had:
///
/// 1. **TIME_WAIT collisions across repeated runs.** Linux holds
///    just-released listening sockets in `TIME_WAIT` for ~60s, during
///    which a fresh `bind()` to the same port fails with `EADDRINUSE`
///    even though `ss -ltn` shows nothing listening. With ephemeral
///    ports the OS picks one that isn't TIME_WAIT-occupied.
/// 2. **Spawn-before-bind race.** Pre-binding here means the listener
///    is ready before we ever return; the spawned task only needs to
///    accept. The readiness probe below then catches genuine "task
///    crashed" failures rather than masking timing races.
async fn start_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let service = EigeniusService::new().unwrap();
    let server = tonic::transport::Server::builder()
        .add_service(service.into_server())
        .serve_with_incoming(incoming);
    let server_handle = tokio::spawn(server);

    // The listener is already bound, so this probe normally succeeds
    // on the first iteration. We keep the loop for two reasons: it
    // surfaces a panic in the spawned `accept` future quickly (via
    // `server_handle.is_finished()`), and it gives the runtime a
    // chance to schedule the spawned task on a parallel-test-heavy
    // run before declaring failure.
    let probe_addr = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut server_handle = Some(server_handle);
    loop {
        if tokio::net::TcpStream::connect(&probe_addr).await.is_ok() {
            break;
        }
        if let Some(h) = server_handle.as_ref() {
            if h.is_finished() {
                let h = server_handle.take().expect("checked above");
                let detail = match h.await {
                    Ok(Ok(())) => "future returned Ok(()) without ever binding".to_string(),
                    Ok(Err(e)) => {
                        let mut s = format!("{e:?}");
                        let mut src = std::error::Error::source(&e);
                        while let Some(inner) = src {
                            s.push_str(&format!(" -> {inner:?}"));
                            src = inner.source();
                        }
                        format!("transport error: {s}")
                    }
                    Err(join_err) if join_err.is_panic() => {
                        format!("server task panicked: {join_err}")
                    }
                    Err(join_err) => format!("server task join error: {join_err}"),
                };
                panic!("test server on port {port} exited before becoming ready: {detail}");
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "test server never accepted a connection on port {port} within 30s; \
                 spawned task still running, likely starved by parallel test load"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    format!("http://127.0.0.1:{port}")
}

#[tokio::test(flavor = "multi_thread")]
async fn health_check() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client.health(HealthRequest {}).await.unwrap();
    let health = response.into_inner();

    assert!(health.healthy);
    assert!(!health.version.is_empty());
    assert!(health.resource_count > 0); // Core + program ontology resources
}

#[tokio::test(flavor = "multi_thread")]
async fn inspect_core_class() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client
        .inspect(InspectRequest {
            at_layer: String::new(),
            iri: "urn:eigenius:core:Class".to_string(),
            branch: String::new(),
        })
        .await
        .unwrap();

    let resp = response.into_inner();
    assert!(resp.found);
    assert!(!resp.resource.is_empty());

    // Parse the CBOR response
    let resource = eigenius_kernel::ontology::eigon_cbor::parse_resource(&resp.resource).unwrap();
    assert_eq!(resource.id().unwrap().as_str(), "urn:eigenius:core:Class");
}

#[tokio::test(flavor = "multi_thread")]
async fn inspect_not_found() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client
        .inspect(InspectRequest {
            at_layer: String::new(),
            iri: "urn:eigenius:nonexistent:Foo".to_string(),
            branch: String::new(),
        })
        .await
        .unwrap();

    assert!(!response.into_inner().found);
}

#[tokio::test(flavor = "multi_thread")]
async fn query_all_classes() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client
        .query(QueryRequest { at_layer: String::new(),
            eigenql: r#"USING "urn:eigenius:core:Class" MATCH Class(?c) { short_name: ?name } RETURN [] { short_name: ?name }"#.to_string(),
            branch: String::new(),
        })
        .await
        .unwrap();

    let resp = response.into_inner();
    assert!(resp.success, "query failed: {}", resp.error);
    let count = row_count_from_document(&resp.document);

    // Should find core classes (Class, Property, DataType, etc.) + program classes
    assert!(count >= 6, "expected at least 6 classes, got {count}");
}

#[tokio::test(flavor = "multi_thread")]
async fn load_and_query() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    // Load the animals ontology
    let animals_json = include_str!("../../ontologies/examples/animals.json");

    let load_response = client
        .load(LoadRequest {
            resources: animals_json.as_bytes().to_vec(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
        })
        .await
        .unwrap();

    let load = load_response.into_inner();
    assert!(load.success, "load failed: {:?}", load.errors);
    assert_eq!(load.resource_count, 5);

    // Sanity-check that Rex (the single Dog instance in animals.json)
    // is reachable post-commit. If this fails, the load reported success
    // but didn't actually land Rex; if it passes, the failure below is
    // in the query engine's MATCH path, not in the commit pipeline.
    let rex_inspect = client
        .inspect(InspectRequest {
            iri: "urn:eigenius:example:rex".to_string(),
            at_layer: String::new(),
            branch: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        rex_inspect.found,
        "Rex was not reachable after load; load reported success but the user-layer \
         commit didn't actually land the resource (or it was tombstoned by a cascade)"
    );

    // Query for dogs
    let query_response = client
        .query(QueryRequest { at_layer: String::new(),
            eigenql: r#"MATCH "urn:eigenius:example:Dog"(?d) { "urn:eigenius:example:name": ?name } RETURN [] { "urn:eigenius:example:name": ?name }"#.to_string(),
            branch: String::new(),
        })
        .await
        .unwrap();

    let resp = query_response.into_inner();
    assert!(resp.success, "query failed: {}", resp.error);
    let count = row_count_from_document(&resp.document);
    assert_eq!(count, 1, "expected 1 dog, got {count}");
}

/// Decode a Query response document and return the ResultSet's
/// `urn:eigenius:query:row_count`.
fn row_count_from_document(document: &[u8]) -> i64 {
    use eigenius_kernel::ontology::eigon_cbor;
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::Value;
    use eigenius_kernel::ontology::well_known as wk;

    let resources = eigon_cbor::parse_document(document).expect("parse document");
    let is_a = Iri::parse(wk::IS_A).unwrap();
    let row_count_prop = Iri::parse("urn:eigenius:query:row_count").unwrap();
    for r in &resources {
        match r.get(&is_a) {
            Some(Value::Array(a))
                if a.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }) =>
            {
                if let Some(Value::Integer(n)) = r.get(&row_count_prop) {
                    return *n;
                }
            }
            _ => {}
        }
    }
    panic!("no ResultSet in query response document");
}
