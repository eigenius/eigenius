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

//! ESL compiler: AST → Eigon-JSON resources.
//!
//! Walks the AST and produces a Vec<Resource> that can be
//! serialized to Eigon-JSON or loaded directly into the kernel.
//! Namespace aliases are resolved to full IRIs.

use crate::esl::ast;
use crate::esl::error::{EslError, Position};
use crate::nbe::term::{Exp, Patt};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::collections::BTreeMap;

/// Compile an ESL AST to Eigon-JSON resources.
/// Lower an ESL level expression to a kernel [`Level`](crate::nbe::level::Level) (eigenius#188).
///
/// `Add(l, n)` becomes `n` iterated `Succ`s, which is what `l + n` means; the kernel has no
/// offset form because `Succ` composes.
fn lower_level(
    l: &ast::LevelExpr,
    declared: &std::collections::BTreeSet<String>,
    pos: &crate::esl::error::Position,
) -> Result<crate::nbe::level::Level, EslError> {
    use crate::nbe::level::Level;
    Ok(match l {
        ast::LevelExpr::Num(n) => Level::of_nat(*n),
        ast::LevelExpr::Var(v) => {
            if !declared.contains(v) {
                return Err(EslError::compiler(
                    Some(pos.clone()),
                    format!(
                        "universe level `{v}` is not declared — add `universe {v};` to this file. \
                         An undeclared level variable is not auto-bound (eigenius#188): silently \
                         minting one turns a typo into a second, unrelated universe."
                    ),
                ));
            }
            Level::Param(v.clone())
        }
        ast::LevelExpr::Add(inner, n) => {
            (0..*n).fold(lower_level(inner, declared, pos)?, |acc, _| acc.succ())
        }
        ast::LevelExpr::Max(a, b) => Level::Max(
            Box::new(lower_level(a, declared, pos)?),
            Box::new(lower_level(b, declared, pos)?),
        ),
        ast::LevelExpr::IMax(a, b) => Level::IMax(
            Box::new(lower_level(a, declared, pos)?),
            Box::new(lower_level(b, declared, pos)?),
        ),
    })
}

/// The kernel level a `SortKind` denotes. `Prop` is `0`, `Set` is `1`, `Type l` is `l + 1`
/// (Lean's numbering), and `Sort l` is `l`.
fn sort_kind_level(
    k: &ast::SortKind,
    declared: &std::collections::BTreeSet<String>,
    pos: &crate::esl::error::Position,
) -> Result<crate::nbe::level::Level, EslError> {
    use crate::nbe::level::Level;
    Ok(match k {
        ast::SortKind::Prop => Level::of_nat(0),
        ast::SortKind::Set => Level::of_nat(1),
        ast::SortKind::Type(l) => lower_level(l, declared, pos)?.succ(),
        ast::SortKind::Sort(l) => lower_level(l, declared, pos)?,
    })
}

/// A declaration's own sort, as the `core:Level` value `core:result_sort` now carries
/// (eigenius#188).
///
/// This was a string — `"Prop"` / `"Set"` / `"Type:N"` — which could not express a level VARIABLE,
/// so `data X : Sort u` had to be rejected and nothing validated the string's shape. Emitting the
/// same `core:Level` tree every other level uses removes both problems: one representation, and
/// the validator checks it against the ctor schema like any other inductive value.
/// The level variables an `ast::LevelExpr` mentions, appended in first-mention
/// order and without duplicates.
fn level_expr_params(l: &ast::LevelExpr, out: &mut Vec<String>) {
    match l {
        ast::LevelExpr::Num(_) => {}
        ast::LevelExpr::Var(v) => {
            if !out.iter().any(|x| x == v) {
                out.push(v.clone());
            }
        }
        ast::LevelExpr::Add(inner, _) => level_expr_params(inner, out),
        ast::LevelExpr::Max(a, b) | ast::LevelExpr::IMax(a, b) => {
            level_expr_params(a, out);
            level_expr_params(b, out);
        }
    }
}

/// The level variables a `SortKind` mentions.
fn sort_kind_params(k: &ast::SortKind, out: &mut Vec<String>) {
    match k {
        ast::SortKind::Prop | ast::SortKind::Set => {}
        ast::SortKind::Type(l) | ast::SortKind::Sort(l) => level_expr_params(l, out),
    }
}

fn sort_kind_result_value(
    k: &ast::SortKind,
    declared: &std::collections::BTreeSet<String>,
    pos: &crate::esl::error::Position,
) -> Result<Value, EslError> {
    Ok(Value::Json(
        crate::program::eigentt_type_mirror::encode_level_json(&sort_kind_level(k, declared, pos)?),
    ))
}

/// A resolved IRI as a `ConstRef` value, for sites that already hold the string.
fn const_ref_value(iri: &str) -> Value {
    Value::Json(serde_json::json!({"ctor": "ConstRef", "args": [iri]}))
}

/// A bare type-parameter name as a `Var` value.
fn var_value(name: &str) -> Value {
    Value::Json(serde_json::json!({"ctor": "Var", "args": [name]}))
}

/// A bare (unqualified) kind name as a `Term` value: a reference to a type parameter in scope.
///
/// `Size` used to be the one bare name that was not a parameter reference — it was the size
/// sort, emitted as `SizeSort`. Sized types were removed by eigenius#218, and the decoder's
/// `SizeSort` arm went with them, so the compiler was still producing a constructor nothing
/// could read.
fn bare_kind_value(name: &str) -> Value {
    var_value(name)
}

pub fn compile_file(file: &ast::File) -> Result<Vec<Resource>, Vec<EslError>> {
    compile_file_with_institutions(file, None)
}

/// Compile an ESL AST with access to an [`InstitutionIndex`]. When
/// provided, function-call-shaped references whose function IRI
/// classifies as a registered Decidable QueryClass or a declared
/// Comorphism are emitted as specialized program resources (decoded
/// by `program::expr` into the corresponding kernel AST node). When
/// absent, all function calls emit plain `Apply` resources.
///
/// [`InstitutionIndex`]: crate::institution::registry::InstitutionIndex
pub fn compile_file_with_institutions(
    file: &ast::File,
    institutions: Option<std::sync::Arc<crate::institution::registry::InstitutionIndex>>,
) -> Result<Vec<Resource>, Vec<EslError>> {
    compile_file_with_context(file, institutions, CtorSeed::default(), BTreeMap::new())
}

/// Compile an ESL AST with institution context plus external ctor
/// and macro table seeds. The external maps cover chain-resident
/// inductives and `macro` declarations that the current file does
/// not redeclare — typically produced from
/// [`collect_ctors_from_layer`] and [`collect_macros_from_layer`]
/// walks of the layer the user file is being committed against.
///
/// Without these seeds, cross-file references (e.g.
/// `justification:Certificate`'s ctors used in a sentence, or a
/// `stats:IID(...)` macro called in a fixture) resolve only against
/// decls in the current file. With them, child files cite parent-
/// layer ctors and macros without re-declaring.
pub fn compile_file_with_context(
    file: &ast::File,
    institutions: Option<std::sync::Arc<crate::institution::registry::InstitutionIndex>>,
    external_ctors: CtorSeed,
    external_macros: BTreeMap<String, ast::MacroDecl>,
) -> Result<Vec<Resource>, Vec<EslError>> {
    let mut compiler = Compiler::new();
    compiler.institutions = institutions;
    compiler.ctors_by_iri = external_ctors.by_iri;
    compiler.ctors_by_short_name = external_ctors.by_short_name;
    compiler.macros = external_macros;

    // Register namespace aliases.
    for ns in &file.namespaces {
        compiler.namespaces.insert(ns.alias.clone(), ns.uri.clone());
    }

    // Register level variables (eigenius#188). File-scoped like namespaces.
    //
    // A duplicate is REJECTED, not absorbed (eigenius#219). `declared_universes` is a set, so
    // `universe u u;` and `universe u; universe u;` both used to insert twice and compile — the
    // second insert silently did nothing. nanoda asserts the same thing at declaration admission
    // (`no_dupes_all_params`, `references/nanoda_lib/src/tc.rs:167`), where the stakes are higher:
    // its `uparams` is a per-declaration ORDERED LIST used for level substitution, and a duplicate
    // there makes substitution ambiguous. Here it is only redundant. It is still a mistake, and
    // slice 5c added the `universe` form without the companion check.
    let mut universe_errors: Vec<EslError> = Vec::new();
    for u in &file.universes {
        for n in &u.names {
            if !compiler.declared_universes.insert(n.clone()) {
                universe_errors.push(EslError::compiler(
                    Some(u.pos.clone()),
                    format!(
                        "level variable `{n}` is declared more than once — a `universe` \
                         declaration introduces each name exactly once"
                    ),
                ));
            }
        }
    }
    if !universe_errors.is_empty() {
        return Err(universe_errors);
    }

    // First pass: collect every declared inductive constructor in the
    // current file. Adds to (and may shadow) the external seed; ctor
    // conflicts within the current file are caught here.
    if let Err(e) = compiler.collect_ctor_table(file) {
        return Err(vec![e]);
    }

    // D52 §12 — collect every `macro` declaration in the file so
    // `Value::MacroCall` expansion can resolve forward references
    // (a macro declared later in the file referenced earlier). Adds
    // to (and may shadow) the external seed, matching the ctor pattern.
    if let Err(e) = compiler.collect_macro_table(file) {
        return Err(vec![e]);
    }

    let mut errors = Vec::new();
    let mut resources = Vec::new();

    for decl in &file.declarations {
        match compiler.compile_declaration(decl) {
            Ok(mut rs) => resources.append(&mut rs),
            Err(e) => errors.push(e),
        }
    }

    if errors.is_empty() {
        Ok(resources)
    } else {
        Err(errors)
    }
}

/// Ctor seed harvested from a layer chain: every chain-resident
/// inductive's constructors, indexed both by full IRI (for qualified
/// references) and by short name (for unqualified references plus
/// ambiguity detection).
///
/// Both indices accumulate across the entire chain — no first-wins
/// shadowing. When two chain-resident inductives in different
/// namespaces declare a ctor with the same short name (e.g.
/// `eigentt:Term.App` and `justification:Term.App`),
/// both land in `by_short_name[name]`. The ESL surface's bare-name
/// lookup turns that into an "ambiguous — qualify as one of [...]"
/// error rather than picking one silently.
#[derive(Debug, Default, Clone)]
pub struct CtorSeed {
    pub by_iri: std::collections::BTreeSet<String>,
    pub by_short_name: BTreeMap<String, Vec<String>>,
}

/// Walk a layer chain and collect every chain-resident inductive's
/// constructors into a [`CtorSeed`] suitable for seeding an ESL
/// compile via [`compile_file_with_context`]. Mirrors the same
/// `parent_iri:ctor_name` IRI convention `collect_ctor_table` uses
/// for in-file ctors.
pub fn collect_ctors_from_layer(layer: &crate::layer::Layer) -> CtorSeed {
    use crate::ontology::iri::Iri;
    use crate::ontology::well_known as wk;
    let mut out = CtorSeed::default();
    let ctor_name_iri = match Iri::parse(wk::CTOR_NAME) {
        Ok(i) => i,
        Err(_) => return out,
    };
    let ctors_iri = match Iri::parse(wk::CTORS) {
        Ok(i) => i,
        Err(_) => return out,
    };
    // D23 scaling: discover `InductiveType` resources via `resolve_typed_resources`
    // (triple index for stored layers + `pending` for freshly-built ones) instead of
    // materialising the whole chain. O(inductive types), not O(chain) — the difference
    // between a fast ESL compile and a multi-second one on a large knowledge-graph
    // chain. The in-flight (`pending`) pass is what makes this safe during bootstrap,
    // where `compile_full` runs against not-yet-stored layers (e.g. `lexicon:Cat`
    // while compiling `closed-class.esl`).
    for resource in crate::layer::resolve_typed_resources(layer, &[wk::INDUCTIVE_TYPE]) {
        let Some(parent_iri) = resource.id().cloned() else {
            continue;
        };
        let ctors = match resource.get(&ctors_iri) {
            Some(Value::Array(a)) => a,
            _ => continue,
        };
        for ctor_value in ctors {
            let ctor_resource = match ctor_value {
                Value::Embedded(r) => r.as_ref(),
                _ => continue,
            };
            let name = match ctor_resource.get(&ctor_name_iri) {
                Some(Value::String(s)) => s.clone(),
                _ => continue,
            };
            let ctor_iri = format!("{parent_iri}:{}", ctor_value_short_name(ctor_resource));
            if out.by_iri.insert(ctor_iri.clone()) {
                // First time we see this exact ctor IRI; also index it
                // by short name. Duplicate IRIs (same ctor visible via
                // a merged-view walk that hits two layers carrying it)
                // are deduplicated by `by_iri.insert` returning false.
                let bucket = out.by_short_name.entry(name).or_default();
                if !bucket.contains(&ctor_iri) {
                    bucket.push(ctor_iri);
                }
            }
        }
    }
    out
}

