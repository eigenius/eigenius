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

//! A compact, human-readable pretty-printer for parse [`Exp`] terms — categories
//! (`cat_s(dcl, fin)`, `fwd(A, B)`) and semantics (`affects(brca1, hela)`,
//! `And(p, q)`, `λx. P`). The derived `Debug` on `Exp` inlines whole `InductiveDecl`
//! bodies (every constructor of `Cat`, `Mood`, …), which is unreadable; this renders
//! just the spine. Best-effort: any variant it doesn't special-case falls back to its
//! constructor name so output stays bounded.

use crate::nbe::term::{Exp, Patt};
use crate::ontology::Iri;

/// The local segment of an IRI (the part after the final `:`), for compact display.
fn local(iri: &Iri) -> String {
    let s = iri.as_str();
    s.rsplit(':').next().unwrap_or(s).to_string()
}

fn patt(p: &Patt) -> String {
    match p {
        Patt::Var(n) => n.clone(),
        Patt::Unit => "_".to_string(),
        Patt::Pair(a, b) => format!("({}, {})", patt(a), patt(b)),
    }
}

/// Flatten an application spine `((f a) b) c` into `(f, [a, b, c])`.
fn unspine(e: &Exp) -> (&Exp, Vec<&Exp>) {
    let mut args = Vec::new();
    let mut head = e;
    while let Exp::App(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    args.reverse();
    (head, args)
}

fn join(args: &[&Exp]) -> String {
    args.iter()
        .map(|a| pretty_term(a))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render `e` as a compact one-line string.
pub fn pretty_term(e: &Exp) -> String {
    match e {
        Exp::App(_, _) => {
            let (head, args) = unspine(e);
            if args.is_empty() {
                pretty_term(head)
            } else {
                format!("{}({})", pretty_term(head), join(&args))
            }
        }
        // `Con` is a single-arg constructor wrapper.
        Exp::Con(name, body) => format!("{name}({})", pretty_term(body)),
        Exp::InductiveCtor(_, name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                format!("{name}({})", join(&args.iter().collect::<Vec<_>>()))
            }
        }
        Exp::InductiveType(decl, args) => {
            if args.is_empty() {
                decl.name.clone()
            } else {
                format!("{}({})", decl.name, join(&args.iter().collect::<Vec<_>>()))
            }
        }
        Exp::EigonAxiom(iri) | Exp::EigonClass(iri) => local(iri),
        Exp::EigonResource(r) => r
            .id()
            .map(local)
            .unwrap_or_else(|| "<resource>".to_string()),
        Exp::Var(n) => n.clone(),
        Exp::Lam(p, body) => format!("λ{}. {}", patt(p), pretty_term(body)),
        Exp::Pi(p, a, b) => match p {
            Patt::Unit => format!("{} → {}", pretty_term(a), pretty_term(b)),
            _ => format!("Π{}:{}. {}", patt(p), pretty_term(a), pretty_term(b)),
        },
        Exp::Arrow(a, b) => format!("{} → {}", pretty_term(a), pretty_term(b)),
        Exp::Sig(p, a, b) => match p {
            Patt::Unit => format!("{} × {}", pretty_term(a), pretty_term(b)),
            _ => format!("Σ{}:{}. {}", patt(p), pretty_term(a), pretty_term(b)),
        },
        Exp::Times(a, b) => format!("{} × {}", pretty_term(a), pretty_term(b)),
        Exp::Pair(a, b) => format!("({}, {})", pretty_term(a), pretty_term(b)),
        Exp::Fst(a) => format!("{}.1", pretty_term(a)),
        Exp::Snd(a) => format!("{}.2", pretty_term(a)),
        Exp::Ann(a, _) => pretty_term(a),
        Exp::Sort(0) => "Prop".to_string(),
        Exp::Sort(1) => "Set".to_string(),
        Exp::Sort(n) => format!("Sort({n})"),
        Exp::One => "1".to_string(),
        Exp::Unit => "()".to_string(),
        Exp::LitString(s) => format!("{s:?}"),
        Exp::LitInt(i) => i.to_string(),
        Exp::LitFloat(f) => f.to_string(),
        // Bounded fallback for any variant not special-cased: the variant kind, never
        // the full Debug (which would inline inductive declarations).
        other => exp_kind(other).to_string(),
    }
}

/// Render a category's **structural shape** — `pretty_term` with every type INDEX
/// (an `EigonClass` / `EigonResource` / refined `Σ`) erased to `_`, while keeping the
/// constructor spine (`cat_n`, `cat_np`, `cat_s`, `fwd`, `bwd`, `cat_forall`) and the
/// FEATURE atoms (`sg`/`pl`/`num_any`/`mass`/`dcl`/`fin`/…). So `cat_n(wn:n123, sg)`
/// and `cat_n(umlscui:C1, sg)` both render `cat_n(_, sg)`. Used to histogram chart cells:
/// many items sharing ONE shape ⇒ lexical/sense variation (a type-narrowing candidate);
/// many distinct shapes ⇒ structural ambiguity (type-narrowing won't help). Diagnostic.
pub fn cat_shape(e: &Exp) -> String {
    match e {
        Exp::App(_, _) => {
            let (head, args) = unspine(e);
            if args.is_empty() {
                cat_shape(head)
            } else {
                let inner = args
                    .iter()
                    .map(|a| cat_shape(a))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({inner})", cat_shape(head))
            }
        }
        Exp::Con(name, body) => format!("{name}({})", cat_shape(body)),
        Exp::InductiveCtor(_, name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner = args.iter().map(cat_shape).collect::<Vec<_>>().join(", ");
                format!("{name}({inner})")
            }
        }
        Exp::InductiveType(decl, args) => {
            if args.is_empty() {
                decl.name.clone()
            } else {
                let inner = args.iter().map(cat_shape).collect::<Vec<_>>().join(", ");
                format!("{}({inner})", decl.name)
            }
        }
        // Type indices / sense identities — erased.
        Exp::EigonClass(_) | Exp::EigonResource(_) | Exp::EigonAxiom(_) => "_".to_string(),
        // A refined noun's index is a `Σ`; keep the shape marker, erase the components.
        Exp::Sig(_, _, _) | Exp::Times(_, _) => "Σ_".to_string(),
        Exp::Arrow(a, b) => format!("{} → {}", cat_shape(a), cat_shape(b)),
        Exp::Pi(_, a, b) => format!("{} → {}", cat_shape(a), cat_shape(b)),
        Exp::Var(_) => "_".to_string(),
        Exp::Sort(0) => "Prop".to_string(),
        Exp::Sort(1) => "Set".to_string(),
        // Sems (a leaf's `sem`, not a cat) collapse — we only shape categories.
        _ => "_".to_string(),
    }
}

