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

//! Printing D47-encoded terms back as ESL source — the inverse of [`super::compile`].
//!
//! # Why the input is JSON, not `Exp`
//!
//! [`crate::program::eigentt_type_mirror::decode_type`] needs a `Layer` to classify a `ConstRef`
//! as a class, an axiom, or an inductive decl. Requiring a resumed 7.6M-resource chain to read a
//! term out of a file would make `eigenius decompile some.json` useless. The D47 JSON *is* the
//! serialized term, and printing it needs no chain at all.
//!
//! # Fail closed
//!
//! Every ctor the printer emits must REPARSE to the same term. A ctor with no ESL surface is a
//! hard [`PrintError`], never a guess or a comment — a decompiler that silently drops a subterm
//! produces source that compiles to something *other* than what was on the chain, which is worse
//! than no output. [`round_trip`](super::print::tests) is the gate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::Value;

/// A term the printer cannot express in ESL, with the path to the offending node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintError {
    pub message: String,
    /// Structural path from the term root, e.g. `.App[1].Sig[2]`.
    pub path: String,
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at {})", self.message, self.path)
    }
}

impl std::error::Error for PrintError {}

/// Precedence of a printed form. A child printed in a context of lower precedence is wrapped.
///
/// ESL applications are *call syntax* (`f(a, b)`), not juxtaposition, so an application is
/// self-delimiting and binds as tightly as an atom. Only `->` and the binders need brackets.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prec {
    /// `f(a)`, `x`, `ns:C`, `"lit"`, `()`, `(e : T)`
    Atom = 0,
    /// `A -> B`, right-associative
    Arrow = 1,
    /// `forall (…) => B`, `exists x : T => B`, `fun (…) => B`
    Binder = 2,
}

/// Namespace aliases accumulated while printing, emitted as the source preamble.
///
/// ESL has no way to write a bare absolute IRI in reference position, so every `ConstRef` the
/// printer emits requires a `namespace` declaration. Collecting them during the walk (rather than
/// pre-scanning) means the preamble contains exactly the aliases the body uses.
#[derive(Default)]
pub struct Namespaces {
    /// prefix (IRI up to the last `:`) → alias
    by_prefix: BTreeMap<String, String>,
    taken: BTreeSet<String>,
}

impl Namespaces {
    pub fn new() -> Self {
        Self::default()
    }

    /// Split an IRI into `(alias, local)`, minting an alias for the prefix on first sight.
    fn split(&mut self, iri: &str) -> Result<(String, String), String> {
        let (prefix, local) = iri.rsplit_once(':').ok_or_else(|| {
            format!("IRI `{iri}` has no `:` — cannot split into namespace + name")
        })?;
        if !is_ident(local) {
            return Err(format!(
                "IRI `{iri}` has local name `{local}`, which is not a legal ESL identifier"
            ));
        }
        if let Some(a) = self.by_prefix.get(prefix) {
            return Ok((a.clone(), local.to_string()));
        }
        // The last prefix segment is the natural alias (`urn:eigenius:umlscui` → `umlscui`).
        // Sanitised, since IRI segments admit characters identifiers do not (`onco-typed`).
        let base: String = prefix
            .rsplit(':')
            .next()
            .unwrap_or("ns")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let base = if base.chars().next().is_some_and(|c| c.is_ascii_digit()) || base.is_empty() {
            format!("ns_{base}")
        } else {
            base
        };
        let mut alias = base.clone();
        let mut n = 2;
        while self.taken.contains(&alias) {
            alias = format!("{base}{n}");
            n += 1;
        }
        self.taken.insert(alias.clone());
        self.by_prefix.insert(prefix.to_string(), alias.clone());
        Ok((alias, local.to_string()))
    }

    /// `namespace a = "prefix";` lines, in alias order.
    pub fn preamble(&self) -> String {
        let mut by_alias: Vec<_> = self.by_prefix.iter().map(|(p, a)| (a, p)).collect();
        by_alias.sort();
        let mut out = String::new();
        for (alias, prefix) in by_alias {
            let _ = writeln!(out, "namespace {alias} = \"{prefix}\";");
        }
        out
    }
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// How a term is laid out. Both layouts compile to the same term; only whitespace differs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Layout {
    /// One line, however long. What a machine consumer wants.
    #[default]
    Flat,
    /// Break applications and binder bodies across lines once they exceed [`WIDTH`], indenting
    /// each level. A parsed sentence's proposition is a deeply nested application spine; on one
    /// line it is a 900-character string in which the argument structure is invisible.
    Pretty,
}

