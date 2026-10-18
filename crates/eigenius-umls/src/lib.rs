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

//! UMLS Metathesaurus → Eigenius importer (D65 §5).
//!
//! A deterministic sibling of [`eigenius-wordnet`](../eigenius_wordnet/index.html)
//! and [`eigenius-ncbi-gene`](../eigenius_ncbi_gene/index.html): it parses the UMLS
//! Rich Release Format files ([`rrf`]) and renders ([`convert`]) a faithful typed
//! **mirror** plus a **derived domain lexicon** (D65 §5, mirror-then-derive).
//!
//! Unlike NCBI Gene (whose entries are named *individuals* → NP witnesses), a UMLS
//! concept (CUI) is predominantly a **type/kind** ("Werner syndrome", "microsatellite
//! instability"), so the model is WordNet's **common-noun path**:
//!
//! 1. **Mirror.** Each UMLS *Semantic Type* (TUI) → a `umls:SemanticType`-shaped
//!    **class** `⊑ lexicon:Entity`; each *concept* (CUI) → a **class** whose
//!    `subclass_of` edges ARE its semantic typing (`umlscui:C0043119 ⊑ umlssty:T047`,
//!    structurally queryable), reaching `lexicon:Entity` transitively. The CUI is the
//!    IRI local (`umlscui:C…`); the definition is the class `description`. This exactly
//!    parallels WordNet (synset offset = IRI local, hypernym = `subclass_of`, gloss =
//!    description) — no parallel `umls:cui`/`umls:tui` properties are minted.
//! 2. **Lexicon (derived).** One `lexicon:Lexicon` (`lexicon:umls`) and, per concept,
//!    a common-noun **N** `lexicon:LexicalEntry` for each English surface string —
//!    `cat_n(umlscui:C…, num_any)`, `sem =` the concept class, `sem_type = Set`,
//!    `in_lexicon = lexicon:umls` (the WordNet common-noun archetype).
//!
//! **License (load-bearing).** UMLS is licensed, not public-domain. The emitted
//! artifact carries the UMLS Metathesaurus License notice and the redistribution
//! constraint flows downstream (every recipient must obtain their own UMLS license),
//! and the importer honors UMLS Source Restriction Levels: only **SRL-0** (Level 0)
//! sources are emitted (`MRSAB.SRL`), so SNOMED CT / CPT and other restricted sources
//! are excluded even when present in the input.

pub mod convert;
pub mod rrf;
