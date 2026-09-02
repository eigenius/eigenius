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
pub fn bytes_to_lean_expr(
    bytes: &[u8],
    target_name: &str,
    layer: &eigenius_kernel::layer::Layer,
) -> Result<Value, ChainMirrorError> {
    // The chain declares `LeanName`, `LeanLevel`, `LeanLevelList` and `LeanExpr`; their
    // constructors' argument names are what a value states (D85 §6.1), so the mirror reads
    // them rather than carrying a copy.
    let names = LeanNames::from_layer(layer);
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.as_file_mut().write_all(bytes)?;
    tmp.as_file_mut().flush()?;

    let config = mirror_config(tmp.path());
    let (export, _skipped) = config
        .to_export_file()
        .map_err(|e| ChainMirrorError::ParseFailed(format!("{e}")))?;

    let value = export.with_ctx(|ctx| -> Result<Value, ChainMirrorError> {
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
        encode_expr(ctx, target_ty, "<target>", &names)
    })?;

    Ok(value)
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
/// The four inductives this mirror writes, and the argument names each constructor declares.
///
/// The values it produces land at `lean:proposition`, which declares
/// `class_types: [lean:LeanExpr]` — so they are value resources whose `is_a` names the
/// constructor's class (D85 §6.1), not tagged dicts. The names come from
/// `lean-expressions.eigon.json` through the chain, so this file does not carry a second copy.
struct LeanNames {
    codec: eigenius_kernel::program::eigentt_type_mirror::CodecNames,
}

impl LeanNames {
    fn from_layer(layer: &eigenius_kernel::layer::Layer) -> Self {
        Self {
            codec: eigenius_kernel::program::eigentt_type_mirror::CodecNames::from_layer(layer),
        }
    }

    fn build(&self, inductive: &str, ctor: &str, args: Vec<Value>) -> Value {
        self.codec.value(inductive, ctor, args).unwrap_or_else(|e| {
            panic!("`{inductive}` must declare `{ctor}` in the chain this mirror runs against: {e}")
        })
    }

    fn name(&self, ctor: &str, args: Vec<Value>) -> Value {
        self.build("urn:eigenius:lean:LeanName", ctor, args)
    }
    fn level(&self, ctor: &str, args: Vec<Value>) -> Value {
        self.build("urn:eigenius:lean:LeanLevel", ctor, args)
    }
    fn levels(&self, ctor: &str, args: Vec<Value>) -> Value {
        self.build("urn:eigenius:lean:LeanLevelList", ctor, args)
    }
    fn expr(&self, ctor: &str, args: Vec<Value>) -> Value {
        self.build("urn:eigenius:lean:LeanExpr", ctor, args)
    }
}

fn encode_name<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, name: NamePtr<'t>, names: &LeanNames) -> Value {
    match ctx.read_name(name) {
        Name::Anon => names.name("Anon", vec![]),
        Name::Str(prefix, suffix, _) => {
            let pfx = encode_name(ctx, prefix, names);
            let sfx = ctx.read_string(suffix).as_ref().to_string();
            names.name("Str", vec![pfx, Value::String(sfx)])
        }
        Name::Num(prefix, suffix, _) => {
            let pfx = encode_name(ctx, prefix, names);
            names.name("Num", vec![pfx, Value::Integer(suffix as i64)])
        }
    }
}

/// Encode a nanoda `Level` into the `lean:LeanLevel` tagged-dict
/// shape per D40 §3.2.
fn encode_level<'t, 'p: 't>(ctx: &TcCtx<'t, 'p>, level: LevelPtr<'t>, names: &LeanNames) -> Value {
    match ctx.read_level(level) {
        Level::Zero => names.level("Zero", vec![]),
        Level::Succ(base, _) => {
            let b = encode_level(ctx, base, names);
            names.level("Succ", vec![b])
        }
        Level::Max(left, right, _) => {
            let l = encode_level(ctx, left, names);
            let r = encode_level(ctx, right, names);
            names.level("Max", vec![l, r])
        }
        Level::IMax(left, right, _) => {
            let l = encode_level(ctx, left, names);
            let r = encode_level(ctx, right, names);
            names.level("IMax", vec![l, r])
        }
        Level::Param(name, _) => {
            let nm = encode_name(ctx, name, names);
            names.level("Param", vec![nm])
        }
    }
}

