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
//
// Shared gRPC helpers used by the mirror / env / script / institution
// CLI surfaces.

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::server::proto;
use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;
use tonic::transport::Channel;

/// Issue a query that returns the resource at `iri` as part of the
/// result document. Walks the document, finds the matching resource by
/// IRI, returns it. Returns `None` if not found.
pub(crate) async fn fetch_resource(
    client: &mut EigeniusKernelClient<Channel>,
    iri: &str,
) -> Option<Resource> {
    // Use the kernel's Inspect RPC — the canonical "give me this
    // resource by IRI" surface. It walks the parent-layer chain on
    // the kernel side and returns the resource as Eigon-CBOR.
    let resp = client
        .inspect(proto::InspectRequest {
            iri: iri.to_string(),
            at_layer: String::new(),
            branch: String::new(),
        })
        .await
        .ok()?
        .into_inner();
    if !resp.found {
        return None;
    }
    eigon_cbor::parse_resource_lenient(&resp.resource).ok()
}

/// Submit a single Resource via Load with auto_commit. Helper for
/// `mirror create` etc. — anywhere the CLI needs to commit a single
/// resource the substrate built locally.
pub(crate) async fn submit_resource_for_load(
    client: &mut EigeniusKernelClient<Channel>,
    resource: &Resource,
) {
    let cbor_bytes = eigon_cbor::serialize_resource(resource);
    let request = proto::LoadRequest {
        resources: cbor_bytes,
        content_type: "application/eigon+cbor".to_string(),
        auto_commit: true,
        branch: String::new(),
        // Default policy (Reject{100}) and no explicit tombstones —
        // this surface predates D41's policy wire-through.
        policy: None,
        explicit_tombstones: Vec::new(),
    };
    match client.load(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.success {
                eprintln!("Load failed:");
                for err in &resp.errors {
                    eprintln!("  {}: {}", err.rule, err.message);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}
