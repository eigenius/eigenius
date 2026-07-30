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

//! **Reserved construct tokens** — the parser's grammatical function words that are NOT lexical
//! entries. Coordination, relativization, the contrastive `but not`, the reciprocal `each other`, and
//! the list/appositive comma are *category-polymorphic* rules `⟦·⟧` cannot denote (they range over
//! `Cat`), so they cannot be seeded as ordinary lexemes and are handled by reserved-word rules in the
//! CKY (both the packed and unpacked paths).
//!
//! As of §11 3g.3 the **packed** CKY mirrors every one of these constructs — coordination
//! (`Coordinate`), the reciprocal (`Reciprocal`), `but not` (`ButNot`), the restrictive relative
//! (`Relativize`), the appositive (`Appositive*`), and the fronted-modifier comma (`AbsorbComma`) —
//! plus the wh-determiner `which` as an ordinary leaf. The lone construct still routed to the unpacked
//! path is **pied-piping** (`[prep] which`), a ternary rule with no packing benefit, detected
//! structurally by [`super::parse::Parser::parse_needs_unpacked`] rather than by a token guard.
//!
//! The reserved-word FORM SET is **data** (§11 3g.3 / B10): `lexicon:ReservedConstruct { form,
//! construct_kind }` resources in `closed-class.esl`, loaded index-driven into a [`ReservedTable`] at
//! `Parser` build ([`crate::layer::typed_resource_iris`] over the `is_a` triple index — NOT a
//! full-chain resource scan, so it costs O(#reserved)). Adding a coordinator / relativizer is an
//! ontology edit, not a code change. The construct *semantics* (which rule fires) stays the parser
//! engine; [`ReservedKind`] is the internal role each `lexicon:construct_kind` individual maps to —
//! a **resource enum** constrained by `allows_only` (core's own `then_recommends` for a `resource`
//! property; a typo'd kind is rejected at commit, not silently dropped here) — and a coordinator's
//! connective IRI is *derived* from the kind (the ontology stores only the kind).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::layer::{typed_resource_iris, Layer};
use crate::ontology::resource::Value;
use crate::ontology::Iri;

/// The grammatical role a reserved construct token plays — the role the CKY keys its reserved-word
/// rules on. Each variant corresponds one-to-one to a `lexicon:ReservedKind` individual (the
/// `allows_only` vocabulary); [`ReservedKind::from_iri`] is the anchor. The parser owns the semantics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReservedKind {
    /// `and` — conjunction. (The list comma, [`ReservedKind::Comma`], also reads as conjunction.)
    CoordAnd,
    /// `or` — disjunction.
    CoordOr,
    /// The list / appositive / fronted-modifier comma (`,`); reads as conjunction for lists.
    Comma,
    /// `that` — restrictive relativizer (its complementizer use is an ordinary lexical leaf).
    Relativizer,
    /// `which` — wh-relativizer: the restrictive which-relative AND the pied-piping / wh marker (the
    /// wh-*determiner* `which` is a separate lexical leaf).
    WhRelativizer,
    /// `but` — the contrastive (first token of `but not`); plain `but` is the `but_subord` lexeme.
    ContrastiveBut,
    /// `not` — the second token of `but not` (verbal do-support negation elsewhere).
    Negator,
    /// `each` — first token of the reciprocal `each other`.
    ReciprocalEach,
    /// `other` — second token of the reciprocal `each other`.
    ReciprocalOther,
}

impl ReservedKind {
    /// The kind named by a `lexicon:ReservedKind` individual IRI — the closed `allows_only` vocabulary
    /// declared in `lexicon-ontology.esl`. This is the anchor: the Rust arms match the SAME IRIs the
    /// ontology enumerates (a typo is caught at commit by `allows_only`, not silently here). An
    /// unrecognised IRI is ignored (the entry is dropped).
    fn from_iri(iri: &str) -> Option<Self> {
        Some(match iri {
            "urn:eigenius:lexicon:rk_coord_and" => Self::CoordAnd,
            "urn:eigenius:lexicon:rk_coord_or" => Self::CoordOr,
            "urn:eigenius:lexicon:rk_comma" => Self::Comma,
            "urn:eigenius:lexicon:rk_relativizer" => Self::Relativizer,
            "urn:eigenius:lexicon:rk_wh_relativizer" => Self::WhRelativizer,
            "urn:eigenius:lexicon:rk_contrastive_but" => Self::ContrastiveBut,
            "urn:eigenius:lexicon:rk_negator" => Self::Negator,
            "urn:eigenius:lexicon:rk_reciprocal_each" => Self::ReciprocalEach,
            "urn:eigenius:lexicon:rk_reciprocal_other" => Self::ReciprocalOther,
            _ => return None,
        })
    }

