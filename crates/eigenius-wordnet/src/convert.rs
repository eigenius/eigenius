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

//! Map WordNet synsets → Eigon lexicon **ESL** (D62 §8.7). Deterministic.
//!
//! - noun synset → `core:Class`; `@` hypernyms → `core:subclass_of` (the
//!   `entity.n.01`-rooted lattice the subsumption rule consumes).
//! - verb synset → `eigentt:Axiom`; category from the sentence frames; stage-1
//!   argument types are generic at the noun root [`ENTITY_TOP`], so the verb
//!   composes with any noun by subsumption (§8.6).
//! - adjective synset → predicative `eigentt:Axiom` (`S\NP`).
//! - each lemma → a `lexicon:LexicalEntry`; `sem_type = ⟦cat⟧` by construction
//!   (the same `⟦·⟧` the kernel gate checks), so entries are felicitous by
//!   construction and the gate is a confirmation.

use std::collections::{BTreeMap, BTreeSet};

use crate::inflect::{comparison, gerund, past_participles, third_singular, Comparison};
use crate::wndb::{Offset, Pos, Synset};

/// Sense-frequency ranks keyed by the entry's `sense` key (`wn:{lemma}.{tag}.{offset}`,
/// as [`sense_key`] forms it) → 0-based rank (sense 1 → 0). Built by
/// [`crate::import::read_sense_ranks`] from `index.<pos>` (whose per-lemma synset list
/// is frequency-sorted). An absent key ⇒ rank 0. Threaded into [`render_document`] so a
/// polysemous lemma's rarer senses get a higher `lexicon:sense_rank` (D63 §8.7 Stage B).
pub type SenseRanks = BTreeMap<String, u32>;

/// **Countability lexicon** (D62 bare-mass arguments): the set of noun lemmas that have an
/// *uncountable* (mass) sense, so a bare singular occurrence is a felicitous NP argument
/// (`mutation occurs`, `lethality matters`), the same way a bare plural is. Sourced externally
/// (Wiktionary's `Category:English uncountable nouns` ∩ the WordNet noun lemmas — see
/// `scripts/provision-countability.sh`); countability is **not** morphologically derivable
/// (`mutation`/`function` share a suffix yet differ), so it must come from a lexicon, not a rule.
/// Keys are normalized by [`norm_lemma`]. Threaded into [`render_document`] / [`render_sections`];
/// an empty set ⇒ no mass marking (every noun stays count-only, the prior behaviour).
pub type MassNouns = BTreeSet<String>;

/// Normalize a WordNet lemma to a [`MassNouns`] lookup key: lowercase, `_` → space (WordNet
/// multiword lemmas use `_`; the Wiktionary-derived list uses spaces).
fn norm_lemma(lemma: &str) -> String {
    lemma.to_lowercase().replace('_', " ")
}

/// The **entity top** (D63 §8.3, decision ii): the schema-level foundational
/// entity type (`lexicon:Entity`) that verb/adjective argument slots — and the
/// determiners' subject `E` — are typed at. WordNet's `entity.n.01`
/// (`wn:n00001740`, the noun-lattice root) is rooted here, so every imported
/// noun is `≤ lexicon:Entity` and flows into these slots by coercive subtyping.
/// Provided by the bootstrapped lexicon schema, which the import builds on.
pub const ENTITY_TOP: &str = "lexicon:Entity";

/// The namespace + schema preamble the emitted entries reference, prefixed with the
/// mandatory **WordNet 3.0 attribution**. Princeton's license requires its copyright
/// notice + disclaimer on ALL copies *including modifications*; every emitted document is
/// a WordNet derivative (it embeds glosses, lemmas, and the synset lattice), so the notice
/// rides at the top of the output to keep any emitted / redistributed artifact compliant.
pub const ESL_HEADER: &str = "\
// ════════════════════════════════════════════════════════════════════
// This file is DERIVED FROM WordNet 3.0 and embeds its content (glosses,
// lemmas, synset structure). Redistributed under the WordNet 3.0 license:
//
//   WordNet 3.0 Copyright 2006 by Princeton University. All rights reserved.
//
//   Permission to use, copy, modify and distribute this software and database
//   and its documentation for any purpose and without fee or royalty is hereby
//   granted, provided that you agree to comply with the following copyright
//   notice and statements, including the disclaimer, and that the same appear on
//   ALL copies of the software, database and documentation, including
//   modifications that you make for internal use or for distribution.
//
//   THIS SOFTWARE AND DATABASE IS PROVIDED \"AS IS\" AND PRINCETON UNIVERSITY
//   MAKES NO REPRESENTATIONS OR WARRANTIES, EXPRESS OR IMPLIED. BY WAY OF
//   EXAMPLE, BUT NOT LIMITATION, PRINCETON UNIVERSITY MAKES NO REPRESENTATIONS
//   OR WARRANTIES OF MERCHANTABILITY OR FITNESS FOR ANY PARTICULAR PURPOSE OR
//   THAT THE USE OF THE LICENSED SOFTWARE, DATABASE OR DOCUMENTATION WILL NOT
//   INFRINGE ANY THIRD PARTY PATENTS, COPYRIGHTS, TRADEMARKS OR OTHER RIGHTS.
//
//   The name of Princeton University or Princeton may not be used in advertising
//   or publicity pertaining to distribution of the software and/or database.
//   Title to copyright in this software, database and any associated
//   documentation shall at all times remain with Princeton University and
//   LICENSEE agrees to preserve same.
// ════════════════════════════════════════════════════════════════════
namespace core       = \"urn:eigenius:core\";
namespace reflection = \"urn:eigenius:reflection\";
namespace epistemic  = \"urn:eigenius:reflection:epistemic\";
namespace eigentt    = \"urn:eigenius:eigentt\";
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace measurements = \"urn:eigenius:measurements\";
namespace wn         = \"urn:eigenius:wn\";
";

/// The WordNet `lexicon:Lexicon` descriptor (D65 §3) — the stable logical identity
/// of "the WordNet lexicon", decoupled from the version-specific layer this import
/// produces. Emitted once per document; every `lexicon:LexicalEntry` binds back via
/// `lexicon:in_lexicon = lexicon:wordnet`, so a parse scope can select WordNet and
/// "available lexica" is a plain EigenQL query over `lexicon:Lexicon` instances.
pub const WORDNET_LEXICON: &str = "\
resource lexicon:wordnet : lexicon:Lexicon {
    lexicon:source   = \"WordNet 3.0, Princeton University\";
    lexicon:version  = \"3.0\";
    lexicon:language = \"en\";
    lexicon:domain   = \"general\";
    lexicon:license  = \"WordNet 3.0 License (Princeton University)\";
}
";

/// Coverage of one import run.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub noun_classes: usize,
    /// Proper-noun individuals (`@i` synsets) emitted as `EigonResource`
    /// instances of their class(es) — the NP archetype (§8.7.3).
    pub instances: usize,
    pub verb_axioms: usize,
    pub adj_axioms: usize,
    pub entries: usize,
    /// Of `entries`, the participle (`ger`/`pss`) verb-form entries (D63 §8.9 6-aux):
    /// the generated gerund + past-participle forms an auxiliary selects.
    pub participle_entries: usize,
    /// Verb synsets with no emittable frame (only predicative / clausal /
    /// control frames, or no frame) — deferred, never guessed.
    pub verbs_deferred: usize,
    /// Of `entries`, the additive **mass** noun entries (D62 countability lexicon): a second
    /// `cat_n(C, mass)` entry emitted for a lemma flagged uncountable, enabling the bare-mass
    /// argument shift. Zero when no countability lexicon is supplied.
    pub mass_entries: usize,
    /// Multiword adjective lemmas skipped because they merely restate a governed-preposition frame
    /// the base adjective already carries ([`restates_governed_frame`]).
    pub frame_duplicate_skipped: usize,
    /// Verb lemmas skipped because they are the COPULA (`be`) — grammar the closed-class bootstrap
    /// owns. WordNet's content senses of it (including a frame-6 LINKING entry over an opaque 2-place
    /// axiom) compete with the copula and re-encode `X is P` as `be(λx.P(x), X)`. Counts lemma skips,
    /// so one per (frame-kind × synset) occurrence of `be`.
    pub copula_skipped: usize,
    /// Of `entries`, the STATIVE RELATIONAL PARTICIPLE entries ([`push_stative_relational`]): the
    /// `(S[adj]\NP)/cat_pp_arg(prep)` reading of a past participle whose verb's WordNet frames NAME a
    /// governed preposition (`associated WITH`, `linked WITH`).
    pub stative_entries: usize,
}

/// The emittable categorial shapes a verb frame maps to. Higher-order shapes
/// (predicative-complement, clausal, control / raising) are not emittable at
/// stage-1 and are deferred (§8.7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FrameKind {
    Intransitive,
    Transitive,
    Ditransitive,
    /// Verb + a **single PP complement** the verb subcategorizes for ("contributes **to** cancers",
    /// "depends **on** X", "----s PP"): category `(S\NP)/cat_pp_arg`, an argument-PP whose ⟦·⟧ = Entity
    /// (D63 verb+PP frames). Same `Entity → Entity → Prop` axiom as a transitive verb — only the
    /// category differs, so the argument-marker preposition is forced and a plain transitive verb still
    /// rejects a stray `to X`. Replaces the old coarse "PP-oblique → transitive/intransitive" mapping.
    PpOblique,
    /// Clause-taking (report) verb — frame 26, "Somebody ----s that CLAUSE" (D63 §8.11
    /// 6-cl): an opaque `Prop → Entity → Prop` axiom, category `(S\NP)/cat_cp`.
    Clausal,
    /// Linking (copular) verb — WordNet frames 6/7 ("Something ----s Adjective/Noun", "Somebody
    /// ----s Adjective"): `remain`/`become`/`seem`/`appear` + a predicative adjective. An opaque
    /// `(Entity → Prop) → Entity → Prop` axiom (the verb relates the subject to the property),
    /// category `(S[dcl,fin]\NP)/(S[dcl,adj]\NP)`. Mirrors the copula `be`'s adjective complement but
    /// keeps the verb's OWN opaque relation — faithful for both veridical (`remain` ⊨ the adjective)
    /// and evidential (`seem` ⊭ it) senses without a veridicality list, exactly as the Clausal report
    /// verb keeps `Prop` opaque rather than asserting it (the copula likewise defers tense; D61). Frame
    /// 5 ("----s something Adjective", object-predicate `consider X important`) is a distinct category,
    /// still deferred.
    LinkingAdj,
    /// Essive / object-predicative — the `identify / regard / describe / classify X AS Y` construction
    /// (D63 §5.3). WordNet's frame inventory does NOT capture the `as`-complement (frame 14 is the
    /// double-object "give" frame), so this kind is not assigned by [`classify`]; it is added per-lemma
    /// for a curated essive-verb set ([`is_essive_verb`], in [`push_verb`]). Category
    /// `((S\NP)/cat_pp_arg(prep_as))/NP` — object first, then the `as`-marked predicative complement
    /// (⟦cat_pp_arg⟧ = Entity); an opaque 3-place axiom `Entity → Entity → Entity → Prop`
    /// (`identify(obj, as_obj, subj)`), the essive analogue of the ditransitive shape.
    Essive,
}

impl FrameKind {
    fn tag(self) -> &'static str {
        match self {
            FrameKind::Intransitive => "i",
            FrameKind::Transitive => "t",
            FrameKind::Ditransitive => "d",
            FrameKind::PpOblique => "p",
            FrameKind::Clausal => "c",
            FrameKind::LinkingAdj => "j",
            FrameKind::Essive => "as",
        }
    }

    /// The axiom / `sem_type` arrow — every slot generic at the noun root
    /// (stage-1; §8.7.4), so the verb composes with any noun by subsumption. The
    /// clausal report verb leads with the propositional complement (`Prop`).
    fn arrow(self) -> String {
        match self {
            FrameKind::Intransitive => format!("{ENTITY_TOP} -> Prop"),
            // Same relation shape as transitive — the PP's object is the second entity argument.
            FrameKind::Transitive | FrameKind::PpOblique => {
                format!("{ENTITY_TOP} -> {ENTITY_TOP} -> Prop")
            }
            FrameKind::Ditransitive | FrameKind::Essive => {
                format!("{ENTITY_TOP} -> {ENTITY_TOP} -> {ENTITY_TOP} -> Prop")
            }
            FrameKind::Clausal => format!("Prop -> {ENTITY_TOP} -> Prop"),
            // Linking verb: takes the predicative-adjective's denotation (`Entity → Prop`) then the
            // subject — an opaque relation between the subject and the property.
            FrameKind::LinkingAdj => format!("({ENTITY_TOP} -> Prop) -> {ENTITY_TOP} -> Prop"),
        }
    }

    /// The `lexicon:Cat` term (object-first; `⟦cat⟧` equals [`Self::arrow`]) for a given
    /// `Fin` form. NP slots are number-underspecified (`num_any`); the result sentence is
    /// declarative with the supplied finiteness — `fin` for the lemma entry, `ger` / `pss`
    /// for the participle entries (D63 §5.1, §8.9 6-aux). Finiteness is erased by `⟦·⟧`,
    /// so [`Self::arrow`] (the `sem_type`) is unchanged across forms.
    fn cat(self, fin: &str, subj_num: &str) -> String {
        // Subject slot carries the agreement number (D63 §8.10 6-agr: `sg` for the
        // 3sg `fin`, `pl` for the plural-finite, `num_any` for `bse`/`ger`/`pss` where
        // the auxiliary supplies agreement); object slots stay `num_any`.
        let subj = format!("lexicon:cat_np({ENTITY_TOP}, lexicon:{subj_num})");
        let obj = format!("lexicon:cat_np({ENTITY_TOP}, lexicon:num_any)");
        let s = format!("lexicon:cat_s(lexicon:dcl, lexicon:{fin})");
        match self {
            FrameKind::Intransitive => format!("lexicon:bwd(lexicon:m_all, {s}, {subj})"),
            FrameKind::Transitive => format!("lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, {s}, {subj}), {obj})"),
            FrameKind::Ditransitive => {
                format!("lexicon:fwd(lexicon:m_all, lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, {s}, {subj}), {obj}), {obj})")
            }
            // Essive `((S\NP)/cat_pp_arg(prep_as))/NP` — the object NP binds first (adjacent to the
            // verb), then the `as`-marked predicative complement `cat_pp_arg(prep_as)` (⟦·⟧ = Entity).
            // Same 3-place `Entity → Entity → Entity → Prop` axiom as the ditransitive; the distinct
            // `prep_as` marker forces the `as`, so a plain transitive verb still rejects a stray `as Y`.
            FrameKind::Essive => format!(
                "lexicon:fwd(lexicon:m_all, lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, {s}, {subj}), lexicon:cat_pp_arg(lexicon:prep_as)), {obj})"
            ),
            // Argument-PP verb: `(S\NP)/cat_pp_arg(prep_any)` — the object arrives through a transparent
            // argument-marker preposition (`to`/`on`/…). Distinct from a bare NP so the preposition is
            // forced; `⟦cat_pp_arg⟧ = Entity`, so the sem_type equals the transitive one above. WordNet's
            // PP frames (4, 22) are preposition-AGNOSTIC (no governed prep recorded), so the verb takes the
            // `prep_any` wildcard — it accepts any marker (C3-precision; the specific-prep gate applies to
            // gloss-governed ADJECTIVES, where WordNet's gloss does carry the governance).
            FrameKind::PpOblique => {
                format!(
                    "lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, {s}, {subj}), lexicon:cat_pp_arg(lexicon:prep_any))"
                )
            }
            // Clause-taking: `(S\NP)/cat_cp` — the complement is an embedded clause.
            FrameKind::Clausal => format!("lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, {s}, {subj}), lexicon:cat_cp)"),
            // Linking (copular) verb: `(S[dcl,fin]\NP)/(S[dcl,adj]\NP)` — consumes a predicative-
            // adjective VP and yields a finite VP, like the copula `be`'s `adj` complement.
            FrameKind::LinkingAdj => format!(
                "lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, {s}, {subj}), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), {obj}))"
            ),
        }
    }
}

