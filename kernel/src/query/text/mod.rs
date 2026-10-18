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

//! D43 §3 / M3 text retrieval: tokeniser pipeline, chain-aware BM25
//! scorer, query-time caches.
//!
//! The submodules here mirror the implementation-plan layout:
//!
//! - [`analyzer`] — the tokenisation pipeline (Unicode segmentation +
//!   lowercase + optional Porter stemming) consumed by both the
//!   indexing side (`LayerBuilder::build` at M3.5) and the query
//!   side (`TEXT_MATCH` / `TEXT_SCORE` evaluation at M3.7).
//!
//! Subsequent sub-milestones add `bm25` (chain-aware scoring), the
//! query caches, and the evaluator wiring.

pub mod analyzer;
pub mod bm25;
pub mod cache;
pub mod indexing;
pub mod search;
