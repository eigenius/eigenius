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

//! In-process Lean 4 proof checking. Wraps
//! [`nanoda_lib::util::ExportFile::check_all_declars`] behind a
//! `Verdict`-returning surface so callers don't see nanoda's panic
//! contract.
//!
//! ## Why panic-catch instead of structured errors?
//!
//! nanoda's type-checker reports failure by panicking with a
//! diagnostic string (see `references/nanoda_lib/src/tc.rs`). Until
//! upstream offers a `Result`-returning entry point, we trap the
//! panic with [`std::panic::catch_unwind`] and lift the message into
//! [`Verdict::Fails`]. Per D28 §2.3, nanoda still runs in-process so
//! we keep the small TCB; the catch only handles its current control
//! flow.

use std::panic::{catch_unwind, AssertUnwindSafe};

use nanoda_lib::pretty_printer::PpOptions;
use nanoda_lib::util::Config;

/// Result of checking a Lean export file against a target theorem.
///
/// `Holds` means nanoda accepted every declaration in the export and
/// the target theorem was present; `Fails` means either the export
/// failed to parse, the target name is absent, or the checker
/// rejected at least one declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every declaration type-checks and the target name resolves.
    Holds,
    /// Verification refused the proof. `diagnostic` is the
    /// human-readable message returned by nanoda (panic payload or
    /// parser error). Treated as opaque by callers.
    Fails {
        /// Reason verification refused. Stable enough to log; not a
        /// structured error code (yet — D28 enumerates these for
        /// 20a.4's institution surface).
        diagnostic: String,
    },
}

/// Errors that prevent us from running the checker at all.
/// Distinct from [`Verdict::Fails`], which is "checker ran and said
/// no."
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    /// Could not stage the export bytes into a tempfile that nanoda
    /// can open. Path-only API is a vendor constraint; we hop through
    /// disk on every call until upstream takes a `Reader`.
    #[error("failed to stage export bytes: {0}")]
    TempFile(#[from] std::io::Error),
}

/// Check a `lean4export`-format JSON export for the named theorem.
///
/// `bytes` is the verbatim export-file content (newline-delimited
/// JSON, semver 3.1.x). `target_name` is the fully-qualified Lean
/// name of the theorem to verify — it must be present in the export
/// or the call returns [`Verdict::Fails`]. `permitted_axioms` is the
/// allowlist of axioms the proof may depend on; any axiom outside
/// this list causes [`Verdict::Fails`] (per
/// `unpermitted_axiom_hard_error: true`).
///
/// The function returns `CheckError` only for *infrastructure*
/// failures (cannot create tempfile). Anything the checker has an
/// opinion on — bad parse, missing target, type error — comes back
/// as a `Verdict`.
pub fn check_proof(
    bytes: &[u8],
    target_name: &str,
    permitted_axioms: &[String],
) -> Result<Verdict, CheckError> {
    use std::io::Write;

    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.as_file_mut().write_all(bytes)?;
    tmp.as_file_mut().flush()?;

    let config = Config {
        export_file_path: Some(tmp.path().to_path_buf()),
        use_stdin: false,
        permitted_axioms: Some(permitted_axioms.to_vec()),
        unpermitted_axiom_hard_error: true,
        // Allow Nat + String literal extensions during checking.
        // Any proof against modern Lean stdlib pulls these through
        // the `OfNat` / `OfScientific` instance chain even when the
        // user's source mentions no literals directly (e.g. `0.0`
        // expands to `OfScientific.ofScientific 0 …`). The
        // checker's literal-extension config is a parser knob, not
        // an axiom-acceptance one — turning it on doesn't widen the
        // soundness surface; it's just nanoda's way of saying "the
        // proof carries a primitive literal you didn't pre-declare".
        nat_extension: true,
        string_extension: true,
        // The parser uses `pp_declars` + `unknown_pp_declar_hard_error`
        // as a precondition check: if the export doesn't declare
        // `target_name`, `to_export_file` returns Err. We never
        // actually pretty-print (pp_to_stdout=false, no output path).
        pp_declars: Some(vec![target_name.to_string()]),
        pp_options: PpOptions::default(),
        unknown_pp_declar_hard_error: true,
        pp_output_path: None,
        pp_to_stdout: false,
        num_threads: 1,
        print_success_message: false,
        print_axioms: false,
        unsafe_permit_all_axioms: false,
    };

    let export = match config.to_export_file() {
        Ok((ef, _skipped)) => ef,
        Err(e) => {
            return Ok(Verdict::Fails {
                diagnostic: format!("parse/load: {e}"),
            });
        }
    };

    // `check_all_declars` panics on type errors. `AssertUnwindSafe`
    // is sound here: we discard the `ExportFile` on panic and don't
    // expose any partially-checked state.
    match catch_unwind(AssertUnwindSafe(|| export.check_all_declars())) {
        Ok(()) => Ok(Verdict::Holds),
        Err(p) => Ok(Verdict::Fails {
            diagnostic: panic_payload_to_string(p),
        }),
    }
}

/// Best-effort recovery of a panic message. Rust panics may carry
/// either a `&'static str`, a `String`, or an arbitrary type — the
/// first two cover ~all `panic!()` and `unwrap()` cases.
fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else {
        "<opaque panic payload>".to_string()
    }
}
