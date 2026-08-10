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

//! **D62 S6 — assembly**: turn a parsed sentence's `Prop` into a chain resource.
//!
//! The DCG engine (D63) produces a closed, felicity-gated `Prop` per sentence; the reasoning
//! institution (D39) consumes chain-resident propositions carrying `IsDerivedAs` witnesses. This
//! crate is the join: parse → select one reading → D47-encode the term → emit Eigon-JSON that
//! `eigenius load` puts on the chain as a `reflection:DerivedResource` under a
//! `reflection:ProgramTrace`.
//!
//! **The grade is Derived, not Declared.** The parser is a deterministic program run over a
//! content-hashed input span, so the strongest witness the mechanics allow is
//! `ProgramTrace → IsDerivedAs parsed_i P_i`. That is what makes an *edit to the prose* visible to
//! the commit gate: a downstream certificate naming `derived(parsed_i, P)` stops resolving the
//! moment the parser derives a different `P`.
//!
//! **Reading selection is pin-driven and fails closed** ([`select`]). The page runs 60/62 ambiguous,
//! so "which reading" is not solved here — it is *declared*, against the human-verified skeletons in
//! `experiments/parsing/expected-readings.tsv`. Zero or several matches is an error with a
//! diagnostic, never a silent pick.

pub mod emit;
pub mod pipeline;
pub mod select;
pub mod snapshot;

pub use emit::{emit_document, EmitError, ParsedSentence};
pub use select::{select_pinned, Pin, SelectError};
pub use snapshot::{build_parser, open_head, ParserConfig};
