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

//! Reproduce the deployed-stack path: domain lexica (the demo verb layer and a
//! UMLS-shaped disease layer) committed via the `Load` RPC over a real `RocksStore`,
//! then parsed via the `ParseSentence` RPC. This mirrors the `just up` / `eigenius
//! load` / `eigenius lexicon parse` flow, proving a user-committed layer's
//! `lexicon:form` value index is populated in RocksDB and that the lazy `LexicalIndex`
//! resolves it at parse time. The sentence "every Werner syndrome affects HeLa"
//! composes because the UMLS disease KIND fills the demo verb's Entity slot by
//! subsumption (the concept class is a subclass of its semantic-type class, which is a
//! subclass of `lexicon:Entity`).

use std::sync::Arc;

use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{LoadRequest, ParseSentenceRequest};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

// A minimal UMLS-shaped layer: a semantic-type class, the lexicon descriptor, one
// concept class subclassed under it, and one common-noun entry for "Werner Syndrome".
const UMLS_ESL: &str = r#"
namespace core       = "urn:eigenius:core";
namespace reflection = "urn:eigenius:reflection";
namespace epistemic  = "urn:eigenius:reflection:epistemic";
namespace eigentt    = "urn:eigenius:eigentt";
namespace lexicon    = "urn:eigenius:lexicon";
namespace umlssty    = "urn:eigenius:umlssty";
namespace umlscui    = "urn:eigenius:umlscui";

class umlssty:T047 : lexicon:Entity {
    description = "UMLS Semantic Type T047 — Disease or Syndrome.";
}

resource lexicon:umls : lexicon:Lexicon {
    lexicon:source   = "UMLS Metathesaurus 2026AA — Level 0 / SRL-0 sources only";
    lexicon:language = "en";
}

class umlscui:C0043119 : umlssty:T047 {
    description = "Werner Syndrome. UMLS CUI C0043119.";
}

resource umlscui:e_C0043119_0 : lexicon:LexicalEntry {
    lexicon:form       = "Werner Syndrome";
    lexicon:cat        = type_expr( lexicon:cat_n(umlscui:C0043119, lexicon:num_any) );
    lexicon:sem        = umlscui:C0043119;
    lexicon:sem_type   = type_expr( Set );
    lexicon:sense      = "umls:C0043119";
    lexicon:grade      = epistemic:declared;
    lexicon:in_lexicon = lexicon:umls;
}
"#;

// The demo lexicon: a general transitive verb `affects` (Entity slots) + the proper
// noun `HeLa`. Untagged ⇒ always available regardless of parse scope.
const DEMO_ESL: &str = include_str!("../../../experiments/lexicon/lexicon.esl");

#[tokio::test(flavor = "multi_thread")]
async fn umls_layer_loads_and_parses_a_sentence_over_rocksdb() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    // Commit two layers via the Load RPC (ESL → compile → validate → commit →
    // store_layer), exactly as `eigenius load <file>` does: the demo verb layer, then
    // the UMLS disease layer.
    for (label, esl) in [("demo", DEMO_ESL), ("umls", UMLS_ESL)] {
        let load = service
            .load(Request::new(LoadRequest {
                resources: esl.as_bytes().to_vec(),
                content_type: "application/esl".to_string(),
                auto_commit: true,
                branch: String::new(),
                policy: None,
                explicit_tombstones: Vec::new(),
            }))
            .await
            .expect("load rpc")
            .into_inner();
        assert!(load.success, "{label} load failed: {:?}", load.errors);
    }

    // Parse a full sentence scoped to the UMLS lexicon: "Werner syndrome" is the UMLS
    // disease KIND (a common noun), `every` is the bootstrap closed-class determiner,
    // and `affects`/`HeLa` come untagged from the demo layer. The quantified kind fills
    // the verb's Entity slot by subsumption — this exercises the lazy form-value-index
    // lookup over the committed RocksDB chain end to end.
    let resp = service
        .parse_sentence(Request::new(ParseSentenceRequest {
            sentence: "every Werner syndrome affects HeLa".to_string(),
            scope: vec!["urn:eigenius:lexicon:umls".to_string()],
            profile: String::new(),
            at_layer: String::new(),
            branch: String::new(),
        }))
        .await
        .expect("parse rpc")
        .into_inner();

    assert!(
        !resp.parses.is_empty(),
        "'every Werner syndrome affects HeLa' must parse over the committed RocksDB \
         chain (the committed UMLS form resolves through the lazy value index)"
    );
    assert!(
        resp.parses.iter().any(|p| p.is_sentence),
        "at least one parse is a complete sentence (S); got {:?}",
        resp.parses.iter().map(|p| &p.category).collect::<Vec<_>>()
    );
}
