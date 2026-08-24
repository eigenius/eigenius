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

//! Verbalizer — render a reading's `sem` back to approximate English. **FAIL-HONEST**: any
//! construct it does not understand is emitted as `⟦raw⟧`, never smoothed into fluent-but-wrong
//! English — a partial gloss must LOOK partial.
//!
//! Two consumers share this one renderer, deliberately (the same argument as `dcg::skeleton`):
//! the measurement harness's expected-reading verification (a skeleton is hard to check by eye;
//! "every nucleotide-repeat region is a microsatellite" is easy), and the reading-selection stage
//! ([`crate::dcg::pipeline`] / D63 reading selection), whose candidates are presented to the
//! ranker as these glosses. If the gate's renderer and the selector's renderer diverged, the gate
//! could verify a reading the selector never saw.
//!
//! Sense NAMING uses the LOADED lexicon, never a source vocabulary's data files: names come from
//! the loaded entries' `sense` keys ([`unit_sense_names`] — the unit's own tokens yield the
//! actual surface lemma, not just a synonym) and from resource descriptions on the layer
//! ([`resource_label`]). Two limits remain: a sem atom not contributed by any single token falls
//! back to the layer/local name, and generalized-quantifier (Π-CPS) sems outside the known shapes
//! are bracketed, not verbalized.
//!
//! **Lexicon coupling.** This module has NO dependency on any importer crate — every name
//! resolves through the loaded layer (`core:description`, entry `sense` keys). What it does carry
//! is the seeded importers' *string conventions*: the `wn:…`/`umls:…` sense-key layouts, the
//! CUI-in-local-name reconstruction (`cui_label`), the `v{offset}_{frame}`/`deg_*` atom-naming
//! scheme, and two description-format tolerances in [`resource_label`]. Against a lexicon
//! following none of these the verbaliser still runs and degrades honestly: names fall back to
//! the IRI local name, structure to the ⟦…⟧ bracket. The structural replacement — importers emit
//! a first-class preferred-label property, read generically here — is folded into the
//! candidate-label work (`docs/notes/d64-demonstratives-as-holes.md` §4 slice 4) and is NOT built.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::dcg::pretty::pretty_term;
use crate::dcg::segment::tokenize;
use crate::dcg::{Lemmatizer, Parser};
use crate::layer::Layer;
use crate::nbe::term::{Exp, Patt};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;

/// `sense key → display name` for every sense reachable from a unit's tokens — read off the
/// LOADED lexicon's entry `sense` keys, so it is the seeded data. Parses the seeded importers'
/// key conventions: `wn:{lemma}.{tag}.{offset}` → the lemma itself, and `umls:{CUI}` → the
/// concept's layer label. A key following neither convention contributes no name (callers fall
/// back to the local name).
pub fn unit_sense_names(
    text: &str,
    index: &Parser,
    lem: &dyn Lemmatizer,
    layer: &Arc<Layer>,
) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for tok in tokenize(text) {
        let tok = tok.trim_matches(|c: char| !c.is_alphanumeric()); // shed attached commas/periods
        for (_closed, _cat, sense) in index.debug_form_entries(tok, lem) {
            // `wn:{lemma}.{tag}.{offset}` — split from the RIGHT: offset, tag, then the lemma (which
            // may itself contain '.').
            if let Some(rest) = sense.strip_prefix("wn:") {
                let parts: Vec<&str> = rest.rsplitn(3, '.').collect(); // [offset, tag, lemma]
                if let [offset, tag, lemma] = parts.as_slice() {
                    m.entry(format!("{tag}{offset}"))
                        .or_insert_with(|| lemma.replace('_', " "));
                }
            } else if let Some(cui) = sense.strip_prefix("umls:") {
                if let Some(name) = cui_label(cui, layer) {
                    m.entry(cui.to_string()).or_insert(name);
                }
            }
        }
    }
    m
}

/// A short display label for a chain resource: its `core:description` up to the definition
/// separator. Generic over the lexicon — the caller supplies the IRI; nothing here names a
/// source vocabulary. `None` when the resource is absent or carries no string description
/// (callers fall back to the IRI local name — the fail-honest degradation).
pub fn resource_label(iri: &Iri, layer: &Arc<Layer>) -> Option<String> {
    let res = layer.resolve(iri)?;
    let d = res.get(&Iri::parse("urn:eigenius:core:description").ok()?)?;
    let Value::String(d) = d else {
        return None;
    };
    // Descriptions follow "Label — Definition …". Split at the em-dash to keep just the label.
    // Two tolerances for current importer data quirks: in UMLS-imported data the dash renders as
    // the mojibake `â…` (a separate importer encoding bug), so split on either form.
    let name = d.split('—').next().unwrap_or(d);
    let name = name.split('â').next().unwrap_or(name);
    let name = name.split(" - ").next().unwrap_or(name);
    // And a UMLS concept with NO definition has no em-dash at all — its description is just
    // `"Depletion. UMLS CUI C0333668."` — so the splits above leave the provenance suffix attached
    // and every reading mentioning it verbalises as "a Depletion. UMLS CUI C0333668. of WRN gene".
    // Cut at the suffix directly, then drop the sentence period the label is left with.
    let name = name.split(" UMLS CUI ").next().unwrap_or(name);
    Some(name.trim().trim_end_matches('.').trim().to_string())
}

/// The right-hand side of a comparative: the implicit norm (`std_a…`) or the term compared
/// against (`deg_a…(t)` — an elided «than t», recovered from the discourse).
fn comparative_standard(e: &Exp, vb: &Vb) -> String {
    let (h, a) = app_spine(e);
    match axiom_local(h) {
        Some(l) if l.starts_with("std_") => "the norm".to_string(),
        Some(l) if l.starts_with("deg_") && !a.is_empty() => verbalize(a[0], vb),
        _ => verbalize(e, vb),
    }
}

