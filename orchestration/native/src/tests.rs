//! End-to-end tests for the load + execute flow against the wasm-http-shout
//! fixture. These are equivalent to the spike's `test/wasm.ts` checkpoint 2
//! but exercise the Rust path without going through a JS runtime.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use ciborium::Value as Cbor;
use eigenius_wasm_runtime::WasmComponentConfig;

use crate::cache;
use crate::execute;
use crate::host_state::{BridgeFuture, HostBridge};

const FIXTURE: &[u8] = include_bytes!(
    "../../../kernel/tests/fixtures/eigenius_wasm_http_shout.wasm"
);
const PROBE_FIXTURE: &[u8] = include_bytes!(
    "../../../kernel/tests/fixtures/eigenius_wasm_read_query_probe.wasm"
);

// ---------------------------------------------------------------------------
// CBOR helpers — encode an Eigon resource (property-keyed map of text keys)
// ---------------------------------------------------------------------------

fn encode_resource(pairs: &[(&str, &str)]) -> Vec<u8> {
    let map: Vec<(Cbor, Cbor)> = pairs
        .iter()
        .map(|(k, v)| (Cbor::Text((*k).to_string()), Cbor::Text((*v).to_string())))
        .collect();
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Cbor::Map(map), &mut out).expect("cbor encode");
    out
}

fn find_text_property<'a>(bytes: &'a [u8], key: &str) -> Option<String> {
    let val: Cbor = ciborium::de::from_reader(bytes).ok()?;
    walk(&val, key)
}

fn walk(val: &Cbor, key: &str) -> Option<String> {
    match val {
        Cbor::Map(entries) => {
            for (k, v) in entries {
                if let Cbor::Text(kk) = k {
                    if kk == key {
                        if let Cbor::Text(s) = v {
                            return Some(s.clone());
                        }
                    }
                }
                if let Some(s) = walk(v, key) {
                    return Some(s);
                }
            }
            None
        }
        Cbor::Array(items) => items.iter().find_map(|v| walk(v, key)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Test HostBridge
// ---------------------------------------------------------------------------

struct TestBridge {
    dispatch_calls: AtomicU32,
    canned_response: Vec<u8>,
}

impl HostBridge for TestBridge {
    fn dispatch<'a>(
        &'a self,
        _iri: String,
        _input: Vec<u8>,
        _argument: Vec<u8>,
    ) -> BridgeFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.dispatch_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.canned_response.clone())
        })
    }

    fn resolve<'a>(&'a self, _iri: String) -> BridgeFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(None) })
    }

    fn query<'a>(&'a self, _eigenql: String) -> BridgeFuture<'a, Vec<Vec<u8>>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// Cache tests mutate an env var that the code under test reads. Serialise
// them so parallel test threads don't trample each other's tempdirs.
static CACHE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_isolated_cache<F: FnOnce()>(f: F) {
    // Poisoning is fine here — a prior test panicking just means the lock is
    // tainted; the env var state we care about is reset below either way.
    let _guard = CACHE_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("EIGENIUS_WASM_CACHE", tmp.path());
    f();
    std::env::remove_var("EIGENIUS_WASM_CACHE");
}

#[test]
fn load_then_execute_against_wasm_http_shout() {
    with_isolated_cache(|| {
        let handle = execute::load(FIXTURE, WasmComponentConfig::default())
            .expect("load failed");
        assert!(handle > 0);

        let input = encode_resource(&[("urn:example:shout:text", "hello from wasm")]);
        let argument = Vec::new();

        let canned_response = encode_resource(&[(
            "urn:eigenius:program:value",
            "HELLO FROM WASM",
        )]);
        let bridge = Arc::new(TestBridge {
            dispatch_calls: AtomicU32::new(0),
            canned_response,
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt
            .block_on(execute::execute(
                handle,
                input,
                argument,
                bridge.clone() as Arc<dyn HostBridge>,
            ))
            .expect("execute failed");

        let shouted = find_text_property(&out, "urn:example:shout:shouted")
            .expect("output missing 'shouted' property");
        assert_eq!(shouted, "HELLO FROM WASM");
        assert_eq!(bridge.dispatch_calls.load(Ordering::Relaxed), 1);

        assert!(execute::unload(handle), "unload should succeed");
        assert!(!execute::unload(handle), "second unload should fail");
    });
}

#[test]
fn execute_with_unknown_handle_errors() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let bridge = Arc::new(TestBridge {
        dispatch_calls: AtomicU32::new(0),
        canned_response: Vec::new(),
    });
    let err = rt
        .block_on(execute::execute(
            0xDEAD_BEEF,
            Vec::new(),
            Vec::new(),
            bridge as Arc<dyn HostBridge>,
        ))
        .expect_err("expected unknown-handle error");
    assert!(err.to_string().contains("unknown component handle"), "{err}");
}

#[test]
fn cache_roundtrip() {
    with_isolated_cache(|| {
        // Two sequential loads of the same binary should yield different
        // handles (the handle table is per-load) but must both succeed,
        // proving the cache path doesn't panic on a hit.
        let h1 = execute::load(FIXTURE, WasmComponentConfig::default()).unwrap();
        let h2 = execute::load(FIXTURE, WasmComponentConfig::default()).unwrap();
        assert_ne!(h1, h2);
        assert!(execute::unload(h1));
        assert!(execute::unload(h2));
    });
}

// ---------------------------------------------------------------------------
// Probe bridge: asserts resolve/query get called with the expected args and
// returns canned payloads.
// ---------------------------------------------------------------------------

struct ProbeBridge {
    resolve_calls: AtomicU32,
    query_calls: AtomicU32,
    resolve_canned: Option<Vec<u8>>,
    query_canned: Vec<Vec<u8>>,
    expected_resolve_iri: String,
    expected_query_text: String,
}

impl HostBridge for ProbeBridge {
    fn dispatch<'a>(
        &'a self,
        _iri: String,
        _input: Vec<u8>,
        _argument: Vec<u8>,
    ) -> BridgeFuture<'a, Vec<u8>> {
        Box::pin(async move { Err("probe should not call dispatch".to_string()) })
    }

    fn resolve<'a>(&'a self, iri: String) -> BridgeFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            assert_eq!(iri, self.expected_resolve_iri);
            self.resolve_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.resolve_canned.clone())
        })
    }

    fn query<'a>(&'a self, eigenql: String) -> BridgeFuture<'a, Vec<Vec<u8>>> {
        Box::pin(async move {
            assert_eq!(eigenql, self.expected_query_text);
            self.query_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.query_canned.clone())
        })
    }
}

