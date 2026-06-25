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

//! WordNet → Eigon lexicon importer (D62 §8.7) — the *general* English framework.
//!
//! Deterministic structural transform, **no LLM**: noun synsets → `core:Class`es
//! (with `@` hypernymy → `core:subclass_of`), `@i` instance synsets →
//! `EigonResource` individuals, verb/adjective synsets → `eigentt:Axiom`
//! predicates, lemmas → `lexicon:LexicalEntry`. Output is ESL text; the kernel
//! validates it. [`morphy`] ports WordNet's Morphy, exposed via [`lemmatizer`] as
//! the kernel `dcg::Lemmatizer` that drives the lookup bridge's surface→lemma
//! stage (D62 §8.8.1). An LLM proposer is the intended *augmentation* tool that
//! layers domain vocabulary on top of this framework (§8.7.8).

pub mod convert;
pub mod import;
pub mod inflect;
pub mod lemmatizer;
pub mod morphy;
pub mod wndb;