/// A short label for the `Exp` variant — the fallback so output never explodes.
fn exp_kind(e: &Exp) -> &'static str {
    match e {
        Exp::Lam(_, _) => "<lam>",
        Exp::Sort(_) => "<sort>",
        Exp::Pi(_, _, _) => "<pi>",
        Exp::Sig(_, _, _) => "<sig>",
        Exp::Data(_) => "<data>",
        Exp::Case(_) => "<case>",
        Exp::Dec(_, _) => "<dec>",
        Exp::Id(_, _, _) => "<id>",
        Exp::Refl(_) => "<refl>",
        Exp::IdJ(_) => "<idj>",
        Exp::NativeDecide(_, _) => "<native-decide>",
        Exp::DecEq(_, _, _) => "<deceq>",
        Exp::EigonPrimitive(_) => "<primitive>",
        Exp::PropAccess(_, _) => "<prop-access>",
        Exp::Template(_, _) => "<template>",
        Exp::Construct(_, _) => "<construct>",
        Exp::Codata(_) => "<codata>",
        Exp::CoRecord(_) => "<corecord>",
        Exp::Observe(_, _) => "<observe>",
        Exp::Map(_, _) => "<map>",
        Exp::Reduce(_, _, _) => "<reduce>",
        _ => "<term>",
    }
}
