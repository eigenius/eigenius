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

//! **The alignment layer** — redefine each merged UMLS lexical entry so it denotes the WordNet
//! class instead of the UMLS one.
//!
//! The entries are read **from the chain**, not reconstructed from the importer: the committed
//! resource is the truth, and rebuilding it from scratch would silently drift (the additive mass
//! variants, `sense_rank`, whatever a future importer adds). Every property is passed through
//! **unchanged** except the two that carry the concept's identity:
//!
//! ```text
//!   cat  : cat_n(umlscui:C1442792, num_any)  →  cat_n(wn:n00024720, num_any)
//!   sem  : umlscui:C1442792                  →  wn:n00024720
//! ```
//!
//! **`sense` is deliberately NOT rewritten.** The seed-time dedup (`dedup_same_concept`) keys on
//! `(cat, sem)` only, so the redefined entry collapses against WordNet's own entry regardless of the
//! sense label. Rewriting it would be one more thing to get wrong for no gain.
//!
//! **No `subclass_of` edges are emitted, and no class is touched.** The alignment changes *which
//! class an entry denotes*; it does not restructure the type lattice. (2026-07-11: adding lattice
//! edges — a supersense parent on every WordNet noun, the UMLS TUI ISA tree — broke the parses and
//! the branch was reverted. The lattice stays exactly as it is.)
//!
//! **Named individuals are excluded.** A UMLS concept that is a proper name emits
//! `cat_np(umlssty:<TUI>, sg)` — an *instance*, not a class, and it does not even mention the CUI in
//! its category. Pointing an instance at a WordNet class is a type error, so any entry whose `cat`
//! is not exactly `cat_n(umlscui:<CUI>, N)` is skipped.

use std::collections::BTreeMap;

/// One entry rewrite: the entry's IRI, and the class it should now denote.
#[derive(Debug, Clone)]
pub struct Rewrite {
    pub entry_iri: String,
    /// The `num` argument of the original `cat_n(C, num)` — `num_any` or `mass`. **Preserved**: the
    /// additive mass variant must stay a mass variant.
    pub num: String,
    /// The canonical class, as an ESL name the layer's header declares a namespace for:
    /// `wn:n00024720` (WordNet↔UMLS), `hp:'0001250'` (HPO↔UMLS, [`crate::hpo::hp_qname`]).
    pub class: String,
    /// Everything else, passed through verbatim.
    pub form: String,
    pub sense: String,
    pub in_lexicon: String,
    pub sem_type: String,
}

/// The ESL header: the namespaces the redefinitions reference. All resolve to layers **below** this
/// one (Rule 22: references must resolve same-or-lower), which is why the alignment must be a layer
/// above both lexica and cannot be an importer-side lookup table.
pub const HEADER: &str = "\
// ════════════════════════════════════════════════════════════════════
// WordNet↔UMLS concept unification (D63) — the ALIGNMENT LAYER.
//
// Each resource below REDEFINES a UMLS lexical entry that the adjudicator judged to name the same
// concept as a WordNet synset. Only `cat` and `sem` change: the entry now denotes the WordNet class.
// Every other property is passed through from the committed entry unchanged.
//
// No class is created or modified; no `subclass_of` edge is emitted. The type lattice is untouched.
// ════════════════════════════════════════════════════════════════════
namespace core       = \"urn:eigenius:core\";
namespace reflection = \"urn:eigenius:reflection\";
namespace epistemic  = \"urn:eigenius:reflection:epistemic\";
namespace eigentt    = \"urn:eigenius:eigentt\";
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace umlscui    = \"urn:eigenius:umlscui\";
namespace wn         = \"urn:eigenius:wn\";
";

/// The header of the HPO↔UMLS layer ([`crate::hpo`]). Same mechanism, same guarantees: only `cat`
/// and `sem` change, no class is created or modified, no `subclass_of` edge is emitted.
pub const HPO_HEADER: &str = "\
// ════════════════════════════════════════════════════════════════════
// HPO↔UMLS concept unification — the ALIGNMENT LAYER.
//
// Each resource below REDEFINES a UMLS lexical entry whose concept NLM maps (MRCONSO, SAB=HPO) to
// exactly one live HPO class, and whose surface is one of that class's own HPO names. Only `cat` and
// `sem` change: the entry now denotes the HPO class.
// Every other property is passed through from the committed entry unchanged. Concepts the
// WordNet↔UMLS layer already took to a synset are left to WordNet.
//
// No class is created or modified; no `subclass_of` edge is emitted.
// ════════════════════════════════════════════════════════════════════
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace umlscui    = \"urn:eigenius:umlscui\";
namespace hp         = \"urn:obo:HP\";
";

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Render one redefinition.
pub fn render(r: &Rewrite) -> String {
    let local = r
        .entry_iri
        .rsplit_once(':')
        .map(|(_, l)| l)
        .unwrap_or(&r.entry_iri);
    format!(
        "resource umlscui:{local} : lexicon:LexicalEntry {{\n\
         \x20   lexicon:form       = \"{form}\";\n\
         \x20   lexicon:cat        = type_expr( lexicon:cat_n({class}, lexicon:{num}) );\n\
         \x20   lexicon:sem        = {class};\n\
         \x20   lexicon:sem_type   = type_expr( {sem_type} );\n\
         \x20   lexicon:sense      = \"{sense}\";\n\
         \x20   lexicon:in_lexicon = {in_lexicon};\n\
         }}\n\n",
        form = esc(&r.form),
        class = r.class,
        num = r.num,
        sem_type = r.sem_type,
        sense = esc(&r.sense),
        in_lexicon = r.in_lexicon,
    )
}

/// The merge table: `(cui, lowercased surface) → WordNet offset`.
pub type Merges = BTreeMap<(String, String), String>;