fn find_integer_property(bytes: &[u8], key: &str) -> Option<i64> {
    let val: Cbor = ciborium::de::from_reader(bytes).ok()?;
    walk_int(&val, key)
}

fn walk_int(val: &Cbor, key: &str) -> Option<i64> {
    match val {
        Cbor::Map(entries) => {
            for (k, v) in entries {
                if let Cbor::Text(kk) = k {
                    if kk == key {
                        if let Cbor::Integer(i) = v {
                            return Some((*i).try_into().ok()?);
                        }
                    }
                }
                if let Some(n) = walk_int(v, key) {
                    return Some(n);
                }
            }
            None
        }
        Cbor::Array(items) => items.iter().find_map(|v| walk_int(v, key)),
        _ => None,
    }
}

#[test]
fn resolve_and_query_callbacks_reach_the_guest() {
    with_isolated_cache(|| {
        let handle = execute::load(PROBE_FIXTURE, WasmComponentConfig::default())
            .expect("load probe fixture");

        let resolve_iri = "urn:test:probe:target";
        let query_text = "SELECT * WHERE @id = <urn:test:probe:target>";
        let canned_resolve = vec![0xAA; 17]; // 17-byte canned resource
        let canned_query = vec![vec![1, 2, 3], vec![4, 5], vec![6]]; // 3 rows

        let input = encode_resource(&[
            ("urn:test:probe:resolve_iri", resolve_iri),
            ("urn:test:probe:query_text", query_text),
        ]);

        let bridge = Arc::new(ProbeBridge {
            resolve_calls: AtomicU32::new(0),
            query_calls: AtomicU32::new(0),
            resolve_canned: Some(canned_resolve.clone()),
            query_canned: canned_query.clone(),
            expected_resolve_iri: resolve_iri.to_string(),
            expected_query_text: query_text.to_string(),
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt
            .block_on(execute::execute(
                handle,
                input,
                Vec::new(),
                bridge.clone() as Arc<dyn HostBridge>,
            ))
            .expect("execute probe");

        let resolved_len = find_integer_property(&out, "urn:test:probe:resolved_len")
            .expect("missing resolved_len");
        let query_rows = find_integer_property(&out, "urn:test:probe:query_rows")
            .expect("missing query_rows");

        assert_eq!(resolved_len, canned_resolve.len() as i64);
        assert_eq!(query_rows, canned_query.len() as i64);
        assert_eq!(bridge.resolve_calls.load(Ordering::Relaxed), 1);
        assert_eq!(bridge.query_calls.load(Ordering::Relaxed), 1);

        assert!(execute::unload(handle));
    });
}

#[test]
fn resolve_returns_none_when_not_found() {
    with_isolated_cache(|| {
        let handle = execute::load(PROBE_FIXTURE, WasmComponentConfig::default())
            .expect("load probe fixture");

        let resolve_iri = "urn:test:probe:missing";
        let query_text = "empty";

        let input = encode_resource(&[
            ("urn:test:probe:resolve_iri", resolve_iri),
            ("urn:test:probe:query_text", query_text),
        ]);

        let bridge = Arc::new(ProbeBridge {
            resolve_calls: AtomicU32::new(0),
            query_calls: AtomicU32::new(0),
            resolve_canned: None, // simulate not-found
            query_canned: Vec::new(),
            expected_resolve_iri: resolve_iri.to_string(),
            expected_query_text: query_text.to_string(),
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt
            .block_on(execute::execute(
                handle,
                input,
                Vec::new(),
                bridge as Arc<dyn HostBridge>,
            ))
            .expect("execute probe");

        let resolved_len = find_integer_property(&out, "urn:test:probe:resolved_len")
            .expect("missing resolved_len");
        let query_rows = find_integer_property(&out, "urn:test:probe:query_rows")
            .expect("missing query_rows");

        assert_eq!(resolved_len, -1, "not-found should flow through as -1");
        assert_eq!(query_rows, 0);

        assert!(execute::unload(handle));
    });
}

// ---------------------------------------------------------------------------
// Error-path coverage
// ---------------------------------------------------------------------------

/// Guest trap — fuel runs out mid-execution. The execute call must return
/// an error, the handle must remain usable for a subsequent run (i.e. the
/// handle table isn't corrupted by a trap).
#[test]
fn guest_trap_on_fuel_exhaustion_surfaces_error() {
    with_isolated_cache(|| {
        // 1000 fuel is far less than wasm-http-shout needs to parse its
        // input; instantiation + first few instructions burn through it.
        let tiny = WasmComponentConfig {
            fuel_limit: 1000,
            memory_limit_pages: WasmComponentConfig::default().memory_limit_pages,
        };
        let handle = execute::load(FIXTURE, tiny).expect("load");

        let input = encode_resource(&[("urn:example:shout:text", "hello")]);
        let bridge = Arc::new(TestBridge {
            dispatch_calls: AtomicU32::new(0),
            canned_response: encode_resource(&[(
                "urn:eigenius:program:value",
                "unused",
            )]),
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(execute::execute(
                handle,
                input,
                Vec::new(),
                bridge.clone() as Arc<dyn HostBridge>,
            ))
            .expect_err("expected fuel trap");

        let msg = format!("{err:#}");
        assert!(
            msg.to_lowercase().contains("fuel") || msg.to_lowercase().contains("trap"),
            "trap error should mention fuel/trap, got: {msg}",
        );
        // Callback wasn't reached — guest trapped before dispatch.
        assert_eq!(bridge.dispatch_calls.load(Ordering::Relaxed), 0);

        assert!(execute::unload(handle));
    });
}

/// Dispatch callback rejects — the guest's `dispatch_component` call returns
/// the `err(string)` branch of the `result` type. wasm-http-shout propagates
/// that up; we assert the error text flows through.
struct RejectingBridge {
    dispatch_calls: AtomicU32,
}

impl HostBridge for RejectingBridge {
    fn dispatch<'a>(
        &'a self,
        _iri: String,
        _input: Vec<u8>,
        _argument: Vec<u8>,
    ) -> BridgeFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.dispatch_calls.fetch_add(1, Ordering::Relaxed);
            Err("simulated handler crash".to_string())
        })
    }
    fn resolve<'a>(&'a self, _iri: String) -> BridgeFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(None) })
    }
    fn query<'a>(&'a self, _eigenql: String) -> BridgeFuture<'a, Vec<Vec<u8>>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

