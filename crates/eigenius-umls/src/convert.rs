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

//! Render a joined UMLS [`Subset`] into an Eigon/ESL document: a faithful **typed
//! mirror** plus a **derived domain lexicon** (D65 §5).
//!
//! Three parts, one document, layered so the lexicon is a *view* of the mirror:
//!
//! 1. **Semantic-type classes.** Each used TUI → `class umlssty:<TUI> : lexicon:Entity`
//!    (the semantic network, flattened at the `Entity` top for v1 — the TUI ISA
//!    hierarchy is a follow-on). The TUI is the IRI local; the name is the description.
//! 2. **Concept classes.** Each CUI → `class umlscui:<CUI> : <its TUI classes>` — the
//!    `subclass_of` edges ARE the semantic typing (queryable structurally), reaching
//!    `lexicon:Entity` transitively. The CUI is the IRI local; the definition is the
//!    description. This parallels WordNet's common-noun synset (offset = IRI local,
//!    hypernym = subclass_of, gloss = description).
//! 3. **Lexicon (derived).** One `lexicon:Lexicon` (`lexicon:umls`) and, per concept,
//!    a common-noun **N** `lexicon:LexicalEntry` per English surface string —
//!    `cat_n(umlscui:<CUI>, num_any)`, `sem =` the concept class, `sem_type = Set`,
//!    `in_lexicon = lexicon:umls`.
//!
//! Because a concept is a *class* under `lexicon:Entity`, it is used as a **kind**
//! (the WordNet common-noun path) — a determiner quantifies it ("every Werner
//! syndrome …"), and it flows into general predicate slots by subsumption.

use std::collections::{BTreeMap, BTreeSet};

use crate::rrf::Subset;

/// The stable lexicon identity for this importer's output (D65 §3).
pub const UMLS_LEXICON: &str = "lexicon:umls";

/// **Junk-atom drop set** (D63 cross-lexicon alignment): `CUI → { original-case forms to skip }`.
///
/// Produced by `lexicon-align drops` — atoms whose only contribution is a case-mangled collision
/// with a common word (`gENE`→`gene`), which the adjudicator judged a **different concept** at high
/// confidence. Matched by the **exact original casing**: the importer keeps one form per
/// `(cui, lowercase)` (first-seen casing wins, [`crate::rrf`]), so an exact match drops the surviving
/// form only when it IS the irregular one — a clean `GENE` that survived dedup is correctly kept.
/// Every dropped surface is by construction also a WordNet lemma, so the common word stays covered.
pub type DropSet = BTreeMap<String, BTreeSet<String>>;

/// Whether `(cui, form)` is a junk atom to skip ([`DropSet`]).
fn is_dropped(drops: &DropSet, cui: &str, form: &str) -> bool {
    drops.get(cui).is_some_and(|fs| fs.contains(form))
}

/// Document header: the **UMLS license notice** (load-bearing — the redistribution
/// constraint flows to every downstream user) + namespace declarations. `{version}`
/// is the Metathesaurus release the import was built from.
fn esl_header(version: &str) -> String {
    format!(
        "\
// ════════════════════════════════════════════════════════════════════
// DERIVED FROM the UMLS Metathesaurus (U.S. National Library of Medicine),
// release {version}. This is a DERIVATIVE WORK governed by the UMLS
// Metathesaurus License Agreement:
//   https://uts.nlm.nih.gov/uts/assets/LicenseAgreement.pdf
//
// Redistribution of this artifact does NOT grant a UMLS license. Each
// downstream user MUST obtain their own UMLS license from the NLM
// (https://uts.nlm.nih.gov/uts/) before use.
//
// Only SRL-0 (Level 0 / Category 0) sources are included; sources with a
// higher Source Restriction Level (e.g. SNOMED CT, CPT) are EXCLUDED.
// ════════════════════════════════════════════════════════════════════
namespace core       = \"urn:eigenius:core\";
namespace reflection = \"urn:eigenius:reflection\";
namespace epistemic  = \"urn:eigenius:reflection:epistemic\";
namespace eigentt    = \"urn:eigenius:eigentt\";
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace umlssty    = \"urn:eigenius:umlssty\";
namespace umlscui    = \"urn:eigenius:umlscui\";
"
    )
}

/// Coverage of one import run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// `umls:SemanticType` classes emitted (one per used TUI).
    pub semantic_types: usize,
    /// `umls:Concept` classes emitted (one per CUI).
    pub concepts: usize,
    /// `lexicon:LexicalEntry` entries emitted (one per concept surface form).
    pub entries: usize,
    /// Additive `cat_n(C, mass)` entries emitted for mass concepts (countability by type-mass OR
    /// count-vetoed head-inheritance; [`concept_is_mass`]).
    pub mass_entries: usize,
    /// Content entries skipped because the surface is a grammatical filler UMLS mints as a concept
    /// ([`is_grammatical_surface`], D63 §5.3) — `does not`, `not`, `to`, `lead`, `alone`.
    pub grammatical_skipped: usize,
    /// Content entries skipped because the `(cui, form)` atom is a junk case-collision in the
    /// [`DropSet`] (`gENE`) — a mangled acronym folding onto a common word (D63 alignment).
    pub junk_skipped: usize,
    /// Content entries skipped because the surface is a regular INFLECTION of another form of the same
    /// concept ("genes" beside "gene") — [`is_inflection_of_sibling`]. The lexicon is lemma-keyed and the
    /// lemmatizer reaches the concept from the plural via the singular entry, so the inflected entry is
    /// redundant; keeping it made the parser read a plural surface as SINGULAR (`seed`'s
    /// surface-equals-lemma rule) and licensed a singular classifier for "genes".
    pub inflected_skipped: usize,
    /// Common-noun entries withheld because the concept's semantic types are entirely NON-CONTENT
    /// ([`NON_CONTENT_TUIS`]) — a relation/idea/qualifier reification (`And` C1515981, `Associated
    /// with` C0332281), not a thing. The concept CLASS still ships; only the `cat_n` entry is withheld.
    pub non_content_skipped: usize,
}

/// Escape a string for an ESL double-quoted literal.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Emit one semantic-type class (`umlssty:<TUI> : lexicon:Entity`).
fn push_semantic_type(buf: &mut String, tui: &str, name: &str) {
    buf.push_str(&format!(
        "class umlssty:{tui} : lexicon:Entity {{\n\
         \x20   description = \"UMLS Semantic Type {tui} — {name}.\";\n\
         }}\n\n",
        name = esc(name),
    ));
}