/// Column past which [`Layout::Pretty`] breaks a composite form.
pub const WIDTH: usize = 96;

/// Indent added per nesting level in [`Layout::Pretty`].
const STEP: usize = 4;

/// Print a D47-encoded term as an ESL type-expression, on one line.
///
/// Aliases for every IRI mentioned are added to `ns`; the caller emits [`Namespaces::preamble`].
pub fn print_type_expr(term: &Value, ns: &mut Namespaces) -> Result<String, PrintError> {
    print_type_expr_with(term, ns, Layout::Flat, 0)
}

/// Print a D47-encoded term, laid out per `layout` and starting at column `indent`.
///
/// `indent` is the column the term's first character lands on, so continuation lines can be
/// aligned under it. It affects only where breaks fall — never which term is printed.
pub fn print_type_expr_with(
    term: &Value,
    ns: &mut Namespaces,
    layout: Layout,
    indent: usize,
) -> Result<String, PrintError> {
    let mut p = Printer {
        ns,
        scope: Vec::new(),
        reserved: BTreeSet::new(),
        layout,
    };
    p.reserve_names(term);
    p.go(term, Prec::Binder, ".", indent)
}

struct Printer<'a> {
    ns: &'a mut Namespaces,
    /// Binder renamings in scope, innermost last. Shadowing is handled by reverse lookup.
    scope: Vec<(String, String)>,
    /// Every name occurring anywhere in the term — a renamed binder must avoid all of them, or
    /// it would capture a free variable that happens to carry the name we picked.
    reserved: BTreeSet<String>,
    layout: Layout,
}

