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

//! Bytes → `lean:LeanExpr` chain-mirror translator per
//! [D40](../../docs/design/d40-chain-mirrored-lean-expressions.md).
//!
//! Standalone authoring-side utility: takes verbatim `lean4export`
//! bytes plus a target theorem name and emits the theorem's *type*
//! (its proposition) as a `serde_json::Value` tagged-dict tree
//! matching D40 §3's four chain inductives (`lean:LeanName` /
//! `lean:LeanLevel` / `lean:LeanLevelList` / `lean:LeanExpr`).
//!
//! Not on the verification path. Verification operates on the raw
//! bytes via [`crate::check_proof`]; this translator is the bridge
//! between the bytes and the chain-readable `proposition` value on a
//! [`LeanProofTerm`]. Caller decides when to invoke it (typically
//! authoring-time, before committing the resource).
//!
//! ## Soundness boundary
//!
//! Per D40 §1.2 (3), the translator's correctness is *not* required
//! for verification soundness — a buggy translator would produce a
//! wrong-shape `proposition` but the verdict still rides on the
//! verbatim bytes. The chain-mirror discipline is for queries and
//! audits; treating it as load-bearing for re-checking is the
//! soundness hazard D40 explicitly forecloses.

use std::io::Write;

use nanoda_lib::expr::{BinderStyle, Expr};
use nanoda_lib::level::Level;
use nanoda_lib::name::Name;
use nanoda_lib::pretty_printer::PpOptions;
use nanoda_lib::util::{Config, ExprPtr, LevelPtr, LevelsPtr, NamePtr, TcCtx};

use serde_json::{json, Value as JsonValue};

use eigenius_kernel::ontology::resource::Value;

/// Errors the translator surfaces. Distinct from
/// [`crate::CheckError`] / [`crate::Verdict`]: the translator runs
/// outside the verification path, so its failure modes are about
/// shape, not type-checking outcomes.
#[derive(Debug, thiserror::Error)]
pub enum ChainMirrorError {
    /// Couldn't stage the export bytes to a tempfile nanoda can open.
    #[error("failed to stage export bytes: {0}")]
    TempFile(#[from] std::io::Error),

    /// nanoda's parser rejected the bytes. Includes parser diagnostic.
    #[error("nanoda parse failed: {0}")]
    ParseFailed(String),

    /// The declared `target_name` does not appear in the parsed
    /// export environment.
    #[error("target declaration `{0}` not found in export")]
    TargetNotFound(String),

    /// nanoda's parsed tree contains an `Expr::Local`. Closed
    /// committed propositions never contain `Local` (D40 §3.3); if
    /// this fires the export bytes are not closed and shouldn't have
    /// reached this translator.
    #[error("unexpected `Expr::Local` at `{0}` — closed terms only")]
    UnexpectedLocal(String),
}

/// Translate `bytes` (a verbatim `lean4export` JSON export) into a
/// chain `lean:LeanExpr` tagged-dict value for the theorem named
/// `target_name`. The value mirrors the theorem's *type* — its
/// proposition — per D40 §4.1.
///
/// The returned [`Value::Json`] is suitable for direct assignment
/// onto a `LeanProofTerm.proposition` property and will validate
/// against the `lean:LeanExpr` InductiveType once committed.
pub fn bytes_to_lean_expr(bytes: &[u8], target_name: &str) -> Result<Value, ChainMirrorError> {
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.as_file_mut().write_all(bytes)?;
    tmp.as_file_mut().flush()?;

    let config = mirror_config(tmp.path());
    let (export, _skipped) = config
        .to_export_file()
        .map_err(|e| ChainMirrorError::ParseFailed(format!("{e}")))?;

    let json = export.with_ctx(|ctx| -> Result<JsonValue, ChainMirrorError> {
        // Linear scan — the declars map is keyed by NamePtr, not by
        // string, so we render each name and compare. Practical
        // exports have hundreds-to-thousands of declarations; this is
        // O(n) but n is bounded and the work runs once per
        // translation, not on the verification path.
        let mut target_ty: Option<ExprPtr> = None;
        for (name_ptr, declar) in export.declars.iter() {
            if rendered_name(ctx, *name_ptr) == target_name {
                target_ty = Some(declar.info().ty);
                break;
            }
        }
        let target_ty =
            target_ty.ok_or_else(|| ChainMirrorError::TargetNotFound(target_name.to_string()))?;
        encode_expr(ctx, target_ty, "<target>")
    })?;

    Ok(Value::Json(json))
}

/// Construct the nanoda `Config` used by the translator. We don't
/// gate on axioms or pretty-print declarations here — the translator
/// only walks Expr trees, doesn't run the type-checker.
fn mirror_config(path: &std::path::Path) -> Config {
    Config {
        export_file_path: Some(path.to_path_buf()),
        use_stdin: false,
        permitted_axioms: None,
        unpermitted_axiom_hard_error: false,
        // Allow Nat + String literal extensions in the parsed
        // environment. The translator only walks the resulting Expr
        // trees (it never runs nanoda's type-checker), and modern
        // Lean stdlib uses these literal forms freely — even small
        // numeric proofs pull `0` / `1` Nat literals through the
        // `OfScientific` / `OfNat` instance chain. Disabling them
        // would force the translator to reject perfectly ordinary
        // export bytes.
        nat_extension: true,
        string_extension: true,
        pp_declars: None,
        pp_options: PpOptions::default(),
        unknown_pp_declar_hard_error: false,
        pp_output_path: None,
        pp_to_stdout: false,
        num_threads: 1,
        print_success_message: false,
        print_axioms: false,
        unsafe_permit_all_axioms: true,
    }
}

/// Render a nanoda `Name` to its dotted-string form (e.g.
/// `Foo.bar.42`). Used to match the user-supplied `target_name`
/// against the parsed environment's NamePtrs.
fn rendered_name<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, name: NamePtr<'t>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = name;
    loop {
        match ctx.read_name(cur) {
            Name::Anon => break,
            Name::Str(prefix, suffix, _) => {
                let s = ctx.read_string(suffix);
                parts.push(s.as_ref().to_string());
                cur = prefix;
            }
            Name::Num(prefix, suffix, _) => {
                parts.push(suffix.to_string());
                cur = prefix;
            }
        }
    }
    parts.reverse();
    parts.join(".")
}

