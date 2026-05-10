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

//! `MethodInvocation` — payload type carried in
//! [`Request::DispatchMethod`](super::protocol::Request::DispatchMethod)
//! when `target_kind = Method`. Tells the worker which function to
//! call and which `RuntimeMethodSignature` IRI it implements.
//!
//! The worker is responsible for typed dispatch on `inputs`: each
//! input is a CBOR-encoded mirror struct value carrying its `is_a`
//! list, the worker uses the `is_a` to find the right mirror-struct
//! decoder, then calls `function_name` via Julia's multiple dispatch.

use serde::{Deserialize, Serialize};

/// A typed-method dispatch directive sent on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodInvocation {
    /// The unqualified function name the worker resolves in `Main`
    /// (or in any handler module loaded into `Main`'s scope). For
    /// Julia, this is the symbol the user-side handler module
    /// `export`s. Multi-method dispatch on the typed inputs picks the
    /// concrete method.
    pub function_name: String,

    /// IRI of the `RuntimeMethodSignature` resource this dispatch is
    /// intended to satisfy. Carried so the worker can echo it on
    /// `Response::DispatchOk` (substrate uses it to thread the trace
    /// through `RuntimeInvocation.script`) — *not* used by the worker
    /// for dispatch.
    pub signature_iri: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cbor_round_trip<T: Serialize + for<'de> Deserialize<'de>>(value: &T) -> T {
        let mut buf = Vec::new();
        ciborium::into_writer(value, &mut buf).expect("encode");
        ciborium::from_reader(&buf[..]).expect("decode")
    }

    #[test]
    fn method_invocation_round_trips_through_cbor() {
        let mi = MethodInvocation {
            function_name: "compute_selectivity_index".to_string(),
            signature_iri: "urn:eigenius:demo:assay:methods:compute_selectivity_index".to_string(),
        };
        let restored: MethodInvocation = cbor_round_trip(&mi);
        assert_eq!(restored, mi);
    }
}
