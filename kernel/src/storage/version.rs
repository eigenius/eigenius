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

//! On-disk schema versioning (D24).
//!
//! Phase 14 makes the kernel's RocksDB layout a stable on-disk contract.
//! [`SCHEMA_VERSION`] is the kernel's compiled-in expectation; every DB
//! carries its current version in [`SCHEMA_VERSION_KEY`] and the
//! `bootstrap_persistent` boot check (see [`crate::bootstrap`]) refuses
//! to open a DB whose version disagrees.
//!
//! Schema version is independent of `CARGO_PKG_VERSION` — most kernel
//! releases don't change the on-disk shape. Only PRs that change
//! persisted bytes (new prefix, changed CBOR field, renamed key, etc.)
//! bump `SCHEMA_VERSION`. See `docs/design/d24-schema-versioning.md`
//! for the full policy and contributor checklist.

use crate::storage::{PersistentBackend, StorageError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Compiled-in schema version this kernel expects on disk.
///
/// Bumped by 1 in any PR that changes persisted bytes such that a
/// kernel built before the PR would fail to read a DB written by a
/// kernel built after the PR. Each bump lands with a corresponding
/// `Migration` impl (see [`Migration`]) and a `schema-changelog.md`
/// entry. See `docs/design/d24-schema-versioning.md` §4 for the
/// contributor checklist.
///
/// **Current value (v1):** the cumulative Phase 14 layout. Prefixes:
/// `layer:`, `chain:`, `trace:`, `meta:`, `topo:`, `bloom:`,
/// `branch:`, `idx_pos:`, `idx_layer:`. Pre-v1 DBs are not supported.
pub const SCHEMA_VERSION: u32 = 1;

/// Meta key holding the DB's current schema version (4 BE bytes).
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Meta key holding the `CARGO_PKG_VERSION` of the kernel that last
/// wrote (or migrated) this DB. Diagnostic only — never consulted by
/// the boot check.
pub const LAST_WRITER_VERSION_KEY: &str = "last_writer_version";

/// Meta key holding a CBOR-encoded `Vec<MigrationRecord>` — every
/// successful migration appends one record. Populated as an empty
/// vector at seed time so subsequent migrations can append without a
/// "first migration is special" branch.
pub const SCHEMA_HISTORY_KEY: &str = "schema_history";

/// One row of [`SCHEMA_HISTORY_KEY`]. Records that this DB was
/// migrated from `from` to `to` at `applied_at_ms` by a kernel
/// identifying as `kernel_version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    pub from: u32,
    pub to: u32,
    pub applied_at_ms: i64,
    pub kernel_version: String,
}

/// Errors a migration may produce. Kept thin — the bootstrap caller
/// translates these into `BootstrapError::MigrationFailed` so the
/// operator-facing message comes out uniform.
#[derive(Debug)]
pub enum MigrationError {
    Storage(StorageError),
    /// Migration encountered data it can't move forward (e.g., a
    /// resource it expected to find but didn't). Never normal in a
    /// well-formed DB; usually a bug or external corruption.
    Inconsistent(String),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::Storage(e) => write!(f, "storage error during migration: {e}"),
            MigrationError::Inconsistent(msg) => {
                write!(f, "migration found inconsistent state: {msg}")
            }
        }
    }
}

impl std::error::Error for MigrationError {}

impl From<StorageError> for MigrationError {
    fn from(e: StorageError) -> Self {
        MigrationError::Storage(e)
    }
}

/// A registered upgrade step from one schema version to the next.
///
/// Implementations are required to be:
/// - **Idempotent** — re-running on an already-migrated DB is a no-op.
/// - **Atomic** — succeed completely or leave the DB at the previous
///   version. Achieved via `WriteBatch` for backends that support it.
/// - **Forward-only** — no `revert` method; once at version N, no path
///   back to N-1.
/// - **Self-contained** — uses only the [`PersistentBackend`] trait
///   surface; never calls back into general kernel code that requires
///   booting (which would be circular).
///
/// See `docs/design/d24-schema-versioning.md` §3.3 for the full
/// contract.
//
// `from_version` is domain-specific (the version a migration starts
// from), not a `from_*` conversion constructor. Allow the lint scoped
// to this trait.
#[allow(clippy::wrong_self_convention)]
pub trait Migration: Send + Sync {
    /// The version this migration upgrades *from* (the version
    /// stamped on the DB it expects to find).
    fn from_version(&self) -> u32;

    /// The version this migration upgrades *to*. By convention always
    /// `from_version() + 1`; the registry enforces no-gaps.
    fn to_version(&self) -> u32;

    /// One-line human description of what the migration does. Surfaces
    /// in `MigrationRecord.description` (when added) and in
    /// debug-logging during boot.
    fn description(&self) -> &str;

    /// Apply the migration. Implementations write all data changes
    /// before returning `Ok`; the bootstrap caller stamps the new
    /// `meta:schema_version` after this returns successfully.
    fn apply(&self, backend: &dyn PersistentBackend) -> Result<(), MigrationError>;
}

