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
//!   argument types are generic at the noun root [`ENTITY_ROOT`], so the verb
//!   composes with any noun by subsumption (§8.6).
//! - adjective synset → predicative `eigentt:Axiom` (`S\NP`).
//! - each lemma → a `lexicon:LexicalEntry`; `sem_type = ⟦cat⟧` by construction
//!   (the same `⟦·⟧` the kernel gate checks), so entries are felicitous by
//!   construction and the gate is a confirmation.

use crate::wndb::{Pos, Synset};

/// `entity.n.01` — the noun root; the stage-1 generic argument type for verbs
/// and adjectives. Must be present in any import that emits verbs/adjectives.
pub const ENTITY_ROOT: &str = "wn:n00001740";

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
}

impl FrameKind {
    fn tag(self) -> &'static str {
        match self {
            FrameKind::Intransitive => "i",
            FrameKind::Transitive => "t",
            FrameKind::Ditransitive => "d",
        }
    }

    /// The axiom / `sem_type` arrow — every slot generic at the noun root
    /// (stage-1; §8.7.4), so the verb composes with any noun by subsumption.
    fn arrow(self) -> String {
        match self {
            FrameKind::Intransitive => format!("{ENTITY_ROOT} -> Prop"),
            FrameKind::Transitive => format!("{ENTITY_ROOT} -> {ENTITY_ROOT} -> Prop"),
            FrameKind::Ditransitive => {
                format!("{ENTITY_ROOT} -> {ENTITY_ROOT} -> {ENTITY_ROOT} -> Prop")
            }
        }
    }

    /// The `lexicon:Cat` term (object-first; `⟦cat⟧` equals [`Self::arrow`]).
    /// Features (D63 §5.1): NP slots are number-underspecified (`num_any`), the
    /// result sentence is declarative + finite (`cat_s(dcl, fin)`).
    fn cat(self) -> String {
        let np = format!("lexicon:cat_np({ENTITY_ROOT}, lexicon:num_any)");
        let s = "lexicon:cat_s(lexicon:dcl, lexicon:fin)";
        match self {
            FrameKind::Intransitive => format!("lexicon:bwd({s}, {np})"),
            FrameKind::Transitive => format!("lexicon:fwd(lexicon:bwd({s}, {np}), {np})"),
            FrameKind::Ditransitive => {
                format!("lexicon:fwd(lexicon:fwd(lexicon:bwd({s}, {np}), {np}), {np})")
            }
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
        _ => None, // 5,6,7 predicative; 24,25,26,28,29,30,32,33,34,35 clausal/control
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
    let header = if parents.is_empty() {
        format!("class wn:{} {{", local(syn))
    } else {
        format!("class wn:{} : {} {{", local(syn), parents.join(", "))
    };
    buf.push_str(&format!(
        "{header}\n    description = \"{}\";\n}}\n\n",
        esc(&syn.gloss)
    ));
    rep.noun_classes += 1;
    for (i, lemma) in syn.words.iter().enumerate() {
        push_entry(
            buf,
            &format!("e_{}_{i}", local(syn)),
            lemma,
            "lexicon:cat_n(lexicon:num_any)",
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
/// (`cat_np(C)`, `sem` = this resource), typed at the **first** class, which is
/// what the kernel infers for a multi-class resource (`is_a().first()`) and what
/// the felicity gate checks; the other classes stay on the resource (no drop).
fn push_instance(buf: &mut String, syn: &Synset, rep: &mut Report) {
    // Types: `@i` first (the instance-hypernyms), then any rare plain `@`.
    let classes: Vec<String> = syn
        .instance_of
        .iter()
        .chain(syn.hypernyms.iter())
        .map(|h| format!("wn:n{h}"))
        .collect();
    let primary = classes
        .first()
        .expect("push_instance requires a non-empty instance_of");
    buf.push_str(&format!(
        "resource wn:{} : {} {{\n    core:description = \"{}\";\n}}\n\n",
        local(syn),
        classes.join(", "),
        esc(&syn.gloss),
    ));
    rep.instances += 1;
    let cat = format!("lexicon:cat_np({primary}, lexicon:num_any)");
    for (i, lemma) in syn.words.iter().enumerate() {
        push_entry(
            buf,
            &format!("e_{}_{i}", local(syn)),
            lemma,
            &cat,
            &local(syn),
            primary,
            &sense_key(syn, lemma),
        );
        rep.entries += 1;
    }
}

/// Verb synset → an `eigentt:Axiom` + entries **per distinct emittable frame
/// kind** (a verb with both intransitive and transitive frames yields both — its
/// alternations). Returns `false` (deferred) when no frame is emittable.
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
        let cat = kind.cat();
        let arrow = kind.arrow();
        for (i, lemma) in syn.words.iter().enumerate() {
            push_entry(
                buf,
                &format!("e_v{off}_{tag}_{i}"),
                lemma,
                &cat,
                &format!("v{off}_{tag}"),
                &arrow,
                &sense_key(syn, lemma),
            );
            rep.entries += 1;
        }
    }
    true
}

/// Adjective synset → a predicative `eigentt:Axiom` (`S\NP`) + entries.
fn push_adj(buf: &mut String, syn: &Synset, rep: &mut Report) {
    buf.push_str(&format!(
        "axiom wn:{} : {ENTITY_ROOT} -> Prop\n\n",
        local(syn)
    ));
    rep.adj_axioms += 1;
    let cat = format!(
        "lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np({ENTITY_ROOT}, lexicon:num_any))"
    );
    let arrow = format!("{ENTITY_ROOT} -> Prop");
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
/// closure (every `@` parent + [`ENTITY_ROOT`] present); rendering is order-
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
        // deferred higher-order frames → None (never guessed).
        assert_eq!(classify(5), None); // predicative complement
        assert_eq!(classify(26), None); // that CLAUSE
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
        assert!(buf.contains("lexicon:cat      = type_expr( lexicon:cat_n(lexicon:num_any) );"));
        assert!(buf.contains("lexicon:sem      = wn:n05444328;"));
        assert!(buf.contains("lexicon:sem_type = type_expr( Set );"));
        assert_eq!(rep.noun_classes, 1);
        assert_eq!(rep.entries, 3); // gene, cistron, factor
    }

    #[test]
    fn root_noun_has_no_parent_clause() {
        let entity = syn("00001740 03 n 01 entity 0 001 ~ 00001930 n 0000 | that which exists");
        let mut buf = String::new();
        push_noun(&mut buf, &entity, &mut Report::default());
        assert!(buf.contains("class wn:n00001740 {")); // no `:` parents
        assert!(!buf.contains("class wn:n00001740 :"));
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
        assert!(buf.contains("resource wn:e_n10954498_0 : lexicon:LexicalEntry {"));
        assert!(buf.contains("lexicon:form     = \"Einstein\";"));
        assert!(buf.contains("lexicon:form     = \"Albert Einstein\";"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:cat_np(wn:n10428004, lexicon:num_any) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:n10954498;"));
        assert!(buf.contains("lexicon:sem_type = type_expr( wn:n10428004 );"));
        assert_eq!(rep.instances, 1);
        assert_eq!(rep.noun_classes, 0);
        assert_eq!(rep.entries, 2); // Einstein, Albert Einstein
    }

    #[test]
    fn multi_instance_of_keeps_all_classes_types_at_first() {
        // A rare instance of two classes: `resource r : A, B` (no drop); the NP
        // entry types at the FIRST (what the kernel infers via is_a().first()).
        let v = syn("00000009 18 n 01 Enlightenment 0 002 @i 15254028 n 0000 @ 08473623 n 0000 | a movement");
        let mut rep = Report::default();
        let mut buf = String::new();
        push_instance(&mut buf, &v, &mut rep);
        // both classes on the resource — @i first, then the rare plain @.
        assert!(buf.contains("resource wn:n00000009 : wn:n15254028, wn:n08473623 {"));
        // entry types at the first class only (exact-match gate).
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:cat_np(wn:n15254028, lexicon:num_any) );"
        ));
        assert!(buf.contains("lexicon:sem_type = type_expr( wn:n15254028 );"));
        assert_eq!(rep.instances, 1);
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
        assert!(doc.contains("class wn:n10428004 {"));
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
        assert!(buf.contains("axiom wn:v00275082_t : wn:n00001740 -> wn:n00001740 -> Prop"));
        assert!(buf.contains(
            "lexicon:cat      = type_expr( lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(wn:n00001740, lexicon:num_any)), lexicon:cat_np(wn:n00001740, lexicon:num_any)) );"
        ));
        assert!(buf.contains("lexicon:sem      = wn:v00275082_t;"));
        assert_eq!(rep.verb_axioms, 1);
        assert_eq!(rep.entries, 3); // corrode, eat, rust
    }

    #[test]
    fn verb_alternation_emits_one_axiom_per_kind() {
        // frames 2 (intransitive) + 8 (transitive) → BOTH axioms (the verb's
        // alternations), each with its own kind-tagged IRI.
        let v = syn("00001740 29 v 01 breathe 0 000 02 + 02 00 + 08 00 | respire");
        let mut rep = Report::default();
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut rep));
        assert!(buf.contains("axiom wn:v00001740_i : wn:n00001740 -> Prop"));
        assert!(buf.contains("axiom wn:v00001740_t : wn:n00001740 -> wn:n00001740 -> Prop"));
        assert_eq!(rep.verb_axioms, 2);
        assert_eq!(rep.entries, 2); // one lemma × two kinds
    }

    #[test]
    fn ditransitive_verb_curries_three_entity_slots() {
        // frame 14 "Somebody ----s somebody something" → ditransitive.
        let v = syn("00001234 30 v 01 give 0 000 01 + 14 00 | transfer");
        let mut buf = String::new();
        assert!(push_verb(&mut buf, &v, &mut Report::default()));
        assert!(buf.contains(
            "axiom wn:v00001234_d : wn:n00001740 -> wn:n00001740 -> wn:n00001740 -> Prop"
        ));
        assert!(buf.contains("lexicon:fwd(lexicon:fwd(lexicon:bwd(lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(wn:n00001740, lexicon:num_any)), lexicon:cat_np(wn:n00001740, lexicon:num_any)), lexicon:cat_np(wn:n00001740, lexicon:num_any))"));
    }

    #[test]
    fn verb_with_only_deferred_frames_is_skipped() {
        // frame 26 "Somebody ----s that CLAUSE" — clausal, deferred (not guessed).
        let v = syn("00000001 00 v 01 cogitate 0 000 01 + 26 00 | think");
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
        assert!(doc.contains("class wn:n00001740 {"));
        assert!(doc.contains("class wn:n05444328 : wn:n00001740 {"));
        // a class declaration must appear before the entry section
        let class_pos = doc.find("class wn:n05444328").unwrap();
        let entry_pos = doc.find("resource wn:e_n05444328_0").unwrap();
        assert!(class_pos < entry_pos);
        assert_eq!(rep.noun_classes, 2);
    }
}