/// Encode a nanoda `LevelsPtr` (flat universe-instantiation array)
/// into the `lean:LeanLevelList` cons-list shape per D40 §3.3.
fn encode_levels<'t, 'p: 't>(
    ctx: &TcCtx<'t, 'p>,
    levels: LevelsPtr<'t>,
    names: &LeanNames,
) -> Value {
    let arr = ctx.read_levels(levels);
    let mut out = names.levels("Nil", vec![]);
    for level_ptr in arr.iter().rev() {
        let head = encode_level(ctx, *level_ptr, names);
        out = names.levels("Cons", vec![head, out]);
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
    names: &LeanNames,
) -> Result<Value, ChainMirrorError> {
    Ok(match ctx.read_expr(expr) {
        Expr::Var { dbj_idx, .. } => names.expr("Var", vec![Value::Integer(dbj_idx as i64)]),
        Expr::Sort { level, .. } => {
            let l = encode_level(ctx, level, names);
            names.expr("Sort", vec![l])
        }
        Expr::Const { name, levels, .. } => {
            let n = encode_name(ctx, name, names);
            let ls = encode_levels(ctx, levels, names);
            names.expr("Const", vec![n, ls])
        }
        Expr::App { fun, arg, .. } => {
            let f = encode_expr(ctx, fun, &format!("{path}.fun"), names)?;
            let a = encode_expr(ctx, arg, &format!("{path}.arg"), names)?;
            names.expr("App", vec![f, a])
        }
        Expr::Pi {
            binder_name,
            binder_style,
            binder_type,
            body,
            ..
        } => {
            let bn = encode_name(ctx, binder_name, names);
            let bs = encode_binder_style(binder_style);
            let bt = encode_expr(ctx, binder_type, &format!("{path}.binder_type"), names)?;
            let bd = encode_expr(ctx, body, &format!("{path}.body"), names)?;
            names.expr("Pi", vec![bn, bs, bt, bd])
        }
        Expr::Lambda {
            binder_name,
            binder_style,
            binder_type,
            body,
            ..
        } => {
            let bn = encode_name(ctx, binder_name, names);
            let bs = encode_binder_style(binder_style);
            let bt = encode_expr(ctx, binder_type, &format!("{path}.binder_type"), names)?;
            let bd = encode_expr(ctx, body, &format!("{path}.body"), names)?;
            names.expr("Lambda", vec![bn, bs, bt, bd])
        }
        Expr::Let {
            binder_name,
            binder_type,
            val,
            body,
            nondep,
            ..
        } => {
            let bn = encode_name(ctx, binder_name, names);
            let bt = encode_expr(ctx, binder_type, &format!("{path}.binder_type"), names)?;
            let v = encode_expr(ctx, val, &format!("{path}.val"), names)?;
            let bd = encode_expr(ctx, body, &format!("{path}.body"), names)?;
            names.expr("Let", vec![bn, bt, v, bd, Value::Boolean(nondep)])
        }
        Expr::Proj {
            ty_name,
            idx,
            structure,
            ..
        } => {
            let tn = encode_name(ctx, ty_name, names);
            let s = encode_expr(ctx, structure, &format!("{path}.structure"), names)?;
            names.expr("Proj", vec![tn, Value::Integer(idx as i64), s])
        }
        Expr::StringLit { ptr, .. } => {
            let s = ctx.read_string(ptr).as_ref().to_string();
            names.expr("StringLit", vec![Value::String(s)])
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
            names.expr("NatLit", vec![Value::String(s)])
        }
        Expr::Local { .. } => {
            return Err(ChainMirrorError::UnexpectedLocal(path.to_string()));
        }
    })
}

/// Map nanoda's `BinderStyle` to the four pinned strings per
/// D40 §3.4 notes: `default` / `implicit` / `strictImplicit` /
/// `instImplicit`.
fn encode_binder_style(style: BinderStyle) -> Value {
    let s = match style {
        BinderStyle::Default => "default",
        BinderStyle::Implicit => "implicit",
        BinderStyle::StrictImplicit => "strictImplicit",
        BinderStyle::InstanceImplicit => "instImplicit",
    };
    Value::String(s.to_string())
}
