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

use crate::ontology::well_known as wk;
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
    /// Level-variable names emitted while printing (eigenius#188).
    ///
    /// Recorded for the same reason as the aliases above: since a level variable must be bound by
    /// a `universe` declaration, printed source that mentions one does not recompile without it.
    /// Collecting them as they are printed keeps the preamble exact rather than hand-maintained.
    universes: BTreeSet<String>,
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
        let local = spell(local).ok_or_else(|| {
            format!(
                "IRI `{iri}` has local name `{local}`, which no ESL identifier can spell — the \
                 quoted form admits [A-Za-z0-9_-] only, and `#` is reserved"
            )
        })?;
        let local = local.as_str();
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
        // A keyword is not usable as an alias: `program:Foo` never lexes as one `QualName` token,
        // because `program` lexes as the `program` KEYWORD. The parser then reads the name as bare
        // `program`, eats the `:` as the class-list colon, and fails on the next one —
        // `expected LBrace, found Colon`. That is 79 of the shipped resources, all of them under
        // `urn:eigenius:program` (eigenius#222).
        //
        // Bumping is the existing collision behaviour, so keyword collisions join it rather than
        // getting their own path: `program` becomes `program2`.
        while self.taken.contains(&alias) || RESERVED.contains(&alias.as_str()) {
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
        // eigenius#188 — bind every level variable the body mentions. Without this, printed
        // source carrying `Sort u` does not recompile: a level variable is not auto-bound.
        if !self.universes.is_empty() {
            let names: Vec<&str> = self.universes.iter().map(String::as_str).collect();
            let _ = writeln!(out, "universe {};", names.join(" "));
        }
        out
    }

    /// Record a level variable the printer is about to emit.
    fn note_universe(&mut self, name: &str) {
        self.universes.insert(name.to_string());
    }
}

/// ESL keywords, which cannot serve as a namespace alias — a keyword never lexes as the namespace
/// half of a `QualName`. Read off the lexer's keyword table.
const RESERVED: &[&str] = &[
    "namespace",
    "class",
    "property",
    "resource",
    "program",
    "data",
    "merge_comorphism",
    "for",
    "text_index",
    "vector_index",
    "let",
    "alias",
    "in",
    "case",
    "match",
    "returning",
    "map",
    "reduce",
    "lambda",
    "pi",
    "forall",
    "exists",
    "fun",
    "axiom",
    "def",
    "macro",
    "universe",
    "true",
    "false",
    // `json` and `type_expr` are deliberately ABSENT: they are CONTEXTUAL, recognised as
    // `Ident(_) LParen` in value position rather than lexed as keywords, so neither breaks the
    // tight-colon `QualName` rule and both are usable as aliases. Verified.
    "Construct",
    "Prop",
    "Set",
    "Type",
    "Sort",
];