impl Printer<'_> {
    fn reserve_names(&mut self, v: &Value) {
        match v {
            Value::Object(o) => {
                if let (Some(Value::String(c)), Some(Value::Array(a))) =
                    (o.get("ctor"), o.get("args"))
                {
                    if c == "Var" || c == "Pi" || c == "Sig" || c == "Lam" {
                        if let Some(Value::String(n)) = a.first() {
                            self.reserved.insert(n.clone());
                        }
                    }
                }
                for x in o.values() {
                    self.reserve_names(x);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| self.reserve_names(x)),
            _ => {}
        }
    }

    fn err(&self, msg: impl Into<String>, path: &str) -> PrintError {
        PrintError {
            message: msg.into(),
            path: path.to_string(),
        }
    }

    /// The name to print for a bound occurrence — the rename if the binder was renamed.
    fn lookup(&self, name: &str) -> String {
        self.scope
            .iter()
            .rev()
            .find(|(orig, _)| orig == name)
            .map(|(_, new)| new.clone())
            .unwrap_or_else(|| name.to_string())
    }

    /// Bind `name`, renaming it if it is not a legal ESL identifier.
    ///
    /// The DCG emits gensyms like `G#0`, which no ESL lexer will accept. The replacement avoids
    /// every name anywhere in the term, so renaming can never capture.
    fn bind(&mut self, name: &str) -> String {
        if is_ident(name) {
            self.scope.push((name.to_string(), name.to_string()));
            return name.to_string();
        }
        let mut n = 0;
        let fresh = loop {
            let c = format!("x{n}");
            if !self.reserved.contains(&c) && !self.scope.iter().any(|(_, v)| *v == c) {
                break c;
            }
            n += 1;
        };
        self.scope.push((name.to_string(), fresh.clone()));
        fresh
    }

    fn unbind(&mut self) {
        self.scope.pop();
    }

    fn wrap(s: String, own: Prec, ctx: Prec) -> String {
        if own > ctx {
            format!("({s})")
        } else {
            s
        }
    }

    /// Render `v` on one line regardless of width, restoring the layout afterwards.
    ///
    /// `Pretty` decides each composite form by measuring its flat rendering — the classic
    /// "group": lay it out flat if it fits, otherwise break it. The measurement is a real render
    /// because a term's width depends on namespace aliases and binder renaming, neither of which
    /// is known without doing the work.
    fn flat(&mut self, v: &Value, ctx: Prec, path: &str) -> Result<String, PrintError> {
        let saved = self.layout;
        self.layout = Layout::Flat;
        let out = self.go(v, ctx, path, 0);
        self.layout = saved;
        out
    }

    fn go(&mut self, v: &Value, ctx: Prec, path: &str, ind: usize) -> Result<String, PrintError> {
        // Composite forms are the only ones with anywhere to break; everything else is an atom
        // whose flat rendering is the only rendering.
        if self.layout == Layout::Pretty {
            let flat = self.flat(v, ctx, path)?;
            if ind + flat.len() <= WIDTH {
                return Ok(flat);
            }
        }
        let obj = v
            .as_object()
            .ok_or_else(|| self.err(format!("expected a D47 node object, found `{v}`"), path))?;
        let ctor = obj
            .get("ctor")
            .and_then(Value::as_str)
            .ok_or_else(|| self.err("node has no `ctor`", path))?;
        let args = obj
            .get("args")
            .and_then(Value::as_array)
            .map_or(&[][..], |a| a);
        let sub = |i: usize| format!("{path}{ctor}[{i}].");

        // Free function, not a closure over `self`: the binder arms need `&mut self` while a
        // string arg is in hand.
        fn str_arg(args: &[Value], i: usize, ctor: &str, path: &str) -> Result<String, PrintError> {
            args.get(i)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| PrintError {
                    message: format!("`{ctor}` arg {i} must be a string"),
                    path: path.to_string(),
                })
        }
        let str_arg = |i: usize| str_arg(args, i, ctor, path);

        match ctor {
            "Var" => Ok(self.lookup(&str_arg(0)?)),

            "ConstRef" => {
                let iri = str_arg(0)?;
                let (a, l) = self.ns.split(&iri).map_err(|e| self.err(e, path))?;
                Ok(format!("{a}:{l}"))
            }

            // A constructor is written `<ns>:<CtorName>`, where `<ns>` maps to a URI that
            // PREFIXES the parent inductive's IRI — see `Compiler::resolve_ctor_iri`. So
            // `CtorApp["urn:eigenius:reasoning:JustifiedBy", "app"]` prints `reasoning:app`.
            //
            // Qualified rather than bare on purpose: bare resolution is by short name across every
            // chain-resident inductive, and `App` alone is already ambiguous between
            // `eigentt:TypeExpr:App` and `reasoning:JustificationTerm:App`.
            "CtorApp" => {
                let decl = str_arg(0)?;
                let name = str_arg(1)?;
                // `split` on the DECL IRI yields the alias for the ontology it lives in; the ctor
                // short name replaces the decl's own local part.
                let (alias, _decl_local) = self.ns.split(&decl).map_err(|e| self.err(e, path))?;
                Ok(format!("{alias}:{name}"))
            }

            // `Type n` is the ONLY undelimited multi-token form the printer emits, so it is the
            // only one whose atomicity is a claim rather than a syntactic fact. It holds because
            // ESL has no juxtaposition: application is `f(a, b)`, so nothing can bind between
            // `Type` and its level. `sorts_round_trip_in_every_position` in
            // kernel/tests/esl_round_trip.rs pins that — the parser wants `Type 1`, and an earlier
            // `Type(1)` here printed source that would not reparse at all.
            "Sort" => match args.first().and_then(Value::as_u64) {
                Some(0) => Ok("Prop".into()),
                Some(1) => Ok("Set".into()),
                // `Type n` is `Sort(n + 1)` — kernel/src/esl/compile.rs, SortKind::Type.
                Some(n) => Ok(format!("Type {}", n - 1)),
                None => Err(self.err("`Sort` needs a level", path)),
            },

            "LitString" => Ok(format!("\"{}\"", escape(&str_arg(0)?))),
            "LitInt" => args
                .first()
                .and_then(Value::as_i64)
                .map(|n| n.to_string())
                .ok_or_else(|| self.err("`LitInt` needs an integer", path)),
            "LitFloat" => args
                .first()
                .and_then(Value::as_f64)
                // A float must reparse as a float: `0` would lex as IntLit.
                .map(|f| {
                    if f.fract() == 0.0 {
                        format!("{f:.1}")
                    } else {
                        f.to_string()
                    }
                })
                .ok_or_else(|| self.err("`LitFloat` needs a number", path)),

            "UnitVal" => Ok("()".into()),

            "Fst" | "Snd" => {
                let (a, _) = self
                    .ns
                    .split("urn:eigenius:eigentt:x")
                    .map_err(|e| self.err(e, path))?;
                let prefix = format!("{a}:{}(", ctor.to_lowercase());
                // The operand opens on this same line, so its continuation lines align under
                // where it actually starts — not under this node's own indent.
                let inner = self.go(
                    args.first()
                        .ok_or_else(|| self.err(format!("`{ctor}` needs an operand"), path))?,
                    Prec::Binder,
                    &sub(0),
                    ind + prefix.len(),
                )?;
                Ok(format!("{prefix}{inner})"))
            }

            "Ann" => {
                let e = self.go(&args[0], Prec::Binder, &sub(0), ind + STEP)?;
                let t = self.go(&args[1], Prec::Binder, &sub(1), ind + STEP)?;
                Ok(format!("({e} : {t})"))
            }

            // `Pi` with an empty binder name is the non-dependent arrow.
            "Pi" if args.first().and_then(Value::as_str) == Some("") => {
                let dom = self.go(&args[1], Prec::Atom, &sub(1), ind)?;
                let cod = self.go(&args[2], Prec::Arrow, &sub(2), ind)?;
                // The arrow stays with the codomain, so a chain reads as a column of `-> T`.
                let joined = if self.layout == Layout::Pretty {
                    format!("{dom}\n{:ind$}-> {cod}", "")
                } else {
                    format!("{dom} -> {cod}")
                };
                Ok(Self::wrap(joined, Prec::Arrow, ctx))
            }

            "Pi" | "Sig" | "Lam" => {
                if args.len() != 3 {
                    return Err(self.err(format!("`{ctor}` needs 3 args"), path));
                }
                let dom = self.go(&args[1], Prec::Binder, &sub(1), ind + STEP)?;
                let name = self.bind(&str_arg(0)?);
                // The body starts a fresh line one level in, so nested binders stair-step.
                let body_col = if self.layout == Layout::Pretty {
                    ind + STEP
                } else {
                    ind
                };
                let body = self.go(&args[2], Prec::Binder, &sub(2), body_col);
                self.unbind();
                let body = body?;
                let head = match ctor {
                    "Pi" => format!("forall ({name} : {dom}) =>"),
                    "Sig" => format!("exists {name} : {dom} =>"),
                    _ => format!("fun ({name} : {dom}) =>"),
                };
                let s = if self.layout == Layout::Pretty {
                    format!("{head}\n{:body_col$}{body}", "")
                } else {
                    format!("{head} {body}")
                };
                Ok(Self::wrap(s, Prec::Binder, ctx))
            }

            "App" => {
                // Unfold the curried spine: ESL writes `f(a, b)`, never `f(a)(b)`.
                let (head, spine) = unfold_app(v);
                let head_path = format!("{path}{}", "App[0].".repeat(spine.len()));
                let h = self.go(head, Prec::Atom, &head_path, ind)?;
                let arg_col = ind + STEP;
                let mut parts = Vec::with_capacity(spine.len());
                for (i, a) in spine.iter().enumerate() {
                    let col = if self.layout == Layout::Pretty {
                        arg_col
                    } else {
                        ind
                    };
                    parts.push(self.go(a, Prec::Arrow, &format!("{path}App#{i}."), col)?);
                }
                Ok(if self.layout == Layout::Pretty {
                    // One argument per line: the spine's shape is the whole point of breaking.
                    format!(
                        "{h}(\n{:arg_col$}{}\n{:ind$})",
                        "",
                        parts.join(&format!(",\n{:arg_col$}", "")),
                        ""
                    )
                } else {
                    format!("{h}({})", parts.join(", "))
                })
            }

            other => Err(self.err(
                format!("`{other}` has no ESL surface form — cannot decompile this term"),
                path,
            )),
        }
    }
}

