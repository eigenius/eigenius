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

//! Subscriber initialization. Reads `RUST_LOG` for the level filter
//! and `EIGENIUS_LOG_FORMAT` for `json` vs `pretty` output.

use std::io::IsTerminal;

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Install a global tracing subscriber. Call once at process start.
///
/// - **Filter:** taken from `RUST_LOG`. Defaults to `info` if unset.
/// - **Format:** `EIGENIUS_LOG_FORMAT=json` writes one-line JSON
///   suitable for log aggregators; `EIGENIUS_LOG_FORMAT=pretty`
///   writes the human-readable multi-line format. If unset, picks
///   `pretty` when stdout is a TTY and `json` otherwise.
///
/// Idempotent: calling more than once silently no-ops via
/// `try_init`. This means tests that drive the kernel as a library
/// won't fight each other for the global subscriber slot.
pub fn init() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let format = std::env::var("EIGENIUS_LOG_FORMAT").unwrap_or_else(|_| {
        if std::io::stdout().is_terminal() {
            "pretty".to_string()
        } else {
            "json".to_string()
        }
    });

    match format.as_str() {
        "json" => {
            let layer = fmt::layer()
                .json()
                .with_current_span(false)
                .with_span_list(false);
            let _ = tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init();
        }
        // Default to pretty for any other (or unset) value.
        _ => {
            let layer = fmt::layer().pretty().with_target(true);
            let _ = tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init();
        }
    }
}