/// How this name is spelled in ESL: bare when it lexes as an identifier, quoted when it does not
/// but is still within the quoted charset, and `None` when nothing can spell it (eigenius#222).
///
/// **Quoting is minimal by design.** Quoting defensively would put `'…'` around every name in a
/// decompiled file and make the output unreadable, so the bare form wins whenever it lexes. The
/// consequence is that this predicate and the lexer must agree about what "lexes as an identifier"
/// means — the third place today where print and parse need one shared predicate rather than two
/// that drift.
///
/// A KEYWORD is spelled bare too. `expect_ident` accepts the full keyword set, so `fun` and
/// `program` are legal identifiers in an identifier position; what a keyword cannot do is form the
/// namespace half of a tight `QualName`, and that is the namespace ALIAS's problem, solved by
/// bumping the alias rather than by quoting it.
fn spell(name: &str) -> Option<String> {
    if is_ident(name) {
        return Some(name.to_string());
    }
    let quotable = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    quotable.then(|| format!("'{name}'"))
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

    /// Print an `eigentt:Level` tree in ESL's level syntax (eigenius#188), which is Lean 4's:
    /// numerals, variables, `l + n`, `max l r`, `imax l r`.
    ///
    /// Parenthesised whenever it is not an atom, because the level sits after `Sort` / `Type` and
    /// `max u v + 1` would otherwise reparse with the wrong shape.
    fn print_level(&mut self, v: &Value, path: &str) -> Result<String, PrintError> {
        if let Some(n) = level_as_nat(v) {
            return Ok(n.to_string());
        }
        let obj = v
            .as_object()
            .ok_or_else(|| self.err("universe level is not a Level value", path))?;
        let name = obj
            .get("ctor")
            .and_then(Value::as_str)
            .ok_or_else(|| self.err("universe level has no ctor", path))?;
        let args = obj
            .get("args")
            .and_then(Value::as_array)
            .ok_or_else(|| self.err("universe level has no args", path))?;
        let arg = |i: usize| -> Result<&Value, PrintError> {
            args.get(i)
                .ok_or_else(|| self.err("universe level is missing an argument", path))
        };
        match name {
            "Param" => {
                let n = arg(0)?
                    .as_str()
                    .ok_or_else(|| self.err("`Param` level takes a name", path))?
                    .to_string();
                self.ns.note_universe(&n);
                Ok(n)
            }
            // A `Succ` over a non-numeral base: `l + 1`, accumulated so `Succ(Succ(u))` is `u + 2`
            // rather than `(u + 1) + 1`.
            "Succ" => {
                let mut n = 0u64;
                let mut cur = v;
                while let Some(o) = cur.as_object() {
                    if o.get("ctor").and_then(Value::as_str) != Some("Succ") {
                        break;
                    }
                    n += 1;
                    cur = o
                        .get("args")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .ok_or_else(|| self.err("`Succ` level takes a base", path))?;
                }
                let base = cur.clone();
                Ok(format!("({} + {n})", self.print_level(&base, path)?))
            }
            "Max" | "IMax" => {
                // Bind the arguments before recursing: `arg` borrows `args`, and `print_level`
                // needs `&mut self` to record level variables into the preamble.
                let (a0, a1) = (arg(0)?.clone(), arg(1)?.clone());
                let l = self.print_level(&a0, path)?;
                let r = self.print_level(&a1, path)?;
                let op = if name == "Max" { "max" } else { "imax" };
                Ok(format!("({op} {l} {r})"))
            }
            other => Err(self.err(
                format!("`{other}` is not an eigentt:Level constructor"),
                path,
            )),
        }
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
            // `CtorApp["urn:eigenius:justification:Certificate", "app"]` prints `justification:app`.
            //
            // Qualified rather than bare on purpose: bare resolution is by short name across every
            // chain-resident inductive, and `App` alone is already ambiguous between
            // `eigentt:Term:App` and `justification:Term:App`.
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
            "Sort" => match args.first().and_then(level_as_nat) {
                Some(0) => Ok("Prop".into()),
                Some(1) => Ok("Set".into()),
                // `Type n` is `Sort(n + 1)` — kernel/src/esl/compile.rs, SortKind::Type.
                Some(n) => Ok(format!("Type {}", n - 1)),
                // eigenius#188: a polymorphic level prints in the general form, `Sort <level>`.
                // The numeral cases above stay on the abbreviations so the 942 monomorphic uses
                // in the tree print exactly as they are written.
                None => {
                    let l = args
                        .first()
                        .ok_or_else(|| self.err("`Sort` needs a level", path))?;
                    Ok(format!("Sort {}", self.print_level(l, path)?))
                }
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
            "LitBool" => args
                .first()
                .and_then(Value::as_bool)
                .map(|b| b.to_string())
                .ok_or_else(|| self.err("`LitBool` needs a boolean", path)),

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
/// property carries, e.g. `justification:term`.
///
/// This is NOT D47. Compare the two encodings of the same idea:
///
/// ```text
/// D47 (type position):    {"ctor":"App","args":[{"ctor":"CtorApp","args":[<decl>,"app"]}, …]}
/// value dialect:          {"ctor":"Declared","args":["urn:eigenius:…"]}
/// ```
///
/// The value dialect names the constructor directly, applies it uncurried, and admits bare string
/// leaves. It also **omits the decl IRI**, so the namespace to qualify with cannot be recovered
/// from the term — the caller supplies it. The decompiler uses the holding property's own
/// namespace, since a property and the inductive its values inhabit are declared in the same
/// ontology (`justification:term` holds a `justification:Term`).
///
/// Qualification is not optional: bare `App` is ambiguous between `eigentt:Term:App` and
/// `justification:Term:App`, and the compiler rightly refuses it.
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

/// Read a `Sort`'s level argument as a numeral.
///
/// Since eigenius#188 the argument is an `eigentt:Level` tree, so `Set` arrives as `Succ(Zero)`.
/// The pre-#188 bare integer is NOT accepted: retyping the ctor moved the bootstrap manifest, so
/// every store carrying the old form had to be reseeded, and a reseed re-encodes from source.
///
/// `None` for a level that is not a closed `Succ`-chain — there is no ESL surface syntax for one.
fn level_as_nat(v: &Value) -> Option<u64> {
    let obj = v.as_object()?;
    let mut n = 0u64;
    let mut cur = obj;
    loop {
        match cur.get("ctor").and_then(Value::as_str)? {
            "Zero" => return Some(n),
            "Succ" => {
                n = n.checked_add(1)?;
                cur = cur.get("args")?.as_array()?.first()?.as_object()?;
            }
            _ => return None,
        }
    }
}

/// D47 constructor names — the closed set [`print_type_expr`] understands.
///
/// Used to tell the two dialects apart when walking a document: a term carrying a ctor outside
/// this set cannot be D47. `App` is in both sets, which is why the test is over *every* node
/// rather than the root.
const D47_CTORS: &[&str] = &[
    "Lam",
    "Sort",
    // eigenius#188 — `Sort`'s argument is an `eigentt:Level` tree, so its constructors are part
    // of the D47 dialect too. Without them a term containing any sort is classified non-D47 and
    // printed by the generic `alias:Ctor(...)` printer, whose output does not reparse.
    "Zero",
    "Succ",
    "Max",
    "IMax",
    "Param",
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
    "LitBool",
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

    // An inductive DECLARATION has its own surface form, and printing it as a `resource` block
    // does not round-trip: the text recompiles through the resource path, never reaching
    // `compile_data`, so the constructor telescope is not reconstructed (eigenius#217).
    //
    // In practice it did not even get that far — `core:ctors` holds embedded `InductiveCtor`
    // resources, and `print_property_value` has no surface for those, so decompiling ANY inductive
    // failed outright with "no ESL surface for property value". Measured over the shipped
    // ontologies: 5 of 5 inductives failed; every other resource printed.
    if classes
        .iter()
        .any(|c| c.as_str() == Some(wk::INDUCTIVE_TYPE))
    {
        return print_data(obj, &id_ns, &id_local, ns, path, layout);
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

/// Print a `core:InductiveType` resource as the `data` declaration it came from.
///
/// Inverts `esl::compile`'s `compile_data`: the parameter telescope from `core:type_params`, the
/// index telescope from `core:indices`, the result sort from `core:result_sort`, and each
/// constructor from `core:ctors` — positional (`core:arg_types`) or typed (`core:ctor_type`),
/// whichever the resource carries, matching the two forms the compiler emits.
fn print_data(
    obj: &serde_json::Map<String, Value>,
    id_ns: &str,
    id_local: &str,
    ns: &mut Namespaces,
    path: &str,
    layout: Layout,
) -> Result<String, PrintError> {
    let bad = |m: String| PrintError {
        message: m,
        path: path.to_string(),
    };

    let telescope = |key: &str, ns: &mut Namespaces| -> Result<Vec<String>, PrintError> {
        let Some(arr) = obj.get(key).and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        arr.iter()
            .map(|entry| {
                let e = entry
                    .as_object()
                    .ok_or_else(|| bad(format!("`{key}` entry is not a resource")))?;
                let name = e
                    .get(wk::PARAM_NAME)
                    .and_then(Value::as_str)
                    .ok_or_else(|| bad(format!("`{key}` entry has no `param_name`")))?;
                let kind = e
                    .get(wk::PARAM_KIND)
                    .ok_or_else(|| bad(format!("`{key}` entry has no `param_kind`")))?;
                Ok(format!("{name} : {}", print_kind(kind, ns, path)?))
            })
            .collect()
    };

    let params = telescope(wk::TYPE_PARAMS, ns)?;
    let indices = telescope(wk::INDICES, ns)?;

    // `core:result_sort` is a `core:Level` tree; absent defaults to `Set` (`Succ(Zero)`).
    let result = match obj.get(wk::RESULT_SORT) {
        Some(v) => sort_text(v, ns, path)?,
        None => "Set".to_string(),
    };

    let mut header = format!("data {id_ns}:{id_local}");
    if !params.is_empty() {
        let _ = write!(header, "({})", params.join(", "));
    }
    // The header's type is `index₁ -> … -> resultSort`. It is omitted only when there are no
    // indices AND the result is the `Set` default, which is what `compile_data` assumes.
    if !indices.is_empty() || result != "Set" {
        let mut chain: Vec<String> = indices
            .iter()
            .map(|p| p.rsplit(" : ").next().unwrap_or(p).to_string())
            .collect();
        chain.push(result);
        let _ = write!(header, " : {}", chain.join(" -> "));
    }

    let ctors = obj
        .get(wk::CTORS)
        .and_then(Value::as_array)
        .map_or(&[][..], |a| a);
    let mut lines = Vec::with_capacity(ctors.len());
    for (i, c) in ctors.iter().enumerate() {
        let cpath = format!("{path}.ctors[{i}]");
        let co = c
            .as_object()
            .ok_or_else(|| bad(format!("`ctors[{i}]` is not a resource")))?;
        let name = co
            .get(wk::CTOR_NAME)
            .and_then(Value::as_str)
            .ok_or_else(|| bad(format!("`ctors[{i}]` has no `ctor_name`")))?;
        if let Some(ct) = co.get(wk::CTOR_TYPE) {
            // Typed form: the whole Π chain is one D47 term.
            lines.push(format!(
                "    {name} : {},",
                print_type_expr_with(ct, ns, layout, 4)?
            ));
        } else {
            let args = co
                .get(wk::ARG_TYPES)
                .and_then(Value::as_array)
                .map_or(&[][..], |a| a);
            if args.is_empty() {
                lines.push(format!("    {name},"));
            } else {
                let rendered: Vec<String> = args
                    .iter()
                    .map(|a| {
                        let t = print_arg_type(a, ns, &cpath)?;
                        // `core:arg_name` prints as the named form `base : ex:Nat` (eigenius#221).
                        match a
                            .as_object()
                            .and_then(|o| o.get(wk::ARG_NAME))
                            .and_then(Value::as_str)
                        {
                            Some(n) => Ok(format!("{n} : {t}")),
                            None => Ok(t),
                        }
                    })
                    .collect::<Result<Vec<_>, PrintError>>()?;
                lines.push(format!("    {name}({}),", rendered.join(", ")));
            }
        }
    }
    // `description = "…";` leads the body, as it does for `class` (eigenius#221).
    if let Some(d) = obj.get(wk::DESCRIPTION).and_then(Value::as_str) {
        lines.insert(0, format!("    description = \"{}\";", escape(d)));
    }
    Ok(format!("{header} {{\n{}\n}}\n", lines.join("\n")))
}

/// An `InductiveArgType` as constructor-argument source: `type_name` applied to `type_args`.
fn print_arg_type(v: &Value, ns: &mut Namespaces, path: &str) -> Result<String, PrintError> {
    let o = v.as_object().ok_or_else(|| PrintError {
        message: "constructor argument is not a resource".into(),
        path: path.to_string(),
    })?;
    let head = o.get(wk::TYPE_NAME).ok_or_else(|| PrintError {
        message: "constructor argument has no `type_name`".into(),
        path: path.to_string(),
    })?;
    let head = print_kind(head, ns, path)?;
    let args = o
        .get(wk::TYPE_ARGS)
        .and_then(Value::as_array)
        .map_or(&[][..], |a| a);
    if args.is_empty() {
        return Ok(head);
    }
    let rendered: Vec<String> = args
        .iter()
        .map(|a| print_arg_type(a, ns, path))
        .collect::<Result<_, _>>()?;
    Ok(format!("{head}({})", rendered.join(", ")))
}

/// A kind or type reference — the `eigentt:Term` head that `Compiler::lower_kind` produced.
/// Inverts it: `Var` is a bare parameter name, `ConstRef` a qualified IRI, `Sort` a sort keyword.
fn print_kind(v: &Value, ns: &mut Namespaces, path: &str) -> Result<String, PrintError> {
    let bad = |m: &str| PrintError {
        message: m.to_string(),
        path: path.to_string(),
    };
    let o = v.as_object().ok_or_else(|| bad("kind is not a Term"))?;
    let ctor = o
        .get("ctor")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("kind has no ctor"))?;
    let arg0 = || {
        o.get("args")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
    };
    match ctor {
        "Var" => Ok(arg0()
            .and_then(Value::as_str)
            .ok_or_else(|| bad("`Var` takes a name"))?
            .to_string()),
        "ConstRef" => {
            let iri = arg0()
                .and_then(Value::as_str)
                .ok_or_else(|| bad("`ConstRef` takes an IRI"))?;
            let (a, b) = ns.split(iri).map_err(|m| bad(&m))?;
            Ok(format!("{a}:{b}"))
        }
        "Sort" => sort_text(arg0().ok_or_else(|| bad("`Sort` takes a level"))?, ns, path),
        other => Err(bad(&format!("`{other}` is not a kind"))),
    }
}

/// A `core:Level` as the sort keyword the surface spells it with: `Prop`, `Set`, `Type n`, or
/// `Sort <level>` when the level is not a numeral.
fn sort_text(level: &Value, ns: &mut Namespaces, path: &str) -> Result<String, PrintError> {
    if let Some(n) = level_as_nat(level) {
        return Ok(match n {
            0 => "Prop".to_string(),
            1 => "Set".to_string(),
            n => format!("Type {}", n - 1),
        });
    }
    let mut p = Printer {
        ns,
        scope: Vec::new(),
        reserved: BTreeSet::new(),
        layout: Layout::Flat,
    };
    Ok(format!("Sort {}", p.print_level(level, path)?))
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
        // An array-valued property (`core:value_array` / `core:resource_array`): each element
        // through the same rendering. Refs and strings are indistinguishable in Eigon-JSON —
        // a reference IS an IRI string — so elements print as STRING LITERALS,
        // valid ESL that round-trips because the validator reinterprets a string IRI per the
        // property's data_type (the persist-round-trip invariant, Rule 3).
        Value::Array(a) => {
            let els: Vec<String> = a
                .iter()
                .enumerate()
                .map(|(i, el)| {
                    print_property_value(el, ns, ctor_ns, &format!("{path}[{i}]"), layout)
                })
                .collect::<Result<_, _>>()?;
            Ok(format!("[{}]", els.join(", ")))
        }
        // An EMBEDDED RESOURCE — `{ ns:prop = value; … }`. A general chain feature, not one tied
        // to any construct: `ast::Value::Block` compiles to `Resource::new_embedded()` in any
        // property position, and this is its inverse.
        //
        // The arm was missing entirely, so decompiling a resource with ANY embedded value failed
        // (eigenius#222) — `core:ConditionalRequirement` on `core:Property`, `julia:interval`
        // bounds, `program:Apply` inside a comorphism `Lambda`. The same absence is what made an
        // inductive undecompilable before eigenius#217, since `core:ctors` holds embedded
        // `InductiveCtor` resources.
        //
        // An `@id` is NOT expressible in a block — the surface mints embedded resources
        // anonymously — so a keyed embedded resource is refused rather than printed lossily.
        Value::Object(o) => {
            // Embedded RESOURCE or opaque JSON? `serialize_resource` flattens both to a plain
            // object — `Value::Embedded(r) => serialize_resource(r)`, `Value::Json(v) => v` — so
            // the distinction is not in the wire form and must be recovered.
            //
            // Recovered with the SAME rule the reader uses, deliberately: `parse_json_value`
            // decides on `keys().any(|k| k == "@id" || Iri::parse(k).is_ok())`. Sharing the
            // predicate is what makes print and parse agree; inventing a second one here is how
            // they would drift.
            let any_iri_key = o
                .keys()
                .any(|k| k == "@id" || crate::ontology::iri::Iri::parse(k).is_ok());
            if !any_iri_key {
                // Opaque JSON — `json( … )`. The wrapper is load-bearing: without it the same
                // braces reparse as a `Block`, i.e. an embedded resource.
                return Ok(format!(
                    "json({})",
                    serde_json::to_string(v).map_err(|e| PrintError {
                        message: format!("value is not serialisable JSON: {e}"),
                        path: path.to_string(),
                    })?
                ));
            }
            if let Some(id) = o.get("@id").and_then(Value::as_str) {
                return Err(PrintError {
                    message: format!(
                        "embedded resource carries `@id` `{id}`, which a block value cannot \
                         express — the ESL surface mints embedded resources anonymously"
                    ),
                    path: path.to_string(),
                });
            }
            let mut fields = Vec::with_capacity(o.len());
            for (k, v) in o {
                let (p_ns, p_local) = ns.split(k).map_err(|m| PrintError {
                    message: m,
                    path: path.to_string(),
                })?;
                let inner_ctor_ns = k.rsplit_once(':').map_or("", |(p, _)| p).to_string();
                let rendered =
                    print_property_value(v, ns, &inner_ctor_ns, &format!("{path}.{k}"), layout)?;
                fields.push(format!("{p_ns}:{p_local} = {rendered};"));
            }
            Ok(format!("{{ {} }}", fields.join(" ")))
        }
        other => Err(PrintError {
            message: format!("no ESL surface for property value `{other}`"),
            path: path.to_string(),
        }),
    }
}
