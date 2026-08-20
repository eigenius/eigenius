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

//! `FormalizeDocument` — prose to artifact, as a task (D71 §6/§7.1).
//!
//! **Asynchronous, and not by preference.** A document costs minutes and N LLM round-trips; the MCP
//! surface has no long-call idiom, the notebook wants progress and cancel, and a CI driver wants to
//! poll. So the RPC starts a `TaskKind::Formalize` and returns its id; the artifact is fetched
//! afterwards with `GetFormalizationResult`.
//!
//! **The work runs off the async executor.** `DocumentFormalizer::formalize` parses, which is
//! CPU-bound for minutes. Running it on a tokio worker would stall every other request on that
//! thread, so it goes to `spawn_blocking`.
//!
//! **The artifact is a meta blob, not a layer.** Committing it would defeat the point: D71 §4 keeps
//! generation decoupled from commitment so a person can read and diff the artifact before `Load`ing
//! it, and re-serialising a committed layer back to ESL is not byte-identical to what was emitted.

use std::sync::Arc;

use tonic::{Response, Status};

use crate::dcg::formalizer::{ArtifactFormat, DrawSource, FormalizeRequest};
use crate::dcg::model_config::ModelConfig;
use crate::ontology::iri::Iri;
use crate::server::proto::{
    FormalizeDocumentRequest, FormalizeDocumentResponse, GetFormalizationResultRequest,
    GetFormalizationResultResponse,
};
use crate::server::EigeniusService;
use crate::task::{TaskRecord, TaskStatus};

/// Meta key holding a completed run's artifact. Namespaced away from the task record's own key so
/// a task listing never walks artifact-sized values.
fn result_key(task_id: &uuid::Uuid) -> String {
    format!("formalize:result:{task_id}")
}

/// The stored outcome of a run. CBOR like every other meta blob (`TaskRecord`, `Checkpoint`).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredResult {
    artifact: Vec<u8>,
    content_type: String,
    structure_iri: String,
    encoded: u32,
    cut: u32,
    draws_committed: u32,
    error: String,
}

fn parse_format(s: &str) -> Result<ArtifactFormat, Status> {
    Ok(match s {
        "" | "application/cbor" => ArtifactFormat::Cbor,
        "application/eigon+json" => ArtifactFormat::EigonJson,
        "text/x-esl" => ArtifactFormat::Esl,
        other => {
            return Err(Status::invalid_argument(format!(
                "unknown artifact format {other:?} — expected application/cbor, \
                 application/eigon+json, or text/x-esl"
            )))
        }
    })
}

