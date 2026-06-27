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

//! Ad-hoc profiling harness (D65 load scaling): load the partitioned UMLS chain
//! through the `Load` RPC over a real `RocksStore`, timing each commit phase, to
//! locate the per-chunk-time growth observed during the live witness.
//!
//! Ignored by default (it reads a multi-GB on-disk `umls-chain/` produced by
//! `umls-import --out-dir` and takes minutes). Run with:
//!
//! ```text
//! EIG_PHASE_TIMING=1 UMLS_CHAIN_DIR=$PWD/umls-chain UMLS_CHUNKS=6 \
//!   cargo test -p eigenius-storage-rocksdb --test profile_chain_load -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::LoadRequest;
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "profiling harness; needs umls-chain/ on disk; minutes-long"]
async fn profile_umls_chain_load() {
    let chain_dir = std::env::var("UMLS_CHAIN_DIR")
        .unwrap_or_else(|_| format!("{}/../../umls-chain", env!("CARGO_MANIFEST_DIR")));
    let n_chunks: usize = std::env::var("UMLS_CHUNKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);

    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;
    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    // Build the file list: base + the first N concept chunks, in filename order.
    let mut files: Vec<String> = vec![format!("{chain_dir}/umls-000-base.esl")];
    for i in 1..=n_chunks {
        files.push(format!("{chain_dir}/umls-{i:03}.esl"));
    }

    for f in &files {
        let bytes = std::fs::read(f).unwrap_or_else(|e| panic!("read {f}: {e}"));
        let nbytes = bytes.len();
        let t0 = Instant::now();
        let load = service
            .load(Request::new(LoadRequest {
                resources: bytes,
                content_type: "application/esl".to_string(),
                auto_commit: true,
                branch: String::new(),
                policy: None,
                explicit_tombstones: Vec::new(),
            }))
            .await
            .expect("load rpc")
            .into_inner();
        let wall = t0.elapsed().as_secs_f64();
        assert!(load.success, "{f} load failed: {:?}", load.errors);
        // Marker line the analysis greps for; the per-phase PHASE_TIMING lines
        // (from the pipeline, under EIG_PHASE_TIMING) interleave above this.
        eprintln!(
            "CHUNK_DONE file={} bytes={} wall_s={:.1}",
            f.rsplit('/').next().unwrap(),
            nbytes,
            wall
        );
    }
}