fn ctor_value_short_name(ctor_resource: &Resource) -> String {
    use crate::ontology::iri::Iri;
    use crate::ontology::well_known as wk;
    ctor_resource
        .get(&Iri::parse(wk::CTOR_NAME).expect("static IRI"))
        .and_then(|v| {
            if let Value::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// D52 §12 cross-file macros — walk a layer chain and re-hydrate every
/// `core:Macro` resource's `MacroDecl` into a `full-IRI → decl` table
/// suitable for seeding an ESL compile via [`compile_file_with_context`].
/// Counterpart to [`collect_ctors_from_layer`] for macros.
///
/// First-wins on IRI collisions (top-of-chain layers shadow ancestors
/// in the merged-view walk). Malformed macro resources — missing
/// `macro_decl_json` or with a payload that doesn't deserialize as a
/// `MacroDecl` — are silently skipped: the chain shouldn't crash at
/// `compile_against_layer` time just because a stray malformed
/// resource exists, and the consuming file's expansion site will
/// surface a clean "macro not declared" diagnostic if the skip
/// matters. (The producing-file compile would have already caught
/// genuine authoring errors.)
pub fn collect_macros_from_layer(layer: &crate::layer::Layer) -> BTreeMap<String, ast::MacroDecl> {
    use crate::ontology::iri::Iri;
    use crate::ontology::well_known as wk;
    let mut out: BTreeMap<String, ast::MacroDecl> = BTreeMap::new();
    let decl_json_iri = match Iri::parse(wk::MACRO_DECL_JSON) {
        Ok(i) => i,
        Err(_) => return out,
    };
    // D23 scaling: discover `core:Macro` resources via `resolve_typed_resources`
    // (index for stored + `pending` for in-flight), not a full-chain scan — O(macros),
    // not O(chain). See `collect_ctors_from_layer`.
    for resource in crate::layer::resolve_typed_resources(layer, &[wk::MACRO]) {
        let Some(iri_key) = resource.id().cloned() else {
            continue;
        };
        let decl_json = match resource.get(&decl_json_iri) {
            Some(Value::Json(j)) => j,
            _ => continue,
        };
        let decl: ast::MacroDecl = match serde_json::from_value(decl_json.clone()) {
            Ok(d) => d,
            Err(_) => continue,
        };
        // First-wins matches the merged-view walk's top-of-chain
        // shadowing for ctors.
        out.entry(iri_key.as_str().to_string()).or_insert(decl);
    }
    out
}

struct Compiler {
    namespaces: BTreeMap<String, String>,
    /// Level variables bound by `universe` declarations in this file (eigenius#188).
    ///
    /// A `Sort u` whose `u` is not in here is an ERROR, not a fresh parameter. Lean's
    /// `autoBound` would silently mint one, which turns a typo — `Sort v` for `Sort u` — into a
    /// second unrelated universe rather than a diagnostic. Level variables are cheap to declare
    /// and expensive to get silently wrong, so the binding is required.
    declared_universes: std::collections::BTreeSet<String>,
    /// Per-file constructor index. Two views over the same set of
    /// chain-resident + in-file ctors:
    ///
    /// - `ctors_by_iri`: the canonical "is this IRI a constructor?" set.
    ///   IRI is the stable identifier (gh #75 extended to the ESL
    ///   surface). Qualified references (`justification:App(...)`) resolve
    ///   the namespace prefix to an IRI and check membership here.
    /// - `ctors_by_short_name`: short name → list of qualifying ctor
    ///   IRIs, for bare-name lookup with ambiguity detection. Two
    ///   inductives that share a ctor short name (e.g.
    ///   `eigentt:Term.App` and `justification:Term.App`)
    ///   are both recorded; a bare `App(...)` reference becomes a hard
    ///   "ambiguous — qualify as one of [...]" error instead of
    ///   silently picking the chain-order-first one.
    ///
    /// Both are built in `collect_ctor_table` (in-file decls) plus
    /// `collect_ctors_from_layer` (chain seed) before any declaration
    /// is compiled.
    ctors_by_iri: std::collections::BTreeSet<String>,
    ctors_by_short_name: BTreeMap<String, Vec<String>>,
    /// D52 §12 — per-file smart-constructor macro table: full macro
    /// IRI → its declaration AST. Built in `collect_macro_table`
    /// before any value is compiled, so `Value::MacroCall` resolution
    /// can find macros declared later in the same file. Macros
    /// disappear at compile time (no resource is emitted); the table
    /// is purely an in-compiler expansion environment.
    macros: BTreeMap<String, ast::MacroDecl>,
    /// Optional institution index — when present, drives
    /// compile-time classification of function-call IRIs as a
    /// Decidable QueryClass call or a Comorphism invocation, emitting
    /// specialized program resources instead of plain `Apply`.
    institutions: Option<std::sync::Arc<crate::institution::registry::InstitutionIndex>>,
}

/// Resolve a function-name reference in an ESL `Apply` to its full
/// IRI, given the compiler's namespace table. Returns `None` if the
/// name has no namespace and contains no `:` (i.e. a truly bare
/// reference that can't be an IRI).
///
/// The ESL parser collapses `ns:local` function references in
/// expression position back into `QualifiedName { namespace: None,
/// name: "ns:local" }`, so this helper splits on the first `:` when
/// the explicit namespace field is absent — symmetric with
/// `compile_ctor_arg_type`'s treatment of bare names.
fn resolve_apply_function(
    namespace: Option<&str>,
    name: &str,
    namespaces: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(ns) = namespace {
        if let Some(uri) = namespaces.get(ns) {
            return Some(format!("{uri}:{name}"));
        }
        return None;
    }
    let (ns_alias, local) = name.split_once(':')?;
    let uri = namespaces.get(ns_alias)?;
    Some(format!("{uri}:{local}"))
}

/// D52 §12 — substitute macro-parameter references in a macro body's
/// `Value` AST with their actual-argument values, returning a new
/// `Value` with substitutions applied.
///
/// Substitution rule: a `Value::Ref` whose qualified name has no
/// namespace and whose local name appears in `env` is replaced by a
/// clone of the corresponding arg `Value`. Everything else is
/// structurally cloned with recursion into compound shapes (`Array`,
/// `Block`, `CtorApp`, `MacroCall`).
///
/// Substitution does *not* descend into `Term` — parameter
/// references inside `type_expr(...)` bodies are not supported in
/// v1 because the Term AST has its own name-resolution scope
/// (bound vs free type-level variables) that would require parallel
/// substitution machinery. Add if a real use case arrives.
fn substitute_in_value(body: &ast::Value, env: &BTreeMap<&str, &ast::Value>) -> ast::Value {
    match body {
        ast::Value::Ref(qn) if qn.namespace.is_none() => {
            if let Some(arg) = env.get(qn.name.as_str()) {
                (*arg).clone()
            } else {
                body.clone()
            }
        }
        ast::Value::Array(items) => {
            ast::Value::Array(items.iter().map(|v| substitute_in_value(v, env)).collect())
        }
        ast::Value::Block(fields) => ast::Value::Block(
            fields
                .iter()
                .map(|f| ast::ResourceField {
                    property: f.property.clone(),
                    value: substitute_in_value(&f.value, env),
                })
                .collect(),
        ),
        ast::Value::CtorApp { ctor, args, pos } => ast::Value::CtorApp {
            ctor: ctor.clone(),
            args: args.iter().map(|v| substitute_in_value(v, env)).collect(),
            pos: pos.clone(),
        },
        ast::Value::MacroCall { name, args, pos } => ast::Value::MacroCall {
            name: name.clone(),
            args: args.iter().map(|v| substitute_in_value(v, env)).collect(),
            pos: pos.clone(),
        },
        // Literals, qualified refs, type expressions: pass through.
        _ => body.clone(),
    }
}

/// Expand all `Term::Alias` forms by substituting each binding's
/// value into the body at the names it introduces. The result is an
/// alias-free `Term` ready for the standard compile passes
/// (`lower_type_expr_to_exp` / `encode_type_expr_to_json` /
/// `compile_type_expr`).
///
/// Substitution rules:
///
/// - `Ref { namespace: None, name, args: [] }` → if `name` is bound
///   in `env`, replace with the bound `Term`. Otherwise leave
///   alone. The empty-args check is intentional: name-with-args is
///   either a chain-resident ctor call (`screen:HasLowIC50(c)`) or a
///   forall-bound variable application (`P(x)`), neither of which an
///   alias should silently capture. Authors who want application
///   sugar bind the fully-applied form.
/// - `Pi` / `Lambda` / `BinderArrow` introduce binders that shadow
///   alias names in their bodies — the binder name is removed from
///   the env when recursing into the body. (Each `Pi`/`Lambda` param
///   shadows from its declaration site onward.)
/// - `Alias { bindings, body }` extends the env sequentially: each
///   later binding is substituted with prior bindings already in env,
///   then added to env for subsequent bindings + the body.
/// - All other variants (`Sort`, `LitString`, `LitInt`, `LitFloat`,
///   `LitBool`, `Arrow`) recurse into their children unchanged.
fn expand_aliases(typ: &ast::Term, env: &BTreeMap<String, ast::Term>) -> ast::Term {
    match typ {
        ast::Term::Unit { .. } => typ.clone(),
        ast::Term::Ref { name, args, pos } => {
            if name.namespace.is_none() && args.is_empty() {
                if let Some(bound) = env.get(&name.name) {
                    return bound.clone();
                }
            }
            ast::Term::Ref {
                name: name.clone(),
                args: args.iter().map(|a| expand_aliases(a, env)).collect(),
                pos: pos.clone(),
            }
        }
        ast::Term::Arrow {
            domain,
            codomain,
            pos,
        } => ast::Term::Arrow {
            domain: Box::new(expand_aliases(domain, env)),
            codomain: Box::new(expand_aliases(codomain, env)),
            pos: pos.clone(),
        },
        ast::Term::Ann { expr, typ, pos } => ast::Term::Ann {
            expr: Box::new(expand_aliases(expr, env)),
            typ: Box::new(expand_aliases(typ, env)),
            pos: pos.clone(),
        },
        ast::Term::BinderArrow {
            name,
            kind,
            body,
            pos,
        } => {
            let mut inner = env.clone();
            inner.remove(name);
            ast::Term::BinderArrow {
                name: name.clone(),
                kind: kind.clone(),
                body: Box::new(expand_aliases(body, &inner)),
                pos: pos.clone(),
            }
        }
        ast::Term::Pi {
            params,
            codomain,
            pos,
        } => {
            let mut inner = env.clone();
            let new_params: Vec<_> = params
                .iter()
                .map(|p| {
                    let new_typ = expand_aliases(&p.typ, &inner);
                    inner.remove(&p.name);
                    ast::TypedParam {
                        name: p.name.clone(),
                        typ: new_typ,
                        pos: p.pos.clone(),
                    }
                })
                .collect();
            ast::Term::Pi {
                params: new_params,
                codomain: Box::new(expand_aliases(codomain, &inner)),
                pos: pos.clone(),
            }
        }
        ast::Term::Sigma { params, body, pos } => {
            let mut inner = env.clone();
            let new_params: Vec<_> = params
                .iter()
                .map(|p| {
                    let new_typ = expand_aliases(&p.typ, &inner);
                    inner.remove(&p.name);
                    ast::TypedParam {
                        name: p.name.clone(),
                        typ: new_typ,
                        pos: p.pos.clone(),
                    }
                })
                .collect();
            ast::Term::Sigma {
                params: new_params,
                body: Box::new(expand_aliases(body, &inner)),
                pos: pos.clone(),
            }
        }
        ast::Term::Lambda { params, body, pos } => {
            let mut inner = env.clone();
            let new_params: Vec<_> = params
                .iter()
                .map(|p| {
                    let new_typ = expand_aliases(&p.typ, &inner);
                    inner.remove(&p.name);
                    ast::TypedParam {
                        name: p.name.clone(),
                        typ: new_typ,
                        pos: p.pos.clone(),
                    }
                })
                .collect();
            ast::Term::Lambda {
                params: new_params,
                body: Box::new(expand_aliases(body, &inner)),
                pos: pos.clone(),
            }
        }
        ast::Term::Alias {
            bindings,
            body,
            pos: _,
        } => {
            let mut inner = env.clone();
            for binding in bindings {
                let substituted = expand_aliases(&binding.value, &inner);
                inner.insert(binding.name.clone(), substituted);
            }
            expand_aliases(body, &inner)
        }
        ast::Term::Sort { .. }
        | ast::Term::LitString { .. }
        | ast::Term::LitInt { .. }
        | ast::Term::LitFloat { .. }
        | ast::Term::LitBool { .. } => typ.clone(),
    }
}

impl Compiler {
    /// **Universe generalisation** — the level variables a `data` declaration
    /// binds, in first-mention order (eigenius#188, D76 Phase E2).
    ///
    /// Walks the three places a declaration can mention a level: its result sort,
    /// its parameter kinds, and its index kinds. A constructor argument cannot
    /// introduce a *new* one — it may only mention a parameter or the declaration's
    /// own sort — so the walk is closed over these three.
    ///
    /// **First-mention order is the instantiation contract.** A reference
    /// substitutes level arguments by position (`Level::subst`), so this order is
    /// what a `ConstRef`'s level list is understood against. A `BTreeSet` would
    /// make it alphabetical, which is arbitrary and would silently permute a
    /// two-parameter declaration's arguments.
    fn declaration_universe_params(&self, decl: &ast::DataDecl) -> Result<Vec<String>, EslError> {
        let mut out: Vec<String> = Vec::new();
        if let Some(sort) = &decl.result_sort {
            sort_kind_params(sort, &mut out);
        }
        for p in &decl.params {
            if let ast::IndexKind::Sort(k) = &p.kind {
                sort_kind_params(k, &mut out);
            }
        }
        for ix in &decl.indices {
            if let ast::IndexKind::Sort(k) = &ix.kind {
                sort_kind_params(k, &mut out);
            }
        }
        // Every name here already passed `lower_level`'s declared-universe check on
        // the way to the emitted `result_sort` / `param_kind`, so an undeclared one
        // cannot reach this point. Asserted rather than re-checked: a second,
        // divergent check is how `decode_param_kind` and `decode_arg_type` came to
        // disagree (eigenius#199).
        debug_assert!(
            out.iter().all(|u| self.declared_universes.contains(u)),
            "generalisation found an undeclared level variable, which `lower_level` should have rejected"
        );
        Ok(out)
    }

    fn new() -> Self {
        Self {
            namespaces: BTreeMap::new(),
            declared_universes: std::collections::BTreeSet::new(),
            ctors_by_iri: std::collections::BTreeSet::new(),
            ctors_by_short_name: BTreeMap::new(),
            macros: BTreeMap::new(),
            institutions: None,
        }
    }

    /// Walk every `data` declaration in the file and register its
    /// constructors in both indices. Each ctor's IRI is derived from
    /// the parent inductive's IRI plus its local name (`urn:…:Nat:succ`).
    ///
    /// Two ctors with the same short name across different parent
    /// inductives are allowed — both go into `ctors_by_short_name[name]`,
    /// and a bare reference must qualify to disambiguate. Two ctors
    /// at the same full IRI is a hard error (would mean the same
    /// inductive declared two ctors with one name, which is malformed).
    fn collect_ctor_table(&mut self, file: &ast::File) -> Result<(), EslError> {
        for decl in &file.declarations {
            if let ast::Declaration::Data(d) = decl {
                let parent_iri = self.resolve(&d.name)?;
                for ctor in &d.ctors {
                    let ctor_iri = format!("{parent_iri}:{}", ctor.name());
                    if !self.ctors_by_iri.insert(ctor_iri.clone()) {
                        return Err(EslError::compiler(
                            Some(ctor.pos().clone()),
                            format!(
                                "constructor `{}` declared twice at IRI `{ctor_iri}`",
                                ctor.name()
                            ),
                        ));
                    }
                    let bucket = self
                        .ctors_by_short_name
                        .entry(ctor.name().to_string())
                        .or_default();
                    if !bucket.contains(&ctor_iri) {
                        bucket.push(ctor_iri);
                    }
                }
            }
        }
        Ok(())
    }

    /// D52 §12 — walk every `macro` declaration in the file and
    /// register it in the macros table keyed by its fully-resolved
    /// IRI. Forward references are supported (a macro declared later
    /// in the file may be called earlier) because expansion happens
    /// during the per-declaration compile pass, after this
    /// collection pass populates the table.
    ///
    /// In-file decls shadow any external-seed entry at the same IRI
    /// (matching the ctor behavior — the current file's declaration
    /// is canonical for the file's compile). Two in-file decls at the
    /// same IRI is an error.
    fn collect_macro_table(&mut self, file: &ast::File) -> Result<(), EslError> {
        let mut declared_in_file: std::collections::BTreeSet<String> = Default::default();
        for decl in &file.declarations {
            if let ast::Declaration::Macro(m) = decl {
                let iri = self.resolve(&m.name)?;
                if !declared_in_file.insert(iri.clone()) {
                    return Err(EslError::compiler(
                        Some(m.pos.clone()),
                        format!("macro `{iri}` is declared twice in this file"),
                    ));
                }
                self.macros.insert(iri, m.clone());
            }
        }
        Ok(())
    }

    /// Resolve a `QualifiedName` to a constructor IRI, if any.
    ///
    /// IRI conventions:
    /// - Surface form (what the author writes): `<ns>:<CtorName>`,
    ///   e.g. `justification:Declared`. This resolves to
    ///   `<ns_uri>:<CtorName>` via the standard namespace table.
    /// - Canonical chain IRI (what `ctors_by_iri` stores):
    ///   `<parent_inductive_iri>:<CtorName>`, e.g.
    ///   `urn:eigenius:justification:Term:Declared`.
    ///
    /// The two never match by string equality, so the resolution
    /// strategy is short-name-based with namespace filtering:
    ///
    /// - **Qualified** `ns:Name` → look up `Name` in `ctors_by_short_name`,
    ///   filter the candidate ctor IRIs to those whose parent IRI
    ///   starts with `ns_uri:`. If exactly one match, use it. The
    ///   namespace prefix is what disambiguates between
    ///   `eigentt:App` (= `eigentt:Term:App`) and `justification:App`
    ///   (= `justification:Term:App`).
    /// - **Bare** `Name` → look up the short name in
    ///   `ctors_by_short_name`. If exactly one ctor IRI matches, use
    ///   it. If two or more, error with an "ambiguous" message that
    ///   lists the candidate IRIs so the author can pick a qualifier.
    ///
    /// Returns `Ok(None)` when the name doesn't match any known ctor
    /// — caller falls through to its non-ctor paths (variable
    /// lookup, EigonClass, etc.).
    fn resolve_ctor_iri(&self, qn: &ast::QualifiedName) -> Result<Option<String>, EslError> {
        // **eigenius#24 — `[ns:]Type:ctor`, the fully-disambiguated form.** Until this,
        // two inductives in one file declaring the same constructor short name could
        // only be told apart by renaming one, which is what the ambiguity errors below
        // told the author to do. Naming the constructor by its type is the right
        // disambiguator because `(inductive, ctor name)` **is** a constructor's
        // identity (D79 §2.2.1) — constructors have no IRI of their own, so there is
        // nothing else it could be qualified by.
        if let Some((type_local, ctor_local)) = qn.name.split_once(':') {
            let bucket = match self.ctors_by_short_name.get(ctor_local) {
                Some(b) => b,
                None => return Ok(None),
            };
            // A candidate matches when its parent's local name is `type_local` and,
            // when a namespace alias was given, the parent lives in that namespace.
            let ns_uri = match &qn.namespace {
                Some(alias) => Some(self.namespaces.get(alias).ok_or_else(|| {
                    EslError::compiler(
                        Some(qn.pos.clone()),
                        format!("unknown namespace alias `{alias}`"),
                    )
                })?),
                None => None,
            };
            let matches: Vec<&String> = bucket
                .iter()
                .filter(|ctor_iri| {
                    let Some((parent, _)) = ctor_iri.rsplit_once(':') else {
                        return false;
                    };
                    let Some((parent_ns, parent_local)) = parent.rsplit_once(':') else {
                        return false;
                    };
                    parent_local == type_local
                        && ns_uri
                            .is_none_or(|u| parent_ns == u || parent.starts_with(&format!("{u}:")))
                })
                .collect();
            return match matches.as_slice() {
                [single] => Ok(Some((*single).clone())),
                [] => Ok(None),
                multiple => Err(EslError::compiler(
                    Some(qn.pos.clone()),
                    format!(
                        "qualified constructor `{}` is ambiguous — more than one inductive named                          `{type_local}` declares `{ctor_local}`: [{}]. Add a namespace prefix.",
                        qn.name,
                        multiple
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                )),
            };
        }
        let bucket = match self.ctors_by_short_name.get(&qn.name) {
            Some(b) => b,
            None => return Ok(None),
        };
        match &qn.namespace {
            Some(ns_alias) => {
                let ns_uri = match self.namespaces.get(ns_alias) {
                    Some(u) => u,
                    None => {
                        return Err(EslError::compiler(
                            Some(qn.pos.clone()),
                            format!("unknown namespace alias `{ns_alias}`"),
                        ));
                    }
                };
                // A ctor IRI matches a `ns:Name` reference iff its
                // parent inductive's IRI lives inside `ns_uri`. The
                // parent IRI is `iri.rsplit_once(':')` (the ctor short
                // name is the trailing segment).
                let prefix = format!("{ns_uri}:");
                let matches: Vec<&String> = bucket
                    .iter()
                    .filter(|ctor_iri| {
                        ctor_iri
                            .rsplit_once(':')
                            .map(|(parent, _)| parent.starts_with(&prefix) || parent == ns_uri)
                            .unwrap_or(false)
                    })
                    .collect();
                match matches.as_slice() {
                    [single] => Ok(Some((*single).clone())),
                    [] => Ok(None),
                    multiple => Err(EslError::compiler(
                        Some(qn.pos.clone()),
                        format!(
                            "qualified constructor `{ns_alias}:{}` is still ambiguous — two or \
                             more inductives in `{ns_uri}` declare a constructor with this short \
                             name: [{}]. The fully-disambiguated form (per-inductive ctor \
                             qualifier) is not yet supported in the surface; rename one of the \
                             ctors as a workaround.",
                            qn.name,
                            multiple
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    )),
                }
            }
            None => match bucket.as_slice() {
                [single] => Ok(Some(single.clone())),
                multiple => Err(EslError::compiler(
                    Some(qn.pos.clone()),
                    format!(
                        "bare constructor reference `{}` is ambiguous — multiple chain-resident \
                         inductives declare a constructor with this short name: [{}]. \
                         Qualify with a namespace prefix to pick one.",
                        qn.name,
                        multiple
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                )),
            },
        }
    }

    /// Construct an `Exp::InductiveCtor` from a ctor short-name + its
    /// resolved IRI (`parent_iri:ctor_name` shape). Used by both the
    /// pre-resolve bare-name ctor lookup and the post-resolve
    /// namespaced lookup in `lower_type_expr_to_exp`; factored out to
    /// keep the two paths from drifting.
    fn emit_ctor_app_from_ctor_iri(
        &self,
        pos: &crate::esl::error::Position,
        ctor_name: &str,
        ctor_iri_str: &str,
        args: &[ast::Term],
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Exp, EslError> {
        // The ctor IRI shape is `parent_iri:ctor_name` — strip the
        // trailing `:<ctor_name>` to recover the parent inductive IRI.
        let parent_iri_str = ctor_iri_str
            .rsplit_once(':')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_else(|| ctor_iri_str.to_string());
        let parent_iri = Iri::parse(&parent_iri_str).map_err(|e| {
            EslError::compiler(
                Some(pos.clone()),
                format!("invalid parent IRI `{parent_iri_str}` for ctor `{ctor_name}`: {e}"),
            )
        })?;
        // D76 Phase B — a constructor reference is `(inductive IRI, ctor name)`.
        // This built a stub declaration whose only real content was that same IRI:
        // per gh #75 the `name` was a diagnostic label and the identity was always
        // the IRI.
        let arg_exps: Result<Vec<Exp>, EslError> = args
            .iter()
            .map(|a| self.lower_type_expr_to_exp(a, scope))
            .collect();
        Ok(Exp::InductiveCtor(
            parent_iri,
            ctor_name.to_string(),
            arg_exps?,
        ))
    }

    /// Lower a `data` / `codata` parameter or index KIND to its `eigentt:Term` value.
    ///
    /// One function for all three telescope sites — `codata` params, `data` params, `data`
    /// indices. They were three copies of this match, and the copies had already drifted: the
    /// `codata` param site called `var_value` where the other two called `bare_kind_value`, so a
    /// `Size`-kinded parameter lowered to `Var("Size")` — a reference to a binder that does not
    /// exist — instead of the size sort. Nothing caught it, because nothing type-checked a
    /// declaration's telescope until `check_inductive_decl_telescopes`. Both the drift and the
    /// sized machinery it turned on are gone (eigenius#218); the lesson is why this is one match.
    ///
    /// A bare name is a reference to an earlier parameter when one is in scope, and otherwise the
    /// size sort or a namespace-resolved IRI. A sort keyword is `Sort(level)`, so `Sort u` works
    /// wherever a kind is written (eigenius#188); this was a canonical STRING — `"Prop"` / `"Set"`
    /// / `"Type:N"` — which could not carry a level variable.
    fn lower_kind(
        &self,
        kind: &ast::IndexKind,
        param_names: &std::collections::HashSet<&str>,
        pos: &crate::esl::error::Position,
    ) -> Result<Value, EslError> {
        Ok(match kind {
            ast::IndexKind::Named(qn) => {
                if qn.namespace.is_none() && param_names.contains(qn.name.as_str()) {
                    var_value(&qn.name)
                } else {
                    const_ref_value(&self.resolve(qn)?)
                }
            }
            ast::IndexKind::Sort(sk) => Value::Json(serde_json::json!({
                "ctor": "Sort",
                "args": [crate::program::eigentt_type_mirror::encode_level_json(
                    &sort_kind_level(sk, &self.declared_universes, pos)?
                )],
            })),
        })
    }

    /// Resolve a qualified name to a full IRI string.
    fn resolve(&self, qn: &ast::QualifiedName) -> Result<String, EslError> {
        match &qn.namespace {
            Some(ns) => match self.namespaces.get(ns) {
                Some(uri) => Ok(format!("{uri}:{}", qn.name)),
                None => Err(EslError::compiler(
                    Some(qn.pos.clone()),
                    format!("unknown namespace alias: '{ns}'"),
                )),
            },
            None => Err(EslError::compiler(
                Some(qn.pos.clone()),
                format!(
                    "bare name '{}' has no namespace — use a qualified name like ns:{}",
                    qn.name, qn.name
                ),
            )),
        }
    }

    /// Resolve a qualified name to an Iri.
    fn resolve_iri(&self, qn: &ast::QualifiedName) -> Result<Iri, EslError> {
        let s = self.resolve(qn)?;
        Iri::parse(&s).map_err(|e| {
            EslError::compiler(Some(qn.pos.clone()), format!("invalid IRI '{s}': {e}"))
        })
    }

    fn compile_declaration(&self, decl: &ast::Declaration) -> Result<Vec<Resource>, EslError> {
        match decl {
            ast::Declaration::Class(c) => self.compile_class(c),
            ast::Declaration::Property(p) => self.compile_property(p),
            ast::Declaration::Resource(r) => self.compile_resource(r),
            ast::Declaration::Program(p) => self.compile_program(p),
            ast::Declaration::Data(d) => self.compile_data(d),
            ast::Declaration::MergeComorphism(mc) => self.compile_merge_comorphism(mc),
            // D52 §12 — macros are pure compile-time expansion
            // machinery, but their declaration ALSO emits a chain
            // resource so child-file compiles can re-hydrate the
            // MacroDecl via `collect_macros_from_layer` (cross-file
            // macro visibility). The expansion still happens at
            // compile time; the chain resource is just the persisted
            // declaration that downstream layers can deserialize.
            ast::Declaration::Macro(m) => self.compile_macro_resource(m),
            // D43 §3.1 — text_index / vector_index lowering to Resource
            // (M2+). M1 lands the AST + parser; the compile stage will
            // synthesise the equivalent `Resource` with class
            // `core:TextIndex` / `core:VectorIndex` once M2 storage
            // substrate work begins.
            ast::Declaration::TextIndex(ti) => Err(EslError::parser(
                Some(ti.pos.clone()),
                "text_index lowering not yet implemented (D43 M2)".to_string(),
            )),
            ast::Declaration::VectorIndex(vi) => Err(EslError::parser(
                Some(vi.pos.clone()),
                "vector_index lowering not yet implemented (D43 M2)".to_string(),
            )),
            ast::Declaration::Axiom(ax) => self.compile_axiom(ax),
            ast::Declaration::Def(d) => self.compile_def(d),
        }
    }

    /// D37 §3.3 / §4.3 — lower a `merge_comorphism <iri> for <class>`
    /// declaration to chain resources.
    ///
    /// **Reference form** (`transformation = <iri>`): emits a single
    /// `MergeComorphism` resource at `<iri>` with `merge_target_class`
    /// + `merge_transformation` populated.
    ///
    /// **Inline form** (`(a, b, opt) => <expr>`): emits two resources:
    /// 1. A synthesised standalone `Lambda` at a content-hash IRI of
    ///    shape `urn:eigenius:auto:lambda:<sha256>`, with
    ///    `program:type = pi a : C, b : C, opt : Option<C> => C`
    ///    materialised from the surrounding `for <class>` clause.
    ///    The compiler folds the three-parameter inline body into
    ///    three nested `Lambda` resources, each carrying the
    ///    appropriate `parameter_type`.
    /// 2. A `MergeComorphism` resource at the declaration's IRI
    ///    pointing at the synthesised lambda.
    ///
    /// The content-hash IRI gives free deduplication via the
    /// anchored-commit cache — re-declaring the same inline body
    /// (regardless of which comorphism's surrounding `for` clause)
    /// hashes to the same lambda IRI and short-circuits the commit.
    fn compile_merge_comorphism(
        &self,
        decl: &ast::MergeComorphismDecl,
    ) -> Result<Vec<Resource>, EslError> {
        let comorphism_iri_str = self.resolve(&decl.name)?;
        let comorphism_iri = Iri::parse(&comorphism_iri_str).map_err(|e| {
            EslError::compiler(
                Some(decl.pos.clone()),
                format!("invalid comorphism IRI '{comorphism_iri_str}': {e}"),
            )
        })?;
        let target_class_str = self.resolve(&decl.target_class)?;
        let target_class_iri = Iri::parse(&target_class_str).map_err(|e| {
            EslError::compiler(
                Some(decl.pos.clone()),
                format!("invalid target class IRI '{target_class_str}': {e}"),
            )
        })?;

        match &decl.body {
            ast::MergeComorphismBody::Reference { transformation, .. } => {
                let transformation_str = self.resolve(transformation)?;
                let transformation_iri = Iri::parse(&transformation_str).map_err(|e| {
                    EslError::compiler(
                        Some(transformation.pos.clone()),
                        format!("invalid transformation IRI '{transformation_str}': {e}"),
                    )
                })?;
                let comorphism = build_merge_comorphism_resource(
                    comorphism_iri,
                    target_class_iri,
                    transformation_iri,
                );
                Ok(vec![comorphism])
            }
            ast::MergeComorphismBody::Inline { params, body, pos } => {
                if params.len() != 3 {
                    return Err(EslError::compiler(
                        Some(pos.clone()),
                        format!(
                            "inline merge_comorphism body must have exactly 3 parameters \
                             (the witness signature is `(a, b, opt) => …`); got {}",
                            params.len()
                        ),
                    ));
                }
                // Synthesise the standalone Lambda resource at the
                // content-hash IRI.
                let synthesised =
                    self.synthesise_witness_lambda(&target_class_iri, params, body, pos)?;
                let synth_iri = synthesised
                    .id()
                    .cloned()
                    .expect("synthesised witness lambda must carry an @id");
                let comorphism =
                    build_merge_comorphism_resource(comorphism_iri, target_class_iri, synth_iri);
                Ok(vec![synthesised, comorphism])
            }
        }
    }

    /// Build the synthesised standalone Lambda resource for an
    /// inline `merge_comorphism` body.
    ///
    /// Shape:
    /// - 3 nested Lambda resources for the (a, b, opt) parameters
    /// - Each Lambda's `parameter_type` populated:
    ///   - parameters 1 and 2: the class `C` (target_class)
    ///   - parameter 3: `Option<C>`
    /// - The outermost Lambda's `program:type` carries the full
    ///   `pi a : C, b : C, opt : Option<C> => C` Pi-term so the
    ///   commit-time validator can verify the body in one shot.
    /// - `@id` set to `urn:eigenius:auto:lambda:<sha256>` of the
    ///   resource's canonical Eigon-CBOR (with `@id` cleared) so
    ///   structurally-identical inline bodies dedupe via the
    ///   anchored-commit cache.
    fn synthesise_witness_lambda(
        &self,
        target_class: &Iri,
        params: &[String],
        body: &ast::Expr,
        pos: &Position,
    ) -> Result<Resource, EslError> {
        use crate::ontology::well_known as wk;
        // Compile the body expression first — the resulting embedded
        // Lambda chain has no `@id` until we attach the content-hash.
        let body_r = self.compile_expr(body)?;

        // Build the parameter types: [C, C, Option<C>].
        let class_value = Value::iri(&target_class.clone());
        let option_arg = {
            let mut ar = Resource::new_embedded();
            set_is_a(&mut ar, wk::INDUCTIVE_ARG_TYPE);
            ar.set(iri(wk::TYPE_NAME), const_ref_value(wk::OPTION));
            ar.set(iri(wk::TYPE_ARGS), Value::Array(vec![class_value.clone()]));
            Value::Embedded(Box::new(ar))
        };
        let param_types = [class_value.clone(), class_value.clone(), option_arg.clone()];

        // Build the Pi-term: `pi a : C, b : C, opt : Option<C> => C`.
        // Nested TypeBinderArrow resources, same shape `Term::Pi`
        // would have produced.
        let mut pi_acc: Value = class_value.clone();
        for (name, kind_value) in params.iter().zip(param_types.iter()).rev() {
            let mut ar = Resource::new_embedded();
            set_is_a(&mut ar, wk::TYPE_BINDER_ARROW);
            ar.set(iri(wk::BINDER_NAME), Value::String(name.clone()));
            ar.set(iri(wk::BINDER_KIND), kind_value.clone());
            ar.set(iri(wk::BINDER_BODY), pi_acc);
            pi_acc = Value::Embedded(Box::new(ar));
        }

        // Wrap the body in 3 nested Lambdas, each carrying its
        // `parameter_type`. The innermost lambda's body is the
        // user-supplied expression; the outermost is the
        // synthesised standalone Lambda resource.
        let mut current: Resource = body_r;
        let n = params.len();
        for i in (0..n).rev() {
            let mut lam = Resource::new_embedded();
            set_is_a(&mut lam, "urn:eigenius:program:Lambda");
            lam.set(
                iri("urn:eigenius:program:parameter"),
                Value::String(params[i].clone()),
            );
            lam.set(
                iri("urn:eigenius:program:parameter_type"),
                param_types[i].clone(),
            );
            lam.set(
                iri("urn:eigenius:program:body"),
                Value::Embedded(Box::new(current)),
            );
            current = lam;
        }

        // Attach the full Pi-type so the commit-time validator can
        // type-check the body against the declared signature in one
        // step rather than walking the parameter chain.
        current.set(iri(wk::PROGRAM_TYPE), pi_acc);

        // Compute the content-hash IRI. The hash is over the
        // resource's canonical Eigon-CBOR with @id cleared — so
        // structurally-identical bodies produce the same IRI
        // regardless of which `merge_comorphism` synthesised them.
        let id = compute_witness_lambda_iri(&current);
        current.set_id(Some(id));
        let _ = pos; // pos retained for future diagnostic surfaces
        Ok(current)
    }

    fn compile_type_expr(
        &self,
        typ: &ast::Term,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Value, EslError> {
        use crate::ontology::well_known as wk;
        // `alias` sugar — expand bindings into the body and recurse.
        // The expanded body is alias-free, so the recursion terminates.
        if let ast::Term::Alias { .. } = typ {
            let expanded = expand_aliases(typ, &BTreeMap::new());
            return self.compile_type_expr(&expanded, scope);
        }
        match typ {
            ast::Term::Unit { pos } => Err(EslError::compiler(
                Some(pos.clone()),
                "the unit value `()` is a TERM, not a type — it is only meaningful inside \
                 `type_expr(...)`"
                    .to_string(),
            )),
            ast::Term::Ref { name, args, .. } => {
                let resolved = if name.namespace.is_none() {
                    let n = name.name.as_str();
                    if scope.contains(n) {
                        n.to_string()
                    } else {
                        self.resolve(name)?
                    }
                } else {
                    self.resolve(name)?
                };
                if args.is_empty() {
                    // Simple Ref — keep the legacy string form so
                    // existing codata resources (and their tests) are
                    // unchanged.
                    Ok(Value::String(resolved))
                } else {
                    let mut ar = Resource::new_embedded();
                    set_is_a(&mut ar, wk::INDUCTIVE_ARG_TYPE);
                    ar.set(iri(wk::TYPE_NAME), const_ref_value(&resolved));
                    let arg_values: Result<Vec<Value>, EslError> = args
                        .iter()
                        .map(|a| self.compile_type_expr(a, scope))
                        .collect();
                    ar.set(iri(wk::TYPE_ARGS), Value::Array(arg_values?));
                    Ok(Value::Embedded(Box::new(ar)))
                }
            }
            ast::Term::Arrow {
                domain, codomain, ..
            } => {
                let mut ar = Resource::new_embedded();
                set_is_a(&mut ar, wk::TYPE_ARROW);
                ar.set(
                    iri(wk::ARROW_DOMAIN),
                    self.compile_type_expr(domain, scope)?,
                );
                ar.set(
                    iri(wk::ARROW_CODOMAIN),
                    self.compile_type_expr(codomain, scope)?,
                );
                Ok(Value::Embedded(Box::new(ar)))
            }
            // A term-level annotation `(e : T)` is a category error in a
            // type-declaration position (codata observation type / inductive ctor
            // arg type). Annotations belong in `type_expr(...)` term slots, which
            // compile via `encode_type_expr_to_json`, not here.
            ast::Term::Ann { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "a type annotation `(e : T)` is not valid in a type-declaration \
                 position; it belongs in a term `type_expr(...)`"
                    .to_string(),
            )),
            ast::Term::BinderArrow {
                name, kind, body, ..
            } => {
                let mut ar = Resource::new_embedded();
                set_is_a(&mut ar, wk::TYPE_BINDER_ARROW);
                ar.set(iri(wk::BINDER_NAME), Value::String(name.clone()));
                let kind_str = if kind.namespace.is_none() && scope.contains(kind.name.as_str()) {
                    kind.name.clone()
                } else {
                    self.resolve(kind)?
                };
                ar.set(iri(wk::BINDER_KIND), Value::String(kind_str));
                // The body sees the binder `name` in scope.
                let mut body_scope = scope.clone();
                body_scope.insert(name.as_str());
                ar.set(
                    iri(wk::BINDER_BODY),
                    self.compile_type_expr(body, &body_scope)?,
                );
                Ok(Value::Embedded(Box::new(ar)))
            }
            // D37 §3.5 — `pi x_1 : T_1, …, x_N : T_N => U`. Lowers
            // to N nested `TypeBinderArrow` resources, each carrying
            // its parameter's name + type. The innermost body is the
            // codomain U. Reuses the existing `TypeBinderArrow`
            // shape rather than introducing a new marker class —
            // the decoder in `kernel/src/program/ground.rs` already
            // produces `Exp::Pi` from a non-size-kind `TypeBinderArrow`,
            // so D37 Pi-types decode through the same path.
            //
            // Parameter types can be arbitrary `Term`s (including
            // parametric types like `Option<A>` whose lowering
            // produces an embedded `InductiveArgType`). The kind
            // slot accepts both string and embedded forms — the
            // decoder dispatches on the value's shape.
            ast::Term::Sigma { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "`exists` (Sigma) is only available inside `type_expr(...)`, which lowers to the \
                 D47 ctor encoding; the resource-shaped type language has no binder for it"
                    .to_string(),
            )),
            ast::Term::Pi {
                params, codomain, ..
            } => {
                // Compile parameter types left-to-right so dependent
                // forms like `pi a : A, b : F<a> => …` see `a` in
                // scope when compiling `F<a>`. Then assemble the
                // nested `TypeBinderArrow` resources right-to-left
                // (the rightmost binder wraps the codomain directly).
                let mut working_scope = scope.clone();
                let mut compiled_kinds: Vec<(String, Value)> = Vec::with_capacity(params.len());
                for p in params {
                    let k = self.compile_type_expr(&p.typ, &working_scope)?;
                    compiled_kinds.push((p.name.clone(), k));
                    working_scope.insert(p.name.as_str());
                }
                let mut acc = self.compile_type_expr(codomain, &working_scope)?;
                for (name, kind_value) in compiled_kinds.into_iter().rev() {
                    let mut ar = Resource::new_embedded();
                    set_is_a(&mut ar, wk::TYPE_BINDER_ARROW);
                    ar.set(iri(wk::BINDER_NAME), Value::String(name));
                    ar.set(iri(wk::BINDER_KIND), kind_value);
                    ar.set(iri(wk::BINDER_BODY), acc);
                    acc = Value::Embedded(Box::new(ar));
                }
                Ok(acc)
            }
            // eigenius#72 — sort literals in type position. For the
            // existing chain-Value-producing paths (Lambda type slots,
            // codata observation types, merge_comorphism transformation
            // signatures), we emit a string representation. None of
            // those paths currently consume sorts structurally; if a
            // future use site needs a richer chain shape we'll extend.
            // The proper Exp-side lowering for `axiom` statements lives
            // in `lower_type_expr_to_exp` (Layer 1) and reads the AST
            // directly, bypassing this chain-Value path.
            ast::Term::Sort { kind, .. } => {
                let s = match kind {
                    ast::SortKind::Prop => "Prop".to_string(),
                    ast::SortKind::Set => "Set".to_string(),
                    ast::SortKind::Type(l) => format!("Type({l})"),
                    ast::SortKind::Sort(l) => format!("Sort({l})"),
                };
                Ok(Value::String(s))
            }
            ast::Term::Lambda { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "`fun (…) => …` is only allowed inside `match … returning <motive>` \
                 motives, axiom statements, and other Exp-encoded contexts — not in \
                 the chain-value type-expression slots (codata observation types, \
                 lambda type slots, etc.). If you reached this from a `returning` \
                 clause, the motive is encoded via the D47 codec instead and this \
                 branch is not exercised."
                    .to_string(),
            )),
            ast::Term::LitString { pos, .. }
            | ast::Term::LitInt { pos, .. }
            | ast::Term::LitFloat { pos, .. }
            | ast::Term::LitBool { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "literal values are not allowed in chain-value type-expression slots \
                 (codata observation types, etc.); they only appear in Exp-encoded \
                 contexts (axiom statements, `type_expr(...)` resource fields, \
                 indexed ctor return types)"
                    .to_string(),
            )),
            // Eliminated by the early-return at the top of this fn.
            ast::Term::Alias { .. } => unreachable!("alias expanded above"),
        }
    }

    // --- Axiom declarations (eigenius#72 Layer 1, D46 §10) ---

    /// Lower an `axiom Name : <type-expr>` declaration to a chain
    /// `core:Axiom` Resource whose `axiom_statement` is the encoded
    /// EigenTT type expression. Goes through the D47 codec
    /// (`encode_type`) after lowering the ESL Term to a kernel
    /// `Exp` via [`Self::lower_type_expr_to_exp`].
    /// D52 §12 cross-file macros — emit a `core:Macro` chain resource
    /// carrying the macro's serialized `MacroDecl` AST. The resource's
    /// IRI is the macro's canonical name (e.g.
    /// `urn:eigenius:measurements:IID`); its `core:macro_decl_json`
    /// property holds the full AST as a `Value::Json` blob (via
    /// `serde_json::to_value` on the `MacroDecl`). Child-file compiles
    /// re-hydrate via [`collect_macros_from_layer`].
    fn compile_macro_resource(&self, decl: &ast::MacroDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);
        r.set(
            iri(crate::ontology::well_known::IS_A),
            Value::Array(vec![Value::String(
                crate::ontology::well_known::MACRO.to_string(),
            )]),
        );
        let decl_json = serde_json::to_value(decl).map_err(|e| {
            EslError::compiler(
                Some(decl.pos.clone()),
                format!("macro `{}` AST serialization failed: {e}", decl.name.name),
            )
        })?;
        r.set(
            iri(crate::ontology::well_known::MACRO_DECL_JSON),
            Value::Json(decl_json),
        );
        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    fn compile_axiom(&self, decl: &ast::AxiomDecl) -> Result<Vec<Resource>, EslError> {
        let empty_scope: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let statement_exp = self.lower_type_expr_to_exp(&decl.statement, &empty_scope)?;
        let encoded =
            crate::program::eigentt_type_mirror::encode_type(&statement_exp).map_err(|e| {
                EslError::compiler(
                    Some(decl.pos.clone()),
                    format!("axiom statement encoding failed: {e}"),
                )
            })?;
        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:eigentt:Axiom".to_string(),
            )]),
        );
        r.set(iri("urn:eigenius:eigentt:axiom_statement"), encoded);
        if let Some(d) = &decl.description {
            r.set(
                iri("urn:eigenius:core:description"),
                Value::String(d.clone()),
            );
        }
        if let Some(j) = &decl.justification {
            r.set(
                iri("urn:eigenius:eigentt:axiom_justification"),
                Value::String(j.clone()),
            );
        }
        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    /// Lower `def ex:F(m : Set, g : Set) : Prop = <body>` to an `eigentt:Definition` (D66).
    ///
    /// The parameters give both stored halves:
    /// - `definition_type` = `Pi (m : Set). Pi (g : Set). Prop`
    /// - `definition_body` = the lambda chain `Lam(m, Lam(g, <body>))`
    ///
    /// Arity and parameter types live only in the type, so a stored arity can never contradict it.
    ///
    /// **The body is stored as written, not normalized here.** D9 requires what is *stored* to be
    /// the normal form of the right-hand side. This compiler satisfies that by not producing a
    /// non-normal body, and Rule 24 refuses any that slips through. Normalizing here would mean
    /// evaluating an open term and reading it back, which renames every binder — and a compiler
    /// silently rewriting an author's body is worse than telling them it contains a redex.
    fn compile_def(&self, decl: &ast::DefDecl) -> Result<Vec<Resource>, EslError> {
        let mut scope: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut binders: Vec<(crate::nbe::term::Patt, Exp)> = Vec::new();
        for p in &decl.params {
            let dom = self.lower_type_expr_to_exp(&p.typ, &scope)?;
            binders.push((crate::nbe::term::Patt::Var(p.name.clone()), dom));
            scope.insert(p.name.as_str());
        }
        let result_exp = self.lower_type_expr_to_exp(&decl.result, &scope)?;
        let body_exp = self.lower_type_expr_to_exp(&decl.body, &scope)?;

        // The declared type: one `Pi` per parameter, ending in the result type.
        let mut type_exp = result_exp;
        for (patt, dom) in binders.iter().rev() {
            type_exp = Exp::Pi(patt.clone(), Box::new(dom.clone()), Box::new(type_exp));
        }

        let encoded_type =
            crate::program::eigentt_type_mirror::encode_type(&type_exp).map_err(|e| {
                EslError::compiler(
                    Some(decl.pos.clone()),
                    format!("definition type encoding failed: {e}"),
                )
            })?;
        // `Exp::Lam` carries no domain slot, so the encoder takes the annotations separately.
        let encoded_body = crate::program::eigentt_type_mirror::encode_lam_chain(
            &binders, &body_exp,
        )
        .map_err(|e| {
            EslError::compiler(
                Some(decl.pos.clone()),
                format!("definition body encoding failed: {e}"),
            )
        })?;

        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:eigentt:Definition".to_string(),
            )]),
        );
        r.set(iri("urn:eigenius:eigentt:definition_type"), encoded_type);
        r.set(iri("urn:eigenius:eigentt:definition_body"), encoded_body);
        if let Some(d) = &decl.description {
            r.set(
                iri("urn:eigenius:core:description"),
                Value::String(d.clone()),
            );
        }
        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    /// eigenius#72 — lower an ESL `Term` to a kernel `Exp`.
    ///
    /// Used by Layer 1's `axiom` declaration (statement encoding) and
    /// Layer 2's indexed `data` ctor result types. Recognises:
    /// - `Sort(...)` → `Exp::Sort(n)` per the Prop/Set/Type mapping.
    /// - `Ref(name, args)` → bound-variable `Exp::Var` for in-scope
    ///   bare names; otherwise resolves the IRI and produces
    ///   `Exp::EigonClass` (nullary) or `Exp::InductiveType` with a
    ///   name-only stub decl (applied — args become the InductiveType's
    ///   args slot, which the D47 codec App-curries on encode and the
    ///   decoder re-folds at use time).
    /// - `Arrow(a, b)` → `Exp::Pi(Patt::Unit, a, b)`.
    /// - `Pi(params, codomain)` → nested `Exp::Pi` chain, threading
    ///   binder names into scope so later params can reference earlier
    ///   ones (dependent telescope).
    /// - `BinderArrow(name, kind, bound, body)` → `Exp::Pi` for non-
    ///   size kinds; sized binders defer to the existing kernel-side
    ///   `Exp::SizedPi` handling but are rare in axiom statements.
    fn lower_type_expr_to_exp(
        &self,
        typ: &ast::Term,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Exp, EslError> {
        // `alias` sugar — expand bindings into the body and recurse.
        if let ast::Term::Alias { .. } = typ {
            let expanded = expand_aliases(typ, &BTreeMap::new());
            return self.lower_type_expr_to_exp(&expanded, scope);
        }
        match typ {
            ast::Term::Unit { .. } => Ok(Exp::Unit),
            ast::Term::Sigma { params, body, .. } => {
                // Nested `Exp::Sig`, rightmost binder innermost — the mirror of `Pi` below.
                let mut working = scope.clone();
                let mut doms = Vec::with_capacity(params.len());
                for p in params {
                    doms.push((
                        p.name.clone(),
                        self.lower_type_expr_to_exp(&p.typ, &working)?,
                    ));
                    working.insert(p.name.as_str());
                }
                let mut acc = self.lower_type_expr_to_exp(body, &working)?;
                for (name, dom) in doms.into_iter().rev() {
                    acc = Exp::Sig(
                        crate::nbe::term::Patt::Var(name),
                        Box::new(dom),
                        Box::new(acc),
                    );
                }
                Ok(acc)
            }
            ast::Term::Sort { kind, pos } => Ok(Exp::Sort(sort_kind_level(
                kind,
                &self.declared_universes,
                pos,
            )?)),
            // Sigma ELIMINATION — see the twin arm in `encode_type_expr_to_json`. Both paths
            // are live: `axiom X : T` lowers through here, `type_expr(...)` in a resource
            // property through the JSON encoder.
            ast::Term::Ref { name, args, .. }
                if args.len() == 1
                    && matches!(
                        self.resolve(name).as_deref(),
                        Ok("urn:eigenius:eigentt:fst") | Ok("urn:eigenius:eigentt:snd")
                    ) =>
            {
                let inner = self.lower_type_expr_to_exp(&args[0], scope)?;
                Ok(if self.resolve(name)?.ends_with(":fst") {
                    Exp::Fst(Box::new(inner))
                } else {
                    Exp::Snd(Box::new(inner))
                })
            }
            ast::Term::Ref { name, args, .. } => {
                let is_bound = name.namespace.is_none() && scope.contains(name.name.as_str());
                if is_bound {
                    // Bound variable: lowers to `Exp::Var`. If args are
                    // present, the user is writing a function-application
                    // shape like `P(x)` where `P : T -> Prop` is a
                    // forall-bound function. Curry into `Exp::App` chain
                    // so EigenTT's NbE can beta-reduce at use time —
                    // required by D39's `justification:Certificate.spec` constructor
                    // whose result type writes `P(t)` for a forall-bound
                    // `P` and `t`.
                    let head = Exp::Var(name.name.clone());
                    if args.is_empty() {
                        return Ok(head);
                    }
                    let mut acc = head;
                    for arg in args {
                        let arg_exp = self.lower_type_expr_to_exp(arg, scope)?;
                        acc = Exp::App(Box::new(acc), Box::new(arg_exp));
                    }
                    return Ok(acc);
                }

                // Bare-name ctor lookup before namespace resolution:
                // `app`, `declared`, `observed`, etc. — references to
                // ctors of in-file or chain-resident inductives (the
                // latter seeded via `compile_against_layer`). Checked
                // *before* `self.resolve(name)` because bare names
                // would otherwise fail namespace resolution and never
                // reach the post-resolve ctor lookup below. Ambiguity
                // (two ctors sharing the short name across inductives)
                // surfaces here as a hard error from `resolve_ctor_iri`.
                if name.namespace.is_none() {
                    if let Some(ctor_iri_str) = self.resolve_ctor_iri(name)? {
                        return self.emit_ctor_app_from_ctor_iri(
                            &name.pos,
                            &name.name,
                            &ctor_iri_str,
                            args,
                            scope,
                        );
                    }
                }

                let iri_str = self.resolve(name)?;
                let iri_val = Iri::parse(&iri_str).map_err(|e| {
                    EslError::compiler(
                        Some(name.pos.clone()),
                        format!("invalid IRI `{iri_str}`: {e}"),
                    )
                })?;

                // Constructor disambiguation: when the qualified name
                // matches a declared ctor (in-file or chain-resident),
                // emit `Exp::InductiveCtor` rather than
                // `Exp::EigonClass` / `InductiveType`. Required for
                // D39 §5 `justification:Certificate.declared : ... ->
                // justification:Certificate(Declared iri) P` and any similar
                // shape where a ctor of one inductive appears in
                // another inductive's index/result-type position.
                //
                // `resolve_ctor_iri` walks `ctors_by_short_name` and
                // filters by namespace prefix, so `justification:App(...)`
                // unambiguously picks the `reasoning` namespace's
                // `App` ctor even when `eigentt:Term:App` shares
                // the short name.
                if let Some(ctor_iri_str) = self.resolve_ctor_iri(name)? {
                    return self.emit_ctor_app_from_ctor_iri(
                        &name.pos,
                        &name.name,
                        &ctor_iri_str,
                        args,
                        scope,
                    );
                }

                if args.is_empty() {
                    Ok(Exp::EigonClass(iri_val))
                } else {
                    // D76 Phase B — the name, applied, which is what the D47 codec
                    // has always produced: `App(App(ConstRef(iri), a1), a2)…`. A
                    // stub declaration was built here only to fill the fused node's
                    // declaration slot before being discarded by the encoder.
                    let arg_exps: Result<Vec<Exp>, EslError> = args
                        .iter()
                        .map(|a| self.lower_type_expr_to_exp(a, scope))
                        .collect();
                    Ok(Exp::const_applied(iri_val.clone(), Vec::new(), arg_exps?))
                }
            }
            ast::Term::Arrow {
                domain, codomain, ..
            } => {
                let dom = self.lower_type_expr_to_exp(domain, scope)?;
                let body = self.lower_type_expr_to_exp(codomain, scope)?;
                Ok(Exp::arrow(dom, body))
            }
            // `(e : T)` — bidirectional annotation → `Exp::Ann`.
            ast::Term::Ann { expr, typ, .. } => {
                let e = self.lower_type_expr_to_exp(expr, scope)?;
                let t = self.lower_type_expr_to_exp(typ, scope)?;
                Ok(Exp::Ann(Box::new(e), Box::new(t)))
            }
            ast::Term::Pi {
                params, codomain, ..
            } => {
                // Dependent telescope: thread each binder into scope
                // before lowering subsequent param types and the body.
                let mut working: std::collections::HashSet<String> =
                    scope.iter().map(|s| s.to_string()).collect();
                let mut compiled_doms: Vec<(String, Exp)> = Vec::with_capacity(params.len());
                for p in params {
                    let local: std::collections::HashSet<&str> =
                        working.iter().map(|s| s.as_str()).collect();
                    let dom = self.lower_type_expr_to_exp(&p.typ, &local)?;
                    compiled_doms.push((p.name.clone(), dom));
                    working.insert(p.name.clone());
                }
                let inner_scope: std::collections::HashSet<&str> =
                    working.iter().map(|s| s.as_str()).collect();
                let mut body = self.lower_type_expr_to_exp(codomain, &inner_scope)?;
                for (name, dom) in compiled_doms.into_iter().rev() {
                    body = Exp::Pi(Patt::Var(name), Box::new(dom), Box::new(body));
                }
                Ok(body)
            }
            ast::Term::BinderArrow {
                name, kind, body, ..
            } => {
                // A binder arrow lowers as a plain Pi. It also had a sized form, lowering to a
                // SizeSort-typed binder with an ignored upper bound; that went with sized types
                // (eigenius#218).
                let kind_str = self.resolve(kind)?;
                let iri_val = Iri::parse(&kind_str).map_err(|e| {
                    EslError::compiler(
                        Some(typ.pos().clone()),
                        format!("invalid kind IRI `{kind_str}`: {e}"),
                    )
                })?;
                let dom = Exp::EigonClass(iri_val);
                let mut inner_scope: std::collections::HashSet<&str> = scope.clone();
                inner_scope.insert(name.as_str());
                let body_exp = self.lower_type_expr_to_exp(body, &inner_scope)?;
                Ok(Exp::Pi(
                    Patt::Var(name.clone()),
                    Box::new(dom),
                    Box::new(body_exp),
                ))
            }
            // eigenius#72 Layer 3 — `fun (i_1 : T_1, …, i_n : T_n) =>
            // body`. Nests N single-parameter `Exp::Lam` chains,
            // threading binder names into scope so later params can
            // reference earlier ones (parallels how Pi lowers).
            // Parameter type annotations are *not* attached to the
            // resulting `Exp::Lam` nodes — EigenTT lambdas are untyped
            // at the term level; the annotation lives in the
            // accompanying Pi when one exists (in motives, the
            // matching `Exp::Pi` is the scrutinee's type signature
            // which the kernel already knows). The ESL surface
            // requires the annotation for readability and to thread
            // the binder into scope during further lowering.
            ast::Term::Lambda { params, body, .. } => {
                let mut working: std::collections::HashSet<String> =
                    scope.iter().map(|s| s.to_string()).collect();
                for p in params {
                    working.insert(p.name.clone());
                }
                let inner_scope: std::collections::HashSet<&str> =
                    working.iter().map(|s| s.as_str()).collect();
                let mut body_exp = self.lower_type_expr_to_exp(body, &inner_scope)?;
                for p in params.iter().rev() {
                    body_exp = Exp::Lam(Patt::Var(p.name.clone()), Box::new(body_exp));
                }
                Ok(body_exp)
            }
            // Literals in type/term position lower to the Phase-2
            // `Exp::Lit*` constructors. Used as arguments to value-
            // indexed inductives (e.g. `Asserts("urn:foo")`,
            // `Vec(3, A)`, etc.) inside `type_expr(...)`.
            ast::Term::LitString { value, .. } => Ok(Exp::LitString(value.clone())),
            ast::Term::LitInt { value, .. } => Ok(Exp::LitInt(*value)),
            ast::Term::LitFloat { value, .. } => Ok(Exp::LitFloat(*value)),
            ast::Term::LitBool { value, .. } => Ok(Exp::LitBool(*value)),
            // Eliminated by the early-return at the top of this fn.
            ast::Term::Alias { .. } => unreachable!("alias expanded above"),
        }
    }

    /// Encode an ESL `Term` directly to the D47 chain-JSON shape,
    /// preserving `fun (x : T) => body` binder-type annotations.
    ///
    /// `lower_type_expr_to_exp` + `encode_type` would otherwise reject
    /// any Lambda: `Exp::Lam` doesn't carry its binder's type, so the
    /// generic encoder has nowhere to recover the annotation from. The
    /// D47 `Lam` ctor expects `[binder_name, dom_json, body_json]` —
    /// we have the dom directly in the AST, so walking the AST is the
    /// natural shape.
    ///
    /// Required by D39's universal-rule certificates: writing the
    /// predicate `P : core:string -> Prop` as `fun (x : core:string)
    /// => HasLowIC50(x) -> StrongInhibitor(x)` inside a `type_expr(...)`
    /// resource property value.
    ///
    /// Cases that can contain nested `Lambda`s (Arrow, Pi, BinderArrow,
    /// Ref with args) recurse here so the annotation survives at any
    /// depth. Leaves with no Lambda exposure (Sort, literals) delegate
    /// to `lower_type_expr_to_exp` + `encode_type`.
    fn encode_type_expr_to_json(
        &self,
        typ: &ast::Term,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<serde_json::Value, EslError> {
        use crate::program::eigentt_type_mirror::encode_type;
        use serde_json::json;
        // `alias` sugar — expand bindings into the body and recurse.
        if let ast::Term::Alias { .. } = typ {
            let expanded = expand_aliases(typ, &BTreeMap::new());
            return self.encode_type_expr_to_json(&expanded, scope);
        }

        // Wrap a leaf Term: lower to Exp, encode via the D47
        // encoder, unwrap to raw JSON. Safe for any subtree whose
        // lowered Exp contains no `Lam`.
        let encode_leaf = |this: &Self, t: &ast::Term| -> Result<serde_json::Value, EslError> {
            let exp = this.lower_type_expr_to_exp(t, scope)?;
            let v = encode_type(&exp).map_err(|e| {
                EslError::compiler(
                    Some(t.pos().clone()),
                    format!("type_expr encoding failed: {e}"),
                )
            })?;
            match v {
                Value::Json(j) => Ok(j),
                other => Err(EslError::compiler(
                    Some(t.pos().clone()),
                    format!("type_expr encoding did not produce JSON: {other:?}"),
                )),
            }
        };

        match typ {
            ast::Term::Unit { .. } => Ok(serde_json::json!({"ctor": "UnitVal", "args": []})),
            ast::Term::Lambda { params, body, .. } => {
                // Mirror the lowering's scope-threading so later params
                // can mention earlier binders. Each dom is encoded
                // against the scope where prior binders are visible.
                let mut working: std::collections::HashSet<String> =
                    scope.iter().map(|s| s.to_string()).collect();
                let mut binder_doms: Vec<(String, serde_json::Value)> =
                    Vec::with_capacity(params.len());
                for p in params {
                    let local: std::collections::HashSet<&str> =
                        working.iter().map(|s| s.as_str()).collect();
                    let dom_json = self.encode_type_expr_to_json(&p.typ, &local)?;
                    binder_doms.push((p.name.clone(), dom_json));
                    working.insert(p.name.clone());
                }
                let inner_scope: std::collections::HashSet<&str> =
                    working.iter().map(|s| s.as_str()).collect();
                let mut acc = self.encode_type_expr_to_json(body, &inner_scope)?;
                for (name, dom) in binder_doms.into_iter().rev() {
                    acc = json!({
                        "ctor": "Lam",
                        "args": [name, dom, acc],
                    });
                }
                Ok(acc)
            }
            ast::Term::Sigma { params, body, .. } => {
                let mut working: std::collections::HashSet<String> =
                    scope.iter().map(|s| s.to_string()).collect();
                let mut binder_doms: Vec<(String, serde_json::Value)> =
                    Vec::with_capacity(params.len());
                for p in params {
                    let local: std::collections::HashSet<&str> =
                        working.iter().map(|s| s.as_str()).collect();
                    binder_doms.push((
                        p.name.clone(),
                        self.encode_type_expr_to_json(&p.typ, &local)?,
                    ));
                    working.insert(p.name.clone());
                }
                let inner_scope: std::collections::HashSet<&str> =
                    working.iter().map(|s| s.as_str()).collect();
                let mut acc = self.encode_type_expr_to_json(body, &inner_scope)?;
                for (name, dom) in binder_doms.into_iter().rev() {
                    acc = json!({ "ctor": "Sig", "args": [name, dom, acc] });
                }
                Ok(acc)
            }
            ast::Term::Pi {
                params, codomain, ..
            } => {
                let mut working: std::collections::HashSet<String> =
                    scope.iter().map(|s| s.to_string()).collect();
                let mut binder_doms: Vec<(String, serde_json::Value)> =
                    Vec::with_capacity(params.len());
                for p in params {
                    let local: std::collections::HashSet<&str> =
                        working.iter().map(|s| s.as_str()).collect();
                    let dom_json = self.encode_type_expr_to_json(&p.typ, &local)?;
                    binder_doms.push((p.name.clone(), dom_json));
                    working.insert(p.name.clone());
                }
                let inner_scope: std::collections::HashSet<&str> =
                    working.iter().map(|s| s.as_str()).collect();
                let mut acc = self.encode_type_expr_to_json(codomain, &inner_scope)?;
                for (name, dom) in binder_doms.into_iter().rev() {
                    acc = json!({
                        "ctor": "Pi",
                        "args": [name, dom, acc],
                    });
                }
                Ok(acc)
            }
            ast::Term::Arrow {
                domain, codomain, ..
            } => {
                let dom_json = self.encode_type_expr_to_json(domain, scope)?;
                let cod_json = self.encode_type_expr_to_json(codomain, scope)?;
                Ok(json!({
                    "ctor": "Pi",
                    "args": ["", dom_json, cod_json],
                }))
            }
            // `(e : T)` — bidirectional annotation. Recurse into both children so
            // a `fun` lambda inside `e` keeps its binder annotations (the whole
            // reason `sem` can carry a λ-term that `check_infer` then accepts).
            ast::Term::Ann { expr, typ, .. } => {
                let e_json = self.encode_type_expr_to_json(expr, scope)?;
                let t_json = self.encode_type_expr_to_json(typ, scope)?;
                Ok(json!({
                    "ctor": "Ann",
                    "args": [e_json, t_json],
                }))
            }
            ast::Term::BinderArrow {
                name, kind, body, ..
            } => {
                let kind_str = self.resolve(kind)?;
                let dom_json = json!({
                    "ctor": "ConstRef",
                    "args": [kind_str],
                });
                let mut inner_scope: std::collections::HashSet<&str> = scope.clone();
                inner_scope.insert(name.as_str());
                let body_json = self.encode_type_expr_to_json(body, &inner_scope)?;
                Ok(json!({
                    "ctor": "Pi",
                    "args": [name.clone(), dom_json, body_json],
                }))
            }
            // Sigma ELIMINATION. `eigentt:fst(p)` / `eigentt:snd(p)` are surface spellings of
            // the `Fst`/`Snd` term nodes, not axioms — an axiom would be opaque and never
            // reduce, so `fst(pair)` would not compute. Written as pseudo-application because
            // `Term` has no postfix form at all; a `.1` / `.fst` postfix could be added
            // later and would desugar to these same nodes, leaving encoded terms identical.
            ast::Term::Ref { name, args, .. }
                if args.len() == 1
                    && matches!(
                        self.resolve(name).as_deref(),
                        Ok("urn:eigenius:eigentt:fst") | Ok("urn:eigenius:eigentt:snd")
                    ) =>
            {
                let resolved = self.resolve(name)?;
                let ctor = if resolved.ends_with(":fst") {
                    "Fst"
                } else {
                    "Snd"
                };
                let inner = self.encode_type_expr_to_json(&args[0], scope)?;
                Ok(json!({ "ctor": ctor, "args": [inner] }))
            }
            ast::Term::Ref { name, args, .. } => {
                // Mirror `lower_type_expr_to_exp`'s Ref resolution: bound
                // variable check first, then bare-name ctor lookup, then
                // namespace resolution, then post-resolve ctor lookup,
                // else EigonClass / parametric InductiveType. Args are
                // App-curried regardless of which head shape applies —
                // and we recurse into each arg so any nested Lambda
                // there keeps its annotation.
                let is_bound = name.namespace.is_none() && scope.contains(name.name.as_str());
                let head_json = if is_bound {
                    json!({"ctor": "Var", "args": [name.name.clone()]})
                } else {
                    // Pre-resolution bare-name ctor lookup (with
                    // ambiguity detection via `resolve_ctor_iri`).
                    let bare_ctor = if name.namespace.is_none() {
                        self.resolve_ctor_iri(name)?
                    } else {
                        None
                    };
                    if let Some(ctor_iri_str) = bare_ctor {
                        let parent_iri_str = ctor_iri_str
                            .rsplit_once(':')
                            .map(|(p, _)| p.to_string())
                            .unwrap_or(ctor_iri_str);
                        json!({
                            "ctor": "CtorApp",
                            "args": [parent_iri_str, name.name.clone()],
                        })
                    } else {
                        // Namespace-resolve, then check via
                        // `resolve_ctor_iri` (which walks the
                        // short-name bucket filtered by namespace).
                        let iri_str = self.resolve(name)?;
                        if let Some(ctor_iri_str) = self.resolve_ctor_iri(name)? {
                            let parent_iri_str = ctor_iri_str
                                .rsplit_once(':')
                                .map(|(p, _)| p.to_string())
                                .unwrap_or(ctor_iri_str);
                            json!({
                                "ctor": "CtorApp",
                                "args": [parent_iri_str, name.name.clone()],
                            })
                        } else {
                            // Primitive IRIs ride the ConstRef path
                            // (the D47 decoder maps the five primitive
                            // IRIs to EigonPrimitive directly).
                            json!({"ctor": "ConstRef", "args": [iri_str]})
                        }
                    }
                };
                let mut acc = head_json;
                for arg in args {
                    let arg_json = self.encode_type_expr_to_json(arg, scope)?;
                    acc = json!({
                        "ctor": "App",
                        "args": [acc, arg_json],
                    });
                }
                Ok(acc)
            }
            // Leaves with no Lambda-exposure: lower + encode.
            ast::Term::Sort { .. }
            | ast::Term::LitString { .. }
            | ast::Term::LitInt { .. }
            | ast::Term::LitFloat { .. }
            | ast::Term::LitBool { .. } => encode_leaf(self, typ),
            // Eliminated by the early-return at the top of this fn.
            ast::Term::Alias { .. } => unreachable!("alias expanded above"),
        }
    }

    // --- Data (Phase 11b step 8, D19 §10) ---

    /// Compile a `data` declaration to an `InductiveType` resource.
    ///
    /// The resource shape is documented in
    /// [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json):
    /// embedded `InductiveParam` resources for type parameters and
    /// embedded `InductiveCtor` resources for constructors, each with
    /// embedded `InductiveArgType` resources for arg types.
    ///
    /// Argument-type names that match a declared parameter are
    /// recorded as bare names; everything else is resolved through
    /// the namespace table to a class IRI. Phase 11b step 8b will
    /// decode this back into an `Arc<InductiveDecl>` for use by the
    /// kernel.
    fn compile_data(&self, decl: &ast::DataDecl) -> Result<Vec<Resource>, EslError> {
        use crate::ontology::well_known as wk;

        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);
        // D52 §12 #8 — the primary `is_a` is the implicit
        // `InductiveType` membership; any author-declared extra
        // classes (header form `data X : T, Marker1, Marker2 { ... }`)
        // are appended here so a single inductive-type resource can
        // carry scope markers (`stats:PopulationLevel`, etc.) without
        // a separate companion `resource X : Marker {}` declaration
        // (which would collide via `stamp_attribution` + LayerBuilder
        // last-wins).
        let mut is_a_values: Vec<Value> = vec![Value::String(wk::INDUCTIVE_TYPE.to_string())];
        for extra in &decl.extra_classes {
            let extra_iri = self.resolve(extra)?;
            is_a_values.push(Value::String(extra_iri));
        }
        r.set(iri(wk::IS_A), Value::Array(is_a_values));
        r.set(iri(wk::SHORT_NAME), Value::String(decl.name.name.clone()));
        // eigenius#221 — `description = "…";` in the body. `core:description` is INDEXED
        // (`core:description_text_index`, read by the DCG glossary and OOV grounding), so its
        // absence kept every ESL-authored inductive out of that index.
        if let Some(d) = &decl.description {
            r.set(iri(wk::DESCRIPTION), Value::String(d.clone()));
        }

        let param_names: std::collections::HashSet<&str> =
            decl.params.iter().map(|p| p.name.as_str()).collect();

        let params: Result<Vec<Value>, EslError> = decl
            .params
            .iter()
            .map(|p| {
                let mut pr = Resource::new_embedded();
                set_is_a(&mut pr, wk::INDUCTIVE_PARAM);
                pr.set(iri(wk::PARAM_NAME), Value::String(p.name.clone()));
                let kind = self.lower_kind(&p.kind, &param_names, &p.pos)?;
                pr.set(iri(wk::PARAM_KIND), kind);
                Ok(Value::Embedded(Box::new(pr)))
            })
            .collect();
        r.set(iri(wk::TYPE_PARAMS), Value::Array(params?));

        // eigenius#72 Layer 2 — index telescope. Same shape as
        // `type_params`; absent / empty for non-indexed declarations.
        // Bare references that match a declared parameter name are
        // stored verbatim (so the decoder emits `Exp::Var(name)`);
        // qualified names go through the namespace registry.
        if !decl.indices.is_empty() {
            let indices: Result<Vec<Value>, EslError> = decl
                .indices
                .iter()
                .map(|p| {
                    let mut pr = Resource::new_embedded();
                    set_is_a(&mut pr, wk::INDUCTIVE_PARAM);
                    pr.set(iri(wk::PARAM_NAME), Value::String(p.name.clone()));
                    let kind = self.lower_kind(&p.kind, &param_names, &p.pos)?;
                    pr.set(iri(wk::PARAM_KIND), kind);
                    Ok(Value::Embedded(Box::new(pr)))
                })
                .collect();
            r.set(iri(wk::INDICES), Value::Array(indices?));
        }

        // eigenius#72 Layer 2 — explicit result sort. Encoded as a
        // string; the decoder parses it back into `Exp::Sort(n)`.
        if let Some(sort) = &decl.result_sort {
            r.set(
                iri(wk::RESULT_SORT),
                sort_kind_result_value(sort, &self.declared_universes, &decl.name.pos)?,
            );
        }

        // **Universe generalisation** (eigenius#188, D76 Phase E2). The level
        // variables this declaration actually mentions become its `uparams`, in
        // first-mention order.
        //
        // Generalised rather than written, per N3 §3's binder decision: `universe u;`
        // is FILE-scoped, so a file declaring `u` does not thereby make every
        // declaration in it polymorphic. What binds `u` on a declaration is that the
        // declaration *uses* it — which is also what makes the common case free, since
        // a monomorphic declaration mentions none and gets an empty list.
        //
        // ORDER IS THE INSTANTIATION ORDER. A reference substitutes by position, so
        // first-mention order is the contract between this function and
        // `Level::subst`; a set would make it arbitrary.
        let uparams = self.declaration_universe_params(decl)?;
        if !uparams.is_empty() {
            r.set(
                iri(wk::UNIVERSE_PARAMS),
                Value::Array(uparams.into_iter().map(Value::String).collect()),
            );
        }

        let ctors: Result<Vec<Value>, EslError> = decl
            .ctors
            .iter()
            .map(|c| {
                // **D79 §2.2.1 — a constructor has no chain identity, and the
                // representation now says so.** This used to be
                // `Resource::new("{parent_iri}:{ctor_name}")`, giving every ctor
                // payload an `@id` that looked chain-resolvable and was not: the
                // resource is stored `Value::Embedded` inside `core:ctors`, so
                // nothing resolves it. That `@id` was **written and never read** —
                // every consumer goes through the `core:ctor_name` property
                // (`ground::decode_ctors`, `esl::print`, institution dispatch), and
                // even `external_ctors`, the one place that uses the
                // `{parent}:{name}` string form, reconstructs it from the parent's
                // `id()` plus `ctor_name` rather than reading the `@id` beside it.
                //
                // It is removed because it asserted the wrong thing. Constructors
                // are *closed*: a type's constructors are exhaustively given by its
                // declaration, which is what makes case analysis and the recursor
                // sound. That is exactly what distinguishes them from resources,
                // which are open-world — anyone may add an instance of a class in a
                // later layer, and nobody may add a constructor to an inductive in a
                // later layer. A chain IRI states openness.
                //
                // The validator still reaches these: its embedded-resource recursion
                // gates on `is_a`, not on `@id` (`validation/mod.rs:559`), and
                // attributes any error to the nearest ancestor that has one.
                let mut cr = Resource::new_embedded();
                set_is_a(&mut cr, wk::INDUCTIVE_CTOR);
                cr.set(iri(wk::CTOR_NAME), Value::String(c.name().to_string()));
                match c {
                    ast::CtorDecl::Positional { args, .. } => {
                        // Legacy positional / named-arg form. The ctor's
                        // conclusion is implicitly `Self(params)`; the
                        // chain decoder reassembles the Π-telescope from
                        // `core:arg_types`.
                        let mut local_binders: Vec<String> = Vec::new();
                        let mut arg_values: Vec<Value> = Vec::with_capacity(args.len());
                        for arg in args {
                            let mut scope: std::collections::HashSet<&str> = param_names.clone();
                            for b in &local_binders {
                                scope.insert(b.as_str());
                            }
                            match arg {
                                ast::CtorArg::Positional(t) => {
                                    arg_values.push(self.compile_ctor_arg_type(t, &scope)?);
                                }
                                ast::CtorArg::Named { name, typ, .. } => {
                                    arg_values
                                        .push(self.compile_named_ctor_arg(name, typ, &scope)?);
                                    local_binders.push(name.clone());
                                }
                            }
                        }
                        cr.set(iri(wk::ARG_TYPES), Value::Array(arg_values));
                    }
                    ast::CtorDecl::Typed { typ, pos, .. } => {
                        // eigenius#72 Layer 2 — the typed form supplies
                        // the full Π-telescope (including conclusion
                        // indices) as a single Term. Lower it to
                        // `Exp` and stash the D47-encoded payload under
                        // `core:ctor_type`; the kernel decoder uses it
                        // directly without going through arg_types.
                        let mut scope = param_names.clone();
                        for idx in &decl.indices {
                            scope.insert(idx.name.as_str());
                        }
                        let ctor_exp = self.lower_type_expr_to_exp(typ, &scope)?;
                        let encoded = crate::program::eigentt_type_mirror::encode_type(&ctor_exp)
                            .map_err(|e| {
                            EslError::compiler(
                                Some(pos.clone()),
                                format!("failed to encode ctor type for `{}`: {e}", c.name()),
                            )
                        })?;
                        cr.set(iri(wk::CTOR_TYPE), encoded);
                    }
                }
                Ok(Value::Embedded(Box::new(cr)))
            })
            .collect();
        r.set(iri(wk::CTORS), Value::Array(ctors?));

        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    /// Compile a constructor argument type to an embedded
    /// `InductiveArgType` resource.
    ///
    /// Bare references that match a declared parameter name are kept
    /// as the bare string (so the decoder can recognise them as
    /// parameter substitutions). Everything else must namespace-resolve
    /// to a class IRI.
    fn compile_ctor_arg_type(
        &self,
        arg: &ast::CtorArgType,
        params: &std::collections::HashSet<&str>,
    ) -> Result<Value, EslError> {
        use crate::ontology::well_known as wk;
        let mut ar = Resource::new_embedded();
        set_is_a(&mut ar, wk::INDUCTIVE_ARG_TYPE);

        // Resolution rules, in order:
        // 1. Declared type parameter → bare name (decoder emits `Var`)
        // 2. Otherwise resolve through the namespace registry
        //
        // A third rule covered the built-in size literal `Inf` and sort `Size`; both went with
        // sized types (eigenius#218).
        let type_name = if arg.name.namespace.is_none() {
            let n = arg.name.name.as_str();
            if params.contains(n) {
                bare_kind_value(&arg.name.name)
            } else {
                const_ref_value(&self.resolve(&arg.name)?)
            }
        } else {
            const_ref_value(&self.resolve(&arg.name)?)
        };
        ar.set(iri(wk::TYPE_NAME), type_name);

        let type_args: Result<Vec<Value>, EslError> = arg
            .params
            .iter()
            .map(|p| self.compile_ctor_arg_type(p, params))
            .collect();
        ar.set(iri(wk::TYPE_ARGS), Value::Array(type_args?));

        Ok(Value::Embedded(Box::new(ar)))
    }

    /// A NAMED constructor argument — `succ(base : ex:Nat)`.
    ///
    /// Identical to `compile_ctor_arg_type` bar one property: the name lands in `core:arg_name`, a
    /// declared `recommends` on `core:InductiveArgType` that the Julia mirror generator reads as
    /// the slot's readable field name (D32 §3.2), falling back to `arg_0`/`arg_1` without it.
    ///
    /// This emitted `core:binder_name` until eigenius#221, which is **not a declared property** —
    /// so every use of the form was rejected at commit by Rule 22 §c
    /// (`property key '…:binder_name' is not defined as a core:Property`). Hence 0 occurrences of
    /// `binder_name` on any chain against 85 of `arg_name`: the surface produced one property and
    /// the data used the other.
    fn compile_named_ctor_arg(
        &self,
        name: &str,
        typ: &ast::CtorArgType,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Value, EslError> {
        use crate::ontology::well_known as wk;
        let compiled = self.compile_ctor_arg_type(typ, scope)?;
        let Value::Embedded(mut ar) = compiled else {
            return Err(EslError::compiler(
                None,
                "constructor argument did not compile to a resource".to_string(),
            ));
        };
        ar.set(iri(wk::ARG_NAME), Value::String(name.to_string()));
        Ok(Value::Embedded(ar))
    }

    // --- Class ---

    fn compile_class(&self, class: &ast::ClassDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&class.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".to_string())]),
        );

        // short_name from the local part of the qualified name
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(class.name.name.clone()),
        );

        // subclass_of — from the header form (`class X : A, B { … }`),
        // which is the only authoring style the parser accepts: there
        // is no `ClassItem::SubclassOf` variant and `parse_class_item`
        // rejects a body-level `subclass_of` as an unknown class item.
        // The loop below therefore contributes nothing to this vector
        // (eigenius#29 landed the multi-parent header list only).
        let mut subclass_of: Vec<Value> = Vec::new();
        for parent in &class.parents {
            subclass_of.push(Value::String(self.resolve(parent)?));
        }

        for item in &class.body {
            match item {
                ast::ClassItem::Description(s) => {
                    r.set(
                        iri("urn:eigenius:core:description"),
                        Value::String(s.clone()),
                    );
                }
                ast::ClassItem::Requires(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:requires"), Value::Array(iris?));
                }
                ast::ClassItem::Recommends(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:recommends"), Value::Array(iris?));
                }
            }
        }

        if !subclass_of.is_empty() {
            r.set(
                iri("urn:eigenius:core:subclass_of"),
                Value::Array(subclass_of),
            );
        }

        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    // --- Property ---

    fn compile_property(&self, prop: &ast::PropertyDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&prop.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:core:Property".to_string(),
            )]),
        );

        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(prop.name.name.clone()),
        );

        let dt = self.resolve(&prop.data_type)?;
        r.set(iri("urn:eigenius:core:data_type"), Value::String(dt));

        for item in &prop.body {
            match item {
                ast::PropertyItem::Description(s) => {
                    r.set(
                        iri("urn:eigenius:core:description"),
                        Value::String(s.clone()),
                    );
                }
                ast::PropertyItem::MinValue(v) => {
                    if *v == (*v as i64) as f64 {
                        r.set(
                            iri("urn:eigenius:core:min_value"),
                            Value::Integer(*v as i64),
                        );
                    } else {
                        r.set(iri("urn:eigenius:core:min_value"), Value::Float(*v));
                    }
                }
                ast::PropertyItem::MaxValue(v) => {
                    if *v == (*v as i64) as f64 {
                        r.set(
                            iri("urn:eigenius:core:max_value"),
                            Value::Integer(*v as i64),
                        );
                    } else {
                        r.set(iri("urn:eigenius:core:max_value"), Value::Float(*v));
                    }
                }
                ast::PropertyItem::MinLength(v) => {
                    r.set(iri("urn:eigenius:core:min_length"), Value::Integer(*v));
                }
                ast::PropertyItem::MaxLength(v) => {
                    r.set(iri("urn:eigenius:core:max_length"), Value::Integer(*v));
                }
                ast::PropertyItem::Pattern(s) => {
                    r.set(iri("urn:eigenius:core:pattern"), Value::String(s.clone()));
                }
                ast::PropertyItem::Format(f) => {
                    let fmt = self.resolve(f)?;
                    r.set(iri("urn:eigenius:core:format"), Value::String(fmt));
                }
                ast::PropertyItem::AllowsOnly(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:allows_only"), Value::Array(iris?));
                }
                ast::PropertyItem::Domain(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:domain"), Value::Array(iris?));
                }
                ast::PropertyItem::ClassTypes(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:class_types"), Value::Array(iris?));
                }
                ast::PropertyItem::ElementType(t) => {
                    let et = self.resolve(t)?;
                    r.set(iri("urn:eigenius:core:element_type"), Value::String(et));
                }
                // `expected_type` holds a TERM, so it goes through the D47
                // codec exactly as any other `eigentt:Term`-ranged value does.
                ast::PropertyItem::ExpectedType(typ) => {
                    let scope: std::collections::HashSet<&str> = std::collections::HashSet::new();
                    let exp = self.lower_type_expr_to_exp(typ, &scope)?;
                    let encoded =
                        crate::program::eigentt_type_mirror::encode_type(&exp).map_err(|e| {
                            EslError::compiler(
                                Some(prop.pos.clone()),
                                format!("expected_type encoding failed: {e}"),
                            )
                        })?;
                    r.set(iri("urn:eigenius:eigentt:expected_type"), encoded);
                }
                ast::PropertyItem::IsAType => {
                    r.set(iri("urn:eigenius:eigentt:is_a_type"), Value::Boolean(true));
                }
            }
        }

        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    // --- Resource ---

    fn compile_resource(&self, res: &ast::ResourceDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&res.name)?;
        let mut r = Resource::new(id);

        // is_a is the (one or more) classes from the resource header.
        // Multi-class resources (eigenius#29) emit every class into
        // the array, so they participate in the requires/recommends
        // sets of all of them.
        let class_iris: Result<Vec<Value>, _> = res
            .classes
            .iter()
            .map(|c| self.resolve(c).map(Value::String))
            .collect();
        r.set(iri("urn:eigenius:core:is_a"), Value::Array(class_iris?));

        for field in &res.body {
            let prop_iri = self.resolve_iri(&field.property)?;
            let value = self.compile_value(&field.value)?;
            r.set(prop_iri, value);
        }

        // NOT stamped (D72 §5). `resource { }` is the general instance form: it carries
        // ProgramTraces, ObservationTraces, measured witnesses and imported data as
        // readily as human assertions, so inferring the epistemic category from the
        // keyword asserts something the compiler cannot know. The D71 demo artifact had
        // a `prov:ProgramTrace` whose own `source` names the producing lander
        // stamped `DeclaredResource` + "a human asserted this". The author's `is_a` is
        // the category; the eight theory forms below still stamp, because writing
        // `axiom` or `class` IS a human assertion.
        Ok(vec![r])
    }

    fn compile_value(&self, value: &ast::Value) -> Result<Value, EslError> {
        match value {
            // `json( … )` — an opaque JSON value for a `core:json`-typed property. Passes through
            // verbatim; the kernel stores it as `Value::Json` (eigenius#222).
            ast::Value::Json(j) => Ok(Value::Json(j.clone())),
            ast::Value::String(s) => Ok(Value::String(s.clone())),
            ast::Value::Int(n) => Ok(Value::Integer(*n)),
            ast::Value::Float(f) => Ok(Value::Float(*f)),
            ast::Value::Bool(b) => Ok(Value::Boolean(*b)),
            ast::Value::Ref(qn) => {
                let s = self.resolve(qn)?;
                Ok(Value::String(s))
            }
            ast::Value::Array(items) => {
                let compiled: Result<Vec<_>, _> =
                    items.iter().map(|v| self.compile_value(v)).collect();
                Ok(Value::Array(compiled?))
            }
            ast::Value::Block(fields) => {
                let mut embedded = Resource::new_embedded();
                for field in fields {
                    let prop_iri = self.resolve_iri(&field.property)?;
                    let val = self.compile_value(&field.value)?;
                    embedded.set(prop_iri, val);
                }
                Ok(Value::Embedded(Box::new(embedded)))
            }
            // D32 inductive-value literals. Lower to a chain `Value::Json`
            // carrying the canonical tagged-dict shape (`{ctor, args}`)
            // the kernel's inductive-value validator (Phase 19d.0.b)
            // walks against the target property's declared
            // `class_types` InductiveType. The ctor name + arity
            // type-check happens at commit time on the kernel side;
            // ESL compile is structurally agnostic to which inductive
            // a `CtorApp` lands against — the chain validator has the
            // full ctor schema and reports a clean structural error
            // if the name + arg shapes don't match.
            ast::Value::CtorApp { .. } => Ok(Value::Json(self.ctor_value_to_json(value)?)),
            // `type_expr(<Term>)` — inline D47-encoded EigenTT
            // type expression. Lowers via the same path as `axiom`
            // and `data` ctor types: ESL Term →
            // `lower_type_expr_to_exp` → `encode_type` → chain JSON.
            // Used by D39 justification:Conclusion authors so propositions
            // and certificates can be written in EigenTT surface
            // rather than the hand-built D47 tagged-dict tree.
            ast::Value::Term { typ, pos: _ } => {
                // Walk the AST directly so `fun (x : T) => body`
                // lambdas retain their binder type annotations through
                // the D47 codec. The generic `encode_type` rejects bare
                // `Exp::Lam` (no annotation to recover post-lowering).
                let scope = std::collections::HashSet::new();
                let json = self.encode_type_expr_to_json(typ, &scope)?;
                Ok(Value::Json(json))
            }
            // The parser routes any `ns:Name(args)` to `MacroCall`
            // because it can't tell at parse time whether `Name` is a
            // ctor or a macro. The compiler disambiguates here: try
            // the qualified-ctor lookup first (which surfaces the
            // ambiguity-aware diagnostic when needed), then fall
            // through to D52 §12 macro expansion only if it's not a
            // ctor. This is what makes
            // `justification:App(...)` resolve to the
            // `justification:Term.App` ctor inside a value
            // slot — the disambiguator authors need when bare `App`
            // collides with another inductive's ctor short name.
            ast::Value::MacroCall { name, args, pos } => {
                if self.resolve_ctor_iri(name)?.is_some() {
                    let json = self.qualified_ctor_to_json(&name.name, args)?;
                    return Ok(Value::Json(json));
                }
                let expanded = self.expand_macro_call(name, args, pos)?;
                self.compile_value(&expanded)
            }
        }
    }

    /// D52 §12 — expand a `Value::MacroCall` by looking up the macro,
    /// validating arity, and substituting the positional `args` into
    /// a clone of the macro's body. Returns the substituted `Value`
    /// AST; the caller is responsible for recursively compiling it
    /// (so the substituted ctor application / further macro calls
    /// flow through the normal compile path).
    fn expand_macro_call(
        &self,
        name: &ast::QualifiedName,
        args: &[ast::Value],
        pos: &crate::esl::error::Position,
    ) -> Result<ast::Value, EslError> {
        let iri = self.resolve(name)?;
        let decl = self.macros.get(&iri).ok_or_else(|| {
            EslError::compiler(
                Some(pos.clone()),
                format!("macro `{iri}` is not declared in this file"),
            )
        })?;
        if args.len() != decl.params.len() {
            return Err(EslError::compiler(
                Some(pos.clone()),
                format!(
                    "macro `{iri}` expects {} argument(s), got {}",
                    decl.params.len(),
                    args.len()
                ),
            ));
        }
        // Build the substitution environment: param name → arg Value.
        // Positional binding, no defaults, no named args.
        let env: BTreeMap<&str, &ast::Value> = decl
            .params
            .iter()
            .map(|p| p.name.as_str())
            .zip(args.iter())
            .collect();
        Ok(substitute_in_value(&decl.body, &env))
    }

    /// Recursively convert a ctor-context value into the chain's
    /// inductive tagged-dict JSON. Called for `CtorApp` itself and
    /// for each arg position inside a CtorApp.
    ///
    /// String / Int / Float / Bool become their JSON counterparts;
    /// `Ref` resolves to its IRI string (consistent with how `Ref`
    /// flows in `Value::String` for ordinary properties); `Array`
    /// becomes a JSON array of recursively-converted elements;
    /// `CtorApp` becomes `{"ctor": ..., "args": [...]}`. `Block`
    /// embedded resources are rejected — inductive ctor args are
    /// flat values or other ctors, not nested resources.
    fn ctor_value_to_json(&self, value: &ast::Value) -> Result<serde_json::Value, EslError> {
        match value {
            ast::Value::Json(j) => Ok(j.clone()),
            ast::Value::String(s) => Ok(serde_json::Value::String(s.clone())),
            ast::Value::Int(n) => Ok(serde_json::Value::Number((*n).into())),
            ast::Value::Float(f) => Ok(serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)),
            ast::Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
            ast::Value::Ref(qn) => Ok(serde_json::Value::String(self.resolve(qn)?)),
            ast::Value::Array(items) => {
                let json_items: Result<Vec<_>, _> =
                    items.iter().map(|v| self.ctor_value_to_json(v)).collect();
                Ok(serde_json::Value::Array(json_items?))
            }
            ast::Value::Block(_) => Err(EslError::compiler(
                None,
                "embedded `{...}` resource blocks cannot appear as constructor arguments — \
                 ctor args are flat values or nested constructor applications",
            )),
            ast::Value::CtorApp { ctor, args, .. } => {
                let json_args: Result<Vec<_>, _> =
                    args.iter().map(|v| self.ctor_value_to_json(v)).collect();
                let mut obj = serde_json::Map::new();
                obj.insert("ctor".to_string(), serde_json::Value::String(ctor.clone()));
                obj.insert("args".to_string(), serde_json::Value::Array(json_args?));
                Ok(serde_json::Value::Object(obj))
            }
            ast::Value::Term { .. } => Err(EslError::compiler(
                None,
                "`type_expr(...)` cannot appear as an argument inside a chain inductive ctor — \
                 D32 §3.7 ctor args are flat values or nested ctor applications, not D47-encoded \
                 type expressions. Lift the type_expr to the property value directly.",
            )),
            // Same disambiguation as `compile_value`: try ctor
            // resolution first (qualified ctor refs reach this site
            // when an outer ctor's arg is `justification:App(...)`),
            // fall back to macro expansion otherwise.
            ast::Value::MacroCall { name, args, pos } => {
                if self.resolve_ctor_iri(name)?.is_some() {
                    return self.qualified_ctor_to_json(&name.name, args);
                }
                let expanded = self.expand_macro_call(name, args, pos)?;
                self.ctor_value_to_json(&expanded)
            }
        }
    }

    /// Encode a qualified ctor call to the same `{ctor, args}` JSON
    /// shape as a bare `Value::CtorApp`. The "ctor" field carries the
    /// short name (the inductive's per-ctor identifier inside its
    /// decl); chain consumers disambiguate by the expected inductive
    /// at extract time, so the qualifier doesn't need to land in the
    /// serialised form.
    fn qualified_ctor_to_json(
        &self,
        ctor_short_name: &str,
        args: &[ast::Value],
    ) -> Result<serde_json::Value, EslError> {
        let json_args: Result<Vec<_>, _> =
            args.iter().map(|v| self.ctor_value_to_json(v)).collect();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "ctor".to_string(),
            serde_json::Value::String(ctor_short_name.to_string()),
        );
        obj.insert("args".to_string(), serde_json::Value::Array(json_args?));
        Ok(serde_json::Value::Object(obj))
    }

    // --- Program ---

    fn compile_program(&self, prog: &ast::ProgramDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&prog.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:program:Program".to_string(),
            )]),
        );

        let input_type = self.resolve(&prog.input_type)?;
        r.set(
            iri("urn:eigenius:program:input_type"),
            Value::String(input_type),
        );

        let output_type = self.resolve(&prog.output_type)?;
        r.set(
            iri("urn:eigenius:program:output_type"),
            Value::String(output_type),
        );

        for attr in &prog.attributes {
            match attr {
                ast::ProgramAttribute::Description(s) => {
                    r.set(
                        iri("urn:eigenius:core:description"),
                        Value::String(s.clone()),
                    );
                }
            }
        }

        let body = self.compile_expr(&prog.body)?;
        r.set(
            iri("urn:eigenius:program:body"),
            Value::Embedded(Box::new(body)),
        );

        stamp_attribution(&mut r);
        Ok(vec![r])
    }

    // --- Expression compilation ---

    fn compile_expr(&self, expr: &ast::Expr) -> Result<Resource, EslError> {
        match expr {
            ast::Expr::Let {
                name,
                typ,
                value,
                body,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Let");
                r.set(
                    iri("urn:eigenius:program:name"),
                    Value::String(name.clone()),
                );
                let type_iri = self.resolve(typ)?;
                r.set(iri("urn:eigenius:program:type"), Value::String(type_iri));
                let value_r = self.compile_expr(value)?;
                r.set(
                    iri("urn:eigenius:program:value"),
                    Value::Embedded(Box::new(value_r)),
                );
                let body_r = self.compile_expr(body)?;
                r.set(
                    iri("urn:eigenius:program:body"),
                    Value::Embedded(Box::new(body_r)),
                );
                Ok(r)
            }

            ast::Expr::Apply {
                function,
                args,
                component_argument,
                pos,
            } => {
                // Constructor dispatch (Phase 11b step 10): bare names
                // matching a declared ctor route to a `CtorApply`
                // resource carrying every positional arg. Constructor
                // application accepts any arity ≥ 0; the kernel-side
                // type checker validates against the declared
                // constructor's expected arg count.
                // Bare or qualified ctor reference. The ambiguity-aware
                // `resolve_ctor_iri` handles both: bare names trigger
                // "ambiguous" diagnostics when two inductives share a
                // short name, qualified names resolve to the unique IRI.
                if let Some(ctor_iri) = self.resolve_ctor_iri(function)? {
                    if component_argument.is_some() {
                        return Err(EslError::compiler(
                            Some(pos.clone()),
                            format!(
                                "constructor `{}` cannot take a configuration block — \
                                 constructors are pure data",
                                function.name
                            ),
                        ));
                    }
                    let mut r = Resource::new_embedded();
                    set_is_a(&mut r, "urn:eigenius:program:CtorApply");
                    r.set(
                        iri("urn:eigenius:program:function"),
                        Value::String(ctor_iri),
                    );
                    let arg_resources: Result<Vec<Value>, EslError> = args
                        .iter()
                        .map(|a| Ok(Value::Embedded(Box::new(self.compile_expr(a)?))))
                        .collect();
                    r.set(
                        iri("urn:eigenius:program:arguments"),
                        Value::Array(arg_resources?),
                    );
                    return Ok(r);
                }

                // institution capability classification (D14 §6.2,
                // §9.2). When the function resolves to a Decidable
                // QueryClass or a Comorphism declared in the chain,
                // emit a specialized program resource. Otherwise fall
                // through to ordinary component-dispatch.
                //
                // The parser collapses `ns:local` function names
                // into a bare `Expr::Var { name: "ns:local" }` with
                // `QualifiedName.namespace = None`, so we split on
                // the first `:` and look up the namespace ourselves.
                if let Some(index) = &self.institutions {
                    use crate::institution::registry::DispatchRole;
                    let resolved_func_iri = resolve_apply_function(
                        function.namespace.as_deref(),
                        &function.name,
                        &self.namespaces,
                    );
                    if let Some(func_iri_str) = resolved_func_iri {
                        if let Ok(func_iri_parsed) = Iri::parse(&func_iri_str) {
                            if index.comorphism(&func_iri_parsed).is_some() {
                                if args.len() != 1 || component_argument.is_some() {
                                    return Err(EslError::compiler(
                                        Some(pos.clone()),
                                        format!(
                                            "comorphism `{}` expects exactly 1 source \
                                             argument, got {} positional arg(s){}",
                                            func_iri_str,
                                            args.len(),
                                            if component_argument.is_some() {
                                                " plus a configuration block"
                                            } else {
                                                ""
                                            }
                                        ),
                                    ));
                                }
                                let src_r = self.compile_expr(&args[0])?;
                                let mut r = Resource::new_embedded();
                                set_is_a(&mut r, "urn:eigenius:program:ComorphismInvokeApply");
                                r.set(
                                    iri("urn:eigenius:program:function"),
                                    Value::String(func_iri_str),
                                );
                                r.set(
                                    iri("urn:eigenius:program:source"),
                                    Value::Embedded(Box::new(src_r)),
                                );
                                return Ok(r);
                            }
                            if let Some(qc) = index.query_class(&func_iri_parsed) {
                                if qc.dispatch_roles.contains(&DispatchRole::Decidable) {
                                    if component_argument.is_some() {
                                        return Err(EslError::compiler(
                                            Some(pos.clone()),
                                            format!(
                                                "decide predicate `{}` does not accept a \
                                                 configuration block",
                                                func_iri_str
                                            ),
                                        ));
                                    }
                                    let arg_resources: Result<Vec<Value>, EslError> = args
                                        .iter()
                                        .map(|a| {
                                            Ok(Value::Embedded(Box::new(self.compile_expr(a)?)))
                                        })
                                        .collect();
                                    let mut r = Resource::new_embedded();
                                    set_is_a(&mut r, "urn:eigenius:program:DecideApply");
                                    r.set(
                                        iri("urn:eigenius:program:function"),
                                        Value::String(func_iri_str),
                                    );
                                    r.set(
                                        iri("urn:eigenius:program:arguments"),
                                        Value::Array(arg_resources?),
                                    );
                                    return Ok(r);
                                }
                            }
                        }
                    }
                }

                // Non-ctor function (component dispatch or qualified
                // function reference). Arity rules:
                // - Exactly 1 positional arg → that arg is the input;
                //   optional trailing `{ … }` block becomes
                //   `component_argument`.
                // - Exactly 2 positional args, no block → the legacy
                //   sugar `f(a, b)` ≡ `f(a) { … b … }`; the second
                //   positional becomes `component_argument`.
                // - Anything else for a non-ctor is a compile error.
                let (argument_expr, comp_arg_expr): (&ast::Expr, Option<&ast::Expr>) =
                    match (args.as_slice(), &component_argument) {
                        ([a], None) => (a, None),
                        ([a], Some(b)) => (a, Some(b.as_ref())),
                        ([a, b], None) => (a, Some(b)),
                        ([], _) => {
                            return Err(EslError::compiler(
                                Some(pos.clone()),
                                format!(
                                    "function `{}` called with no positional arguments",
                                    function.name
                                ),
                            ))
                        }
                        ([_, _], Some(_)) => {
                            return Err(EslError::compiler(
                                Some(pos.clone()),
                                format!(
                                    "function `{}` got both a 2nd positional argument and a \
                                 configuration block — supply only one",
                                    function.name
                                ),
                            ))
                        }
                        (more, _) => {
                            return Err(EslError::compiler(
                                Some(pos.clone()),
                                format!(
                                    "function `{}` called with {} positional arguments; \
                                 non-constructor calls accept 1 (with optional config block) \
                                 or 2 (legacy sugar). Multi-positional-arg dispatch is only \
                                 defined for declared inductive constructors.",
                                    function.name,
                                    more.len()
                                ),
                            ))
                        }
                    };

                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Apply");

                let func_iri = if function.namespace.is_some() {
                    self.resolve(function)?
                } else {
                    format!("urn:eigenius:program:components:{}", function.name)
                };
                r.set(
                    iri("urn:eigenius:program:function"),
                    Value::String(func_iri),
                );

                let arg_r = self.compile_expr(argument_expr)?;
                r.set(
                    iri("urn:eigenius:program:argument"),
                    Value::Embedded(Box::new(arg_r)),
                );

                if let Some(comp_arg) = comp_arg_expr {
                    let comp_arg_r = self.compile_expr(comp_arg)?;
                    r.set(
                        iri("urn:eigenius:program:component_argument"),
                        Value::Embedded(Box::new(comp_arg_r)),
                    );
                }

                Ok(r)
            }

            ast::Expr::Var { name, pos } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Var");
                // Bare name matching a declared ctor → ctor IRI as the
                // var name (Phase 11b step 9). The expression builder
                // recognises the IRI shape and produces an
                // `Exp::InductiveCtor` with no arguments.
                //
                // Bare-name lookup is ambiguity-aware: one match → use
                // it, multiple → ambiguous error, none → leave the
                // name as-is for normal variable binding.
                let resolved = match self.ctors_by_short_name.get(name) {
                    Some(iris) if iris.len() == 1 => iris[0].clone(),
                    Some(iris) => {
                        return Err(EslError::compiler(
                            Some(pos.clone()),
                            format!(
                                "bare reference `{}` is ambiguous between multiple chain-resident \
                                 constructors: [{}]. Qualify with a namespace prefix to pick one.",
                                name,
                                iris.join(", "),
                            ),
                        ));
                    }
                    None => name.clone(),
                };
                r.set(iri("urn:eigenius:program:name"), Value::String(resolved));
                Ok(r)
            }

            ast::Expr::Lambda {
                param,
                param_type,
                body,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Lambda");
                r.set(
                    iri("urn:eigenius:program:parameter"),
                    Value::String(param.clone()),
                );
                // D37 §3.1 — when the typed-lambda surface supplied a
                // parameter type, emit it on the Lambda resource so
                // the commit-time validator (PR 2's later step) and
                // the runtime evaluator can both see the binder's
                // declared type. Untyped `\x -> e` lambdas inside
                // `program` bodies omit this slot and rely on the
                // surrounding Pi for inference.
                if let Some(t) = param_type {
                    let scope = std::collections::HashSet::new();
                    let kind_value = self.compile_type_expr(t, &scope)?;
                    r.set(iri("urn:eigenius:program:parameter_type"), kind_value);
                }
                let body_r = self.compile_expr(body)?;
                r.set(
                    iri("urn:eigenius:program:body"),
                    Value::Embedded(Box::new(body_r)),
                );
                Ok(r)
            }

            ast::Expr::Case {
                scrutinee,
                branches,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Case");
                let scrut_r = self.compile_expr(scrutinee)?;
                r.set(
                    iri("urn:eigenius:program:scrutinee"),
                    Value::Embedded(Box::new(scrut_r)),
                );
                let mut branch_resources = Vec::new();
                for (constructor, body) in branches {
                    let mut br = Resource::new_embedded();
                    set_is_a(&mut br, "urn:eigenius:program:Branch");
                    br.set(
                        iri("urn:eigenius:program:constructor"),
                        Value::String(constructor.clone()),
                    );
                    let body_r = self.compile_expr(body)?;
                    br.set(
                        iri("urn:eigenius:program:body"),
                        Value::Embedded(Box::new(body_r)),
                    );
                    branch_resources.push(Value::Embedded(Box::new(br)));
                }
                r.set(
                    iri("urn:eigenius:program:branches"),
                    Value::Array(branch_resources),
                );
                Ok(r)
            }

            ast::Expr::ConstructExpr { class, fields, .. } => {
                // Anonymous block (empty class name) — used for component arguments.
                // Emit a plain embedded resource with resolved keys and data values.
                // Unlike expression compilation, qualified names here resolve to
                // IRI strings (data references), not variable references.
                if class.name.is_empty() {
                    let mut r = Resource::new_embedded();
                    for (prop, expr) in fields {
                        let prop_iri = self.resolve_iri(prop)?;
                        let val = self.compile_block_value(expr)?;
                        r.set(prop_iri, val);
                    }
                    return Ok(r);
                }

                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Construct");
                let class_iri = self.resolve(class)?;
                r.set(iri("urn:eigenius:program:class"), Value::String(class_iri));
                let mut fields_r = Resource::new_embedded();
                for (prop, expr) in fields {
                    let prop_iri = match self.resolve(prop) {
                        Ok(iri_str) => Iri::parse(&iri_str).map_err(|e| {
                            EslError::compiler(Some(prop.pos.clone()), format!("{e}"))
                        })?,
                        Err(_) => {
                            return Err(EslError::compiler(
                                Some(prop.pos.clone()),
                                format!("field name '{}' needs a namespace qualifier", prop.name),
                            ));
                        }
                    };
                    let expr_r = self.compile_expr(expr)?;
                    fields_r.set(prop_iri, Value::Embedded(Box::new(expr_r)));
                }
                r.set(
                    iri("urn:eigenius:program:fields"),
                    Value::Embedded(Box::new(fields_r)),
                );
                Ok(r)
            }

            ast::Expr::Project { expr, property, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Project");
                let expr_r = self.compile_expr(expr)?;
                r.set(
                    iri("urn:eigenius:program:expression"),
                    Value::Embedded(Box::new(expr_r)),
                );
                // Bare names are treated as codata observation names
                // (D11 §8) and emitted under a synthetic URN so the
                // resulting IRI's `local_name()` returns the bare name.
                // Namespaced names resolve to full IRIs as before.
                let prop_iri = match &property.namespace {
                    Some(_) => self.resolve(property)?,
                    None => format!("urn:eigenius:_obs:{}", property.name),
                };
                r.set(
                    iri("urn:eigenius:program:property"),
                    Value::String(prop_iri),
                );
                Ok(r)
            }

            ast::Expr::MapExpr {
                function,
                collection,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Map");
                let func_r = self.compile_expr(function)?;
                r.set(
                    iri("urn:eigenius:program:function"),
                    Value::Embedded(Box::new(func_r)),
                );
                let coll_r = self.compile_expr(collection)?;
                r.set(
                    iri("urn:eigenius:program:collection"),
                    Value::Embedded(Box::new(coll_r)),
                );
                Ok(r)
            }

            ast::Expr::ReduceExpr {
                function,
                initial,
                collection,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Reduce");
                let func_r = self.compile_expr(function)?;
                r.set(
                    iri("urn:eigenius:program:function"),
                    Value::Embedded(Box::new(func_r)),
                );
                let init_r = self.compile_expr(initial)?;
                r.set(
                    iri("urn:eigenius:program:initial"),
                    Value::Embedded(Box::new(init_r)),
                );
                let coll_r = self.compile_expr(collection)?;
                r.set(
                    iri("urn:eigenius:program:collection"),
                    Value::Embedded(Box::new(coll_r)),
                );
                Ok(r)
            }

            ast::Expr::Pair { first, second, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Pair");
                let first_r = self.compile_expr(first)?;
                r.set(
                    iri("urn:eigenius:program:first"),
                    Value::Embedded(Box::new(first_r)),
                );
                let second_r = self.compile_expr(second)?;
                r.set(
                    iri("urn:eigenius:program:second"),
                    Value::Embedded(Box::new(second_r)),
                );
                Ok(r)
            }

            ast::Expr::Literal { value, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Literal");
                let v = match value {
                    ast::LiteralValue::String(s) => Value::String(s.clone()),
                    ast::LiteralValue::Int(n) => Value::Integer(*n),
                    ast::LiteralValue::Float(f) => Value::Float(*f),
                    ast::LiteralValue::Bool(b) => Value::Boolean(*b),
                };
                r.set(iri("urn:eigenius:program:value"), v);
                Ok(r)
            }

            ast::Expr::Match {
                scrutinee,
                returning,
                arms,
                pos,
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Match");

                let scrutinee_r = self.compile_expr(scrutinee)?;
                r.set(
                    iri("urn:eigenius:program:scrutinee"),
                    Value::Embedded(Box::new(scrutinee_r)),
                );

                // `returning` is optional (Phase 11b step 12). When
                // present, the kernel decoder desugars to
                // `Exp::InductiveRec` using the supplied motive. When
                // absent it builds `Exp::Match` and the type checker
                // infers the motive from context.
                //
                // Two on-chain motive encodings (eigenius#72 Layer 3):
                // - A bare `Term::Ref` (qualified name, no args) is
                //   emitted as an IRI string under
                //   `program:result_type` — the pre-Layer-3 wire shape;
                //   kernel decoder wraps it as the constant motive
                //   `λ_. T`.
                // - Anything else (Lambda motives over indices, applied
                //   types, etc.) is lowered to `Exp` via
                //   `lower_type_expr_to_exp` and encoded via the D47
                //   codec, then emitted as a `program:result_motive`
                //   payload. Kernel decoder uses it directly.
                if let Some(te) = returning {
                    match te {
                        ast::Term::Ref { name, args, .. } if args.is_empty() => {
                            let result_iri = self.resolve(name)?;
                            r.set(
                                iri("urn:eigenius:program:result_type"),
                                Value::String(result_iri),
                            );
                        }
                        ast::Term::Lambda { params, body, pos } => {
                            // Encode the Lambda's binder-type annotations
                            // explicitly via `encode_lam_chain` — the
                            // generic `encode_type` rejects bare
                            // `Exp::Lam` because EigenTT Lams are
                            // type-erased and the codec needs the dom
                            // for chain round-trip. Walk params left-to-
                            // right, threading binder names into scope
                            // so dependent forms (`fun (a : Nat, b :
                            // Vec(A, a)) => …`) see earlier binders
                            // when lowering later ones.
                            let mut working: std::collections::HashSet<String> =
                                std::collections::HashSet::new();
                            let mut binders: Vec<(crate::nbe::term::Patt, Exp)> =
                                Vec::with_capacity(params.len());
                            for p in params {
                                let local: std::collections::HashSet<&str> =
                                    working.iter().map(|s| s.as_str()).collect();
                                let dom = self.lower_type_expr_to_exp(&p.typ, &local)?;
                                binders.push((crate::nbe::term::Patt::Var(p.name.clone()), dom));
                                working.insert(p.name.clone());
                            }
                            let inner_scope: std::collections::HashSet<&str> =
                                working.iter().map(|s| s.as_str()).collect();
                            let body_exp = self.lower_type_expr_to_exp(body, &inner_scope)?;
                            let encoded = crate::program::eigentt_type_mirror::encode_lam_chain(
                                &binders, &body_exp,
                            )
                            .map_err(|e| {
                                EslError::compiler(
                                    Some(pos.clone()),
                                    format!("failed to encode match motive: {e}"),
                                )
                            })?;
                            r.set(iri("urn:eigenius:program:result_motive"), encoded);
                        }
                        other => {
                            // Applied refs, arrows, sorts, etc. — lower
                            // via the standard type-expr path. These
                            // contain no Lams so `encode_type` is OK.
                            let scope = std::collections::HashSet::new();
                            let motive_exp = self.lower_type_expr_to_exp(other, &scope)?;
                            let encoded =
                                crate::program::eigentt_type_mirror::encode_type(&motive_exp)
                                    .map_err(|e| {
                                        EslError::compiler(
                                            Some(other.pos().clone()),
                                            format!("failed to encode match motive: {e}"),
                                        )
                                    })?;
                            r.set(iri("urn:eigenius:program:result_motive"), encoded);
                        }
                    }
                }

                let arm_resources: Result<Vec<Value>, EslError> = arms
                    .iter()
                    .map(|arm| {
                        // Match arms today carry a bare short ctor
                        // name (no namespace prefix in the surface).
                        // Ambiguity surfaces here as a hard error too;
                        // qualifying match-arm ctors needs a parser
                        // extension and isn't on the critical path.
                        let ctor_iri = match self.ctors_by_short_name.get(&arm.ctor_name) {
                            Some(iris) if iris.len() == 1 => iris[0].clone(),
                            Some(iris) => {
                                return Err(EslError::compiler(
                                    Some(arm.pos.clone()),
                                    format!(
                                        "match arm constructor `{}` is ambiguous — multiple \
                                         chain-resident inductives declare a constructor with \
                                         this short name: [{}]. Qualifying match-arm ctors with \
                                         a namespace prefix is not yet supported in the surface; \
                                         rename one of the colliding ctors as a workaround.",
                                        arm.ctor_name,
                                        iris.join(", "),
                                    ),
                                ))
                            }
                            None => {
                                return Err(EslError::compiler(
                                    Some(arm.pos.clone()),
                                    format!(
                                        "match arm references unknown constructor `{}` — \
                                         not declared in any `data` block in this file",
                                        arm.ctor_name
                                    ),
                                ))
                            }
                        };
                        let mut ar = Resource::new_embedded();
                        set_is_a(&mut ar, "urn:eigenius:program:MatchArm");
                        ar.set(
                            iri("urn:eigenius:program:ctor"),
                            Value::String(ctor_iri.clone()),
                        );
                        let bindings: Vec<Value> = arm
                            .bindings
                            .iter()
                            .map(|b| Value::String(b.clone()))
                            .collect();
                        ar.set(iri("urn:eigenius:program:bindings"), Value::Array(bindings));
                        let body_r = self.compile_expr(&arm.body)?;
                        ar.set(
                            iri("urn:eigenius:program:body"),
                            Value::Embedded(Box::new(body_r)),
                        );
                        Ok(Value::Embedded(Box::new(ar)))
                    })
                    .collect();
                r.set(
                    iri("urn:eigenius:program:arms"),
                    Value::Array(arm_resources?),
                );

                let _ = pos; // kept on AST for future diagnostics
                Ok(r)
            }
        }
    }

    /// Compile a block value expression to a resource Value.
    ///
    /// Unlike `compile_expr`, this treats qualified names as IRI string
    /// references (data), not as variable references (code). Used for
    /// component argument blocks where `patent:PatentAnalysis` means
    /// the IRI string, not a program variable.
    fn compile_block_value(&self, expr: &ast::Expr) -> Result<Value, EslError> {
        match expr {
            ast::Expr::Literal { value, .. } => match value {
                ast::LiteralValue::String(s) => Ok(Value::String(s.clone())),
                ast::LiteralValue::Int(n) => Ok(Value::Integer(*n)),
                ast::LiteralValue::Float(f) => Ok(Value::Float(*f)),
                ast::LiteralValue::Bool(b) => Ok(Value::Boolean(*b)),
            },
            ast::Expr::Var { name, pos } => {
                // Resolve qualified name to IRI string
                let qn = ast::QualifiedName {
                    namespace: if name.contains(':') {
                        Some(name.split(':').next().unwrap().to_string())
                    } else {
                        None
                    },
                    name: if name.contains(':') {
                        name.split(':').nth(1).unwrap().to_string()
                    } else {
                        name.clone()
                    },
                    pos: pos.clone(),
                };
                let iri_str = self.resolve(&qn)?;
                Ok(Value::String(iri_str))
            }
            ast::Expr::ConstructExpr { class, fields, .. } if class.name.is_empty() => {
                // Nested block — recurse
                let mut r = Resource::new_embedded();
                for (prop, inner_expr) in fields {
                    let prop_iri = self.resolve_iri(prop)?;
                    let val = self.compile_block_value(inner_expr)?;
                    r.set(prop_iri, val);
                }
                Ok(Value::Embedded(Box::new(r)))
            }
            _ => {
                // Fall back to expression compilation for complex cases
                let expr_r = self.compile_expr(expr)?;
                Ok(extract_literal_value(&expr_r))
            }
        }
    }
}