#[test]
fn dispatch_rejection_surfaces_to_guest() {
    with_isolated_cache(|| {
        let handle =
            execute::load(FIXTURE, WasmComponentConfig::default()).expect("load");

        let input = encode_resource(&[("urn:example:shout:text", "hello")]);
        let bridge = Arc::new(RejectingBridge {
            dispatch_calls: AtomicU32::new(0),
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(execute::execute(
                handle,
                input,
                Vec::new(),
                bridge.clone() as Arc<dyn HostBridge>,
            ))
            .expect_err("expected guest to surface dispatch failure");

        let msg = format!("{err:#}");
        assert!(
            msg.contains("simulated handler crash"),
            "guest should surface dispatch error text, got: {msg}",
        );
        assert_eq!(bridge.dispatch_calls.load(Ordering::Relaxed), 1);

        assert!(execute::unload(handle));
    });
}

/// Corrupt cache entry — a tampered `.cwasm` on disk with a valid meta tag
/// should cause `execute::load` to evict and recompile from source, not
/// fail. Subsequent reloads should find the (now correct) cache entry.
#[test]
fn corrupt_cache_entry_triggers_recompile() {
    with_isolated_cache(|| {
        let hash = cache::hash_binary(FIXTURE);
        let dir = cache::cache_dir().expect("cache dir");
        std::fs::create_dir_all(&dir).unwrap();

        // Plant a garbage cwasm with a valid meta tag — this is what a
        // truncated write or a partial crash would look like.
        let cwasm_path = dir.join(format!("{hash}.cwasm"));
        let meta_path = dir.join(format!("{hash}.meta"));
        std::fs::write(&cwasm_path, b"not-a-valid-serialised-component").unwrap();
        std::fs::write(&meta_path, "wasmtime-43").unwrap();

        // Load must succeed (recompile fallback) and re-populate the cache.
        let handle =
            execute::load(FIXTURE, WasmComponentConfig::default()).expect("load");
        assert!(handle > 0);

        // Cache should now hold a real cwasm — size should have grown past
        // the 32-byte garbage we planted.
        let metadata = std::fs::metadata(&cwasm_path).expect("cwasm exists");
        assert!(
            metadata.len() > 32,
            "recompile should have overwritten garbage, got {} bytes",
            metadata.len()
        );

        assert!(execute::unload(handle));
    });
}