/// Encode a nanoda `Name` into the `lean:LeanName` tagged-dict shape
/// per D40 §3.1: `Anon` / `Str(prefix, "suffix")` / `Num(prefix, 42)`.
fn encode_name<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, name: NamePtr<'t>) -> JsonValue {
    match ctx.read_name(name) {
        Name::Anon => json!({"ctor": "Anon"}),
        Name::Str(prefix, suffix, _) => {
            let pfx = encode_name(ctx, prefix);
            let sfx = ctx.read_string(suffix).as_ref().to_string();
            json!({"ctor": "Str", "args": [pfx, sfx]})
        }
        Name::Num(prefix, suffix, _) => {
            let pfx = encode_name(ctx, prefix);
            json!({"ctor": "Num", "args": [pfx, suffix as i64]})
        }
    }
}

/// Encode a nanoda `Level` into the `lean:LeanLevel` tagged-dict
/// shape per D40 §3.2.
fn encode_level<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, level: LevelPtr<'t>) -> JsonValue {
    match ctx.read_level(level) {
        Level::Zero => json!({"ctor": "Zero"}),
        Level::Succ(base, _) => {
            let b = encode_level(ctx, base);
            json!({"ctor": "Succ", "args": [b]})
        }
        Level::Max(left, right, _) => {
            let l = encode_level(ctx, left);
            let r = encode_level(ctx, right);
            json!({"ctor": "Max", "args": [l, r]})
        }
        Level::IMax(left, right, _) => {
            let l = encode_level(ctx, left);
            let r = encode_level(ctx, right);
            json!({"ctor": "IMax", "args": [l, r]})
        }
        Level::Param(name, _) => {
            let n = encode_name(ctx, name);
            json!({"ctor": "Param", "args": [n]})
        }
    }
}

/// Encode a nanoda `LevelsPtr` (flat universe-instantiation array)
/// into the `lean:LeanLevelList` cons-list shape per D40 §3.3.
fn encode_levels<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, levels: LevelsPtr<'t>) -> JsonValue {
    let arr = ctx.read_levels(levels);
    let mut out = json!({"ctor": "Nil"});
    for level_ptr in arr.iter().rev() {
        let head = encode_level(ctx, *level_ptr);
        out = json!({"ctor": "Cons", "args": [head, out]});
    }
    out
}

