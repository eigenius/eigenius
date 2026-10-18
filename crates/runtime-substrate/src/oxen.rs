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

//! D53 §2 Oxen backend — fetch a versioned, content-addressed dataset file from
//! an Oxen remote into the substrate's depot cache.
//!
//! **Prebuilt client, not the crate (D53 §9).** Embedding `liboxen` as a Rust
//! dependency is rejected (~160 transitive crates: actix, arrow, polars, …,
//! wrong for the Eigenius build and TCB). Instead we shell out to the prebuilt
//! `oxen` CLI installed in the orchestrator image — the weight lives in the
//! image, not our build graph. The raw HTTP protocol is undocumented (the
//! private `liboxen`↔server wire format), so a hand-rolled client would chase a
//! moving target; the CLI is the supported surface.
//!
//! **Availability TCB, not correctness TCB (D53 §2).** Oxen is trusted only to
//! *deliver bytes*. The caller ([`crate::external_file`]) recomputes Eigenius's
//! own `content_hash` over the materialized bytes and fails closed on mismatch,
//! so a compromised or buggy Oxen never affects correctness.
//!
//! **Reference grammar.** `oxen://[<host>/]<namespace>/<repo>@<revision>/<path>`
//! - `<host>` optional; defaults to `hub.oxen.ai` (OxenHub). A 2-segment
//!   coordinate before `@` (`ns/repo`) uses the default host; a 3-segment one
//!   (`host/ns/repo`) names the host explicitly (a port is allowed:
//!   `0.0.0.0:3000/ns/repo`).
//! - `<revision>` is a branch name or commit id.
//! - `<path>` is the file's path inside the repo.
//!
//! **Auth (D53 §10).** Per-host bearer token in `auth_config.toml` under the
//! oxen config dir (`$OXEN_CONFIG_DIR`, honored by the CLI). The token is a
//! deployment secret held substrate-side only; the orchestrator writes it via
//! [`write_auth_config`] (or the deployment pre-populates the dir). The token
//! never enters a worker image — fetch is substrate-side (D53 §5).

use std::path::{Path, PathBuf};

use crate::error::RunError;

/// Default Oxen host when a reference omits one (OxenHub).
pub const DEFAULT_OXEN_HOST: &str = "hub.oxen.ai";

/// A parsed `oxen://` reference (see module docs for the grammar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OxenRef {
    /// `<host>` (with optional `:port`), e.g. `hub.oxen.ai`.
    pub host: String,
    /// `<namespace>/<repo>` — the `oxen download` ID (exactly one `/`).
    pub repo_id: String,
    /// Branch name or commit id.
    pub revision: String,
    /// File path inside the repo.
    pub path: String,
}

impl OxenRef {
    /// The last path segment — the filename `oxen download` writes into the
    /// output dir, and the name the cache entry takes.
    pub fn basename(&self) -> &str {
        self.path
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or(&self.path)
    }
}

/// Parse an `oxen://…` reference. Returns a human-readable reason on a
/// malformed locator (the caller wraps it in
/// [`RunError::ExternalFetchFailed`]).
pub fn parse(reference: &str) -> Result<OxenRef, String> {
    let rest = reference
        .strip_prefix("oxen://")
        .ok_or_else(|| format!("not an oxen:// reference: `{reference}`"))?;
    let (coord, rev_and_path) = rest
        .split_once('@')
        .ok_or_else(|| format!("oxen reference missing `@<revision>`: `{reference}`"))?;
    let (revision, path) = rev_and_path
        .split_once('/')
        .ok_or_else(|| format!("oxen reference missing `/<path>` after revision: `{reference}`"))?;
    if revision.is_empty() {
        return Err(format!(
            "oxen reference has an empty revision: `{reference}`"
        ));
    }
    if path.is_empty() {
        return Err(format!("oxen reference has an empty path: `{reference}`"));
    }

    let segs: Vec<&str> = coord.split('/').filter(|s| !s.is_empty()).collect();
    let (host, repo_id) = match segs.as_slice() {
        [ns, repo] => (DEFAULT_OXEN_HOST.to_string(), format!("{ns}/{repo}")),
        [host, ns, repo] => (host.to_string(), format!("{ns}/{repo}")),
        _ => {
            return Err(format!(
                "oxen reference coordinate must be `[<host>/]<namespace>/<repo>`, got `{coord}`"
            ))
        }
    };
    Ok(OxenRef {
        host,
        repo_id,
        revision: revision.to_string(),
        path: path.to_string(),
    })
}

/// The `oxen` binary to invoke. Overridable via `EIGENIUS_OXEN_BIN` (a stub in
/// integration tests, or a non-`PATH` install); defaults to `oxen`.
pub fn oxen_binary() -> String {
    std::env::var("EIGENIUS_OXEN_BIN").unwrap_or_else(|_| "oxen".to_string())
}

/// The scheme to talk to the Oxen host with. Overridable via
/// `EIGENIUS_OXEN_SCHEME` (self-hosted `oxen-server` is often `http`);
/// defaults to `https` (OxenHub).
pub fn oxen_scheme() -> String {
    std::env::var("EIGENIUS_OXEN_SCHEME").unwrap_or_else(|_| "https".to_string())
}