/// Emit one concept's chain node. A **concept class** (`class umlscui:<CUI> : <TUI classes>`) by
/// default; a **named individual** (`is_individual`, D62 — `docs/notes/d62-named-individual-typing.md`)
/// is instead an **instance** (`resource umlscui:<CUI> : <TUI classes>`) of its semantic-type
/// class(es), so a `cat_np` entry can name it.
fn push_concept(buf: &mut String, cui: &str, tuis: &[String], desc: &str, is_individual: bool) {
    let parents: Vec<String> = tuis.iter().map(|t| format!("umlssty:{t}")).collect();
    // A `class` body takes the bare `description` class-item keyword; a `resource` (instance) body
    // takes the qualified `core:description` property.
    let (keyword, desc_prop) = if is_individual {
        ("resource", "core:description")
    } else {
        ("class", "description")
    };
    buf.push_str(&format!(
        "{keyword} umlscui:{cui} : {} {{\n\
         \x20   {desc_prop} = \"{}\";\n\
         }}\n\n",
        parents.join(", "),
        esc(desc),
    ));
}

/// The concept's `description` text: the preferred name, the definition (with its
/// source) when present, and the CUI for provenance.
fn concept_description(
    preferred: &str,
    definition: Option<&(String, String)>,
    cui: &str,
) -> String {
    match definition {
        Some((sab, def)) => format!("{preferred} — {def} [{sab}] UMLS CUI {cui}."),
        None => format!("{preferred}. UMLS CUI {cui}."),
    }
}

/// The mass/uncountable-noun lookup set (lowercased lemmas) — the **shared** `--countability` lexicon
/// (Wiktionary `Category:English uncountable nouns` ∩ WordNet, `scripts/provision-countability.sh`) the
/// WordNet importer also consumes. Empty ⇒ no mass shim (count-only, the prior behaviour).
pub type MassNouns = std::collections::BTreeSet<String>;

/// UMLS semantic types that are **inherently mass (uncountable)** — mass *regardless of the
/// preferred-name head* — by BRANCH of the Semantic Network tree (STN positions in `MRSTY.RRF`;
/// `docs/notes/d63-countability-from-subsumption.md` §4a): the Substance subtree (A1.4.\*) and the
/// Phenomenon/Process/Function/Dysfunction subtree (B2.\* core). This fires for concepts whose head is
/// countable — `methylation` (T044) is mass though "methylation" is not in the uncountable list.
/// (Diseases/neoplasms are deliberately NOT here — they reach mass via head-inheritance, which the
/// [`COUNT_VETO_TUIS`] does not veto; see [`concept_is_mass`].)
const MASS_DENOTING_TUIS: &[&str] = &[
    // Substance (A1.4.*) — chemicals, nucleic acids, proteins, body substances, food.
    "T031", "T103", "T104", "T109", "T114", "T116", "T120", "T121", "T122", "T123", "T125", "T126",
    "T127", "T129", "T130", "T131", "T167", "T168", "T192", "T195", "T196", "T197",
    // Phenomenon / Process / Function / Dysfunction (B2.* core).
    "T038", "T039", "T040", "T041", "T042", "T043", "T044", "T045", "T046", "T049", "T067", "T068",
    "T069", "T070",
];

/// UMLS semantic types that denote a **discrete COUNT entity**, for which head-inheritance's
/// uncountable-head mass is a **false positive** and is VETOED — the Physical-Object non-substance
/// branches (Organism A1.1.\*, Anatomical Structure A1.2.\* incl. Gene or Genome T028 / Cell T025,
/// Manufactured Object A1.3.\*) plus Finding / Laboratory Result / Sign or Symptom (A2.2) and
/// Experimental Model of Disease (T050). This is the subclass-hierarchy **precision veto**
/// (`d63-countability-from-subsumption.md` §5, count-veto): it removes the head-string false positives
/// — the `gENE`→"gene" collision (its concept "Gross Extranodal Extension" C5849123 is T033 Finding,
/// head "extension") — **without** touching head-inheritance's coverage of diseases/neoplasms used as
/// bare mass in scientific prose (`cause cancer`, `arise from Lynch syndrome`; T191/T047 are absent
/// here so their head "cancer" still masses).
const COUNT_VETO_TUIS: &[&str] = &[
    // Organism (A1.1.*).
    "T001", "T002", "T004", "T005", "T007", "T008", "T010", "T011", "T012", "T013", "T014", "T015",
    "T016", "T194", "T204",
    // Anatomical Structure (A1.2.*) — incl. Gene or Genome T028, Cell T025.
    "T017", "T018", "T019", "T020", "T021", "T023", "T024", "T025", "T026", "T028", "T190",
    // Manufactured Object (A1.3.*).
    "T073", "T074", "T075", "T200", "T203",
    // Finding / Laboratory Result / Sign or Symptom (A2.2) + Experimental Model of Disease.
    "T033", "T034", "T184", "T050",
];

/// **Countability = type-mass OR gated head-inheritance (`docs/notes/d63-countability-from-subsumption.md`
/// §5, count-veto).** A concept is mass iff EITHER
/// - **(type-mass)** its semantic type is inherently mass ([`MASS_DENOTING_TUIS`]), which fires even
///   when the preferred-name head is countable (`methylation` T044); OR
/// - **(head-inheritance, VETOED)** the last word of its preferred name is uncountable AND its type is
///   NOT a discrete count entity ([`COUNT_VETO_TUIS`]).
///
/// A mass concept gets an ADDITIVE `cat_n(C, mass)` entry (the count `cat_n(C, num_any)` stays). The
/// subclass-hierarchy veto is what makes head-inheritance precise: it keeps its broad coverage —
/// diseases/neoplasms are bare-mass in scientific prose (`cause cancer`, `arise from Lynch syndrome`,
/// head "cancer") and reach mass through it — while removing the false positives on discrete count
/// entities (`gene` T028; the junk atom `gENE` = "Gross Extranodal Extension", C5849123, T033 Finding →
/// vetoed → no mass, so the `gENE`→"gene" collision is gone). A prior pure-TUI-subsumption version
/// (no head-inheritance) regressed: it excluded T191/T047, dropping the mass of `cancer`/`Lynch
/// syndrome` and gapping `cause Lynch syndrome` — the veto is the fix (§8 of the note).
fn concept_is_mass(preferred_name: &str, tuis: &[String], mass: &MassNouns) -> bool {
    let type_mass = tuis
        .iter()
        .any(|t| MASS_DENOTING_TUIS.contains(&t.as_str()));
    let head_mass = head_is_uncountable(preferred_name, mass)
        && !tuis.iter().any(|t| COUNT_VETO_TUIS.contains(&t.as_str()));
    type_mass || head_mass
}