impl EigeniusService {
    pub(super) async fn handle_formalize_document(
        &self,
        req: FormalizeDocumentRequest,
    ) -> Result<Response<FormalizeDocumentResponse>, Status> {
        let Some(formalizer) = self.formalizer.clone() else {
            return Err(Status::unimplemented(
                "no document formalizer is installed — the kernel declares the seam and the \
                 top-level binary injects the implementation (D71 §7.1)",
            ));
        };
        let Some(store) = self.task_store.clone() else {
            return Err(Status::failed_precondition(
                "formalization is a task and this server has no task store",
            ));
        };
        let Some(backend) = self.backend.clone() else {
            return Err(Status::failed_precondition(
                "formalization writes a doc-<id> working branch and needs a persistent backend",
            ));
        };

        if req.source_text.trim().is_empty() {
            return Err(Status::invalid_argument("source_text is required"));
        }
        if req.doc_id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "doc_id is required — it names the run's doc-<id> working branch",
            ));
        }
        if !req.scope.is_empty() && !req.profile.is_empty() {
            return Err(Status::invalid_argument(
                "scope and profile are mutually exclusive",
            ));
        }

        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;

        // Scope resolves HERE, against the read layer, so a profile IRI and an explicit list take
        // the same path — the formalizer only ever sees the resolved order.
        let scope: Option<Vec<Iri>> = if !req.scope.is_empty() {
            let mut v = Vec::with_capacity(req.scope.len());
            for s in &req.scope {
                v.push(
                    Iri::parse(s)
                        .map_err(|e| Status::invalid_argument(format!("invalid scope IRI: {e}")))?,
                );
            }
            Some(v)
        } else if !req.profile.is_empty() {
            let p = Iri::parse(&req.profile)
                .map_err(|e| Status::invalid_argument(format!("invalid profile IRI: {e}")))?;
            Some(
                crate::dcg::resolve_lexicon_profile(&layer, &p).ok_or_else(|| {
                    Status::invalid_argument(format!(
                        "lexicon profile {} not found in the served chain",
                        req.profile
                    ))
                })?,
            )
        } else {
            None
        };

        let opts = req.options.unwrap_or_default();
        let cfg = &self.parse_config;
        let model = ModelConfig {
            model: if opts.model.is_empty() {
                crate::dcg::model_config::DEFAULT_MODEL.to_string()
            } else {
                opts.model.clone()
            },
            max_tokens: if opts.max_tokens == 0 {
                ModelConfig::default().max_tokens
            } else {
                opts.max_tokens
            },
        };

        let draws = match (&req.inline_draws, req.live_draws) {
            (Some(d), _) => {
                use crate::dcg::draw::DrawSeam;
                let mut m = std::collections::BTreeMap::new();
                for (seam, text) in [
                    (DrawSeam::SenseRank, &d.sense_rank),
                    (DrawSeam::ReadingSelection, &d.reading_selection),
                    (DrawSeam::Anaphora, &d.anaphora),
                    (DrawSeam::DiscourseKind, &d.discourse_kind),
                ] {
                    if !text.is_empty() {
                        m.insert(seam, text.clone());
                    }
                }
                DrawSource::Inline(m)
            }
            (None, true) => DrawSource::Live,
            (None, false) => DrawSource::Branch,
        };

        let fr = FormalizeRequest {
            source_text: req.source_text,
            source_path: req.source_path,
            source_ref: (!req.source_ref.is_empty()).then_some(req.source_ref),
            doc_id: req.doc_id.clone(),
            ns: if req.ns.is_empty() {
                format!("urn:eigenius:doc:{}", req.doc_id)
            } else {
                req.ns
            },
            timestamp: if req.timestamp.is_empty() {
                default_timestamp()
            } else {
                req.timestamp
            },
            scope,
            model,
            sense_cap: (opts.sense_cap > 0)
                .then_some(opts.sense_cap as usize)
                .or(cfg.sense_cap),
            cell_beam: (opts.cell_beam > 0)
                .then_some(opts.cell_beam as usize)
                .or(cfg.cell_beam),
            strict: opts.strict,
            draws,
            format: parse_format(&req.format)?,
        };

        let task_id = uuid::Uuid::new_v4();
        let session_id = uuid::Uuid::nil();
        let source_sha = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(fr.source_text.as_bytes()))
        };
        let record = TaskRecord::new_formalize(
            session_id,
            task_id,
            fr.doc_id.clone(),
            source_sha,
            layer.id().clone(),
            crate::server::helpers::now_millis(),
        );
        store
            .put_task(&record)
            .map_err(|e| Status::internal(format!("persist task: {e}")))?;

        let doc_branch = format!("doc-{}", fr.doc_id);
        let base = Arc::clone(&layer);
        let result_backend = Arc::clone(&backend);
        tokio::task::spawn_blocking(move || {
            let outcome = formalizer.formalize(base, backend, &fr);
            let stored = match &outcome {
                Ok(o) => StoredResult {
                    artifact: o.artifact.clone(),
                    content_type: o.content_type.content_type().to_string(),
                    structure_iri: o.structure_iri.clone(),
                    encoded: o.encoded as u32,
                    cut: o.cut as u32,
                    draws_committed: o.draws_committed as u32,
                    error: String::new(),
                },
                Err(e) => StoredResult {
                    artifact: Vec::new(),
                    content_type: String::new(),
                    structure_iri: String::new(),
                    encoded: 0,
                    cut: 0,
                    draws_committed: 0,
                    error: e.clone(),
                },
            };
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&stored, &mut buf);
            let _ = result_backend.put_meta(&result_key(&task_id), &buf);

            let mut rec = record;
            rec.status = if outcome.is_ok() {
                TaskStatus::Completed
            } else {
                TaskStatus::Failed
            };
            rec.updated_at = crate::server::helpers::now_millis();
            let _ = store.put_task(&rec);
        });

        Ok(Response::new(FormalizeDocumentResponse {
            task_id: task_id.to_string(),
            doc_branch,
        }))
    }

    pub(super) async fn handle_get_formalization_result(
        &self,
        req: GetFormalizationResultRequest,
    ) -> Result<Response<GetFormalizationResultResponse>, Status> {
        let task_id = uuid::Uuid::parse_str(&req.task_id)
            .map_err(|e| Status::invalid_argument(format!("invalid task_id: {e}")))?;
        let Some(backend) = self.backend.clone() else {
            return Err(Status::failed_precondition("no persistent backend"));
        };
        let Some(bytes) = backend
            .get_meta(&result_key(&task_id))
            .map_err(|e| Status::internal(format!("read result: {e}")))?
        else {
            // Not an error: the task may still be running. The caller polls GetTaskStatus.
            return Ok(Response::new(GetFormalizationResultResponse {
                found: false,
                ..Default::default()
            }));
        };
        let stored: StoredResult = ciborium::from_reader(&bytes[..])
            .map_err(|e| Status::internal(format!("decode result: {e}")))?;
        Ok(Response::new(GetFormalizationResultResponse {
            found: true,
            artifact: stored.artifact,
            content_type: stored.content_type,
            structure_iri: stored.structure_iri,
            encoded: stored.encoded,
            cut: stored.cut,
            draws_committed: stored.draws_committed,
            error: stored.error,
        }))
    }
}

/// Milliseconds since the epoch, rendered RFC-3339 — the default `reflection:timestamp` when a
/// caller does not fix one. A caller that wants a byte-reproducible artifact supplies its own.
fn default_timestamp() -> String {
    let ms = crate::server::helpers::now_millis();
    let secs = ms / 1000;
    // No chrono in the kernel; this is the same shape the emitters use.
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (y, m, d) — Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