/// Map a WordNet sentence frame (1–35; `wninput(5WN)`) to an emittable kind, or
/// `None` for the **deferred** higher-order frames:
///   - 6, 7 — subject-predicate linking verb (`----s Adjective`) → [`FrameKind::LinkingAdj`];
///   - 5 — object-predicate small clause (`----s something Adjective`, `consider X important`) — a
///     distinct `((S\NP)/adj)/NP` category, still deferred;
///   - 26, 29, 34 — clausal complement (`that` / `whether CLAUSE`, only 26 emitted);
///   - 24, 25, 28, 30, 32, 33, 35 — control / raising (INFINITIVE / V-ing).
///
/// **Single-PP-complement frames** — the verb subcategorizes for one PP ("----s to X" 12/27, "is
/// ----ing PP" 4, "Somebody ----s PP" 22) — map to [`FrameKind::PpOblique`] (`(S\NP)/cat_pp_arg`). This
/// replaces the former coarse handling (12/27 → transitive with the preposition dropped; 4/22 →
/// intransitive with the PP dropped). **Object+PP** frames (13, 20, 21) and other PP shapes stay coarse
/// for now — a follow-up (`((S\NP)/cat_pp_arg)/NP`). Frame 14 is left as this importer already classifies
/// it.
///
/// Frames **22 and 23 were swapped** here until 2026-08-02. `doc/man/wninput.5` (vendored at
/// `references/WordNet-3.0/doc/man/wninput.5`, line 212) reads `22  Somebody ----s PP` and
/// `23  Somebody's (body part) ----s`, so 22 is the PP frame and 23 is an intransitive. The inversion
/// dropped 22's PP — leaving `give rise` with no argument slot for its `to`-PP, so the PP fell to adjunct
/// position and an adjunct escaped a finite `that`-clause onto the matrix subject — and handed 23 a
/// spurious oblique slot.
fn classify(frame: u8) -> Option<FrameKind> {
    match frame {
        1 | 2 | 3 | 23 => Some(FrameKind::Intransitive),
        4 | 12 | 22 | 27 => Some(FrameKind::PpOblique),
        8 | 9 | 10 | 11 | 13 | 20 | 21 => Some(FrameKind::Transitive),
        14 | 15 | 16 | 17 | 18 | 19 | 31 => Some(FrameKind::Ditransitive),
        6 | 7 => Some(FrameKind::LinkingAdj), // "----s Adjective" — copular/linking verb (D63 §8.5)
        26 => Some(FrameKind::Clausal),       // "Somebody ----s that CLAUSE" (D63 §8.11 6-cl)
        _ => None, // 5 object-predicate (`consider X important`); 29,34 whether; 24,25,28,30,32,33,35 control/raising
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `<tag><offset>` — the local name of a noun class / adjective predicate.
fn local(syn: &Synset) -> String {
    format!("{}{}", syn.pos.tag(), syn.offset)
}

fn sense_key(syn: &Synset, lemma: &str) -> String {
    format!(
        "wn:{}.{}.{}",
        lemma.replace(' ', "_"),
        syn.pos.tag(),
        syn.offset
    )
}

/// Emit one `lexicon:LexicalEntry` block. `entry_id` and `sem` are local names
/// (under `wn:`); `cat` / `sem_type` are `type_expr` bodies.
#[allow(clippy::too_many_arguments)]
fn push_entry(
    buf: &mut String,
    entry_id: &str,
    form: &str,
    cat: &str,
    sem: &str,
    sem_type: &str,
    sense: &str,
    ranks: &SenseRanks,
) {
    // CLOSED-CLASS SURFACE: the bootstrap owns this word's grammatical reading, so WordNet must not
    // seed a content entry on it (`eigenius_kernel::dcg::closed_class`, the list both importers share).
    // WordNet's collisions here are element-symbol / acronym homonyms — `As` is BOTH arsenic
    // (14629149) and American Samoa (08991878), `Be` beryllium, `In` indium — which let a function word
    // pile into a compound noun: "We evaluated MSI **as** a biomarker for WRN dependency" parsed as a
    // compound "a Microsatellite-Instability *As* dependency" (19 structural readings, the WRN page's
    // worst unit). The single choke point for all emission sites, so every POS is covered at once; the
    // synset's axioms still ship (only the ENTRY is withheld, mirroring the UMLS importer).
    if eigenius_kernel::dcg::closed_class::is_closed_class_surface(form) {
        return;
    }
    // Sense-frequency rank (D63 §8.7 Stage B): emit `lexicon:sense_rank` only when it is
    // non-zero (rank 0 = the most-frequent sense, and the parser's default — so the
    // overwhelming majority of entries stay rank-free, keeping the ESL lean).
    let rank_line = match ranks.get(sense).copied().unwrap_or(0) {
        0 => String::new(),
        r => format!("    lexicon:sense_rank = {r};\n"),
    };
    buf.push_str(&format!(
        "resource wn:{entry_id} : lexicon:LexicalEntry {{\n\
         \x20   lexicon:form     = \"{form}\";\n\
         \x20   lexicon:cat      = type_expr( {cat} );\n\
         \x20   lexicon:sem      = wn:{sem};\n\
         \x20   lexicon:sem_type = type_expr( {sem_type} );\n\
         \x20   lexicon:sense    = \"{sense}\";\n\
         {rank_line}\
         \x20   lexicon:grade    = epistemic:declared;\n\
         \x20   lexicon:in_lexicon = lexicon:wordnet;\n\
         }}\n\n",
        form = esc(form),
    ));
}

/// Noun synset → a `core:Class` (with `subclass_of` from `@`) + one `N` entry
/// per lemma.
fn push_noun(
    buf: &mut String,
    syn: &Synset,
    rep: &mut Report,
    noun: &BTreeMap<Offset, &Synset>,
    ranks: &SenseRanks,
    mass: &MassNouns,
) {
    // `@` hypernyms → `subclass_of`, each resolved to the nearest CLASS. WordNet has
    // class synsets whose `@` parent is an INSTANCE (British_West_Indies `@` West_Indies
    // `@i` archipelago) — a class can't be a subclass of an individual, so climb the
    // instance to its class ([`nearest_classes`]).
    let mut parent_offsets: Vec<Offset> = Vec::new();
    let mut seen: BTreeSet<Offset> = BTreeSet::new();
    for h in &syn.hypernyms {
        nearest_classes(h, noun, &mut parent_offsets, &mut seen);
    }
    let parents: Vec<String> = parent_offsets.iter().map(|h| format!("wn:n{h}")).collect();
    // A hypernym-less noun (WordNet's root `entity.n.01`) is rooted at the schema
    // entity top so the whole noun lattice sits under `lexicon:Entity` (D63 §8.3
    // ii); all other nouns parent at their `@` hypernyms.
    let header = if parents.is_empty() {
        format!("class wn:{} : {ENTITY_TOP} {{", local(syn))
    } else {
        format!("class wn:{} : {} {{", local(syn), parents.join(", "))
    };
    buf.push_str(&format!(
        "{header}\n    description = \"{}\";\n}}\n\n",
        esc(&syn.gloss)
    ));
    rep.noun_classes += 1;
    // `cat_n` carries the noun's own class as its (denotation-erased) type index
    // — load-bearing for polymorphic determiners (D63 §8.2).
    let cat = format!("lexicon:cat_n(wn:{}, lexicon:num_any)", local(syn));
    // The mass-Num cat for the same class: emitted ADDITIVELY (alongside the count entry) for a
    // lemma the countability lexicon flags uncountable, so a bare singular occurrence shifts to an
    // NP argument (D62 `bare_mass_nps`) WITHOUT removing count uses — `mutation` is tagged
    // uncountable yet `a mutation`/`three mutations` must still parse (the count entry handles them).
    let mass_cat = format!("lexicon:cat_n(wn:{}, lexicon:mass)", local(syn));
    for (i, lemma) in syn.words.iter().enumerate() {
        push_entry(
            buf,
            &format!("e_{}_{i}", local(syn)),
            lemma,
            &cat,
            &local(syn),
            "Set",
            &sense_key(syn, lemma),
            ranks,
        );
        rep.entries += 1;
        if mass.contains(&norm_lemma(lemma)) {
            push_entry(
                buf,
                &format!("e_{}_{i}_mass", local(syn)),
                lemma,
                &mass_cat,
                &local(syn),
                "Set",
                &sense_key(syn, lemma),
                ranks,
            );
            rep.entries += 1;
            rep.mass_entries += 1;
        }
    }
}

/// Instance synset (`@i`) → a proper-noun **individual** (the NP archetype,
/// §8.7.3): an `EigonResource` instance of its class(es), **not** a class. Its
/// `@i` (and any rare co-occurring `@`) targets become the resource's types — an
/// individual *is an instance of* all of them. Each lemma → an `NP` entry
/// one `NP` entry per `(class, lemma)` — `cat_np(C, num_any)`, `sem` = this
/// resource — so a multi-class individual is usable in each class's typing context
/// (now admissible via the check-mode resource-inhabitation rule, #91). The other
/// classes also stay on the resource.
fn push_instance(
    buf: &mut String,
    syn: &Synset,
    rep: &mut Report,
    noun: &BTreeMap<Offset, &Synset>,
    ranks: &SenseRanks,
) {
    // Types: `@i` first (the instance-hypernyms), then any rare plain `@` — each
    // resolved to the nearest CLASS. WordNet chains instances (`@i`) — Paternoster
    // `@i` Lord's_Prayer `@i` prayer — but an individual cannot be typed by another
    // individual (the parent was emitted as a `resource`, not a `class`). So climb
    // each `@i`/`@` target to the nearest class ([`nearest_classes`]); the
    // intermediate instances collapse into co-referential individuals of that class.
    let mut class_offsets: Vec<Offset> = Vec::new();
    let mut seen: BTreeSet<Offset> = BTreeSet::new();
    for t in syn.instance_of.iter().chain(syn.hypernyms.iter()) {
        nearest_classes(t, noun, &mut class_offsets, &mut seen);
    }
    let classes: Vec<String> = class_offsets.iter().map(|h| format!("wn:n{h}")).collect();
    assert!(
        !classes.is_empty(),
        "push_instance requires a non-empty instance_of"
    );
    buf.push_str(&format!(
        "resource wn:{} : {} {{\n    core:description = \"{}\";\n}}\n\n",
        local(syn),
        classes.join(", "),
        esc(&syn.gloss),
    ));
    rep.instances += 1;
    for (ci, class) in classes.iter().enumerate() {
        // Proper-noun individuals are singular (D63 §8.10 6-agr) → they take the 3sg verb.
        let cat = format!("lexicon:cat_np({class}, lexicon:sg)");
        for (li, lemma) in syn.words.iter().enumerate() {
            push_entry(
                buf,
                &format!("e_{}_{ci}_{li}", local(syn)),
                lemma,
                &cat,
                &local(syn),
                class,
                &sense_key(syn, lemma),
                ranks,
            );
            rep.entries += 1;
        }
    }
}

/// Resolve a hypernym/`@i` target to the nearest CLASS offset(s), appending to `out`
/// (deduped via `seen`, deterministic order, cycle-guarded). A target that is itself
/// an instance (non-empty `instance_of` in the noun index) is climbed through its own
/// `@i`/`@`; a class (empty `instance_of`) — or an unknown offset, treated as a class —
/// is emitted. This collapses WordNet's instance-of-instance `@i` chains so an
/// individual is always typed by a class, never by another individual.
fn nearest_classes(
    start: &Offset,
    noun: &BTreeMap<Offset, &Synset>,
    out: &mut Vec<Offset>,
    seen: &mut BTreeSet<Offset>,
) {
    if !seen.insert(start.clone()) {
        return;
    }
    match noun.get(start) {
        Some(s) if !s.instance_of.is_empty() => {
            for t in s.instance_of.iter().chain(s.hypernyms.iter()) {
                nearest_classes(t, noun, out, seen);
            }
        }
        _ => out.push(start.clone()),
    }
}

/// Apply a single-word inflector to the **head** word of a (possibly multiword) verb
/// lemma, keeping the remainder (particle / light-verb tail): "depend on" → "depending
/// on", "take a breath" → "taken a breath".
fn inflect_head(lemma: &str, f: impl Fn(&str) -> String) -> String {
    match lemma.split_once(' ') {
        Some((head, rest)) => format!("{} {rest}", f(head)),
        None => f(lemma),
    }
}

/// The past-participle surface(s) of a (possibly multiword) verb lemma — [`past_participles`]
/// on the head, remainder kept ("depend on" → "depended on").
fn head_pps(lemma: &str) -> Vec<String> {
    match lemma.split_once(' ') {
        Some((head, rest)) => past_participles(head)
            .into_iter()
            .map(|p| format!("{p} {rest}"))
            .collect(),
        None => past_participles(lemma),
    }
}

/// Verb synset → an `eigentt:Axiom` + entries **per distinct emittable frame
/// kind** (a verb with both intransitive and transitive frames yields both — its
/// alternations). Per lemma, emits the full verb paradigm — **base** (`bse`, the lemma
/// surface, selected by do-support / modals), **finite 3sg** (`fin`, the generated
/// "affects", which heads a declarative), **present participle** (`ger`, progressive),
/// and **past participle(s)** (`pss`, perfect/passive) — all the *same* axiom (finiteness
/// is erased by `⟦·⟧`), differing only in the result clause's `Fin` feature (D63 §8.9
/// 6-aux). Emitting `bse` distinct from `fin` is what makes do-support, polar/object-wh
/// questions, negation, and modals fire on imported verbs (not just the hand demo), and
/// fixes the former base-as-`fin` mistag (bare "affect" no longer parses as finite).
/// Returns `false` (deferred) when no frame is emittable.
/// Verbs that take an object + an `as`-marked predicative complement — the essive construction
/// (`identify / regard / describe X as Y`), which WordNet's frame inventory does not encode (D63 §5.3).
/// A curated, high-precision set: each canonically licenses `V NP as NP`. Any synset containing one of
/// these lemmas gets an ADDITIONAL [`FrameKind::Essive`] category (in [`push_verb`]) on top of its
/// frame-derived categories. British + American spellings both listed.
/// Verbs that take an oblique PP complement WordNet's frame inventory does not record — the verb
/// analogue of `adjective-frames.tsv`. WordNet's PP frames (4/12/23/27) are the only route to
/// [`FrameKind::PpOblique`], and a verb whose synsets carry only transitive frames therefore cannot
/// take a marked complement at all.
///
/// Witnessed 2026-07-26 on `compare`: its synsets carry frames 8/9/10/11 (transitive) only, so
/// `compared` had `(S[pss]\NP)/NP` but NO `(S[pss]\NP)/cat_pp_arg(...)`. "compared to MSS cell
/// lines" therefore had no participial derivation and collapsed into the NOUN reading
/// (`wn:compare.n.04746842`, *comparison*) — which is why the reference page's worst unit (204
/// skeletons) heads its essive NP on *comparison* in 84 readings and has NO fully-correct reading.
///
/// Additive, like [`ESSIVE_VERBS`]: a synset containing one of these lemmas gets an EXTRA
/// [`FrameKind::PpOblique`] category on top of its frame-derived ones. The emitted marker is
/// `prep_any` (the wildcard that meets any specific marker), matching how WordNet's own PP frames are
/// treated — the governed preposition is not recorded per-verb here, only the fact that an oblique
/// complement is licensed.
const PP_OBLIQUE_VERBS: &[&str] = &["compare", "contrast"];

/// Whether `lemma` licenses an oblique PP complement per the curated set ([`PP_OBLIQUE_VERBS`]).
fn is_pp_oblique_verb(lemma: &str) -> bool {
    PP_OBLIQUE_VERBS.contains(&lemma.to_ascii_lowercase().as_str())
}

const ESSIVE_VERBS: &[&str] = &[
    "identify",
    "regard",
    "view",
    "perceive",
    "reckon",
    "construe",
    "misconstrue",
    "interpret",
    "misinterpret",
    "understand",
    "conceive",
    "envisage",
    "envision",
    "recognize",
    "recognise",
    "accept",
    // --- Group 2: Categorization & Naming ---
    "classify",
    "define",
    "categorize",
    "categorise",
    "label",
    "brand",
    "designate",
    "qualify",
    "disqualify",
    // --- Group 3: Depiction, Framing & Public Statement ---
    "describe",
    "characterize",
    "characterise",
    "portray",
    "depict",
    "represent",
    "frame",
    "cite",
    "report",
    "cast",
    "denounce",
    "hail",
    "praise",
    "condemn",
    "advertise",
    "market",
    // --- Group 4: Role Designation & Appointment ---
    "appoint",
    "select",
    "choose",
    "elect",
    "hire",
    "install",
    "establish",
    // --- Group 5: Utilization & Operational Essives ---
    "utilize",
    "utilise",
    "employ",
    "adopt",
    // --- Group 6: Appraisal & Assessment (verb-dominant; `evaluate X as Y` — the WRN `We evaluated
    //     MSI as a biomarker for WRN dependency` case, which gapped without this) ---
    "evaluate",
    "assess",
    "deem",
    // --- Group 7: High-Frequency / High-Risk (Apply low verb-prior weights) ---
    //   "see",   // Danger: Hyper-frequent. Must not override standard transitive "see [NP]".
    //   "use",   // Danger: Hyper-frequent noun/verb.
    //   "class", // Danger: Hyper-frequent noun ("the python class").
    //   "treat"  // Danger: Common noun/regular transitive ("treat the patient").
];

/// Whether `lemma` is in the curated essive-verb set ([`ESSIVE_VERBS`]) — case-insensitive.
fn is_essive_verb(lemma: &str) -> bool {
    let l = lemma.to_ascii_lowercase();
    ESSIVE_VERBS.contains(&l.as_str())
}

/// A WordNet sentence frame that NAMES a governed preposition → that preposition, for the STATIVE
/// RELATIONAL PARTICIPLE rule ([`push_stative_relational`]).
///
/// The full domain `wninput(5WN)` records is
///
/// ```text
/// 15 to | 16 from | 17 with | 18 of | 19 on | 31 with
/// ```
///
/// (frame 14, "Somebody ----s somebody something", is the true double-object and names none). Only
/// **17 and 31** are shipped. The other four are held back on a MEASUREMENT, not a guess: the full
/// rule over all five prepositions (378 synsets / 844 entries) FAILS coverage, `grammar-gap 1`,
/// bisected to frame 19 alone and then to ONE synset (`set.v.01115006`). Its `set` entry EVICTS
/// `cat_n(n05674584, mass)` from the `sets` leaf of «These data sets are project Achilles and project
/// DRIVE.», which then has no parse — because the per-entry sense cap (`dcg::parse::seed`) is keyed
/// per SENSE but truncates per ENTRY, and with both entries unranked the emission order decides.
/// Frame 15 (`to`, 185 synsets) is completely INERT. So the blocker is the sense cap, which is a
/// separate structural finding; widen this set once it is fixed.
fn stative_prep(frame: u8) -> Option<&'static str> {
    match frame {
        17 | 31 => Some("with"),
        _ => None,
    }
}

/// Verb synset → the **stative relational participle** entries, additively on top of the
/// frame-derived verbal categories.
///
/// A verb whose frames name a governed preposition also lexicalises a participial RELATION —
/// `associated with`, `linked with` — that is stative: a two-place fact holding between subject and
/// relatum, with no agent who performed the act. It is categorially an ADJECTIVE taking a PP
/// argument, which the existing copula already consumes:
///
/// ```text
/// (S[dcl,adj]\NP)/cat_pp_arg(prep_X)   over a 2-place axiom   `wn:v{off}_rel`
/// ```
///
/// so this needs no new copula entry and no ontology change. [`FrameKind::Essive`]
/// (`((S\NP)/cat_pp_arg(prep_as))/NP`) is the same template; the machinery already existed and simply
/// was not applied to the frames that name a preposition.
///
/// WHY IT IS NEEDED. [`classify`] collapses 14|15|16|17|18|19|31 into one preposition-less
/// [`FrameKind::Ditransitive`] `((S\NP)/NP)/NP` and DISCARDS the preposition the frame names, so the
/// relatum could only ever attach as a free ADJUNCT — `And(associated(x), prep_with(x, r))` rather
/// than one saturated predication. The adjectival route cannot cover it either: [`governed_preposition`]
/// is reached only from [`push_adj`], over the words of ADJECTIVE synsets, and `associated` is not a
/// WordNet adjective lemma (`index.adj` 0, unlike `dependent`/`essential`/`concordant`, all 1), so the
/// `associated<TAB>with` row in `adjective-frames.tsv` never fires.
///
/// The sem is the axiom ITSELF, unwrapped: `⟦(S[adj]\NP)/cat_pp_arg⟧ = Entity(relatum) →
/// Entity(subject) → Prop`, so the category's own denotation order puts the COMPLEMENT first and the
/// SUBJECT second — the house convention throughout (`is_passive` applies `TV(p, a)`, patient then
/// agent). A flipping lambda would read backwards in every gloss.
fn push_stative_relational(buf: &mut String, syn: &Synset, rep: &mut Report, ranks: &SenseRanks) {
    // One preposition per synset, chosen by the LOWEST-NUMBERED naming frame so the result does not
    // depend on frame order in `data.verb`.
    let Some(prep) = syn
        .frames
        .iter()
        .copied()
        .filter(|&f| stative_prep(f).is_some())
        .min()
        .and_then(stative_prep)
    else {
        return;
    };
    let off = &syn.offset;
    buf.push_str(&format!(
        "axiom wn:v{off}_rel : {ENTITY_TOP} -> {ENTITY_TOP} -> Prop desc: \"{} (stative relational participle; governs `{prep}`)\"\n\n",
        esc(&syn.gloss)
    ));
    rep.verb_axioms += 1;
    let sem = format!("v{off}_rel");
    let arrow = format!("{ENTITY_TOP} -> {ENTITY_TOP} -> Prop");
    let cat = format!(
        "lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np({ENTITY_TOP}, lexicon:num_any)), lexicon:cat_pp_arg({}))",
        prep_ctor(prep)
    );
    for (i, lemma) in syn.words.iter().enumerate() {
        // The copula is grammar, not a content verb — same per-LEMMA skip as the verbal paradigm.
        if is_copula_lemma(lemma) {
            rep.copula_skipped += 1;
            continue;
        }
        let sense = sense_key(syn, lemma);
        for (k, pp) in head_pps(lemma).iter().enumerate() {
            push_entry(
                buf,
                &format!("e_v{off}_rel_{i}_p{k}"),
                pp,
                &cat,
                &sem,
                &arrow,
                &sense,
                ranks,
            );
            rep.entries += 1;
            rep.participle_entries += 1;
            rep.stative_entries += 1;
        }
    }
}

fn push_verb(buf: &mut String, syn: &Synset, rep: &mut Report, ranks: &SenseRanks) -> bool {
    let mut kinds: std::collections::BTreeSet<FrameKind> =
        syn.frames.iter().filter_map(|&f| classify(f)).collect();
    // Essive (`identify / regard / describe X as Y`) is NOT a WordNet frame (its inventory has no
    // `as`-complement), so add it per-lemma for the curated essive set (D63 §5.3) — additively, on top
    // of the synset's frame-derived categories.
    if syn.words.iter().any(|w| is_essive_verb(w)) {
        kinds.insert(FrameKind::Essive);
    }
    // Same shape for the oblique-PP complement WordNet does not record ("compared TO X") — see
    // `PP_OBLIQUE_VERBS`.
    if syn.words.iter().any(|w| is_pp_oblique_verb(w)) {
        kinds.insert(FrameKind::PpOblique);
    }
    if kinds.is_empty() {
        rep.verbs_deferred += 1;
        return false;
    }
    // Additive: the STATIVE RELATIONAL PARTICIPLE for a frame that names a governed preposition. Safe
    // after the early return — every frame in `stative_prep`'s domain also classifies (all of
    // 15|16|17|18|19|31 map to `Ditransitive`), so `kinds` is never empty when it fires.
    push_stative_relational(buf, syn, rep, ranks);
    let off = &syn.offset;
    for kind in kinds {
        let tag = kind.tag();
        // The axiom is the verb sense's denotation (entries' `lexicon:sem` names it); carry the
        // synset gloss as its `core:description` so the concept-description text index makes verb
        // senses searchable for OOV grounding, symmetric with the noun classes (D63 §6a index c).
        buf.push_str(&format!(
            "axiom wn:v{off}_{tag} : {} desc: \"{}\"\n\n",
            kind.arrow(),
            esc(&syn.gloss)
        ));
        rep.verb_axioms += 1;
        let sem = format!("v{off}_{tag}");
        let arrow = kind.arrow();
        let cat_bse = kind.cat("bse", "num_any");
        let (cat_fin_sg, cat_fin_pl) = (kind.cat("fin", "sg"), kind.cat("fin", "pl"));
        let (cat_ger, cat_pss) = (kind.cat("ger", "num_any"), kind.cat("pss", "num_any"));
        // Finite SIMPLE PAST ("HeLa affected BRCA1"): `fin` (a finite declarative root) with a
        // `num_any` subject — English past tense has NO number agreement ("it/they affected"). This
        // is what makes the WRN page's past-tense narrative parse at all; without it only present
        // (3sg/pl) and the participles existed.
        let cat_fin_past = kind.cat("fin", "num_any");
        for (i, lemma) in syn.words.iter().enumerate() {
            // The COPULA is grammar, not a content verb: skip WordNet's `be` senses (D63, the WRN
            // page's worst unit). WordNet gives `be` 8 verb synsets, and `02604760` "have the quality
            // of being" carries frame 6 → `FrameKind::LinkingAdj`, so it emits a linking entry
            // `(S[dcl,fin]\NP)/(S[dcl,adj]\NP)` over an OPAQUE 2-place axiom. That re-encodes "X is P"
            // as `be(λx.P(x), X)` — destroying the copula's transparency, since `X is P` **is** `P(X)`
            // — and competes with the closed-class copula that already covers every inflection
            // (`ontologies/lexicon/closed-class.esl`). Measured: that opaque-`be` family plus a
            // type-raising artifact riding on it accounted for 8 of the 16 structural readings of
            // "These classifications were highly concordant with … and with …". Per-LEMMA, not
            // per-synset: `be`'s synsets also carry legitimate content lemmas (`follow`, `live`,
            // `equal`, `cost`), which keep their entries. `have`/`do` are NOT skipped — they are
            // genuine content verbs on this corpus ("this state has frequent … mutations").
            if is_copula_lemma(lemma) {
                rep.copula_skipped += 1;
                continue;
            }
            let sense = sense_key(syn, lemma);
            // Base form — the lemma surface (do-support / modal complement; num_any).
            push_entry(
                buf,
                &format!("e_v{off}_{tag}_{i}_b"),
                lemma,
                &cat_bse,
                &sem,
                &arrow,
                &sense,
                ranks,
            );
            rep.entries += 1;
            // Finite 3sg ("affects") — SINGULAR subject (D63 §8.10 6-agr).
            let fin = inflect_head(lemma, third_singular);
            push_entry(
                buf,
                &format!("e_v{off}_{tag}_{i}"),
                &fin,
                &cat_fin_sg,
                &sem,
                &arrow,
                &sense,
                ranks,
            );
            rep.entries += 1;
            // Finite plural ("affect", = the lemma surface) — PLURAL subject (6-agr):
            // heads a clause with a plural/coordinated subject. Distinct from `bse`.
            push_entry(
                buf,
                &format!("e_v{off}_{tag}_{i}_fp"),
                lemma,
                &cat_fin_pl,
                &sem,
                &arrow,
                &sense,
                ranks,
            );
            rep.entries += 1;
            // Present participle — progressive ("is affecting"); always regular.
            let ger = inflect_head(lemma, gerund);
            push_entry(
                buf,
                &format!("e_v{off}_{tag}_{i}_g"),
                &ger,
                &cat_ger,
                &sem,
                &arrow,
                &sense,
                ranks,
            );
            rep.entries += 1;
            rep.participle_entries += 1;
            // Past participle(s) — perfect/passive ("has/is affected"); table-or-regular.
            for (k, pp) in head_pps(lemma).iter().enumerate() {
                let id = format!("e_v{off}_{tag}_{i}_p{k}");
                push_entry(buf, &id, pp, &cat_pss, &sem, &arrow, &sense, ranks);
                rep.entries += 1;
                rep.participle_entries += 1;
            }
            // Finite SIMPLE PAST — the past-tense surface heading a declarative ("affected"). Reuses
            // the past-participle surface(s): correct for regular verbs and the many irregulars where
            // past = participle ("found", "led", "said"); the `went`/`gone` class (past ≠ participle)
            // is a known edge — its true past surface isn't emitted (a follow-on irregular-past table).
            for (k, pp) in head_pps(lemma).iter().enumerate() {
                let id = format!("e_v{off}_{tag}_{i}_fpast{k}");
                push_entry(buf, &id, pp, &cat_fin_past, &sem, &arrow, &sense, ranks);
                rep.entries += 1;
            }
        }
    }
    true
}

/// Adjective synset → a predicative `eigentt:Axiom` (`S\NP`) + entries.
/// Emit a `lexicon:SemTerm` resource (a lambda the gradable-adjective entries reference).
fn push_sem_term(buf: &mut String, id: &str, term: &str) {
    buf.push_str(&format!(
        "resource wn:{id} : lexicon:SemTerm {{\n    lexicon:term = type_expr( {term} );\n}}\n\n"
    ));
}

/// The predicative adjective category `S[dcl,adj]\NP` (requires the copula; D63 §8.5 3a).
/// Whether `lemma` is the COPULA — grammar the closed-class bootstrap owns, so WordNet's content verb
/// senses of it must not be emitted (see the skip in [`push_verb`]). Only `be` itself: the importer
/// derives every inflection (`is`/`are`/`was`/`were`) from the lemma, so skipping the lemma removes them
/// all. `being` is not listed because it is not a verb LEMMA here (and is a legitimate common noun).
fn is_copula_lemma(lemma: &str) -> bool {
    lemma.trim().eq_ignore_ascii_case("be")
}

fn adj_cat() -> String {
    format!("lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np({ENTITY_TOP}, lexicon:num_any))")
}

/// Curated adjective **subcategorization frames** (lemma → governed preposition) — the frame-acquisition
/// source for [`governed_preposition`] when WordNet's gloss yields none (low-recall: it needs the lemma
/// followed by its prep in its OWN gloss, missing e.g. "dependent" → "on"). Embedded at compile time
/// (`include_str!`) and parsed once; the high-confidence output an LLM proposer gives for a gradable
/// adjective's frame (offline generation is the scale path). Crate-local (`crates/eigenius-wordnet/
/// adjective-frames.tsv`) so it is embeddable inside the Docker build context (a sibling of the
/// runtime-arg `experiments/lexicon-align/drops.json`/`merges.json`, which are read at import instead).
fn adjective_frames() -> &'static BTreeMap<String, String> {
    static FRAMES: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    FRAMES.get_or_init(|| {
        include_str!("../adjective-frames.tsv")
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with('#') {
                    return None;
                }
                let mut f = l.split('\t');
                Some((
                    f.next()?.trim().to_lowercase(),
                    f.next()?.trim().to_string(),
                ))
            })
            .collect()
    })
}

