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
//! diagnostic string (see `src/tc.rs` in the nanoda_lib git
//! dependency, pinned at rev `6d2f037` in this crate's manifest).
//! Until
//! upstream offers a `Result`-returning entry point, we trap the
//! panic with [`std::panic::catch_unwind`] and lift the message into
//! [`Verdict::Fails`]. Per D28 §2.3, nanoda still runs in-process so
//! we keep the small TCB; the catch only handles its current control
//! flow.

use std::panic::{catch_unwind, AssertUnwindSafe};

use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::term::Exp;
use nanoda_lib::env::EnvLimit;
use nanoda_lib::pretty_printer::PpOptions;
use nanoda_lib::util::{Config, ExportFile};

use crate::externalize;

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
/// `expected` is D74's statement-level check. When `Some`, the claim's own proposition is
/// externalized into the SAME `TcCtx` the export was parsed into and compared to the target's
/// type with nanoda's `def_eq` — so the proof is bound to the claim because the goal was
/// manufactured from the claim (#159). When `None` the call is the pre-D74 name-level check
/// alone: "a theorem called `target_name` type-checks", which does not say what it proves.
///
/// Both sides must share one arena for `def_eq` to be callable at all, which is why this lives
/// inside `check_proof` rather than beside it (D74 §6.2) — the alternative pays a second parse
/// of the same bytes to keep this signature.
///
/// The function returns `CheckError` only for *infrastructure*
/// failures (cannot create tempfile). Anything the checker has an
/// opinion on — bad parse, missing target, type error, a statement
/// that is not the claim's — comes back as a `Verdict`.
pub fn check_proof(
    bytes: &[u8],
    target_name: &str,
    permitted_axioms: &[String],
    expected: Option<&ExpectedStatement<'_>>,
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
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| export.check_all_declars())) {
        return Ok(Verdict::Fails {
            diagnostic: panic_payload_to_string(p),
        });
    }

    // Check 1 passed: the export is internally sound and `target_name` is present. That is all
    // it establishes. D74's check is what relates the named theorem to the claim.
    let Some(expected) = expected else {
        return Ok(Verdict::Holds);
    };

    // Same containment as check 1, for the same reason. nanoda asserts liberally inside
    // `def_eq` — `subst_expr_levels` panics on a universe-arity mismatch rather than returning
    // `false`, and `externalize` cannot rule out every such shape ahead of it. An institution
    // that aborts the process instead of returning a verdict takes the kernel with it.
    match catch_unwind(AssertUnwindSafe(|| {
        check_statement(&export, target_name, expected)
    })) {
        Ok(v) => Ok(v),
        Err(p) => Ok(Verdict::Fails {
            diagnostic: format!(
                "the statement check panicked comparing `{target_name}` against the claim's \
                 proposition: {}",
                panic_payload_to_string(p)
            ),
        }),
    }
}

/// The claim's proposition, and what externalizing it needs.
pub struct ExpectedStatement<'a> {
    /// The claim's `reflection:canonical_proposition`, decoded.
    pub proposition: &'a Exp,
    /// Resolves a chain IRI's `core:short_name` for D30's mangling.
    pub layer: &'a Layer,
}

/// D74 — externalize `expected` and compare it to `target_name`'s type under `def_eq`.
///
/// Runs under `EnvLimit::ByName(target)` via `with_tc_and_declar`, which is the environment
/// nanoda itself checks that declaration under (D74 §6.5): it cuts the environment off AT the
/// declaration, so δ-unfolding cannot reach anything the proof's own check could not.
fn check_statement(
    export: &ExportFile<'_>,
    target_name: &str,
    expected: &ExpectedStatement<'_>,
) -> Verdict {
    // The declars map is keyed by `NamePtr`, an interning handle, so finding one by spelling
    // means rendering each — the same scan the parser's `pp_declars` precondition already did.
    let info = export.with_ctx(|ctx| {
        export
            .declars
            .iter()
            .find(|(n, _)| externalize::render_name(ctx, **n) == target_name)
            .map(|(_, d)| *d.info())
    });
    let Some(info) = info else {
        // Unreachable in practice: `unknown_pp_declar_hard_error` already failed the parse.
        return Verdict::Fails {
            diagnostic: format!("`{target_name}` is not declared in the export"),
        };
    };

    // Build the goal in the ctx, then compare inside `with_tc` — `TypeChecker::ctx` is private,
    // so the expression cannot be built from inside the closure. `ExprPtr` is an arena handle, so
    // carrying it in costs nothing.
    export.with_ctx(|ctx| {
        let uparams: Vec<String> = ctx
            .read_levels(info.uparams)
            .iter()
            .filter_map(|l| match ctx.read_level(*l) {
                nanoda_lib::level::Level::Param(n, _) => Some(externalize::render_name(ctx, n)),
                _ => None,
            })
            .collect();

        // The comparison and any nested inference run under the same environment (§6.5).
        let target_index = export.declars.get_index_of(&info.name).unwrap_or(0);
        let declared: Vec<_> = export.declars.keys().copied().collect();
        let names = externalize::NameTable::build(ctx, &declared);

        // Universe arity per declaration. A `Const` whose level list does not match makes
        // nanoda's `subst_expr_levels` assert — a panic inside `def_eq`, not a `false` — so
        // externalization needs the arities up front (D74 §6.5).
        let arities: std::collections::HashMap<_, _> = export
            .declars
            .iter()
            .map(|(n, d)| (*n, ctx.read_levels(d.info().uparams).len()))
            .collect();

        let goal = match externalize::externalize(
            expected.proposition,
            ctx,
            &names,
            expected.layer,
            &uparams,
            &arities,
            target_index,
        ) {
            Ok(g) => g,
            Err(e) => {
                return Verdict::Fails {
                    diagnostic: format!("cannot externalize the claim's proposition: {e}"),
                }
            }
        };

        // D74 §6.5 — the environment nanoda checks this declaration under, cut off AT it, so
        // δ-unfolding cannot reach anything the proof's own check could not.
        let holds = ctx.with_tc(EnvLimit::ByName(info.name), |tc| tc.def_eq(goal, info.ty));
        if holds {
            Verdict::Holds
        } else {
            Verdict::Fails {
                diagnostic: format!(
                    "`{target_name}` proves a statement that is not the claim's proposition"
                ),
            }
        }
    })
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
