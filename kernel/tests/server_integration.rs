//! Integration tests for the gRPC server.
//!
//! These tests start an actual gRPC server, connect a client,
//! and verify end-to-end behavior.

use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;
use eigenius_kernel::server::proto::*;
use eigenius_kernel::server::EigeniusService;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(50200);

async fn start_test_server() -> String {
    let port = PORT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let addr = format!("0.0.0.0:{port}");

    let service = EigeniusService::new().unwrap();
    let server = tonic::transport::Server::builder()
        .add_service(service.into_server())
        .serve(addr.parse().unwrap());

    tokio::spawn(server);
    sleep(Duration::from_millis(200)).await; // Wait for server to start

    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn health_check() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client.health(HealthRequest {}).await.unwrap();
    let health = response.into_inner();

    assert!(health.healthy);
    assert!(!health.version.is_empty());
    assert!(health.resource_count > 0); // Core + program ontology resources
}

#[tokio::test]
async fn inspect_core_class() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client
        .inspect(InspectRequest {
            iri: "urn:eigenius:core:Class".to_string(),
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

#[tokio::test]
async fn inspect_not_found() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client
        .inspect(InspectRequest {
            iri: "urn:eigenius:nonexistent:Foo".to_string(),
        })
        .await
        .unwrap();

    assert!(!response.into_inner().found);
}

#[tokio::test]
async fn query_all_classes() {
    let endpoint = start_test_server().await;
    let mut client = EigeniusKernelClient::connect(endpoint).await.unwrap();

    let response = client
        .query(QueryRequest {
            eigenql: r#"USING "urn:eigenius:core:Class" MATCH Class(?c) { short_name: ?name } RETURN [] { short_name: ?name }"#.to_string(),
        })
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut count = 0;
    while let Ok(Some(_result)) = stream.message().await {
        count += 1;
    }

    // Should find core classes (Class, Property, DataType, etc.) + program classes
    assert!(count >= 6, "expected at least 6 classes, got {count}");
}

#[tokio::test]
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
        })
        .await
        .unwrap();

    let load = load_response.into_inner();
    assert!(load.success, "load failed: {:?}", load.errors);
    assert_eq!(load.resource_count, 5);

    // Query for dogs
    let query_response = client
        .query(QueryRequest {
            eigenql: r#"MATCH "urn:eigenius:example:Dog"(?d) { "urn:eigenius:example:name": ?name } RETURN [] { "urn:eigenius:example:name": ?name }"#.to_string(),
        })
        .await
        .unwrap();

    let mut stream = query_response.into_inner();
    let mut count = 0;
    while let Ok(Some(_result)) = stream.message().await {
        count += 1;
    }

    assert_eq!(count, 1, "expected 1 dog, got {count}");
}