/// Whether `lemma` is a **multiword adjective that merely restates a governed-preposition frame** —
/// `X P` where the base adjective `X` is already known to govern `P` ([`adjective_frames`]).
///
/// Such a lemma is REDUNDANT with the compositional analysis and competes destructively with it for
/// the same span. WordNet lists `dependent on` as its own adjective lemma, sole sense `a00555859`
/// (`contingent`), beside the base `dependent` whose sense 1 `a00725772` ("relying on or requiring a
/// person or thing for support") already carries a `cat_pp_arg(prep_on)` frame from this very table.
/// So the span "dependent on WRN" has two analyses: the compositional relational one, and the MWE —
/// which SWALLOWS THE PREPOSITION and leaves the PP's object stranded as a bare noun.
///
/// Measured on the WRN page: that stranding is what let «The lines from rare lineages were less
/// dependent on WRN.» parse as `is_a(the line …, Σ:WRN-protein. And(contingent, less))` — asserting a
/// cell line IS a WRN protein — while its correct comparative reading was lost. Six other hypotheses
/// for that regression were tried and refuted; this is the one the evidence supports.
///
/// The gate is DELIBERATELY NARROW and fails safe: the drop fires only where the base adjective's
/// governance of that exact preposition is KNOWN, so genuine idioms — `all in`, `boxed in`, `agreed
/// upon`, `contingent on` — are untouched, because their bases are not gloss-governed for those
/// prepositions. Against WordNet 3.0 it removes exactly ONE lemma today (`dependent on`) out of 57
/// multiword prepositional adjectives, and it widens automatically as `adjective-frames.tsv` grows —
/// that file is the frame-acquisition source, so a new frame retires its own MWE duplicate.
///
/// Same discipline as the closed-class surface list and `GRAMMATICAL_SURFACES`: do not seed a lexical
/// entry that merely restates a grammatical relation the grammar already builds.
fn restates_governed_frame(lemma: &str) -> bool {
    let Some((base, prep)) = lemma.rsplit_once(' ') else {
        return false;
    };
    adjective_frames()
        .get(&base.to_lowercase())
        .is_some_and(|p| p.eq_ignore_ascii_case(prep))
}

