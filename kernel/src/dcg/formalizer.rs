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

//! **The formalization seam** (D71 §7.1) — the kernel declares it, the top-level binary fills it in.
//!
//! Formalizing a document ends in an ARTIFACT: the resource set an `enc:ReasoningStructure` roots.
//! Building that set needs `emit_document` and `ParsedClaimGrader`, which live in
//! `eigenius-encoding` and `eigenius-reasoning` — both of which DEPEND ON this crate. The kernel's
//! gRPC layer therefore cannot call them, and inverting the dependency is not on the table: the
//! grader belongs above the kernel, not inside it.
//!
//! This is the same seam, for the same reason, as [`crate::dcg::ParseConfig::lemmatizer`]: *"the
//! kernel cannot depend on `eigenius-wordnet` (cycle), so a real `MorphyLemmatizer` is injected by
//! the top-level binary."* The kernel holds a trait object; the binary that links everything
//! constructs the impl and hands it over at startup.

use std::sync::Arc;

use crate::dcg::draw::DrawSeam;
use crate::dcg::model_config::ModelConfig;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::storage::PersistentBackend;
use std::collections::BTreeMap;

/// Where a run's proposer draws come from.
///
/// A draw is keyed on the presented pool, so its source decides whether a run reproduces or
/// re-asks. Both arms REPLAY; neither calls a model.
#[derive(Clone, Debug, Default)]
pub enum DrawSource {
    /// Read them off the run's `doc-<id>` branch (D71 §9.1) — the served default, and the
    /// arrangement in which the draws and the glossary that produced their pool are committed by
    /// the same run and cannot drift apart.
    #[default]
    Branch,
    /// Supplied verbatim with the request, one JSON array per seam. The shape the deterministic
    /// harnesses use, where the draws travel beside the source rather than on a branch.
    Inline(BTreeMap<DrawSeam, String>),
    /// No recordings: every seam asks live (needs a key and `use-llm`), and the run records what it
    /// asked so the next one need not.
    Live,
}

/// How the artifact comes back.
///
/// Mirrors [`crate::server::proto::LoadRequest`]'s `bytes` + `content_type` pair rather than
/// pinning one encoding, because the artifact is BOTH a machine payload and something a person
/// reads: D71 §4 makes generation/commitment decoupling the point — the artifact is inspected and
/// diffed, then `Load`ed — and `Load` itself accepts either encoding.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ArtifactFormat {
    /// Canonical Eigon-CBOR. The default: it is what `Load` prefers and what nothing has to
    /// re-parse.
    #[default]
    Cbor,
    /// Eigon-JSON — the form the committed test fixtures and the demo diff against.
    EigonJson,
    /// ESL source. The most readable, and the only form that can FAIL to render: a document the
    /// printer cannot express is an error, never a silent fallback to another encoding — writing a
    /// different format under an `.esl` name would be worse than failing.
    Esl,
}

impl ArtifactFormat {
    /// The `content_type` string, matching `LoadRequest`'s vocabulary.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Cbor => "application/cbor",
            Self::EigonJson => "application/eigon+json",
            Self::Esl => "text/x-esl",
        }
    }
}

/// Render a resource set in the requested format.
///
/// Lives here, beside the format enum, because all three serializers are kernel-side and a caller
/// that re-derived the mapping would be the second place format policy lives.
pub fn render_artifact(
    resources: &[crate::ontology::resource::Resource],
    format: ArtifactFormat,
) -> Result<Vec<u8>, String> {
    match format {
        ArtifactFormat::Cbor => Ok(crate::ontology::eigon_cbor::serialize_document(resources)),
        ArtifactFormat::EigonJson => Ok(json_bytes(resources)),
        ArtifactFormat::Esl => {
            let doc: serde_json::Value = serde_json::from_slice(&json_bytes(resources))
                .map_err(|e| format!("emitted document is not valid JSON: {e}"))?;
            crate::esl::print::print_document_with(&doc, crate::esl::print::Layout::Pretty)
                .map(String::into_bytes)
                .map_err(|e| format!("cannot render the artifact as ESL: {e}"))
        }
    }
}

/// Pretty Eigon-JSON. Pretty because this artifact exists to be read: a parsed proposition is a
/// deep application spine, and on one line its structure is invisible.
fn json_bytes(resources: &[crate::ontology::resource::Resource]) -> Vec<u8> {
    serde_json::to_string_pretty(&crate::ontology::eigon_json::serialize_document(resources))
        .expect("serialize Eigon-JSON")
        .into_bytes()
}

/// One formalization run, as values — no paths, no clap, no snapshot.
pub struct FormalizeRequest {
    /// The prose to formalize.
    pub source_text: String,
    /// Where the bytes came from, recorded on the artifact root. Caller-supplied text: the sha256
    /// is what actually pins them, so this only has to be meaningful to a human.
    pub source_path: String,
    /// An existing `reference:Reference` to cite. `None` mints a document-local one into the
    /// artifact; `Some` must resolve on the chain the artifact loads onto (Rule 22 verifies).
    pub source_ref: Option<String>,
    /// Names the run's `doc-<id>` working branch — glossary layer and proposer draws.
    pub doc_id: String,
    /// IRI prefix for the emitted resources.
    pub ns: String,
    /// The `prov:timestamp` on each ProgramTrace. Caller-fixed so emission is reproducible.
    pub timestamp: String,
    /// D65 §4 parse scope — ordered `lexicon:Lexicon` IRIs. `None` is the whole chain.
    pub scope: Option<Vec<Iri>>,
    /// Which model the run's untrusted proposers call, and what its draws record as the answerer.
    pub model: ModelConfig,
    /// Per-run scale controls. `None` takes the server's configured value.
    pub sense_cap: Option<usize>,
    pub cell_beam: Option<usize>,
    /// Abort on the first unit that does not encode, instead of recording it as an `enc:CutItem`.
    /// The service default is NOT strict: an interactive surface wants the failing units named,
    /// not an aborted run (D71 §8).
    pub strict: bool,
    pub draws: DrawSource,
    /// How the artifact should come back.
    pub format: ArtifactFormat,
}

/// What a run produced.
pub struct FormalizeOutput {
    /// The artifact, in [`FormalizeRequest::format`].
    pub artifact: Vec<u8>,
    /// What `artifact` is encoded as — echoed so a caller need not track the request to read it.
    pub content_type: ArtifactFormat,
    /// The `enc:ReasoningStructure` that roots it — the handle a caller re-opens or supersedes.
    pub structure_iri: String,
    pub encoded: usize,
    pub cut: usize,
    /// Draws committed to the working branch by this run. Zero when every seam replayed and
    /// re-recorded identically (the content-addressed IRIs collide, the branch does not advance).
    pub draws_committed: usize,
}

/// Turn prose into an artifact. Implemented above the kernel; see the module docs.
///
/// Blocking: the pipeline parses, and a document takes minutes. The server runs it off the async
/// executor, which is also why a formalization is a task rather than a synchronous RPC (D71 §6).
pub trait DocumentFormalizer: Send + Sync {
    fn formalize(
        &self,
        base: Arc<Layer>,
        backend: Arc<dyn PersistentBackend>,
        req: &FormalizeRequest,
    ) -> Result<FormalizeOutput, String>;
}