    /// The kind named by a `construct_kind` value — a `ResourceRef` in memory, or the same IRI as a
    /// `String` after a persist round-trip (CBOR collapses `ResourceRef` → the content-hash string).
    fn from_value(v: &Value) -> Option<Self> {
        let iri = match v {
            Value::ResourceRef(i) => i.as_str(),
            Value::String(s) => s.as_str(),
            _ => return None,
        };
        Self::from_iri(iri)
    }
}

/// The reserved-construct table: `form → kind`, loaded index-driven from the ontology
/// (`lexicon:ReservedConstruct` resources) at `Parser` build. The single source of truth the
/// CKY's reserved-word rules (both paths) classify tokens against, replacing the former hard-coded
/// string consts.
#[derive(Clone, Default)]
pub(crate) struct ReservedTable {
    by_form: BTreeMap<String, ReservedKind>,
}

impl ReservedTable {
    /// Load every `lexicon:ReservedConstruct` in the layer chain, index-driven
    /// ([`typed_resource_iris`] over the `is_a` triple index — O(#reserved), never a full-chain
    /// resource scan). Entries missing `form` / `construct_kind`, or carrying an unknown kind, are
    /// skipped. Forms are lower-cased to match the parser's lookup key.
    pub fn load(layer: &Arc<Layer>) -> Self {
        let mut by_form = BTreeMap::new();
        let (Ok(form_prop), Ok(kind_prop)) = (
            Iri::parse("urn:eigenius:lexicon:construct_form"),
            Iri::parse("urn:eigenius:lexicon:construct_kind"),
        ) else {
            return ReservedTable { by_form };
        };
        for subj in typed_resource_iris(layer, &["urn:eigenius:lexicon:ReservedConstruct"]) {
            let Some(r) = layer.resolve(&subj) else {
                continue;
            };
            let Some(Value::String(form)) = r.get(&form_prop) else {
                continue;
            };
            let Some(k) = r.get(&kind_prop).and_then(ReservedKind::from_value) else {
                continue;
            };
            let key = form.trim().to_lowercase();
            if !key.is_empty() {
                by_form.insert(key, k);
            }
        }
        ReservedTable { by_form }
    }

    /// The kind of `token`, if it is a reserved construct.
    ///
    /// Folds case: `by_form` is lowercase-keyed by construction, and since [`tokenize`] stopped
    /// lowercasing (2026-07-29) a sentence-initial `That`/`And` arrives capitalised. A reserved
    /// construct is grammar, not vocabulary — its casing carries nothing.
    ///
    /// [`tokenize`]: super::segment::tokenize
    pub fn kind(&self, token: &str) -> Option<ReservedKind> {
        self.by_form
            .get(token)
            .or_else(|| self.by_form.get(token.to_lowercase().as_str()))
            .copied()
    }

    /// Whether `token` is the given reserved kind.
    pub fn is(&self, token: &str, kind: ReservedKind) -> bool {
        self.kind(token) == Some(kind)
    }

    /// A **relativizer** (`that` / `which`) — keys the restrictive-relative and appositive rules.
    pub fn is_relativizer(&self, token: &str) -> bool {
        matches!(
            self.kind(token),
            Some(ReservedKind::Relativizer | ReservedKind::WhRelativizer)
        )
    }

    /// The list / appositive / fronted-modifier comma (`,`).
    pub fn is_comma(&self, token: &str) -> bool {
        self.is(token, ReservedKind::Comma)
    }

    /// The connective a coordinator contributes: `and` → `logic:And`, `or` → `logic:Or`, and the list
    /// **comma** → the neutral [`LIST_CONN`](super::rules::constructions::LIST_CONN) that the trailing `and`/`or`
    /// finalizes (D63 §8.4 Phase 6, Step 5b — the comma inherits the list's final connective, so it is
    /// NOT hardcoded to `and`). `None` if `token` is not a coordinator. Derived from the kind.
    pub fn coord_connective(&self, token: &str) -> Option<&'static str> {
        match self.kind(token) {
            Some(ReservedKind::CoordAnd) => Some("urn:eigenius:logic:And"),
            Some(ReservedKind::CoordOr) => Some("urn:eigenius:logic:Or"),
            Some(ReservedKind::Comma) => Some(super::rules::constructions::LIST_CONN),
            _ => None,
        }
    }
}