/// One concept a candidate reading names, with whatever the chain says it MEANS (D69 §4).
///
/// The decisive fact when choosing between «exonuclease activity» as the single GO concept
/// C1148824 and as `activity ⊗ exonuclease` is C1148824's definition — and it is sitting on the
/// chain, unused, while the ranker guesses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConceptNote {
    /// The atom's local name — `C1148824`, `n00407535`. What `Expanded` prints in `[…]`.
    pub id: String,
    pub label: String,
    /// The chain's `core:description` with the leading label stripped, when there is one beyond
    /// the label itself.
    pub definition: Option<String>,
}

/// Every concept named across `terms`, once each, in a stable order — the legend a presentation
/// prints beside the candidates instead of repeating definitions on every line.
pub fn concept_notes(terms: &[&Exp], vb: &Vb) -> Vec<ConceptNote> {
    let mut seen: BTreeMap<String, ConceptNote> = BTreeMap::new();
    for t in terms {
        collect_concepts(t, vb, &mut seen);
    }
    seen.into_values().collect()
}

fn collect_concepts(e: &Exp, vb: &Vb, out: &mut BTreeMap<String, ConceptNote>) {
    // Classes AND axioms: a verb or adjective atom is a choice the ranker makes just as much as
    // a noun concept is, and WordNet gives it a gloss the chain carries. Missing them left the
    // legend naming «screen» [v02533109] without saying that THAT sense is "test or examine for
    // the presence of disease or infection" while its rival is "examine in order to test
    // suitability" — the concept-vs-compound blindness one level down (found 2026-08-13).
    if let Exp::EigonClass(iri) | Exp::EigonAxiom(iri) = e {
        // The gloss prints the STRIPPED key (`name_atom` drops the `_t`/`deg_` wrappers), so the
        // legend must key on that to line up — while the DEFINITION lives at the full IRI.
        let local = iri.as_str().rsplit(':').next().unwrap_or("");
        let core = local
            .strip_prefix("deg_")
            .or_else(|| local.strip_prefix("std_"))
            .unwrap_or(local);
        let id = core.split('_').next().unwrap_or(core).to_string();
        if !id.is_empty() && !out.contains_key(&id) {
            let label = atom_label(&id, vb).unwrap_or_else(|| id.clone());
            out.insert(
                id.clone(),
                ConceptNote {
                    definition: concept_definition(iri, &label, vb),
                    id,
                    label,
                },
            );
        }
    }
    for child in child_exps(e) {
        collect_concepts(child, vb, out);
    }
}

/// The chain's description for `iri`, minus the leading label and the importer's provenance
/// suffix. `None` when the description says nothing the label does not already say.
fn concept_definition(iri: &Iri, label: &str, vb: &Vb) -> Option<String> {
    let res = vb.layer.resolve(iri)?;
    let Value::String(d) = res.get(&Iri::parse("urn:eigenius:core:description").ok()?)? else {
        return None;
    };
    let body = d.split(" UMLS CUI ").next().unwrap_or(d);
    // "Label — Definition …" → the definition half.
    let body = match body.split_once('—') {
        Some((_, rest)) => rest,
        None => body.strip_prefix(label).unwrap_or(body),
    };
    let body = body.trim().trim_start_matches(['-', ':']).trim();
    if body.is_empty() || body.eq_ignore_ascii_case(label) {
        return None;
    }
    Some(body.to_string())
}

/// The immediate sub-expressions of `e` — enough of the shape for a concept sweep.
fn child_exps(e: &Exp) -> Vec<&Exp> {
    match e {
        Exp::App(f, x) => vec![f.as_ref(), x.as_ref()],
        Exp::Sig(_, a, b) | Exp::Pi(_, a, b) => vec![a.as_ref(), b.as_ref()],
        Exp::Ann(a, b) => vec![a.as_ref(), b.as_ref()],
        Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) => vec![b.as_ref()],
        Exp::Pair(a, b) => vec![a.as_ref(), b.as_ref()],
        Exp::InductiveType(_, args) => args.iter().collect(),
        _ => Vec::new(),
    }
}

/// A concept label from a bare CUI embedded in a lexicon atom's LOCAL NAME (`C0333668`, or the
/// stripped core of a `deg_C…_rel` wrapper). The `urn:eigenius:umlscui:` reconstruction exists
/// because derived atoms carry only the sense key, not a link to the concept resource; it goes
/// away when importers emit the label/link first-class
/// (`docs/notes/d64-demonstratives-as-holes.md` §4 slice 4 — deferred, not built).
fn cui_label(cui: &str, layer: &Arc<Layer>) -> Option<String> {
    let iri = Iri::parse(&format!("urn:eigenius:umlscui:{cui}")).ok()?;
    resource_label(&iri, layer)
}

/// Naming + layer context threaded through the walk.
pub struct Vb<'a> {
    pub names: &'a BTreeMap<String, String>,
    pub layer: &'a Arc<Layer>,
    /// Which register to render in — see [`Register`]. [`Vb::surface`] is the historical
    /// behaviour and is what every human-facing caller wants.
    pub register: Register,
}

impl<'a> Vb<'a> {
    /// The reader's register: prose that reads like the source sentence.
    pub fn surface(names: &'a BTreeMap<String, String>, layer: &'a Arc<Layer>) -> Self {
        Self {
            names,
            layer,
            register: Register::Surface,
        }
    }

    /// The chooser's register: the reading's semantic commitments, spelled out (D69 §4).
    pub fn expanded(names: &'a BTreeMap<String, String>, layer: &'a Arc<Layer>) -> Self {
        Self {
            names,
            layer,
            register: Register::Expanded,
        }
    }

