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

//! `ParseSentence` — run the D63/D65 DCG sentence parser over the served chain.
//!
//! Builds the (lazy, when a `lexicon:form` `core:ValueIndex` is active over the
//! committed storage) `Parser` over the read layer and returns the typed parse
//! forest. An optional per-parse scope — a set of `lexicon:Lexicon` IRIs, or a named
//! `lexicon:LexiconProfile` — restricts which lexica are in play (D65 §4); empty scope
//! parses against the whole chain unscoped.

use super::proto::*;
use super::EigeniusService;
use crate::dcg::{
    is_ctor, pretty_term, resolve_lexicon_profile, Identity, Item, Lemmatizer, Parser,
};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::observability::{operation, RpcGuard};
use crate::ontology::Iri;
use std::sync::Arc;
use tonic::{Response, Status};

/// Configuration for the `ParseSentence` parse path (D63/GH#97 Lever 1). Held by
/// [`EigeniusService`]; a fresh `Parser` is built per request with these settings.
///
/// Defaults are the safe production shape: the **sense cap + cell beam ON** (the only OOM defense
/// over the full WordNet+UMLS lexicon — without them a nontrivial sentence over the dense lexicon
/// blows the chart), the **`Identity` (no-op) lemmatizer** (preserving prior behaviour until a binary
/// injects a real one — see [`Self::lemmatizer`]), and the **contextual LLM reranker OFF** (so the
/// server stays deterministic by default; opt in where `--features use-llm` + `ANTHROPIC_API_KEY`).
pub struct ParseConfig {
    /// Surface→lemma reducer. Defaults to [`Identity`] (no reduction). The kernel cannot depend on
    /// `eigenius-wordnet` (cycle), so a real `MorphyLemmatizer` is injected by the top-level binary
    /// via [`EigeniusService::with_parse_config`] — the kernel holds only the trait object.
    pub lemmatizer: Arc<dyn Lemmatizer + Send + Sync>,
    /// Adaptive-supertagging per-lemma sense cap (Lever A). `None` = uncapped.
    pub sense_cap: Option<usize>,
    /// Per-cell beam (Lever B). `None` = unbounded chart.
    pub cell_beam: Option<usize>,
    /// Enable the contextual LLM sense reranker when built with `--features use-llm` and
    /// `ANTHROPIC_API_KEY` is set (one reranker call per sentence). No effect otherwise.
    pub use_ranker: bool,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Self {
            lemmatizer: Arc::new(Identity),
            sense_cap: Some(2),
            cell_beam: Some(64),
            use_ranker: false,
        }
    }
}

impl EigeniusService {
    pub(super) async fn handle_parse_sentence(
        &self,
        req: ParseSentenceRequest,
    ) -> Result<Response<ParseSentenceResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_PARSE_SENTENCE);

        if !req.scope.is_empty() && !req.profile.is_empty() {
            return Err(Status::invalid_argument(
                "scope and profile are mutually exclusive",
            ));
        }

        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;

        // Resolve the scope: an explicit ordered IRI list, a named profile, or none.
        let scope: Option<Vec<Iri>> = if !req.scope.is_empty() {
            let mut iris = Vec::with_capacity(req.scope.len());
            for s in &req.scope {
                iris.push(
                    Iri::parse(s)
                        .map_err(|e| Status::invalid_argument(format!("invalid scope IRI: {e}")))?,
                );
            }
            Some(iris)
        } else if !req.profile.is_empty() {
            let profile = Iri::parse(&req.profile)
                .map_err(|e| Status::invalid_argument(format!("invalid profile IRI: {e}")))?;
            Some(resolve_lexicon_profile(&layer, &profile).ok_or_else(|| {
                Status::invalid_argument(format!(
                    "lexicon profile {} not found in the served chain",
                    req.profile
                ))
            })?)
        } else {
            None
        };

        // Build the index with the configured scale controls (Lever A cap + Lever B beam) — the
        // serving path's only defense against the full-lexicon chart blow-up. The contextual LLM
        // reranker is opt-in and `use-llm`-gated; widen-on-failure (in `parse_scoped`) recovers any
        // sense a bad rank or the cap drops, so neither can lose a parse a known sentence would get.
        let cfg = &self.parse_config;
        let mut index = Parser::build(Arc::clone(&layer));
        if let Some(n) = cfg.sense_cap {
            index = index.with_sense_cap(n);
        }
        if let Some(m) = cfg.cell_beam {
            index = index.with_cell_beam(m);
        }
        #[cfg(feature = "use-llm")]
        if cfg.use_ranker {
            if let Some(ranker) = crate::dcg::AnthropicSenseRanker::from_env() {
                index = index.with_sense_ranker(Box::new(ranker));
            }
        }
        let forest = index.parse_scoped(&req.sentence, &*cfg.lemmatizer, scope.as_deref());

        let parses = forest.iter().map(parse_to_proto).collect();
        Ok(Response::new(ParseSentenceResponse { parses }))
    }
}

/// Project a parse [`Item`] into the wire shape: the category and the (β/η-normalized)
/// semantics pretty-printed, plus whether it is a complete sentence and its rank key.
fn parse_to_proto(item: &Item) -> Parse {
    // Read the sem back at level 0 so the wire form is the normalized term. On eval
    // failure (an open/partial fragment), fall back to the raw term so we still return
    // a useful rendering rather than dropping the parse.
    let sem = match eval(item.sem(), &Rho::Nil) {
        Ok(v) => pretty_term(&readback_val(0, &v)),
        Err(_) => pretty_term(item.sem()),
    };
    Parse {
        category: pretty_term(item.cat()),
        sem,
        is_sentence: is_ctor(item.cat(), "cat_s").is_some(),
        lexicon_order: item.cost().lexicon_order,
        sense_rank: item.cost().sense_rank,
    }
}