/// Build the `oxen download` argument vector (pure — no I/O), matching the CLI:
/// `oxen download <id> <path> --output <dir> --revision <rev> --host <host>
/// --scheme <scheme>`.
pub fn download_args(oref: &OxenRef, output_dir: &Path, scheme: &str) -> Vec<String> {
    vec![
        "download".to_string(),
        oref.repo_id.clone(),
        oref.path.clone(),
        "--output".to_string(),
        output_dir.to_string_lossy().into_owned(),
        "--revision".to_string(),
        oref.revision.clone(),
        "--host".to_string(),
        oref.host.clone(),
        "--scheme".to_string(),
        scheme.to_string(),
    ]
}

/// Download `oref`'s file into `output_dir` via the `oxen` CLI and return the
/// path it landed at (`output_dir/<basename>` — the CLI names the file by its
/// in-repo basename when `--output` is a directory). Auth flows through the
/// inherited `OXEN_CONFIG_DIR` (see [`write_auth_config`]).
pub fn download_into(oref: &OxenRef, output_dir: &Path) -> Result<PathBuf, RunError> {
    let reference = format!(
        "oxen://{}/{}@{}/{}",
        oref.host, oref.repo_id, oref.revision, oref.path
    );
    let bin = oxen_binary();
    let args = download_args(oref, output_dir, &oxen_scheme());
    let output = std::process::Command::new(&bin)
        .args(&args)
        .output()
        .map_err(|e| RunError::ExternalFetchFailed {
            reference: reference.clone(),
            reason: format!("spawning `{bin}` failed: {e} (is the oxen CLI installed?)"),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RunError::ExternalFetchFailed {
            reference,
            reason: format!(
                "`oxen download` exited with {}: {}",
                output.status,
                stderr.trim()
            ),
        });
    }
    let landed = output_dir.join(oref.basename());
    if !landed.exists() {
        return Err(RunError::ExternalFetchFailed {
            reference,
            reason: format!(
                "`oxen download` reported success but {} is missing",
                landed.display()
            ),
        });
    }
    Ok(landed)
}

/// Render an `auth_config.toml` (the oxen CLI's auth file schema) granting
/// `token` for `host`. Pure — the caller writes it under the substrate-owned
/// config dir.
pub fn render_auth_config_toml(host: &str, token: &str) -> String {
    // Matches liboxen's `AuthConfig` serde shape: a `default_host` plus an
    // array-of-tables `[[host_configs]]` with `host` / `auth_token`.
    format!(
        "default_host = \"{host}\"\n\n[[host_configs]]\nhost = \"{host}\"\nauth_token = \"{token}\"\n"
    )
}

/// Write an `auth_config.toml` into `config_dir` (created if absent) so a later
/// `oxen` subprocess run with `OXEN_CONFIG_DIR=<config_dir>` authenticates to
/// `host`. The deployment calls this from its secret store; the token never
/// reaches a worker image (D53 §10).
pub fn write_auth_config(config_dir: &Path, host: &str, token: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(
        config_dir.join("auth_config.toml"),
        render_auth_config_toml(host, token),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_host() {
        let r = parse("oxen://ox/CatDogBBox@main/annotations/train.csv").unwrap();
        assert_eq!(r.host, DEFAULT_OXEN_HOST);
        assert_eq!(r.repo_id, "ox/CatDogBBox");
        assert_eq!(r.revision, "main");
        assert_eq!(r.path, "annotations/train.csv");
        assert_eq!(r.basename(), "train.csv");
    }

    #[test]
    fn parse_explicit_host_with_port() {
        let r = parse("oxen://0.0.0.0:3000/myns/repo@abc123/data/x.parquet").unwrap();
        assert_eq!(r.host, "0.0.0.0:3000");
        assert_eq!(r.repo_id, "myns/repo");
        assert_eq!(r.revision, "abc123");
        assert_eq!(r.path, "data/x.parquet");
        assert_eq!(r.basename(), "x.parquet");
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(parse("file:///x").is_err());
        assert!(parse("oxen://ox/repo/main/path.csv").is_err()); // no @revision
        assert!(parse("oxen://ox/repo@main").is_err()); // no /path
        assert!(parse("oxen://repo@main/path.csv").is_err()); // 1-segment coord
        assert!(parse("oxen://a/b/c/d@main/path.csv").is_err()); // 4-segment coord
        assert!(parse("oxen://ox/repo@/path.csv").is_err()); // empty revision
    }

    #[test]
    fn download_args_match_cli_shape() {
        let r = parse("oxen://hub.oxen.ai/ox/repo@v1/sub/file.csv").unwrap();
        let args = download_args(&r, Path::new("/tmp/out"), "https");
        assert_eq!(
            args,
            vec![
                "download",
                "ox/repo",
                "sub/file.csv",
                "--output",
                "/tmp/out",
                "--revision",
                "v1",
                "--host",
                "hub.oxen.ai",
                "--scheme",
                "https",
            ]
        );
    }

    #[test]
    fn auth_config_toml_shape() {
        let toml = render_auth_config_toml("hub.oxen.ai", "secret-token");
        assert!(toml.contains("default_host = \"hub.oxen.ai\""));
        assert!(toml.contains("[[host_configs]]"));
        assert!(toml.contains("host = \"hub.oxen.ai\""));
        assert!(toml.contains("auth_token = \"secret-token\""));
    }

    #[test]
    fn write_auth_config_creates_file() {
        let dir = std::env::temp_dir().join(format!("eig_oxen_auth_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_auth_config(&dir, "h", "t").unwrap();
        let contents = std::fs::read_to_string(dir.join("auth_config.toml")).unwrap();
        assert!(contents.contains("auth_token = \"t\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