    fn expanded_mode(&self) -> bool {
        self.register == Register::Expanded
    }
}

/// Which register [`verbalize`] renders in (D69).
///
/// Strict verbalization is approximately a LEFT INVERSE OF PARSING: it reconstructs the input
/// sentence, so every reading of one sentence converges on that sentence and the renderer
/// collapses exactly the ambiguities the parse resolved. Measured 2026-08-13 on «MSI cancer
/// models did not have the exonuclease activity of WRN.»: 120 candidate readings carrying 120
/// DISTINCT sems rendered to **4 distinct strings** — «exonuclease activity» is what both the
/// single UMLS concept C1148824 and the `activity ⊗ exonuclease` compound come out as. A model
/// asked to choose between them was choosing blind.
///
/// So a chooser needs a different function, not better prose.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Register {
    /// Reads like the source sentence. Humans, the gate's narration, `narrate.py`, claim
    /// descriptions. Byte-stable — D69 slice 2 gates on it not moving.
    #[default]
    Surface,
    /// Says what the reading COMMITS TO: every content position carries its concept label AND
    /// IRI (labels collide, IRIs do not), a compound modifier is marked as the unspecified
    /// relation it is, and structure is explicit rather than implied by word order.
    Expanded,
}

fn app_spine(e: &Exp) -> (&Exp, Vec<&Exp>) {
    let mut args = Vec::new();
    let mut cur = e;
    while let Exp::App(f, x) = cur {
        args.push(x.as_ref());
        cur = f;
    }
    args.reverse();
    (cur, args)
}

/// The local name of a sense ATOM — an axiom, a class, or a named INDIVIDUAL.
///
/// The individual arm was missing until 2026-07-27, and it mattered: an importer declares a
/// concept as a `resource` (not a `class`) when it is a named INDIVIDUAL — in the seeded data,
/// `C0879389` "MLH1 gene", `C1337007` "WRN gene" — and those reach the sem as
/// [`Exp::EigonResource`]. Without this arm `axiom_local` returned `None`, so [`name_atom`] was
/// never consulted and every `compound(x, <individual>)` reading verbalised as the raw
/// `⟦C0879389⟧` bracket.
///
/// That blinded the verbaliser on exactly the readings under review when adjudicating the
/// `compound` / `compound_kind` split, since `compound` (`Entity -> Entity`) is the INDIVIDUAL
/// relation and `compound_kind` (`Entity -> Set`) the kind one — so the individual side of every
/// such pair was unreadable. [`resource_label`] resolves these fine (the resource carries a
/// `core:description`); only the extractor was refusing to hand it the key.
fn axiom_local(e: &Exp) -> Option<&str> {
    match e {
        Exp::EigonAxiom(i) | Exp::EigonClass(i) => {
            Some(i.as_str().rsplit(':').next().unwrap_or(""))
        }
        Exp::EigonResource(r) => r.id().map(|i| i.as_str().rsplit(':').next().unwrap_or("")),
        _ => None,
    }
}

/// `logic:False` — the negation codomain. It is built as `Exp::InductiveType(logic:False, [])`
/// (`constructions::negate_prop`), NOT as an axiom or class, so `axiom_local` never matched it and
/// the verbaliser's negation arms were dead: every negated proposition reached the ⟦…⟧ bracket.
fn is_false(e: &Exp) -> bool {
    match e {
        Exp::InductiveType(d, args) => args.is_empty() && d.iri.as_str().ends_with("logic:False"),
        _ => axiom_local(e) == Some("False"),
    }
}

/// The word for a sense atom: the unit's own lemma map first, then the concept's layer label
/// (via `cui_label`), else the local name.
fn name_atom(local: &str, vb: &Vb) -> String {
    // Normalise: strip the `deg_`/`std_` adjective wrappers and any verb frame suffix (`_t`/`_i`/…).
    let core = local
        .strip_prefix("deg_")
        .or_else(|| local.strip_prefix("std_"))
        .unwrap_or(local);
    let key = core.split('_').next().unwrap_or(core);
    let label = atom_label(key, vb);
    if vb.expanded_mode() {
        // Label AND identity: two concepts routinely share a label (C1148824's label IS
        // "exonuclease activity"), and the identity is what a chooser needs.
        return match label {
            Some(l) if l != key => format!("«{l}» [{key}]"),
            _ => format!("[{key}]"),
        };
    }
    label.unwrap_or_else(|| key.to_string())
}

/// The display label for an atom's key, or `None` when the lexicon offers none.
fn atom_label(key: &str, vb: &Vb) -> Option<String> {
    if let Some(w) = vb.names.get(key) {
        return Some(w.clone());
    }
    if key.starts_with('C') && key[1..].chars().all(|c| c.is_ascii_digit()) {
        return cui_label(key, vb.layer);
    }
    None
}