/// Set of migrations the current kernel knows about, indexed by
/// `from_version()`. The `default()` impl registers every
/// `vN_to_vN+1` migration from `1` to `SCHEMA_VERSION` — a debug
/// assertion at construction time verifies the chain has no gaps.
pub struct MigrationRegistry {
    migrations: BTreeMap<u32, Box<dyn Migration>>,
}

impl MigrationRegistry {
    /// Construct an empty registry. Use [`MigrationRegistry::default`]
    /// for the kernel's actual set.
    pub fn empty() -> Self {
        Self {
            migrations: BTreeMap::new(),
        }
    }

    /// Register a migration. Panics in debug builds if a migration is
    /// already registered for the same `from_version()` (the chain
    /// must be unique) or if `to_version() != from_version() + 1`.
    pub fn register(&mut self, m: Box<dyn Migration>) {
        debug_assert!(
            m.to_version() == m.from_version() + 1,
            "migration must increment version by 1: from {} to {}",
            m.from_version(),
            m.to_version()
        );
        let from = m.from_version();
        let prev = self.migrations.insert(from, m);
        debug_assert!(
            prev.is_none(),
            "duplicate migration registered for from_version={from}"
        );
    }

    /// Look up the migration that upgrades from `from`.
    pub fn get(&self, from: u32) -> Option<&dyn Migration> {
        self.migrations.get(&from).map(|b| b.as_ref())
    }

    /// True iff a contiguous chain `from → from+1 → ... → to` exists.
    /// Used by bootstrap to give the operator an actionable error
    /// before starting any partial migration work.
    pub fn has_path(&self, from: u32, to: u32) -> bool {
        if from > to {
            return false;
        }
        let mut cursor = from;
        while cursor < to {
            if !self.migrations.contains_key(&cursor) {
                return false;
            }
            cursor += 1;
        }
        true
    }

    /// Number of registered migrations.
    pub fn len(&self) -> usize {
        self.migrations.len()
    }

    /// True iff no migrations are registered.
    pub fn is_empty(&self) -> bool {
        self.migrations.is_empty()
    }
}

impl Default for MigrationRegistry {
    fn default() -> Self {
        // v1 is the initial schema (Phase 14). No migrations exist
        // yet — the first registered migration will arrive with
        // whichever PR first bumps SCHEMA_VERSION to 2.
        Self::empty()
    }
}

/// Encode a `u32` as 4 BE bytes for storage in `meta:schema_version`.
pub fn encode_schema_version(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// Decode the 4-BE-byte payload at `meta:schema_version`. Returns
/// `Err` on any length other than 4 — schema-version corruption is
/// not a "best effort" condition, the kernel must refuse to proceed.
pub fn decode_schema_version(bytes: &[u8]) -> Result<u32, String> {
    if bytes.len() != 4 {
        return Err(format!(
            "expected 4 bytes for schema_version, got {}",
            bytes.len()
        ));
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(bytes);
    Ok(u32::from_be_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_round_trip() {
        for v in [0u32, 1, 2, 7, 1024, u32::MAX] {
            let encoded = encode_schema_version(v);
            assert_eq!(encoded.len(), 4);
            assert_eq!(decode_schema_version(&encoded).unwrap(), v);
        }
    }

    #[test]
    fn decode_rejects_wrong_length() {
        assert!(decode_schema_version(&[]).is_err());
        assert!(decode_schema_version(&[1, 2, 3]).is_err());
        assert!(decode_schema_version(&[1, 2, 3, 4, 5]).is_err());
    }

    #[test]
    fn empty_registry_is_default_for_v1_kernel() {
        let registry = MigrationRegistry::default();
        assert_eq!(SCHEMA_VERSION, 1);
        assert!(
            registry.is_empty(),
            "v1 is the initial schema — no migrations should be registered yet"
        );
    }

    struct FakeMigration {
        from: u32,
    }

    impl Migration for FakeMigration {
        fn from_version(&self) -> u32 {
            self.from
        }
        fn to_version(&self) -> u32 {
            self.from + 1
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn apply(&self, _backend: &dyn PersistentBackend) -> Result<(), MigrationError> {
            Ok(())
        }
    }

    #[test]
    fn has_path_finds_contiguous_chain() {
        let mut registry = MigrationRegistry::empty();
        registry.register(Box::new(FakeMigration { from: 1 }));
        registry.register(Box::new(FakeMigration { from: 2 }));
        registry.register(Box::new(FakeMigration { from: 3 }));
        assert!(registry.has_path(1, 4));
        assert!(registry.has_path(2, 3));
        assert!(registry.has_path(1, 1)); // empty path is fine
    }

    #[test]
    fn has_path_rejects_gap() {
        let mut registry = MigrationRegistry::empty();
        registry.register(Box::new(FakeMigration { from: 1 }));
        registry.register(Box::new(FakeMigration { from: 3 })); // skips 2
        assert!(!registry.has_path(1, 4));
        assert!(!registry.has_path(2, 4));
    }

    #[test]
    fn has_path_rejects_backwards() {
        let registry = MigrationRegistry::empty();
        assert!(!registry.has_path(5, 3));
    }
}
