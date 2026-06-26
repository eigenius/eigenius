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
//! committed storage) `LexicalIndex` over the read layer and returns the typed parse
//! forest. An optional per-parse scope — a set of `lexicon:Lexicon` IRIs, or a named
//! `lexicon:LexiconProfile` — restricts which lexica are in play (D65 §4); empty scope
//! parses against the whole chain unscoped.

use super::proto::*;
use super::EigeniusService;
use crate::dcg::{is_ctor, pretty_term, resolve_lexicon_profile, Identity, Item, LexicalIndex};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::observability::{operation, RpcGuard};
use crate::ontology::Iri;
use std::sync::Arc;
use tonic::{Response, Status};

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

        let index = LexicalIndex::build(Arc::clone(&layer));
        let forest = index.parse_scoped(&req.sentence, &Identity, scope.as_deref());

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
    let sem = match eval(&item.sem, &Rho::Nil) {
        Ok(v) => pretty_term(&readback_val(0, &v)),
        Err(_) => pretty_term(&item.sem),
    };
    Parse {
        category: pretty_term(&item.cat),
        sem,
        is_sentence: is_ctor(&item.cat, "cat_s").is_some(),
        lexicon_order: item.cost.lexicon_order,
        sense_rank: item.cost.sense_rank,
    }
}