pub fn verbalize(sem: &Exp, vb: &Vb) -> String {
    match sem {
        Exp::Ann(inner, _) | Exp::Fst(inner) | Exp::Snd(inner) => return verbalize(inner, vb),
        Exp::Lam(_, body) => return verbalize(body, vb),
        Exp::Var(_) => return String::new(), // a bound restrictor variable — carries no surface
        _ => {}
    }
    if let Exp::InductiveType(decl, args) = sem {
        let d = decl.iri.as_str();
        if args.len() == 2 && (d.ends_with("logic:And") || d.ends_with("logic:Or")) {
            // Verb + shared-subject PP is ONE clause, not a conjunction: `And(V(subj), prep(subj, o))`
            // → "subj V prep o" (e.g. "MSI arises from Lynch syndrome"), the dominant sentence shape.
            if d.ends_with("And") {
                if let Some(merged) = verb_pp(&args[0], &args[1], vb) {
                    return merged;
                }
            }
            let op = if d.ends_with("And") { "and" } else { "or" };
            return format!(
                "{} {op} {}",
                verbalize(&args[0], vb),
                verbalize(&args[1], vb)
            );
        }
    }
    // Negation `A → False`. The Pi branch below catches the `Pi(_, A, False)` readback, but a
    // non-dependent arrow can also read back as `Exp::Arrow`, which that branch never sees — so a
    // negated coordination (`And(respond(x), prep_to(x, …)) → False`) stayed bracketed.
    if let Some((a, f)) = as_arrow(sem) {
        if is_false(f) {
            return format!("not ({})", verbalize(a, vb));
        }
    }
    if let Exp::Pi(binder, dom, cod) = sem {
        if is_false(cod) {
            return format!("not ({})", verbalize(dom, vb));
        }
        // Existential GQ (`exists_sem`/`obj_exists_sem`, closed-class.esl): `∀C:Prop. (∀x:A. body(x) →
        // C) → C`, readback `Pi(C, Prop, (Pi(x, A=Σ, body → C)) → C)`. Non-dependent `→` reads back as
        // `Pi(Patt::Unit, …)`, so match via `as_arrow`. → "some {A} {body}".
        if let Some((Exp::Pi(xb, a, arr), _c)) = as_arrow(cod) {
            // The restrictor may be a Σ-REFINED noun ("some MSI cell lines") or a PLAIN class
            // ("many cancers", "some cancers") — the quantifier encoding is identical either way,
            // and `quant_clause` goes through `bare_np`, which handles both. Requiring `Σ` here left
            // every unrefined-subject GQ bracketed: 46 of the residual ⟦…⟧ on the audited units.
            if matches!(a.as_ref(), Exp::Sig(..) | Exp::EigonClass(..)) {
                let parts = cps_body_parts(arr);
                if !parts.is_empty() {
                    let preds: Vec<String> = parts
                        .iter()
                        .map(|p| quant_clause_pred(xb, p, vb))
                        .filter(|x| !x.is_empty())
                        .collect();
                    let np = bare_np(a, vb);
                    return if preds.is_empty() {
                        format!("some {np}")
                    } else {
                        format!("some {np}, {}", preds.join(" and "))
                    };
                }
            }
        }
        // Universal / negative GQ over a Σ noun (`forall_sem`: `∀x:A. body`; `no_sem`: `∀x:A. body →
        // False`). Object variants (`obj_*`) fill the subject in, so the readback shape matches.
        if matches!(dom.as_ref(), Exp::Sig(..) | Exp::EigonClass(..)) {
            if let Some((body, f)) = as_arrow(cod) {
                if is_false(f) {
                    return format!("no {}", quant_clause(dom, binder, body, vb));
                }
            }
            return format!("every {}", quant_clause(dom, binder, cod, vb));
        }
        return format!("⟦{}⟧", pretty_term(sem)); // other Π — not verbalizable yet
    }
    if let Exp::Sig(_, base, restr) = sem {
        let np = noun_phrase(base, restr, vb);
        return format!("{} {np}", article(&np));
    }
    let (head, args) = app_spine(sem);
    // An application headed by a BOUND VARIABLE. The predicate slot of a clausal complement
    // ("These findings show that WRN is …", "We found that WRN was …") holds the abstracted
    // variable, so the embedded clause reads back as `G#0(C1337007)`. A bare `Var` already
    // verbalises to the empty string — it carries no surface — and its application should too;
    // render just the arguments. Without this the whole embedded clause fell to the ⟦…⟧ bracket,
    // which made every `that`-complement unit unauditable.
    if matches!(head, Exp::Var(_)) && !args.is_empty() {
        let parts: Vec<String> = args
            .iter()
            .map(|a| verbalize(a, vb))
            .filter(|s| !s.is_empty())
            .collect();
        return parts.join(" ");
    }
    if let Some(local) = axiom_local(head) {
        match (local, args.len()) {
            ("subclass_of", 2) => {
                return format!(
                    "every {} is {}",
                    bare_np(args[0], vb),
                    indefinite(args[1], vb)
                );
            }
            ("is_a", 2) => {
                return format!("{} is {}", verbalize(args[0], vb), indefinite(args[1], vb));
            }
            // Top-level gradable-adjective predication: `gt(deg_X(subj), std_X)` → "subj is X".
            // Two shapes share `gt` and only the ADJECTIVE one renders. A plain gradable
            // predication compares against the STANDARD — `gt(deg_X(subj), std_X)` -> "subj is X".
            // A COMPARATIVE compares against a real target, `gt(deg_X_rel(subj), <target>)`, and its
            // `than`-clause is currently DROPPED: "MSI cell lines showed greater dependence on WRN
            // than their MSS counterparts." renders as "WRN protein, human is a00725772".
            //
            // TRACED 2026-07-29, and the fix is NOT this arm alone. The discriminator is `args[1]`
            // (standard vs target), not a `deg_` prefix on `args[0]` — that prefix is present in
            // BOTH shapes (`deg_a00725772_rel`). But rendering the target requires an arm for
            // `deg_X_rel(a, b)` as well, which has none: adding the "more … than …" branch WITHOUT
            // it took bracketed glosses from 31 to 1833 of 2871, because that shape is pervasive.
            // Measured and reverted. The comparative stays mis-rendered until `deg_*_rel` renders.
            // Two shapes share `gt`.
            //
            //   PLAIN GRADABLE     gt(deg_X(subj), std_X)                     -> "subj is X"
            //   RELATIONAL COMPARATIVE
            //                      gt(deg_X_rel(g, s0), deg_X_rel(g, s1))     -> "s0 is more X on g than s1"
            //
            // `deg_{loc}_rel : Entity(ground) -> Entity(subject) -> float` (the WordNet
            // importer's atom convention, `eigenius-wordnet` `convert.rs`), so a
            // comparative is TWO relational degrees over the SAME ground with different subjects.
            // "MSI cell lines … showed greater dependence on WRN than their MSS counterparts."
            //
            // TWO EARLIER ATTEMPTS FAILED HERE, both measured:
            //  - discriminating on a `deg_` prefix in `args[0]` did nothing, because BOTH shapes
            //    carry it (`deg_a00725772_rel`);
            //  - discriminating on `args[1]` and then verbalising that argument took bracketed
            //    glosses from 31 to 1833 of 2871, because a bare `deg_X_rel(a, b)` has no arm of its
            //    own and the shape is pervasive.
            // Destructuring BOTH arguments here avoids that: `verbalize` is never called on a
            // relational degree, only on its operands.
            ("gt" | "lt", 2) => {
                let (h0, a0) = app_spine(args[0]);
                let (h1, a1) = app_spine(args[1]);
                let l0 = axiom_local(h0);
                if let (Some(d0), Some(d1)) = (l0, axiom_local(h1)) {
                    if d0.ends_with("_rel") && d0 == d1 && a0.len() == 2 && a1.len() == 2 {
                        let word = if local == "gt" { "more" } else { "less" };
                        return format!(
                            "{} is {word} {} on {} than {}",
                            verbalize(a0[1], vb),
                            name_atom(d0, vb),
                            verbalize(a0[0], vb),
                            verbalize(a1[1], vb)
                        );
                    }
                }
                if let (Some(dl), Some(subj)) = (l0, a0.first()) {
                    return format!("{} is {}", verbalize(subj, vb), name_atom(dl, vb));
                }
            }
            ("kind_of", 1) => return verbalize(args[0], vb),
            ("the", 1) => return format!("the {}", bare_np(args[0], vb)),
            // Referential predication (D63 Defect 3): `the(subject-class, restrictor, x)` = "x is the
            // {subject-class} that is {restrictor}" — the copula's referential distribution over a
            // coordinated predicate nominal ("These groups are MSI lines, microsatellite-stable lines
            // and indeterminate lines"). Each And-conjunct is one of these; without this case the
            // 3-arg `the` fell through to the ⟦…⟧ bracket. `x` is usually a bound restrictor var (so
            // `verbalize` returns ""), giving "the {class} that is {restrictor}".
            ("the", 3) => {
                let subj = verbalize(args[2], vb);
                let cls = bare_np(args[0], vb);
                let restr = verbalize(args[1], vb);
                return if subj.is_empty() {
                    format!("the {cls} that is {restr}")
                } else {
                    format!("{subj} is the {cls} that is {restr}")
                };
            }
            // `poss_of` is POLYMORPHIC — `forall (A:Set) => A -> Entity -> Prop` — so it reads back
            // with the Set as a leading argument and the pair (possessed, possessor) after it.
            // Accept both arities; without this "their MSS counterparts" bracketed.
            ("poss_of", 2 | 3) => {
                let (owned, owner) = if args.len() == 3 {
                    (args[1], args[2])
                } else {
                    (args[0], args[1])
                };
                let o = verbalize(owner, vb);
                let n = verbalize(owned, vb);
                return if o.is_empty() {
                    format!("its {n}")
                } else {
                    format!("{o}'s {n}")
                };
            }
            ("Possible" | "modal", 1) => return format!("possibly, {}", verbalize(args[0], vb)),
            ("speaker", _) => return "we".to_string(),
            ("anaphor", _) => return "it".to_string(),
            _ => {}
        }
        // A PP predication standing ALONE — `prep_in(subj, obj)`. `verb_pp` merges the common
        // `And(V(subj), prep(subj, obj))` shape into a single clause, but a PP conjunct it cannot
        // merge — a distributed coordination, or a clausal complement — reached the ⟦…⟧ bracket.
        // The subject is usually a bound restrictor variable (verbalising to ""), giving "in X".
        if let Some(p) = local.strip_prefix("prep_") {
            if args.len() == 2 {
                let subj = verbalize(args[0], vb);
                let obj = verbalize(args[1], vb);
                return if subj.is_empty() {
                    format!("{p} {obj}")
                } else {
                    format!("{subj} {p} {obj}")
                };
            }
        }
        // Verb: `v{offset}_{frame}(obj, subj)` transitive / `(subj)` intransitive (category
        // `(S\NP)/NP` — object first; the WordNet importer's verb-atom convention,
        // `eigenius-wordnet` `convert.rs`).
        if local.starts_with('v') && local.contains('_') {
            let verb = name_atom(local, vb);
            // The frame tag is the suffix after the last `_` (the importer's frame tags: `_i` intransitive,
            // `_t` transitive, `_p` PP-oblique, `_as` ESSIVE, `_d` ditransitive). A 3-argument
            // frame had no arm at all, so every essive clause — "identified WRN AS the top
            // dependency", "evaluated MSI AS a biomarker" — bracketed in full.
            let tag = local.rsplit('_').next().unwrap_or("");
            return match args.as_slice() {
                [subj] => format!("{} {verb}", verbalize(subj, vb)),
                [obj, subj] => format!("{} {verb} {}", verbalize(subj, vb), verbalize(obj, vb)),
                [obj, comp, subj] if tag == "as" => format!(
                    "{} {verb} {} as {}",
                    verbalize(subj, vb),
                    verbalize(obj, vb),
                    verbalize(comp, vb)
                ),
                [a, b, subj] => format!(
                    "{} {verb} {} {}",
                    verbalize(subj, vb),
                    verbalize(a, vb),
                    verbalize(b, vb)
                ),
                _ => format!("⟦{}⟧", pretty_term(sem)),
            };
        }
    }
    if let Some(local) = axiom_local(sem) {
        return name_atom(local, vb);
    }
    format!("⟦{}⟧", pretty_term(sem))
}