/// `App(App(f, a), b)` → `(f, [a, b])`.
fn unfold_app(v: &Value) -> (&Value, Vec<&Value>) {
    let mut spine = Vec::new();
    let mut cur = v;
    while let Some(args) = cur
        .get("ctor")
        .and_then(Value::as_str)
        .filter(|c| *c == "App")
        .and(cur.get("args"))
        .and_then(Value::as_array)
        .filter(|a| a.len() == 2)
    {
        spine.push(&args[1]);
        cur = &args[0];
    }
    spine.reverse();
    (cur, spine)
}

fn escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\t' => vec!['\\', 't'],
            c => vec![c],
        })
        .collect()
}

/// Print a term in the **inductive-value dialect** — the encoding a non-`type_expr` resource
/// property carries, e.g. `reasoning:justification`.
///
/// This is NOT D47. Compare the two encodings of the same idea:
///
/// ```text
/// D47 (type position):    {"ctor":"App","args":[{"ctor":"CtorApp","args":[<decl>,"app"]}, …]}
/// value dialect:          {"ctor":"DeclaredEvidence","args":["urn:eigenius:…"]}
/// ```
///
/// The value dialect names the constructor directly, applies it uncurried, and admits bare string
/// leaves. It also **omits the decl IRI**, so the namespace to qualify with cannot be recovered
/// from the term — the caller supplies it. The decompiler uses the holding property's own
/// namespace, since a property and the inductive its values inhabit are declared in the same
/// ontology (`reasoning:justification` holds a `reasoning:JustificationTerm`).
///
/// Qualification is not optional: bare `App` is ambiguous between `eigentt:TypeExpr:App` and
/// `reasoning:JustificationTerm:App`, and the compiler rightly refuses it.
pub fn print_value_term(
    term: &Value,
    ns: &mut Namespaces,
    ctor_namespace: &str,
) -> Result<String, PrintError> {
    let path = ".";
    let obj = term.as_object().ok_or_else(|| PrintError {
        message: format!("expected an inductive-value node, found `{term}`"),
        path: path.into(),
    })?;
    let ctor = obj
        .get("ctor")
        .and_then(Value::as_str)
        .ok_or_else(|| PrintError {
            message: "node has no `ctor`".into(),
            path: path.into(),
        })?;
    // Alias the ctor namespace by handing `split` a dummy local part; only the alias is used.
    let (alias, _) = ns
        .split(&format!("{ctor_namespace}:x"))
        .map_err(|e| PrintError {
            message: e,
            path: path.into(),
        })?;
    let args = obj
        .get("args")
        .and_then(Value::as_array)
        .map_or(&[][..], |a| a);
    if args.is_empty() {
        return Ok(format!("{alias}:{ctor}"));
    }
    let mut parts = Vec::with_capacity(args.len());
    for a in args {
        parts.push(match a {
            // A bare string leaf is an IRI or tag carried as `core:string`.
            Value::String(s) => format!("\"{}\"", escape(s)),
            Value::Number(n) => n.to_string(),
            _ => print_value_term(a, ns, ctor_namespace)?,
        });
    }
    Ok(format!("{alias}:{ctor}({})", parts.join(", ")))
}

