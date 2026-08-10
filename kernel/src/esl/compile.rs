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
use crate::nbe::term::{Exp, InductiveDecl, Patt};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::collections::BTreeMap;

/// Compile an ESL AST to Eigon-JSON resources.
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
/// `reasoning:JustifiedBy`'s ctors used in a sentence, or a
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
/// `eigentt:TypeExpr.App` and `reasoning:JustificationTerm.App`),
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
    /// Per-file constructor index. Two views over the same set of
    /// chain-resident + in-file ctors:
    ///
    /// - `ctors_by_iri`: the canonical "is this IRI a constructor?" set.
    ///   IRI is the stable identifier (gh #75 extended to the ESL
    ///   surface). Qualified references (`reasoning:App(...)`) resolve
    ///   the namespace prefix to an IRI and check membership here.
    /// - `ctors_by_short_name`: short name → list of qualifying ctor
    ///   IRIs, for bare-name lookup with ambiguity detection. Two
    ///   inductives that share a ctor short name (e.g.
    ///   `eigentt:TypeExpr.App` and `reasoning:JustificationTerm.App`)
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
/// Substitution does *not* descend into `TypeExpr` — parameter
/// references inside `type_expr(...)` bodies are not supported in
/// v1 because the TypeExpr AST has its own name-resolution scope
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