/// The head-inheritance signal: the last word of `preferred_name` (stripped of non-alphanumerics,
/// lowercased) is in the uncountable-noun list. Gated by [`COUNT_VETO_TUIS`] in [`concept_is_mass`].
fn head_is_uncountable(preferred_name: &str, mass: &MassNouns) -> bool {
    preferred_name
        .split_whitespace()
        .last()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .map(|head| mass.contains(&head))
        .unwrap_or(false)
}

/// Emit one `lexicon:LexicalEntry` for `form` under `cat` / `sem_type`, IRI `umlscui:e_{cui}_{i}{suffix}`.
fn emit_entry(
    buf: &mut String,
    cui: &str,
    i: usize,
    suffix: &str,
    form: &str,
    cat: &str,
    sem_type: &str,
) {
    buf.push_str(&format!(
        "resource umlscui:e_{cui}_{i}{suffix} : lexicon:LexicalEntry {{\n\
         \x20   lexicon:form       = \"{form}\";\n\
         \x20   lexicon:cat        = type_expr( {cat} );\n\
         \x20   lexicon:sem        = umlscui:{cui};\n\
         \x20   lexicon:sem_type   = type_expr( {sem_type} );\n\
         \x20   lexicon:sense      = \"umls:{cui}\";\n\
         \x20   lexicon:grade      = epistemic:declared;\n\
         \x20   lexicon:in_lexicon = lexicon:umls;\n\
         }}\n\n",
        form = esc(form),
    ));
}

// A concept class yields common-noun `cat_n(C, num_any)` entries (`sem_type = Set`); a mass concept
// (head-inheritance, [`concept_is_mass`]) ALSO gets an additive `cat_n(C, mass)` reading per form, so a
// bare occurrence shifts to a mass NP (the RC-1 fix — bare `MSI` was a grammar-gap because UMLS typed it
// count-only). A named individual (gene symbol) yields proper-noun `cat_np(TUI, sg)` entries and is never
// mass-shimmed. `DNA`/`RNA` etc. that are English nouns get their mass reading here too (same lexicon the
// WordNet importer uses), a harmless duplicate of any WordNet mass entry (distinct entry IRIs).
/// **Grammatical surfaces UMLS mints as concepts** (D63 §5.3) — do-support, negation, and the
/// qualifier/linkage fillers UMLS encodes as `cat_n` concepts (`does not`=C1299585, `Not`/`Non`=C1518422,
/// `Lead`/`Leading`=C1522538, `To`=C1883351, `alone`=C0679994). In prose these are function words / verbs
/// whose real reading is in WordNet or the closed-class bootstrap; the UMLS *noun* concept only feeds the
/// spurious noun-compound pile (the sentence-3 negation-dropping parse). Their per-form entry is skipped
/// (the concept class stays); WordNet's `lead` verb and the closed-class `not`/`to`/`does` are untouched.
/// Case-insensitive, exact surface match. Curated (the SNOMED "Interpretation value" / "Linkage concept"
/// hierarchy is the principled superset but is not in the SRL-0 subset we import).
const GRAMMATICAL_SURFACES: &[&str] = &[
    "do", "does", "did", "doing", "done", "do not", "does not", "did not", "not", "non", "non-",
    "negation", "negated", "to", "alone", "lead", "leading",
];

// The closed-class SURFACE lists (prepositions/conjunctions, determiners, copula forms) now live in
// `eigenius_kernel::dcg::closed_class`, shared with the WordNet importer so the two cannot drift; see
// that module for the rationale. Retained here is the UMLS-specific EVIDENCE for why each group must be
// withheld — the content senses colliding on these surfaces are function-word reifications
// (`For (preposition)` C0521125 seeded `[for] therapeutics`; `From` C1517320, `Into` C0332286,
// `At` C1516077, `Within` C0332285, `As - qualifier` C1883713), qualifier-value reifications of
// determiners (`Some (qualifier value)` C0205392 piled into a *some-MSI-line* compound, D63 Defect 2a),
// and chemical-symbol / gene-acronym homonyms (`as`=arsenic, `in`=indium, `Be`=beryllium, `no`=NO nitric
// oxide). A content VERB sense of the copula is the worst of them: it supplies a linking frame
// `be(λx.P(x), subj)` that re-encodes "X is P" as an opaque 2-place relation, destroying the copula's
// transparency. In every case the concept CLASS stays (the mirror is intact) and only the per-form
// common-noun entry is skipped, so nothing becomes unreachable.

/// Semantic types that denote a RELATION / IDEA / QUALIFIER, not a THING — UMLS terminology-cruft that
/// must not seed a common noun. A concept typed ONLY by these reifies a grammatical relation and piles
/// into compounds (`And` C1515981 = T078; `Associated with` C0332281 = T080 → "MSI is an *associated-
/// with disease-response*"). These are INVISIBLE to the lexicon-align drops (which require a WordNet-
/// NOUN collision; a conjunction/qualifier collides with neither), so they must be filtered HERE, by
/// TYPE. The concept CLASS + any named individual still ship; only the `cat_n` common-noun entry is
/// withheld, and the surface stays known via WordNet / the closed-class bootstrap.
const NON_CONTENT_TUIS: &[&str] = &[
    "T078", // Idea or Concept
    "T080", // Qualitative Concept
];

/// A concept whose semantic types are ENTIRELY non-content — so it is not a common noun. A concept that
/// ALSO carries a content type (a body part, a substance, a disease, …) keeps its noun entry.
fn is_non_content_concept(tuis: &[String]) -> bool {
    !tuis.is_empty() && tuis.iter().all(|t| NON_CONTENT_TUIS.contains(&t.as_str()))
}