/// The preposition governed by a relational gradable adjective, derived from its WordNet **gloss**
/// (C3, d63-comparative-phrasal.md §5.3 — WordNet has no structured subcat frame, so the gloss is the
/// only WordNet-internal signal). Two patterns, most-confident first:
///   1. WordNet's explicit ``followed by `PREP'`` convention (67 adj synsets — `proportional`:
///      *"usually followed by `to'"*).
///   2. the **lemma itself** immediately followed by a preposition in the gloss/examples
///      (`proportional to the crime`, `she is addicted to chocolate`). Keying on the lemma (not any
///      word) avoids the verb+prep noise of examples (`spoke in`, `came to`) and gives the right
///      per-lemma preposition within one synset (`addicted`→`to`, `dependent`→`on`).
///
/// `None` ⇒ no governance signal → a NON-relational bare measure (C1). Drives the relational emission
/// in `push_adj` (a 2-place `deg_rel` + a `cat_measure/cat_pp_arg` reading; the bare 1-place forms stay
/// for the ground-less reading — two independent measures, no optional-ground shift needed).
fn governed_preposition(gloss: &str, lemma: &str) -> Option<String> {
    const PREPS: &[&str] = &[
        "to", "on", "in", "with", "from", "for", "at", "upon", "about", "against", "into",
    ];
    // (1) explicit ``followed by `PREP'``.
    if let Some(rest) = gloss.split("followed by `").nth(1) {
        if let Some(p) = rest.split('\'').next() {
            if PREPS.contains(&p.trim()) {
                return Some(p.trim().to_string());
            }
        }
    }
    // (2) `<lemma> <prep>` in the gloss (lemma-keyed).
    let g = gloss.to_lowercase();
    let key = format!("{} ", lemma.to_lowercase());
    let mut from = 0;
    while let Some(i) = g[from..].find(&key) {
        let next = g[from + i + key.len()..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(|c: char| !c.is_ascii_alphabetic());
        if PREPS.contains(&next) {
            return Some(next.to_string());
        }
        from += i + key.len();
    }
    // (3) Curated / LLM frame fallback — the gloss heuristic is low-recall (misses "dependent" → "on",
    // whose gloss says "contingent on"). A frame is admitted only if its preposition is in `PREPS`.
    adjective_frames()
        .get(&lemma.to_lowercase())
        .filter(|p| PREPS.contains(&p.as_str()))
        .cloned()
}

/// Map a `governed_preposition` result to its `lexicon:Prep` feature constructor (D63 §5.3
/// C3-precision). The domain is exactly `governed_preposition`'s `PREPS`; anything else falls back
/// to the `prep_any` wildcard (defensive — the closed set makes the fallback unreachable).
fn prep_ctor(prep: &str) -> &'static str {
    match prep {
        "to" => "lexicon:prep_to",
        "on" => "lexicon:prep_on",
        "in" => "lexicon:prep_in",
        "with" => "lexicon:prep_with",
        "from" => "lexicon:prep_from",
        "for" => "lexicon:prep_for",
        "at" => "lexicon:prep_at",
        "upon" => "lexicon:prep_upon",
        "about" => "lexicon:prep_about",
        "against" => "lexicon:prep_against",
        "into" => "lexicon:prep_into",
        _ => "lexicon:prep_any",
    }
}