/// `And(V(subj), prep_X(subj, obj))` → "subj V prep obj" when the two share a subject; else `None`.
fn verb_pp(left: &Exp, right: &Exp, vb: &Vb) -> Option<String> {
    let (lh, la) = app_spine(left);
    let (rh, ra) = app_spine(right);
    let ll = axiom_local(lh)?;
    let rl = axiom_local(rh)?;
    if !(ll.starts_with('v') && ll.contains('_') && rl.starts_with("prep_") && ra.len() == 2) {
        return None;
    }
    // Intransitive/PP verb: its sole arg is the subject; it must match the PP's first arg.
    let subj = match la.as_slice() {
        [s] => s,
        _ => return None,
    };
    if pretty_term(subj) != pretty_term(ra[0]) {
        return None;
    }
    Some(format!(
        "{} {} {} {}",
        verbalize(subj, vb),
        name_atom(ll, vb),
        &rl[5..],
        verbalize(ra[1], vb)
    ))
}

/// A quantifier's body over the bound entity: "{NP}, {predicate}" with the bound variable (already
/// named by the NP) rendered as "it", so the coreference is legible — "some group of cell lines, we
/// identified it". Fail-honest: an empty predicate degrades to just the NP.
/// One conjunct of a quantifier body, with the bound variable replaced by the anaphor placeholder —
/// the per-part half of [`quant_clause`], so a CPS body with SEVERAL conjuncts can render each.
fn quant_clause_pred(xbinder: &Patt, body: &Exp, vb: &Vb) -> String {
    let body = match xbinder {
        Patt::Var(x) => subst_var(body, x, &anaphor_atom()),
        _ => body.clone(),
    };
    verbalize(&body, vb).trim().to_string()
}