// --- Helpers ---

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI must be valid")
}

/// Extract the value from a compiled expression resource.
/// If it's a Literal (has urn:eigenius:program:value), return the value directly.
/// If it's an anonymous block (no is_a), return as embedded resource.
/// Otherwise wrap as embedded.
fn extract_literal_value(resource: &Resource) -> Value {
    // Check for literal value
    if let Some(val) = resource.get(&iri("urn:eigenius:program:value")) {
        return val.clone();
    }
    // Return as embedded resource
    Value::Embedded(Box::new(resource.clone()))
}

fn set_is_a(resource: &mut Resource, class_iri: &str) {
    resource.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String(class_iri.to_string())]),
    );
}

/// Build a `MergeComorphism` resource (D37 §3.3 / §4.3) with the
/// three required slots: `is_a`, `merge_target_class`, and
/// `merge_transformation`. Used by both the inline and reference
/// `merge_comorphism` lowering paths.
fn build_merge_comorphism_resource(
    comorphism_iri: Iri,
    target_class: Iri,
    transformation: Iri,
) -> Resource {
    use crate::ontology::well_known as wk;
    let mut r = Resource::new(comorphism_iri);
    set_is_a(&mut r, wk::MERGE_COMORPHISM);
    r.set(iri(wk::MERGE_TARGET_CLASS), Value::iri(&target_class));
    r.set(iri(wk::MERGE_TRANSFORMATION), Value::iri(&transformation));
    r
}

