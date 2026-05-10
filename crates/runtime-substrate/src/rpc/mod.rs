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

//! Worker RPC — CBOR over Unix domain socket per D26 §8.1.
//!
//! The substrate communicates with worker processes through five verbs:
//!
//! - `health` — liveness check; returns `numerical_metadata` and the
//!   in-image cross-check signals (D26 §9.3).
//! - `instantiate` — boot the worker against the env's pinned runtime.
//! - `register_mirror` — load a `RuntimePackageMirror`'s library
//!   archive into the runtime's package manager.
//! - `dispatch_method` — execute a script or method call against
//!   resolved input resources; returns the produced output resource.
//! - `evict` — graceful shutdown signal (used by the warm-worker pool
//!   that lands in Phase 19c; declared in v1 so workers know about it).
//!
//! ## Wire format
//!
//! - **Marshalling**: CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949))
//!   via `ciborium`. Eigon resources at the payload level use RFC 8746
//!   typed-array tags for numerical arrays — that concern lives in the
//!   per-language crate's marshalling layer; the substrate's protocol
//!   envelope just carries opaque payload bytes.
//! - **Framing**: each frame is a 4-byte big-endian length prefix
//!   followed by that many CBOR bytes. The length-prefix gives
//!   deterministic recovery from a malformed message — the reader
//!   knows exactly where a frame ends and can resync the next one.
//! - **Max frame size**: configurable, defaults to 64 MiB. Hard cap to
//!   reject pathological input before allocation.
//!
//! ## Phase 18a scope
//!
//! Protocol types + framing codec + a sync client wrapping a
//! `UnixStream`. No actual worker speaks this protocol yet — the bash
//! smoke runtime that exercises it ships in a later 18a milestone.

pub mod client;
pub mod codec;
pub mod method;
pub mod protocol;

pub use client::{ClientError, WorkerRpcClient};
pub use codec::{decode_frame, encode_frame, FrameError, MAX_FRAME_SIZE_DEFAULT};
pub use method::MethodInvocation;
pub use protocol::{HealthInfo, NumericalMetadata, Request, Response, TargetKind};