/// Adjective synset → predicative entries. **Relational** (pertainym) adjectives are
/// non-gradable → a Boolean predicate (`is_X : Entity → Prop`, sem = the axiom).
/// **Descriptive** (gradable) adjectives (D63 §8.12 6-cmp) become a **measure**
/// `deg_X : Entity → core:float` (+ standard `std_X`): the **positive** is the measure
/// vs. the standard (`gt(deg_X(x), std_X)`) and the **comparative** compares degrees
/// (`gt(deg_X(x), deg_X(y))`) via the opaque float ordering `measurements:gt` (combo 1).
/// Comparative surfaces: the synthetic `-er` from [`comparison`], plus a bare `cat_measure` reading
/// (C1, d63-comparative-phrasal.md §5.3) exposing `deg_X` so the closed-class `more`/`less` handle the
/// periphrastic ("more X") comparative at scale. Superlatives ("most") await "the".
fn push_adj(
    buf: &mut String,
    syn: &Synset,
    rep: &mut Report,
    noun_index: &BTreeMap<Offset, &Synset>,
    ranks: &SenseRanks,
) {
    let loc = local(syn);
    let prop_arrow = format!("{ENTITY_TOP} -> Prop");
    if syn.relational {
        // Non-gradable: the existing Boolean predicate. The axiom is the sense's denotation —
        // carry the gloss as `core:description` for the concept-description index (D63 §6a index c).
        buf.push_str(&format!(
            "axiom wn:{loc} : {prop_arrow} desc: \"{}\"\n\n",
            esc(&syn.gloss)
        ));
        rep.adj_axioms += 1;
        let cat = adj_cat();
        for (i, lemma) in syn.words.iter().enumerate() {
            if restates_governed_frame(lemma) {
                rep.frame_duplicate_skipped += 1;
                continue;
            }
            push_entry(
                buf,
                &format!("e_{loc}_{i}"),
                lemma,
                &cat,
                &loc,
                &prop_arrow,
                &sense_key(syn, lemma),
                ranks,
            );
            rep.entries += 1;
        }
        return;
    }
    // Gradable: a measure + standard, with measure-based positive + degree comparative. The
    // measure `deg_{loc}` is the sense's semantic anchor — carry the gloss on it (the standard
    // `std_{loc}` is a derived threshold, not a sense) for the concept-description index (§6a c).
    buf.push_str(&format!(
        "axiom wn:deg_{loc} : {ENTITY_TOP} -> core:float desc: \"{}\"\n\n",
        esc(&syn.gloss)
    ));
    buf.push_str(&format!("axiom wn:std_{loc} : core:float\n\n"));
    rep.adj_axioms += 1;
    // C3 (d63-comparative-phrasal.md §5.3): a RELATIONAL gradable adjective — one whose gloss governs a
    // preposition (`governed_preposition`) — ALSO gets a 2-place measure `deg_{loc}_rel : Entity(ground)
    // → Entity(subject) → float` and a `cat_measure/cat_pp_arg` reading (below), so `more dependent ON
    // WRN` / `greater dependence ON WRN` thread the ground faithfully. The bare 1-place `deg_{loc}` forms
    // (positive + C1 measure) STAY for the ground-less reading (`more dependent than Y`) — two
    // independent opaque measures (the `∃g` relation between them is deferred, §7; an `∃`-close would be
    // ill-typed over a float).
    // C3-precision: the synset's governed preposition (the first lemma that governs one) tags the
    // nominalization projection's `cat_pp_arg(prep)`. A per-adjective-lemma prep (which may differ
    // within one synset — `addicted`→to vs a co-lemma→on) is taken separately in the lemma loop.
    let syn_prep: Option<String> = syn
        .words
        .iter()
        .find_map(|l| governed_preposition(&syn.gloss, l));
    let relational = syn_prep.is_some();
    if relational {
        buf.push_str(&format!(
            "axiom wn:deg_{loc}_rel : {ENTITY_TOP} -> {ENTITY_TOP} -> core:float\n\n"
        ));
        // C3-positive (Fix A piece (c), d63-single-skeleton-defects.md): the POSITIVE relational
        // predication — "these classifications are concordant WITH X", "WRN is essential FOR
        // proliferation". Exactly the 1-place positive (`pos_sem`: measure vs the absolute standard)
        // with the GROUND threaded: `λr. λx. gt(deg_rel(r, x), std)`. Without it a governed adjective
        // had NO positive relational reading — only the comparative consumed the `cat_measure/cat_pp_arg`
        // form — so "concordant with X" fell back to a 1-place adjective plus a free `with` VP-adjunct
        // (`And(gt(concordant(x)), prep_with(x, X))`), which does not bind the relatum into the degree.
        // Reuses `std_{loc}` deliberately: the positive's standard is ABSOLUTE (unlike the comparative's
        // anaphoric `cmp_attrib_sem`), and the standard is a per-sense threshold, not per-ground.
        push_sem_term(
            buf,
            &format!("pos_rel_sem_{loc}"),
            &format!("( fun (r : {ENTITY_TOP}) => fun (x : {ENTITY_TOP}) => measurements:gt(wn:deg_{loc}_rel(r, x), wn:std_{loc}) : {ENTITY_TOP} -> {prop_arrow} )"),
        );
    }
    push_sem_term(
        buf,
        &format!("pos_sem_{loc}"),
        &format!("( fun (x : {ENTITY_TOP}) => measurements:gt(wn:deg_{loc}(x), wn:std_{loc}) : {prop_arrow} )"),
    );
    push_sem_term(
        buf,
        &format!("cmp_sem_{loc}"),
        &format!("( fun (y : {ENTITY_TOP}) => fun (x : {ENTITY_TOP}) => measurements:gt(wn:deg_{loc}(x), wn:deg_{loc}(y)) : {ENTITY_TOP} -> {prop_arrow} )"),
    );
    // Attributive / elided-`than` comparative (d63-comparative-phrasal §8): `a stronger phenotype` /
    // `X is stronger` — the comparison STANDARD is anaphoric (`lexicon:anaphor`, freshened to a per-span
    // hole → an OPEN parse the D64 resolver fills), NOT the positive's absolute `std_{loc}`. A bare
    // `S[adj]\NP`, so the existing attributive refine rule turns `stronger phenotype` into a refined noun.
    push_sem_term(
        buf,
        &format!("cmp_attrib_sem_{loc}"),
        &format!("( fun (x : {ENTITY_TOP}) => measurements:gt(wn:deg_{loc}(x), wn:deg_{loc}(lexicon:anaphor)) : {prop_arrow} )"),
    );
    let pos_cat = adj_cat();
    let cmp_cat = format!(
        "lexicon:fwd(lexicon:m_all, {}, lexicon:cat_pp_than)",
        adj_cat()
    );
    let cmp_arrow = format!("{ENTITY_TOP} -> {prop_arrow}");
    for (i, lemma) in syn.words.iter().enumerate() {
        if restates_governed_frame(lemma) {
            rep.frame_duplicate_skipped += 1;
            continue;
        }
        let sense = sense_key(syn, lemma);
        // Positive: gt(deg(x), std).
        push_entry(
            buf,
            &format!("e_{loc}_{i}"),
            lemma,
            &pos_cat,
            &format!("pos_sem_{loc}"),
            &prop_arrow,
            &sense,
            ranks,
        );
        rep.entries += 1;
        // C1 (d63-comparative-phrasal.md §5.3): a bare `cat_measure` reading — the degree function
        // `deg_X : Entity → float` itself — so the closed-class `more`/`less` operators
        // (`((S[adj]\NP)/cat_pp_than)/cat_measure`) combine with a periphrastic-comparative adjective.
        // Closes NON-relational adjectival comparatives (`X is more sensitive than Y`) at scale.
        // (Relational adjectives additionally get the ground-taking `cat_measure/cat_pp_arg` form via
        // the C3 curated prep map; the synthetic `-er` below is the same operator pre-bundled.)
        push_entry(
            buf,
            &format!("e_{loc}_{i}_m"),
            lemma,
            "lexicon:cat_measure",
            &format!("deg_{loc}"),
            &format!("{ENTITY_TOP} -> core:float"),
            &sense,
            ranks,
        );
        rep.entries += 1;
        // C3: relational lemmas (gloss governs a prep) also get the ground-taking cat_measure/cat_pp_arg
        // reading — `deg_rel` (ground, subject); `on X` fills the ground → a cat_measure over the subject.
        if let Some(prep) = governed_preposition(&syn.gloss, lemma) {
            push_entry(
                buf,
                &format!("e_{loc}_{i}_r"),
                lemma,
                &format!(
                    "lexicon:fwd(lexicon:m_all, lexicon:cat_measure, lexicon:cat_pp_arg({}))",
                    prep_ctor(&prep)
                ),
                &format!("deg_{loc}_rel"),
                &format!("{ENTITY_TOP} -> {ENTITY_TOP} -> core:float"),
                &sense,
                ranks,
            );
            rep.entries += 1;
            // C3-positive (Fix A (c)): the POSITIVE relational predication `(S[adj]\NP)/cat_pp_arg(prep)`
            // — consume the governed PP (the ground), yield a predicative adjective comparing the
            // 2-place measure to the absolute standard. This is what lets "concordant WITH X" bind X as
            // the relatum instead of stranding it as a free VP-adjunct; the copula then lifts it as any
            // other predicative adjective. Additive: the `cat_measure` form above still serves the
            // comparative (`more concordant with X than …`).
            push_entry(
                buf,
                &format!("e_{loc}_{i}_rp"),
                lemma,
                &format!(
                    "lexicon:fwd(lexicon:m_all, {}, lexicon:cat_pp_arg({}))",
                    adj_cat(),
                    prep_ctor(&prep)
                ),
                &format!("pos_rel_sem_{loc}"),
                &cmp_arrow,
                &sense,
                ranks,
            );
            rep.entries += 1;
        }
        // Synthetic `-er` comparative (`larger`); periphrastic "more X" now rides the `cat_measure`
        // reading above + the closed-class `more`/`less`.
        if let Comparison::Synthetic { comparative, .. } = comparison(lemma) {
            for (k, c) in comparative.iter().enumerate() {
                push_entry(
                    buf,
                    &format!("e_{loc}_{i}_c{k}"),
                    c,
                    &cmp_cat,
                    &format!("cmp_sem_{loc}"),
                    &cmp_arrow,
                    &sense,
                    ranks,
                );
                rep.entries += 1;
                // Attributive / elided-`than` reading of the same synthetic comparative (bare `S[adj]\NP`,
                // anaphoric standard) → `a stronger phenotype` refines the noun, opens the standard hole.
                push_entry(
                    buf,
                    &format!("e_{loc}_{i}_ca{k}"),
                    c,
                    &pos_cat,
                    &format!("cmp_attrib_sem_{loc}"),
                    &prop_arrow,
                    &sense,
                    ranks,
                );
                rep.entries += 1;
            }
        }
    }

    // C2 (d63-comparative-phrasal.md §5.3): project `deg_{loc}` onto derivationally-related NOUNS
    // (`dependent` → `dependence`) as a bare `cat_measure` reading — the nominalization's measure IS
    // the adjective's degree, so `greater/less <nominalization>` parses at scale. (Relational nouns get
    // the ground-taking `cat_measure/cat_pp_arg` form via the C3 curated prep map.)
    for (off, tpos) in &syn.derivational {
        if tpos != "n" {
            continue;
        }
        if let Some(&noun) = noun_index.get(off) {
            for (j, nlemma) in noun.words.iter().enumerate() {
                push_entry(
                    buf,
                    &format!("e_{loc}_d_{}_{j}", local(noun)),
                    nlemma,
                    "lexicon:cat_measure",
                    &format!("deg_{loc}"),
                    &format!("{ENTITY_TOP} -> core:float"),
                    &sense_key(noun, nlemma),
                    ranks,
                );
                rep.entries += 1;
                // C3: relational projection — the nominalization (`dependence`) also gets the
                // ground-taking `cat_measure/cat_pp_arg` reading, so `greater dependence ON WRN` threads.
                if let Some(prep) = &syn_prep {
                    push_entry(
                        buf,
                        &format!("e_{loc}_dr_{}_{j}", local(noun)),
                        nlemma,
                        &format!(
                            "lexicon:fwd(lexicon:m_all, lexicon:cat_measure, lexicon:cat_pp_arg({}))",
                            prep_ctor(prep)
                        ),
                        &format!("deg_{loc}_rel"),
                        &format!("{ENTITY_TOP} -> {ENTITY_TOP} -> core:float"),
                        &sense_key(noun, nlemma),
                        ranks,
                    );
                    rep.entries += 1;
                }
            }
        }
    }
}

/// Render a set of synsets to one ESL document. The caller is responsible for
/// closure (every `@` parent + [`ENTITY_TOP`] present); rendering is order-
/// independent (references resolve at layer time). Output is deterministic:
/// synsets are emitted sorted by `(pos, offset)`, declarations before entries.
/// `ranks` supplies each lemma's sense-frequency rank → `lexicon:sense_rank` (D63
/// §8.7 Stage B); pass an empty map to omit ranks (all default 0).
pub fn render_document(
    synsets: &[Synset],
    ranks: &SenseRanks,
    mass: &MassNouns,
) -> (String, Report) {
    let (decls, entries, rep) = render_core(synsets, ranks, mass);
    // The `lexicon:wordnet` descriptor (D65 §3) leads the body — every entry's
    // `lexicon:in_lexicon` points at it, so it must resolve in the same document.
    let doc = format!(
        "{ESL_HEADER}\n{WORDNET_LEXICON}\n{decls}{}",
        entries.concat()
    );
    (doc, rep)
}

/// Render the import as two independently-loadable sections for the partitioned
/// (`--out-dir`) emit: the **base body** (`ESL_HEADER` + the `lexicon:wordnet`
/// descriptor + every class/axiom declaration), and the list of **`LexicalEntry`
/// blocks** (each a self-contained paragraph that references its synset class and the
/// descriptor *by IRI*, so it resolves against the base layer below it). The base is
/// ~20 MB (all decls); the entries are the bulk (~150 MB) and are what the caller
/// batches under the gRPC size cap. Same split `render_document` makes internally —
/// exposed so a chain emit can put decls in layer 0 and stream entries into chunks
/// without any cross-chunk dependency (entries depend only on the base).
pub fn render_sections(
    synsets: &[Synset],
    ranks: &SenseRanks,
    mass: &MassNouns,
) -> (String, Vec<String>, Report) {
    let (decls, entries, rep) = render_core(synsets, ranks, mass);
    let base = format!("{ESL_HEADER}\n{WORDNET_LEXICON}\n{decls}");
    (base, entries, rep)
}

