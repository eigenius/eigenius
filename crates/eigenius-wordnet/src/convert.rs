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

use crate::inflect::{gerund, past_participles, third_singular};
use crate::wndb::{Pos, Synset};

/// The **entity top** (D63 §8.3, decision ii): the schema-level foundational
/// entity type (`lexicon:Entity`) that verb/adjective argument slots — and the
/// determiners' subject `E` — are typed at. WordNet's `entity.n.01`
/// (`wn:n00001740`, the noun-lattice root) is rooted here, so every imported
/// noun is `≤ lexicon:Entity` and flows into these slots by coercive subtyping.
/// Provided by the bootstrapped lexicon schema, which the import builds on.
pub const ENTITY_TOP: &str = "lexicon:Entity";

/// The namespace + schema preamble the emitted entries reference.
pub const ESL_HEADER: &str = "\
namespace core       = \"urn:eigenius:core\";
namespace reflection = \"urn:eigenius:reflection\";
namespace epistemic  = \"urn:eigenius:reflection:epistemic\";
namespace eigentt    = \"urn:eigenius:eigentt\";
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace wn         = \"urn:eigenius:wn\";
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
}

/// The emittable categorial shapes a verb frame maps to. Higher-order shapes
/// (predicative-complement, clausal, control / raising) are not emittable at
/// stage-1 and are deferred (§8.7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FrameKind {
    Intransitive,
    Transitive,
    Ditransitive,
    /// Clause-taking (report) verb — frame 26, "Somebody ----s that CLAUSE" (D63 §8.11
    /// 6-cl): an opaque `Prop → Entity → Prop` axiom, category `(S\NP)/cat_cp`.
    Clausal,
}

impl FrameKind {
    fn tag(self) -> &'static str {
        match self {
            FrameKind::Intransitive => "i",
            FrameKind::Transitive => "t",
            FrameKind::Ditransitive => "d",
            FrameKind::Clausal => "c",
        }
    }

    /// The axiom / `sem_type` arrow — every slot generic at the noun root
    /// (stage-1; §8.7.4), so the verb composes with any noun by subsumption. The
    /// clausal report verb leads with the propositional complement (`Prop`).
    fn arrow(self) -> String {
        match self {
            FrameKind::Intransitive => format!("{ENTITY_TOP} -> Prop"),
            FrameKind::Transitive => format!("{ENTITY_TOP} -> {ENTITY_TOP} -> Prop"),
            FrameKind::Ditransitive => {
                format!("{ENTITY_TOP} -> {ENTITY_TOP} -> {ENTITY_TOP} -> Prop")
            }
            FrameKind::Clausal => format!("Prop -> {ENTITY_TOP} -> Prop"),
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
            FrameKind::Intransitive => format!("lexicon:bwd({s}, {subj})"),
            FrameKind::Transitive => format!("lexicon:fwd(lexicon:bwd({s}, {subj}), {obj})"),
            FrameKind::Ditransitive => {
                format!("lexicon:fwd(lexicon:fwd(lexicon:bwd({s}, {subj}), {obj}), {obj})")
            }
            // Clause-taking: `(S\NP)/cat_cp` — the complement is an embedded clause.
            FrameKind::Clausal => format!("lexicon:fwd(lexicon:bwd({s}, {subj}), lexicon:cat_cp)"),
        }
    }
}