/// Compute the content-hash IRI for a synthesised standalone Lambda
/// resource (D37 §4.3, §10.1). The hash is SHA-256 over the
/// resource's canonical Eigon-CBOR bytes with `@id` cleared, so
/// structurally-identical bodies — including ones synthesised by
/// different `merge_comorphism` declarations — produce the same IRI
/// and dedupe through the anchored-commit cache.
fn compute_witness_lambda_iri(resource: &Resource) -> Iri {
    use sha2::{Digest, Sha256};
    // Clone, clear @id, serialize to canonical Eigon-CBOR, hash.
    // `serialize_resource` already produces a deterministic encoding
    // (BTreeMap iteration is sorted, ciborium emits shortest form).
    let mut canonical = resource.clone();
    canonical.set_id(None);
    let bytes = crate::ontology::eigon_cbor::serialize_resource(&canonical);
    let digest = Sha256::digest(&bytes);
    let hex = format!("{digest:x}");
    Iri::parse(&format!("urn:eigenius:auto:lambda:{hex}")).expect("synthesised IRI must be valid")
}

/// Placeholder `declared_by` for an ESL declaration whose source
/// names no declarer.
///
/// `prov:was_attributed_to` answers "who declared this resource", and
/// [`stamp_attribution`] puts it on every compiled resource that did not
/// supply one, so the property is never simply left off. This value is
/// the *absence* of an author attribution, not an answer to the
/// question; it names the channel the declaration arrived through.
/// A `declared_by` written in the ESL source is the real attribution
/// and always wins over it.
/// The bootstrap agent meaning "no agent was recorded" (D72 §3.1). An explicit marker
/// of absence, and a real resolvable resource — `declared_by` is resource-typed since
/// D72 §3.2, so the old `"esl-compiler"` literal would now fail Rule 22 at commit.
const UNATTRIBUTED_DECLARER: &str = "urn:eigenius:prov:agent:unattributed";