/// D47 constructor names — the closed set [`print_type_expr`] understands.
///
/// Used to tell the two dialects apart when walking a document: a term carrying a ctor outside
/// this set cannot be D47. `App` is in both sets, which is why the test is over *every* node
/// rather than the root.
const D47_CTORS: &[&str] = &[
    "Lam",
    "Sort",
    "Pi",
    "Sig",
    "One",
    "UnitVal",
    "Pair",
    "Fst",
    "Snd",
    "App",
    "Ann",
    "Var",
    "ConstRef",
    "CtorApp",
    "Id",
    "LitString",
    "LitInt",
    "LitFloat",
];

/// Whether every constructor in `term` belongs to the D47 set.
///
/// Fails toward the value dialect: a term with an unrecognised ctor is treated as an inductive
/// value, and if that guess is wrong the recompile fails loudly rather than producing a term that
/// silently differs from the one on the chain.
pub fn is_d47_term(term: &Value) -> bool {
    match term {
        Value::Object(o) => {
            if let Some(c) = o.get("ctor").and_then(Value::as_str) {
                if !D47_CTORS.contains(&c) {
                    return false;
                }
            }
            o.values().all(is_d47_term)
        }
        Value::Array(a) => a.iter().all(is_d47_term),
        _ => true,
    }
}

/// Print an Eigon-JSON document (one resource object, or an array of them) as an ESL source file.
///
/// The inverse of loading that document: `eigenius compile` on the output yields the same
/// resources back, which [`kernel/tests/esl_round_trip.rs`] checks term-by-term.
///
/// Namespace aliases are pooled across every resource, so the file carries one preamble rather
/// than a per-resource one.
pub fn print_document(doc: &Value) -> Result<String, PrintError> {
    print_document_with(doc, Layout::Flat)
}

