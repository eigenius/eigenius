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

//! **Reserved construct tokens** — the single source of truth for the parser's grammatical function
//! words that are NOT lexical entries. Coordination, relativization, the contrastive `but not`, the
//! reciprocal `each other`, and the list/appositive comma are all *category-polymorphic* rules that
//! `⟦·⟧` cannot denote (they range over `Cat`), so they cannot be seeded as ordinary lexemes and are
//! instead handled by reserved-word rules in the CKY. Previously each rule hard-coded its trigger
//! string; this module centralises them so the relative rule, [`super::lookup::coord_connective`],
//! and the CKY special-construct rules (both packed and unpacked) classify tokens in one place (D63
//! blueprint §11, the 3g.3 refactor).
//!
//! As of §11 3g.3 the **packed** CKY mirrors every one of these constructs — coordination
//! (`Coordinate`), the reciprocal (`Reciprocal`), `but not` (`ButNot`), the restrictive relative
//! (`Relativize`), the appositive (`Appositive*`), and the fronted-modifier comma (`AbsorbComma`) —
//! plus the wh-determiner `which` as an ordinary leaf. The lone construct still routed to the unpacked
//! path is **pied-piping** (`[prep] which`), a ternary rule with no packing benefit, detected
//! structurally by [`super::lookup::LexicalIndex::parse_needs_unpacked`] rather than by a token guard.
//!
//! FOLLOW-UP (reseed-gated): back this table with an ontology declaration (e.g. a
//! `lexicon:ReservedConstruct { form, construct_kind }` in `closed-class.esl`, loaded into a map at
//! index build) so the reserved-token set is *data*, not code — matching the rest of the platform.
//! Deferred because `closed-class.esl` is bootstrap: changing it forces a reseed + DB-snapshot
//! re-alignment, so it should ride the next reseed rather than force one.

/// Coordinator `and` (and the list comma, which reads as conjunction).
pub(crate) const AND: &str = "and";
/// Coordinator `or`.
pub(crate) const OR: &str = "or";
/// The list / appositive / fronted-modifier comma.
pub(crate) const COMMA: &str = ",";
/// Restrictive-relative / complementizer `that`.
pub(crate) const THAT: &str = "that";
/// Relativizer / pied-piping / wh `which`.
pub(crate) const WHICH: &str = "which";
/// Contrastive `but` (the `but not` construction; also the sentential subordinator).
pub(crate) const BUT: &str = "but";
/// Negation `not` (verbal do-support negation; the second token of `but not`).
pub(crate) const NOT: &str = "not";
/// Reciprocal `each` (first token of `each other`).
pub(crate) const EACH: &str = "each";
/// Reciprocal `other` (second token of `each other`).
pub(crate) const OTHER: &str = "other";

/// A **relativizer** (`that` / `which`) — keys the restrictive-relative, appositive, and
/// pied-piping rules.
pub(crate) fn is_relativizer(t: &str) -> bool {
    t == THAT || t == WHICH
}
