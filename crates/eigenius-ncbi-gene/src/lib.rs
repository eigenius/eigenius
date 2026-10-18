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

//! NCBI Gene (Entrez) → Eigenius importer (D65 §5).
//!
//! A deterministic sibling of [`eigenius-wordnet`](../eigenius_wordnet/index.html):
//! it parses NCBI Gene's `gene_info` dump ([`gene_info`]) and renders ([`convert`])
//! a faithful typed **mirror** (`ncbi:Gene` witnesses anchored into the
//! WordNet–`lexicon:Entity` lattice) plus a **derived domain lexicon** (each gene's
//! symbol and synonyms → named-entity NP entries tagged `lexicon:ncbi_gene`). The
//! mirror is the source of truth; the lexicon is a view, so re-import regenerates it.

pub mod convert;
pub mod gene_info;
