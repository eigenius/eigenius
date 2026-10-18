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

//! Content-addressed array capability (D53 §6.1).
//!
//! The kernel-facing *read half* of the native-over-file seam: given a
//! `PinnedExternalFile`'s `reference` + `content_hash` and a column selector, it
//! returns the column as a `Vec<f64>` so an in-kernel native recompute (D52) can
//! operate on genome-scale data that lives off-chain — **native grade is
//! unchanged**, only the storage is (D53 §6).
//!
//! The kernel does **not** fetch (that is the substrate's job, D53 §5). This
//! capability reads a **materialized local file**: a `file://` on a shared
//! volume, or a content-addressed entry the orchestrator already populated under
//! a cache root. The bytes are **re-verified against `content_hash` (fail
//! closed)** before any value is read — so the array the recompute trusts is
//! exactly the pinned input, re-checkable by re-fetching by hash.
//!
//! Scope: delimited text (CSV/TSV) — the format the WRN DepMap matrices use.
//! Columnar formats (Parquet/Arrow) are a follow-up (see the D53 plan); the
//! correctness root (`content_hash`) is format-independent.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// IRI prefix for content-addressed `PinnedExternalFile` entries — mirrors the
/// substrate's `pinned_external_file_iri` so a cache layout stays consistent
/// across the fetch (substrate) and read (kernel) halves of the seam.
const PINNED_FILE_CACHE_HEX_LEN: usize = 64;

/// Failure modes for [`ContentArrayStore::read_column`].
#[derive(Debug, thiserror::Error)]
pub enum ContentArrayError {
    /// The reference scheme isn't readable by the kernel (only a materialized
    /// `file://`, or a content-addressed cache entry, is). `oxen://` must be
    /// fetched substrate-side into the cache first (D53 §5).
    #[error("reference `{0}` is not locally readable (expected file:// or a cached entry; oxen:// must be materialized substrate-side first)")]
    NotLocallyReadable(String),

    /// The materialized file could not be read.
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// The bytes did not hash to the pinned `content_hash` — fail closed before
    /// any value is read (D53 §5).
    #[error("content hash mismatch for `{reference}`: expected {expected}, got {got}")]
    ContentHashMismatch {
        reference: String,
        expected: String,
        got: String,
    },

    /// The requested column is not present in the file header.
    #[error("column `{column}` not found in header of `{reference}`")]
    ColumnNotFound { reference: String, column: String },

    /// A data cell in the requested column is not a number.
    #[error("non-numeric value `{value}` in column `{column}` at data row {row} of `{reference}`")]
    NonNumeric {
        reference: String,
        column: String,
        row: usize,
        value: String,
    },
}

/// Reads content-verified columns from materialized external files (D53 §6.1).
#[derive(Debug, Clone, Default)]
pub struct ContentArrayStore {
    /// Root of the local content-addressed cache the orchestrator populates
    /// (`<root>/<sha256-hex>/<name>`). `None` ⇒ only `file://` references (a
    /// shared volume) are readable.
    cache_root: Option<PathBuf>,
}

impl ContentArrayStore {
    /// A store that reads only `file://` references (no content cache).
    pub fn new() -> Self {
        Self { cache_root: None }
    }

    /// A store backed by a local content-addressed cache (the depot's
    /// `extfile-cache`) plus `file://`.
    pub fn with_cache_root(root: impl Into<PathBuf>) -> Self {
        Self {
            cache_root: Some(root.into()),
        }
    }

    /// Resolve `reference` to a local path. `file://` strips to the path; any
    /// other scheme resolves against the content cache by `content_hash`
    /// (`<root>/<hex>/<basename>`), which is where the orchestrator's §5 resolver
    /// materializes it.
    fn local_path(
        &self,
        reference: &str,
        content_hash: &str,
    ) -> Result<PathBuf, ContentArrayError> {
        if let Some(rest) = reference.strip_prefix("file://") {
            return Ok(PathBuf::from(rest));
        }
        let root = self
            .cache_root
            .as_ref()
            .ok_or_else(|| ContentArrayError::NotLocallyReadable(reference.to_string()))?;
        let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
        if hex.len() != PINNED_FILE_CACHE_HEX_LEN {
            return Err(ContentArrayError::NotLocallyReadable(reference.to_string()));
        }
        let basename = reference
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("data");
        Ok(root.join(hex).join(basename))
    }