/// Encode a nanoda `Expr` into the `lean:LeanExpr` tagged-dict shape
/// per D40 §3.4. `path` accumulates a structured trail for the
/// `UnexpectedLocal` diagnostic.
fn encode_expr<'t, 'p: 't>(
    ctx: &TcCtx<'t, 'p>,
    expr: ExprPtr<'t>,
    path: &str,
) -> Result<JsonValue, ChainMirrorError> {
    Ok(match ctx.read_expr(expr) {
        Expr::Var { dbj_idx, .. } => {
            json!({"ctor": "Var", "args": [dbj_idx as i64]})
        }
        Expr::Sort { level, .. } => {
            let l = encode_level(ctx, level);
            json!({"ctor": "Sort", "args": [l]})
        }
        Expr::Const { name, levels, .. } => {
            let n = encode_name(ctx, name);
            let ls = encode_levels(ctx, levels);
            json!({"ctor": "Const", "args": [n, ls]})
        }
        Expr::App { fun, arg, .. } => {
            let f = encode_expr(ctx, fun, &format!("{path}.fun"))?;
            let a = encode_expr(ctx, arg, &format!("{path}.arg"))?;
            json!({"ctor": "App", "args": [f, a]})
        }
        Expr::Pi {
            binder_name,
            binder_style,
            binder_type,
            body,
            ..
        } => {
            let bn = encode_name(ctx, binder_name);
            let bs = encode_binder_style(binder_style);
            let bt = encode_expr(ctx, binder_type, &format!("{path}.binder_type"))?;
            let bd = encode_expr(ctx, body, &format!("{path}.body"))?;
            json!({"ctor": "Pi", "args": [bn, bs, bt, bd]})
        }
        Expr::Lambda {
            binder_name,
            binder_style,
            binder_type,
            body,
            ..
        } => {
            let bn = encode_name(ctx, binder_name);
            let bs = encode_binder_style(binder_style);
            let bt = encode_expr(ctx, binder_type, &format!("{path}.binder_type"))?;
            let bd = encode_expr(ctx, body, &format!("{path}.body"))?;
            json!({"ctor": "Lambda", "args": [bn, bs, bt, bd]})
        }
        Expr::Let {
            binder_name,
            binder_type,
            val,
            body,
            nondep,
            ..
        } => {
            let bn = encode_name(ctx, binder_name);
            let bt = encode_expr(ctx, binder_type, &format!("{path}.binder_type"))?;
            let v = encode_expr(ctx, val, &format!("{path}.val"))?;
            let bd = encode_expr(ctx, body, &format!("{path}.body"))?;
            json!({"ctor": "Let", "args": [bn, bt, v, bd, nondep]})
        }
        Expr::Proj {
            ty_name,
            idx,
            structure,
            ..
        } => {
            let tn = encode_name(ctx, ty_name);
            let s = encode_expr(ctx, structure, &format!("{path}.structure"))?;
            json!({"ctor": "Proj", "args": [tn, idx as i64, s]})
        }
        Expr::StringLit { ptr, .. } => {
            let s = ctx.read_string(ptr).as_ref().to_string();
            json!({"ctor": "StringLit", "args": [s]})
        }
        Expr::NatLit { ptr, .. } => {
            // `BigUint` → decimal digit string. The chain spec
            // (D40 §3.4) carries `NatLit.value` as `core:string` to
            // sidestep `i64` overflow on Mathlib-scale literals.
            // `read_bignum` returns `Option<&BigUint>`; absent means
            // the parser cached the literal in a way we can't read,
            // which would be a nanoda regression — surface it as
            // `ParseFailed` rather than silently emitting "0".
            let s = ctx
                .read_bignum(ptr)
                .ok_or_else(|| {
                    ChainMirrorError::ParseFailed(format!(
                        "{path}: NatLit pointer doesn't resolve to a bignum"
                    ))
                })?
                .to_string();
            json!({"ctor": "NatLit", "args": [s]})
        }
        Expr::Local { .. } => {
            return Err(ChainMirrorError::UnexpectedLocal(path.to_string()));
        }
    })
}

/// Map nanoda's `BinderStyle` to the four pinned strings per
/// D40 §3.4 notes: `default` / `implicit` / `strictImplicit` /
/// `instImplicit`.
fn encode_binder_style(style: BinderStyle) -> JsonValue {
    let s = match style {
        BinderStyle::Default => "default",
        BinderStyle::Implicit => "implicit",
        BinderStyle::StrictImplicit => "strictImplicit",
        BinderStyle::InstanceImplicit => "instImplicit",
    };
    JsonValue::String(s.to_string())
}