/// Env var naming the agent to attribute declarations to for this compile.
///
/// Deliberately NOT the git committer identity: a commit author is who *committed*,
/// which diverges from who *asserted* the moment anyone lands someone else's work.
const SESSION_AGENT_ENV: &str = "EIGENIUS_DECLARED_BY";

/// The agent this compile attributes unattributed declarations to.
///
/// An explicitly configured agent wins; otherwise the unattributed marker. A malformed
/// value is NOT silently ignored — it falls back and the caller sees the marker rather
/// than a fabricated attribution, which is the whole point of D72.
fn session_declarer() -> String {
    declarer_from(std::env::var(SESSION_AGENT_ENV).ok().as_deref())
}

/// The pure half of [`session_declarer`], so the policy is testable without mutating
/// process environment from a parallel test.
fn declarer_from(configured: Option<&str>) -> String {
    match configured.map(str::trim) {
        Some(v) if crate::ontology::iri::Iri::parse(v).is_ok() => v.to_string(),
        _ => UNATTRIBUTED_DECLARER.to_string(),
    }
}

/// Default `prov:was_attributed_to` on a compiled resource.
///
/// Additive, never overwriting: it is set only when the source supplied none,
/// because the author's attribution is the accountability record and the
/// compiler has no standing to replace it (eigenius#141, eigenius#167).
///
/// It also appended `reflection:DeclaredResource` to `is_a`, which is what the
/// function was named for. That class is gone: it recorded a WARRANT grade on a
/// resource whose provenance was the only thing actually known, and stamping it
/// meant every ESL-authored resource in the tree asserted a grade nothing
/// checked. What the stamp was really carrying is the attribution below —
/// provenance, which every resource has — so that half survives alone.
fn stamp_attribution(resource: &mut Resource) {
    let declared_by_iri = iri(crate::ontology::well_known::DECLARED_BY);
    if resource.get(&declared_by_iri).is_none() {
        // `prov:was_attributed_to` is resource-typed with `class_types prov:Agent`, so
        // Rule 8 and Rule 22 require a declarer that resolves same-or-lower.
        resource.set(
            declared_by_iri,
            Value::String(iri(&session_declarer()).as_str().to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    /// **eigenius#188 — a level variable must be declared with `universe`.**
    ///
    /// Lean's `autoBound` mints an undeclared level parameter on first use. That is the wrong
    /// trade here: level variables are cheap to declare and expensive to get silently wrong,
    /// because a typo — `Sort v` where `Sort u` was meant — becomes a SECOND, unrelated universe
    /// rather than an error, and the declaration still compiles and commits.
    ///
    /// Follows Lean's `universe ident ident*` otherwise: space-separated, ESL's semicolon.
    #[test]
    fn a_level_variable_must_be_declared() {
        let head = r#"namespace core = "urn:eigenius:core";
                      namespace p = "urn:eigenius:probe";"#;

        // Declared — compiles, in a type expression and as a declaration's own sort.
        for body in [
            "universe u; axiom p:a : forall (T : Sort u) => T -> T;",
            "universe u v; axiom p:b : forall (T : Sort (max u v)) => T -> T;",
            "universe u; data p:D : Sort u { mk : p:D }",
            "universe u; data p:E : Type u + 1 { mk : p:E }",
        ] {
            crate::esl::compile(&format!("{head}\n{body}"))
                .unwrap_or_else(|e| panic!("`{body}` must compile: {e:?}"));
        }

        // Undeclared — rejected, with the name and a fix in the message.
        let e = crate::esl::compile(&format!(
            "{head}\nuniverse u; axiom p:c : forall (T : Sort v) => T -> T;"
        ))
        .expect_err("`Sort v` with only `u` declared must be rejected");
        let msg = e[0].to_string();
        assert!(msg.contains("`v` is not declared"), "{msg}");
        assert!(
            msg.contains("universe v;"),
            "the message should say the fix: {msg}"
        );

        // And with NO universe declaration at all — the auto-bound case.
        crate::esl::compile(&format!(
            "{head}\naxiom p:d : forall (T : Sort u) => T -> T;"
        ))
        .expect_err("an undeclared level must not auto-bind");
    }

    /// **eigenius#219 — a level variable may not be declared twice.**
    ///
    /// `declared_universes` is a set, so both spellings of a duplicate used to insert twice and
    /// compile, the second insert silently doing nothing. nanoda asserts the same at declaration
    /// admission (`no_dupes_all_params`, `references/nanoda_lib/src/tc.rs:167`), where the stakes
    /// are higher — its `uparams` is an ordered list used for level substitution and a duplicate
    /// makes substitution ambiguous. Here it is merely redundant, and still a mistake.
    ///
    /// eigenius#188 slice 5c added the `universe` form without this check.
    #[test]
    fn a_level_variable_may_not_be_declared_twice() {
        let head = r#"namespace p = "urn:eigenius:p";"#;

        for dup in [
            "universe u u;",           // twice in one declaration
            "universe u; universe u;", // twice across declarations
        ] {
            let e = crate::esl::compile(&format!("{head}\n{dup}"))
                .expect_err("a duplicate level variable must be rejected");
            let msg = e[0].to_string();
            assert!(
                msg.contains("`u` is declared more than once"),
                "the diagnostic must name the offending variable: {msg}"
            );
        }

        // The non-duplicate forms still compile — the check must not reject distinct names.
        crate::esl::compile(&format!("{head}\nuniverse u v;"))
            .expect("distinct level variables in one declaration are fine");
        crate::esl::compile(&format!("{head}\nuniverse u; universe v;"))
            .expect("distinct level variables across declarations are fine");
    }

    /// **eigenius#188 — a declaration's own sort can be POLYMORPHIC.**
    ///
    /// `core:result_sort` was a string (`"Prop"` / `"Set"` / `"Type:N"`), so `data X : Sort u`
    /// had to be rejected: a level variable has no spelling in that grammar. Retyping the
    /// property to a `core:Level` value — the same representation every other level uses — makes
    /// it as writable as `data X : Set`, and lets the validator check it against the ctor schema
    /// instead of nothing checking the string at all.
    ///
    /// The algebra lives in CORE rather than beside `eigentt:Term` because `core:Asserts`
    /// carries a `result_sort`, and a lower layer cannot reference a higher one.
    #[test]
    fn a_declaration_sort_may_be_polymorphic() {
        let cases = [
            (
                "Set",
                serde_json::json!({"ctor": "Succ", "args": [{"ctor": "Zero", "args": []}]}),
            ),
            (
                "Sort u",
                serde_json::json!({"ctor": "Param", "args": ["u"]}),
            ),
            (
                "Sort (max u v)",
                serde_json::json!({"ctor": "Max", "args": [
                    {"ctor": "Param", "args": ["u"]},
                    {"ctor": "Param", "args": ["v"]},
                ]}),
            ),
            (
                "Sort (imax u v)",
                serde_json::json!({"ctor": "IMax", "args": [
                    {"ctor": "Param", "args": ["u"]},
                    {"ctor": "Param", "args": ["v"]},
                ]}),
            ),
        ];
        for (sort_src, expected) in cases {
            let src = format!(
                r#"namespace core = "urn:eigenius:core";
                   namespace p = "urn:eigenius:probe";
                   universe u v;
                   data p:D : {sort_src} {{ mk : p:D }}"#
            );
            let rs = crate::esl::compile(&src)
                .unwrap_or_else(|e| panic!("`data p:D : {sort_src}` must compile: {e:?}"));
            let got = rs[0]
                .get(&Iri::parse(crate::ontology::well_known::RESULT_SORT).unwrap())
                .expect("result_sort present");
            let Value::Json(j) = got else {
                panic!("result_sort must be a Level value, got {got:?}")
            };
            assert_eq!(j, &expected, "`{sort_src}` lowers to the wrong level");
        }
    }

    /// The numeral a `core:result_sort` Level value denotes, for tests that used to compare it
    /// against `"Prop"` / `"Set"` / `"Type:N"` (eigenius#188 retyped it to a `core:Level`).
    fn result_sort_nat(r: &Resource) -> Option<usize> {
        let v = r.get(&Iri::parse(crate::ontology::well_known::RESULT_SORT).unwrap())?;
        let Value::Json(j) = v else { return None };
        let mut n = 0usize;
        let mut cur = j;
        loop {
            match cur.get("ctor")?.as_str()? {
                "Zero" => return Some(n),
                "Succ" => {
                    n += 1;
                    cur = cur.get("args")?.as_array()?.first()?;
                }
                _ => return None,
            }
        }
    }

    use super::*;
    use crate::esl;
    use crate::ontology::eigon_json;

    /// The `eigentt:Term` value a reference to `iri` encodes to. `core:type_name` and
    /// `core:param_kind` carried a bare IRI STRING until eigenius#188 retyped both to
    /// `eigentt:Term`; these two helpers keep the assertions readable.
    fn const_ref_json(target: &str) -> Value {
        Value::Json(serde_json::json!({"ctor": "ConstRef", "args": [target]}))
    }

    /// The `eigentt:Term` value a reference to the type parameter `name` encodes to.
    fn var_json(name: &str) -> Value {
        Value::Json(serde_json::json!({"ctor": "Var", "args": [name]}))
    }

    fn compile_esl(input: &str) -> Vec<Resource> {
        esl::compile(input).unwrap()
    }

    #[test]
    fn compile_class() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Document {
                description = "A text document";
                requires ex:text;
            }
        "#,
        );
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:Document");
        let is_a = r.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:core:Class");
    }

    #[test]
    fn compile_class_with_parent() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Dog : ex:Animal {
                description = "A dog";
                requires ex:breed;
            }
        "#,
        );
        let r = &resources[0];
        let parent = r
            .get(&iri("urn:eigenius:core:subclass_of"))
            .unwrap()
            .as_iri_array();
        assert_eq!(parent[0].as_str(), "urn:eigenius:example:Animal");
    }

    // --- eigenius#29: multi-parent class header + multi-class resources ---

    #[test]
    fn compile_class_with_multiple_parents_in_header() {
        // The colon list accepts more than one class. Both end up in
        // the emitted `core:subclass_of` array, in source order.
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            class ex:HybridCell : ex:Cell, ex:Visualisable {
                description = "A hybrid cell.";
            }
        "#,
        );
        let r = &resources[0];
        let parents: Vec<String> = r
            .get(&iri("urn:eigenius:core:subclass_of"))
            .unwrap()
            .as_iri_array()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        assert_eq!(
            parents,
            vec![
                "urn:eigenius:example:Cell".to_string(),
                "urn:eigenius:example:Visualisable".to_string(),
            ]
        );
    }

    #[test]
    fn compile_resource_with_multiple_classes() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            resource ex:rex : ex:Dog, ex:Pet {
                ex:name = "Rex";
            }
        "#,
        );
        let r = &resources[0];
        let is_a: Vec<String> = r
            .get(&iri("urn:eigenius:core:is_a"))
            .unwrap()
            .as_iri_array()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        // `stamp_declared` appends `reflection:DeclaredResource`; only
        // assert that BOTH author-declared classes survived in source
        // order at the front of the array.
        assert!(is_a.len() >= 2);
        assert_eq!(is_a[0], "urn:eigenius:example:Dog");
        assert_eq!(is_a[1], "urn:eigenius:example:Pet");
    }

    #[test]
    fn compile_resource_with_single_class_unchanged() {
        // Backwards-compatibility: the single-class form is still
        // valid and produces a one-element is_a array (plus the
        // reflection:DeclaredResource tag stamped by `stamp_declared`).
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            resource ex:rex : ex:Dog {
                ex:name = "Rex";
            }
        "#,
        );
        let r = &resources[0];
        let is_a: Vec<String> = r
            .get(&iri("urn:eigenius:core:is_a"))
            .unwrap()
            .as_iri_array()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        assert!(is_a.first().map(|s| s.as_str()) == Some("urn:eigenius:example:Dog"));
    }

    #[test]
    fn compile_property() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            property ex:count : core:integer {
                description = "Number of items";
                min_value = 0;
                max_value = 100;
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:count");
        assert_eq!(
            r.get(&iri("urn:eigenius:core:data_type")).unwrap().as_str(),
            Some("urn:eigenius:core:integer")
        );
        assert_eq!(
            r.get(&iri("urn:eigenius:core:min_value"))
                .unwrap()
                .as_integer(),
            Some(0)
        );
    }

    #[test]
    fn compile_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            resource ex:rex : ex:Dog {
                ex:name = "Rex";
                ex:breed = "German Shepherd";
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:rex");
        assert_eq!(
            r.get(&iri("urn:eigenius:example:name")).unwrap().as_str(),
            Some("Rex")
        );
    }

    #[test]
    fn compile_resource_with_inductive_ctor_value() {
        // D32 inductive-value literals lower to `Value::Json` carrying
        // the canonical `{ctor, args}` tagged-dict shape — the same
        // shape the kernel's inductive-value validator (Phase 19d.0.b)
        // walks against the target property's class_types InductiveType.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:eigenius:example";

            resource ex:t : ex:Holder {
                ex:term = App(OpRef("urn:eigenius:formulas:ops:mul"),
                              LitFloat(2.0));
            }
        "#,
        );
        let r = &resources[0];
        let term = r
            .get(&iri("urn:eigenius:example:term"))
            .expect("term property must be set");
        let Value::Json(json) = term else {
            panic!("expected Value::Json, got {term:?}");
        };
        assert_eq!(json["ctor"], serde_json::json!("App"));
        let args = json["args"].as_array().expect("args must be array");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0]["ctor"], serde_json::json!("OpRef"));
        assert_eq!(
            args[0]["args"][0],
            serde_json::json!("urn:eigenius:formulas:ops:mul")
        );
        assert_eq!(args[1]["ctor"], serde_json::json!("LitFloat"));
        assert_eq!(args[1]["args"][0], serde_json::json!(2.0));
    }

    #[test]
    fn compile_formula_sublanguage() {
        // `formula(...)` lowers through the same Value::CtorApp path
        // as the explicit `App(...)` literal form, producing the
        // canonical chain `{ctor, args}` JSON. Verify the SSE-residual
        // shape from the kinase Ki-fit demo collapses cleanly.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:eigenius:example";

            resource ex:t : ex:Holder {
                ex:term = formula((4 - 2 * Ki) ^ 2);
            }
        "#,
        );
        let r = &resources[0];
        let term = r
            .get(&iri("urn:eigenius:example:term"))
            .expect("term property");
        let Value::Json(json) = term else {
            panic!("expected Value::Json on ex:term");
        };
        // Outermost is pow; rhs is the LitFloat(2.0) exponent.
        assert_eq!(json["ctor"], serde_json::json!("App"));
        assert_eq!(
            json["args"][0]["args"][0]["ctor"],
            serde_json::json!("OpRef")
        );
        assert_eq!(
            json["args"][0]["args"][0]["args"][0],
            serde_json::json!("urn:eigenius:formulas:ops:pow")
        );
        assert_eq!(json["args"][1]["ctor"], serde_json::json!("LitFloat"));
        assert_eq!(json["args"][1]["args"][0], serde_json::json!(2.0));
    }

    #[test]
    fn compile_nullary_ctor_value() {
        // Nullary ctor (`LE()`) lowers to `{ "ctor": "LE", "args": [] }`.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:eigenius:example";

            resource ex:c : ex:Constraint {
                ex:relation = LE();
            }
        "#,
        );
        let r = &resources[0];
        let rel = r
            .get(&iri("urn:eigenius:example:relation"))
            .expect("relation property must be set");
        let Value::Json(json) = rel else {
            panic!("expected Value::Json, got {rel:?}");
        };
        assert_eq!(json["ctor"], serde_json::json!("LE"));
        assert_eq!(json["args"], serde_json::json!([]));
    }

    #[test]
    fn compile_kinase_institutions_notebook_esl() {
        // Smoke-test the ESL flavour the
        // `notebooks/examples/kinase-institutions.json` notebook uses
        // — moderate-depth FormulaTerm trees in resource fields, mixed
        // with the existing array / Ref / scalar shapes. Catches
        // regressions in the inductive-value literal surface that
        // would silently break the notebook on Run All.
        let resources = compile_esl(
            r#"
            namespace core   = "urn:eigenius:core";
            namespace diffeq = "urn:eigenius:diffeq";
            namespace nb     = "urn:eigenius:notebook:kinase_demo";

            resource nb:rhs_A : diffeq:RhsComponent {
                diffeq:term = App(
                    App(
                        App(OpRef("urn:eigenius:formulas:ops:mul"), LitFloat(-1.0)),
                        Var("A")
                    ),
                    Var("k")
                );
            }

            resource nb:rhs_B : diffeq:RhsComponent {
                diffeq:term = App(
                    App(OpRef("urn:eigenius:formulas:ops:mul"), Var("A")),
                    Var("k")
                );
            }

            resource nb:ode_problem : diffeq:OdeProblem {
                core:short_name           = "ab_decay";
                diffeq:state_names        = ["A", "B"];
                diffeq:parameter_names    = ["k"];
                diffeq:rhs                = [nb:rhs_A, nb:rhs_B];
                diffeq:initial_conditions = [1.0, 0.0];
                diffeq:parameters         = [1.0];
                diffeq:time_span_start    = 0.0;
                diffeq:time_span_end      = 1.0;
            }

            resource nb:ode_solution : diffeq:OdeSolution {
                core:short_name    = "ab_solution";
                diffeq:problem     = nb:ode_problem;
                diffeq:algorithm   = "Tsit5";
                diffeq:abstol      = 0.00000001;
                diffeq:reltol      = 0.00000001;
                diffeq:final_state = [0.36787944117144233, 0.6321205588285577];
            }
        "#,
        );
        assert_eq!(resources.len(), 4, "expected 4 resources committed");

        let rhs_a = resources
            .iter()
            .find(|r| r.id().is_some_and(|i| i.as_str().ends_with(":rhs_A")))
            .expect("rhs_A");
        let term = rhs_a
            .get(&iri("urn:eigenius:diffeq:term"))
            .expect("term property");
        let Value::Json(json) = term else {
            panic!("expected Value::Json on diffeq:term");
        };
        assert_eq!(json["ctor"], serde_json::json!("App"));
        // Walk the App-spine: App(App(App(OpRef, Lit), Var(A)), Var(k)).
        // args[0] is the inner App(App(OpRef, Lit), Var(A));
        // args[0]["args"][0] is App(OpRef, Lit);
        // args[0]["args"][0]["args"][0] is OpRef(...:mul).
        assert_eq!(
            json["args"][0]["args"][0]["args"][0]["ctor"],
            serde_json::json!("OpRef")
        );
        assert_eq!(
            json["args"][0]["args"][0]["args"][0]["args"][0],
            serde_json::json!("urn:eigenius:formulas:ops:mul")
        );
        assert_eq!(
            json["args"][0]["args"][0]["args"][1]["ctor"],
            serde_json::json!("LitFloat")
        );
        assert_eq!(
            json["args"][0]["args"][0]["args"][1]["args"][0],
            serde_json::json!(-1.0)
        );
        assert_eq!(json["args"][1]["ctor"], serde_json::json!("Var"));
        assert_eq!(json["args"][1]["args"][0], serde_json::json!("k"));
    }

    /// Pull every ESL cell out of the shipped kinase-institutions
    /// notebook, compile each one, AND run the chain validator over
    /// the resulting resources after loading every institution
    /// ontology the cells reference. Catches two classes of drift:
    ///
    /// 1. *Parse / compile* failures (a future ESL grammar change
    ///    inadvertently breaks the notebook's syntax).
    /// 2. *Validator* failures (operator-arity mismatches, missing
    ///    required properties, malformed inductive payloads, …).
    ///
    /// The arity-mismatch error that surfaced when the user ran cell
    /// 5 against a live kernel — `mul` declares arity 2 but the
    /// FormulaTerm supplied 3 args — is exactly the class of bug
    /// the parse-only smoke test would have missed; the validator
    /// drive here forces it into compile-time.
    /// Whether a compiled resource (or any resource embedded within it,
    /// at any depth) applies a comorphism — i.e. carries a
    /// `program:function` value in the `urn:eigenius:comorphisms:`
    /// namespace. Such programs depend on the runtime-env closure that
    /// an offline compile test does not build (see the call site).
    fn references_comorphism(r: &crate::ontology::resource::Resource) -> bool {
        use crate::ontology::resource::Value;
        const FUNCTION: &str = "urn:eigenius:program:function";
        // The compiler lowers a bare application head to a component
        // IRI, so `comorphisms:symbolics_to_jump(input)` becomes
        // `urn:eigenius:program:components:comorphisms:symbolics_to_jump`
        // — match the `comorphisms:` segment wherever it lands.
        const COMORPHISM_SEG: &str = "comorphisms:";
        fn value_hits(v: &Value) -> bool {
            match v {
                Value::Embedded(inner) => references_comorphism(inner),
                Value::Array(items) => items.iter().any(value_hits),
                _ => false,
            }
        }
        r.properties().iter().any(|(prop, value)| {
            (prop.as_str() == FUNCTION
                && value
                    .as_iri()
                    .is_some_and(|i| i.as_str().contains(COMORPHISM_SEG)))
                || value_hits(value)
        })
    }

    #[test]
    fn compile_every_esl_cell_in_kinase_institutions_notebook_validates_cleanly() {
        use crate::bootstrap::bootstrap_with_storage;
        use crate::lattice::commit_layer_default;
        use crate::layer::LayerStorage;
        use crate::storage::memory::MemoryPersistentBackend;
        use crate::storage::PersistentBackend;
        use crate::validation::Validator;
        use std::sync::Arc;

        const NOTEBOOK_JSON: &str =
            include_str!("../../../notebooks/examples/kinase-institutions.json");
        // Institution ontologies the notebook cells reference. The
        // commit order matches the cross-reference dependency graph
        // (jump before symbolics because Symbolics' SymbolicsToJuMPInput
        // class_types reach into jump:VariableBound / jump:Constraint;
        // diffeq before catalyst because Catalyst's qc_cat_to_ode
        // result_class reaches into diffeq:OdeProblem; symbolics
        // before intervals because intervals' BoundsRequest reaches
        // into symbolics:SymbolicExpression).
        const JUMP_ONTOLOGY: &str =
            include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");
        const SYMBOLICS_ONTOLOGY: &str = include_str!(
            "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
        );
        const INTERVALS_ONTOLOGY: &str = include_str!(
            "../../../julia/institutions/intervals/declarations/intervals-ontology.eigon.json"
        );
        const DIFFEQ_ONTOLOGY: &str = include_str!(
            "../../../julia/institutions/diffeq/declarations/diffeq-ontology.eigon.json"
        );
        const CATALYST_ONTOLOGY: &str = include_str!(
            "../../../julia/institutions/catalyst/declarations/catalyst-ontology.eigon.json"
        );
        // Memory-backed persistent backend so layer commits go through
        // `commit_layer_default` — the D41 supported single-layer-commit
        // surface. `ExecutionContext::commit` was retired in D41 Phase G.
        let backend = Arc::new(MemoryPersistentBackend::new());
        let storage =
            LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
        let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");
        for (label, json) in [
            ("jump_ontology", JUMP_ONTOLOGY),
            ("symbolics_ontology", SYMBOLICS_ONTOLOGY),
            ("intervals_ontology", INTERVALS_ONTOLOGY),
            ("diffeq_ontology", DIFFEQ_ONTOLOGY),
            ("catalyst_ontology", CATALYST_ONTOLOGY),
        ] {
            for r in eigon_json::parse_document(json).expect("parse ontology") {
                ctx.add_resource(r).expect("add ontology resource");
            }
            let working = ctx.take_working(label).expect("take_working");
            let layer = commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
                .expect("commit ontology layer");
            ctx.advance_head(layer, label).expect("advance_head");
        }

        let parsed: serde_json::Value =
            serde_json::from_str(NOTEBOOK_JSON).expect("notebook JSON parses");
        let cells = parsed["cells"]
            .as_array()
            .expect("notebook has a cells array");

        let mut esl_cell_count = 0;
        for cell in cells {
            let cell_type = cell["type"].as_str().unwrap_or("");
            if cell_type != "esl" {
                continue;
            }
            esl_cell_count += 1;
            let id = cell["id"].as_str().unwrap_or("?");
            let source = cell["source"].as_str().expect("esl cell has source");

            let resources = std::panic::catch_unwind(|| compile_esl(source))
                .unwrap_or_else(|_| panic!("ESL cell {id} failed to compile"));
            assert!(
                !resources.is_empty(),
                "ESL cell {id} compiled to zero resources"
            );

            // Part C's program cells apply a comorphism
            // (`comorphisms:symbolics_to_jump`) whose reference closure
            // — comorphism → export/import formats → institution
            // declaration → `symbolics:env:v1` — bottoms out at a
            // Julia runtime-env build artifact that only exists after
            // the setup script's `env build` step. That closure is
            // unresolvable in an offline compile test, so such cells are
            // compile-checked (above) but not committed to the
            // clean-validation chain. Before Rule 23 (embedded-resource
            // recursion) landed, the dangling comorphism reference sat
            // inside an embedded Apply node and escaped validation, so
            // these cells appeared to "validate cleanly" — they never
            // did. Detected structurally: a compiled resource whose
            // `program:function` (at any depth) names the comorphisms
            // namespace.
            if resources.iter().any(references_comorphism) {
                continue;
            }

            for r in resources {
                ctx.add_resource(r)
                    .unwrap_or_else(|e| panic!("ESL cell {id}: add_resource: {e:?}"));
            }
            let cell_label = format!("notebook_cell_{id}");
            let working = ctx
                .take_working(&cell_label)
                .unwrap_or_else(|e| panic!("ESL cell {id}: take_working: {e}"));
            let layer = commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
                .unwrap_or_else(|e| panic!("ESL cell {id}: commit failed: {e:?}"));
            ctx.advance_head(layer, &cell_label)
                .unwrap_or_else(|e| panic!("ESL cell {id}: advance_head: {e}"));
        }
        assert!(
            esl_cell_count >= 3,
            "expected the notebook to ship ≥ 3 ESL cells; got {esl_cell_count}"
        );

        let validator = Validator::new(std::sync::Arc::clone(ctx.head()));
        let errors = validator.validate();
        assert!(
            errors.is_empty(),
            "notebook chain must validate cleanly; got errors:\n{}",
            errors
                .iter()
                .map(|e| format!("  [{:?}] {} on {:?}", e.rule, e.message, e.resource_id))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn compile_simple_program() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:identity : ex:Document -> ex:Document {
                input
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:identity");
        assert_eq!(
            r.get(&iri("urn:eigenius:program:input_type"))
                .unwrap()
                .as_str(),
            Some("urn:eigenius:example:Document")
        );
    }

    #[test]
    fn compile_program_with_let_and_construct() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:summarize : ex:Document -> ex:Document {
                let summary : core:string = CompleteText(input);
                Construct ex:Document { ex:text = summary }
            }
        "#,
        );
        let r = &resources[0];
        let body = r
            .get(&iri("urn:eigenius:program:body"))
            .unwrap()
            .as_embedded()
            .unwrap();
        // Body should be a Let
        let is_a = body.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:program:Let");
    }

    #[test]
    fn compile_component_shorthand() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:test : ex:A -> ex:B {
                CompleteText(input)
            }
        "#,
        );
        let r = &resources[0];
        let body = r
            .get(&iri("urn:eigenius:program:body"))
            .unwrap()
            .as_embedded()
            .unwrap();
        // Function should be the full component IRI
        let func = body
            .get(&iri("urn:eigenius:program:function"))
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(func, "urn:eigenius:program:components:CompleteText");
    }

    #[test]
    fn compile_full_file() {
        let input = r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Document {
                description = "A text document";
                requires ex:text;
            }

            property ex:text : core:string {
                description = "The text content";
            }

            resource ex:doc1 : ex:Document {
                ex:text = "Hello world";
            }

            program ex:summarize : ex:Document -> ex:Document {
                let summary : core:string = CompleteText(input);
                Construct ex:Document { ex:text = summary }
            }
        "#;

        let resources = compile_esl(input);
        assert_eq!(resources.len(), 4);

        // Verify all resources serialize to valid Eigon-JSON
        for r in &resources {
            let json = eigon_json::serialize_resource(r);
            assert!(json.is_object(), "resource should serialize to JSON object");
        }
    }

    #[test]
    fn compile_unknown_namespace_error() {
        let result = esl::compile(
            r#"
            class unknown:Foo {
                description = "Bad";
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn round_trip_demo() {
        // Compile the demo ESL and verify it produces the same structure
        // as the hand-written demo/document.json
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace demo = "urn:eigenius:demo";

            class demo:Document {
                description = "A text document for analysis.";
                requires demo:text;
            }

            property demo:text : core:string {
                description = "The text content of a document.";
            }

            resource demo:doc_001 : demo:Document {
                demo:text = "Eigenius is a typed knowledge graph platform.";
            }
        "#,
        );

        assert_eq!(resources.len(), 3);
        // Class
        assert_eq!(
            resources[0].id().unwrap().as_str(),
            "urn:eigenius:demo:Document"
        );
        // Property
        assert_eq!(
            resources[1].id().unwrap().as_str(),
            "urn:eigenius:demo:text"
        );
        // Resource
        assert_eq!(
            resources[2].id().unwrap().as_str(),
            "urn:eigenius:demo:doc_001"
        );
        assert_eq!(
            resources[2]
                .get(&iri("urn:eigenius:demo:text"))
                .unwrap()
                .as_str(),
            Some("Eigenius is a typed knowledge graph platform.")
        );
    }

    // --- attribution stamping tests (Phase 10b; the grade half deleted) ---

    /// No compiled resource may carry a grade class in `is_a`. The compiler used to
    /// append `reflection:DeclaredResource` to every class, property and program it
    /// compiled; the class is deleted and the stamp with it, so this asserts absence.
    fn has_grade_class(r: &Resource) -> bool {
        r.is_a().iter().any(|i| {
            matches!(
                i.as_str(),
                "urn:eigenius:reflection:DeclaredResource"
                    | "urn:eigenius:reflection:ObservedResource"
                    | "urn:eigenius:reflection:DerivedResource"
                    | "urn:eigenius:reflection:VerifiedResource"
            )
        })
    }

    /// Reads the value through an accessor rather than matching a variant, so the helper
    /// cannot pass or fail for the wrong reason.
    fn declared_by(r: &Resource) -> Option<String> {
        r.get(&iri(crate::ontology::well_known::DECLARED_BY))
            .and_then(|v| v.as_iri_str().map(|s| s.to_string()))
    }

    #[test]
    fn esl_class_gets_an_attribution_and_no_grade() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Foo {
                description = "test";
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            !has_grade_class(r),
            "no compiled resource may carry a grade class"
        );
        assert_eq!(declared_by(r), Some(UNATTRIBUTED_DECLARER.to_string()));
    }

    #[test]
    fn esl_property_gets_an_attribution_and_no_grade() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            property ex:bar : core:string {
                description = "test";
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            !has_grade_class(r),
            "no compiled resource may carry a grade class"
        );
        assert_eq!(declared_by(r), Some(UNATTRIBUTED_DECLARER.to_string()));
    }

    /// `resource { }` is NOT stamped (D72 §5): it is the general instance form and
    /// carries traces, measurements and imported data as readily as human assertions,
    /// so the compiler cannot infer the epistemic category from the keyword. Renamed
    /// from `esl_resource_stamped_declared_resource`, which pinned the old behaviour.
    #[test]
    fn esl_resource_gets_no_attribution() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            resource ex:thing : ex:Foo {
                ex:name = "test";
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            !has_grade_class(r),
            "no compiled resource may carry a grade class"
        );
        assert_eq!(
            declared_by(r),
            None,
            "no declarer may be invented for a resource whose category the compiler does not know"
        );
    }

    #[test]
    fn esl_program_gets_an_attribution_and_no_grade() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:identity : ex:A -> ex:B {
                input
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            !has_grade_class(r),
            "no compiled resource may carry a grade class"
        );
        assert_eq!(declared_by(r), Some(UNATTRIBUTED_DECLARER.to_string()));
    }

    /// eigenius#141 / #167 — a `declared_by` written in the source is
    /// the resource's accountability record and must survive
    /// compilation. Mirrors the WRN chain's bridge shape
    /// (`experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl`).
    #[test]
    fn author_declared_by_survives_compilation() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ref = "urn:eigenius:reflection";
            namespace prov = "urn:eigenius:prov";
            namespace wrn = "urn:eigenius:pub:wrn";

            resource wrn:bridge_msi_selective : core:Class {
                prov:was_attributed_to = "wrn-paper:selective-essentiality-criterion";
                prov:rationale   = "Independent-platform replication is the warrant.";
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(
            declared_by(r),
            Some("wrn-paper:selective-essentiality-criterion".to_string()),
            "author-supplied declared_by must not be replaced by the compiler"
        );
        assert!(!has_grade_class(r));
    }

    /// A theory form with no configured session agent gets the unattributed marker.
    ///
    /// The `class` / `property` / `data` / `program` bodies are fixed grammars with no
    /// slot for `declared_by`, so this is the only value they can carry unless
    /// `EIGENIUS_DECLARED_BY` names one. Previously written against the `resource`
    /// form, which no longer stamps at all (D72 §5).
    #[test]
    fn unattributed_theory_form_gets_the_absence_marker() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Dog {
                description = "a dog";
            }
        "#,
        );
        assert_eq!(
            declared_by(&resources[0]),
            Some(UNATTRIBUTED_DECLARER.to_string())
        );
    }

    /// The session agent overrides the marker, and a malformed value falls back to it
    /// rather than being written through — a value that is not an IRI would fail
    /// Rule 22 at commit, and inventing an attribution is what D72 exists to stop.
    #[test]
    fn session_declarer_policy() {
        assert_eq!(declarer_from(None), UNATTRIBUTED_DECLARER);
        assert_eq!(declarer_from(Some("")), UNATTRIBUTED_DECLARER);
        assert_eq!(declarer_from(Some("not an iri")), UNATTRIBUTED_DECLARER);
        assert_eq!(
            declarer_from(Some("  urn:eigenius:agent:hmw  ")),
            "urn:eigenius:agent:hmw",
            "surrounding whitespace is trimmed, not treated as malformed"
        );
    }

    /// eigenius#141 / #167, `is_a` half — stamping is idempotent, so a
    /// decompile/recompile round trip does not accumulate
    /// `DeclaredResource` entries.
    #[test]
    fn compilation_adds_no_grade_class() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ref = "urn:eigenius:reflection";
            namespace prov = "urn:eigenius:prov";
            namespace ex = "urn:eigenius:example";

            resource ex:rex : core:Class {
                prov:was_attributed_to = "someone";
            }
        "#,
        );
        let r = &resources[0];
        // This pinned that the compiler appended `DeclaredResource` exactly once
        // when the source already carried it. Nothing appends a grade class now,
        // and an author who writes one gets no help either: the class does not
        // resolve, so the resource fails at commit rather than being stamped twice.
        assert!(
            !has_grade_class(r),
            "no grade class may survive compilation"
        );
    }

    // --- `data` declaration compilation (Phase 11b step 8) ---

    /// **eigenius#221 — `data` can document itself and name its constructor arguments.**
    ///
    /// Both properties are declared in the core schema and neither was reachable from the surface:
    /// `core:description` is universal (and INDEXED — `core:description_text_index`), and
    /// `core:arg_name` is a `recommends` on `core:InductiveArgType` that the Julia mirror generator
    /// reads for field names.
    ///
    /// The named form previously spelled `{base : ex:Nat}` and compiled to `core:binder_name`,
    /// which is **not a declared property** — Rule 22 §c rejected every use at commit. Hence 0
    /// `binder_name` on any chain against 85 `arg_name`.
    #[test]
    fn data_carries_a_description_and_named_ctor_args() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Tree(A : Set) {
                description = "a binary tree";
                leaf,
                node(left : ex:Tree(A), value : A),
            }
            "#,
        );
        let r = &resources[0];
        assert_eq!(
            r.get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("a binary tree")
        );

        let ctors = match r.get(&iri("urn:eigenius:core:ctors")) {
            Some(Value::Array(a)) => a,
            other => panic!("expected ctors array, got {other:?}"),
        };
        let node = match &ctors[1] {
            Value::Embedded(e) => e.as_ref(),
            other => panic!("expected embedded ctor, got {other:?}"),
        };
        let args = match node.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            other => panic!("expected arg_types array, got {other:?}"),
        };
        let names: Vec<Option<&str>> = args
            .iter()
            .map(|a| match a {
                Value::Embedded(e) => e
                    .get(&iri("urn:eigenius:core:arg_name"))
                    .and_then(|v| v.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec![Some("left"), Some("value")]);

        // The undeclared property the old brace form emitted must not reappear: it fails Rule 22.
        for a in args {
            if let Value::Embedded(e) = a {
                assert!(
                    e.get(&iri("urn:eigenius:core:binder_name")).is_none(),
                    "core:binder_name is not a declared property and must not be emitted"
                );
            }
        }
    }

    /// A positional argument stays anonymous — the named form is opt-in, and a bare qualified name
    /// must not be mistaken for `name : type`. `ex:Tree` lexes as one `QualName` token; the
    /// standalone `Colon` is a different token, so the two are distinguishable without spacing
    /// conventions.
    #[test]
    fn a_positional_ctor_arg_carries_no_arg_name() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";
            data ex:Nat { zero, succ(ex:Nat), }
            "#,
        );
        let ctors = match resources[0].get(&iri("urn:eigenius:core:ctors")) {
            Some(Value::Array(a)) => a,
            other => panic!("expected ctors, got {other:?}"),
        };
        let succ = match &ctors[1] {
            Value::Embedded(e) => e.as_ref(),
            other => panic!("expected embedded, got {other:?}"),
        };
        let args = match succ.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            other => panic!("expected arg_types, got {other:?}"),
        };
        match &args[0] {
            Value::Embedded(e) => assert!(
                e.get(&iri("urn:eigenius:core:arg_name")).is_none(),
                "a positional argument must not acquire a name"
            ),
            other => panic!("expected embedded arg, got {other:?}"),
        }
    }

    #[test]
    fn compile_data_nat_non_parametric() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }
            "#,
        );
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:Nat");
        assert!(r
            .is_a()
            .iter()
            .any(|i| i.as_str() == "urn:eigenius:core:InductiveType"));
        assert_eq!(
            r.get(&iri("urn:eigenius:core:short_name"))
                .and_then(|v| v.as_str()),
            Some("Nat")
        );

        // No params for Nat.
        let params = match r.get(&iri("urn:eigenius:core:type_params")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_params must be an array"),
        };
        assert!(params.is_empty());

        // Two constructors.
        let ctors = match r.get(&iri("urn:eigenius:core:ctors")) {
            Some(Value::Array(a)) => a,
            _ => panic!("ctors must be an array"),
        };
        assert_eq!(ctors.len(), 2);

        // zero
        let zero = match &ctors[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("ctor must be embedded"),
        };
        // **No `@id`** (D79 §2.2.1). A constructor has no chain identity: its
        // identity is `(inductive IRI, ctor_name)`, which is what the D47 wire
        // carries as `CtorApp(D, c)`. Until D79 P4 this asserted
        // `urn:eigenius:example:Nat:zero` — an `@id` that looked chain-resolvable,
        // was stored `Value::Embedded` so nothing resolved it, and was read by no
        // consumer. Constructors are *closed*, resources are open-world; a chain
        // IRI stated the wrong one.
        assert_eq!(
            zero.id(),
            None,
            "a constructor is an embedded resource with no @id"
        );
        assert_eq!(
            zero.get(&iri("urn:eigenius:core:ctor_name"))
                .and_then(|v| v.as_str()),
            Some("zero")
        );
        let zero_args = match zero.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            _ => panic!("arg_types must be an array"),
        };
        assert!(zero_args.is_empty());

        // succ(ex:Nat)
        let succ = match &ctors[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("ctor must be embedded"),
        };
        assert_eq!(succ.id(), None, "D79 §2.2.1 — no @id on a constructor");
        assert_eq!(
            succ.get(&iri("urn:eigenius:core:ctor_name"))
                .and_then(|v| v.as_str()),
            Some("succ")
        );
        let succ_args = match succ.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            _ => panic!("arg_types must be an array"),
        };
        assert_eq!(succ_args.len(), 1);
        let succ_arg = match &succ_args[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("arg type must be embedded"),
        };
        assert_eq!(
            succ_arg.get(&iri("urn:eigenius:core:type_name")),
            Some(&const_ref_json("urn:eigenius:example:Nat"))
        );
    }

    #[test]
    fn compile_data_list_parametric_records_param_references_as_bare_names() {
        // The bare `A` in `cons(A, ex:List(A))` is a reference to the
        // type parameter — compile encodes it as the raw name `"A"`,
        // not a resolved IRI.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:List(A : Set) {
                nil,
                cons(A, ex:List(A)),
            }
            "#,
        );
        let r = &resources[0];

        // One param, name=A, kind=`Sort(Succ(Zero))` — the sort `Set`.
        let params = match r.get(&iri("urn:eigenius:core:type_params")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_params must be an array"),
        };
        assert_eq!(params.len(), 1);
        let p = match &params[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("param must be embedded"),
        };
        assert_eq!(
            p.get(&iri("urn:eigenius:core:param_name"))
                .and_then(|v| v.as_str()),
            Some("A")
        );
        assert_eq!(
            p.get(&iri("urn:eigenius:core:param_kind")),
            Some(&Value::Json(serde_json::json!({
                "ctor": "Sort",
                "args": [{"ctor": "Succ", "args": [{"ctor": "Zero", "args": []}]}],
            })))
        );

        // cons ctor: first arg is bare "A", second is parametric List(A).
        let ctors = match r.get(&iri("urn:eigenius:core:ctors")) {
            Some(Value::Array(a)) => a,
            _ => panic!("ctors must be an array"),
        };
        let cons = match &ctors[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("cons must be embedded"),
        };
        let cons_args = match cons.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            _ => panic!("arg_types must be an array"),
        };
        assert_eq!(cons_args.len(), 2);

        // arg 0: bare A — type_name is "A", no type_args.
        let arg0 = match &cons_args[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("arg must be embedded"),
        };
        assert_eq!(
            arg0.get(&iri("urn:eigenius:core:type_name")),
            Some(&var_json("A"))
        );
        let arg0_args = match arg0.get(&iri("urn:eigenius:core:type_args")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_args must be an array"),
        };
        assert!(arg0_args.is_empty());

        // arg 1: ex:List(A) — type_name is IRI, type_args = [bare A].
        let arg1 = match &cons_args[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("arg must be embedded"),
        };
        assert_eq!(
            arg1.get(&iri("urn:eigenius:core:type_name")),
            Some(&const_ref_json("urn:eigenius:example:List"))
        );
        let arg1_args = match arg1.get(&iri("urn:eigenius:core:type_args")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_args must be an array"),
        };
        assert_eq!(arg1_args.len(), 1);
        let arg1_a = match &arg1_args[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("type arg must be embedded"),
        };
        assert_eq!(
            arg1_a.get(&iri("urn:eigenius:core:type_name")),
            Some(&var_json("A"))
        );
    }

    // --- eigenius#72 Layer 2: indexed data declarations ---

    // --- eigenius#72 Phase 5: end-to-end integration ---

    #[test]
    fn end_to_end_axiom_indexed_data_match_returning_all_in_one_file() {
        // Exercises all three Layers together: axiom statement (Layer 1)
        // referencing an indexed inductive (Layer 2), plus a program
        // that pattern-matches with a Lambda motive (Layer 3). Verifies
        // each surface form emits the expected chain shape.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            data ex:Vec(A : Set) : core:Nat -> Set {
                nil  : ex:Vec(A, ex:zero),
                cons : forall (n : core:Nat) => A -> ex:Vec(A, n) -> ex:Vec(A, ex:succ(n)),
            }

            axiom ex:vec_inhabits_nat_length :
                forall (A : Set, n : core:Nat) => ex:Vec(A, n) -> ex:Nat
            note: "Every Vec carries a Nat-valued length implicit in its index."

            program ex:identity : ex:Nat -> ex:Nat {
                match input returning fun (n : core:Nat) => ex:Nat {
                    zero -> input;
                    succ(k) -> input;
                }
            }
            "#,
        );

        // Layer 2: ex:Vec carries indices, result_sort, and typed ctors.
        let vec = resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()).unwrap_or("").ends_with(":Vec"))
            .expect("Vec resource");
        assert!(
            vec.get(&Iri::parse(crate::ontology::well_known::INDICES).unwrap())
                .is_some(),
            "Vec should carry core:indices"
        );
        assert_eq!(result_sort_nat(vec), Some(1));

        // Layer 1: axiom resource is an eigentt:Axiom with statement +
        // justification.
        let axiom = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str())
                    .unwrap_or("")
                    .ends_with(":vec_inhabits_nat_length")
            })
            .expect("axiom resource");
        assert!(
            axiom
                .is_a()
                .iter()
                .any(|c| c.as_str() == "urn:eigenius:eigentt:Axiom"),
            "axiom should be is_a eigentt:Axiom"
        );
        assert!(
            axiom
                .get(&Iri::parse("urn:eigenius:eigentt:axiom_statement").unwrap())
                .is_some(),
            "axiom should carry axiom_statement payload"
        );
        assert_eq!(
            axiom
                .get(&Iri::parse("urn:eigenius:eigentt:axiom_justification").unwrap())
                .and_then(|v| if let Value::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }),
            Some("Every Vec carries a Nat-valued length implicit in its index.")
        );

        // Layer 3: program carries a Match with result_motive (not
        // result_type).
        let prog = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str())
                    .unwrap_or("")
                    .ends_with(":identity")
            })
            .expect("program resource");
        let body = match prog.get(&Iri::parse("urn:eigenius:program:body").unwrap()) {
            Some(Value::Embedded(e)) => e.as_ref(),
            other => panic!("expected program:body, got {other:?}"),
        };
        assert!(
            body.get(&Iri::parse("urn:eigenius:program:result_motive").unwrap())
                .is_some(),
            "match should carry program:result_motive"
        );
    }

    #[test]
    fn compile_data_indexed_emits_indices_and_result_sort_and_ctor_type() {
        use crate::ontology::well_known as wk_local;

        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Vec(A : Set) : core:Nat -> Set {
                nil : ex:Vec(A, ex:zero),
                cons : forall (n : core:Nat) => A -> ex:Vec(A, n) -> ex:Vec(A, ex:succ(n)),
            }
            "#,
        );
        let r = &resources[0];

        // Indices property — one anonymous index of type `core:Nat`.
        let indices_iri = Iri::parse(wk_local::INDICES).unwrap();
        match r.get(&indices_iri) {
            Some(Value::Array(arr)) => {
                assert_eq!(arr.len(), 1, "expected one index entry, got {arr:?}");
                let entry = match &arr[0] {
                    Value::Embedded(e) => e.as_ref(),
                    other => panic!("expected embedded InductiveParam, got {other:?}"),
                };
                assert_eq!(
                    entry
                        .get(&Iri::parse(wk_local::PARAM_NAME).unwrap())
                        .and_then(|v| if let Value::String(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }),
                    Some("_")
                );
                assert_eq!(
                    entry.get(&Iri::parse(wk_local::PARAM_KIND).unwrap()),
                    Some(&const_ref_json("urn:eigenius:core:Nat"))
                );
            }
            other => panic!("expected `core:indices` array, got {other:?}"),
        }

        // Result sort — explicitly `Set`.
        assert_eq!(result_sort_nat(r), Some(1));

        // Both ctors should carry `core:ctor_type`, none should carry
        // `core:arg_types` (typed form bypasses arg_types entirely).
        let ctors_iri = Iri::parse(wk_local::CTORS).unwrap();
        let ctor_type_iri = Iri::parse(wk_local::CTOR_TYPE).unwrap();
        let arg_types_iri = Iri::parse(wk_local::ARG_TYPES).unwrap();
        match r.get(&ctors_iri) {
            Some(Value::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                for (i, ctor_val) in arr.iter().enumerate() {
                    let cr = match ctor_val {
                        Value::Embedded(e) => e.as_ref(),
                        other => panic!("ctor {i}: expected embedded, got {other:?}"),
                    };
                    assert!(
                        cr.get(&ctor_type_iri).is_some(),
                        "ctor {i} should carry core:ctor_type"
                    );
                    assert!(
                        cr.get(&arg_types_iri).is_none(),
                        "ctor {i} should NOT carry core:arg_types in typed form"
                    );
                }
            }
            other => panic!("expected `core:ctors` array, got {other:?}"),
        }
    }

    #[test]
    fn compile_data_indexed_by_parameter_keeps_param_name_in_kind_string() {
        use crate::ontology::well_known as wk_local;

        // `data Eq(A : Set) : A -> A -> Prop { ... }` — the index kind
        // is the parameter `A`, which the compiler must preserve as
        // the bare string `"A"` (not try to namespace-resolve it).
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Eq(A : Set) : A -> A -> Prop {
                refl : forall (a : A) => ex:Eq(A, a, a),
            }
            "#,
        );
        let r = &resources[0];
        let indices_iri = Iri::parse(wk_local::INDICES).unwrap();
        let arr = match r.get(&indices_iri) {
            Some(Value::Array(a)) => a,
            other => panic!("expected indices array, got {other:?}"),
        };
        assert_eq!(arr.len(), 2);
        for entry in arr {
            let pr = match entry {
                Value::Embedded(e) => e.as_ref(),
                other => panic!("expected embedded, got {other:?}"),
            };
            assert_eq!(
                pr.get(&Iri::parse(wk_local::PARAM_KIND).unwrap()),
                Some(&var_json("A")),
                "param-typed index should keep bare param name as kind"
            );
        }
        // Result sort should be `Prop`.
        assert_eq!(result_sort_nat(r), Some(0));
    }

    #[test]
    fn compile_match_lambda_motive_emits_result_motive_payload() {
        // Layer 3 — `match v returning fun (n : Nat) => Nat { … }`
        // should emit a `program:result_motive` carrying a D47-encoded
        // Exp::Lam, *not* the legacy `program:result_type` IRI string.
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";
            namespace core = "urn:eigenius:core";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:identity : ex:Nat -> ex:Nat {
                match input returning fun (n : core:Nat) => ex:Nat {
                    zero -> input;
                    succ(k) -> input;
                }
            }
            "#,
        );
        // Find the program resource.
        let prog = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str())
                    .unwrap_or("")
                    .ends_with(":identity")
            })
            .expect("program resource");
        // Walk into program:body which holds the Match resource.
        let body = match prog.get(&Iri::parse("urn:eigenius:program:body").unwrap()) {
            Some(Value::Embedded(e)) => e.as_ref(),
            other => panic!("expected program:body Embedded, got {other:?}"),
        };
        // Body is the Match resource itself (no wrapping lambda).
        let match_resource = body;
        let motive_iri = Iri::parse("urn:eigenius:program:result_motive").unwrap();
        assert!(
            match_resource.get(&motive_iri).is_some(),
            "Lambda-motive match should emit program:result_motive"
        );
        let legacy_iri = Iri::parse("urn:eigenius:program:result_type").unwrap();
        assert!(
            match_resource.get(&legacy_iri).is_none(),
            "Lambda-motive match should NOT also emit program:result_type"
        );
    }

    #[test]
    fn compile_match_bare_ref_motive_emits_result_type_iri() {
        // Pre-Layer-3 path — `match v returning T { … }` with `T` a
        // bare type ref keeps emitting `program:result_type` as a flat
        // IRI string (preserving the old wire shape for backward
        // compatibility).
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:identity : ex:Nat -> ex:Nat {
                match input returning ex:Nat {
                    zero -> input;
                    succ(k) -> input;
                }
            }
            "#,
        );
        let prog = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str())
                    .unwrap_or("")
                    .ends_with(":identity")
            })
            .expect("program resource");
        let body = match prog.get(&Iri::parse("urn:eigenius:program:body").unwrap()) {
            Some(Value::Embedded(e)) => e.as_ref(),
            other => panic!("expected program:body Embedded, got {other:?}"),
        };
        let legacy_iri = Iri::parse("urn:eigenius:program:result_type").unwrap();
        let rt = body
            .get(&legacy_iri)
            .expect("bare-ref match should emit program:result_type");
        match rt {
            Value::String(s) => assert!(s.ends_with(":Nat")),
            other => panic!("expected String IRI, got {other:?}"),
        }
        let motive_iri = Iri::parse("urn:eigenius:program:result_motive").unwrap();
        assert!(
            body.get(&motive_iri).is_none(),
            "bare-ref match should NOT emit program:result_motive"
        );
    }

    #[test]
    fn compile_data_indexed_emits_sort_literal_index_kinds() {
        // D39 §5 / D49 ChainWitness path: when an intermediate index is a sort literal
        // (Prop / Set / Type N), the compiler emits `Sort(level)`. This asserted the canonical
        // STRINGS "Prop" / "Set" / "Type:2" that `decode_param_kind_str` recognised; eigenius#188
        // retyped `core:param_kind` to `eigentt:Term` so a level VARIABLE is expressible, and
        // the string grammar could not carry one.
        use crate::ontology::well_known as wk_local;

        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Triple : Prop -> Set -> Type 2 -> Type 3 {
                mk : forall (p : Prop) => forall (s : Set) => forall (t : Type 2) => ex:Triple(p, s, t),
            }
            "#,
        );
        let r = &resources[0];

        let indices_iri = Iri::parse(wk_local::INDICES).unwrap();
        let param_kind_iri = Iri::parse(wk_local::PARAM_KIND).unwrap();
        let arr = match r.get(&indices_iri) {
            Some(Value::Array(a)) => a,
            other => panic!("expected indices array, got {other:?}"),
        };
        assert_eq!(arr.len(), 3);

        let kinds: Vec<Value> = arr
            .iter()
            .map(|v| match v {
                Value::Embedded(e) => e.get(&param_kind_iri).expect("index has a kind").clone(),
                other => panic!("expected embedded index, got {other:?}"),
            })
            .collect();
        let sort_json = |n: usize| {
            let mut lvl = serde_json::json!({"ctor": "Zero", "args": []});
            for _ in 0..n {
                lvl = serde_json::json!({"ctor": "Succ", "args": [lvl]});
            }
            Value::Json(serde_json::json!({"ctor": "Sort", "args": [lvl]}))
        };
        assert_eq!(kinds, vec![sort_json(0), sort_json(1), sort_json(3)]);

        assert_eq!(result_sort_nat(r), Some(4));
    }

    #[test]
    fn compile_data_non_indexed_omits_indices_and_result_sort() {
        use crate::ontology::well_known as wk_local;

        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Bool {
                tt,
                ff,
            }
            "#,
        );
        let r = &resources[0];
        let indices_iri = Iri::parse(wk_local::INDICES).unwrap();
        assert!(
            r.get(&indices_iri).is_none(),
            "non-indexed data should omit `core:indices`"
        );
        assert!(
            r.get(&Iri::parse(wk_local::RESULT_SORT).unwrap()).is_none(),
            "non-indexed data without explicit `: Set` should omit `core:result_sort`"
        );
    }

    #[test]
    fn compile_data_is_stamped_as_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Bool {
                tt,
                ff,
            }
            "#,
        );
        let r = &resources[0];
        assert!(
            !has_grade_class(r),
            "no compiled resource may carry a grade class"
        );
        assert_eq!(declared_by(r), Some(UNATTRIBUTED_DECLARER.to_string()));
    }

    #[test]
    fn ctor_name_collision_is_accepted_at_declaration_time() {
        // Two inductives declaring `mk` are now both admitted into
        // the ctor index; the surface uses qualified-or-ambiguous
        // resolution at REFERENCE time instead of forbidding the
        // declaration. Bare `mk(...)` at use time becomes an
        // "ambiguous" error; `ex:mk(...)` is still ambiguous (both
        // ctors share the namespace), so a use site has to rename
        // one inductive or rely on per-inductive qualifier (the
        // latter not yet supported in the surface — tracked).
        let result = esl::compile(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Foo {
                mk,
            }

            data ex:Bar {
                mk,
            }
            "#,
        );
        result.expect("two inductives may share a ctor short name");
    }

    #[test]
    fn bare_ctor_reference_to_ambiguous_short_name_errors_at_use_site() {
        // Same two-inductive setup, but with a use site: bare `mk`
        // can't pick between `ex:Foo.mk` and `ex:Bar.mk`, so it
        // errors at the reference, not at the declaration.
        let result = esl::compile(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Foo { mk, }
            data ex:Bar { mk, }

            axiom ex:use : ex:Foo -> Prop;
            axiom ex:use_with_arg : ex:use(mk);
            "#,
        );
        let err = result.expect_err("ambiguous bare `mk` use must error");
        let msg = err
            .iter()
            .map(|e| e.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(
            msg.contains("ambiguous") && msg.contains("mk"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn qualified_ctor_in_value_slot_resolves_to_ctor_not_macro() {
        // The parser routes `ns:Name(args)` to `Value::MacroCall`
        // because at parse time it can't distinguish ctor from macro.
        // The compiler disambiguates by trying `resolve_ctor_iri`
        // first; only when no ctor matches does it fall through to
        // macro expansion. Without that order, `justification:App(...)`
        // in a `justification:term = ...` slot errors with
        // "macro not declared" instead of resolving to the
        // `justification:Term.App` ctor.
        let resources = esl::compile(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:eigenius:example";

            data ex:Foo {
                Mk(core:string),
                Compose(ex:Foo, ex:Foo),
            }

            resource ex:my_resource : ex:Foo {
                ex:slot = ex:Compose(
                    ex:Mk("a"),
                    ex:Mk("b")
                );
            }
            "#,
        )
        .expect("qualified ctor in value slot must resolve as a ctor, not a macro");
        // The resource should commit (no error); we don't introspect
        // the encoded value further — the success path is the contract.
        assert!(!resources.is_empty());
    }

    #[test]
    fn alias_substitution_in_type_expr_produces_same_encoding_as_inlined_form() {
        // The `alias ... in body` form is pure compile-time
        // substitution. Two resources — one using `alias` and one
        // with the bindings inlined — must produce byte-identical
        // `canonical_proposition` encodings. If they don't, the
        // alias is leaking into the D47 shape, which would break the
        // chain-witness hashing contract.
        let resources = esl::compile(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:eigenius:example";
            namespace ref  = "urn:eigenius:reflection";
            namespace prov = "urn:eigenius:prov";

            data ex:HasLowIC50 : core:string -> Prop {
            }

            resource ex:with_alias : core:Class {
                prov:was_attributed_to = "test:alias";
                ref:canonical_proposition = type_expr(
                    alias EIG = "urn:ex:EIG_0291"
                    in
                    ex:HasLowIC50(EIG)
                );
            }

            resource ex:without_alias : core:Class {
                prov:was_attributed_to = "test:alias";
                ref:canonical_proposition = type_expr(
                    ex:HasLowIC50("urn:ex:EIG_0291")
                );
            }
            "#,
        )
        .expect("both forms compile");

        let prop_iri = iri("urn:eigenius:reflection:canonical_proposition");
        let with_alias = resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:example:with_alias"))
            .expect("with_alias resource present");
        let without_alias = resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:example:without_alias"))
            .expect("without_alias resource present");
        assert_eq!(
            with_alias.get(&prop_iri),
            without_alias.get(&prop_iri),
            "alias-expanded form must produce the same canonical_proposition \
             JSON as the inlined form — the alias is pure compile-time sugar."
        );
    }

    #[test]
    fn alias_lexical_scope_shadows_forall_binders_when_appropriate() {
        // Sequential lexical scope check: each later binding can
        // reference earlier ones, and forall/fun binders shadow alias
        // names in their own bodies.
        //
        // The body uses `forall (x : core:string) => ex:HasLowIC50(x)` —
        // here `x` is the forall-bound variable, NOT the alias `x`.
        // The alias substitution must NOT replace the forall-bound `x`.
        let resources = esl::compile(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:eigenius:example";
            namespace ref  = "urn:eigenius:reflection";
            namespace prov = "urn:eigenius:prov";

            data ex:HasLowIC50 : core:string -> Prop {
            }

            resource ex:scope_test : core:Class {
                prov:was_attributed_to = "test:scope";
                ref:canonical_proposition = type_expr(
                    alias x = "urn:ex:SHOULD_NOT_LEAK"
                    in
                    forall (x : core:string) => ex:HasLowIC50(x)
                );
            }

            resource ex:scope_expected : core:Class {
                prov:was_attributed_to = "test:scope";
                ref:canonical_proposition = type_expr(
                    forall (x : core:string) => ex:HasLowIC50(x)
                );
            }
            "#,
        )
        .expect("scope-shadowing form compiles");

        let prop_iri = iri("urn:eigenius:reflection:canonical_proposition");
        let scope_test = resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:example:scope_test"))
            .unwrap();
        let scope_expected = resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:example:scope_expected"))
            .unwrap();
        assert_eq!(
            scope_test.get(&prop_iri),
            scope_expected.get(&prop_iri),
            "the forall binder `x` must shadow the alias `x` in its body — \
             the alias must not leak its `urn:ex:SHOULD_NOT_LEAK` value into \
             the forall-bound proposition."
        );
    }

    // --- D37: lambda / pi / merge_comorphism lowering ---

    #[test]
    fn typed_lambda_literal_emits_parameter_type() {
        // Inside a `program` body, a typed lambda literal lowers to
        // a Lambda resource whose `parameter_type` is the class IRI.
        // The untyped `\x -> e` form (verified by existing tests)
        // omits `parameter_type`; this test pins the typed shape.
        let resources = compile_esl(
            r#"
            namespace ex = "urn:ex";
            program ex:identity : ex:A -> ex:A {
                lambda x : ex:A => x
            }
            "#,
        );
        // The program resource is at index 0; its `body` embeds the
        // Lambda. Walk into it and verify `parameter_type` is set.
        let prog = &resources[0];
        let body = prog
            .get(&iri("urn:eigenius:program:body"))
            .expect("program has body");
        let body_r = match body {
            Value::Embedded(b) => b,
            other => panic!("expected embedded body, got {other:?}"),
        };
        // Body's is_a should include Lambda.
        let is_a = body_r.is_a();
        assert!(
            is_a.iter()
                .any(|c| c.as_str() == "urn:eigenius:program:Lambda"),
            "expected Lambda is_a, got {is_a:?}"
        );
        let pt = body_r
            .get(&iri("urn:eigenius:program:parameter_type"))
            .expect("typed lambda must emit parameter_type");
        assert_eq!(
            pt.as_iri_str(),
            Some("urn:ex:A"),
            "expected parameter_type IRI = urn:ex:A, got {pt:?}"
        );
    }

    #[test]
    fn merge_comorphism_reference_form_lowers_to_one_resource() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b for ex:Patient {
                transformation = ex:take_b_term;
            }
            "#,
        );
        assert_eq!(
            resources.len(),
            1,
            "reference form should produce exactly one resource"
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:ex:take_b");
        let is_a = r.is_a();
        assert!(
            is_a.iter()
                .any(|c| c.as_str() == crate::ontology::well_known::MERGE_COMORPHISM),
            "expected MergeComorphism is_a, got {is_a:?}"
        );
        let target_class = r
            .get(&iri(crate::ontology::well_known::MERGE_TARGET_CLASS))
            .expect("merge_target_class must be set");
        assert_eq!(target_class.as_iri_str(), Some("urn:ex:Patient"));
        let transformation = r
            .get(&iri(crate::ontology::well_known::MERGE_TRANSFORMATION))
            .expect("merge_transformation must be set");
        assert_eq!(transformation.as_iri_str(), Some("urn:ex:take_b_term"));
    }

    #[test]
    fn merge_comorphism_inline_form_lowers_to_two_resources() {
        // The inline form emits both the synthesised standalone
        // Lambda (at a content-hash IRI) and the MergeComorphism
        // resource pointing at it.
        let resources = compile_esl(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b for ex:Patient {
                (a, b, opt) => b
            }
            "#,
        );
        assert_eq!(
            resources.len(),
            2,
            "inline form should produce two resources (lambda + comorphism)"
        );

        // First resource: the synthesised lambda at an
        // `urn:eigenius:auto:lambda:<hex>` IRI.
        let lambda_r = &resources[0];
        let lambda_iri = lambda_r.id().unwrap().as_str().to_string();
        assert!(
            lambda_iri.starts_with("urn:eigenius:auto:lambda:"),
            "lambda IRI should be content-hash form, got {lambda_iri}"
        );
        let lambda_is_a = lambda_r.is_a();
        assert!(
            lambda_is_a
                .iter()
                .any(|c| c.as_str() == "urn:eigenius:program:Lambda"),
            "expected Lambda is_a, got {lambda_is_a:?}"
        );
        // The outermost lambda binds `a` and carries `program:type`
        // with the full Pi-term `pi a : C, b : C, opt : Option<C> => C`.
        let param = lambda_r
            .get(&iri("urn:eigenius:program:parameter"))
            .and_then(|v| v.as_str())
            .expect("outermost lambda binds the first parameter `a`");
        assert_eq!(param, "a");
        assert!(
            lambda_r
                .get(&iri(crate::ontology::well_known::PROGRAM_TYPE))
                .is_some(),
            "outermost synthesised lambda must carry `program:type`"
        );

        // Second resource: the MergeComorphism pointing at the lambda.
        let comorphism_r = &resources[1];
        assert_eq!(comorphism_r.id().unwrap().as_str(), "urn:ex:take_b");
        assert_eq!(
            comorphism_r
                .get(&iri(crate::ontology::well_known::MERGE_TARGET_CLASS))
                .and_then(|v| v.as_iri_str()),
            Some("urn:ex:Patient")
        );
        assert_eq!(
            comorphism_r
                .get(&iri(crate::ontology::well_known::MERGE_TRANSFORMATION))
                .and_then(|v| v.as_iri_str()),
            Some(lambda_iri.as_str()),
            "comorphism's `merge_transformation` should point at the synthesised lambda's IRI"
        );
    }

    #[test]
    fn merge_comorphism_inline_form_dedupes_via_content_hash() {
        // Re-declaring the same inline body (regardless of
        // comorphism name + target class differences in the
        // surrounding wrapper) should produce a synthesised lambda
        // at the SAME content-hash IRI, because the hash is over
        // the lambda's structural content with @id cleared.
        let resources_a = compile_esl(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b_v1 for ex:Patient {
                (a, b, opt) => b
            }
            "#,
        );
        let resources_b = compile_esl(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b_v2 for ex:Patient {
                (a, b, opt) => b
            }
            "#,
        );
        let lambda_iri_a = resources_a[0].id().unwrap().as_str();
        let lambda_iri_b = resources_b[0].id().unwrap().as_str();
        assert_eq!(
            lambda_iri_a, lambda_iri_b,
            "structurally-identical inline bodies must hash to the same IRI"
        );
    }

    #[test]
    fn merge_comorphism_inline_form_rejects_wrong_arity() {
        // The inline body's signature is fixed to (a, b, opt) — a
        // wrong arity produces a structured compile error.
        let result = esl::compile(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b for ex:Patient {
                (only_one) => only_one
            }
            "#,
        );
        let err = result.expect_err("wrong arity must be rejected");
        let msg = err[0].message.clone();
        assert!(
            msg.contains("3 parameters") || msg.contains("witness signature"),
            "expected arity error mentioning 3 parameters, got: {msg}"
        );
    }

    // --- D37 §9: worked-example round-trip tests ---
    //
    // Each test compiles the worked example from D37 §9.x through
    // the ESL pipeline and verifies the produced resource pair
    // (synthesised Lambda + MergeComorphism) has the expected shape.
    // These are compile-only smoke tests — the validator-side check
    // for §9.1 is exercised by
    // `compiler_output_validates_clean_end_to_end` in
    // `validation::tests`. §9.2–9.4 require a richer chain (Patient
    // with `description`/`weight` properties, `core:add`/`core:divide`
    // operators) before the Rule 19 NbE check can run; the compile
    // tests below pin the lowering shape regardless.

    #[test]
    fn d37_worked_example_9_1_take_side_b() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:project";
            merge_comorphism ex:patient_take_b for ex:Patient {
                (a, b, opt) => b
            }
            "#,
        );
        assert_eq!(resources.len(), 2, "inline form emits lambda + comorphism");
        // Synthesised lambda: outermost binder is `a`, body chain
        // terminates in a Var resource pointing at `b`.
        let lambda = &resources[0];
        assert!(lambda
            .id()
            .unwrap()
            .as_str()
            .starts_with("urn:eigenius:auto:lambda:"));
        // Comorphism: pinned for the Patient class, points at the
        // synthesised lambda.
        let comorphism = &resources[1];
        assert_eq!(
            comorphism.id().unwrap().as_str(),
            "urn:project:patient_take_b"
        );
        assert_eq!(
            comorphism
                .get(&iri(crate::ontology::well_known::MERGE_TARGET_CLASS))
                .and_then(|v| v.as_iri_str()),
            Some("urn:project:Patient")
        );
    }

    #[test]
    fn d37_worked_example_9_2_field_merge() {
        // Take A's description and B's weight, build a fresh
        // Patient. Uses `Construct` (Σ-introduction) + `Project`
        // (Σ-elimination via `a.description`).
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:project";

            merge_comorphism ex:patient_merge_fields for ex:Patient {
                (a, b, opt) => Construct ex:Patient {
                    ex:description = a.ex:description,
                    ex:weight      = b.ex:weight
                }
            }
            "#,
        );
        assert_eq!(resources.len(), 2);
        let comorphism = &resources[1];
        assert_eq!(
            comorphism.id().unwrap().as_str(),
            "urn:project:patient_merge_fields"
        );
    }

    #[test]
    fn d37_worked_example_9_3_arithmetic_average() {
        // Average a's and b's weight via chain-committed
        // `core:add` + `core:divide` operators. Uses `Apply` over
        // those operator IRIs.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:project";

            merge_comorphism ex:patient_avg_weight for ex:Patient {
                (a, b, opt) => Construct ex:Patient {
                    ex:description = a.ex:description,
                    ex:weight      = core:divide(core:add(a.ex:weight, b.ex:weight), 2.0)
                }
            }
            "#,
        );
        assert_eq!(resources.len(), 2);
        let comorphism = &resources[1];
        assert_eq!(
            comorphism.id().unwrap().as_str(),
            "urn:project:patient_avg_weight"
        );
    }

    #[test]
    fn d37_worked_example_9_4_ancestor_aware() {
        // Match over Option<Patient> for the ancestor argument,
        // branching on whether the ancestor disagrees with A. Uses
        // `Match` over the `Option` inductive's two constructors.
        //
        // The ESL compile pass (Phase 11b) requires constructors
        // referenced in `match` arms to be declared via a `data`
        // block in the *same file*. `Option` is committed in the
        // core ontology rather than re-declared per file, so the
        // worked example needs a local `data` shadowing for the
        // compile-time ctor lookup to find `some` / `none`.
        // Lifting that restriction (so chain-committed inductives'
        // constructors are reachable from `match`) is tracked as a
        // separate ESL extension; until then the worked example
        // declares Option locally to exercise the lowering path.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex   = "urn:project";

            data ex:Option(A : Set) {
                none,
                some(A),
            }

            merge_comorphism ex:patient_ancestor_aware for ex:Patient {
                (a, b, opt) => match opt {
                    some(ancestor) -> a;
                    none -> a;
                }
            }
            "#,
        );
        // 3 resources: the local Option `data` decl + lambda + comorphism.
        assert!(
            resources.len() >= 2,
            "expected at least lambda + comorphism, got {} resources",
            resources.len()
        );
        let comorphism = resources
            .iter()
            .find(|r| {
                r.id()
                    .is_some_and(|i| i.as_str() == "urn:project:patient_ancestor_aware")
            })
            .expect("comorphism resource should be present");
        assert_eq!(
            comorphism
                .get(&iri(crate::ontology::well_known::MERGE_TARGET_CLASS))
                .and_then(|v| v.as_iri_str()),
            Some("urn:project:Patient")
        );
    }

    // --- D43 §3.1 — text_index / vector_index compile stub behaviour (M1) ---

    /// M1 lands the AST + parser for `text_index`; the lowering to a
    /// `core:TextIndex` Resource is M2 work. The compile step emits
    /// a clear "not yet implemented" error so users get a meaningful
    /// signal until M2 lands.
    #[test]
    fn text_index_compile_emits_not_yet_implemented_until_m2() {
        let errs = esl::compile(
            r#"
            namespace ex = "urn:ex";
            namespace core = "urn:eigenius:core";
            text_index ex:description_en {
                core:target_property = ex:description;
                core:text_analyzer = "en-stem-v1";
            }
            "#,
        )
        .expect_err("text_index compilation should fail with M1 stub");
        let combined = errs
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            combined.contains("text_index") && combined.contains("M2"),
            "error should reference text_index and D43 M2, got: {combined}"
        );
    }

    /// Same shape for `vector_index` — M1 parses, M2 lowers.
    #[test]
    fn vector_index_compile_emits_not_yet_implemented_until_m2() {
        let errs = esl::compile(
            r#"
            namespace ex = "urn:ex";
            namespace core = "urn:eigenius:core";
            vector_index ex:description_oai {
                core:target_property = ex:description;
                core:vec_model = ex:openai_text_embedding_3_large_v3;
                core:vec_dim = 1536;
            }
            "#,
        )
        .expect_err("vector_index compilation should fail with M1 stub");
        let combined = errs
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            combined.contains("vector_index") && combined.contains("M2"),
            "error should reference vector_index and D43 M2, got: {combined}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // eigenius#72 Layer 1 — `axiom` declarations
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn compile_trivial_axiom() {
        // axiom triv : Prop → Prop
        let resources = compile_esl(
            r#"
            namespace eg = "urn:eigenius:test";

            axiom eg:triv : Prop -> Prop;
            "#,
        );
        let ax = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:eigenius:test:triv")
                    .unwrap_or(false)
            })
            .expect("axiom triv should be committed");
        let is_a = ax.is_a();
        assert!(
            is_a.iter()
                .any(|i| i.as_str() == "urn:eigenius:eigentt:Axiom"),
            "axiom must be classed as eigentt:Axiom; got is_a = {:?}",
            is_a.iter().map(|i| i.as_str()).collect::<Vec<_>>()
        );
        // The axiom_statement value is the encoded Term.
        let stmt = ax
            .get(&iri("urn:eigenius:eigentt:axiom_statement"))
            .expect("axiom_statement property must be set");
        match stmt {
            Value::Json(j) => {
                // The outer shape should be a Pi (encoded by the
                // D47 codec): {ctor: "Pi", args: ["", <Sort 0>, <Sort 0>]}.
                assert_eq!(j["ctor"], "Pi");
                let args = j["args"].as_array().expect("Pi has args");
                assert_eq!(args[0], serde_json::json!(""));
                assert_eq!(args[1]["ctor"], "Sort");
                assert_eq!(args[1]["args"][0]["ctor"], "Zero");
                assert_eq!(args[2]["ctor"], "Sort");
                assert_eq!(args[2]["args"][0]["ctor"], "Zero");
            }
            other => panic!("expected Value::Json, got {other:?}"),
        }
    }

    #[test]
    fn compile_axiom_with_forall() {
        // axiom myax : forall (P : Prop) => P -> P
        let resources = compile_esl(
            r#"
            namespace eg = "urn:eigenius:test";

            axiom eg:myax : forall (P : Prop) => P -> P;
            "#,
        );
        let ax = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:eigenius:test:myax")
                    .unwrap_or(false)
            })
            .expect("axiom myax should be committed");
        let stmt = ax
            .get(&iri("urn:eigenius:eigentt:axiom_statement"))
            .expect("axiom_statement set");
        match stmt {
            Value::Json(j) => {
                // forall (P : Prop) => P -> P
                //   lowers to Pi(P : Sort(0), Pi(_ : Var(P), Var(P)))
                //   encodes as Pi("P", Sort(0), Pi("", Var("P"), Var("P")))
                assert_eq!(j["ctor"], "Pi");
                assert_eq!(j["args"][0], "P");
                assert_eq!(j["args"][1]["ctor"], "Sort");
                assert_eq!(j["args"][1]["args"][0]["ctor"], "Zero");
                let inner = &j["args"][2];
                assert_eq!(inner["ctor"], "Pi");
                assert_eq!(inner["args"][0], "");
                assert_eq!(inner["args"][1]["ctor"], "Var");
                assert_eq!(inner["args"][1]["args"][0], "P");
                assert_eq!(inner["args"][2]["ctor"], "Var");
                assert_eq!(inner["args"][2]["args"][0], "P");
            }
            other => panic!("expected Value::Json, got {other:?}"),
        }
    }

    #[test]
    fn compile_axiom_with_justification_note() {
        let resources = compile_esl(
            r#"
            namespace eg = "urn:eigenius:test";

            axiom eg:noted : Prop -> Prop note: "Methodological convention from working group X";
            "#,
        );
        let ax = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:eigenius:test:noted")
                    .unwrap_or(false)
            })
            .expect("axiom noted committed");
        let just = ax
            .get(&iri("urn:eigenius:eigentt:axiom_justification"))
            .expect("axiom_justification set");
        match just {
            Value::String(s) => {
                assert_eq!(s, "Methodological convention from working group X");
            }
            other => panic!("expected Value::String, got {other:?}"),
        }
    }

    #[test]
    fn reasoning_ontology_esl_compiles() {
        // D39 Phase 3 — the authored justification.esl source must compile
        // cleanly. Locks the structural contract: namespace declarations,
        // the `justification:Term` five-ctor inductive, and the
        // `justification:Certificate` seven-ctor indexed inductive predicate.
        // Any future edit to the file or to the ESL surface that breaks this
        // round-trip needs to be deliberate.
        let source = include_str!("../../../ontologies/justification/justification.esl");
        let resources = esl::compile(source).expect("justification.esl must compile");

        // Expect: 1 justification:Term + 1 justification:Certificate.
        // The three `witness:Is*As` predicates were here until P7 and are NOT
        // any more — see below.
        let inductive_iri = iri(crate::ontology::well_known::INDUCTIVE_TYPE);
        let inductives: Vec<_> = resources
            .iter()
            .filter(|r| r.is_a().iter().any(|c| c == &inductive_iri))
            .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
            .collect();
        assert!(
            inductives.len() >= 2,
            "expected at least 2 inductive Resources in justification.esl, found {}: {inductives:?}",
            inductives.len()
        );

        // **The witness types are declared in CORE, not here.** The kernel
        // constructs their inhabitants (`layer::synthesize_chain_witness`), so a
        // type the kernel inhabits cannot be owned by a layer above it: while they
        // lived here, an edit to this file could change the arity `check_hooks.rs`
        // assumes, or remove the declaration it resolves by IRI. The certificate
        // ctors below still REFERENCE them across the layer boundary, which is the
        // ordinary direction and is what the rest of this test exercises.
        for witness in [
            crate::ontology::well_known::CHAIN_WITNESS_IS_DECLARED_AS,
            crate::ontology::well_known::CHAIN_WITNESS_IS_OBSERVED_AS,
            crate::ontology::well_known::CHAIN_WITNESS_IS_VERIFIED_AS,
        ] {
            assert!(
                !inductives.iter().any(|i| i == witness),
                "`{witness}` must be declared in core-ontology.json, not justification.esl"
            );
        }

        // The two resource classes. TaskOutput is intentionally not here — D39 §4.4
        // justifies it entirely by the discipline-thesis benchmark work (D50/D51), so it
        // lives with the benchmark harness.
        let class_iri = iri(crate::ontology::well_known::CLASS);
        for expected in &[
            "urn:eigenius:justification:Conclusion",
            "urn:eigenius:justification:VerifiedPropositionView",
        ] {
            assert!(
                resources
                    .iter()
                    .any(|r| r.id().map(|i| i.as_str() == *expected).unwrap_or(false)
                        && r.is_a().iter().any(|c| c == &class_iri)),
                "justification.esl missing class declaration for {expected}"
            );
        }

        // **`urn:eigenius:reasoning` names nothing.** P7 deleted the institution resource,
        // its ExportFormat, all four QueryClasses, and the EntailmentRequest /
        // ConsistencyRequest input classes — ValidateJustification was absorbed into commit
        // by P2, EntailmentQuery's question is a witness-index lookup, ConsistencyCheck
        // returned Undecidable for every non-empty input, and ProjectJustification's algebra
        // moved to `kernel/src/justification/` at P6.0. With no handler left the institution
        // hosted nothing. This asserts the namespace stays vacated.
        for r in &resources {
            if let Some(id) = r.id() {
                assert!(
                    !id.as_str().starts_with("urn:eigenius:reasoning"),
                    "`{}` — the reasoning namespace was retired at P7 and must stay empty",
                    id.as_str()
                );
            }
        }
    }

    #[test]
    fn core_declares_the_three_witness_predicates_with_the_arity_the_kernel_assumes() {
        // The other half of the P7 move asserted above. `check_hooks.rs` resolves these three
        // IRIs and then GUARDS the shape it finds — "expected 2 indices (iri, P), got {n} …
        // the chain ontology drifted from the kernel's expectation". That guard existed
        // because the declaration was owned by a layer the kernel does not control. It is
        // owned by core now, so the drift it guards against is checked here instead of only
        // being diagnosed at synthesis time.
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).expect("core-ontology parses");
        let inductive_iri = iri(crate::ontology::well_known::INDUCTIVE_TYPE);

        for expected in &[
            crate::ontology::well_known::CHAIN_WITNESS_IS_DECLARED_AS,
            crate::ontology::well_known::CHAIN_WITNESS_IS_OBSERVED_AS,
            crate::ontology::well_known::CHAIN_WITNESS_IS_VERIFIED_AS,
        ] {
            let r = resources
                .iter()
                .find(|r| r.id().map(|i| i.as_str() == *expected).unwrap_or(false))
                .unwrap_or_else(|| panic!("core-ontology.json must declare `{expected}`"));
            assert!(
                r.is_a().iter().any(|c| c == &inductive_iri),
                "`{expected}` must be a core:InductiveType"
            );

            // `core:string -> Prop -> Prop`: two indices, and the hook reads `indices[0]` as
            // the IRI and `indices[1]` as the proposition.
            let indices = r
                .get(&iri(crate::ontology::well_known::INDICES))
                .and_then(|v| match v {
                    Value::Array(a) => Some(a.len()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("`{expected}` must carry core:indices as an array"));
            assert_eq!(indices, 2, "`{expected}` must have 2 indices (iri, P)");

            // Zero ctors is what makes the type opaque: the kernel's synthesis against a
            // committed trace is the ONLY route to an inhabitant. A ctor here would let an
            // author write down a witness, which is the whole thing the design forbids.
            let ctors = r
                .get(&iri(crate::ontology::well_known::CTORS))
                .and_then(|v| match v {
                    Value::Array(a) => Some(a.len()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("`{expected}` must carry core:ctors as an array"));
            assert_eq!(
                ctors, 0,
                "`{expected}` must have zero ctors — no user-constructible inhabitant"
            );
        }
    }

    #[test]
    fn reasoning_ontology_resolves_through_codec() {
        // End-to-end sanity check: reasoning.esl compiled on top of the
        // core ontology resolves cleanly through `resolve_class_type`.
        // Exercises (a) the new Sort-typed-index path (justification:Certificate's
        // `Prop` index), (b) the codec self-reference short-circuit
        // (justification:Certificate's ctors reference justification:Certificate itself), and
        // (c) cross-inductive references (justification:Certificate → ChainWitness +
        // justification:Term). If any of these regress, the full Phase 6
        // synthesis path breaks.
        use crate::layer::LayerBuilder;
        use crate::ontology::eigon_json;
        use crate::program::ground::resolve_class_type;
        use std::sync::Arc;

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(crate::layer::LayerStorage::in_memory()));

        // Phase 4 — the resource classes (justification:Conclusion, TaskOutput,
        // VerifiedPropositionView) declare `subclass_of
        // reflection:DerivedResource`, so reflection-ontology has to be
        // in the layer chain before reasoning.esl loads.
        let reflection_json =
            include_str!("../../../ontologies/reflection/reflection-ontology.json");
        let reflection_resources = eigon_json::parse_document(reflection_json).unwrap();
        let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
        for r in reflection_resources {
            reflection_builder.add_resource(r).unwrap();
        }
        // eigentt:Term is referenced from justification:proposition /
        // justification:certificate via class_types; load the fragment too.
        let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
        let eigentt_resources = eigon_json::parse_document(eigentt_json).unwrap();
        for r in eigentt_resources {
            reflection_builder.add_resource(r).unwrap();
        }
        let reflection =
            Arc::new(reflection_builder.build(crate::layer::LayerStorage::in_memory()));

        let source = include_str!("../../../ontologies/justification/justification.esl");
        let user_resources = esl::compile(source).expect("reasoning.esl must compile");
        let mut user_builder = LayerBuilder::new("justification", Some(reflection));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(user_builder.build(crate::layer::LayerStorage::in_memory()));

        // The five inductive types — Phase 3.
        for iri_str in &[
            "urn:eigenius:witness:IsDeclaredAs",
            "urn:eigenius:witness:IsObservedAs",
            "urn:eigenius:witness:IsVerifiedAs",
            "urn:eigenius:justification:Term",
            "urn:eigenius:justification:Certificate",
        ] {
            let class_iri = Iri::parse(iri_str).unwrap();
            resolve_class_type(&class_iri, &layer)
                .unwrap_or_else(|e| panic!("failed to resolve {iri_str}: {e}"));
        }

        // The three resource classes — Phase 4. `resolve_class_type` on
        // a regular Class returns the Σ-chain of its required +
        // recommended properties; we just check that resolution
        // succeeds (the structural contract is "all referenced
        // properties exist and have decoded types"). A failure here
        // would mean a property declaration is malformed or references
        // an unresolved class.
        for iri_str in &[
            "urn:eigenius:justification:Conclusion",
            "urn:eigenius:justification:VerifiedPropositionView",
        ] {
            let class_iri = Iri::parse(iri_str).unwrap();
            resolve_class_type(&class_iri, &layer)
                .unwrap_or_else(|e| panic!("failed to resolve {iri_str}: {e}"));
        }
    }

    #[test]
    fn type_expr_value_encodes_d47_inline_on_resource_property() {
        // `type_expr(<type-expr>)` — inline D47 surface for resource
        // fields. Mirrors `formula(...)` for D32 inductive values.
        // The encoded shape on the property must match what an
        // equivalent top-level `axiom` declaration produces.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace eg   = "urn:eigenius:test:typeexpr";

            class eg:Holder {
                requires eg:body;
            }
            property eg:body : core:resource {
                class_types eigentt:Term;
            }
            namespace eigentt = "urn:eigenius:eigentt";

            resource eg:r1 : eg:Holder {
                eg:body = type_expr(forall (A : Set) => A -> A);
            }
            "#,
        );
        let holder = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:eigenius:test:typeexpr:r1")
                    .unwrap_or(false)
            })
            .expect("eg:r1 committed");
        let body = holder
            .get(&iri("urn:eigenius:test:typeexpr:body"))
            .expect("eg:body set");
        match body {
            Value::Json(j) => {
                // forall (A : Set) => A -> A
                //   → Pi("A", Sort(1), Pi("", Var("A"), Var("A")))
                assert_eq!(j["ctor"], "Pi");
                assert_eq!(j["args"][0], "A");
                assert_eq!(j["args"][1]["ctor"], "Sort");
                assert_eq!(j["args"][1]["args"][0]["ctor"], "Succ");
                assert_eq!(j["args"][1]["args"][0]["args"][0]["ctor"], "Zero");
                let inner = &j["args"][2];
                assert_eq!(inner["ctor"], "Pi");
                assert_eq!(inner["args"][1]["ctor"], "Var");
                assert_eq!(inner["args"][1]["args"][0], "A");
            }
            other => panic!("expected Value::Json, got {other:?}"),
        }
    }

    #[test]
    fn axiom_uses_set_keyword_in_kind_position() {
        // ESL's `Set` keyword in a `forall` binder kind position must
        // be recognised as a sort literal, not as an identifier.
        let resources = compile_esl(
            r#"
            namespace eg = "urn:eigenius:test";

            axiom eg:id_at_set : forall (A : Set) => A -> A;
            "#,
        );
        let ax = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:eigenius:test:id_at_set")
                    .unwrap_or(false)
            })
            .expect("axiom id_at_set committed");
        let stmt = ax
            .get(&iri("urn:eigenius:eigentt:axiom_statement"))
            .expect("axiom_statement set");
        if let Value::Json(j) = stmt {
            // Outermost Pi, binder "A", binder kind Sort(1) = Set.
            assert_eq!(j["ctor"], "Pi");
            assert_eq!(j["args"][0], "A");
            assert_eq!(j["args"][1]["ctor"], "Sort");
            assert_eq!(j["args"][1]["args"][0]["ctor"], "Succ");
            assert_eq!(j["args"][1]["args"][0]["args"][0]["ctor"], "Zero");
        }
    }

    // ────────────────────────────────────────────────────────────────
    // D52 §12 — macro declarations and call-site expansion
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn statistics_ontology_esl_compiles() {
        // D52 Phase 1 — the authored statistics.esl source must
        // compile cleanly. Locks the structural contract: five axis
        // enums, the SampleSet product type, the smart-constructor
        // macros (SingleSampleEstimate, IID), the StatisticalAnalysisPlan
        // resource class with the universal-schema fields, the
        // PopulationLevel/MeasurementLevel scope markers, and the
        // statistics-institution + qc_validate_analysis_plan
        // resources. Any future edit that breaks this needs to be
        // deliberate.
        let source = include_str!("../../../ontologies/statistics/statistics.esl");
        let resources = esl::compile(source).expect("statistics.esl must compile");

        // Expect at least:
        //  - 5 axis enums (Randomization, Blocking, FactorDesign,
        //    Replication, RepeatedMeasuresAxis)
        //  - 5 universal-claim sum types (EffectSize, Directionality,
        //    VarianceAssumption, AutocorrelationStructure, OutlierExclusion)
        //  - SampleSet (1)
        // = 11 inductive Resources.
        let inductive_iri = iri(crate::ontology::well_known::INDUCTIVE_TYPE);
        let ind_count = resources
            .iter()
            .filter(|r| r.is_a().iter().any(|c| c == &inductive_iri))
            .count();
        assert!(
            ind_count >= 15,
            "expected at least 15 inductive Resources in statistics.esl, found {ind_count}"
        );

        // The two smart-constructor macros emit no resources; verify
        // the count is what we'd get from declarations alone.
        let has_sample_set = resources.iter().any(|r| {
            r.id()
                .map(|i| i.as_str() == "urn:eigenius:measurements:SampleSet")
                .unwrap_or(false)
        });
        assert!(has_sample_set, "stats:SampleSet inductive must be emitted");

        let has_institution = resources.iter().any(|r| {
            r.id()
                .map(|i| i.as_str() == "urn:eigenius:measurements:statistics_institution")
                .unwrap_or(false)
        });
        assert!(
            has_institution,
            "stats:statistics_institution resource must be emitted"
        );

        let has_qc = resources.iter().any(|r| {
            r.id()
                .map(|i| i.as_str() == "urn:eigenius:measurements:qc_validate_analysis_plan")
                .unwrap_or(false)
        });
        assert!(
            has_qc,
            "qc_validate_analysis_plan QueryClass must be emitted"
        );
    }

    #[test]
    fn macro_call_expands_into_ctor_app() {
        // Smoke test for the smart-constructor pattern D52 §4.2 needs:
        // a `macro` declaration produces no chain resource on its own,
        // but a call site lowers to the substituted ctor application
        // exactly as if the author had hand-written it.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace eg   = "urn:eigenius:test:macro";

            data eg:Pair(A : Set, B : Set) {
                Both(A, B),
            }

            class eg:Holder {
                requires eg:body;
            }
            property eg:body : core:resource {
                class_types eg:Pair;
            }

            macro eg:swap_both(a : core:string, b : core:string) : eg:Pair =>
                Both(b, a);

            resource eg:r1 : eg:Holder {
                eg:body = eg:swap_both("first", "second");
            }
            "#,
        );
        // Two resources expected: the Pair data declaration emits one
        // (the inductive type itself) and the eg:r1 holder. Macros emit
        // nothing on their own.
        let holder = resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:eigenius:test:macro:r1")
                    .unwrap_or(false)
            })
            .expect("eg:r1 committed");
        let body = holder
            .get(&iri("urn:eigenius:test:macro:body"))
            .expect("eg:body set");
        match body {
            Value::Json(j) => {
                // Expansion: swap_both("first", "second") substitutes
                // into Both(b, a) → Both("second", "first") — the
                // positional swap is what proves substitution happened.
                assert_eq!(j["ctor"], "Both");
                assert_eq!(j["args"][0], "second");
                assert_eq!(j["args"][1], "first");
            }
            other => panic!("expected Value::Json (CtorApp serialization), got {other:?}"),
        }
    }

    #[test]
    fn macro_unknown_name_errors_cleanly() {
        // A call site referencing an undeclared macro IRI must surface
        // a clear compile error rather than panicking or producing a
        // confusing downstream diagnostic.
        let result = esl::compile(
            r#"
            namespace core = "urn:eigenius:core";
            namespace eg   = "urn:eigenius:test:macro";

            class eg:Holder { requires eg:body; }
            property eg:body : core:string { }

            resource eg:r1 : eg:Holder {
                eg:body = eg:undefined_macro("anything");
            }
            "#,
        );
        let err = result.expect_err("undeclared macro should error");
        assert!(
            err.iter()
                .any(|e| format!("{e:?}").contains("is not declared")),
            "diagnostic should name the undeclared macro: got {err:?}"
        );
    }

    #[test]
    fn macro_arity_mismatch_errors_cleanly() {
        let result = esl::compile(
            r#"
            namespace core = "urn:eigenius:core";
            namespace eg   = "urn:eigenius:test:macro";

            data eg:Wrap { Hold(core:string), }
            class eg:Holder { requires eg:body; }
            property eg:body : core:resource { class_types eg:Wrap; }

            macro eg:two_args(a : core:string, b : core:string) : eg:Wrap =>
                Hold(a);

            resource eg:r1 : eg:Holder {
                eg:body = eg:two_args("only_one");
            }
            "#,
        );
        let err = result.expect_err("arity mismatch should error");
        assert!(
            err.iter()
                .any(|e| format!("{e:?}").contains("expects 2 argument")),
            "diagnostic should name the expected vs actual arity: got {err:?}"
        );
    }
}

