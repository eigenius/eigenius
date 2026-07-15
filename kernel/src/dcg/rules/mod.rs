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

//! **The grammar's rules** — everything that says how two constituents may combine.
//!
//! - [`combinators`] — the CATEGORIAL rules: forward/backward application, composition, the dependent
//!   determiner, the nominal-modification family. Decided sem-blind (they see only a
//!   [`CategoryPayload`](super::item::CategoryPayload)), which is what licenses the packed forest.
//! - [`registry`] — the TOKEN-KEYED rules (relatives, coordination, `but not`, the reciprocal, the
//!   appositives) plus the unary shifts. One definition of *where* each fires, consumed by both chart
//!   drivers, so the two cannot drift apart.
//!
//! Everything here depends on the [`Grammar`](super::grammar::Grammar) — the chain, the reserved-word
//! triggers, and the resolved category templates — and on nothing else. In particular, no rule can reach
//! a lexicon; if one could, it would eventually reach for something that is not a grammar constant,
//! which is how a `form → entries` lookup grew a chart parser in the first place.
//!
//! (`combinators.rs` is the file formerly known as `parser.rs`. It never held a parser — the chart
//! drivers live in `super::chart` — it holds the composition rules, and now it says so.)

pub(crate) mod combinators;
pub(crate) mod constructions;
pub(crate) mod registry;