/// [`print_document`], laid out per `layout`.
pub fn print_document_with(doc: &Value, layout: Layout) -> Result<String, PrintError> {
    let resources = match doc {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let mut ns = Namespaces::new();
    let mut bodies = Vec::with_capacity(resources.len());
    for (i, r) in resources.iter().enumerate() {
        bodies.push(print_resource(r, &mut ns, &format!("[{i}]"), layout)?);
    }
    Ok(format!(
        "// Decompiled from Eigon-JSON by `eigenius decompile`.\n\n{}\n{}",
        ns.preamble(),
        bodies.join("\n")
    ))
}

/// `core:is_a` becomes the `: Class` in the resource header rather than a property.
const IS_A: &str = "urn:eigenius:core:is_a";

fn print_resource(
    r: &Value,
    ns: &mut Namespaces,
    path: &str,
    layout: Layout,
) -> Result<String, PrintError> {
    let bad = |m: String| PrintError {
        message: m,
        path: path.to_string(),
    };
    let obj = r
        .as_object()
        .ok_or_else(|| bad(format!("expected a resource object, found `{r}`")))?;
    let id = obj
        .get("@id")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("resource has no `@id`".into()))?;
    let (id_ns, id_local) = ns.split(id).map_err(bad)?;

    // `resource X : A, B { … }` — the header takes a class LIST, so every `is_a` is expressible.
    // Compiling ESL adds `reflection:DeclaredResource` to whatever the header names, so a
    // decompiled resource routinely carries two.
    let classes = obj
        .get(IS_A)
        .and_then(Value::as_array)
        .map_or(&[][..], |a| a);
    if classes.is_empty() {
        return Err(bad(
            "resource has no `core:is_a` class to put in the header".into(),
        ));
    }
    let mut names = Vec::with_capacity(classes.len());
    for c in classes {
        let iri = c
            .as_str()
            .ok_or_else(|| bad(format!("`core:is_a` entry is not an IRI: {c}")))?;
        let (c_ns, c_local) = ns.split(iri).map_err(bad)?;
        names.push(format!("{c_ns}:{c_local}"));
    }

    let mut out = format!("resource {id_ns}:{id_local} : {} {{\n", names.join(", "));
    for (k, v) in obj {
        if k == "@id" || k == IS_A {
            continue;
        }
        let (p_ns, p_local) = ns.split(k).map_err(bad)?;
        // The inductive a property's values inhabit is declared in the same ontology as the
        // property itself, so the property IRI minus its local name names the ctor namespace.
        let ctor_ns = k.rsplit_once(':').map_or("", |(p, _)| p).to_string();
        let rendered = print_property_value(v, ns, &ctor_ns, &format!("{path}.{k}"), layout)?;
        let _ = writeln!(out, "    {p_ns}:{p_local} = {rendered};");
    }
    out.push_str("}\n");
    Ok(out)
}

fn print_property_value(
    v: &Value,
    ns: &mut Namespaces,
    ctor_ns: &str,
    path: &str,
    layout: Layout,
) -> Result<String, PrintError> {
    match v {
        Value::String(s) => Ok(format!("\"{}\"", escape(s))),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(
            if n.is_f64() && n.as_f64().is_some_and(|f| f.fract() == 0.0) {
                // Keep a float a float: `2` would recompile as `LitInt`.
                format!("{:.1}", n.as_f64().unwrap_or_default())
            } else {
                n.to_string()
            },
        ),
        Value::Object(o) if o.contains_key("ctor") => {
            if is_d47_term(v) {
                // `type_expr(...)` is what marks the slot as a D47 TYPE. Without the wrapper the
                // compiler reads the same text as an inductive value — a different encoding.
                if layout == Layout::Pretty {
                    // The term gets its own block, indented under the property line.
                    let body = print_type_expr_with(v, ns, layout, 2 * STEP)?;
                    Ok(format!(
                        "type_expr(\n{:width$}{body}\n{:indent$})",
                        "",
                        "",
                        width = 2 * STEP,
                        indent = STEP
                    ))
                } else {
                    Ok(format!("type_expr({})", print_type_expr(v, ns)?))
                }
            } else {
                print_value_term(v, ns, ctor_ns)
            }
        }
        other => Err(PrintError {
            message: format!("no ESL surface for property value `{other}`"),
            path: path.to_string(),
        }),
    }
}