#[cfg(test)]
mod sigma_surface_tests {
    use crate::esl;

    fn axiom_statement(src: &str) -> serde_json::Value {
        let rs = esl::compile(src).expect("compiles");
        let a = rs
            .iter()
            .find(|r| r.id().is_some_and(|i| i.as_str().ends_with(":t")))
            .expect("axiom resource");
        match a
            .get(&crate::ontology::iri::Iri::parse("urn:eigenius:eigentt:axiom_statement").unwrap())
            .expect("axiom_statement")
        {
            crate::ontology::resource::Value::Json(j) => j.clone(),
            other => panic!("expected Json, got {other:?}"),
        }
    }

    const NS: &str = r#"
        namespace core = "urn:eigenius:core";
        namespace eigentt = "urn:eigenius:eigentt";
        namespace p = "urn:eigenius:probe";
    "#;

    /// `exists x : T => B` is the Sigma binder — the dual of `forall`, and the form every
    /// definite description the DCG produces needs (`the(Sig x : C. P(x)).1`).
    #[test]
    fn exists_lowers_to_sig() {
        let j = axiom_statement(&format!(
            "{NS} axiom p:t : exists x : core:string => core:string"
        ));
        assert_eq!(j["ctor"], "Sig", "got {j}");
        assert_eq!(j["args"][0], "x");
    }