/// Load `merges.json` (the adjudicated, conflict-resolved alignment).
/// The class-equivalence layer (D99 §11): one `core:EquivalentClasses` per UMLS concept that the
/// WordNet↔UMLS alignment took to a synset and exactly one live HPO code names.
pub const HPO_EQUIVALENCE_HEADER: &str = "\
// ════════════════════════════════════════════════════════════════════
// HPO ≡ WordNet ≡ UMLS — the CLASS-EQUIVALENCE LAYER (D99 §11).
//
// Each resource below declares an HPO class, a WordNet synset and a UMLS concept equivalent: NLM maps
// the HPO code to the concept (MRCONSO, SAB=HPO), and the WordNet↔UMLS adjudication maps the concept
// to the synset (merges.json) — a chain of two published mappings, semapv:MappingChaining.
//
// No class is created or modified. Subsumption reads the equivalence; class definitions do not.
// ════════════════════════════════════════════════════════════════════
namespace core       = \"urn:eigenius:core\";
namespace prov       = \"urn:eigenius:prov\";
namespace agent      = \"urn:eigenius:prov:agent\";
namespace umlscui    = \"urn:eigenius:umlscui\";
namespace wn         = \"urn:eigenius:wn\";
namespace hp         = \"urn:obo:HP\";
namespace hpoequiv   = \"urn:eigenius:align:hpo-wordnet\";
";

/// One equivalence of [`HPO_EQUIVALENCE_HEADER`]'s layer.
pub fn render_equivalence(cui: &str, hp_code: &str, hp_class: &str, wn_offset: &str) -> String {
    format!(
        "resource hpoequiv:{cui} : core:EquivalentClasses {{\n\
         \x20   core:description = \"{hp_code}, WordNet synset {wn_offset} and UMLS concept {cui} name one class (D99 §11).\";\n\
         \x20   core:classes = [{hp_class}, wn:n{wn_offset}, umlscui:{cui}];\n\
         \x20   core:mapping_justification = \"semapv:MappingChaining\";\n\
         \x20   prov:was_attributed_to = agent:eigenius_core_team;\n\
         \x20   prov:rationale = \"MRCONSO (SAB=HPO) maps {hp_code} to {cui}; the WordNet-UMLS adjudication (merges.json) maps {cui} to synset {wn_offset}.\";\n\
         }}\n\n"
    )
}

pub fn load_merges(path: &std::path::Path) -> std::io::Result<Merges> {
    #[derive(serde::Deserialize)]
    struct M {
        cui: String,
        offset: String,
        surface: String,
    }
    let text = std::fs::read_to_string(path)?;
    let rows: Vec<M> = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(rows
        .into_iter()
        .map(|m| ((m.cui, m.surface.to_lowercase()), m.offset))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redefinition_points_the_entry_at_the_wordnet_class_and_preserves_everything_else() {
        let r = Rewrite {
            entry_iri: "urn:eigenius:umlscui:e_C1442792_0".into(),
            num: "num_any".into(),
            class: "wn:n00024720".into(),
            form: "State".into(),
            sense: "umls:C1442792".into(),
            in_lexicon: "lexicon:umls".into(),
            sem_type: "Set".into(),
        };
        let esl = render(&r);
        // The two fields that carry concept identity now name the WordNet class…
        assert!(esl.contains(
            "lexicon:cat        = type_expr( lexicon:cat_n(wn:n00024720, lexicon:num_any) );"
        ));
        assert!(esl.contains("lexicon:sem        = wn:n00024720;"));
        // …and the entry's own identity, surface, sense and lexicon membership are untouched.
        assert!(esl.contains("resource umlscui:e_C1442792_0 :"));
        assert!(esl.contains(r#"lexicon:form       = "State";"#));
        assert!(esl.contains(r#"lexicon:sense      = "umls:C1442792";"#));
        assert!(esl.contains("lexicon:in_lexicon = lexicon:umls;"));
        // No class is created, and no subclass edge is emitted. The lattice is untouched.
        assert!(!esl.contains("class "));
        assert!(!esl.contains("subclass"));
    }

    #[test]
    fn the_mass_variant_stays_a_mass_variant() {
        // The importer emits an ADDITIVE `cat_n(C, mass)` alongside the count entry for a mass
        // concept. Rewriting the class must not collapse the two — the `num` argument is preserved.
        let r = Rewrite {
            entry_iri: "urn:eigenius:umlscui:e_C1442792_0_mass".into(),
            num: "mass".into(),
            class: "wn:n00024720".into(),
            form: "State".into(),
            sense: "umls:C1442792".into(),
            in_lexicon: "lexicon:umls".into(),
            sem_type: "Set".into(),
        };
        let esl = render(&r);
        assert!(esl.contains("lexicon:cat_n(wn:n00024720, lexicon:mass)"));
        assert!(esl.contains("resource umlscui:e_C1442792_0_mass :"));
    }

    #[test]
    fn an_hpo_redefinition_points_the_entry_at_the_hp_class() {
        let r = Rewrite {
            entry_iri: "urn:eigenius:umlscui:e_C0036572_0".into(),
            num: "num_any".into(),
            class: crate::hpo::hp_qname("HP:0001250"),
            form: "Seizures".into(),
            sense: "umls:C0036572".into(),
            in_lexicon: "lexicon:umls".into(),
            sem_type: "Set".into(),
        };
        let esl = render(&r);
        assert!(esl.contains("lexicon:cat_n(hp:'0001250', lexicon:num_any)"));
        assert!(esl.contains("lexicon:sem        = hp:'0001250';"));
        assert!(esl.contains("resource umlscui:e_C0036572_0 :"));
        assert!(!esl.contains("subclass"));
    }
}