fn quant_clause(np_sig: &Exp, xbinder: &Patt, body: &Exp, vb: &Vb) -> String {
    let np = bare_np(np_sig, vb);
    let body = match xbinder {
        Patt::Var(x) => subst_var(body, x, &anaphor_atom()),
        _ => body.clone(),
    };
    let pred = verbalize(&body, vb);
    if pred.trim().is_empty() {
        np
    } else {
        format!("{np}, {pred}")
    }
}

/// The BODY of a CPS-encoded quantifier: peel the whole arrow chain `A → B → … → C → C` and drop the
/// trailing continuation variables, leaving `[A, B, …]` — the conjuncts the quantifier asserts.
///
/// Taking only the FIRST antecedent silently DROPS the rest, and on this corpus that lost an entire
/// comparative: "MSI cell lines … showed greater dependence on WRN than their MSS counterparts."
/// reads back as `poss_of(…) → gt(…) → G#0 → G#0`, and rendering just `poss_of` gave the stub
/// "some SIL1 gene counterpart, its it" — the `gt` comparison, which is the whole claim, vanished.
fn cps_body_parts(e: &Exp) -> Vec<&Exp> {
    let mut parts = Vec::new();
    let mut cur = e;
    while let Some((a, b)) = as_arrow(cur) {
        parts.push(a);
        cur = b;
    }
    while matches!(parts.last(), Some(Exp::Var(_))) {
        parts.pop();
    }
    parts
}

/// A function type `A → B`, however it reads back — the explicit `Exp::Arrow` or the non-dependent
/// `Pi(Patt::Unit, A, B)` (readback uses the latter for `→`).
fn as_arrow(e: &Exp) -> Option<(&Exp, &Exp)> {
    match e {
        Exp::Arrow(a, b) => Some((a.as_ref(), b.as_ref())),
        Exp::Pi(Patt::Unit, a, b) => Some((a.as_ref(), b.as_ref())),
        _ => None,
    }
}

/// The `lexicon:anaphor` placeholder — verbalizes as "it" (the entity the NP already names).
fn anaphor_atom() -> Exp {
    Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:anaphor").expect("anaphor iri"))
}

/// Replace the free variable `name` with `to` throughout `e` (glossing the bound quantifier entity).
fn subst_var(e: &Exp, name: &str, to: &Exp) -> Exp {
    let go = |x: &Exp| subst_var(x, name, to);
    match e {
        Exp::Var(v) if v == name => to.clone(),
        Exp::App(f, x) => Exp::App(Box::new(go(f)), Box::new(go(x))),
        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(go(b))),
        Exp::Pi(p, a, b) => Exp::Pi(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Sig(p, a, b) => Exp::Sig(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(go(a)), Box::new(go(b))),
        Exp::Times(a, b) => Exp::Times(Box::new(go(a)), Box::new(go(b))),
        Exp::Fst(x) => Exp::Fst(Box::new(go(x))),
        Exp::Snd(x) => Exp::Snd(Box::new(go(x))),
        Exp::Pair(a, b) => Exp::Pair(Box::new(go(a)), Box::new(go(b))),
        Exp::Ann(x, t) => Exp::Ann(Box::new(go(x)), Box::new(go(t))),
        Exp::InductiveType(d, args) => Exp::InductiveType(d.clone(), args.iter().map(go).collect()),
        Exp::InductiveCtor(d, n, args) => {
            Exp::InductiveCtor(d.clone(), n.clone(), args.iter().map(go).collect())
        }
        other => other.clone(),
    }
}

/// "a" / "an" for the following word (vowel-initial → "an").
fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some(c) if "aeiou".contains(c.to_ascii_lowercase()) => "an",
        _ => "a",
    }
}

/// "a NP" / "an NP" for a bare kind / class argument (a Σ already supplies its own article).
fn indefinite(e: &Exp, vb: &Vb) -> String {
    match e {
        Exp::Sig(..) => verbalize(e, vb),
        _ => {
            let w = verbalize(e, vb);
            format!("{} {w}", article(&w))
        }
    }
}

