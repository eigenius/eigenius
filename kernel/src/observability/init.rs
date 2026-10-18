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
/// - **Stream:** **stderr, always.** Diagnostics must never share a stream with data.
///   `fmt::layer()` defaults to stdout, and several subcommands write their artifact
///   there — `eigenius compile <esl> > out.json` is the demo's own idiom. That worked
///   only because no `info!` happened to fire during a compile; the first one that did
///   (`kernel.validate.memo`, D76 §4.2) turned every compiled document into a stream
///   of log lines followed by the JSON, which `json.load` reports as *"Extra data:
///   line 2 column 1"*. Downgrading the level would have hidden that instance and left
///   the next one to find. `cli/src/main.rs` already states the rule for its own
///   messages: "Note on stderr so the data output stays" clean.
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

    // TTY detection stays on stdout: it is asking "is a human watching", and a
    // redirected artifact with an attached terminal should still format for the human.
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
                .with_writer(std::io::stderr)
                .with_current_span(false)
                .with_span_list(false);
            let _ = tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init();
        }
        // Default to pretty for any other (or unset) value.
        _ => {
            let layer = fmt::layer()
                .pretty()
                .with_writer(std::io::stderr)
                .with_target(true);
            let _ = tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init();
        }
    }
}
