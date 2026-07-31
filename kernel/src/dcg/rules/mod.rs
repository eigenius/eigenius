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

//! **The grammar's rules** — everything that says how two constituents may combine.
//!
//! - [`combinators`] — the CATEGORIAL rules: forward/backward application, composition, the dependent
//!   determiner, the nominal-modification family. Decided sem-blind (they see only a
//!   [`CategoryPayload`](super::item::CategoryPayload)), which is what licenses the packed forest.
//! - [`registry`] — the TOKEN-KEYED rules (relatives, coordination, `but not`, the reciprocal, the
//!   appositives) plus the unary shifts. One definition of *where* each fires, consumed by both chart
//!   drivers, so the two cannot drift apart.
//!
//! Everything here depends on the [`Grammar`](super::grammar::Grammar) — the chain, the reserved-word
//! triggers, and the resolved category templates — and on nothing else. In particular, no rule can reach
//! a lexicon; if one could, it would eventually reach for something that is not a grammar constant,
//! which is how a `form → entries` lookup grew a chart parser in the first place.
//!
//! (`combinators.rs` is the file formerly known as `parser.rs`. It never held a parser — the chart
//! drivers live in `super::chart` — it holds the composition rules, and now it says so.)

pub(crate) mod combinators;
pub(crate) mod constructions;
pub(crate) mod registry;

/// **What immediately follows a chart cell** — the only right-context any rule needs, and the reason
/// two separate defects were unfixable without it.
///
/// A CCG rule normally sees only its two operands. Two constraints genuinely cannot be stated that way,
/// because both are about whether a construction is *complete* at the point it is built:
///
/// - **Classifier capture.** In "the MMR genes MSH2, MSH6, PMS2 or MLH1" the classifier must appose the
///   WHOLE designator list; binding it to `MSH2` alone and coordinating that NP with the remaining
///   names is a different (wrong) bracketing. Whether the list continues is exactly [`Self::Comma`] at
///   the `[classifier designator]` cell's right edge.
/// - **List finalization.** A comma list may fold only when no coordinator follows it, which is what
///   separates the asyndetic `A, B affect X` (folds as `∧`) from the prefix `A, B` of `A, B, C or D`
///   (must not fold at all). See [`constructions::complete_coord`].
///
/// **Packing-safe by construction, and that is the point.** This is a property of the SPAN, so it is
/// identical for every item in a chart node. A rule decision that consults it is therefore still sound
/// to take on a node's *representative* — no [`super::chart::forest::Sig`] field is needed, unlike the
/// per-item `is_designation` bit, which varied within a node and had to be added there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightContext {
    /// A list-continuing comma — the cell is followed by `,`.
    Comma,
    /// An explicit coordinator (`and` / `or`) — the cell is a conjunct of a larger coordination.
    Coordinator,
    /// Anything else, including the end of the sentence: nothing about this cell's right edge licenses
    /// or forbids a construction. The value rules must treat as "no constraint", and the value
    /// standalone rule tests pass.
    Other,
}

impl RightContext {
    /// The context at the right edge of the cell ending at `j`, read from the sentence's tokens. Both
    /// chart drivers compute this once per cell; a cell at the end of the sentence is [`Self::Other`].
    pub(crate) fn after(
        reserved: &super::reserved::ReservedTable,
        tokens: &[String],
        j: usize,
    ) -> Self {
        match tokens.get(j + 1) {
            Some(t) if reserved.is_comma(t) => Self::Comma,
            Some(t) if reserved.coord_connective(t).is_some() => Self::Coordinator,
            _ => Self::Other,
        }
    }

    /// Whether a coordination ending at this cell is **final** — nothing follows that could extend it.
    /// A comma or an explicit coordinator means the list continues, so a neutral
    /// [`LIST_CONN`](constructions::LIST_CONN) group is a PREFIX and carries no connective to fold
    /// with; anything else (including end-of-sentence) makes it complete, and an asyndetic list then
    /// takes its documented conjunctive reading. See [`constructions::complete_coord`].
    pub(crate) fn list_is_final(self) -> bool {
        matches!(self, Self::Other)
    }
}