/// The NP text without a leading article (for `the …`).
fn bare_np(e: &Exp, vb: &Vb) -> String {
    if let Exp::Sig(_, base, restr) = e {
        return noun_phrase(base, restr, vb);
    }
    verbalize(e, vb)
}

/// "adjs compound-mods HEAD pps" from a Σ's base type and restrictor conjuncts.
///
/// In [`Register::Expanded`] the same conjuncts are rendered as an explicit commitment list
/// instead — see [`noun_phrase_expanded`]. The two shapes must differ: this function is where
/// the measured collision lives, because "modifier head" is also how a single concept whose
/// label happens to be two words comes out.
fn noun_phrase(base: &Exp, restr: &Exp, vb: &Vb) -> String {
    if vb.expanded_mode() {
        return noun_phrase_expanded(base, restr, vb);
    }
    let head = verbalize(base, vb);
    let (mut pre, mut post): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut conj = Vec::new();
    flatten_and_exp(restr, &mut conj);
    for c in conj {
        let (h, a) = app_spine(c);
        match axiom_local(h) {
            // A compound modifier is a bare noun ("nucleotide-repeat"), not "a nucleotide-repeat".
            Some("compound_kind" | "compound") if a.len() == 2 => pre.push(bare_np(a[1], vb)),
            Some("gt" | "lt") => {
                if let Some(first) = a.first() {
                    let (dh, _) = app_spine(first);
                    if let Some(dl) = axiom_local(dh) {
                        pre.push(name_atom(dl, vb));
                    }
                }
            }
            Some(p) if p.starts_with("prep_") => {
                if let Some(x) = a.get(1) {
                    post.push(format!("{} {}", &p[5..], verbalize(x, vb)));
                }
            }
            Some("is_a") if a.len() == 2 => post.push(format!("that is {}", indefinite(a[1], vb))),
            Some("named") if a.len() == 2 => post.push(format!("named {}", verbalize(a[1], vb))),
            // A possessive restrictor — `Σx:N. poss_of(N, x, owner)`, "their MSS counterparts".
            Some("poss_of") if a.len() == 2 || a.len() == 3 => {
                let owner = verbalize(a[a.len() - 1], vb);
                pre.push(if owner.is_empty() {
                    "its".to_string()
                } else {
                    format!("{owner}'s")
                });
            }
            // A restrictor headed by the Σ's OWN BOUND VARIABLE — `G#0(C1337007)`. This is the
            // clausal complement's predicate slot ("the finding that … WRN"): the abstracted
            // predicate applied to its argument. A bare `Var` carries no surface, so render the
            // ARGUMENTS. `about` is a gloss for the predication, in the same spirit as the `that
            // is` / `named` arms above — it names the participant without claiming the relation.
            // Without this every `that`-complement unit bracketed its entire embedded clause.
            None if matches!(h, Exp::Var(_)) && !a.is_empty() => {
                let inner: Vec<String> = a
                    .iter()
                    .map(|x| verbalize(x, vb))
                    .filter(|x| !x.is_empty())
                    .collect();
                post.push(format!("about {}", inner.join(" ")));
            }
            // Anything else: hand it to `verbalize` rather than bracketing it here. An embedded GQ
            // restrictor (`Π… prep_of …`, "of a DNA repair pathway") is perfectly renderable by the
            // quantifier arms — bracketing it at this level threw that away. `verbalize` still
            // brackets what IT cannot render, so the "never silently dropped" property is kept.
            _ => post.push(verbalize(c, vb)),
        }
    }
    let mut s = String::new();
    for m in pre.iter().filter(|m| !m.is_empty()) {
        s.push_str(m);
        s.push(' ');
    }
    s.push_str(&head);
    for m in post.iter().filter(|m| !m.is_empty()) {
        s.push(' ');
        s.push_str(m);
    }
    s.trim().to_string()
}

/// The chooser's rendering of a Σ noun phrase (D69 §4): head concept, then each restrictor as a
/// NAMED commitment, so nothing rides on word order.
///
/// The contrast this exists for:
///
/// ```text
/// Σx:C1148824. of(x, WRN)                        → «exonuclease activity» [C1148824] of …
/// Σx:n00407535. compound(x, n14606137) ∧ of(…)   → «activity» [n00407535]
///                                                    + compound-with «exonuclease» [n14606137]
///                                                      (relation unspecified) of …
/// ```
///
/// In `Surface` both are "the exonuclease activity of WRN".
fn noun_phrase_expanded(base: &Exp, restr: &Exp, vb: &Vb) -> String {
    let head = verbalize(base, vb);
    let mut parts: Vec<String> = Vec::new();
    let mut conj = Vec::new();
    flatten_and_exp(restr, &mut conj);
    for c in conj {
        let (h, a) = app_spine(c);
        match axiom_local(h) {
            // The whole point: a compound asserts that SOME relation holds between the head and
            // the modifier, and the parse does not say which. Surface hides that behind
            // juxtaposition — the reading it is competing with names one concept outright.
            Some("compound_kind" | "compound") if a.len() == 2 => parts.push(format!(
                "compound-with {} (relation unspecified)",
                bare_np(a[1], vb)
            )),
            Some(p) if p.starts_with("prep_") => {
                if let Some(x) = a.get(1) {
                    parts.push(format!("{} {}", &p[5..], verbalize(x, vb)));
                }
            }
            Some("is_a") if a.len() == 2 => parts.push(format!("is-a {}", bare_np(a[1], vb))),
            Some("named") if a.len() == 2 => parts.push(format!("named {}", verbalize(a[1], vb))),
            Some("poss_of") if a.len() == 2 || a.len() == 3 => {
                parts.push(format!("possessed-by {}", verbalize(a[a.len() - 1], vb)))
            }
            // A comparative carries a STANDARD, and the standard is exactly what two readings of
            // an elided «stronger» differ by: `std_a…` (the norm) vs `deg_a…(t)` (than t,
            // recovered from the discourse). Dropping it made those two readings identical — the
            // injectivity guard caught it live on «…a stronger mutation phenotype» (D69 §7a).
            Some(cmp @ ("gt" | "lt")) if a.len() == 2 => {
                let dir = if cmp == "gt" { "greater" } else { "less" };
                let adj = axiom_local(app_spine(a[0]).0)
                    .map(|l| name_atom(l, vb))
                    .unwrap_or_default();
                parts.push(format!(
                    "degree-{dir} {adj} than {}",
                    comparative_standard(a[1], vb)
                ));
            }
            _ => parts.push(verbalize(c, vb)),
        }
    }
    let parts: Vec<String> = parts.into_iter().filter(|p| !p.trim().is_empty()).collect();
    if parts.is_empty() {
        head
    } else {
        format!("{head} + {}", parts.join(" + "))
    }
}

