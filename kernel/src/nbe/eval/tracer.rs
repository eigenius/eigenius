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

//! Tracing parameter for the evaluator (NbE analysis §3.2).
//!
//! `eval_impl<T: Tracer>` is the single evaluator; the `Tracer`
//! decides what provenance each evaluation step produces. [`NoTrace`]
//! (`Node = ()`) makes every hook a no-op that monomorphizes away —
//! the type-checker's hot path pays nothing. [`TreeTracer`]
//! (`Node = Option<Trace>`) builds the D6b §2 tree-structured
//! `ProgramTrace` used at the IO run boundary.
//!
//! Node-emission rule: a node is produced only where computation
//! worth recording happened (component/comorphism dispatch, a
//! projection, a taken branch); structural expressions contribute a
//! node only by combining their children's — `Trace::Seq` when more
//! than one child carries a trace. Pure subtrees collapse to `None`.

use crate::nbe::term::Patt;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::program::trace::{ComponentTrace, Trace};

/// What one evaluation step reports to the trace being built.
///
/// Methods mirror the D6b trace-node vocabulary; `combine` is the
/// generic structural join for expressions with several children.
pub(crate) trait Tracer {
    type Node;

    /// No computation to record (pure leaf).
    fn leaf() -> Self::Node;
    /// Join the child nodes of one structural expression.
    fn combine(children: Vec<Self::Node>) -> Self::Node;
    /// A `Dec` (let) binding: value and body children.
    fn let_node(patt: &Patt, value: Self::Node, body: Self::Node) -> Self::Node;
    /// A property projection on a resource or neutral.
    fn project(source: Self::Node, property: &Iri) -> Self::Node;
    /// A `Construct` with per-field children.
    fn construct(fields: Vec<(Iri, Self::Node)>) -> Self::Node;
    /// A `Map` with per-element children.
    fn map(elements: Vec<Self::Node>) -> Self::Node;
    /// A `Reduce` with per-step children.
    fn reduce(steps: Vec<Self::Node>) -> Self::Node;
    /// A taken case/match branch: scrutinee child, branch name, body child.
    fn case(scrutinee: Self::Node, branch: &str, body: Self::Node) -> Self::Node;
    /// A component dispatch. `trace` is the `ComponentTrace` the hook's
    /// `dispatch_component` returned (if any) — the node wraps it.
    fn component(trace: Option<ComponentTrace>) -> Self::Node;
    /// A comorphism dispatch (D14 §9.3): source child plus the
    /// target IRI/class read from the translated resource value.
    fn comorphism(comorphism_iri: &Iri, source: Self::Node, translated: &Val) -> Self::Node;
}

/// Zero-cost tracer: every node is `()`, every hook a no-op.
pub(crate) enum NoTrace {}

impl Tracer for NoTrace {
    type Node = ();

    #[inline(always)]
    fn leaf() -> Self::Node {}
    #[inline(always)]
    fn combine(_: Vec<Self::Node>) -> Self::Node {}
    #[inline(always)]
    fn let_node(_: &Patt, _: Self::Node, _: Self::Node) -> Self::Node {}
    #[inline(always)]
    fn project(_: Self::Node, _: &Iri) -> Self::Node {}
    #[inline(always)]
    fn construct(_: Vec<(Iri, Self::Node)>) -> Self::Node {}
    #[inline(always)]
    fn map(_: Vec<Self::Node>) -> Self::Node {}
    #[inline(always)]
    fn reduce(_: Vec<Self::Node>) -> Self::Node {}
    #[inline(always)]
    fn case(_: Self::Node, _: &str, _: Self::Node) -> Self::Node {}
    #[inline(always)]
    fn component(_: Option<ComponentTrace>) -> Self::Node {}
    #[inline(always)]
    fn comorphism(_: &Iri, _: Self::Node, _: &Val) -> Self::Node {}
}

/// Tree-building tracer producing the D6b §2 `Trace` nodes.
pub(crate) enum TreeTracer {}

impl Tracer for TreeTracer {
    type Node = Option<Trace>;

    fn leaf() -> Self::Node {
        None
    }

    fn combine(children: Vec<Self::Node>) -> Self::Node {
        let mut present: Vec<Trace> = children.into_iter().flatten().collect();
        match present.len() {
            0 => None,
            1 => Some(present.pop().unwrap()),
            _ => Some(Trace::Seq(present)),
        }
    }

    fn let_node(patt: &Patt, value: Self::Node, body: Self::Node) -> Self::Node {
        if value.is_none() && body.is_none() {
            return None;
        }
        let name = match patt {
            Patt::Var(n) => n.clone(),
            _ => "_".to_string(),
        };
        Some(Trace::Let {
            name,
            value_trace: value.map(Box::new),
            body_trace: body.map(Box::new),
        })
    }

    fn project(source: Self::Node, property: &Iri) -> Self::Node {
        Some(Trace::Project {
            source_trace: source.map(Box::new),
            property: property.clone(),
        })
    }

    fn construct(fields: Vec<(Iri, Self::Node)>) -> Self::Node {
        if fields.iter().all(|(_, t)| t.is_none()) {
            return None;
        }
        Some(Trace::Construct {
            field_traces: fields.into_iter().collect(),
        })
    }

    fn map(elements: Vec<Self::Node>) -> Self::Node {
        if elements.iter().all(|t| t.is_none()) {
            return None;
        }
        Some(Trace::Map {
            element_traces: elements,
        })
    }

    fn reduce(steps: Vec<Self::Node>) -> Self::Node {
        if steps.iter().all(|t| t.is_none()) {
            return None;
        }
        Some(Trace::Reduce { step_traces: steps })
    }

    fn case(scrutinee: Self::Node, branch: &str, body: Self::Node) -> Self::Node {
        if scrutinee.is_none() && body.is_none() {
            return None;
        }
        Some(Trace::Case {
            scrutinee_trace: scrutinee.map(Box::new),
            branch_taken: branch.to_string(),
            branch_trace: body.map(Box::new),
        })
    }

    fn component(trace: Option<ComponentTrace>) -> Self::Node {
        trace.map(Trace::Component)
    }

    fn comorphism(comorphism_iri: &Iri, source: Self::Node, translated: &Val) -> Self::Node {
        let (target_iri, target_class) = match translated {
            Val::ResourceVal(r) => {
                let id = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
                let class = r
                    .is_a()
                    .first()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                (id, class)
            }
            _ => (String::new(), String::new()),
        };
        Some(Trace::Comorphism {
            comorphism_iri: comorphism_iri.as_str().to_string(),
            source_trace: source.map(Box::new),
            target_iri,
            target_class,
        })
    }
}
