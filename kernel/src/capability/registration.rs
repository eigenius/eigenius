//! Scan a Layer for WASM component/institution resources and register them.
//!
//! A resource declares WASM backing via:
//!
//! ```json
//! {
//!   "@id": "urn:example:components:MyComponent",
//!   "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
//!   "urn:eigenius:program:component:implementation": "wasm",
//!   "urn:eigenius:program:component:wasm_binary": "<base64 bytes>",
//!   "urn:eigenius:program:component:capability_level": "urn:eigenius:program:capability_levels:pure"
//! }
//! ```
//!
//! Institutions use the `urn:eigenius:institution:` namespace for the same properties.
//! `wasm_binary_ref` (blob store IRI) is reserved for future use.

use super::wasm_component::{CapabilityLevel, WasmComponent, WasmComponentConfig};
use super::wasm_institution::WasmFiberReasoner;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::component::ComponentRegistry;

/// Protected namespace prefixes. Domain-supplied WASM modules cannot register
/// IRIs under these (D12 §8.4).
const PROTECTED_NAMESPACES: &[&str] = &[
    "urn:eigenius:core:",
    "urn:eigenius:program:",
    "urn:eigenius:reflection:",
    "urn:eigenius:institution:",
];

/// Exception: built-in components under program:components: can use the
/// program namespace (they're part of the built-in registry anyway).
const PROTECTED_NAMESPACE_EXCEPTIONS: &[&str] = &["urn:eigenius:program:components:"];

/// An error encountered while loading WASM extensions from a layer.
#[derive(Debug, Clone)]
pub struct RegistrationError {
    pub resource_iri: String,
    pub message: String,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.resource_iri, self.message)
    }
}

/// A warning emitted during a scan (non-fatal).
#[derive(Debug, Clone)]
pub struct RegistrationWarning {
    pub resource_iri: String,
    pub message: String,
}

impl std::fmt::Display for RegistrationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] warning: {}", self.resource_iri, self.message)
    }
}

/// Summary of what was registered during a scan.
#[derive(Debug, Default)]
pub struct RegistrationReport {
    pub components_registered: Vec<String>,
    pub institutions_registered: Vec<String>,
    pub errors: Vec<RegistrationError>,
    pub warnings: Vec<RegistrationWarning>,
}

/// An IO-capability WASM component awaiting forwarding to the orchestrator.
/// The kernel can't host IO components itself (they need network/LLM access);
/// the caller ships these to the orchestrator via `RegisterWasmComponent`.
pub struct PendingIoComponent {
    pub resource_iri: String,
    pub wasm_binary: Vec<u8>,
    pub fuel_limit: u64,
    pub memory_limit_pages: u32,
}

/// Full scan result, including IO components that need orchestrator registration.
#[derive(Default)]
pub struct ScanResult {
    pub report: RegistrationReport,
    pub wasm_institutions: Vec<WasmFiberReasoner>,
    pub pending_io_components: Vec<PendingIoComponent>,
}