fn flatten_and_exp<'a>(e: &'a Exp, out: &mut Vec<&'a Exp>) {
    if let Exp::InductiveType(decl, args) = e {
        if decl.iri.as_str().ends_with("logic:And") && args.len() == 2 {
            flatten_and_exp(&args[0], out);
            flatten_and_exp(&args[1], out);
            return;
        }
    }
    out.push(e);
}

#[cfg(test)]
mod register_tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::nbe::term::Patt;

    fn layer() -> Arc<Layer> {
        Arc::new(LayerBuilder::new("t", None).build(LayerStorage::in_memory()))
    }

    fn cls(iri: &str) -> Exp {
        Exp::EigonClass(Iri::parse(iri).expect("iri"))
    }

    /// `Σ x0 : dom. restr` — the shape a refined noun takes.
    fn sig(dom: Exp, restr: Exp) -> Exp {
        Exp::Sig(Patt::Var("x0".into()), Box::new(dom), Box::new(restr))
    }

    fn app2(axiom: &str, a: Exp, b: Exp) -> Exp {
        Exp::App(
            Box::new(Exp::App(
                Box::new(Exp::EigonAxiom(Iri::parse(axiom).expect("iri"))),
                Box::new(a),
            )),
            Box::new(b),
        )
    }

    /// THE MEASURED COLLISION (D69 §1). «exonuclease activity» is C1148824's own label, so the
    /// single-concept reading and the `activity ⊗ exonuclease` compound reading are the same
    /// string in `Surface`. In `Expanded` they must not be.
    #[test]
    fn expanded_separates_a_named_concept_from_a_compound() {
        let l = layer();
        let mut names = BTreeMap::new();
        names.insert("C1148824".to_string(), "exonuclease activity".to_string());
        names.insert("n00407535".to_string(), "activity".to_string());
        names.insert("n14606137".to_string(), "exonuclease".to_string());
        names.insert("C0388246".to_string(), "WRN".to_string());

        let of = |x: Exp| {
            app2(
                "urn:eigenius:ontology:prep_of",
                x,
                cls("urn:eigenius:umlscui:C0388246"),
            )
        };
        let concept = sig(
            cls("urn:eigenius:umlscui:C1148824"),
            of(Exp::Var("x0".into())),
        );
        // `logic:And` is an INDUCTIVE, not an axiom application — that is the shape
        // `flatten_and_exp` splits and the shape the resolver builds.
        let and_decl = Arc::new(crate::nbe::term::InductiveDecl {
            iri: Iri::parse("urn:eigenius:logic:And").expect("iri"),
            name: "And".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(0),
            ctors: Vec::new(),
        });
        let compound = sig(
            cls("urn:eigenius:wn:n00407535"),
            Exp::InductiveType(
                and_decl,
                vec![
                    app2(
                        "urn:eigenius:ontology:compound_kind",
                        Exp::Var("x0".into()),
                        cls("urn:eigenius:wn:n14606137"),
                    ),
                    of(Exp::Var("x0".into())),
                ],
            ),
        );

        let surface = Vb::surface(&names, &l);
        let s_concept = verbalize(&concept, &surface);
        let s_compound = verbalize(&compound, &surface);
        assert_eq!(
            s_concept, s_compound,
            "the collision this register exists for must still be reproducible in Surface \
             (if this ever fails, Surface changed — check the byte-stability gate)"
        );

        let expanded = Vb::expanded(&names, &l);
        let e_concept = verbalize(&concept, &expanded);
        let e_compound = verbalize(&compound, &expanded);
        assert_ne!(
            e_concept, e_compound,
            "Expanded must distinguish a named concept from a compound with the same surface"
        );
        assert!(
            e_concept.contains("C1148824"),
            "the concept's identity is named: {e_concept}"
        );
        assert!(
            e_compound.contains("compound-with") && e_compound.contains("relation unspecified"),
            "the compound's unspecified relation is stated: {e_compound}"
        );
    }

    /// Surface keeps its historical shape — the property the gate narration and the committed
    /// claim descriptions depend on.
    #[test]
    fn surface_is_unchanged_by_the_new_register() {
        let l = layer();
        let mut names = BTreeMap::new();
        names.insert("n00407535".to_string(), "activity".to_string());
        names.insert("n14606137".to_string(), "exonuclease".to_string());
        let e = sig(
            cls("urn:eigenius:wn:n00407535"),
            app2(
                "urn:eigenius:ontology:compound_kind",
                Exp::Var("x0".into()),
                cls("urn:eigenius:wn:n14606137"),
            ),
        );
        assert_eq!(
            verbalize(&e, &Vb::surface(&names, &l)),
            "an exonuclease activity"
        );
    }
}