    /// Binders nest rightmost-innermost, exactly as `forall` does.
    #[test]
    fn exists_binder_list_nests_like_forall() {
        let j = axiom_statement(&format!(
            "{NS} axiom p:t : exists x : core:string, y : core:string => core:string"
        ));
        assert_eq!(j["ctor"], "Sig");
        assert_eq!(j["args"][0], "x");
        assert_eq!(j["args"][2]["ctor"], "Sig");
        assert_eq!(j["args"][2]["args"][0], "y");
    }

    /// `eigentt:fst` / `eigentt:snd` are surface spellings of the projection NODES, not
    /// axioms — an axiom would be opaque and never reduce, so `fst(pair)` would not compute.
    #[test]
    fn eigentt_fst_and_snd_lower_to_projection_nodes() {
        for (name, ctor) in [("fst", "Fst"), ("snd", "Snd")] {
            let j = axiom_statement(&format!(
                "{NS} axiom p:t : eigentt:{name}(exists x : core:string => core:string)"
            ));
            assert_eq!(j["ctor"], ctor, "{name} -> {j}");
            assert_eq!(j["args"][0]["ctor"], "Sig");
        }
    }

    /// A one-argument call to anything else stays an ordinary application — the
    /// interception must not swallow user functions.
    #[test]
    fn only_the_eigentt_projections_are_intercepted() {
        let j = axiom_statement(&format!("{NS} axiom p:t : core:Asserts(core:string)"));
        assert_eq!(j["ctor"], "App", "got {j}");
    }
}

