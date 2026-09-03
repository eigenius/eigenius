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

//! Every ESL cell of `notebooks/examples/stats-and-reasoning.json` compiles
//! against the bootstrap chain, in cell order.
//!
//! **Why this exists.** Nothing executed this notebook, so nothing noticed when
//! it stopped working. It used `spec_str`, a rule retired on `2026-08-21`
//! (eigenius#203), and went on referencing it for months; the provenance/warrant
//! refactor then found it broken rather than broke it. A user-facing example is
//! the worst place to leave that, because the reader has no way to tell a stale
//! cell from a working one.
//!
//! Cells are compiled cumulatively — each against a layer carrying everything
//! the earlier cells committed — because that is how a reader runs them, and a
//! cell that only compiles in isolation is not a cell that works.
//!
//! This checks COMPILATION, not the institution dispatches the `eigenql` cells
//! perform: those need a running statistics institution and are covered by
//! `crates/eigenius-statistics/tests/ic50_measurement.rs` against the same
//! fixture shapes.

use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};

#[test]
fn every_esl_cell_of_the_stats_notebook_compiles() {
    const NOTEBOOK: &str = include_str!("../../notebooks/examples/stats-and-reasoning.json");
    let parsed: serde_json::Value = serde_json::from_str(NOTEBOOK).expect("notebook JSON parses");
    let cells = parsed["cells"]
        .as_array()
        .expect("notebook has a cells array");

    let ctx = bootstrap().expect("bootstrap seeds");
    let mut head = Arc::clone(ctx.head());
    let mut compiled = 0;

    for cell in cells {
        if cell["type"].as_str().unwrap_or("") != "esl" {
            continue;
        }
        let id = cell["id"].as_str().unwrap_or("?");
        let source = cell["source"].as_str().expect("esl cell has a source");

        let resources = esl::compile(source, &head).unwrap_or_else(|errs| {
            panic!(
                "notebook ESL cell `{id}` failed to compile:\n{}",
                errs.into_iter()
                    .map(|e| format!("  - {e:?}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
        assert!(
            !resources.is_empty(),
            "notebook ESL cell `{id}` compiled to zero resources"
        );

        // Stack it so the next cell sees what this one committed, as a reader would.
        let mut b = LayerBuilder::new(id, Some(Arc::clone(&head)));
        for r in resources {
            b.add_resource(r)
                .unwrap_or_else(|e| panic!("cell `{id}`: {e:?}"));
        }
        head = Arc::new(b.build(LayerStorage::in_memory()));
        compiled += 1;
    }

    assert!(
        compiled >= 8,
        "expected the notebook's ESL cells to be found and compiled; got {compiled}"
    );
}