/// Whether `form` is a grammatical or function-word surface UMLS should not seed as a content noun
/// ([`GRAMMATICAL_SURFACES`] / [`FUNCTION_WORD_SURFACES`]).
fn is_grammatical_surface(form: &str) -> bool {
    let f = form.trim().to_ascii_lowercase();
    // The prepositions/conjunctions, determiners and copula forms now come from the SHARED list
    // (`eigenius_kernel::dcg::closed_class`) that the WordNet importer also consults — one definition of
    // "the closed class owns this surface", so the two importers cannot drift. GRAMMATICAL_SURFACES
    // stays LOCAL: `lead`/`alone`/`negation`/do-support are UMLS *reification* artefacts, and `lead` is
    // a legitimate WordNet content noun and verb that must not be dropped corpus-wide.
    GRAMMATICAL_SURFACES.contains(&f.as_str())
        || eigenius_kernel::dcg::closed_class::is_closed_class_surface(&f)
}

/// Whether `form` is a **regular inflection of another form of the same concept** — "genes" beside
/// "gene". Uses the shared detachment rule ([`eigenius_kernel::dcg::regular_plural_stem`]) so the gate
/// removes exactly the surfaces the lemmatizer can already reach from the sibling, case-insensitively.
///
/// UMLS `MRCONSO.STR` holds SURFACE strings, plurals included, while the lexicon is lemma-keyed and
/// WordNet honours that by construction. Emitting an inflected form as a lemma-equivalent entry with
/// `num_any` made the parser's number heuristic ("a surface equal to the lemma is singular",
/// `dcg::parse::seed`) infer SINGULAR for "genes" — which is sound for a lemma-keyed lexicon and wrong
/// only because this importer broke that assumption. Dropping the entry restores the assumption instead
/// of making the parser compensate.
///
/// **Scoped WITHIN one concept, which is what makes it safe.** The sibling must be present, so a concept
/// whose only form is plural ("Vital Signs") keeps it and is never lost. It also sidesteps the exception
/// class a parser-side rule would have to adjudicate: `species` is untouched because no concept of it
/// carries a sibling `specie`.
fn is_inflection_of_sibling(form: &str, forms: &[String]) -> bool {
    let Some(stem) = eigenius_kernel::dcg::regular_plural_stem(form) else {
        return false;
    };
    forms
        .iter()
        .any(|other| other.trim().to_lowercase() == stem)
}

fn push_entries(
    buf: &mut String,
    cui: &str,
    forms: &[String],
    named_tui: Option<&str>,
    is_mass: bool,
    drops: &DropSet,
    rep: &mut Report,
) {
    let (cat, sem_type) = match named_tui {
        Some(tui) => (
            format!("lexicon:cat_np(umlssty:{tui}, lexicon:sg)"),
            format!("umlssty:{tui}"),
        ),
        None => (
            format!("lexicon:cat_n(umlscui:{cui}, lexicon:num_any)"),
            "Set".to_string(),
        ),
    };
    let mass_cat = format!("lexicon:cat_n(umlscui:{cui}, lexicon:mass)");
    for (i, form) in forms.iter().enumerate() {
        // Grammatical surface (do-support / negation / qualifier filler): skip the content entry — the
        // real reading is WordNet's or the closed-class bootstrap's (D63 §5.3, `is_grammatical_surface`).
        if is_grammatical_surface(form) {
            rep.grammatical_skipped += 1;
            continue;
        }
        // Junk case-collision (D63 alignment): a mangled acronym (`gENE`) the adjudicator judged a
        // different concept from the common word it folds onto. Skip the content entry; WordNet still
        // covers the word (every dropped surface is a WordNet lemma). The concept class stays.
        if is_dropped(drops, cui, form) {
            rep.junk_skipped += 1;
            continue;
        }
        // Inflected duplicate ("genes" beside "gene"): the lexicon is lemma-keyed, and the lemmatizer
        // reaches this concept from the plural through the singular sibling's entry
        // ([`is_inflection_of_sibling`]).
        if is_inflection_of_sibling(form, forms) {
            rep.inflected_skipped += 1;
            continue;
        }
        emit_entry(buf, cui, i, "", form, &cat, &sem_type);
        rep.entries += 1;
        if is_mass && named_tui.is_none() {
            emit_entry(buf, cui, i, "_mass", form, &mass_cat, "Set");
            rep.entries += 1;
            rep.mass_entries += 1;
        }
    }
}

/// The `lexicon:umls` descriptor (D65 §3) — the stable identity of this domain lexicon.
fn lexicon_descriptor(version: &str) -> String {
    format!(
        "resource lexicon:umls : lexicon:Lexicon {{\n\
         \x20   lexicon:source   = \"UMLS Metathesaurus {version} — Level 0 / SRL-0 sources only\";\n\
         \x20   lexicon:version  = \"{version}\";\n\
         \x20   lexicon:language = \"en\";\n\
         \x20   lexicon:domain   = \"biomedical\";\n\
         \x20   lexicon:license  = \"UMLS Metathesaurus License (NLM). This is a derivative work; redistribution requires each recipient to hold their own UMLS license — https://uts.nlm.nih.gov/uts/\";\n\
         }}\n\n",
    )
}

/// The document header (license notice + namespace declarations). Public so a
/// partitioned emit can prepend it to every chunk file — each chunk must carry the
/// UMLS license notice and the namespaces it references.
pub fn header(version: &str) -> String {
    esl_header(version)
}

/// Render the **base layer**: the semantic-type classes (`umlssty:*`) + the
/// `lexicon:umls` descriptor. In a partitioned import this is layer 0; every concept
/// chunk resolves its `subclass_of umlssty:*` and `in_lexicon lexicon:umls` against it.
/// Returns the document (header + base) and the count of semantic-type classes.
pub fn render_base(subset: &Subset, version: &str) -> (String, usize) {
    let mut body = String::from(
        "// ── Semantic-type classes (the UMLS semantic network, flat at Entity) ──\n",
    );
    for st in &subset.semantic_types {
        push_semantic_type(&mut body, &st.tui, &st.name);
    }
    body.push_str(&lexicon_descriptor(version));
    (
        format!("{}\n{body}", esl_header(version)),
        subset.semantic_types.len(),
    )
}

