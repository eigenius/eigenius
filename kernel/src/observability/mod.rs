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

//! Observability primitives: structured-logging conventions used
//! across the kernel and (mirrored by review-time discipline) the
//! orchestrator.
//!
//! ## Logging convention
//!
//! Every `tracing` log call follows the same shape:
//!
//! 1. **`operation` field** — a stable dotted name identifying *what*
//!    is being logged. Pulled from the [`operation`] constant table
//!    so call sites can be discovered with `grep`. Format:
//!    `<crate>.<area>.<verb>` — lowercase, dot-separated.
//! 2. **A constant message string** — for human readers in pretty-mode
//!    output. Never a `format!()` interpolation; everything variable
//!    goes through fields.
//! 3. **Key–value fields** — using the keys from [`field`]. Flat (no
//!    nested objects); for variable-shape values, pre-serialise with
//!    `serde_json::to_string` and pass as a string field.
//!
//! Example:
//!
//! ```ignore
//! use crate::observability::{field, operation};
//!
//! tracing::info!(
//!     { field::OPERATION } = operation::LAYER_COMMIT,
//!     { field::LAYER_ID } = %layer.id(),
//!     { field::COUNT } = layer.resources().len(),
//!     "layer committed"
//! );
//! ```
//!
//! ## Subscriber
//!
//! [`init`] installs a `tracing-subscriber` formatter at process start.
//! Format and level come from environment variables:
//!
//! - `RUST_LOG` — standard per-module level filter (e.g.
//!   `info,eigenius_kernel::query=debug`).
//! - `EIGENIUS_LOG_FORMAT` — `json` for one-record-per-line JSON
//!   (production / log aggregator), `pretty` for the human-readable
//!   multi-line format (local dev). Defaults to `pretty` when stdout
//!   is a TTY, `json` otherwise.

pub mod field;
pub mod operation;

mod guard;
mod init;

pub use guard::RpcGuard;
pub use init::init;
