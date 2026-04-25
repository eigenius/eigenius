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

//! Compiled-component disk cache.
//!
//! Component::from_binary takes ~225ms for a 4.7MB component (measured in
//! the Phase 8 spike). Persisting the compiled form to disk makes cold-path
//! instantiation effectively free on the second run.
//!
//! Layout (under `$EIGENIUS_WASM_CACHE` or `~/.cache/eigenius/wasm/`):
//!
//!   <sha256_hex>.cwasm   ← wasmtime-serialised Component
//!   <sha256_hex>.meta    ← JSON metadata (currently only wasmtime version)
//!
//! The meta file guards against loading a cwasm produced by a different
//! wasmtime version — deserialisation across versions is unsafe.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use wasmtime::component::Component;
use wasmtime::Engine;

/// Identifier for the wasmtime build this addon was compiled against.
/// Cache entries with a different tag are rejected because
/// `Component::deserialize_file` is not guaranteed safe across versions.
/// Bump this when the wasmtime dep changes.
const WASMTIME_TAG: &str = "wasmtime-43";

pub fn hash_binary(binary: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(binary);
    hex::encode(hasher.finalize())
}

pub fn cache_dir() -> Result<PathBuf> {
    if let Ok(override_path) = std::env::var("EIGENIUS_WASM_CACHE") {
        return Ok(PathBuf::from(override_path));
    }
    let base = dirs::cache_dir().context("cannot determine user cache dir")?;
    Ok(base.join("eigenius").join("wasm"))
}

/// Try to load a previously-compiled component from disk. Returns `Ok(None)`
/// if no entry exists or if the entry was produced by a different wasmtime
/// version (stale).
pub fn try_load(engine: &Engine, hash: &str) -> Result<Option<Component>> {
    let dir = cache_dir()?;
    let cwasm_path = dir.join(format!("{hash}.cwasm"));
    let meta_path = dir.join(format!("{hash}.meta"));

    if !cwasm_path.exists() || !meta_path.exists() {
        return Ok(None);
    }

    let meta = std::fs::read_to_string(&meta_path).context("read cache meta")?;
    if meta.trim() != WASMTIME_TAG {
        return Ok(None);
    }

    // SAFETY: `deserialize_file` is unsafe because it assumes the file was
    // produced by this process's wasmtime build. The tag check above is the
    // only guard we have — if an attacker can write to the cache dir, they
    // can compromise this. The cache is per-user and treated as trusted.
    let component = unsafe { Component::deserialize_file(engine, &cwasm_path) }
        .map_err(|e| anyhow::anyhow!("deserialize {}: {e}", cwasm_path.display()))?;
    Ok(Some(component))
}

/// Serialise the component to disk. Writes atomically via tmp+rename so
/// concurrent loads don't read a half-written file.
pub fn store(component: &Component, hash: &str) -> Result<()> {
    let dir = cache_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;

    let cwasm_path = dir.join(format!("{hash}.cwasm"));
    let meta_path = dir.join(format!("{hash}.meta"));
    let cwasm_tmp = dir.join(format!("{hash}.cwasm.tmp"));
    let meta_tmp = dir.join(format!("{hash}.meta.tmp"));

    let bytes = component
        .serialize()
        .map_err(|e| anyhow::anyhow!("serialize component: {e}"))?;
    std::fs::write(&cwasm_tmp, &bytes).context("write cwasm tmp")?;
    std::fs::write(&meta_tmp, WASMTIME_TAG).context("write meta tmp")?;

    std::fs::rename(&cwasm_tmp, &cwasm_path).context("rename cwasm")?;
    std::fs::rename(&meta_tmp, &meta_path).context("rename meta")?;

    Ok(())
}

/// Clear any cache entry for the given hash. Used when a stored entry turns
/// out to be corrupt.
#[allow(dead_code)]
pub fn evict(hash: &str) -> Result<()> {
    let dir = cache_dir()?;
    let _ = std::fs::remove_file(dir.join(format!("{hash}.cwasm")));
    let _ = std::fs::remove_file(dir.join(format!("{hash}.meta")));
    Ok(())
}

/// Sanity check that the cache dir can be created. Called at startup so
/// misconfiguration (e.g. invalid `EIGENIUS_WASM_CACHE` path) fails fast.
#[allow(dead_code)]
pub fn ensure_writable() -> Result<()> {
    let dir = cache_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        let a = hash_binary(b"hello");
        let b = hash_binary(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // 256 bits in hex
    }

    #[test]
    fn hash_differs_on_change() {
        let a = hash_binary(b"hello");
        let b = hash_binary(b"hello world");
        assert_ne!(a, b);
    }
}
