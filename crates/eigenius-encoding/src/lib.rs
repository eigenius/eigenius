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

//! **D62 S6 — assembly**: turn a parsed sentence's `Prop` into a chain resource.
//!
//! The DCG engine (D63) produces a closed, felicity-gated `Prop` per sentence; the reasoning
//! institution (D39) consumes chain-resident propositions carrying `IsDeclaredAs` witnesses. This
//! crate is the join: parse → select one reading → D47-encode the term → emit Eigon-JSON that
//! `eigenius load` puts on the chain. The claims land Declared, under a
//! `prov:DeclarationTrace` — see [`grade`] for why the parser fixes their form and not their
//! content.
//!
//! **Parsed claims land Declared** (D73 §6, superseding the Derived landing this crate was built
//! against). The parser is a formulation instrument: it establishes that the text parses to this
//! well-typed term, not that the term is faithful to what the author wrote, nor that what the
//! author wrote is true. The agent named in `prov:was_attributed_to` takes responsibility for the
//! proposition; the RUN is recorded once on the `enc:ReasoningStructure`'s `ProgramTrace`. An
//! *edit to the prose* is still visible to the commit gate — the witness key hashes the
//! proposition, so a certificate citing the claim stops resolving the moment the parser derives a
//! different `P`.
//!
//! **Grading and kind assignment live here** ([`grade`], [`claim_kind`], [`land`]). They were in
//! `eigenius-reasoning` until they moved: building a claim cluster from a parse is this pipeline's
//! own job, not the justification calculus's.
//!
//! **Reading selection is pin-driven and fails closed** ([`select`]). The page runs 60/62 ambiguous,
//! so "which reading" is not solved here — it is *declared*, against the human-verified skeletons in
//! `experiments/parsing/expected-readings.tsv`. Zero or several matches is an error with a
//! diagnostic, never a silent pick.

pub mod claim_kind;
pub mod emit;
pub mod formalize;
pub mod grade;
pub mod land;
pub mod pipeline;
pub mod select;
pub mod snapshot;

#[cfg(feature = "use-llm")]
pub use claim_kind::AnthropicKindClassifier;
pub use claim_kind::{
    frame_kind, KindClassifier, KindRecord, KindVerdict, NoKindClassifier, RecordingKindClassifier,
    ReplayKindClassifier,
};
pub use emit::{
    emit_document, CutReason, CutSentence, DocumentMeta, EmitError, ParsedSentence,
    SentenceSelection,
};
pub use grade::{
    ClaimGrader, ClaimSource, GradeError, GradedClaim, ParsedClaimGrader, UNATTRIBUTED_AGENT,
};
pub use land::DerivedClaimLander;
pub use select::{load_pins, Pin};
pub use snapshot::{build_parser, open_head, ParserConfig};