/// Shared core: render every synset to a declaration section (`decls`) and a vector of
/// `LexicalEntry` blocks (`entries`), keeping the two separable so both the
/// single-document and partitioned emits can assemble them.
fn render_core(
    synsets: &[Synset],
    ranks: &SenseRanks,
    mass: &MassNouns,
) -> (String, Vec<String>, Report) {
    let mut sorted: Vec<&Synset> = synsets.iter().collect();
    sorted.sort_by(|a, b| (a.pos, &a.offset).cmp(&(b.pos, &b.offset)));

    let mut rep = Report::default();
    let mut decls = String::new(); // classes + axioms
    let mut entries: Vec<String> = Vec::new();

    // Noun offset → synset, for resolving instance-of-instance `@i` chains to a
    // class ([`nearest_classes`]). The caller closes the set under hypernymy, so
    // every `@i`/`@` ancestor is present.
    let noun_index: BTreeMap<Offset, &Synset> = synsets
        .iter()
        .filter(|s| s.pos == Pos::Noun)
        .map(|s| (s.offset.clone(), s))
        .collect();

    for syn in sorted {
        match syn.pos {
            Pos::Noun => {
                let mut block = String::new();
                if syn.instance_of.is_empty() {
                    push_noun(&mut block, syn, &mut rep, &noun_index, ranks, mass);
                } else {
                    push_instance(&mut block, syn, &mut rep, &noun_index, ranks);
                }
                route(&block, &mut decls, &mut entries);
            }
            Pos::Verb => {
                let mut block = String::new();
                if push_verb(&mut block, syn, &mut rep, ranks) {
                    route(&block, &mut decls, &mut entries);
                }
            }
            Pos::Adj => {
                let mut block = String::new();
                push_adj(&mut block, syn, &mut rep, &noun_index, ranks);
                route(&block, &mut decls, &mut entries);
            }
            Pos::Adv => {} // deferred (§8.7.5)
        }
    }

    (decls, entries, rep)
}

/// Split a rendered synset block (decl + entries) into the declaration section and the
/// list of entry blocks. A block is `<class|axiom …>\n\n<resource …>\n\n…`; the first
/// paragraph is the declaration, the rest are entries.
fn route(block: &str, decls: &mut String, entries: &mut Vec<String>) {
    let mut paras = block.split_inclusive("\n\n");
    if let Some(decl) = paras.next() {
        decls.push_str(decl);
    }
    for entry in paras {
        entries.push(entry.to_string());
    }
}

#[cfg(test)]
mod tests {

    /// The frame-duplicate drop must be NARROW: it fires only where the base adjective's governance
    /// of that exact preposition is known, so genuine idioms survive.
    #[test]
    fn frame_duplicate_drop_spares_idioms() {
        // `dependent` -> `on` IS in adjective-frames.tsv, so the MWE duplicates the compositional
        // relational analysis and must go.
        assert!(restates_governed_frame("dependent on"));

        // Idioms and non-governed pairs must SURVIVE. `contingent on` shares a synset with
        // `dependent on`, but `contingent` is not gloss-governed, so the criterion leaves it alone —
        // deliberately: the drop is keyed on KNOWN governance, not on shape.
        for keep in [
            "contingent on",
            "contingent upon",
            "all in",
            "boxed in",
            "agreed upon",
            "adequate to",
            "comparable to",
            "dependent",
            "essential",
        ] {
            assert!(
                !restates_governed_frame(keep),
                "{keep} must not be dropped — its base is not gloss-governed for that preposition"
            );
        }

        // Every governed pair in the table is a drop candidate by construction; check one more so a
        // future table edit cannot silently make the rule inert.
        assert!(
            restates_governed_frame("essential for")
                || !adjective_frames().contains_key("essential")
        );
    }
    use super::*;
    use crate::wndb::parse_data_line;

    fn syn(line: &str) -> Synset {
        parse_data_line(line).unwrap()
    }

    #[test]
    fn stative_relational_participle_binds_the_frame_named_preposition() {
        // The real `associate` synset (00713167), frames 17 + 31 — both "… WITH something".
        let s = syn(
            "00713167 31 v 07 associate 0 tie_in 0 relate 0 link 0 colligate 2 link_up 0 connect 0 \
             000 02 + 17 00 + 31 00 | make a logical or causal connection",
        );
        let mut rep = Report::default();
        let mut buf = String::new();
        push_verb(&mut buf, &s, &mut rep, &SenseRanks::new());

        // A 2-place axiom, distinct from the verbal `_d` ditransitive one.
        assert!(
            buf.contains(
                "axiom wn:v00713167_rel : lexicon:Entity -> lexicon:Entity -> Prop desc: \"make a logical or causal connection (stative relational participle; governs `with`)\""
            ),
            "stative axiom missing:\n{buf}"
        );
        // The participle takes the PP as an ARGUMENT, and the preposition is the one the frame names —
        // not the `prep_any` wildcard. This is what makes `with` a case-marker instead of an adjunct.
        assert!(
            buf.contains(
                "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_pp_arg(lexicon:prep_with)) );"
            ),
            "stative category missing or wrong preposition:\n{buf}"
        );
        // Emitted on the PAST PARTICIPLE surface only — `associated`, never `associate`/`associates`
        // with this category (a stative relation has no finite active paradigm of its own).
        assert!(buf.contains("lexicon:form     = \"associated\";"));
        assert_eq!(rep.stative_entries, s.words.len(), "one per lemma: {buf}");
        // ADDITIVE — the frame-derived ditransitive entries survive, so the ledger decides.
        assert!(
            buf.contains("wn:v00713167_d"),
            "verbal entries dropped:\n{buf}"
        );
    }

    #[test]
    fn stative_participle_is_restricted_to_the_shipped_with_frames() {
        // Frames 15/16/18/19 name a preposition too, but are HELD BACK on a measurement (frame 19
        // gaps a unit through the per-entry sense cap; frame 15 is inert). Pin the scope so widening
        // it is a deliberate edit, not a drive-by.
        assert_eq!(stative_prep(17), Some("with"));
        assert_eq!(stative_prep(31), Some("with"));
        for f in [14u8, 15, 16, 18, 19] {
            assert_eq!(stative_prep(f), None, "frame {f} is not shipped");
        }
        // A synset carrying ONLY a held-back naming frame emits no stative entry at all.
        let s = syn("01115006 41 v 01 set 0 000 01 + 19 00 | put into a certain place");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_verb(&mut buf, &s, &mut rep, &SenseRanks::new());
        assert_eq!(rep.stative_entries, 0);
        assert!(
            !buf.contains("_rel"),
            "held-back frame emitted a stative:\n{buf}"
        );
    }

    #[test]
    fn frame_classification_covers_all_35() {
        // intransitive / transitive / ditransitive — the emittable kinds.
        assert_eq!(classify(2), Some(FrameKind::Intransitive));
        assert_eq!(classify(23), Some(FrameKind::Intransitive)); // "Somebody's (body part) ----s"
        assert_eq!(classify(8), Some(FrameKind::Transitive));
        assert_eq!(classify(13), Some(FrameKind::Transitive)); // "----s on something"
        assert_eq!(classify(14), Some(FrameKind::Ditransitive));
        assert_eq!(classify(31), Some(FrameKind::Ditransitive));
        // single-PP-complement frames → argument-PP verb `(S\NP)/cat_pp_arg` (D63 verb+PP).
        assert_eq!(classify(12), Some(FrameKind::PpOblique)); // "----s to somebody"
        assert_eq!(classify(27), Some(FrameKind::PpOblique)); // "----s to somebody"
        assert_eq!(classify(4), Some(FrameKind::PpOblique)); //  "is ----ing PP"
        assert_eq!(classify(22), Some(FrameKind::PpOblique)); // "Somebody ----s PP"
                                                              // frame 26 "that CLAUSE" → clause-taking (D63 §8.11 6-cl).
        assert_eq!(classify(26), Some(FrameKind::Clausal));
        // frames 6/7 "----s Adjective" → linking (copular) verb (D63 §8.5).
        assert_eq!(classify(6), Some(FrameKind::LinkingAdj));
        assert_eq!(classify(7), Some(FrameKind::LinkingAdj));
        // still-deferred higher-order frames → None (never guessed).
        assert_eq!(classify(5), None); // object-predicate small clause (`consider X important`)
        assert_eq!(classify(29), None); // whether CLAUSE (interrogative — deferred)
        assert_eq!(classify(32), None); // bare INFINITIVE (control)
                                        // every frame 1..=35 is classified deliberately (no silent gap).
        for f in 1u8..=35 {
            let _ = classify(f);
        }
    }