#[cfg(test)]
mod qualified_ctor_tests {
    //! **eigenius#24 / D79 P6.** Two inductives in one file declaring the same
    //! constructor short name used to be unresolvable: `resolve_ctor_iri` reported
    //! the ambiguity and told the author to *"rename one of the ctors as a
    //! workaround"*. `Type:ctor` names it directly.
    //!
    //! `(inductive, ctor name)` **is** a constructor's identity (D79 §2.2.1) —
    //! constructors have no IRI of their own — so the type is the only thing a
    //! constructor reference could be qualified by. #24's own rationale argued from
    //! the opposite premise ("each constructor has a canonical IRI … lookup is
    //! IRI-keyed"), which P4 removed; the feature survives the correction because it
    //! never depended on that premise, only on the pair.
    use super::*;

    fn compile(src: &str) -> Result<Vec<crate::ontology::resource::Resource>, Vec<EslError>> {
        crate::esl::compile(src)
    }

    fn errors(src: &str) -> String {
        match compile(src) {
            Ok(_) => String::new(),
            Err(es) => es
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        }
    }

    const TWO_INDUCTIVES: &str = r#"
namespace ex = "urn:eigenius:example";
data ex:Colour { red, mk }
data ex:Shape  { square, mk }
"#;

    /// The case the ambiguity error existed for.
    #[test]
    fn a_shared_ctor_short_name_is_ambiguous_unqualified() {
        let src = format!(
            "{TWO_INDUCTIVES}\naxiom ex:a : ex:Colour -> Prop\ndef ex:v : ex:Colour = mk;\n"
        );
        let err = errors(&src);
        assert!(
            err.contains("ambiguous"),
            "expected an ambiguity diagnostic, got {err:?}"
        );
    }

    /// And the same reference, qualified by its type, resolves.
    #[test]
    fn qualifying_by_the_inductive_disambiguates() {
        let src = format!("{TWO_INDUCTIVES}\ndef ex:v : ex:Colour = ex:Colour:mk;\n");
        assert_eq!(
            errors(&src),
            "",
            "ex:Colour:mk names exactly one constructor"
        );
    }

    /// The lexer change must not eat an annotation colon. `ex:Colour : Prop` is
    /// space-surrounded, so it stays a binder colon — tightness is the discriminator.
    #[test]
    fn a_space_surrounded_annotation_colon_is_untouched() {
        let src = format!("{TWO_INDUCTIVES}\naxiom ex:p : ex:Colour -> Prop\n");
        assert_eq!(
            errors(&src),
            "",
            "` : ` is an annotation colon, not a name separator"
        );
    }
}