    /// Verify the file at `path` hashes to `content_hash` (streamed — never
    /// buffers the whole file), failing closed on mismatch.
    fn verify(
        path: &std::path::Path,
        reference: &str,
        content_hash: &str,
    ) -> Result<(), ContentArrayError> {
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(|source| ContentArrayError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|source| ContentArrayError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let got = format!("sha256:{:x}", hasher.finalize());
        if got != content_hash {
            return Err(ContentArrayError::ContentHashMismatch {
                reference: reference.to_string(),
                expected: content_hash.to_string(),
                got,
            });
        }
        Ok(())
    }

    /// Read `column` from the content-verified materialized file as a `Vec<f64>`.
    ///
    /// `media_type` selects the delimiter (`text/tab-separated-values` → tab,
    /// else comma). Empty cells are skipped (missing data); any other
    /// non-numeric cell fails closed. The bytes are hash-verified first.
    pub fn read_column(
        &self,
        reference: &str,
        content_hash: &str,
        media_type: &str,
        column: &str,
    ) -> Result<Vec<f64>, ContentArrayError> {
        let path = self.local_path(reference, content_hash)?;
        Self::verify(&path, reference, content_hash)?;

        let text = std::fs::read_to_string(&path).map_err(|source| ContentArrayError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let delim = if media_type == "text/tab-separated-values" {
            '\t'
        } else {
            ','
        };
        let split = |line: &str| -> Vec<String> {
            line.trim_end_matches(['\r', '\n'])
                .split(delim)
                .map(|c| c.trim_matches('"').to_string())
                .collect()
        };

        let mut lines = text.lines();
        let header = match lines.next() {
            Some(h) => split(h),
            None => {
                return Err(ContentArrayError::ColumnNotFound {
                    reference: reference.to_string(),
                    column: column.to_string(),
                })
            }
        };
        let col_idx = header.iter().position(|h| h == column).ok_or_else(|| {
            ContentArrayError::ColumnNotFound {
                reference: reference.to_string(),
                column: column.to_string(),
            }
        })?;

        let mut out = Vec::new();
        for (row, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fields = split(line);
            let Some(cell) = fields.get(col_idx) else {
                continue; // ragged short row — no value in this column
            };
            let cell = cell.trim();
            if cell.is_empty() {
                continue; // missing value
            }
            let v = cell
                .parse::<f64>()
                .map_err(|_| ContentArrayError::NonNumeric {
                    reference: reference.to_string(),
                    column: column.to_string(),
                    row,
                    value: cell.to_string(),
                })?;
            out.push(v);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_csv(name: &str, body: &str) -> (PathBuf, String) {
        let p = std::env::temp_dir().join(format!("eig_carray_{}_{name}", std::process::id()));
        std::fs::write(&p, body).unwrap();
        let hash = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
        (p, hash)
    }

    #[test]
    fn reads_named_column_after_verify() {
        let (p, hash) = write_csv(
            "ok.csv",
            "DepMap_ID,WRN,BRCA1\nACH-1,0.5,0.1\nACH-2,1.5,0.2\n",
        );
        let store = ContentArrayStore::new();
        let col = store
            .read_column(&format!("file://{}", p.display()), &hash, "text/csv", "WRN")
            .unwrap();
        assert_eq!(col, vec![0.5, 1.5]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn skips_empty_cells() {
        let (p, hash) = write_csv("na.csv", "id,x\na,1.0\nb,\nc,3.0\n");
        let store = ContentArrayStore::new();
        let col = store
            .read_column(&format!("file://{}", p.display()), &hash, "text/csv", "x")
            .unwrap();
        assert_eq!(col, vec![1.0, 3.0]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn tsv_delimiter() {
        let (p, hash) = write_csv("t.tsv", "id\tval\na\t2\nb\t4\n");
        let store = ContentArrayStore::new();
        let col = store
            .read_column(
                &format!("file://{}", p.display()),
                &hash,
                "text/tab-separated-values",
                "val",
            )
            .unwrap();
        assert_eq!(col, vec![2.0, 4.0]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn fails_closed_on_hash_mismatch() {
        let (p, _) = write_csv("tamper.csv", "id,x\na,1\n");
        let store = ContentArrayStore::new();
        let err = store
            .read_column(
                &format!("file://{}", p.display()),
                "sha256:0000",
                "text/csv",
                "x",
            )
            .unwrap_err();
        assert!(matches!(err, ContentArrayError::ContentHashMismatch { .. }));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_column_errors() {
        let (p, hash) = write_csv("mc.csv", "id,x\na,1\n");
        let store = ContentArrayStore::new();
        let err = store
            .read_column(&format!("file://{}", p.display()), &hash, "text/csv", "y")
            .unwrap_err();
        assert!(matches!(err, ContentArrayError::ColumnNotFound { .. }));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn non_numeric_fails_closed() {
        let (p, hash) = write_csv("nn.csv", "id,x\na,1\nb,oops\n");
        let store = ContentArrayStore::new();
        let err = store
            .read_column(&format!("file://{}", p.display()), &hash, "text/csv", "x")
            .unwrap_err();
        assert!(matches!(err, ContentArrayError::NonNumeric { .. }));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn cache_root_resolves_by_hash() {
        let body = "id,v\na,7\nb,8\n";
        let hash = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
        let hex = hash.strip_prefix("sha256:").unwrap();
        let root = std::env::temp_dir().join(format!("eig_carray_cache_{}", std::process::id()));
        let dir = root.join(hex);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("matrix.csv"), body).unwrap();
        let store = ContentArrayStore::with_cache_root(&root);
        // A non-file:// reference resolves against the cache by hash.
        let col = store
            .read_column("oxen://repo@c/matrix.csv", &hash, "text/csv", "v")
            .unwrap();
        assert_eq!(col, vec![7.0, 8.0]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