/// Map a WordNet sentence frame (1–35; `wninput(5WN)`) to an emittable kind, or
/// `None` for the **deferred** higher-order frames:
///   - 5, 6, 7 — predicative complement (`----s Adjective/Noun`);
///   - 26, 29, 34 — clausal complement (`that` / `whether CLAUSE`);
///   - 24, 25, 28, 30, 32, 33, 35 — control / raising (INFINITIVE / V-ing).
///
/// PP-oblique frames are mapped **coarsely** — the PP object becomes an entity
/// argument: 12/13/27 → transitive (e.g. *depend on*), 20/21 → transitive,
/// 4/22 → intransitive (the PP is dropped). Documented as a stage-1 loss.
fn classify(frame: u8) -> Option<FrameKind> {
    match frame {
        1 | 2 | 3 | 4 | 22 | 23 => Some(FrameKind::Intransitive),
        8 | 9 | 10 | 11 | 12 | 13 | 20 | 21 | 27 => Some(FrameKind::Transitive),
        14 | 15 | 16 | 17 | 18 | 19 | 31 => Some(FrameKind::Ditransitive),
        26 => Some(FrameKind::Clausal), // "Somebody ----s that CLAUSE" (D63 §8.11 6-cl)
        _ => None, // 5,6,7 predicative; 29,34 whether-clause; 24,25,28,30,32,33,35 control/raising
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
fn push_entry(
    buf: &mut String,
    entry_id: &str,
    form: &str,
    cat: &str,
    sem: &str,
    sem_type: &str,
    sense: &str,
) {
    buf.push_str(&format!(
        "resource wn:{entry_id} : lexicon:LexicalEntry {{\n\
         \x20   lexicon:form     = \"{form}\";\n\
         \x20   lexicon:cat      = type_expr( {cat} );\n\
         \x20   lexicon:sem      = wn:{sem};\n\
         \x20   lexicon:sem_type = type_expr( {sem_type} );\n\
         \x20   lexicon:sense    = \"{sense}\";\n\
         \x20   lexicon:grade    = epistemic:declared;\n\
         }}\n\n",
        form = esc(form),
    ));
}

/// Noun synset → a `core:Class` (with `subclass_of` from `@`) + one `N` entry
/// per lemma.
fn push_noun(buf: &mut String, syn: &Synset, rep: &mut Report) {
    let parents: Vec<String> = syn.hypernyms.iter().map(|h| format!("wn:n{h}")).collect();
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
    for (i, lemma) in syn.words.iter().enumerate() {
        push_entry(
            buf,
            &format!("e_{}_{i}", local(syn)),
            lemma,
            &cat,
            &local(syn),
            "Set",
            &sense_key(syn, lemma),
        );
        rep.entries += 1;
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
fn push_instance(buf: &mut String, syn: &Synset, rep: &mut Report) {
    // Types: `@i` first (the instance-hypernyms), then any rare plain `@`.
    let classes: Vec<String> = syn
        .instance_of
        .iter()
        .chain(syn.hypernyms.iter())
        .map(|h| format!("wn:n{h}"))
        .collect();
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
            );
            rep.entries += 1;
        }
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
fn push_verb(buf: &mut String, syn: &Synset, rep: &mut Report) -> bool {
    let kinds: std::collections::BTreeSet<FrameKind> =
        syn.frames.iter().filter_map(|&f| classify(f)).collect();
    if kinds.is_empty() {
        rep.verbs_deferred += 1;
        return false;
    }
    let off = &syn.offset;
    for kind in kinds {
        let tag = kind.tag();
        buf.push_str(&format!("axiom wn:v{off}_{tag} : {}\n\n", kind.arrow()));
        rep.verb_axioms += 1;
        let sem = format!("v{off}_{tag}");
        let arrow = kind.arrow();
        let cat_bse = kind.cat("bse", "num_any");
        let (cat_fin_sg, cat_fin_pl) = (kind.cat("fin", "sg"), kind.cat("fin", "pl"));
        let (cat_ger, cat_pss) = (kind.cat("ger", "num_any"), kind.cat("pss", "num_any"));
        for (i, lemma) in syn.words.iter().enumerate() {
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
            );
            rep.entries += 1;
            rep.participle_entries += 1;
            // Past participle(s) — perfect/passive ("has/is affected"); table-or-regular.
            for (k, pp) in head_pps(lemma).iter().enumerate() {
                let id = format!("e_v{off}_{tag}_{i}_p{k}");
                push_entry(buf, &id, pp, &cat_pss, &sem, &arrow, &sense);
                rep.entries += 1;
                rep.participle_entries += 1;
            }
        }
    }
    true
}

/// Adjective synset → a predicative `eigentt:Axiom` (`S\NP`) + entries.
fn push_adj(buf: &mut String, syn: &Synset, rep: &mut Report) {
    buf.push_str(&format!(
        "axiom wn:{} : {ENTITY_TOP} -> Prop\n\n",
        local(syn)
    ));
    rep.adj_axioms += 1;
    // Predicative adjective is the **adjectival** predicate form (`adj`) — distinct
    // from base verbs, so it requires the copula (`is`/`are`) and never do-support
    // (D63 §8.5 Slice 3a/3b). A bare `*X large` is not a finite root; `X is large`
    // composes the copula with this; attributive `large X` is the engine Σ-rule.
    let cat = format!(
        "lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np({ENTITY_TOP}, lexicon:num_any))"
    );
    let arrow = format!("{ENTITY_TOP} -> Prop");
    for (i, lemma) in syn.words.iter().enumerate() {
        push_entry(
            buf,
            &format!("e_{}_{i}", local(syn)),
            lemma,
            &cat,
            &local(syn),
            &arrow,
            &sense_key(syn, lemma),
        );
        rep.entries += 1;
    }
}

/// Render a set of synsets to one ESL document. The caller is responsible for
/// closure (every `@` parent + [`ENTITY_TOP`] present); rendering is order-
/// independent (references resolve at layer time). Output is deterministic:
/// synsets are emitted sorted by `(pos, offset)`, declarations before entries.
pub fn render_document(synsets: &[Synset]) -> (String, Report) {
    let mut sorted: Vec<&Synset> = synsets.iter().collect();
    sorted.sort_by(|a, b| (a.pos, &a.offset).cmp(&(b.pos, &b.offset)));

    let mut rep = Report::default();
    let mut decls = String::new(); // classes + axioms
    let mut entries = String::new();

    for syn in sorted {
        match syn.pos {
            Pos::Noun => {
                let mut block = String::new();
                if syn.instance_of.is_empty() {
                    push_noun(&mut block, syn, &mut rep);
                } else {
                    push_instance(&mut block, syn, &mut rep);
                }
                route(&block, &mut decls, &mut entries);
            }
            Pos::Verb => {
                let mut block = String::new();
                if push_verb(&mut block, syn, &mut rep) {
                    route(&block, &mut decls, &mut entries);
                }
            }
            Pos::Adj => {
                let mut block = String::new();
                push_adj(&mut block, syn, &mut rep);
                route(&block, &mut decls, &mut entries);
            }
            Pos::Adv => {} // deferred (§8.7.5)
        }
    }

    let doc = format!("{ESL_HEADER}\n{decls}{entries}");
    (doc, rep)
}

/// Split a rendered synset block (decl + entries) into the two output sections.
/// A block is `<class|axiom …>\n\n<resource …>\n\n…`; the first paragraph is the
/// declaration, the rest are entries.
fn route(block: &str, decls: &mut String, entries: &mut String) {
    let mut paras = block.split_inclusive("\n\n");
    if let Some(decl) = paras.next() {
        decls.push_str(decl);
    }
    for entry in paras {
        entries.push_str(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wndb::parse_data_line;

    fn syn(line: &str) -> Synset {
        parse_data_line(line).unwrap()
    }

    #[test]
    fn frame_classification_covers_all_35() {
        // intransitive / transitive / ditransitive — the emittable kinds.
        assert_eq!(classify(2), Some(FrameKind::Intransitive));
        assert_eq!(classify(22), Some(FrameKind::Intransitive)); // PP → coarse intrans
        assert_eq!(classify(8), Some(FrameKind::Transitive));
        assert_eq!(classify(13), Some(FrameKind::Transitive)); // "----s on something"
        assert_eq!(classify(14), Some(FrameKind::Ditransitive));
        assert_eq!(classify(31), Some(FrameKind::Ditransitive));
        // frame 26 "that CLAUSE" → clause-taking (D63 §8.11 6-cl).
        assert_eq!(classify(26), Some(FrameKind::Clausal));
        // still-deferred higher-order frames → None (never guessed).
        assert_eq!(classify(5), None); // predicative complement
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
        push_noun(&mut buf, &gene, &mut rep);
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
    fn root_noun_is_rooted_at_the_schema_entity_top() {
        // WordNet's hypernym-less root `entity.n.01` is parented at the schema
        // entity top `lexicon:Entity` (D63 §8.3 ii), so the whole noun lattice
        // sits under it.
        let entity = syn("00001740 03 n 01 entity 0 001 ~ 00001930 n 0000 | that which exists");
        let mut buf = String::new();
        push_noun(&mut buf, &entity, &mut Report::default());
        assert!(buf.contains("class wn:n00001740 : lexicon:Entity {"));
    }

    #[test]
    fn instance_synset_is_an_individual_not_a_class() {
        // Einstein `@i` physicist (10428004): an NP individual, not a class.
        let einstein = syn("10954498 18 n 02 Einstein 0 Albert_Einstein 0 002 @i 10428004 n 0000 + 03031247 a 0301 | a physicist");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_instance(&mut buf, &einstein, &mut rep);
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
        push_instance(&mut buf, &v, &mut rep);
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
    fn render_routes_instances_away_from_classes() {
        // A class (gene) and an individual (Einstein) in one document: the gene
        // is a `class`, Einstein a `resource` instance — the routing split.
        let synsets = [
            syn("10428004 18 n 01 physicist 0 000 | a scientist"),
            syn("10954498 18 n 01 Einstein 0 001 @i 10428004 n 0000 | a physicist"),
        ];
        let (doc, rep) = render_document(&synsets);
        assert!(doc.contains("class wn:n10428004 : lexicon:Entity {"));
        assert!(doc.contains("resource wn:n10954498 : wn:n10428004 {"));
        assert!(!doc.contains("class wn:n10954498"));
        assert_eq!(rep.noun_classes, 1);
        assert_eq!(rep.instances, 1);
    }

    #[test]
    fn transitive_verb_axiom_and_object_first_category() {
        let eat = syn("00275082 30 v 03 corrode 1 eat 0 rust 1 001 @ 00259743 v 0000 01 + 11 00 | to deteriorate");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &eat, &mut rep));
        // frame 11 → transitive; the axiom IRI is kind-tagged (`_t`).
        assert!(buf.contains("axiom wn:v00275082_t : lexicon:Entity -> lexicon:Entity -> Prop"));
        // Finite 3sg has a SINGULAR subject slot (6-agr); object slot stays num_any.
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:v00275082_t;"));
        // base (num_any) + finite 3sg ("eats", sg) + finite plural ("eat", pl) forms.
        assert!(buf.contains("lexicon:form     = \"eat\";")); // bse + fin-pl (lemma surface)
        assert!(buf.contains("lexicon:form     = \"eats\";")); // fin 3sg
                                                               // bse keeps a num_any subject (the aux supplies agreement).
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:bse), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        // plural-finite has a PLURAL subject slot (6-agr).
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:pl)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        assert_eq!(rep.verb_axioms, 1);
        // Per lemma: base + finite 3sg + finite plural + gerund + past participle →
        // 3 lemmas × 5 = 15 entries; 6 of them participles (ger + pss).
        assert_eq!(rep.entries, 15);
        assert_eq!(rep.participle_entries, 6);
    }

    #[test]
    fn verb_emits_participle_forms_with_ger_and_pss_categories() {
        // D63 §8.9 6-aux: per verb lemma, the importer also emits the generated present
        // participle (`ger`, progressive) and past participle (`pss`, perfect/passive),
        // pointing at the SAME axiom, differing only in the result clause's Fin feature.
        let eat = syn("00275082 30 v 03 corrode 1 eat 0 rust 1 001 @ 00259743 v 0000 01 + 11 00 | to deteriorate");
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &eat, &mut Report::default()));
        // gerund (regular -ing) + its `ger` category.
        assert!(buf.contains("lexicon:form     = \"eating\";"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:ger), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"
        ));
        // irregular past participle (eat → eaten) + its `pss` category, same axiom.
        assert!(buf.contains("lexicon:form     = \"eaten\";"));
        assert!(buf.contains("lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:pss), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );"));
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
        assert!(push_verb(&mut buf, &v, &mut Report::default()));
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
        assert!(push_verb(&mut buf, &v, &mut rep));
        assert!(buf.contains("axiom wn:v00001740_i : lexicon:Entity -> Prop"));
        assert!(buf.contains("axiom wn:v00001740_t : lexicon:Entity -> lexicon:Entity -> Prop"));
        assert_eq!(rep.verb_axioms, 2);
        // one lemma × two kinds × (base + 3sg + plural-finite + gerund + 1 pp) = 10.
        assert_eq!(rep.entries, 10);
    }

    #[test]
    fn ditransitive_verb_curries_three_entity_slots() {
        // frame 14 "Somebody ----s somebody something" → ditransitive.
        let v = syn("00001234 30 v 01 give 0 000 01 + 14 00 | transfer");
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut Report::default()));
        assert!(buf.contains(
            "axiom wn:v00001234_d : lexicon:Entity -> lexicon:Entity -> lexicon:Entity -> Prop"
        ));
        assert!(buf.contains("lexicon:fwd(lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any))"));
    }

    #[test]
    fn clausal_verb_emits_report_axiom_and_cp_category() {
        // frame 26 → clause-taking report verb (D63 §8.11 6-cl): an opaque
        // `Prop → Entity → Prop` axiom and the category `(S\NP)/cat_cp`.
        let v = syn("00000003 31 v 01 show 0 000 01 + 26 00 | demonstrate");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut rep));
        assert!(buf.contains("axiom wn:v00000003_c : Prop -> lexicon:Entity -> Prop"));
        assert!(buf.contains("lexicon:form     = \"shows\";")); // 3sg
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_cp) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:v00000003_c;"));
    }

    #[test]
    fn verb_with_only_deferred_frames_is_skipped() {
        // frame 29 "Somebody ----s whether CLAUSE" — interrogative complement, still
        // deferred (not guessed). (Frame 26, the declarative that-clause, is now emitted.)
        let v = syn("00000001 00 v 01 cogitate 0 000 01 + 29 00 | think");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(!push_verb(&mut buf, &v, &mut rep));
        assert_eq!(rep.verbs_deferred, 1);
        assert!(buf.is_empty());
    }

    #[test]
    fn document_has_header_and_separates_decls_from_entries() {
        let nouns = [
            syn("00001740 03 n 01 entity 0 000 | the root"),
            syn("05444328 08 n 01 gene 0 001 @ 00001740 n 0000 | a gene"),
        ];
        let (doc, rep) = render_document(&nouns);
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