/// Render one concept's block — its class (the mirror) plus its derived common-noun
/// entries. No header; callers concatenate blocks into chunk bodies. Returns the
/// rendered text and the block's [`Report`] (lexical-entry + additive mass-entry counts).
pub fn render_concept_block(
    c: &crate::rrf::Concept,
    mass: &MassNouns,
    drops: &DropSet,
) -> (String, Report) {
    let mut buf = String::new();
    let mut rep = Report::default();
    let desc = concept_description(&c.preferred_name, c.definition.as_ref(), &c.cui);
    // A named individual (a nomenclature symbol, e.g. an HGNC gene) is emitted as an INSTANCE of its
    // primary semantic-type class with `cat_np` entries; otherwise a concept class with `cat_n`.
    let named_tui: Option<&str> = c.symbol.as_ref().and(c.tuis.first()).map(|t| t.as_str());
    // RC-1 mass shim: a concept whose preferred-name head is uncountable, OR whose semantic type is a
    // process/function (T044 Molecular Function for `methylation`), gets an additive `mass` entry.
    let is_mass = concept_is_mass(&c.preferred_name, &c.tuis, mass);
    push_concept(&mut buf, &c.cui, &c.tuis, &desc, named_tui.is_some());
    // Withhold the common-noun entries for a purely non-content concept (a relation/idea/qualifier
    // reification), UNLESS it is a named individual (a symbol → `cat_np`, which does not pile into
    // compounds). The concept CLASS shipped above regardless, so the mirror stays intact.
    if named_tui.is_none() && is_non_content_concept(&c.tuis) {
        rep.non_content_skipped += c.forms.len();
    } else {
        push_entries(
            &mut buf, &c.cui, &c.forms, named_tui, is_mass, drops, &mut rep,
        );
    }
    (buf, rep)
}

