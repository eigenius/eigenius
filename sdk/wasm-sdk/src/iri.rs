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

//! Well-known IRI constants used by SDK resource builders.
//!
//! These mirror a subset of the kernel's `kernel/src/ontology/well_known.rs`
//! constants — only those an SDK consumer actually uses when constructing
//! declaration resources for the D14 institution vocabulary.

// --- Core (subset) ---

pub const IS_A: &str = "urn:eigenius:core:is_a";
pub const SHORT_NAME: &str = "urn:eigenius:core:short_name";
pub const DESCRIPTION: &str = "urn:eigenius:core:description";

// --- D14 Institution vocabulary ---

pub const INSTITUTION_CLASS: &str = "urn:eigenius:institution:Institution";
pub const INSTITUTION_IRI: &str = "urn:eigenius:institution:institution_iri";
pub const INSTITUTION_NAME: &str = "urn:eigenius:institution:institution_name";
pub const INSTITUTION_REF: &str = "urn:eigenius:institution:institution_ref";
pub const RUNTIME: &str = "urn:eigenius:institution:runtime";
pub const RUNTIME_WASM: &str = "urn:eigenius:institution:runtimes:wasm";
pub const RUNTIME_EXTERNAL: &str = "urn:eigenius:institution:runtimes:external";
pub const RUNTIME_IN_PROCESS: &str = "urn:eigenius:institution:runtimes:in_process";

pub const EXPORT_FORMAT_CLASS: &str = "urn:eigenius:institution:ExportFormat";
pub const IMPORT_FORMAT_CLASS: &str = "urn:eigenius:institution:ImportFormat";
pub const FROM_CLASS: &str = "urn:eigenius:institution:from_class";
pub const TO_CLASS: &str = "urn:eigenius:institution:to_class";
pub const PAYLOAD_TYPE: &str = "urn:eigenius:institution:payload_type";
pub const PROCEDURE: &str = "urn:eigenius:institution:procedure";

pub const QUERY_CLASS_CLASS: &str = "urn:eigenius:institution:QueryClass";
pub const QUERY_CLASS: &str = "urn:eigenius:institution:query_class";
pub const RESULT_CLASS: &str = "urn:eigenius:institution:result_class";
pub const DISPATCH_ROLE: &str = "urn:eigenius:institution:dispatch_role";
pub const QUERY_HANDLER: &str = "urn:eigenius:institution:query_handler";
pub const DISPATCH_ON_DEMAND: &str = "urn:eigenius:institution:dispatch_roles:on_demand";
pub const DISPATCH_AUTO_ON_LOAD: &str = "urn:eigenius:institution:dispatch_roles:auto_on_load";
pub const DISPATCH_DECIDABLE: &str = "urn:eigenius:institution:dispatch_roles:decidable";

pub const COMORPHISM_CLASS: &str = "urn:eigenius:institution:Comorphism";
pub const EXPORT_FORMAT: &str = "urn:eigenius:institution:export_format";
pub const TRANSFORMATION: &str = "urn:eigenius:institution:transformation";
pub const IMPORT_FORMAT: &str = "urn:eigenius:institution:import_format";
pub const EXACT: &str = "urn:eigenius:institution:exact";

pub const VERDICT: &str = "urn:eigenius:institution:Verdict";
