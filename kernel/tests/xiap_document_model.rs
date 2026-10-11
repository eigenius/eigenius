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

//! The XIAP report's D96 document model loads through the kernel (survey §4 step 5, step 1).
//!
//! `experiments/parsing/uab-xiap-document-model.py` emits it from the report, which is gitignored
//! material — so the ESL is gitignored too and this test is `#[ignore]`d, taking its path from
//! `XIAP_DOCUMENT_MODEL`. What it pins is that the emitted vocabulary is the kernel's: unit kinds
//! that exist, a `source_document` satisfying `class_types reference:Reference`, and spans typed as
//! integers.
//!
//!     XIAP_DOCUMENT_MODEL=$PWD/experiments/uab/step5-document-model/xiap-document-model.esl \
//!       cargo test --test xiap_document_model -- --ignored --nocapture

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::validation::Validator;

#[test]
#[ignore = "reads the gitignored emitted model; set XIAP_DOCUMENT_MODEL"]
fn the_document_model_loads_and_validates() {
    let Ok(path) = std::env::var("XIAP_DOCUMENT_MODEL") else {
        eprintln!("SKIP: XIAP_DOCUMENT_MODEL unset");
        return;
    };
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: {path} not readable");
        return;
    };
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let resources = esl::compile(&src, &head).unwrap_or_else(|errs| {
        panic!(
            "the emitted document model compiles: {}",
            errs.iter()
                .take(4)
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    eprintln!("compiled {} resources", resources.len());
    let mut b = LayerBuilder::new("xiap-document-model", Some(head));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let errors: Vec<String> = Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "{} validation error(s); first: {}",
        errors.len(),
        errors.first().map(String::as_str).unwrap_or("")
    );
}
