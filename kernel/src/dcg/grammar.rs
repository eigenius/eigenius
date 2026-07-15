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
//! **The grammar** — everything the RULES need, as a value they are handed rather than a service they
//! can reach into.
//!
//! It carries the ontology chain (for `⟦·⟧`, unification, subsumption), the reserved-word table (rule
//! triggers — grammar, not lexicon), and the determiner category templates. It carries **no lexicon**,
//! and that is the point: the rules (`super::rules`) and the chart drivers (`super::chart`) are
//! `impl Grammar`, so they *cannot* reach a lexicon even if a future rule wanted to. A rule that can
//! call `entries_for` will eventually call it for something that is not a constant — which is how a
//! `form → entries` lookup grew a chart parser, a beam, and an LLM reranker in the first place.
//!
//! Seeding is the parser's job (it needs the lexicon); driving the chart is the grammar's (it does not).

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::term::Exp;

use super::category::is_ctor;
use super::lexicon::LexicalLookup;
use super::reserved::ReservedTable;

/// The **grammar constants** that a rule needs but that happen to be *stored* in the lexicon: the
/// raised categories of the existential determiners (`a`, `these`). The bare-plural/mass kind shift
/// borrows them, and so does the object appositive (whose `a_obj` raised category is one of `a`'s).
///
/// They are CONSTANTS, not lookups. Fetching them per call — which is what the code did — meant
/// `entries_for("a")` and `entries_for("these")` on **every `cat_n` item in every chart cell**, and on
/// the lazy path each of those takes a mutex lock and clones the entry vector. Resolved once, here.
///
/// Only the CATEGORY is kept: every consumer rebuilds the sem (`λV. V(kind)` etc.) from scratch.
pub(crate) struct DetTemplates {
    /// `cat_forall` categories of `a` — both the subject-raised (`fwd`-headed) and object-raised
    /// (`bwd`-headed) forms; the consumers pick by head.
    pub(crate) a: Vec<Exp>,
    /// `cat_forall` categories of `these` (the plural counterpart, for the bare-plural kind shift).
    pub(crate) these: Vec<Exp>,
}

impl DetTemplates {
    /// Resolve the templates from a lexicon — the ONE place the grammar reads a determiner's category.
    pub(crate) fn resolve(lex: &dyn LexicalLookup) -> Self {
        let cats = |form: &str| -> Vec<Exp> {
            lex.entries_for(form)
                .iter()
                .filter(|e| is_ctor(e.item.cat(), "cat_forall").is_some())
                .map(|e| e.item.cat().clone())
                .collect()
        };
        DetTemplates {
            a: cats("a"),
            these: cats("these"),
        }
    }
}

/// **The grammar** — everything the RULES need, as a value they are handed rather than a service they
/// can reach into.
///
/// That distinction is the whole point. The rules used to hang off the parser (and hence the lexicon)
/// for three reasons, two of which were just the determiner templates above; a rule that can call
/// `entries_for` will eventually call it for something that is not a constant, which is how a lexicon
/// lookup grew a chart parser in the first place. `Grammar` closes that door: it carries the ontology
/// chain (for `⟦·⟧`, unification, subsumption), the reserved-word table (rule triggers — grammar, not
/// lexicon), and the resolved templates. No lexicon.
pub(crate) struct Grammar {
    /// The chain the rules resolve against: inductive decls, class subsumption, the `⟦·⟧` recursor.
    pub(crate) layer: Arc<Layer>,
    /// The **reserved-construct table** (§11 3g.3 / B10): the reserved-word FORM set as *data*, loaded
    /// index-driven from the ontology (`lexicon:ReservedConstruct`). A reserved word has no lexical
    /// entry — it is a rule trigger — so this is grammar, not lexicon.
    pub(crate) reserved: ReservedTable,
    /// The determiner category templates ([`DetTemplates`]).
    pub(crate) dets: DetTemplates,
}