/// Walk a Layer looking for WASM component and institution resources.
///
/// - Pure/read components are loaded and registered in `components` (kernel host).
/// - IO components are collected as `PendingIoComponent` entries for the caller
///   to forward to the orchestrator (kernel can't host IO).
/// - Institutions are returned as `WasmFiberReasoner` instances (kernel host).
///
/// Resources without `implementation = "wasm"` are ignored — they are
/// handled by the regular built-in or remote component path.
pub fn scan_and_register(layer: &Layer, components: &mut ComponentRegistry) -> ScanResult {
    let mut result = ScanResult::default();

    let impl_prop = Iri::parse("urn:eigenius:program:component:implementation").unwrap();
    let inst_impl_prop = Iri::parse("urn:eigenius:institution:implementation").unwrap();

    for resource in layer.resources().values() {
        let id = match resource.id() {
            Some(i) => i.as_str().to_string(),
            None => continue, // Only top-level resources can be registered
        };

        // Component: has program:component:implementation = "wasm"
        if let Some(Value::String(s)) = resource.get(&impl_prop) {
            if s == "wasm" {
                if let Err(e) = check_namespace(&id) {
                    result.report.errors.push(RegistrationError {
                        resource_iri: id,
                        message: e,
                    });
                    continue;
                }

                // Decide kernel-host (pure/read) vs orchestrator-host (io) by
                // inspecting the declared capability level on the resource.
                match classify_component_capability(resource) {
                    Ok(CapabilityClass::KernelHost) => match load_wasm_component(resource, layer) {
                        Ok(component) => {
                            let declared_iri = component.iri().to_string();
                            if declared_iri != id {
                                result.report.warnings.push(RegistrationWarning {
                                        resource_iri: id.clone(),
                                        message: format!(
                                            "WASM binary declares IRI '{declared_iri}' which differs from resource @id '{id}' — using binary's IRI",
                                        ),
                                    });
                            }
                            components.register(declared_iri.clone(), Box::new(component));
                            result.report.components_registered.push(declared_iri);
                        }
                        Err(e) => {
                            result.report.errors.push(RegistrationError {
                                resource_iri: id,
                                message: e,
                            });
                        }
                    },
                    Ok(CapabilityClass::OrchestratorHost) => {
                        match extract_pending_io(resource, &id, layer) {
                            Ok(pending) => {
                                result.pending_io_components.push(pending);
                                result.report.components_registered.push(id);
                            }
                            Err(e) => {
                                result.report.errors.push(RegistrationError {
                                    resource_iri: id,
                                    message: e,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        result.report.errors.push(RegistrationError {
                            resource_iri: id,
                            message: e,
                        });
                    }
                }
                continue;
            }
        }

        // Institution: has institution:implementation = "wasm"
        if let Some(Value::String(s)) = resource.get(&inst_impl_prop) {
            if s == "wasm" {
                match load_wasm_institution(resource, layer) {
                    Ok(reasoner) => {
                        if let Err(e) = check_namespace(&id) {
                            result.report.errors.push(RegistrationError {
                                resource_iri: id,
                                message: e,
                            });
                        } else {
                            let decl_iri = reasoner.institution_iri().as_str().to_string();
                            if decl_iri != id {
                                result.report.warnings.push(RegistrationWarning {
                                    resource_iri: id.clone(),
                                    message: format!(
                                        "WASM binary declares IRI '{decl_iri}' which differs from resource @id '{id}' — using binary's IRI",
                                    ),
                                });
                            }
                            result.wasm_institutions.push(reasoner);
                            result.report.institutions_registered.push(decl_iri);
                        }
                    }
                    Err(e) => {
                        result.report.errors.push(RegistrationError {
                            resource_iri: id,
                            message: e,
                        });
                    }
                }
            }
        }
    }

    result
}

/// Load a WASM component from a resource.
fn load_wasm_component(resource: &Resource, layer: &Layer) -> Result<WasmComponent, String> {
    let bytes = extract_wasm_bytes(
        resource,
        "urn:eigenius:program:component:wasm_binary",
        "urn:eigenius:program:component:wasm_binary_ref",
        layer,
    )?;
    let level =
        extract_capability_level(resource, "urn:eigenius:program:component:capability_level")?;
    let config = extract_config(
        resource,
        "urn:eigenius:program:component:fuel_limit",
        "urn:eigenius:program:component:memory_limit_pages",
    );
    WasmComponent::from_bytes(&bytes, level, config)
}

/// Load a WASM fiber reasoner from an institution resource.
fn load_wasm_institution(resource: &Resource, layer: &Layer) -> Result<WasmFiberReasoner, String> {
    let bytes = extract_wasm_bytes(
        resource,
        "urn:eigenius:institution:wasm_binary",
        "urn:eigenius:institution:wasm_binary_ref",
        layer,
    )?;
    let config = extract_config(
        resource,
        "urn:eigenius:institution:fuel_limit",
        "urn:eigenius:institution:memory_limit_pages",
    );
    WasmFiberReasoner::from_bytes(&bytes, config)
}

/// Extract WASM binary bytes from a resource, trying inline first then blob ref.
fn extract_wasm_bytes(
    resource: &Resource,
    inline_prop: &str,
    _ref_prop: &str,
    _layer: &Layer,
) -> Result<Vec<u8>, String> {
    let inline_iri = Iri::parse(inline_prop).unwrap();

    // Try inline binary (base64-encoded or hex-encoded raw bytes)
    if let Some(Value::String(s)) = resource.get(&inline_iri) {
        return decode_wasm_binary(s);
    }

    // Blob ref is reserved for future implementation
    Err(format!(
        "resource has no '{inline_prop}' (blob ref support is future work)"
    ))
}

/// Decode a WASM binary from a string. Supports:
/// - base64 encoding (standard)
/// - hex encoding (with "hex:" prefix)
fn decode_wasm_binary(s: &str) -> Result<Vec<u8>, String> {
    if let Some(hex_data) = s.strip_prefix("hex:") {
        hex::decode(hex_data).map_err(|e| format!("hex decode failed: {e}"))
    } else {
        base64_decode(s)
    }
}

/// Minimal standard base64 decode (RFC 4648, no padding tolerance).
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decode = [255u8; 256];
    for (i, &b) in ALPHA.iter().enumerate() {
        decode[b as usize] = i as u8;
    }

    let s = s.trim();
    let input: Vec<u8> = s
        .bytes()
        .filter(|&b| b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    for chunk in input.chunks(4) {
        if chunk.is_empty() {
            break;
        }
        let mut buf = [0u8; 4];
        for (i, &b) in chunk.iter().enumerate() {
            let v = decode[b as usize];
            if v == 255 {
                return Err(format!("invalid base64 character: {}", b as char));
            }
            buf[i] = v;
        }
        if chunk.len() >= 2 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if chunk.len() >= 3 {
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if chunk.len() == 4 {
            out.push((buf[2] << 6) | buf[3]);
        }
    }
    Ok(out)
}

/// Where a WASM component should be hosted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityClass {
    /// Pure or read: hosted in the kernel via Wasmtime.
    KernelHost,
    /// IO: hosted in the orchestrator via jco + Deno WebAssembly.
    OrchestratorHost,
}

fn classify_component_capability(resource: &Resource) -> Result<CapabilityClass, String> {
    let iri = Iri::parse("urn:eigenius:program:component:capability_level").unwrap();
    let level_str = match resource.get(&iri) {
        Some(Value::String(s)) => s.as_str(),
        _ => return Ok(CapabilityClass::KernelHost), // Default: most restrictive (pure)
    };
    match level_str {
        "urn:eigenius:program:capability_levels:pure"
        | "urn:eigenius:program:capability_levels:read" => Ok(CapabilityClass::KernelHost),
        "urn:eigenius:program:capability_levels:io" => Ok(CapabilityClass::OrchestratorHost),
        other => Err(format!("unknown capability level: {other}")),
    }
}

fn extract_capability_level(resource: &Resource, prop: &str) -> Result<CapabilityLevel, String> {
    let iri = Iri::parse(prop).unwrap();
    let level_str = match resource.get(&iri) {
        Some(Value::String(s)) => s.as_str(),
        _ => return Ok(CapabilityLevel::Pure), // Default to most restrictive
    };

    match level_str {
        "urn:eigenius:program:capability_levels:pure" => Ok(CapabilityLevel::Pure),
        "urn:eigenius:program:capability_levels:read" => Ok(CapabilityLevel::Read),
        "urn:eigenius:program:capability_levels:io" => Err(
            "IO capability-level components must be classified for orchestrator hosting before reaching this function"
                .to_string(),
        ),
        other => Err(format!("unknown capability level: {other}")),
    }
}

/// Build a `PendingIoComponent` descriptor from the resource.
fn extract_pending_io(
    resource: &Resource,
    resource_iri: &str,
    layer: &Layer,
) -> Result<PendingIoComponent, String> {
    let bytes = extract_wasm_bytes(
        resource,
        "urn:eigenius:program:component:wasm_binary",
        "urn:eigenius:program:component:wasm_binary_ref",
        layer,
    )?;
    let config = extract_config(
        resource,
        "urn:eigenius:program:component:fuel_limit",
        "urn:eigenius:program:component:memory_limit_pages",
    );
    Ok(PendingIoComponent {
        resource_iri: resource_iri.to_string(),
        wasm_binary: bytes,
        fuel_limit: config.fuel_limit,
        memory_limit_pages: config.memory_limit_pages,
    })
}

fn extract_config(resource: &Resource, fuel_prop: &str, mem_prop: &str) -> WasmComponentConfig {
    let mut config = WasmComponentConfig::default();

    if let Some(Value::Integer(n)) = resource.get(&Iri::parse(fuel_prop).unwrap()) {
        if *n > 0 {
            config.fuel_limit = *n as u64;
        }
    }
    if let Some(Value::Integer(n)) = resource.get(&Iri::parse(mem_prop).unwrap()) {
        if *n > 0 {
            config.memory_limit_pages = *n as u32;
        }
    }
    config
}

/// Check that an IRI doesn't violate namespace protection.
fn check_namespace(iri: &str) -> Result<(), String> {
    for exception in PROTECTED_NAMESPACE_EXCEPTIONS {
        if iri.starts_with(exception) {
            return Ok(());
        }
    }
    for protected in PROTECTED_NAMESPACES {
        if iri.starts_with(protected) {
            return Err(format!(
                "cannot register WASM extension under protected namespace '{protected}'"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_check_rejects_core() {
        assert!(check_namespace("urn:eigenius:core:Foo").is_err());
        assert!(check_namespace("urn:eigenius:reflection:Foo").is_err());
        assert!(check_namespace("urn:eigenius:institution:Foo").is_err());
    }

    #[test]
    fn namespace_check_rejects_program_except_components() {
        // Plain program namespace: rejected
        assert!(check_namespace("urn:eigenius:program:Foo").is_err());
        // But program:components: is an exception (built-ins live there)
        assert!(check_namespace("urn:eigenius:program:components:Foo").is_ok());
    }

    #[test]
    fn namespace_check_accepts_user_namespaces() {
        assert!(check_namespace("urn:example:components:MyComp").is_ok());
        assert!(check_namespace("urn:customer:myapp:foo").is_ok());
    }

    #[test]
    fn base64_decode_known_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_decode_tolerates_whitespace() {
        assert_eq!(base64_decode("Zm9v\nYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("Zm9v YmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_decode_rejects_invalid() {
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn hex_prefix_decoding() {
        assert_eq!(decode_wasm_binary("hex:48656c6c6f").unwrap(), b"Hello");
    }
}