/// Expand all `TypeExpr::Alias` forms by substituting each binding's
/// value into the body at the names it introduces. The result is an
/// alias-free `TypeExpr` ready for the standard compile passes
/// (`lower_type_expr_to_exp` / `encode_type_expr_to_json` /
/// `compile_type_expr`).
///
/// Substitution rules:
///
/// - `Ref { namespace: None, name, args: [] }` → if `name` is bound
///   in `env`, replace with the bound `TypeExpr`. Otherwise leave
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
///   `Arrow`) recurse into their children unchanged.
fn expand_aliases(typ: &ast::TypeExpr, env: &BTreeMap<String, ast::TypeExpr>) -> ast::TypeExpr {
    match typ {
        ast::TypeExpr::Unit { .. } => typ.clone(),
        ast::TypeExpr::Ref { name, args, pos } => {
            if name.namespace.is_none() && args.is_empty() {
                if let Some(bound) = env.get(&name.name) {
                    return bound.clone();
                }
            }
            ast::TypeExpr::Ref {
                name: name.clone(),
                args: args.iter().map(|a| expand_aliases(a, env)).collect(),
                pos: pos.clone(),
            }
        }
        ast::TypeExpr::Arrow {
            domain,
            codomain,
            pos,
        } => ast::TypeExpr::Arrow {
            domain: Box::new(expand_aliases(domain, env)),
            codomain: Box::new(expand_aliases(codomain, env)),
            pos: pos.clone(),
        },
        ast::TypeExpr::Ann { expr, typ, pos } => ast::TypeExpr::Ann {
            expr: Box::new(expand_aliases(expr, env)),
            typ: Box::new(expand_aliases(typ, env)),
            pos: pos.clone(),
        },
        ast::TypeExpr::BinderArrow {
            name,
            kind,
            bound,
            body,
            pos,
        } => {
            let mut inner = env.clone();
            inner.remove(name);
            ast::TypeExpr::BinderArrow {
                name: name.clone(),
                kind: kind.clone(),
                bound: bound.clone(),
                body: Box::new(expand_aliases(body, &inner)),
                pos: pos.clone(),
            }
        }
        ast::TypeExpr::Pi {
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
            ast::TypeExpr::Pi {
                params: new_params,
                codomain: Box::new(expand_aliases(codomain, &inner)),
                pos: pos.clone(),
            }
        }
        ast::TypeExpr::Sigma { params, body, pos } => {
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
            ast::TypeExpr::Sigma {
                params: new_params,
                body: Box::new(expand_aliases(body, &inner)),
                pos: pos.clone(),
            }
        }
        ast::TypeExpr::Lambda { params, body, pos } => {
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
            ast::TypeExpr::Lambda {
                params: new_params,
                body: Box::new(expand_aliases(body, &inner)),
                pos: pos.clone(),
            }
        }
        ast::TypeExpr::Alias {
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
        ast::TypeExpr::Sort { .. }
        | ast::TypeExpr::LitString { .. }
        | ast::TypeExpr::LitInt { .. }
        | ast::TypeExpr::LitFloat { .. } => typ.clone(),
    }
}

impl Compiler {
    fn new() -> Self {
        Self {
            namespaces: BTreeMap::new(),
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
    ///   e.g. `reasoning:DeclaredEvidence`. This resolves to
    ///   `<ns_uri>:<CtorName>` via the standard namespace table.
    /// - Canonical chain IRI (what `ctors_by_iri` stores):
    ///   `<parent_inductive_iri>:<CtorName>`, e.g.
    ///   `urn:eigenius:reasoning:JustificationTerm:DeclaredEvidence`.
    ///
    /// The two never match by string equality, so the resolution
    /// strategy is short-name-based with namespace filtering:
    ///
    /// - **Qualified** `ns:Name` → look up `Name` in `ctors_by_short_name`,
    ///   filter the candidate ctor IRIs to those whose parent IRI
    ///   starts with `ns_uri:`. If exactly one match, use it. The
    ///   namespace prefix is what disambiguates between
    ///   `eigentt:App` (= `eigentt:TypeExpr:App`) and `reasoning:App`
    ///   (= `reasoning:JustificationTerm:App`).
    /// - **Bare** `Name` → look up the short name in
    ///   `ctors_by_short_name`. If exactly one ctor IRI matches, use
    ///   it. If two or more, error with an "ambiguous" message that
    ///   lists the candidate IRIs so the author can pick a qualifier.
    ///
    /// Returns `Ok(None)` when the name doesn't match any known ctor
    /// — caller falls through to its non-ctor paths (variable
    /// lookup, EigonClass, etc.).
    fn resolve_ctor_iri(&self, qn: &ast::QualifiedName) -> Result<Option<String>, EslError> {
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
        args: &[ast::TypeExpr],
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
        // Per gh #75 the stub's `name` is the diagnostic label; the
        // identity is the IRI. Pull the short name from the parent
        // IRI's local part so error messages read naturally.
        let parent_short_name = parent_iri.local_name().to_string();
        let stub = std::sync::Arc::new(InductiveDecl {
            iri: parent_iri,
            name: parent_short_name,
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let arg_exps: Result<Vec<Exp>, EslError> = args
            .iter()
            .map(|a| self.lower_type_expr_to_exp(a, scope))
            .collect();
        Ok(Exp::InductiveCtor(stub, ctor_name.to_string(), arg_exps?))
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
            ast::Declaration::Codata(c) => self.compile_codata(c),
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
        let class_value = Value::ResourceRef(target_class.clone());
        let option_arg = {
            let mut ar = Resource::new_embedded();
            set_is_a(&mut ar, wk::INDUCTIVE_ARG_TYPE);
            ar.set(iri(wk::TYPE_NAME), Value::String(wk::OPTION.to_string()));
            ar.set(iri(wk::TYPE_ARGS), Value::Array(vec![class_value.clone()]));
            Value::Embedded(Box::new(ar))
        };
        let param_types = [class_value.clone(), class_value.clone(), option_arg.clone()];

        // Build the Pi-term: `pi a : C, b : C, opt : Option<C> => C`.
        // Nested TypeBinderArrow resources, same shape `TypeExpr::Pi`
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

    // --- Codata ---

    fn compile_codata(&self, decl: &ast::CodataDecl) -> Result<Vec<Resource>, EslError> {
        use crate::ontology::well_known as wk;
        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:core:CodataType".to_string(),
            )]),
        );
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(decl.name.name.clone()),
        );

        // Type parameters (Phase 11b step 15h.3) — same shape as
        // `data`'s params so the decoder can reuse `decode_params`.
        let param_names: std::collections::HashSet<&str> =
            decl.params.iter().map(|p| p.name.as_str()).collect();
        let params: Result<Vec<Value>, EslError> = decl
            .params
            .iter()
            .map(|p| {
                let mut pr = Resource::new_embedded();
                set_is_a(&mut pr, wk::INDUCTIVE_PARAM);
                pr.set(iri(wk::PARAM_NAME), Value::String(p.name.clone()));
                // A parameter's kind is a qualified-name class (possibly an
                // earlier parameter in scope) or a sort literal — the latter
                // for Lean-style sort-parametrized inductives (`And (P : Prop,
                // Q : Prop)`). Same lowering as indices (see `decl.indices`).
                let kind = match &p.kind {
                    ast::IndexKind::Named(qn) => {
                        if qn.namespace.is_none() && param_names.contains(qn.name.as_str()) {
                            qn.name.clone()
                        } else {
                            self.resolve(qn)?
                        }
                    }
                    ast::IndexKind::Sort(sk) => match sk {
                        ast::SortKind::Prop => "Prop".to_string(),
                        ast::SortKind::Set => "Set".to_string(),
                        ast::SortKind::Type(n) => format!("Type:{n}"),
                    },
                };
                pr.set(iri(wk::PARAM_KIND), Value::String(kind));
                Ok(Value::Embedded(Box::new(pr)))
            })
            .collect();
        r.set(iri(wk::TYPE_PARAMS), Value::Array(params?));

        let mut observations = Vec::new();
        for obs in &decl.observations {
            let type_value = self.compile_type_expr(&obs.typ, &param_names)?;
            let mut obs_r = Resource::new_embedded();
            set_is_a(&mut obs_r, "urn:eigenius:core:Observation");
            obs_r.set(
                iri("urn:eigenius:core:observation_name"),
                Value::String(obs.name.clone()),
            );
            obs_r.set(iri("urn:eigenius:core:observation_type"), type_value);
            observations.push(Value::Embedded(Box::new(obs_r)));
        }
        r.set(
            iri("urn:eigenius:core:observations"),
            Value::Array(observations),
        );

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    /// Compile a type expression to a `Value` — either a plain string
    /// (for simple Ref types — preserves backward compat with the
    /// pre-15h.3 String IRI shape) or an embedded resource (for
    /// Arrow/BinderArrow/parameterised Ref).
    fn compile_type_expr(
        &self,
        typ: &ast::TypeExpr,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Value, EslError> {
        use crate::ontology::well_known as wk;
        // `alias` sugar — expand bindings into the body and recurse.
        // The expanded body is alias-free, so the recursion terminates.
        if let ast::TypeExpr::Alias { .. } = typ {
            let expanded = expand_aliases(typ, &BTreeMap::new());
            return self.compile_type_expr(&expanded, scope);
        }
        match typ {
            ast::TypeExpr::Unit { pos } => Err(EslError::compiler(
                Some(pos.clone()),
                "the unit value `()` is a TERM, not a type — it is only meaningful inside \
                 `type_expr(...)`"
                    .to_string(),
            )),
            ast::TypeExpr::Ref { name, args, .. } => {
                let resolved = if name.namespace.is_none() {
                    let n = name.name.as_str();
                    if scope.contains(n) || n == "Inf" || n == "Size" {
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
                    ar.set(iri(wk::TYPE_NAME), Value::String(resolved));
                    let arg_values: Result<Vec<Value>, EslError> = args
                        .iter()
                        .map(|a| self.compile_type_expr(a, scope))
                        .collect();
                    ar.set(iri(wk::TYPE_ARGS), Value::Array(arg_values?));
                    Ok(Value::Embedded(Box::new(ar)))
                }
            }
            ast::TypeExpr::Arrow {
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
            ast::TypeExpr::Ann { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "a type annotation `(e : T)` is not valid in a type-declaration \
                 position; it belongs in a term `type_expr(...)`"
                    .to_string(),
            )),
            ast::TypeExpr::BinderArrow {
                name,
                kind,
                bound,
                body,
                ..
            } => {
                let mut ar = Resource::new_embedded();
                set_is_a(&mut ar, wk::TYPE_BINDER_ARROW);
                ar.set(iri(wk::BINDER_NAME), Value::String(name.clone()));
                let kind_str = if kind.namespace.is_none() {
                    let n = kind.name.as_str();
                    if scope.contains(n) || n == "Inf" || n == "Size" {
                        n.to_string()
                    } else {
                        self.resolve(kind)?
                    }
                } else {
                    self.resolve(kind)?
                };
                ar.set(iri(wk::BINDER_KIND), Value::String(kind_str));
                if let Some(b) = bound {
                    let bound_str = if b.namespace.is_none() {
                        let n = b.name.as_str();
                        if scope.contains(n) || n == "Inf" || n == "Size" {
                            n.to_string()
                        } else {
                            self.resolve(b)?
                        }
                    } else {
                        self.resolve(b)?
                    };
                    ar.set(iri(wk::BINDER_BOUND), Value::String(bound_str));
                }
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
            // Parameter types can be arbitrary `TypeExpr`s (including
            // parametric types like `Option<A>` whose lowering
            // produces an embedded `InductiveArgType`). The kind
            // slot accepts both string and embedded forms — the
            // decoder dispatches on the value's shape.
            ast::TypeExpr::Sigma { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "`exists` (Sigma) is only available inside `type_expr(...)`, which lowers to the \
                 D47 ctor encoding; the resource-shaped type language has no binder for it"
                    .to_string(),
            )),
            ast::TypeExpr::Pi {
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
            ast::TypeExpr::Sort { kind, .. } => {
                let s = match kind {
                    ast::SortKind::Prop => "Prop".to_string(),
                    ast::SortKind::Set => "Set".to_string(),
                    ast::SortKind::Type(n) => format!("Type({n})"),
                };
                Ok(Value::String(s))
            }
            ast::TypeExpr::Lambda { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "`fun (…) => …` is only allowed inside `match … returning <motive>` \
                 motives, axiom statements, and other Exp-encoded contexts — not in \
                 the chain-value type-expression slots (codata observation types, \
                 lambda type slots, etc.). If you reached this from a `returning` \
                 clause, the motive is encoded via the D47 codec instead and this \
                 branch is not exercised."
                    .to_string(),
            )),
            ast::TypeExpr::LitString { pos, .. }
            | ast::TypeExpr::LitInt { pos, .. }
            | ast::TypeExpr::LitFloat { pos, .. } => Err(EslError::compiler(
                Some(pos.clone()),
                "literal values are not allowed in chain-value type-expression slots \
                 (codata observation types, etc.); they only appear in Exp-encoded \
                 contexts (axiom statements, `type_expr(...)` resource fields, \
                 indexed ctor return types)"
                    .to_string(),
            )),
            // Eliminated by the early-return at the top of this fn.
            ast::TypeExpr::Alias { .. } => unreachable!("alias expanded above"),
        }
    }

    // --- Axiom declarations (eigenius#72 Layer 1, D46 §10) ---

    /// Lower an `axiom Name : <type-expr>` declaration to a chain
    /// `core:Axiom` Resource whose `axiom_statement` is the encoded
    /// EigenTT type expression. Goes through the D47 codec
    /// (`encode_type`) after lowering the ESL TypeExpr to a kernel
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
        stamp_declared(&mut r);
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
        stamp_declared(&mut r);
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
        stamp_declared(&mut r);
        Ok(vec![r])
    }

    /// eigenius#72 — lower an ESL `TypeExpr` to a kernel `Exp`.
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
        typ: &ast::TypeExpr,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Exp, EslError> {
        // `alias` sugar — expand bindings into the body and recurse.
        if let ast::TypeExpr::Alias { .. } = typ {
            let expanded = expand_aliases(typ, &BTreeMap::new());
            return self.lower_type_expr_to_exp(&expanded, scope);
        }
        match typ {
            ast::TypeExpr::Unit { .. } => Ok(Exp::Unit),
            ast::TypeExpr::Sigma { params, body, .. } => {
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
            ast::TypeExpr::Sort { kind, .. } => Ok(Exp::Sort(match kind {
                ast::SortKind::Prop => 0,
                ast::SortKind::Set => 1,
                ast::SortKind::Type(n) => n + 1,
            })),
            // Sigma ELIMINATION — see the twin arm in `encode_type_expr_to_json`. Both paths
            // are live: `axiom X : T` lowers through here, `type_expr(...)` in a resource
            // property through the JSON encoder.
            ast::TypeExpr::Ref { name, args, .. }
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
            ast::TypeExpr::Ref { name, args, .. } => {
                let is_bound = name.namespace.is_none() && scope.contains(name.name.as_str());
                if is_bound {
                    // Bound variable: lowers to `Exp::Var`. If args are
                    // present, the user is writing a function-application
                    // shape like `P(x)` where `P : T -> Prop` is a
                    // forall-bound function. Curry into `Exp::App` chain
                    // so EigenTT's NbE can beta-reduce at use time —
                    // required by D39's `JustifiedBy.spec` constructor
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
                // D39 §5 `JustifiedBy.declared : ... ->
                // JustifiedBy(DeclaredEvidence iri) P` and any similar
                // shape where a ctor of one inductive appears in
                // another inductive's index/result-type position.
                //
                // `resolve_ctor_iri` walks `ctors_by_short_name` and
                // filters by namespace prefix, so `reasoning:App(...)`
                // unambiguously picks the `reasoning` namespace's
                // `App` ctor even when `eigentt:TypeExpr:App` shares
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
                    // Stub InductiveDecl for the App-curried encoding.
                    // The D47 codec produces `App(App(ConstRef(iri),
                    // a1), a2)…` and the decoder re-resolves the IRI
                    // against the chain at use time.
                    let short_name = iri_val.local_name().to_string();
                    let stub = std::sync::Arc::new(InductiveDecl {
                        iri: iri_val.clone(),
                        name: short_name,
                        params: Vec::new(),
                        indices: Vec::new(),
                        sort: Exp::Sort(1),
                        ctors: Vec::new(),
                    });
                    let arg_exps: Result<Vec<Exp>, EslError> = args
                        .iter()
                        .map(|a| self.lower_type_expr_to_exp(a, scope))
                        .collect();
                    Ok(Exp::InductiveType(stub, arg_exps?))
                }
            }
            ast::TypeExpr::Arrow {
                domain, codomain, ..
            } => {
                let dom = self.lower_type_expr_to_exp(domain, scope)?;
                let body = self.lower_type_expr_to_exp(codomain, scope)?;
                Ok(Exp::arrow(dom, body))
            }
            // `(e : T)` — bidirectional annotation → `Exp::Ann`.
            ast::TypeExpr::Ann { expr, typ, .. } => {
                let e = self.lower_type_expr_to_exp(expr, scope)?;
                let t = self.lower_type_expr_to_exp(typ, scope)?;
                Ok(Exp::Ann(Box::new(e), Box::new(t)))
            }
            ast::TypeExpr::Pi {
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
            ast::TypeExpr::BinderArrow {
                name,
                kind,
                bound: _,
                body,
                ..
            } => {
                // Size-binder arrows are typically used in sized
                // codata; axiom statements rarely involve sizes. v1
                // lowers as a plain Pi for non-size kinds and a
                // SizeSort-typed binder for size kinds. Bound
                // (upper bound for sized) is currently ignored — sized
                // axiom statements need a follow-on.
                let kind_str = self.resolve(kind)?;
                let dom = if kind_str.ends_with(":Size") || kind_str == "Size" {
                    Exp::SizeSort
                } else {
                    let iri_val = Iri::parse(&kind_str).map_err(|e| {
                        EslError::compiler(
                            Some(typ.pos().clone()),
                            format!("invalid kind IRI `{kind_str}`: {e}"),
                        )
                    })?;
                    Exp::EigonClass(iri_val)
                };
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
            ast::TypeExpr::Lambda { params, body, .. } => {
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
            ast::TypeExpr::LitString { value, .. } => Ok(Exp::LitString(value.clone())),
            ast::TypeExpr::LitInt { value, .. } => Ok(Exp::LitInt(*value)),
            ast::TypeExpr::LitFloat { value, .. } => Ok(Exp::LitFloat(*value)),
            // Eliminated by the early-return at the top of this fn.
            ast::TypeExpr::Alias { .. } => unreachable!("alias expanded above"),
        }
    }

    /// Encode an ESL `TypeExpr` directly to the D47 chain-JSON shape,
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
        typ: &ast::TypeExpr,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<serde_json::Value, EslError> {
        use crate::program::eigentt_type_mirror::encode_type;
        use serde_json::json;
        // `alias` sugar — expand bindings into the body and recurse.
        if let ast::TypeExpr::Alias { .. } = typ {
            let expanded = expand_aliases(typ, &BTreeMap::new());
            return self.encode_type_expr_to_json(&expanded, scope);
        }

        // Wrap a leaf TypeExpr: lower to Exp, encode via the D47
        // encoder, unwrap to raw JSON. Safe for any subtree whose
        // lowered Exp contains no `Lam`.
        let encode_leaf = |this: &Self, t: &ast::TypeExpr| -> Result<serde_json::Value, EslError> {
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
            ast::TypeExpr::Unit { .. } => Ok(serde_json::json!({"ctor": "UnitVal", "args": []})),
            ast::TypeExpr::Lambda { params, body, .. } => {
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
            ast::TypeExpr::Sigma { params, body, .. } => {
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
            ast::TypeExpr::Pi {
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
            ast::TypeExpr::Arrow {
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
            ast::TypeExpr::Ann { expr, typ, .. } => {
                let e_json = self.encode_type_expr_to_json(expr, scope)?;
                let t_json = self.encode_type_expr_to_json(typ, scope)?;
                Ok(json!({
                    "ctor": "Ann",
                    "args": [e_json, t_json],
                }))
            }
            ast::TypeExpr::BinderArrow {
                name,
                kind,
                bound: _,
                body,
                ..
            } => {
                // Size-binder arrows are rare in type_expr — defer to
                // the leaf path which handles SizeSort correctly.
                let kind_str = self.resolve(kind)?;
                if kind_str.ends_with(":Size") || kind_str == "Size" {
                    return encode_leaf(self, typ);
                }
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
            // `TypeExpr` has no postfix form at all; a `.1` / `.fst` postfix could be added
            // later and would desugar to these same nodes, leaving encoded terms identical.
            ast::TypeExpr::Ref { name, args, .. }
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
            ast::TypeExpr::Ref { name, args, .. } => {
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
            ast::TypeExpr::Sort { .. }
            | ast::TypeExpr::LitString { .. }
            | ast::TypeExpr::LitInt { .. }
            | ast::TypeExpr::LitFloat { .. } => encode_leaf(self, typ),
            // Eliminated by the early-return at the top of this fn.
            ast::TypeExpr::Alias { .. } => unreachable!("alias expanded above"),
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
        // (which would collide via `stamp_declared` + LayerBuilder
        // last-wins).
        let mut is_a_values: Vec<Value> = vec![Value::String(wk::INDUCTIVE_TYPE.to_string())];
        for extra in &decl.extra_classes {
            let extra_iri = self.resolve(extra)?;
            is_a_values.push(Value::String(extra_iri));
        }
        r.set(iri(wk::IS_A), Value::Array(is_a_values));
        r.set(iri(wk::SHORT_NAME), Value::String(decl.name.name.clone()));

        let param_names: std::collections::HashSet<&str> =
            decl.params.iter().map(|p| p.name.as_str()).collect();

        let params: Result<Vec<Value>, EslError> = decl
            .params
            .iter()
            .map(|p| {
                let mut pr = Resource::new_embedded();
                set_is_a(&mut pr, wk::INDUCTIVE_PARAM);
                pr.set(iri(wk::PARAM_NAME), Value::String(p.name.clone()));
                // A parameter's kind is a qualified-name class (possibly an
                // earlier parameter in scope) or a sort literal — the latter
                // for Lean-style sort-parametrized inductives (`And (P : Prop,
                // Q : Prop)`). Same lowering as indices (see `decl.indices`).
                let kind = match &p.kind {
                    ast::IndexKind::Named(qn) => {
                        if qn.namespace.is_none() && param_names.contains(qn.name.as_str()) {
                            qn.name.clone()
                        } else {
                            self.resolve(qn)?
                        }
                    }
                    ast::IndexKind::Sort(sk) => match sk {
                        ast::SortKind::Prop => "Prop".to_string(),
                        ast::SortKind::Set => "Set".to_string(),
                        ast::SortKind::Type(n) => format!("Type:{n}"),
                    },
                };
                pr.set(iri(wk::PARAM_KIND), Value::String(kind));
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
                    let kind = match &p.kind {
                        ast::IndexKind::Named(qn) => {
                            if qn.namespace.is_none() && param_names.contains(qn.name.as_str()) {
                                qn.name.clone()
                            } else {
                                self.resolve(qn)?
                            }
                        }
                        // Sort literals encode as canonical strings the
                        // kernel's `decode_param_kind_str` recognises:
                        // "Prop" → Sort(0), "Set" → Sort(1), "Type:N"
                        // → Sort(N+1). Needed for D39 §5's JustifiedBy
                        // and ChainWitness predicates whose intermediate
                        // index kinds are themselves sorts.
                        ast::IndexKind::Sort(sk) => match sk {
                            ast::SortKind::Prop => "Prop".to_string(),
                            ast::SortKind::Set => "Set".to_string(),
                            ast::SortKind::Type(n) => format!("Type:{n}"),
                        },
                    };
                    pr.set(iri(wk::PARAM_KIND), Value::String(kind));
                    Ok(Value::Embedded(Box::new(pr)))
                })
                .collect();
            r.set(iri(wk::INDICES), Value::Array(indices?));
        }

        // eigenius#72 Layer 2 — explicit result sort. Encoded as a
        // string; the decoder parses it back into `Exp::Sort(n)`.
        if let Some(sort) = decl.result_sort {
            let sort_str = match sort {
                ast::SortKind::Prop => "Prop".to_string(),
                ast::SortKind::Set => "Set".to_string(),
                ast::SortKind::Type(n) => format!("Type:{n}"),
            };
            r.set(iri(wk::RESULT_SORT), Value::String(sort_str));
        }

        let parent_iri_str = self.resolve(&decl.name)?;
        let ctors: Result<Vec<Value>, EslError> = decl
            .ctors
            .iter()
            .map(|c| {
                let ctor_iri_str = format!("{parent_iri_str}:{}", c.name());
                let ctor_iri = Iri::parse(&ctor_iri_str).map_err(|e| {
                    EslError::compiler(
                        Some(c.pos().clone()),
                        format!("invalid ctor IRI `{ctor_iri_str}`: {e}"),
                    )
                })?;
                let mut cr = Resource::new(ctor_iri);
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
                                ast::CtorArg::Named {
                                    name, kind, bound, ..
                                } => {
                                    arg_values
                                        .push(self.compile_ctor_binder(name, kind, bound, &scope)?);
                                    local_binders.push(name.clone());
                                }
                            }
                        }
                        cr.set(iri(wk::ARG_TYPES), Value::Array(arg_values));
                    }
                    ast::CtorDecl::Typed { typ, pos, .. } => {
                        // eigenius#72 Layer 2 — the typed form supplies
                        // the full Π-telescope (including conclusion
                        // indices) as a single TypeExpr. Lower it to
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

        stamp_declared(&mut r);
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
        // 2. Built-in size literal (`Inf`) / sort (`Size`) → bare name
        //    (decoder emits `SizeInf` / `SizeSort` respectively)
        // 3. Otherwise resolve through the namespace registry
        let type_name = if arg.name.namespace.is_none() {
            let n = arg.name.name.as_str();
            if params.contains(n) || n == "Inf" || n == "Size" {
                arg.name.name.clone()
            } else {
                self.resolve(&arg.name)?
            }
        } else {
            self.resolve(&arg.name)?
        };
        ar.set(iri(wk::TYPE_NAME), Value::String(type_name));

        let type_args: Result<Vec<Value>, EslError> = arg
            .params
            .iter()
            .map(|p| self.compile_ctor_arg_type(p, params))
            .collect();
        ar.set(iri(wk::TYPE_ARGS), Value::Array(type_args?));

        Ok(Value::Embedded(Box::new(ar)))
    }

    /// Compile a named constructor-argument binder
    /// (`ident : Kind [< Bound]`, Phase 11b step 15h).
    ///
    /// Encoded as an `InductiveArgType` resource carrying a
    /// `binder_name` key — its presence distinguishes binders from
    /// positional args at decode time. `binder_bound` holds the
    /// optional upper bound (resolved identically to type names:
    /// declared params / built-ins bare, everything else via
    /// namespace resolution).
    fn compile_ctor_binder(
        &self,
        name: &str,
        kind: &ast::QualifiedName,
        bound: &Option<ast::QualifiedName>,
        scope: &std::collections::HashSet<&str>,
    ) -> Result<Value, EslError> {
        use crate::ontology::well_known as wk;
        let mut ar = Resource::new_embedded();
        set_is_a(&mut ar, wk::INDUCTIVE_ARG_TYPE);

        // The "type" part of the binder (kind). Resolution rules
        // mirror `compile_ctor_arg_type` — declared params and
        // `Inf`/`Size` built-ins stay bare; other names resolve
        // through the namespace registry.
        let kind_str = if kind.namespace.is_none() {
            let n = kind.name.as_str();
            if scope.contains(n) || n == "Inf" || n == "Size" {
                kind.name.clone()
            } else {
                self.resolve(kind)?
            }
        } else {
            self.resolve(kind)?
        };
        ar.set(iri(wk::TYPE_NAME), Value::String(kind_str));
        ar.set(iri(wk::TYPE_ARGS), Value::Array(Vec::new()));
        ar.set(iri(wk::BINDER_NAME), Value::String(name.to_string()));

        if let Some(b) = bound {
            let bound_str = if b.namespace.is_none() {
                let n = b.name.as_str();
                if scope.contains(n) || n == "Inf" || n == "Size" {
                    b.name.clone()
                } else {
                    self.resolve(b)?
                }
            } else {
                self.resolve(b)?
            };
            ar.set(iri(wk::BINDER_BOUND), Value::String(bound_str));
        }

        Ok(Value::Embedded(Box::new(ar)))
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

        // subclass_of — accumulate from BOTH the header form
        // (`class X : A, B { … }`) AND any in-body `subclass_of A, B;`
        // items. Both authoring styles compose into one array
        // (eigenius#29).
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

        stamp_declared(&mut r);
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
            }
        }

        stamp_declared(&mut r);
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

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    fn compile_value(&self, value: &ast::Value) -> Result<Value, EslError> {
        match value {
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
            // `type_expr(<TypeExpr>)` — inline D47-encoded EigenTT
            // type expression. Lowers via the same path as `axiom`
            // and `data` ctor types: ESL TypeExpr →
            // `lower_type_expr_to_exp` → `encode_type` → chain JSON.
            // Used by D39 ReasoningSentence authors so propositions
            // and certificates can be written in EigenTT surface
            // rather than the hand-built D47 tagged-dict tree.
            ast::Value::TypeExpr { typ, pos: _ } => {
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
            // `reasoning:App(...)` resolve to the
            // `reasoning:JustificationTerm.App` ctor inside a value
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
            ast::Value::TypeExpr { .. } => Err(EslError::compiler(
                None,
                "`type_expr(...)` cannot appear as an argument inside a chain inductive ctor — \
                 D32 §3.7 ctor args are flat values or nested ctor applications, not D47-encoded \
                 type expressions. Lift the type_expr to the property value directly.",
            )),
            // Same disambiguation as `compile_value`: try ctor
            // resolution first (qualified ctor refs reach this site
            // when an outer ctor's arg is `reasoning:App(...)`),
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

        stamp_declared(&mut r);
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

            ast::Expr::CoRecord { fields, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:CoRecord");
                let mut cofields = Vec::new();
                for f in fields {
                    let body_r = self.compile_expr(&f.body)?;
                    let mut cf = Resource::new_embedded();
                    set_is_a(&mut cf, "urn:eigenius:program:CoField");
                    cf.set(
                        iri("urn:eigenius:program:observation_name"),
                        Value::String(f.name.clone()),
                    );
                    cf.set(
                        iri("urn:eigenius:program:body"),
                        Value::Embedded(Box::new(body_r)),
                    );
                    cofields.push(Value::Embedded(Box::new(cf)));
                }
                r.set(iri("urn:eigenius:program:cofields"), Value::Array(cofields));
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
                // - A bare `TypeExpr::Ref` (qualified name, no args) is
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
                        ast::TypeExpr::Ref { name, args, .. } if args.is_empty() => {
                            let result_iri = self.resolve(name)?;
                            r.set(
                                iri("urn:eigenius:program:result_type"),
                                Value::String(result_iri),
                            );
                        }
                        ast::TypeExpr::Lambda { params, body, pos } => {
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
    r.set(
        iri(wk::MERGE_TARGET_CLASS),
        Value::ResourceRef(target_class),
    );
    r.set(
        iri(wk::MERGE_TRANSFORMATION),
        Value::ResourceRef(transformation),
    );
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

/// Append `DeclaredResource` to `is_a` and set `declared_by` on a
/// compiled resource (D6b epistemic stamping, Phase 10b Step 3).
fn stamp_declared(resource: &mut Resource) {
    let is_a_iri = iri("urn:eigenius:core:is_a");
    let mut types = match resource.get(&is_a_iri) {
        Some(Value::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    types.push(Value::String(
        crate::ontology::well_known::DECLARED_RESOURCE.to_string(),
    ));
    resource.set(is_a_iri, Value::Array(types));
    resource.set(
        iri(crate::ontology::well_known::DECLARED_BY),
        Value::String("esl-compiler".to_string()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esl;
    use crate::ontology::eigon_json;

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
    fn compile_codata_declaration() {
        // A codata type with two observations, one referencing itself.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:IntStream {
                head : core:integer;
                tail : ex:IntStream;
            }
        "#,
        );
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:IntStream");
        let is_a = r.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:core:CodataType");
        assert_eq!(
            r.get(&iri("urn:eigenius:core:short_name"))
                .unwrap()
                .as_str(),
            Some("IntStream")
        );

        // Observations array
        let observations = r
            .get(&iri("urn:eigenius:core:observations"))
            .expect("observations property");
        let arr = match observations {
            Value::Array(a) => a,
            _ => panic!("observations must be an array"),
        };
        assert_eq!(arr.len(), 2);

        // First observation: head -> core:integer
        let head = match &arr[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("observation must be embedded"),
        };
        assert_eq!(
            head.get(&iri("urn:eigenius:core:observation_name"))
                .unwrap()
                .as_str(),
            Some("head")
        );
        assert_eq!(
            head.get(&iri("urn:eigenius:core:observation_type"))
                .unwrap()
                .as_str(),
            Some("urn:eigenius:core:integer")
        );

        // Second observation: tail -> ex:IntStream (self-reference)
        let tail = match &arr[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("observation must be embedded"),
        };
        assert_eq!(
            tail.get(&iri("urn:eigenius:core:observation_name"))
                .unwrap()
                .as_str(),
            Some("tail")
        );
        assert_eq!(
            tail.get(&iri("urn:eigenius:core:observation_type"))
                .unwrap()
                .as_str(),
            Some("urn:eigenius:example:IntStream")
        );
    }

    #[test]
    fn compile_corecord_expression() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:mk_pair : ex:Unit -> ex:Pair {
                corecord {
                    fst = 1;
                    snd = 2;
                }
            }
        "#,
        );
        let r = &resources[0];
        let body = r
            .get(&iri("urn:eigenius:program:body"))
            .unwrap()
            .as_embedded()
            .unwrap();
        // Body should be a CoRecord
        assert_eq!(body.is_a()[0].as_str(), "urn:eigenius:program:CoRecord");

        let cofields = body
            .get(&iri("urn:eigenius:program:cofields"))
            .expect("cofields");
        let arr = match cofields {
            Value::Array(a) => a,
            _ => panic!("cofields must be array"),
        };
        assert_eq!(arr.len(), 2);

        let fst = match &arr[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("cofield must be embedded"),
        };
        assert_eq!(fst.is_a()[0].as_str(), "urn:eigenius:program:CoField");
        assert_eq!(
            fst.get(&iri("urn:eigenius:program:observation_name"))
                .unwrap()
                .as_str(),
            Some("fst")
        );
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

    // --- DeclaredResource stamping tests (Phase 10b) ---

    fn has_declared_resource(r: &Resource) -> bool {
        r.is_a()
            .iter()
            .any(|i| i.as_str() == crate::ontology::well_known::DECLARED_RESOURCE)
    }

    fn declared_by(r: &Resource) -> Option<String> {
        r.get(&iri(crate::ontology::well_known::DECLARED_BY))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    #[test]
    fn esl_class_stamped_declared_resource() {
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
            has_declared_resource(r),
            "ESL class should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_property_stamped_declared_resource() {
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
            has_declared_resource(r),
            "ESL property should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_resource_stamped_declared_resource() {
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
            has_declared_resource(r),
            "ESL resource should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_program_stamped_declared_resource() {
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
            has_declared_resource(r),
            "ESL program should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_codata_stamped_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:Stream {
                head : core:integer;
                tail : ex:Stream;
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL codata should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    // --- `data` declaration compilation (Phase 11b step 8) ---

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
        // Each ctor carries an IRI derived from parent + local name
        // (Phase 11b step 9 — IRI as canonical identity).
        assert_eq!(
            zero.id().map(|i| i.as_str()),
            Some("urn:eigenius:example:Nat:zero")
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
        assert_eq!(
            succ.id().map(|i| i.as_str()),
            Some("urn:eigenius:example:Nat:succ")
        );
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
            succ_arg
                .get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("urn:eigenius:example:Nat")
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

            data ex:List(A : core:Set) {
                nil,
                cons(A, ex:List(A)),
            }
            "#,
        );
        let r = &resources[0];

        // One param, name=A, kind=core:Set.
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
            p.get(&iri("urn:eigenius:core:param_kind"))
                .and_then(|v| v.as_str()),
            Some("urn:eigenius:core:Set")
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
            arg0.get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("A")
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
            arg1.get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("urn:eigenius:example:List")
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
            arg1_a
                .get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("A")
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

            data ex:Vec(A : core:Set) : core:Nat -> Set {
                nil  : ex:Vec(A, ex:zero),
                cons : forall (n : core:Nat) => A -> ex:Vec(A, n) -> ex:Vec(A, ex:succ(n)),
            }

            axiom ex:vec_inhabits_nat_length :
                forall (A : core:Set, n : core:Nat) => ex:Vec(A, n) -> ex:Nat
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
        assert_eq!(
            vec.get(&Iri::parse(crate::ontology::well_known::RESULT_SORT).unwrap())
                .and_then(|v| if let Value::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }),
            Some("Set")
        );

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

            data ex:Vec(A : core:Set) : core:Nat -> Set {
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
                    entry
                        .get(&Iri::parse(wk_local::PARAM_KIND).unwrap())
                        .and_then(|v| if let Value::String(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }),
                    Some("urn:eigenius:core:Nat")
                );
            }
            other => panic!("expected `core:indices` array, got {other:?}"),
        }

        // Result sort — explicitly `Set`.
        let sort_iri = Iri::parse(wk_local::RESULT_SORT).unwrap();
        assert_eq!(
            r.get(&sort_iri).and_then(|v| if let Value::String(s) = v {
                Some(s.as_str())
            } else {
                None
            }),
            Some("Set")
        );

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

            data ex:Eq(A : core:Set) : A -> A -> Prop {
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
                pr.get(&Iri::parse(wk_local::PARAM_KIND).unwrap())
                    .and_then(|v| if let Value::String(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }),
                Some("A"),
                "param-typed index should keep bare param name as kind"
            );
        }
        // Result sort should be `Prop`.
        assert_eq!(
            r.get(&Iri::parse(wk_local::RESULT_SORT).unwrap())
                .and_then(|v| if let Value::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }),
            Some("Prop")
        );
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
        // D39 §5 / D49 ChainWitness path: when an intermediate index is
        // a Sort literal (Prop / Set / Type N), the compiler must emit
        // the kind string the kernel's `decode_param_kind_str` recognises
        // ("Prop" → Sort(0), "Set" → Sort(1), "Type:N" → Sort(N+1)).
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

        let kind_strings: Vec<String> = arr
            .iter()
            .map(|v| match v {
                Value::Embedded(e) => match e.get(&param_kind_iri) {
                    Some(Value::String(s)) => s.clone(),
                    other => panic!("expected string kind, got {other:?}"),
                },
                other => panic!("expected embedded index, got {other:?}"),
            })
            .collect();
        assert_eq!(kind_strings, vec!["Prop", "Set", "Type:2"]);

        let sort_iri = Iri::parse(wk_local::RESULT_SORT).unwrap();
        assert_eq!(
            r.get(&sort_iri).and_then(|v| if let Value::String(s) = v {
                Some(s.as_str())
            } else {
                None
            }),
            Some("Type:3")
        );
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
        let sort_iri = Iri::parse(wk_local::RESULT_SORT).unwrap();
        assert!(
            r.get(&sort_iri).is_none(),
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
            has_declared_resource(r),
            "ESL data should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
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
        // macro expansion. Without that order, `reasoning:App(...)`
        // in a `reasoning:justification = ...` slot errors with
        // "macro not declared" instead of resolving to the
        // `reasoning:JustificationTerm.App` ctor.
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

            data ex:HasLowIC50 : core:string -> Prop {
            }

            resource ex:with_alias : ref:DeclaredResource {
                ref:declared_by = "test:alias";
                ref:canonical_proposition = type_expr(
                    alias EIG = "urn:ex:EIG_0291"
                    in
                    ex:HasLowIC50(EIG)
                );
            }

            resource ex:without_alias : ref:DeclaredResource {
                ref:declared_by = "test:alias";
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

            data ex:HasLowIC50 : core:string -> Prop {
            }

            resource ex:scope_test : ref:DeclaredResource {
                ref:declared_by = "test:scope";
                ref:canonical_proposition = type_expr(
                    alias x = "urn:ex:SHOULD_NOT_LEAK"
                    in
                    forall (x : core:string) => ex:HasLowIC50(x)
                );
            }

            resource ex:scope_expected : ref:DeclaredResource {
                ref:declared_by = "test:scope";
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

            data ex:Option(A : core:Set) {
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
        // The axiom_statement value is the encoded TypeExpr.
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
                assert_eq!(args[1]["args"][0], 0);
                assert_eq!(args[2]["ctor"], "Sort");
                assert_eq!(args[2]["args"][0], 0);
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
                assert_eq!(j["args"][1]["args"][0], 0);
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
        // D39 Phase 3 — the authored reasoning.esl source must compile
        // cleanly. Locks the structural contract: namespace declarations,
        // four `ChainWitness.Is*As` zero-ctor predicates, the
        // `JustificationTerm` six-ctor inductive, and the `JustifiedBy`
        // seven-ctor indexed inductive predicate. Any future edit to the
        // file or to the ESL surface that breaks this round-trip needs
        // to be deliberate.
        let source = include_str!("../../../ontologies/reasoning/reasoning.esl");
        let resources = esl::compile(source).expect("reasoning.esl must compile");

        // Expect: 4 ChainWitness predicates + 1 JustificationTerm
        //         + 1 JustifiedBy = 6 inductive-type Resources.
        let inductive_iri = iri(crate::ontology::well_known::INDUCTIVE_TYPE);
        let ind_count = resources
            .iter()
            .filter(|r| r.is_a().iter().any(|c| c == &inductive_iri))
            .count();
        assert!(
            ind_count >= 6,
            "expected at least 6 inductive Resources in reasoning.esl, found {ind_count}"
        );

        // Phase 4 added two resource classes (ReasoningSentence +
        // VerifiedPropositionView) + their property declarations.
        // Phase 7 added the two query-request classes
        // (EntailmentRequest + ConsistencyRequest). TaskOutput is
        // intentionally not here — D39 §4.4 justifies it entirely by
        // the discipline-thesis benchmark work (D50/D51), so it lives
        // with the benchmark harness, not in the foundational
        // Reasoning institution ontology.
        let class_iri = iri(crate::ontology::well_known::CLASS);
        for expected in &[
            "urn:eigenius:reasoning:ReasoningSentence",
            "urn:eigenius:reasoning:VerifiedPropositionView",
            "urn:eigenius:reasoning:EntailmentRequest",
            "urn:eigenius:reasoning:ConsistencyRequest",
        ] {
            assert!(
                resources
                    .iter()
                    .any(|r| r.id().map(|i| i.as_str() == *expected).unwrap_or(false)
                        && r.is_a().iter().any(|c| c == &class_iri)),
                "reasoning.esl missing class declaration for {expected}"
            );
        }

        // Phase 5a — the Reasoning institution resource + three
        // QueryClass resources. Each carries its declared `is_a`
        // pointing at the institution-ontology base class.
        let institution_class = Iri::parse("urn:eigenius:institution:Institution").unwrap();
        let qc_class = Iri::parse("urn:eigenius:institution:QueryClass").unwrap();
        assert!(
            resources.iter().any(|r| r
                .id()
                .map(|i| i.as_str() == "urn:eigenius:reasoning:reasoning_institution")
                .unwrap_or(false)
                && r.is_a().iter().any(|c| c == &institution_class)),
            "reasoning.esl missing the reasoning_institution resource"
        );
        for expected in &[
            "urn:eigenius:reasoning:qc_validate_justification",
            "urn:eigenius:reasoning:qc_entailment_query",
            "urn:eigenius:reasoning:qc_consistency_check",
        ] {
            assert!(
                resources
                    .iter()
                    .any(|r| r.id().map(|i| i.as_str() == *expected).unwrap_or(false)
                        && r.is_a().iter().any(|c| c == &qc_class)),
                "reasoning.esl missing QueryClass resource for {expected}"
            );
        }

        // Phase 6 refactor — the ef_justification ExportFormat the
        // validate handler dispatches through.
        let ef_class = Iri::parse("urn:eigenius:institution:ExportFormat").unwrap();
        assert!(
            resources.iter().any(|r| r
                .id()
                .map(|i| i.as_str() == "urn:eigenius:reasoning:ef_justification")
                .unwrap_or(false)
                && r.is_a().iter().any(|c| c == &ef_class)),
            "reasoning.esl missing the ef_justification ExportFormat resource"
        );

        // Spot-check: the four witness IRIs are present.
        use crate::ontology::well_known as wk_local;
        for expected in &[
            wk_local::CHAIN_WITNESS_IS_DECLARED_AS,
            wk_local::CHAIN_WITNESS_IS_OBSERVED_AS,
            wk_local::CHAIN_WITNESS_IS_DERIVED_AS,
            wk_local::CHAIN_WITNESS_IS_VERIFIED_AS,
        ] {
            assert!(
                resources
                    .iter()
                    .any(|r| r.id().map(|i| i.as_str() == *expected).unwrap_or(false)),
                "reasoning.esl missing witness IRI {expected}"
            );
        }
    }

    #[test]
    fn reasoning_ontology_resolves_through_codec() {
        // End-to-end sanity check: reasoning.esl compiled on top of the
        // core ontology resolves cleanly through `resolve_class_type`.
        // Exercises (a) the new Sort-typed-index path (JustifiedBy's
        // `Prop` index), (b) the codec self-reference short-circuit
        // (JustifiedBy's ctors reference JustifiedBy itself), and
        // (c) cross-inductive references (JustifiedBy → ChainWitness +
        // JustificationTerm). If any of these regress, the full Phase 6
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

        // Phase 4 — the resource classes (ReasoningSentence, TaskOutput,
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
        // eigentt:TypeExpr is referenced from reasoning:proposition /
        // reasoning:certificate via class_types; load the fragment too.
        let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
        let eigentt_resources = eigon_json::parse_document(eigentt_json).unwrap();
        for r in eigentt_resources {
            reflection_builder.add_resource(r).unwrap();
        }
        let reflection =
            Arc::new(reflection_builder.build(crate::layer::LayerStorage::in_memory()));

        let source = include_str!("../../../ontologies/reasoning/reasoning.esl");
        let user_resources = esl::compile(source).expect("reasoning.esl must compile");
        let mut user_builder = LayerBuilder::new("reasoning", Some(reflection));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(user_builder.build(crate::layer::LayerStorage::in_memory()));

        // The six inductive types — Phase 3.
        for iri_str in &[
            "urn:eigenius:reasoning:ChainWitness:IsDeclaredAs",
            "urn:eigenius:reasoning:ChainWitness:IsObservedAs",
            "urn:eigenius:reasoning:ChainWitness:IsDerivedAs",
            "urn:eigenius:reasoning:ChainWitness:IsVerifiedAs",
            "urn:eigenius:reasoning:JustificationTerm",
            "urn:eigenius:reasoning:JustifiedBy",
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
            "urn:eigenius:reasoning:ReasoningSentence",
            "urn:eigenius:reasoning:VerifiedPropositionView",
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
                class_types eigentt:TypeExpr;
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
                assert_eq!(j["args"][1]["args"][0], 1);
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
            assert_eq!(j["args"][1]["args"][0], 1);
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

            data eg:Pair(A : core:Set, B : core:Set) {
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