/// Render the full mirror + derived lexicon as a SINGLE document for `subset`.
/// `version` labels the header notice and the lexicon descriptor (e.g. `"2026AA"`).
/// For large imports use the partitioned emit (the binary's `--out-dir`) instead, so
/// each layer stays under the gRPC message-size limit.
pub fn render_document(
    subset: &Subset,
    version: &str,
    mass: &MassNouns,
    drops: &DropSet,
) -> (String, Report) {
    let mut rep = Report::default();
    let (base, sty) = render_base(subset, version);
    rep.semantic_types = sty;

    let mut body = base;
    body.push_str("\n// ── Concept classes (the mirror) + derived common-noun entries ──\n");
    for c in &subset.concepts {
        let (block, brep) = render_concept_block(c, mass, drops);
        body.push_str(&block);
        rep.entries += brep.entries;
        rep.mass_entries += brep.mass_entries;
        rep.grammatical_skipped += brep.grammatical_skipped;
        rep.junk_skipped += brep.junk_skipped;
        rep.non_content_skipped += brep.non_content_skipped;
        rep.concepts += 1;
    }

    (body, rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rrf::{Concept, SemanticType};

    /// The inflected-form QC gate ([`is_inflection_of_sibling`]). UMLS ships surface strings, the
    /// lexicon is lemma-keyed, and an inflected entry made the parser read a plural surface as singular.
    #[test]
    fn inflected_forms_are_dropped_only_when_a_singular_sibling_exists() {
        let gene = vec!["gene".to_string(), "genes".to_string(), "Genes".to_string()];
        assert!(
            is_inflection_of_sibling("genes", &gene),
            "the plural is redundant beside its singular sibling"
        );
        assert!(
            is_inflection_of_sibling("Genes", &gene),
            "case-insensitive — UMLS varies capitalisation across atoms"
        );
        assert!(
            !is_inflection_of_sibling("gene", &gene),
            "the singular is the form the lexicon must keep"
        );
        // -ies → -y detachment.
        let vuln = vec!["vulnerability".to_string(), "vulnerabilities".to_string()];
        assert!(is_inflection_of_sibling("vulnerabilities", &vuln));

        // SAFETY 1 — a concept whose ONLY form is plural keeps it, so no concept is lost.
        let signs = vec!["Vital Signs".to_string()];
        assert!(
            !is_inflection_of_sibling("Vital Signs", &signs),
            "no singular sibling ⇒ the plural is this concept's only surface and must survive"
        );
        // SAFETY 2 — the exception class a parser-side rule would have had to adjudicate never arises,
        // because the sibling is not there.
        for solo in [
            vec!["species".to_string()],
            vec!["series".to_string()],
            vec!["analysis".to_string()],
            vec!["virus".to_string()],
            vec!["process".to_string()],
        ] {
            assert!(
                !is_inflection_of_sibling(&solo[0], &solo),
                "{} must not be treated as an inflection",
                solo[0]
            );
        }
        // A plural whose sibling belongs to a DIFFERENT concept is not visible here — the gate only
        // ever sees one concept's forms, which is what bounds it.
        let other = vec!["genes".to_string()];
        assert!(!is_inflection_of_sibling("genes", &other));
    }

    fn werner_subset() -> Subset {
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T047".to_string(),
                name: "Disease or Syndrome".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0043119".to_string(),
                tuis: vec!["T047".to_string()],
                preferred_name: "Werner Syndrome".to_string(),
                forms: vec![
                    "Werner Syndrome".to_string(),
                    "Werner's Syndrome".to_string(),
                ],
                definition: Some((
                    "MSH".to_string(),
                    "An autosomal recessive disorder.".to_string(),
                )),
                symbol: None, // a disease concept → stays a class (cat_n)
            }],
        }
    }

    /// The WRN **gene** (HGNC) — a NAMED INDIVIDUAL: `symbol = Some("WRN")`, TUI T028.
    fn wrn_gene_subset() -> Subset {
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T028".to_string(),
                name: "Gene or Genome".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C1337007".to_string(),
                tuis: vec!["T028".to_string()],
                preferred_name: "WRN".to_string(),
                forms: vec![
                    "WRN".to_string(),
                    "Werner syndrome RecQ like helicase".to_string(),
                ],
                definition: None,
                symbol: Some("WRN".to_string()),
            }],
        }
    }

    #[test]
    fn renders_mirror_and_lexicon() {
        let (doc, rep) = render_document(
            &werner_subset(),
            "2026AA",
            &MassNouns::new(),
            &DropSet::new(),
        );
        assert_eq!(rep.semantic_types, 1);
        assert_eq!(rep.concepts, 1);
        assert_eq!(rep.entries, 2);

        // Semantic-type class, rooted at Entity.
        assert!(doc.contains("class umlssty:T047 : lexicon:Entity {"));
        // Concept class subclassed under its semantic type (typing IS the edge).
        assert!(doc.contains("class umlscui:C0043119 : umlssty:T047 {"));
        // Definition + CUI folded into the description.
        assert!(doc.contains(
            "Werner Syndrome — An autosomal recessive disorder. [MSH] UMLS CUI C0043119."
        ));
        // Common-noun (cat_n) entry, sem = the concept class, sem_type = Set.
        assert!(doc.contains("lexicon:form       = \"Werner Syndrome\";"));
        assert!(doc.contains(
            "lexicon:cat        = type_expr( lexicon:cat_n(umlscui:C0043119, lexicon:num_any) );"
        ));
        assert!(doc.contains("lexicon:sem        = umlscui:C0043119;"));
        assert!(doc.contains("lexicon:sem_type   = type_expr( Set );"));
        assert!(doc.contains("lexicon:in_lexicon = lexicon:umls;"));

        // The lexicon descriptor appears exactly once.
        assert_eq!(
            doc.matches("resource lexicon:umls : lexicon:Lexicon")
                .count(),
            1
        );
        // Every LexicalEntry carries the lexicon tag.
        assert_eq!(
            doc.matches(": lexicon:LexicalEntry {").count(),
            doc.matches("lexicon:in_lexicon = lexicon:umls;").count()
        );
    }

    /// A gENE-shaped junk concept: `C5849123` ("Gross Extranodal Extension", a `Finding`) whose only
    /// atom under the `gene` key is the mangled acronym `gENE`. Its content entry must be dropped when
    /// the atom is in the [`DropSet`], and kept otherwise — the concept CLASS survives either way.
    fn gene_junk_subset() -> Subset {
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T033".to_string(),
                name: "Finding".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C5849123".to_string(),
                tuis: vec!["T033".to_string()],
                preferred_name: "Gross Extranodal Extension".to_string(),
                forms: vec!["gENE".to_string()],
                definition: None,
                symbol: None, // a Finding → a concept class (cat_n), the collision path
            }],
        }
    }

    #[test]
    fn a_dropped_junk_atom_is_skipped_but_the_concept_class_stays() {
        // Control: with no drops, the mangled `gENE` atom seeds a content entry (the over-generation).
        let (doc, rep) = render_document(
            &gene_junk_subset(),
            "2026AA",
            &MassNouns::new(),
            &DropSet::new(),
        );
        assert_eq!(rep.entries, 1);
        assert_eq!(rep.junk_skipped, 0);
        assert!(doc.contains("lexicon:form       = \"gENE\";"));

        // With the drop set, the content entry is skipped — but the concept class is untouched.
        let mut drops = DropSet::new();
        drops
            .entry("C5849123".to_string())
            .or_default()
            .insert("gENE".to_string());
        let (doc, rep) = render_document(&gene_junk_subset(), "2026AA", &MassNouns::new(), &drops);
        assert_eq!(rep.entries, 0, "the gENE content entry is not emitted");
        assert_eq!(rep.junk_skipped, 1);
        assert!(!doc.contains("lexicon:form       = \"gENE\";"));
        assert!(
            !doc.contains(": lexicon:LexicalEntry {"),
            "no lexical entry survives"
        );
        // The concept CLASS (the mirror) is still emitted — only the surface form is dropped.
        assert!(doc.contains("class umlscui:C5849123 : umlssty:T033 {"));
    }

    /// The match is EXACT original casing: a drop targeting the mangled `gENE` must NOT remove a clean
    /// `GENE` that survived the importer's case-insensitive dedup (first-seen casing wins, [`crate::rrf`]).
    #[test]
    fn a_clean_cased_form_is_not_dropped_by_a_mangled_atom_drop() {
        let mut subset = gene_junk_subset();
        subset.concepts[0].forms = vec!["GENE".to_string()]; // clean all-caps survived dedup
        let mut drops = DropSet::new();
        drops
            .entry("C5849123".to_string())
            .or_default()
            .insert("gENE".to_string()); // the drop names the MANGLED casing
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &drops);
        assert_eq!(rep.junk_skipped, 0, "the clean GENE form is spared");
        assert_eq!(rep.entries, 1);
        assert!(doc.contains("lexicon:form       = \"GENE\";"));
    }

    #[test]
    fn named_individual_gene_renders_as_instance_with_cat_np_entries() {
        // A gene (HGNC symbol → named individual, D62) is an INSTANCE of its semantic-type class
        // with PROPER-NOUN (cat_np) entries — so it works as both a bare NP and a prenominal modifier.
        let (doc, _) = render_document(
            &wrn_gene_subset(),
            "2026AA",
            &MassNouns::new(),
            &DropSet::new(),
        );
        // The CUI is a `resource` (instance), NOT a `class`, typed by its semantic type.
        assert!(doc.contains("resource umlscui:C1337007 : umlssty:T028 {"));
        assert!(!doc.contains("class umlscui:C1337007"));
        // Proper-noun (cat_np) entry over the semantic-type class, sem = the instance, sg.
        assert!(doc.contains("lexicon:form       = \"WRN\";"));
        assert!(doc.contains(
            "lexicon:cat        = type_expr( lexicon:cat_np(umlssty:T028, lexicon:sg) );"
        ));
        assert!(doc.contains("lexicon:sem        = umlscui:C1337007;"));
        assert!(doc.contains("lexicon:sem_type   = type_expr( umlssty:T028 );"));
        // No common-noun (cat_n) entry for a named individual.
        assert!(!doc.contains("lexicon:cat_n(umlscui:C1337007"));
        // Every form is a name of the individual (both cat_np).
        assert!(doc.contains("lexicon:form       = \"Werner syndrome RecQ like helicase\";"));
    }

    #[test]
    fn header_carries_the_umls_license_and_redistribution_constraint() {
        let (doc, _) = render_document(
            &werner_subset(),
            "2026AA",
            &MassNouns::new(),
            &DropSet::new(),
        );
        assert!(doc.contains("UMLS Metathesaurus"));
        assert!(doc.contains("MUST obtain their own UMLS license"));
        assert!(doc.contains("SRL-0"));
        assert!(doc.contains("2026AA"));
    }

    /// An MSI-like concept: preferred head `instability` is uncountable → the concept + its abbreviation
    /// `MSI` are mass (C0920269, "Microsatellite Instability", a T047 phenomenon, no nomenclature symbol).
    fn msi_subset() -> Subset {
        // MSI (C0920269) is really T049 Cell or Molecular Dysfunction (per MRSTY), a mass-denoting
        // Phenomenon/Process branch type — so it is mass by semantic type, no head-inheritance needed.
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T049".to_string(),
                name: "Cell or Molecular Dysfunction".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0920269".to_string(),
                tuis: vec!["T049".to_string()],
                preferred_name: "Microsatellite Instability".to_string(),
                forms: vec!["MSI".to_string(), "Microsatellite Instability".to_string()],
                definition: None,
                symbol: None,
            }],
        }
    }

    #[test]
    fn mass_concept_is_mass_by_semantic_type_not_head() {
        // Countability by subsumption: MSI (C0920269, T049 Cell or Molecular Dysfunction ∈
        // MASS_DENOTING_TUIS) gets an ADDITIVE `cat_n(C, mass)` per form — bare `MSI` shifts to a mass
        // subject — with an EMPTY countability set, proving the head-string heuristic is retired.
        let (doc, rep) =
            render_document(&msi_subset(), "2026AA", &MassNouns::new(), &DropSet::new());
        assert_eq!(rep.entries, 4, "2 forms × (count + mass)");
        assert_eq!(rep.mass_entries, 2);
        assert!(doc.contains("lexicon:cat_n(umlscui:C0920269, lexicon:num_any)"));
        assert!(doc.contains("lexicon:cat_n(umlscui:C0920269, lexicon:mass)"));
        // The abbreviation `MSI` (form 0) itself carries the mass reading — the fix's whole point.
        assert!(doc.contains("resource umlscui:e_C0920269_0_mass : lexicon:LexicalEntry {"));
        assert!(doc.contains("lexicon:form       = \"MSI\";"));
    }

    #[test]
    fn count_entity_concept_gets_no_mass_entry() {
        // `Werner Syndrome` (T047 Disease) is not inherently mass (not in MASS_DENOTING_TUIS) and its
        // head `syndrome` is not uncountable → no mass. (T047 is NOT count-vetoed: a disease WITH an
        // uncountable head — e.g. "…Cancer" — DOES mass via head-inheritance; see the regression guard.)
        let (doc, rep) = render_document(
            &werner_subset(),
            "2026AA",
            &MassNouns::new(),
            &DropSet::new(),
        );
        assert_eq!(rep.mass_entries, 0);
        assert!(!doc.contains("lexicon:mass"));
    }

    #[test]
    fn count_veto_kills_head_inheritance_false_positive() {
        // The reported failure: "Gross Extranodal Extension" (C5849123, T033 Finding) has the
        // uncountable head "extension", so head-inheritance masses it and its acronym atom `gENE`
        // collides with the surface `gene`. T033 Finding ∈ COUNT_VETO_TUIS → head-inheritance is
        // VETOED → NO mass entry, even with "extension" in the countability set. The precision win.
        let mut mass = MassNouns::new();
        mass.insert("extension".to_string()); // head-inheritance WOULD fire — but the veto suppresses it.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T033".to_string(),
                name: "Finding".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C5849123".to_string(),
                tuis: vec!["T033".to_string()],
                preferred_name: "Gross Extranodal Extension".to_string(),
                forms: vec!["gENE".to_string(), "Gross Extranodal Extension".to_string()],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &mass, &DropSet::new());
        assert_eq!(
            rep.mass_entries, 0,
            "T033 Finding is count-vetoed → no gENE mass form"
        );
        assert!(!doc.contains("lexicon:mass"));
    }

    #[test]
    fn non_content_filler_concept_ships_class_but_no_forms() {
        // UMLS mints grammatical fillers as concepts (`does not`=C1299585, T080 Qualitative Concept).
        // As a purely non-content concept it ships its CLASS but NO common-noun entry — not even its
        // nominalized `absence of action` form, which as a common noun is the same qualifier-reification
        // junk (and is WordNet-backstopped if genuinely a word). This subsumes the older per-surface
        // grammatical skip for filler concepts, catching them by TYPE rather than by an enumerated form.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T080".to_string(),
                name: "Qualitative Concept".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C1299585".to_string(),
                tuis: vec!["T080".to_string()],
                preferred_name: "Does not".to_string(),
                forms: vec!["does not".to_string(), "absence of action".to_string()],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &DropSet::new());
        assert!(doc.contains("class umlscui:C1299585 :")); // the concept class is kept (mirror intact)
        assert!(!doc.contains("lexicon:form       = \"does not\";"));
        assert!(!doc.contains("lexicon:form       = \"absence of action\";"));
        assert_eq!(rep.non_content_skipped, 2);
        assert_eq!(rep.grammatical_skipped, 0);
    }

    #[test]
    fn function_word_surface_gets_no_content_entry() {
        // `For (preposition)` (C0521125, T080) — a UMLS reification of the preposition. Its `for` form
        // must not seed a content noun (it piles into a compound, `[for] therapeutics`); the preposition
        // is the closed-class bootstrap's. A chemical-symbol homonym on the same surface is dropped too.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T080".to_string(),
                name: "Qualitative Concept".to_string(),
            }],
            concepts: vec![
                Concept {
                    cui: "C0521125".to_string(),
                    tuis: vec!["T080".to_string()],
                    preferred_name: "For (preposition)".to_string(),
                    forms: vec!["for".to_string()],
                    definition: None,
                    symbol: None,
                },
                Concept {
                    cui: "C0003818".to_string(),
                    tuis: vec!["T121".to_string()],
                    preferred_name: "arsenic".to_string(),
                    // the element symbol `as` collides with the conjunction; the full name does not.
                    forms: vec!["as".to_string(), "arsenic".to_string()],
                    definition: None,
                    symbol: None,
                },
            ],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &DropSet::new());
        assert!(doc.contains("class umlscui:C0521125 :")); // both concept classes kept
        assert!(doc.contains("class umlscui:C0003818 :"));
        assert!(!doc.contains("lexicon:form       = \"for\";")); // preposition surface dropped
        assert!(!doc.contains("lexicon:form       = \"as\";")); // element-symbol homonym dropped too
        assert!(doc.contains("lexicon:form       = \"arsenic\";")); // the real content surface stays
                                                                    // `For (preposition)` (C0521125, T080) is now caught STRUCTURALLY by the non-content filter (a
                                                                    // Qualitative-Concept reification), not by the surface list. `as`=arsenic is a CONTENT concept
                                                                    // (T121) whose grammatical FORM `as` still goes through the per-form grammatical skip.
        assert_eq!(rep.non_content_skipped, 1);
        assert_eq!(rep.grammatical_skipped, 1);
    }

    #[test]
    fn copula_surface_gets_no_content_entry() {
        // `Be` = beryllium (T121, a CONTENT concept) — its element-symbol form collides with the copula.
        // The copula is the grammatical core of predication (closed-class bootstrap), and a `cat_n`
        // sense on `were`/`be` makes the verb a plural NOUN, so the per-form grammatical skip drops it.
        // Same accepted tradeoff as `as`=arsenic / `in`=indium: the concept CLASS stays and the full
        // name `beryllium` still seeds, so nothing becomes unreachable.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T121".to_string(),
                name: "Pharmacologic Substance".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0004667".to_string(),
                tuis: vec!["T121".to_string()],
                preferred_name: "beryllium".to_string(),
                forms: vec![
                    "be".to_string(),
                    "were".to_string(),
                    "beryllium".to_string(),
                ],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &DropSet::new());
        assert!(
            doc.contains("class umlscui:C0004667 :"),
            "the class is kept"
        );
        assert!(
            !doc.contains("lexicon:form       = \"be\";")
                && !doc.contains("lexicon:form       = \"were\";"),
            "copula surfaces must not seed content entries:\n{doc}"
        );
        assert!(
            doc.contains("lexicon:form       = \"beryllium\";"),
            "the real content surface stays:\n{doc}"
        );
        assert_eq!(rep.grammatical_skipped, 2);
        // `being` is EXCLUDED from the list — it is a legitimate common noun ("a living being").
        assert!(!is_grammatical_surface("being"));
        assert!(is_grammatical_surface("were") && is_grammatical_surface("is"));
    }

    #[test]
    fn disease_neoplasm_stays_mass_via_head_inheritance() {
        // REGRESSION GUARD (the pure-TUI version gapped `cause Lynch syndrome`): Lynch syndrome
        // (C1333990, T191 Neoplastic Process) has preferred name "Hereditary Nonpolyposis Colorectal
        // Cancer" — head "cancer" ∈ the uncountable list. T191 is NOT count-vetoed (diseases/neoplasms
        // ARE bare-mass in scientific prose: `cause cancer`), so head-inheritance masses it → the bare
        // object `cause Lynch syndrome` parses. This coverage is what the count-veto preserves.
        let mut mass = MassNouns::new();
        mass.insert("cancer".to_string());
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T191".to_string(),
                name: "Neoplastic Process".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C1333990".to_string(),
                tuis: vec!["T191".to_string()],
                preferred_name: "Hereditary Nonpolyposis Colorectal Cancer".to_string(),
                forms: vec!["Lynch Syndrome".to_string()],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &mass, &DropSet::new());
        assert_eq!(
            rep.mass_entries, 1,
            "T191 with uncountable head 'cancer' stays mass via head-inheritance"
        );
        assert!(doc.contains("lexicon:cat_n(umlscui:C1333990, lexicon:mass)"));
    }

    #[test]
    fn process_function_concept_is_mass_by_semantic_type() {
        // #2 (gap `arises from methylation`): `methylation` (C0025723, T044 Molecular Function) is an
        // uncountable process, but its head `methylation` is NOT in the countability list. The semantic
        // type (T044 ∈ MASS_DENOTING_TUIS) marks it mass anyway → an ADDITIVE `cat_n(C, mass)` entry,
        // so bare `arises from methylation` shifts to a mass NP. Empty countability set — the TUI alone
        // drives it.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T044".to_string(),
                name: "Molecular Function".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0025723".to_string(),
                tuis: vec!["T044".to_string()],
                preferred_name: "Methylation".to_string(),
                forms: vec!["Methylation".to_string()],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &DropSet::new());
        assert_eq!(
            rep.mass_entries, 1,
            "a T044 process concept is mass by semantic type, with no countability list"
        );
        assert!(doc.contains("lexicon:cat_n(umlscui:C0025723, lexicon:mass)"));
        assert!(doc.contains("lexicon:cat_n(umlscui:C0025723, lexicon:num_any)"));
        // count stays (additive)
    }

    #[test]
    fn non_content_concept_ships_class_but_no_common_noun_entry() {
        // `And` (C1515981, T078 Idea or Concept) reifies a conjunction. Its CLASS ships (mirror intact),
        // but NO `cat_n` common-noun entry — otherwise "and" piles into a compound instead of coordinating.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T078".to_string(),
                name: "Idea or Concept".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C1515981".to_string(),
                tuis: vec!["T078".to_string()],
                preferred_name: "And".to_string(),
                forms: vec!["And".to_string(), "and".to_string()],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &DropSet::new());
        assert_eq!(rep.non_content_skipped, 2, "both forms withheld");
        assert_eq!(rep.entries, 0, "no common-noun entry");
        assert!(
            doc.contains("class umlscui:C1515981"),
            "the concept CLASS still ships (mirror intact)"
        );
        assert!(
            !doc.contains("lexicon:cat_n(umlscui:C1515981"),
            "but no cat_n common-noun entry"
        );
    }

    #[test]
    fn mixed_type_concept_keeps_its_noun_entry() {
        // A concept typed T080 Qualitative Concept AND T023 Body Part is a THING — it keeps its noun.
        let subset = Subset {
            semantic_types: vec![SemanticType {
                tui: "T023".to_string(),
                name: "Body Part, Organ, or Organ Component".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0000001".to_string(),
                tuis: vec!["T080".to_string(), "T023".to_string()],
                preferred_name: "Some Structure".to_string(),
                forms: vec!["some structure".to_string()],
                definition: None,
                symbol: None,
            }],
        };
        let (doc, rep) = render_document(&subset, "2026AA", &MassNouns::new(), &DropSet::new());
        assert_eq!(rep.non_content_skipped, 0);
        assert!(doc.contains("lexicon:cat_n(umlscui:C0000001, lexicon:num_any)"));
    }
}