    #[test]
    fn noun_class_with_subclass_of_and_entry() {
        let gene = syn("05444328 08 n 03 gene 0 cistron 0 factor 0 003 @ 08476263 n 0000 #p 14854534 n 0000 #p 05449707 n 0000 | a segment of DNA");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_noun(
            &mut buf,
            &gene,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
            &MassNouns::new(),
        );
        assert!(buf.contains("class wn:n05444328 : wn:n08476263 {"));
        assert!(buf.contains("description = \"a segment of DNA\";"));
        assert!(buf.contains("resource wn:e_n05444328_0 : lexicon:LexicalEntry {"));
        assert!(buf.contains("lexicon:form     = \"gene\";"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:cat_n(wn:n05444328, lexicon:num_any) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:n05444328;"));
        assert!(buf.contains("lexicon:sem_type = type_expr( Set );"));
        assert_eq!(rep.noun_classes, 1);
        assert_eq!(rep.entries, 3); // gene, cistron, factor
    }

    #[test]
    fn countability_lexicon_adds_an_additive_mass_entry() {
        // D62 countability: a lemma flagged uncountable gets a SECOND `cat_n(C, mass)` entry
        // ALONGSIDE the count one (so `a gene`/`genes` still parse while bare `gene` can shift).
        // A multi-lemma synset only mass-marks the flagged lemma(s).
        let gene = syn(
            "05444328 08 n 03 gene 0 cistron 0 factor 0 001 @ 00001740 n 0000 | a segment of DNA",
        );
        let mut mass = MassNouns::new();
        mass.insert("gene".into()); // flag only `gene`, not `cistron`/`factor`
        let mut rep = Report::default();
        let mut buf = String::new();
        push_noun(
            &mut buf,
            &gene,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
            &mass,
        );
        // count entry (unchanged) + an additive mass entry, both for `gene`.
        assert!(buf.contains("resource wn:e_n05444328_0 : lexicon:LexicalEntry {"));
        assert!(buf.contains("resource wn:e_n05444328_0_mass : lexicon:LexicalEntry {"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:cat_n(wn:n05444328, lexicon:mass) );"
        ));
        // unflagged lemmas get NO mass entry.
        assert!(!buf.contains("wn:e_n05444328_1_mass"));
        assert_eq!(rep.entries, 4); // gene, cistron, factor + gene-mass
        assert_eq!(rep.mass_entries, 1);
    }

    #[test]
    fn sense_rank_is_emitted_only_when_nonzero() {
        // D63 §8.7 Stage B: a ranked sense → `lexicon:sense_rank`; rank 0 → omitted.
        let gene = syn("05444328 08 n 03 gene 0 cistron 0 factor 0 003 @ 08476263 n 0000 #p 14854534 n 0000 #p 05449707 n 0000 | a segment of DNA");
        let mut ranks = SenseRanks::new();
        ranks.insert("wn:gene.n.05444328".into(), 2); // gene = 3rd sense
        ranks.insert("wn:cistron.n.05444328".into(), 0); // most-frequent → omitted
        let mut buf = String::new();
        push_noun(
            &mut buf,
            &gene,
            &mut Report::default(),
            &BTreeMap::new(),
            &ranks,
            &MassNouns::new(),
        );
        // The `gene` entry carries the rank; `cistron` (rank 0) and `factor` (absent) do not.
        assert!(
            buf.contains("lexicon:form     = \"gene\";\n    lexicon:cat      = type_expr( lexicon:cat_n(wn:n05444328, lexicon:num_any) );\n    lexicon:sem      = wn:n05444328;\n    lexicon:sem_type = type_expr( Set );\n    lexicon:sense    = \"wn:gene.n.05444328\";\n    lexicon:sense_rank = 2;\n"),
            "the ranked `gene` entry must carry sense_rank 2, got:\n{buf}"
        );
        assert_eq!(
            buf.matches("lexicon:sense_rank").count(),
            1,
            "only the nonzero rank emits"
        );
    }

    #[test]
    fn root_noun_is_rooted_at_the_schema_entity_top() {
        // WordNet's hypernym-less root `entity.n.01` is parented at the schema
        // entity top `lexicon:Entity` (D63 §8.3 ii), so the whole noun lattice
        // sits under it.
        let entity = syn("00001740 03 n 01 entity 0 001 ~ 00001930 n 0000 | that which exists");
        let mut buf = String::new();
        push_noun(
            &mut buf,
            &entity,
            &mut Report::default(),
            &BTreeMap::new(),
            &SenseRanks::new(),
            &MassNouns::new(),
        );
        assert!(buf.contains("class wn:n00001740 : lexicon:Entity {"));
    }

    #[test]
    fn instance_synset_is_an_individual_not_a_class() {
        // Einstein `@i` physicist (10428004): an NP individual, not a class.
        let einstein = syn("10954498 18 n 02 Einstein 0 Albert_Einstein 0 002 @i 10428004 n 0000 + 03031247 a 0301 | a physicist");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_instance(
            &mut buf,
            &einstein,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
        );
        // Emitted as a RESOURCE (instance of its class), never a `class`.
        assert!(buf.contains("resource wn:n10954498 : wn:n10428004 {"));
        assert!(!buf.contains("class wn:n10954498"));
        assert!(buf.contains("description = \"a physicist\";"));
        // NP entries (cat_np at the class), one per lemma, sem = the individual.
        assert!(buf.contains("resource wn:e_n10954498_0_0 : lexicon:LexicalEntry {"));
        assert!(buf.contains("lexicon:form     = \"Einstein\";"));
        assert!(buf.contains("lexicon:form     = \"Albert Einstein\";"));
        assert!(buf
            .contains("lexicon:cat      = type_expr( lexicon:cat_np(wn:n10428004, lexicon:sg) );"));
        assert!(buf.contains("lexicon:sem      = wn:n10954498;"));
        assert!(buf.contains("lexicon:sem_type = type_expr( wn:n10428004 );"));
        assert_eq!(rep.instances, 1);
        assert_eq!(rep.noun_classes, 0);
        assert_eq!(rep.entries, 2); // Einstein, Albert Einstein
    }

    #[test]
    fn multi_instance_of_emits_an_np_entry_per_class() {
        // A rare instance of two classes: `resource r : A, B` (no drop), and one
        // NP entry per class — admissible via the check-mode resource rule (#91).
        let v = syn("00000009 18 n 01 Enlightenment 0 002 @i 15254028 n 0000 @ 08473623 n 0000 | a movement");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_instance(&mut buf, &v, &mut rep, &BTreeMap::new(), &SenseRanks::new());
        // both classes on the resource — @i first, then the rare plain @.
        assert!(buf.contains("resource wn:n00000009 : wn:n15254028, wn:n08473623 {"));
        // one NP entry per class (both type contexts reachable).
        assert!(buf
            .contains("lexicon:cat      = type_expr( lexicon:cat_np(wn:n15254028, lexicon:sg) );"));
        assert!(buf
            .contains("lexicon:cat      = type_expr( lexicon:cat_np(wn:n08473623, lexicon:sg) );"));
        assert_eq!(rep.instances, 1);
        assert_eq!(rep.entries, 2); // 2 classes × 1 lemma
    }

    #[test]
    fn instance_of_instance_chain_resolves_to_nearest_class() {
        // WordNet chains instances: Paternoster `@i` Lord's_Prayer `@i` prayer (a class).
        // An individual can't be typed by another individual (the parent is emitted as a
        // `resource`, not a `class`), so the type climbs to the nearest class — prayer.
        let prayer = syn("06455990 10 n 01 prayer 1 001 @ 06429590 n 0000 | a prayer");
        let lords = syn("06457612 10 n 01 Lord's_Prayer 0 001 @i 06455990 n 0000 | the prayer");
        let pater = syn(
            "06457796 10 n 01 Paternoster 0 001 @i 06457612 n 0000 | the Lord's Prayer in Latin",
        );
        let noun_index: BTreeMap<Offset, &Synset> = [&prayer, &lords, &pater]
            .into_iter()
            .map(|s| (s.offset.clone(), s))
            .collect();

        let mut rep = Report::default();
        let mut buf = String::new();
        push_instance(&mut buf, &pater, &mut rep, &noun_index, &SenseRanks::new());
        // Typed at the nearest CLASS (prayer = n06455990), NOT the instance Lord's_Prayer.
        assert!(
            buf.contains("resource wn:n06457796 : wn:n06455990 {"),
            "Paternoster must instantiate the class prayer, got:\n{buf}"
        );
        assert!(
            !buf.contains("wn:n06457612"),
            "must not reference the instance parent"
        );
        assert!(buf
            .contains("lexicon:cat      = type_expr( lexicon:cat_np(wn:n06455990, lexicon:sg) );"));
        assert_eq!(rep.instances, 1);
        assert_eq!(rep.entries, 1);
    }

    #[test]
    fn render_routes_instances_away_from_classes() {
        // A class (gene) and an individual (Einstein) in one document: the gene
        // is a `class`, Einstein a `resource` instance — the routing split.
        let synsets = [
            syn("10428004 18 n 01 physicist 0 000 | a scientist"),
            syn("10954498 18 n 01 Einstein 0 001 @i 10428004 n 0000 | a physicist"),
        ];
        let (doc, rep) = render_document(&synsets, &SenseRanks::new(), &MassNouns::new());
        assert!(doc.contains("class wn:n10428004 : lexicon:Entity {"));
        assert!(doc.contains("resource wn:n10954498 : wn:n10428004 {"));
        assert!(!doc.contains("class wn:n10954498"));
        assert_eq!(rep.noun_classes, 1);
        assert_eq!(rep.instances, 1);
    }

    #[test]
    fn emitted_document_carries_the_wordnet_attribution() {
        // License-compliance guard: WordNet 3.0 requires its copyright notice + disclaimer
        // on ALL copies including modifications, and every emitted document is a WordNet
        // derivative (it embeds glosses). The notice must lead the output.
        let (doc, _) = render_document(
            &[syn("00001740 03 n 01 entity 0 000 | that which exists")],
            &SenseRanks::new(),
            &MassNouns::new(),
        );
        assert!(
            doc.starts_with("// "),
            "the attribution comment must lead the document"
        );
        assert!(doc
            .contains("WordNet 3.0 Copyright 2006 by Princeton University. All rights reserved."));
        assert!(
            doc.contains("PROVIDED \"AS IS\""),
            "the disclaimer must be present"
        );
        assert!(doc.contains("DERIVED FROM WordNet 3.0"));
    }

    #[test]
    fn emitted_document_declares_the_wordnet_lexicon_and_tags_every_entry() {
        // D65 §3 slice 3: every import emits a single `lexicon:wordnet` Lexicon
        // descriptor (stable identity + provenance) and tags each LexicalEntry with
        // `lexicon:in_lexicon = lexicon:wordnet` (membership = the inverse).
        let (doc, _) = render_document(
            &[syn("00001740 03 n 01 entity 0 000 | that which exists")],
            &SenseRanks::new(),
            &MassNouns::new(),
        );

        // The descriptor is emitted exactly once, carrying provenance metadata.
        assert_eq!(
            doc.matches("resource lexicon:wordnet : lexicon:Lexicon")
                .count(),
            1,
            "the lexicon:wordnet descriptor must be emitted exactly once"
        );
        assert!(doc.contains("lexicon:source   = \"WordNet 3.0, Princeton University\";"));
        assert!(doc.contains("lexicon:domain   = \"general\";"));

        // Every LexicalEntry binds back to it — no entry left untagged.
        let entry_count = doc.matches(": lexicon:LexicalEntry {").count();
        let tag_count = doc.matches("lexicon:in_lexicon = lexicon:wordnet;").count();
        assert!(entry_count > 0, "the import must emit at least one entry");
        assert_eq!(
            entry_count, tag_count,
            "every LexicalEntry ({entry_count}) must carry in_lexicon ({tag_count})"
        );
    }

    #[test]
    fn transitive_verb_axiom_and_object_first_category() {
        let eat = syn("00275082 30 v 03 corrode 1 eat 0 rust 1 001 @ 00259743 v 0000 01 + 11 00 | to deteriorate");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &eat, &mut rep, &SenseRanks::new()));
        // frame 11 → transitive; the axiom IRI is kind-tagged (`_t`). The synset gloss rides the
        // axiom as a `desc:` clause → `core:description` (D63 §6a index c: verb senses searchable).
        assert!(buf.contains(
            "axiom wn:v00275082_t : lexicon:Entity -> lexicon:Entity -> Prop desc: \"to deteriorate\""
        ));
        // Finite 3sg has a SINGULAR subject slot (6-agr); object slot stays num_any.
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:v00275082_t;"));
        // base (num_any) + finite 3sg ("eats", sg) + finite plural ("eat", pl) forms.
        assert!(buf.contains("lexicon:form     = \"eat\";")); // bse + fin-pl (lemma surface)
        assert!(buf.contains("lexicon:form     = \"eats\";")); // fin 3sg
                                                               // bse keeps a num_any subject (the aux supplies agreement).
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:bse), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        // plural-finite has a PLURAL subject slot (6-agr).
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:pl)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        // Finite SIMPLE PAST — same surface as the participle but a finite (`fin`) clause head with a
        // `num_any` subject (past tense has no number agreement). The object slot stays num_any.
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        assert_eq!(rep.verb_axioms, 1);
        // Per lemma: base + finite 3sg + finite plural + gerund + past participle + finite simple
        // past → 3 lemmas × 6 = 18 entries; 6 of them participles (ger + pss; the finite-past is not
        // counted a participle).
        assert_eq!(rep.entries, 18);
        assert_eq!(rep.participle_entries, 6);
    }

    #[test]
    fn evaluate_is_a_curated_essive_verb() {
        // `We evaluated MSI as a biomarker for WRN dependency` gapped because `evaluate` was not in the
        // essive set; guard the appraisal group. `treat`/`use` stay out (dominant-noun risk).
        assert!(is_essive_verb("evaluate"));
        assert!(is_essive_verb("assess"));
        assert!(is_essive_verb("deem"));
        assert!(!is_essive_verb("treat"));
        assert!(!is_essive_verb("use"));
    }

    #[test]
    fn essive_verb_emits_object_predicative_as_frame() {
        // `identify` is a curated essive verb (D63 §5.3): on TOP of its frame-derived transitive
        // category, `push_verb` emits the object-predicative essive `((S\NP)/cat_pp_arg(prep_as))/NP` —
        // WordNet has no `as`-complement frame, so it is added per-lemma (`is_essive_verb`).
        let v = syn(
            "00618451 31 v 01 identify 0 001 @ 00619183 v 0000 01 + 08 00 | consider to be equal",
        );
        let mut buf = String::new();
        assert!(push_verb(
            &mut buf,
            &v,
            &mut Report::default(),
            &SenseRanks::new()
        ));
        // The essive axiom is a 3-place opaque relation (subj, obj, as-complement), tagged `_as`.
        assert!(buf.contains(
            "axiom wn:v00618451_as : lexicon:Entity -> lexicon:Entity -> lexicon:Entity -> Prop"
        ));
        // Finite 3sg essive category `((S\NP)/cat_pp_arg(prep_as))/NP` for "identifies": object NP binds
        // first (adjacent), then the `as`-marked predicative complement `cat_pp_arg(prep_as)`.
        assert!(buf.contains("lexicon:form     = \"identifies\";"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_pp_arg(lexicon:prep_as)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        // Essive is ADDITIVE: the frame-8 transitive category is still emitted.
        assert!(buf.contains("axiom wn:v00618451_t :"));
    }

    #[test]
    fn non_essive_verb_gets_no_as_frame() {
        // A non-curated verb (`eat`) never gets the essive frame — no `_as` axiom, no `prep_as`.
        let eat = syn("00275082 30 v 03 corrode 1 eat 0 rust 1 001 @ 00259743 v 0000 01 + 11 00 | to deteriorate");
        let mut buf = String::new();
        assert!(push_verb(
            &mut buf,
            &eat,
            &mut Report::default(),
            &SenseRanks::new()
        ));
        assert!(!buf.contains("_as :"));
        assert!(!buf.contains("prep_as"));
    }

    #[test]
    fn verb_emits_participle_forms_with_ger_and_pss_categories() {
        // D63 §8.9 6-aux: per verb lemma, the importer also emits the generated present
        // participle (`ger`, progressive) and past participle (`pss`, perfect/passive),
        // pointing at the SAME axiom, differing only in the result clause's Fin feature.
        let eat = syn("00275082 30 v 03 corrode 1 eat 0 rust 1 001 @ 00259743 v 0000 01 + 11 00 | to deteriorate");
        let mut buf = String::new();
        assert!(push_verb(
            &mut buf,
            &eat,
            &mut Report::default(),
            &SenseRanks::new()
        ));
        // gerund (regular -ing) + its `ger` category.
        assert!(buf.contains("lexicon:form     = \"eating\";"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:ger), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        // irregular past participle (eat → eaten) + its `pss` category, same axiom.
        assert!(buf.contains("lexicon:form     = \"eaten\";"));
        assert!(buf.contains("lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:pss), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"));
        // participles point at the same predicate axiom as the finite form.
        assert!(buf.contains("lexicon:sem      = wn:v00275082_t;"));
        // the regular members inflect too: corrode → corroded, rust → rusting.
        assert!(buf.contains("lexicon:form     = \"corroded\";"));
        assert!(buf.contains("lexicon:form     = \"rusting\";"));
    }

    #[test]
    fn multiword_verb_inflects_only_the_head() {
        // "depend on" (frame 13, PP-oblique → transitive): head inflects, particle kept.
        let v = syn("00000002 31 v 01 depend_on 0 000 01 + 13 00 | rely");
        let mut buf = String::new();
        assert!(push_verb(
            &mut buf,
            &v,
            &mut Report::default(),
            &SenseRanks::new()
        ));
        assert!(buf.contains("lexicon:form     = \"depend on\";")); // bse
        assert!(buf.contains("lexicon:form     = \"depends on\";")); // fin 3sg
        assert!(buf.contains("lexicon:form     = \"depending on\";"));
        assert!(buf.contains("lexicon:form     = \"depended on\";"));
    }

    #[test]
    fn verb_alternation_emits_one_axiom_per_kind() {
        // frames 2 (intransitive) + 8 (transitive) → BOTH axioms (the verb's
        // alternations), each with its own kind-tagged IRI.
        let v = syn("00001740 29 v 01 breathe 0 000 02 + 02 00 + 08 00 | respire");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut rep, &SenseRanks::new()));
        assert!(buf.contains("axiom wn:v00001740_i : lexicon:Entity -> Prop"));
        assert!(buf.contains("axiom wn:v00001740_t : lexicon:Entity -> lexicon:Entity -> Prop"));
        assert_eq!(rep.verb_axioms, 2);
        // one lemma × two kinds × (base + 3sg + plural-finite + gerund + 1 pp + 1 finite-past) = 12.
        assert_eq!(rep.entries, 12);
    }

    #[test]
    fn ditransitive_verb_curries_three_entity_slots() {
        // frame 14 "Somebody ----s somebody something" → ditransitive.
        let v = syn("00001234 30 v 01 give 0 000 01 + 14 00 | transfer");
        let mut buf = String::new();
        assert!(push_verb(
            &mut buf,
            &v,
            &mut Report::default(),
            &SenseRanks::new()
        ));
        assert!(buf.contains(
            "axiom wn:v00001234_d : lexicon:Entity -> lexicon:Entity -> lexicon:Entity -> Prop"
        ));
        assert!(buf.contains("lexicon:fwd(lexicon:m_all, lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any))"));
    }

    #[test]
    fn clausal_verb_emits_report_axiom_and_cp_category() {
        // frame 26 → clause-taking report verb (D63 §8.11 6-cl): an opaque
        // `Prop → Entity → Prop` axiom and the category `(S\NP)/cat_cp`.
        let v = syn("00000003 31 v 01 show 0 000 01 + 26 00 | demonstrate");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut rep, &SenseRanks::new()));
        assert!(buf.contains("axiom wn:v00000003_c : Prop -> lexicon:Entity -> Prop"));
        assert!(buf.contains("lexicon:form     = \"shows\";")); // 3sg
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_cp) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:v00000003_c;"));
    }

    #[test]
    fn copula_lemma_is_skipped_but_its_synset_siblings_survive() {
        // The copula is grammar (closed-class bootstrap), so WordNet's `be` verb senses must not be
        // emitted — its frame-6 LINKING entry re-encodes "X is P" as an opaque `be(λx.P(x), X)` and
        // competed with the copula (8 of 16 readings on the WRN page's worst unit). The skip is
        // per-LEMMA: a synset carrying `be` alongside a content lemma keeps the content lemma.
        let be_and_follow =
            syn("02445925 41 v 02 be 0 follow 9 000 02 + 22 00 + 08 01 | work in a specific place");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_verb(&mut buf, &be_and_follow, &mut rep, &SenseRanks::new());
        assert!(
            !buf.contains("lexicon:form     = \"be\";"),
            "the copula lemma must not be emitted:\n{buf}"
        );
        assert!(
            !buf.contains("lexicon:form     = \"were\";")
                && !buf.contains("lexicon:form     = \"is\";"),
            "nor any inflection derived from it:\n{buf}"
        );
        assert!(
            buf.contains("lexicon:form     = \"follow\";"),
            "the synset's content sibling lemma MUST survive:\n{buf}"
        );
        assert!(rep.copula_skipped > 0, "the skip is counted");
        // `have`/`do` are genuine content verbs on this corpus and are NOT skipped.
        assert!(!is_copula_lemma("have") && !is_copula_lemma("do"));
        assert!(is_copula_lemma("be") && is_copula_lemma("Be"));
    }

    #[test]
    fn linking_verb_emits_copula_adjective_category() {
        // frames 6/7 → linking (copular) verb (D63 §8.5, gap #5 `remained true`): an opaque
        // `(Entity → Prop) → Entity → Prop` axiom and the category `(S[dcl,fin]\NP)/(S[dcl,adj]\NP)`,
        // mirroring the copula `be`'s adjective complement while keeping the verb's own relation.
        let v = syn("00000004 42 v 01 remain 0 000 01 + 06 00 | continue in a state");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut rep, &SenseRanks::new()));
        assert!(buf
            .contains("axiom wn:v00000004_j : (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop"));
        // 3sg "remains" over an `adj` complement, singular subject.
        assert!(buf.contains("lexicon:form     = \"remains\";"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any))) );"
        ));
        // The corpus form: past "remained" (fin, number-agnostic) also emitted.
        assert!(buf.contains("lexicon:form     = \"remained\";"));
        assert!(buf.contains("lexicon:sem      = wn:v00000004_j;"));
    }

    #[test]
    fn gradable_adjective_emits_measure_positive_and_comparative() {
        // A descriptive (no pertainym) adjective is gradable (D63 §8.12 6-cmp): a measure
        // `deg` + standard `std`, a measure-based positive, and a degree comparative.
        let large = syn("00000001 00 a 01 large 0 000 | of great size");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_adj(
            &mut buf,
            &large,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
        );
        assert!(buf.contains("axiom wn:deg_a00000001 : lexicon:Entity -> core:float"));
        assert!(buf.contains("axiom wn:std_a00000001 : core:float"));
        assert!(buf.contains("resource wn:pos_sem_a00000001 : lexicon:SemTerm {"));
        assert!(buf.contains("resource wn:cmp_sem_a00000001 : lexicon:SemTerm {"));
        // positive "large" (measure-based) + comparative "larger" (degree comparison).
        assert!(buf.contains("lexicon:form     = \"large\";"));
        assert!(buf.contains("lexicon:form     = \"larger\";"));
        assert!(buf.contains("lexicon:sem      = wn:pos_sem_a00000001;"));
        assert!(buf.contains("lexicon:sem      = wn:cmp_sem_a00000001;"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_pp_than) );"
        ));
        // C1 (d63-comparative-phrasal.md §5.3): a bare `cat_measure` reading of `deg_X` — so the
        // closed-class `more`/`less` operators combine (periphrastic comparative at scale).
        assert!(buf.contains("lexicon:cat      = type_expr( lexicon:cat_measure );"));
        assert!(buf.contains("lexicon:sem      = wn:deg_a00000001;"));
        // no Boolean is-axiom for a gradable adjective.
        assert!(!buf.contains("axiom wn:a00000001 : lexicon:Entity -> Prop"));
    }

    #[test]
    fn gradable_adjective_projects_deg_onto_its_nominalization() {
        // C2 (d63-comparative-phrasal.md §5.3): a gradable adjective's `deg` projects onto its
        // derivationally-related NOUN (`+` link, `dependent` → `dependence`) as a `cat_measure`
        // reading, so `greater/less <nominalization>` parses. Here `lethal` → `lethality`.
        let lethal = syn("00000001 00 a 01 lethal 0 001 + 00000002 n 0000 | causing death");
        let lethality = syn("00000002 00 n 01 lethality 0 000 | the quality of being lethal");
        assert_eq!(
            lethal.derivational,
            vec![("00000002".to_string(), "n".to_string())]
        );
        let noun_index: BTreeMap<_, _> = [(lethality.offset.clone(), &lethality)]
            .into_iter()
            .collect();
        let mut rep = Report::default();
        let mut buf = String::new();
        push_adj(&mut buf, &lethal, &mut rep, &noun_index, &SenseRanks::new());
        // the noun `lethality` gets a `cat_measure` reading whose sem IS the adjective's `deg`.
        assert!(
            buf.contains("lexicon:form     = \"lethality\";"),
            "projected noun entry:\n{buf}"
        );
        assert!(
            buf.contains("lexicon:sem      = wn:deg_a00000001;"),
            "sem = the adjective's deg:\n{buf}"
        );
        assert!(buf.contains("lexicon:cat      = type_expr( lexicon:cat_measure );"));
    }

    #[test]
    fn governed_preposition_from_gloss() {
        // (1) WordNet's explicit `followed by `to'` convention (`proportional`).
        assert_eq!(
            governed_preposition(
                "properly related in size or degree; usually followed by `to'",
                "proportional"
            ),
            Some("to".to_string())
        );
        // (2) lemma in the gloss/example → its preposition, PER-LEMMA within one synset.
        let g = "compulsively or physiologically dependent on something; \"she is addicted to chocolate\"";
        assert_eq!(governed_preposition(g, "addicted"), Some("to".to_string()));
        assert_eq!(governed_preposition(g, "dependent"), Some("on".to_string()));
        // non-relational: no governance signal.
        assert_eq!(governed_preposition("of great size", "large"), None);
        // lemma-keyed avoids verb+prep noise (the prep follows a VERB, not the lemma).
        assert_eq!(
            governed_preposition("\"she walked with a limp\"", "temperate"),
            None
        );
    }

    #[test]
    fn governed_preposition_falls_back_to_curated_frames() {
        // Fix A piece (a): the REAL "dependent" synset glosses are "addicted to a drug" / "contingent on
        // something else" — no "dependent on", so the gloss heuristic yields NONE. The curated frame file
        // (adjective-frames.tsv) supplies the governed preposition.
        assert_eq!(
            governed_preposition("contingent on something else", "dependent"),
            Some("on".to_string())
        );
        assert_eq!(
            governed_preposition("absolutely necessary; vitally necessary", "essential"),
            Some("for".to_string())
        );
        // A gloss-derived prep still wins (the fallback only fires when the gloss yields none).
        assert_eq!(
            governed_preposition("usually followed by `to'", "proportional"),
            Some("to".to_string())
        );
        // An adjective in neither the gloss nor the frame file stays non-relational.
        assert_eq!(governed_preposition("of great size", "large"), None);
    }

    #[test]
    fn relational_gradable_adjective_emits_ground_taking_measure() {
        // C3: a gradable adjective whose gloss governs a preposition (`dependent on`) also gets a 2-place
        // measure `deg_rel` + a `cat_measure/cat_pp_arg` reading (the ground `on X` fills the first arg),
        // so `more dependent on WRN` threads the ground. The bare 1-place measure (C1) stays.
        let dependent = syn(
            "00000001 00 a 01 dependent 0 000 | contingent on something; \"dependent on charity\"",
        );
        let mut rep = Report::default();
        let mut buf = String::new();
        push_adj(
            &mut buf,
            &dependent,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
        );
        assert!(
            buf.contains(
                "axiom wn:deg_a00000001_rel : lexicon:Entity -> lexicon:Entity -> core:float"
            ),
            "2-place relational measure:\n{buf}"
        );
        assert!(
            buf.contains(
                "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:cat_measure, lexicon:cat_pp_arg(lexicon:prep_on)) );"
            ),
            "relational cat_measure/cat_pp_arg(prep_on) reading — the gloss `dependent on` governs `on` \
             (C3-precision):\n{buf}"
        );
        assert!(buf.contains("lexicon:sem      = wn:deg_a00000001_rel;"));
        // the bare 1-place measure (C1) is STILL present for the ground-less reading.
        assert!(buf.contains("lexicon:sem      = wn:deg_a00000001;"));
    }

    #[test]
    fn relational_gradable_adjective_emits_positive_predication() {
        // C3-positive (Fix A (c)): the relational adjective ALSO gets a POSITIVE predicative reading
        // `(S[adj]\NP)/cat_pp_arg(prep)` — consume the governed PP (the ground), compare the 2-place
        // measure to the ABSOLUTE standard: `λr.λx. gt(deg_rel(r, x), std)`. Without it "dependent ON
        // WRN" / "concordant WITH X" had no reading binding the relatum into the degree — only the
        // comparative consumed the `cat_measure` form — so the PP stranded as a free VP-adjunct.
        let dependent = syn(
            "00000001 00 a 01 dependent 0 000 | contingent on something; \"dependent on charity\"",
        );
        let mut rep = Report::default();
        let mut buf = String::new();
        push_adj(
            &mut buf,
            &dependent,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
        );
        // The positive-relational sem: the 2-place measure vs the absolute standard.
        assert!(
            buf.contains("measurements:gt(wn:deg_a00000001_rel(r, x), wn:std_a00000001)"),
            "positive relational sem `gt(deg_rel(r, x), std)`:\n{buf}"
        );
        // The predicative category taking the governed PP as its argument.
        assert!(
            buf.contains(
                "lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_pp_arg(lexicon:prep_on)) );"
            ),
            "positive relational cat `(S[adj]\\NP)/cat_pp_arg(prep_on)`:\n{buf}"
        );
        assert!(buf.contains("lexicon:sem      = wn:pos_rel_sem_a00000001;"));
        // A NON-relational gradable adjective gets no positive-relational entry (nothing to ground).
        let mut buf2 = String::new();
        push_adj(
            &mut buf2,
            &syn("00000002 00 a 01 large 0 000 | of great size"),
            &mut Report::default(),
            &BTreeMap::new(),
            &SenseRanks::new(),
        );
        assert!(
            !buf2.contains("pos_rel_sem_"),
            "a non-governed adjective must not get a positive-relational reading:\n{buf2}"
        );
    }

    #[test]
    fn relational_adjective_stays_boolean() {
        // A relational (pertainym `\`) adjective is non-gradable → the Boolean predicate.
        let atomic = syn("00000004 00 a 01 atomic 0 001 \\ 00000005 n 0000 | of atoms");
        assert!(
            atomic.relational,
            "pertainym `\\` marks a relational adjective"
        );
        let mut rep = Report::default();
        let mut buf = String::new();
        push_adj(
            &mut buf,
            &atomic,
            &mut rep,
            &BTreeMap::new(),
            &SenseRanks::new(),
        );
        assert!(buf.contains("axiom wn:a00000004 : lexicon:Entity -> Prop"));
        assert!(buf.contains("lexicon:form     = \"atomic\";"));
        // no measure / comparative for a relational adjective.
        assert!(!buf.contains("deg_a00000004"));
        assert!(!buf.contains("cat_pp_than"));
    }

    #[test]
    fn verb_with_only_deferred_frames_is_skipped() {
        // frame 29 "Somebody ----s whether CLAUSE" — interrogative complement, still
        // deferred (not guessed). (Frame 26, the declarative that-clause, is now emitted.)
        let v = syn("00000001 00 v 01 cogitate 0 000 01 + 29 00 | think");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(!push_verb(&mut buf, &v, &mut rep, &SenseRanks::new()));
        assert_eq!(rep.verbs_deferred, 1);
        assert!(buf.is_empty());
    }

    #[test]
    fn document_has_header_and_separates_decls_from_entries() {
        let nouns = [
            syn("00001740 03 n 01 entity 0 000 | the root"),
            syn("05444328 08 n 01 gene 0 001 @ 00001740 n 0000 | a gene"),
        ];
        let (doc, rep) = render_document(&nouns, &SenseRanks::new(), &MassNouns::new());
        assert!(doc.contains("namespace wn         = \"urn:eigenius:wn\";"));
        assert!(doc.contains("class wn:n00001740 : lexicon:Entity {"));
        assert!(doc.contains("class wn:n05444328 : wn:n00001740 {"));
        // a class declaration must appear before the entry section
        let class_pos = doc.find("class wn:n05444328").unwrap();
        let entry_pos = doc.find("resource wn:e_n05444328_0").unwrap();
        assert!(class_pos < entry_pos);
        assert_eq!(rep.noun_classes, 2);
    }
}
