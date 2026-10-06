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
    for tok in index.tokenize(text) {
        if tok.is_comma() {
            continue;
        }
        let tok = tok.surface();
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
    // A document's named individual has no lexicon label: its name is the span that introduced it
    // («Project Achilles»), which a single token never is.
    for (words, sense) in index.span_senses(text, lem) {
        if let Some(key) = sense_key(&sense).filter(|k| k.starts_with(NAMED_INDIVIDUAL)) {
            m.entry(key).or_insert(words);
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
    // A declared label wins: an OBO import carries the term's name in `rdfs:label` (the OBO
    // meta-ontology), and its description is the definition alone, with no label to split off.
    if let Some(Value::String(l)) = res.get(&Iri::parse("urn:rdfs:label").ok()?) {
        if !l.trim().is_empty() {
            return Some(l.trim().to_string());
        }
    }
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
        let local = iri_key(iri.as_str());
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
        e if e.as_const_spine().is_some() => e.as_const_spine().expect("just matched").2,
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
        Exp::EigonAxiom(i) | Exp::EigonClass(i) => Some(iri_key(i.as_str())),
        Exp::EigonResource(r) => r.id().map(|i| iri_key(i.as_str())),
        _ => None,
    }
}

/// The key an atom renders by: its IRI's local name (`C1148824`, `n00407535`) — or, for an OBO
/// term (`urn:obo:HP:0100615`), its CURIE (`HP:0100615`), since an OBO local part alone is a bare
/// number that names nothing.
fn iri_key(iri: &str) -> &str {
    if let Some(curie) = iri.strip_prefix("urn:obo:") {
        return curie;
    }
    iri.rsplit(':').next().unwrap_or("")
}

/// `logic:False` — the negation codomain. It is built as `Exp::const_applied(logic:False.iri.clone(), Vec::new(), [])`
/// (`constructions::negate_prop`), NOT as an axiom or class, so `axiom_local` never matched it and
/// the verbaliser's negation arms were dead: every negated proposition reached the ⟦…⟧ bracket.
fn is_false(e: &Exp) -> bool {
    match e {
        Exp::Const(iri, _) => iri.as_str().ends_with("logic:False"),
        _ => axiom_local(e) == Some("False"),
    }
}

/// The word for a sense atom: the unit's own lemma map first, then the concept's layer label
/// (via `cui_label`), else the local name.
fn name_atom(local: &str, vb: &Vb) -> String {
    let key = atom_key(local);
    let label = atom_label(key, vb);
    if vb.register == Register::Expanded {
        // Label AND identity: two concepts routinely share a label (C1148824's label IS
        // "exonuclease activity"), and the identity is what a chooser needs.
        return match label {
            Some(l) if l != key => format!("«{l}» [{key}]"),
            _ => format!("[{key}]"),
        };
    }
    label.unwrap_or_else(|| key.to_string())
}

/// An atom's sense key: the `deg_`/`std_` adjective wrappers stripped, and any suffix after the key
/// — a verb frame (`_t`/`_i`/…) or a relational degree's `_rel` / `_rel_{p}`.
fn atom_key(local: &str) -> &str {
    // A document's named individual (`ni_project_achilles`): its local name is the key.
    if local.starts_with(NAMED_INDIVIDUAL) {
        return local;
    }
    let core = local
        .strip_prefix("deg_")
        .or_else(|| local.strip_prefix("std_"))
        .unwrap_or(local);
    core.split('_').next().unwrap_or(core)
}

/// The local-name prefix of a document's named individual (`glossary::named_entity_augmentation`
/// mints `urn:eigenius:doc:ni_<slug>`; its lexical entry's sense is `doc:<slug>`).
const NAMED_INDIVIDUAL: &str = "ni_";

/// The atom key a lexical entry's sense names: `wn:{lemma}.{tag}.{offset}` → `{tag}{offset}`,
/// `umls:{cui}` → the CUI, a document entry's `doc:{slug}` → its named individual `ni_{slug}`.
fn sense_key(sense: &str) -> Option<String> {
    if let Some(rest) = sense.strip_prefix("wn:") {
        match rest.rsplitn(3, '.').collect::<Vec<_>>().as_slice() {
            [offset, tag, _lemma] => Some(format!("{tag}{offset}")),
            _ => None,
        }
    } else if let Some(cui) = sense.strip_prefix("umls:") {
        Some(cui.to_string())
    } else {
        sense
            .strip_prefix("doc:")
            .map(|slug| format!("{NAMED_INDIVIDUAL}{slug}"))
    }
}

/// The sense atoms a term mentions, by key ([`atom_key`]), each once in order of first mention: the
/// concepts, senses and named individuals, not the grammatical constants around them.
pub(crate) fn sense_atoms(e: &Exp) -> Vec<String> {
    fn walk(e: &Exp, out: &mut Vec<String>) {
        if let Some(local) = axiom_local(e) {
            let key = atom_key(local);
            let sense = key.starts_with(NAMED_INDIVIDUAL)
                || key
                    .split(|c: char| !c.is_ascii_digit())
                    .any(|run| run.len() >= 4);
            if sense && !out.iter().any(|k| k == key) {
                out.push(key.to_string());
            }
        }
        for c in child_exps(e) {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(e, &mut out);
    out
}

/// An atom's plain word — its display label with no register markup.
fn atom_word(local: &str, vb: &Vb) -> String {
    let key = atom_key(local);
    atom_label(key, vb).unwrap_or_else(|| key.to_string())
}

/// The fail-honest bracket around a shape the verbaliser does not render.
fn bracket(sem: &Exp) -> String {
    format!("⟦{}⟧", pretty_term(sem))
}

/// The display label for an atom's key, or `None` when the lexicon offers none.
fn atom_label(key: &str, vb: &Vb) -> Option<String> {
    if let Some(w) = vb.names.get(key) {
        return Some(w.clone());
    }
    if key.starts_with('C') && key[1..].chars().all(|c| c.is_ascii_digit()) {
        return cui_label(key, vb.layer);
    }
    // An OBO term's key is its CURIE ([`atom_key`]); its name is on the chain.
    if key.contains(':') {
        let iri = Iri::parse(&format!("urn:obo:{key}")).ok()?;
        return resource_label(&iri, vb.layer);
    }
    None
}

pub fn verbalize(sem: &Exp, vb: &Vb) -> String {
    // A bound on a measured value (D95 slice 6) reads back as an existential over the value; render
    // the value as its constraint, `less than 6203/20 K`, wherever the quantity would have rendered.
    if has_bound(sem) {
        return verbalize(&render_bounds(sem), vb);
    }
    match sem {
        Exp::Ann(inner, _) | Exp::Fst(inner) | Exp::Snd(inner) => return verbalize(inner, vb),
        Exp::Lam(_, body) => return verbalize(body, vb),
        Exp::Var(_) => return String::new(), // a bound restrictor variable — carries no surface
        _ => {}
    }
    if let Some((head, _, args)) = sem.as_const_spine() {
        let d = head.as_str();
        if args.len() == 2 && (d.ends_with("logic:And") || d.ends_with("logic:Or")) {
            // Verb + shared-subject PP is ONE clause, not a conjunction: `And(V(subj), prep(subj, o))`
            // → "subj V prep o" (e.g. "MSI arises from Lynch syndrome"), the dominant sentence shape.
            if d.ends_with("And") {
                if let Some(text) = factor_comparison(args[0], args[1], vb) {
                    return text;
                }
                if let Some(merged) = verb_pp(args[0], args[1], vb) {
                    return merged;
                }
            }
            let op = if d.ends_with("And") { "and" } else { "or" };
            return format!("{} {op} {}", verbalize(args[0], vb), verbalize(args[1], vb));
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
        return bracket(sem); // other Π — not verbalizable yet
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
                    if let Some(prep) = relational_degree_preposition(d0) {
                        if d0 == d1 && a0.len() == 2 && a1.len() == 2 {
                            let word = if local == "gt" { "more" } else { "less" };
                            return format!(
                                "{} is {word} {} {prep} {} than {}",
                                verbalize(a0[1], vb),
                                name_atom(d0, vb),
                                verbalize(a0[0], vb),
                                verbalize(a1[1], vb)
                            );
                        }
                    }
                }
                if let (Some(dl), Some(subj)) = (l0, a0.first()) {
                    return format!("{} is {}", verbalize(subj, vb), name_atom(dl, vb));
                }
            }
            ("kind_of", 1) => return verbalize(args[0], vb),
            ("the", 1) => return format!("the {}", bare_np(args[0], vb)),
            // `the_count(A, q)` (D95 slice 7d): the definite with its count, `the 4 Helicase`.
            ("the_count", 2) => {
                let unit = Exp::LitUnit(crate::units::Unit::dimensionless());
                return format!(
                    "the {} {}",
                    quantity_text(args[1], &unit),
                    bare_np(args[0], vb)
                );
            }
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
            // A measured value predicated (D95): `has_quantity(x, u, q)` → "x is q". The subject is
            // a bound restrictor variable when the value modifies a noun, giving just "q".
            // A cardinal's count (D95 slice 7): `has_count(T, λx. body, q)` → "q T, body", the bound
            // variable rendered as the anaphor, as an existential renders "some T, body".
            ("has_count", 3) => {
                let np = bare_np(args[0], vb);
                let unit = Exp::LitUnit(crate::units::Unit::dimensionless());
                let count = quantity_text(args[2], &unit);
                let pred = match args[1] {
                    Exp::Lam(binder, body) => quant_clause_pred(binder, body, vb),
                    other => verbalize(other, vb),
                };
                return if pred.is_empty() {
                    format!("{count} {np}")
                } else {
                    format!("{count} {np}, {pred}")
                };
            }
            // A proportion (D95 slice 7c): `has_proportion(x, λy. body, q)` → "q of x, body".
            ("has_proportion", 3) => {
                let group = verbalize(args[0], vb);
                let unit = Exp::LitUnit(crate::units::Unit::dimensionless());
                let share = quantity_text(args[2], &unit);
                let pred = match args[1] {
                    Exp::Lam(binder, body) => quant_clause_pred(binder, body, vb),
                    other => verbalize(other, vb),
                };
                return if pred.is_empty() {
                    format!("{share} of {group}")
                } else {
                    format!("{share} of {group}, {pred}")
                };
            }
            // A factor comparative (D95 slice 9): `fold_lower(N, card(T, x), card(T, y))` reads `x has
            // N-fold fewer T than y`, and with `median_over` on both counts `x has a median N-fold
            // fewer T than y`; over other measures, `a is N-fold lower than b`.
            ("fold_lower" | "fold_higher", 3) => {
                let unit = Exp::LitUnit(crate::units::Unit::dimensionless());
                let factor = quantity_text(args[0], &unit);
                let lower = local == "fold_lower";
                if let (Some((t, a, sa)), Some((u, b, sb))) = (counted(args[1]), counted(args[2])) {
                    if t == u && sa == sb {
                        let dir = if lower { "fewer" } else { "more" };
                        let stat = if sa { "a median " } else { "" };
                        return format!(
                            "{} has {stat}{factor}-fold {dir} {} than {}",
                            verbalize(a, vb),
                            bare_np(t, vb),
                            verbalize(b, vb)
                        );
                    }
                }
                let dir = if lower { "lower" } else { "higher" };
                return format!(
                    "{} is {factor}-fold {dir} than {}",
                    verbalize(args[1], vb),
                    verbalize(args[2], vb)
                );
            }
            // A statistic (D95 slice 9): `median_over(λm. card(T, m), x)` reads `the median number of T
            // in x`.
            ("median_over", 2) => {
                let group = verbalize(args[1], vb);
                return match counted(args[0]) {
                    Some((t, _, false)) => {
                        format!("the median number of {} in {group}", bare_np(t, vb))
                    }
                    _ => format!("the median of {} over {group}", verbalize(args[0], vb)),
                };
            }
            // The distributive `per` (D95 slice 8d): `prep_per(Y, x, y)` reads `per Y`; `x` and `y` are
            // the variables the count and the universal bind.
            ("prep_per", 3) => return format!("per {}", bare_np(args[0], vb)),
            // A period (D95 slice 8b): `every_period(x, u, q)` reads `x every 259200 s`.
            ("every_period", 3) => {
                let subj = verbalize(args[0], vb);
                let q = quantity_text(args[2], args[1]);
                return if subj.is_empty() {
                    format!("every {q}")
                } else {
                    format!("{subj} every {q}")
                };
            }
            // An offset (D95 slice 8a): `prep_after_offset(x, y, u, q)` reads `x 259200 s after y`.
            ("prep_after_offset" | "prep_before_offset", 4) => {
                let subj = verbalize(args[0], vb);
                let p = if local == "prep_after_offset" {
                    "after"
                } else {
                    "before"
                };
                let tail = format!(
                    "{} {p} {}",
                    quantity_text(args[3], args[2]),
                    verbalize(args[1], vb)
                );
                return if subj.is_empty() {
                    tail
                } else {
                    format!("{subj} {tail}")
                };
            }
            ("has_quantity", 3) => {
                let subj = verbalize(args[0], vb);
                let q = quantity_text(args[2], args[1]);
                return if subj.is_empty() {
                    q
                } else {
                    format!("{subj} is {q}")
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
        if let Some((p, subj, obj)) = prep_parts(local, &args, vb) {
            let subj = verbalize(subj, vb);
            return if subj.is_empty() {
                format!("{p} {obj}")
            } else {
                format!("{subj} {p} {obj}")
            };
        }
        // Verb: `v{offset}_{frame}(obj, subj)` transitive / `(subj)` intransitive (category
        // `(S\NP)/NP` — object first; the WordNet importer's verb-atom convention,
        // `eigenius-wordnet` `convert.rs`).
        if local.starts_with('v') && local.contains('_') {
            let verb = name_atom(local, vb);
            // The frame tag follows the offset (the importer's frame tags: `_i` intransitive,
            // `_t` transitive, `_p` PP-oblique with any preposition, `_p_to` with a named one, `_as`
            // ESSIVE, `_d` ditransitive). A 3-argument frame had no arm at all, so every essive
            // clause — "identified WRN AS the top dependency", "evaluated MSI AS a biomarker" —
            // bracketed in full.
            let tag = verb_frame(local);
            return match args.as_slice() {
                [subj] => format!("{} {verb}", verbalize(subj, vb)),
                [obj, subj] => match named_preposition(tag) {
                    Some(p) => format!("{} {verb} {p} {}", verbalize(subj, vb), verbalize(obj, vb)),
                    None => format!("{} {verb} {}", verbalize(subj, vb), verbalize(obj, vb)),
                },
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
                _ => bracket(sem),
            };
        }
    }
    if let Some(local) = axiom_local(sem) {
        return name_atom(local, vb);
    }
    bracket(sem)
}

/// `And(V(subj), prep_X(subj, obj))` → "subj V prep obj" when the two share a subject; else `None`.
fn verb_pp(left: &Exp, right: &Exp, vb: &Vb) -> Option<String> {
    let (lh, la) = app_spine(left);
    let (rh, ra) = app_spine(right);
    let ll = axiom_local(lh)?;
    let rl = axiom_local(rh)?;
    if !(ll.starts_with('v') && ll.contains('_')) {
        return None;
    }
    let (p, pp_subj, obj) = prep_parts(rl, &ra, vb)?;
    // Intransitive/PP verb: its sole arg is the subject; it must match the PP's first arg.
    let subj = match la.as_slice() {
        [s] => s,
        _ => return None,
    };
    if pretty_term(subj) != pretty_term(pp_subj) {
        return None;
    }
    Some(format!(
        "{} {} {p} {obj}",
        verbalize(subj, vb),
        name_atom(ll, vb),
    ))
}

/// A PP relation's preposition, subject and rendered object: `prep_X(subj, obj)`, or a quantity
/// relation `prep_X_value(subj, unit, quantity)` (D95), whose object is the quantity with its unit.
/// `None` for anything else.
fn prep_parts<'e>(local: &'e str, args: &[&'e Exp], vb: &Vb) -> Option<(&'e str, &'e Exp, String)> {
    let p = local.strip_prefix("prep_")?;
    match (p.strip_suffix("_value"), args) {
        (Some(p), [subj, unit, quantity]) => Some((p, subj, quantity_text(quantity, unit))),
        (None, [subj, obj]) => Some((p, subj, verbalize(obj, vb))),
        _ => None,
    }
}

/// A factor comparative (D95 slice 9): the order `gt(b, a)` beside `fold_lower(N, a, b)`, or `gt(a,
/// b)` beside `fold_higher(N, a, b)`, reads once, as the factor's words say the order.
fn factor_comparison(order: &Exp, fold: &Exp, vb: &Vb) -> Option<String> {
    let (oh, o) = app_spine(order);
    let (fh, f) = app_spine(fold);
    let (Some("gt"), [big, small]) = (axiom_local(oh), o.as_slice()) else {
        return None;
    };
    let agrees = match (axiom_local(fh), f.as_slice()) {
        (Some("fold_lower"), [_, a, b]) => big == b && small == a,
        (Some("fold_higher"), [_, a, b]) => big == a && small == b,
        _ => false,
    };
    agrees.then(|| verbalize(fold, vb))
}

/// The preposition a relational degree governs, from its atom: `deg_{loc}_rel_{p}` names `p`, one
/// relation per preposition (eigenius#263, D97 decision 6); a bare `…_rel` is read with `on`, as
/// before the preposition joined the name. `None` for any other atom.
fn relational_degree_preposition(atom: &str) -> Option<&'static str> {
    if atom.ends_with("_rel") {
        return Some("on");
    }
    let (_, slug) = atom.split_once("_rel_")?;
    crate::dcg::category::preposition_of_slug(slug)
}

/// The number of `T` a count comparative compares, as `(T, x, median)`: `card(T, x)`, or with its
/// statistic `median_over(λm. card(T, m), x)`. A bare `λm. card(T, m)` gives `x` as the bound `m`.
fn counted(e: &Exp) -> Option<(&Exp, &Exp, bool)> {
    if let Exp::Lam(_, body) = e {
        return counted(body);
    }
    let (h, a) = app_spine(e);
    match (axiom_local(h), a.as_slice()) {
        (Some("card"), [t, x]) => Some((*t, *x, false)),
        (Some("median_over"), [f, x]) => match counted(f)? {
            (t, _, false) => Some((t, *x, true)),
            _ => None,
        },
        _ => None,
    }
}

/// A quantity or difference term with its unit, as `6203/20 K` — the base-unit value the chain
/// holds, not the unit the author wrote, which is in `enc:prose` (D93). `π` powers are shown.
fn quantity_text(quantity: &Exp, unit: &Exp) -> String {
    if let Exp::Ann(inner, _) = quantity {
        return quantity_text(inner, unit);
    }
    // A bounded value, already rendered as its constraint by [`render_bounds`].
    if let Exp::LitString(constraint) = quantity {
        return constraint.clone();
    }
    let Exp::InductiveCtor(_, _, parts) = quantity else {
        return pretty_term(quantity);
    };
    let [Exp::LitRat(c), Exp::LitInt(pi)] = parts.as_slice() else {
        return pretty_term(quantity);
    };
    let mut text = c.to_canonical_string();
    if *pi != 0 {
        text.push_str(&format!("·π^{pi}"));
    }
    match unit {
        Exp::LitUnit(u) if !u.is_dimensionless() => format!("{text} {}", u.to_canonical_string()),
        _ => text,
    }
}

/// A bounded value as the `unit_constraint` combinator builds it and the gate reads it back (D95
/// slice 6): `∀P:Prop. (∀q:Quantity(u). And(C, body) → P) → P`. Returns `q`, the constraint `C` and
/// the consumer's `body`.
fn bounded_exists(sem: &Exp) -> Option<(&str, &Exp, &Exp)> {
    let Exp::Pi(Patt::Var(p), prop, cod) = sem else {
        return None;
    };
    if !matches!(prop.as_ref(), Exp::Sort(l) if l.as_nat() == Some(0)) {
        return None;
    }
    let (Exp::Pi(Patt::Var(q), ty, arr), Exp::Var(p2)) = as_arrow(cod)? else {
        return None;
    };
    let (conjunction, Exp::Var(p3)) = as_arrow(arr)? else {
        return None;
    };
    let (head, _, ty_args) = ty.as_const_spine()?;
    if p2 != p || p3 != p || !head.as_str().ends_with("units:Quantity") || ty_args.len() != 1 {
        return None;
    }
    let (and, _, parts) = conjunction.as_const_spine()?;
    match parts.as_slice() {
        [c, body] if and.as_str().ends_with("logic:And") => Some((q.as_str(), c, body)),
        _ => None,
    }
}

/// The words for a constraint on the value `q`: `units:lt(u, q, b)` is "less than b",
/// `units:lt(u, b, q)` "more than b", `le` "at most" / "at least", `approx` "about". A bound on a
/// bound (`less than about 2 h`) is the constraint relating `q` to an inner value, then that value's
/// own constraint.
fn bound_text(c: &Exp, q: &str) -> Option<String> {
    // A range: `And(le(u, lo, q), le(u, q, hi))` is "from lo to hi" (D95 slice 6c).
    if let Some((and, _, parts)) = c.as_const_spine() {
        if let ([low, high], true) = (parts.as_slice(), and.as_str().ends_with("logic:And")) {
            let (("at least", lo), ("at most", hi)) = (bound_words(low, q)?, bound_words(high, q)?)
            else {
                return None;
            };
            let (_, args) = app_spine(low);
            return Some(format!(
                "from {} to {}",
                quantity_text(lo, args[0]),
                quantity_text(hi, args[0])
            ));
        }
    }
    if let Some((inner, c2, rel)) = bounded_exists(c) {
        let (words, other) = bound_words(rel, q)?;
        if !matches!(other, Exp::Var(v) if v == inner) {
            return None;
        }
        return Some(format!("{words} {}", bound_text(c2, inner)?));
    }
    let (words, other) = bound_words(c, q)?;
    let (_, args) = app_spine(c);
    Some(format!("{words} {}", quantity_text(other, args[0])))
}

/// A `units:` order relation over `q` and one other argument, as the words that say it with `q` as
/// the subject, and that other argument.
fn bound_words<'e>(rel: &'e Exp, q: &str) -> Option<(&'static str, &'e Exp)> {
    let (Exp::EigonAxiom(head), args) = app_spine(rel) else {
        return None;
    };
    let [_unit, x, y] = args.as_slice() else {
        return None;
    };
    let local = head.as_str().strip_prefix("urn:eigenius:units:")?;
    let is_q = |e: &Exp| matches!(e, Exp::Var(v) if v == q);
    match (local, is_q(x), is_q(y)) {
        ("lt", true, false) => Some(("less than", y)),
        ("lt", false, true) => Some(("more than", x)),
        ("le", true, false) => Some(("at most", y)),
        ("le", false, true) => Some(("at least", x)),
        ("approx", true, false) => Some(("about", y)),
        _ => None,
    }
}

/// Whether `sem` contains a bounded value [`render_bounds`] can render.
fn has_bound(sem: &Exp) -> bool {
    if let Some((q, c, _)) = bounded_exists(sem) {
        if bound_text(c, q).is_some() {
            return true;
        }
    }
    match sem {
        Exp::App(f, x) => has_bound(f) || has_bound(x),
        Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) | Exp::Ann(b, _) => has_bound(b),
        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Arrow(a, b) | Exp::Pair(a, b) => {
            has_bound(a) || has_bound(b)
        }
        Exp::InductiveCtor(_, _, args) => args.iter().any(has_bound),
        _ => false,
    }
}

/// `sem` with each bounded value replaced by its body, the value rendered as its constraint — a
/// string literal [`quantity_text`] prints as is.
fn render_bounds(sem: &Exp) -> Exp {
    if let Some((q, c, body)) = bounded_exists(sem) {
        if let Some(text) = bound_text(c, q) {
            return render_bounds(&subst_var(body, q, &Exp::LitString(text)));
        }
    }
    let go = |x: &Exp| Box::new(render_bounds(x));
    match sem {
        Exp::App(f, x) => Exp::App(go(f), go(x)),
        Exp::Lam(p, b) => Exp::Lam(p.clone(), go(b)),
        Exp::Pi(p, a, b) => Exp::Pi(p.clone(), go(a), go(b)),
        Exp::Sig(p, a, b) => Exp::Sig(p.clone(), go(a), go(b)),
        Exp::Arrow(a, b) => Exp::Arrow(go(a), go(b)),
        Exp::Pair(a, b) => Exp::Pair(go(a), go(b)),
        Exp::Fst(x) => Exp::Fst(go(x)),
        Exp::Snd(x) => Exp::Snd(go(x)),
        Exp::Ann(x, t) => Exp::Ann(go(x), t.clone()),
        Exp::InductiveCtor(d, n, args) => Exp::InductiveCtor(
            d.clone(),
            n.clone(),
            args.iter().map(render_bounds).collect(),
        ),
        other => other.clone(),
    }
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
    if vb.register == Register::Expanded {
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
                if let Some((p, _, obj)) = prep_parts(p, &a, vb) {
                    post.push(format!("{p} {obj}"));
                }
            }
            // A prenominal measure phrase (D95): "10 μM etoposide" → "a 1/100 m^-3·mol etoposide".
            Some("has_quantity") if a.len() == 3 => pre.push(quantity_text(a[2], a[1])),
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
                if let Some((p, _, obj)) = prep_parts(p, &a, vb) {
                    parts.push(format!("{p} {obj}"));
                }
            }
            Some("has_quantity") if a.len() == 3 => {
                parts.push(format!("has-quantity {}", quantity_text(a[2], a[1])))
            }
            Some("is_a") if a.len() == 2 => parts.push(format!("is-a {}", bare_np(a[1], vb))),
            Some("named") if a.len() == 2 => parts.push(format!("named {}", verbalize(a[1], vb))),
            Some("poss_of") if a.len() == 2 || a.len() == 3 => {
                parts.push(format!("possessed-by {}", verbalize(a[a.len() - 1], vb)))
            }
            // An intersective adjective on the head: `a02734544(x)`, «genetic».
            Some(adj) if is_adjective(adj) && matches!(a.as_slice(), [Exp::Var(_)]) => {
                parts.push(format!("is {}", name_atom(adj, vb)))
            }
            // A counted relation to the head: `has_count(T, λy. prep_X(x, y), q)`, «an interaction
            // between two genetic events» → "between 2 T".
            Some("has_count") if a.len() == 3 => match counted_relation(a[1], vb) {
                Some(p) => {
                    let unit = Exp::LitUnit(crate::units::Unit::dimensionless());
                    parts.push(format!(
                        "{p} {} {}",
                        quantity_text(a[2], &unit),
                        bare_np(a[0], vb)
                    ));
                }
                None => parts.push(verbalize(c, vb)),
            },
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

/// The preposition of a count's predicate that relates the head to the counted phrase:
/// `λy. prep_X(x, y)` → `X`. `None` for any other predicate.
fn counted_relation<'e>(pred: &'e Exp, vb: &Vb) -> Option<&'e str> {
    let Exp::Lam(Patt::Var(y), body) = pred else {
        return None;
    };
    let (h, a) = app_spine(body);
    let (p, _, _) = prep_parts(axiom_local(h)?, &a, vb)?;
    let mut obj = *a.get(1)?;
    while let Exp::Fst(x) | Exp::Snd(x) = obj {
        obj = x;
    }
    matches!(obj, Exp::Var(v) if v == y).then_some(p)
}

fn flatten_and_exp<'a>(e: &'a Exp, out: &mut Vec<&'a Exp>) {
    if let Some((iri, _, args)) = e.as_const_spine() {
        if iri.as_str().ends_with("logic:And") && args.len() == 2 {
            flatten_and_exp(args[0], out);
            flatten_and_exp(args[1], out);
            return;
        }
    }
    out.push(e);
}

/// What a reading's predication says, where its form decides something no link shows: a class
/// generalisation (`subclass_of(X, Y)`, every X is a Y) against a statement about a kind as a whole
/// (`is_a(kind_of(X), Y)`). `None` for any other predication.
pub fn predication(sem: &Exp) -> Option<&'static str> {
    let (h, a) = app_spine(sem);
    match (axiom_local(h)?, a.as_slice()) {
        ("subclass_of", [_, _]) => {
            Some("a generalisation: it says something of every member of the subject's class")
        }
        ("is_a", [subject, _]) if axiom_local(app_spine(subject).0) == Some("kind_of") => {
            Some("a statement about the subject's kind as a whole")
        }
        _ => None,
    }
}

/// One way a phrase hangs on another in a reading, by grammatical function: the `dependent` is the
/// `function` of its `host` — «with sequencing» an adverbial of «ascertained», «MSI» a noun
/// modifier of «status». Both ends are named by their head word. Two analyses that attach a phrase
/// differently differ in a link, which the ranker's structure call states (eigenius#264).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Link {
    pub function: Function,
    pub dependent: String,
    pub host: String,
}

/// A grammatical function, as a grammar of English names it. A preposition-headed function carries
/// its preposition.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Function {
    /// The subject of a verb.
    Subject,
    /// A verb's object.
    Object,
    /// The object of a prepositional verb, which takes it through a preposition («respond to X»).
    PrepositionalObject,
    /// A verb's object complement, marked by its preposition («identified WRN as the top
    /// dependency»).
    ObjectComplement(String),
    /// A noun modifying a noun, the relation between them unstated («MSI status»).
    NounModifier,
    /// An adjective in a noun phrase («genetic events»).
    Adjective,
    /// An adjective predicated of its subject («WRN was essential»).
    PredicateAdjective,
    /// A numeral counting the host; the dependent is empty.
    Numeral,
    /// A prepositional phrase after a noun, inside its phrase («data from screens»).
    Postmodifier(String),
    /// A prepositional phrase modifying a verb («ascertained … with sequencing»).
    Adverbial(String),
    /// A prepositional phrase modifying a predicated adjective («essential in MSI models»).
    PredicateAdverbial(String),
    /// A prepositional phrase predicated of a copula's subject beside its complement («X is an
    /// interaction» and «X is between two events»).
    SecondPredicate(String),
    /// The prepositional complement an adjective governs («dependent on WRN»).
    AdjectiveComplement(String),
}

/// The [`Link`]s of a reading, sorted and deduplicated. Pass names that give each sense atom the
/// words that introduced it (the reading's derivation knows them), so the links of one analysis
/// name the same words whatever senses its readings take.
pub fn structure_links(sem: &Exp, vb: &Vb) -> Vec<Link> {
    let mut out = Vec::new();
    collect_links(sem, vb, &mut Vec::new(), &mut out);
    out.sort();
    out.dedup();
    out
}

/// What a bound variable stands for while the links of its scope are collected: its variable, the
/// head word of its phrase, and whether it binds a noun phrase's restrictor (`Σ`, a count's
/// predicate) — where an adjective is attributive — or a quantifier's scope (`Π`), where it is
/// predicated.
type Binding<'e> = (&'e str, String, bool);

/// Walk a reading, tracking what each bound variable stands for, and record every link on the way.
fn collect_links<'e>(e: &'e Exp, vb: &Vb, env: &mut Vec<Binding<'e>>, out: &mut Vec<Link>) {
    if let Exp::Sig(p, dom, body) | Exp::Pi(p, dom, body) = e {
        collect_links(dom, vb, env, out);
        let restrictor = matches!(e, Exp::Sig(..));
        return with_binder(p, head_word(dom, vb, env), restrictor, body, vb, env, out);
    }
    if let Some((iri, _, args)) = e.as_const_spine() {
        if iri.as_str().ends_with("logic:And") && args.len() == 2 {
            return conjunction_links(e, vb, env, out);
        }
    }
    let (h, args) = app_spine(e);
    if args.is_empty() {
        for c in child_exps(e) {
            collect_links(c, vb, env, out);
        }
        return;
    }
    // An application is read once, with all its arguments, then each argument is walked: a
    // partial application would read `v(obj)` as a verb whose only argument is its subject.
    // `has_count(ΣG:N. …, λG. P(G), q)` — the predicate's variable is the counted N.
    if let (Some("has_count"), [counted, Exp::Lam(p, body), rest @ ..]) =
        (axiom_local(h), args.as_slice())
    {
        collect_links(counted, vb, env, out);
        let head = head_word(counted, vb, env);
        out.push(Link {
            function: Function::Numeral,
            dependent: String::new(),
            host: head.clone(),
        });
        with_binder(p, head, true, body, vb, env, out);
        for r in rest {
            collect_links(r, vb, env, out);
        }
        return;
    }
    if let Some(local) = axiom_local(h) {
        out.extend(link_of(local, &args, vb, env));
        if is_verb_frame(local) {
            verb_links(local, &args, vb, env, out);
        }
    }
    collect_links(h, vb, env, out);
    for a in args {
        collect_links(a, vb, env, out);
    }
}

fn with_binder<'e>(
    p: &'e Patt,
    head: String,
    restrictor: bool,
    body: &'e Exp,
    vb: &Vb,
    env: &mut Vec<Binding<'e>>,
    out: &mut Vec<Link>,
) {
    match p {
        Patt::Var(v) => {
            env.push((v.as_str(), head, restrictor));
            collect_links(body, vb, env, out);
            env.pop();
        }
        _ => collect_links(body, vb, env, out),
    }
}

/// A conjunction's links. A PP conjoined with a clause whose subject is its first argument — the
/// verb-adjunct encoding, `And(V(…, s), prep_P(s, o))` («We ascertained MSI status with
/// sequencing.») — is about that clause's verb, predicated adjective or copula, not the subject.
/// Every other conjunct is walked as itself.
fn conjunction_links<'e>(e: &'e Exp, vb: &Vb, env: &mut Vec<Binding<'e>>, out: &mut Vec<Link>) {
    let mut conj = Vec::new();
    flatten_and_exp(e, &mut conj);
    for &c in &conj {
        match adjunct_of(c, &conj, vb, env) {
            Some((function, obj, hosts)) => {
                let dependent = head_word(obj, vb, env);
                for host in hosts {
                    out.push(Link {
                        function: function.clone(),
                        dependent: dependent.clone(),
                        host,
                    });
                }
                collect_links(obj, vb, env, out);
            }
            None => collect_links(c, vb, env, out),
        }
    }
}

/// The function a conjoined PP `prep_P(s, o)` has, its object, and the words it modifies, when a
/// sibling conjunct has `s` as its subject: an adverbial of each such verb; else of a predicated
/// adjective; else a second predicate beside a copula. `None` for any other conjunct.
fn adjunct_of<'e>(
    pp: &'e Exp,
    conj: &[&'e Exp],
    vb: &Vb,
    env: &[Binding<'e>],
) -> Option<(Function, &'e Exp, Vec<String>)> {
    let (h, a) = app_spine(pp);
    let p = axiom_local(h)?.strip_prefix("prep_")?;
    let [subject, obj] = a.as_slice() else {
        return None;
    };
    if p.ends_with("_value") {
        return None;
    }
    let subject = pretty_term(subject);
    let about = |e: &Exp| pretty_term(e) == subject;
    let (mut verbs, mut adjectives, mut copula) = (Vec::new(), Vec::new(), false);
    for &c in conj.iter().filter(|&&c| !std::ptr::eq(c, pp)) {
        let (ch, ca) = app_spine(c);
        match axiom_local(ch) {
            Some(l) if is_verb_frame(l) && ca.last().is_some_and(|s| about(s)) => {
                verbs.push(atom_word(l, vb))
            }
            Some("is_a") if ca.first().is_some_and(|s| about(s)) => copula = true,
            _ => {
                if let Some((adj, s)) = predicated_adjective(c) {
                    if about(s) {
                        adjectives.push(atom_word(adj, vb));
                    }
                }
            }
        }
    }
    let p = p.to_string();
    if !verbs.is_empty() {
        Some((Function::Adverbial(p), obj, verbs))
    } else if !adjectives.is_empty() {
        Some((Function::PredicateAdverbial(p), obj, adjectives))
    } else if copula {
        let (_, a) = app_spine(pp);
        Some((
            Function::SecondPredicate(p),
            obj,
            vec![head_word(a[0], vb, env)],
        ))
    } else {
        None
    }
}

/// An adjective predicated of a subject: a gradable one's `gt(deg_{a}(s), std_{a})` or an
/// intersective one's `{a}(s)`. The adjective's atom and the subject.
fn predicated_adjective(c: &Exp) -> Option<(&str, &Exp)> {
    let (h, a) = app_spine(c);
    let local = axiom_local(h)?;
    match (local, a.as_slice()) {
        ("gt" | "lt", [degree, _]) => {
            let (dh, da) = app_spine(degree);
            let adj = axiom_local(dh)?;
            match da.as_slice() {
                [s] if is_adjective(adj) => Some((adj, s)),
                _ => None,
            }
        }
        (adj, [s]) if is_adjective(adj) => Some((adj, s)),
        _ => None,
    }
}

/// Whether `e` is a variable bound by a noun phrase's restrictor (through projections).
fn in_restrictor(e: &Exp, env: &[Binding]) -> bool {
    let mut e = e;
    while let Exp::Fst(x) | Exp::Snd(x) = e {
        e = x;
    }
    matches!(e, Exp::Var(v) if env.iter().rev().find(|(b, _, _)| *b == v.as_str()).is_some_and(|(_, _, r)| *r))
}

/// The link an application states, if it is one: `prep_p(host, dependent)`, a compound
/// `compound_kind(head, modifier)`, a governed degree `deg_{loc}_rel_{p}(dependent, subject)`, or
/// an adjective on a phrase.
fn link_of(local: &str, args: &[&Exp], vb: &Vb, env: &[Binding]) -> Option<Link> {
    let link = |function: Function, dependent: &Exp, host: String| Link {
        function,
        dependent: head_word(dependent, vb, env),
        host,
    };
    match args {
        [host, obj] if local.starts_with("prep_") && !local.ends_with("_value") => Some(link(
            Function::Postmodifier(local["prep_".len()..].to_string()),
            obj,
            head_word(host, vb, env),
        )),
        [head, modifier] if matches!(local, "compound_kind" | "compound") => Some(link(
            Function::NounModifier,
            modifier,
            head_word(head, vb, env),
        )),
        [obj, _subject] if local.starts_with("deg_") => {
            let prep = relational_degree_preposition(local)?;
            Some(link(
                Function::AdjectiveComplement(prep.to_string()),
                obj,
                atom_word(local, vb),
            ))
        }
        // An adjective on a phrase: a gradable one's degree `deg_{a}(x)`, or an intersective one's
        // predicate `{a}(x)` — attributive inside a restrictor, predicated elsewhere.
        [subject] if is_adjective(local) => Some(Link {
            function: if in_restrictor(subject, env) {
                Function::Adjective
            } else {
                Function::PredicateAdjective
            },
            dependent: atom_word(local, vb),
            host: head_word(subject, vb, env),
        }),
        _ => None,
    }
}

/// A verb's arguments, as links to the verb: its last argument is the subject, the others its
/// object and complements (`v…_t(object, subject)`, `v…_as(object, complement, subject)`). A
/// PP-oblique verb's object (`v…_p`) is a prepositional object; an essive verb's middle argument
/// its `as` complement. A clausal argument names no phrase and is skipped.
fn verb_links(local: &str, args: &[&Exp], vb: &Vb, env: &[Binding], out: &mut Vec<Link>) {
    let verb = atom_word(local, vb);
    let frame = verb_frame(local);
    let last = args.len().saturating_sub(1);
    for (i, a) in args.iter().enumerate().filter(|(_, a)| is_phrase(a)) {
        let function = match (i, frame) {
            (i, _) if i == last => Function::Subject,
            (0, f) if f == "p" || named_preposition(f).is_some() => Function::PrepositionalObject,
            (1, "as") => Function::ObjectComplement("as".to_string()),
            _ => Function::Object,
        };
        out.push(Link {
            function,
            dependent: head_word(a, vb, env),
            host: verb.clone(),
        });
    }
}

/// A verb atom's frame tag, after the offset: `t` in `v00254150_t`, `p_to` in `v00717358_p_to`.
fn verb_frame(local: &str) -> &str {
    local.split_once('_').map_or("", |(_, f)| f)
}

/// The preposition a PP-oblique frame tag names (`p_to` → `to`, `p_out_of` → `out of`, D97 slice 2);
/// `None` for any other tag, the any-preposition `p` among them.
fn named_preposition(frame: &str) -> Option<&'static str> {
    crate::dcg::category::preposition_of_slug(frame.strip_prefix("p_")?)
}

/// A verb's frame atom: `v{offset}_{frame}`.
fn is_verb_frame(local: &str) -> bool {
    local.split_once('_').is_some_and(|(key, _)| {
        key.len() == 9 && key.starts_with('v') && key[1..].bytes().all(|b| b.is_ascii_digit())
    })
}

/// A term that names a phrase (an entity or a kind), not a clause.
fn is_phrase(e: &Exp) -> bool {
    match e {
        Exp::Var(_)
        | Exp::Sig(..)
        | Exp::Fst(_)
        | Exp::EigonClass(_)
        | Exp::EigonResource(_)
        | Exp::EigonAxiom(_) => true,
        _ => matches!(
            app_spine(e),
            (h, args) if !args.is_empty()
                && matches!(axiom_local(h), Some("kind_of" | "the" | "the_count" | "has_count"))
        ),
    }
}

/// An adjective's degree (`deg_a01580306`) or predicate (`a02734544`) — not a governed degree, a
/// standard, or a verb.
fn is_adjective(local: &str) -> bool {
    let bare = local.strip_prefix("deg_").unwrap_or(local);
    bare.len() > 4 && bare.starts_with(['a', 's']) && bare[1..].bytes().all(|b| b.is_ascii_digit())
}

/// The head word of a phrase: an atom's word, a bound variable's phrase, or the head of the
/// first argument through `kind_of`, `the`, a count, or a projection.
fn head_word(e: &Exp, vb: &Vb, env: &[Binding]) -> String {
    match e {
        Exp::Var(v) => env
            .iter()
            .rev()
            .find(|(b, _, _)| *b == v.as_str())
            .map(|(_, h, _)| h.clone())
            .unwrap_or_else(|| "…".to_string()),
        Exp::Sig(_, dom, _) | Exp::Pi(_, dom, _) => head_word(dom, vb, env),
        Exp::Fst(x) | Exp::Snd(x) | Exp::Ann(x, _) => head_word(x, vb, env),
        _ => match app_spine(e) {
            (_, args) if !args.is_empty() => head_word(args[0], vb, env),
            (h, _) => match axiom_local(h) {
                Some(local) if local.bytes().filter(u8::is_ascii_digit).count() >= 4 => {
                    atom_word(local, vb)
                }
                _ => verbalize(e, vb)
                    .trim_matches(|c| c == '«' || c == '»')
                    .to_string(),
            },
        },
    }
}

#[cfg(test)]
mod register_tests {
    use super::*;

    /// eigenius#263 (D97 decision 6): a relational degree's atom carries its preposition, which the
    /// comparative reads back; a bare `…_rel` keeps the `on` it was read with before.
    #[test]
    fn a_relational_degree_names_its_preposition() {
        assert_eq!(
            relational_degree_preposition("deg_a00725772_rel_on"),
            Some("on")
        );
        assert_eq!(
            relational_degree_preposition("deg_a00482049_rel_with"),
            Some("with")
        );
        assert_eq!(
            relational_degree_preposition("deg_dependent_rel"),
            Some("on")
        );
        assert_eq!(
            relational_degree_preposition("deg_a01234567_rel_out_of"),
            Some("out of")
        );
        assert_eq!(relational_degree_preposition("deg_a00725772"), None);
        assert_eq!(relational_degree_preposition("deg_a00725772_rel_xyz"), None);
    }
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

    /// An OBO term renders by its label and CURIE, `«Ovarian neoplasm» [HP:0100615]`, and its
    /// definition reaches the legend. Its IRI's last segment alone is `0100615`, which named nothing
    /// — the ranker chose between HPO readings without seeing what they were.
    #[test]
    fn an_obo_term_renders_by_its_label_and_curie() {
        let iri = Iri::parse("urn:obo:HP:0100615").unwrap();
        let mut r = crate::ontology::resource::Resource::new(iri);
        r.set(
            Iri::parse("urn:rdfs:label").unwrap(),
            Value::String("Ovarian neoplasm".into()),
        );
        r.set(
            Iri::parse("urn:eigenius:core:description").unwrap(),
            Value::String("A tumor (abnormal growth of tissue) of the ovary.".into()),
        );
        let mut b = LayerBuilder::new("t", None);
        b.add_resource(r).unwrap();
        let l = Arc::new(b.build(LayerStorage::in_memory()));
        let names = BTreeMap::new();
        let vb = Vb::expanded(&names, &l);
        let term = cls("urn:obo:HP:0100615");
        assert_eq!(verbalize(&term, &vb), "«Ovarian neoplasm» [HP:0100615]");
        let notes = concept_notes(&[&term], &vb);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, "HP:0100615");
        assert_eq!(notes[0].label, "Ovarian neoplasm");
        assert_eq!(
            notes[0].definition.as_deref(),
            Some("A tumor (abnormal growth of tissue) of the ovary.")
        );
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
            uparams: Vec::new(),
            iri: Iri::parse("urn:eigenius:logic:And").expect("iri"),
            name: "And".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(0),
            ctors: Vec::new(),
        });
        let compound = sig(
            cls("urn:eigenius:wn:n00407535"),
            Exp::const_applied(
                and_decl.iri.clone(),
                Vec::new(),
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

    /// A measured value (D95) renders in base units: predicated, before its noun, after a
    /// preposition, and in the expanded register as a named commitment.
    #[test]
    fn a_measured_value_renders_with_its_unit() {
        let l = layer();
        let names = BTreeMap::new();
        let kelvin = Exp::LitUnit(crate::units::Unit::parse_canonical("K").expect("unit"));
        let q = Exp::InductiveCtor(
            Iri::parse("urn:eigenius:units:Quantity").expect("iri"),
            "mk_quantity".to_string(),
            vec![
                Exp::LitRat(crate::numeric::Rational::new(6203.into(), 20.into()).expect("q")),
                Exp::LitInt(0),
            ],
        );
        let app3 = |axiom: &str, a: Exp| {
            Exp::App(
                Box::new(app2(axiom, a, kelvin.clone())),
                Box::new(q.clone()),
            )
        };
        let hela = || Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:hela").expect("iri"));
        let surface = Vb::surface(&names, &l);
        assert_eq!(
            verbalize(
                &app3("urn:eigenius:ontology:has_quantity", hela()),
                &surface
            ),
            "hela is 6203/20 K"
        );
        assert_eq!(
            verbalize(
                &app3("urn:eigenius:ontology:prep_at_value", hela()),
                &surface
            ),
            "hela at 6203/20 K"
        );
        let medium = sig(
            cls("urn:eigenius:lexicon:Medium"),
            app3("urn:eigenius:ontology:has_quantity", Exp::Var("x0".into())),
        );
        assert_eq!(verbalize(&medium, &surface), "a 6203/20 K Medium");
        assert_eq!(
            verbalize(&medium, &Vb::expanded(&names, &l)),
            "a [Medium] + has-quantity 6203/20 K"
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

    fn app1(axiom: &str, a: Exp) -> Exp {
        Exp::App(
            Box::new(Exp::EigonAxiom(Iri::parse(axiom).expect("iri"))),
            Box::new(a),
        )
    }

    /// «other strong biomarkers for vulnerabilities» with the PP inside the biomarkers phrase.
    fn nested(vulnerability: &str) -> Exp {
        sig(
            cls("urn:eigenius:umlscui:C0005516"),
            app2(
                "urn:eigenius:ontology:prep_for",
                Exp::Var("x0".into()),
                app1("urn:eigenius:ontology:kind_of", cls(vulnerability)),
            ),
        )
    }

    fn surface_names() -> BTreeMap<String, String> {
        [
            ("C0005516", "biomarkers"),
            ("C1821973", "vulnerabilities"),
            ("n05042871", "vulnerabilities"),
            ("n13780719", "relationship"),
            ("a00725772", "dependent"),
            ("C0388246", "WRN"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn link(function: Function, dependent: &str, host: &str) -> Link {
        Link {
            function,
            dependent: dependent.into(),
            host: host.into(),
        }
    }

    /// The contrast of eigenius#264's witness: «for vulnerabilities» postmodifies the biomarkers
    /// (the Σ's head, through its bound variable) or «the relationship» (through `the(…).1`).
    #[test]
    fn links_name_where_a_phrase_attaches() {
        let (l, names) = (layer(), surface_names());
        let vb = Vb::surface(&names, &l);
        let for_ = |host: &str| {
            link(
                Function::Postmodifier("for".into()),
                "vulnerabilities",
                host,
            )
        };
        assert_eq!(
            structure_links(&nested("urn:eigenius:umlscui:C1821973"), &vb),
            vec![for_("biomarkers")]
        );
        let the_relationship = Exp::Fst(Box::new(app1(
            "urn:eigenius:ontology:the",
            cls("urn:eigenius:wn:n13780719"),
        )));
        let flat = app2(
            "urn:eigenius:ontology:prep_for",
            the_relationship,
            app1(
                "urn:eigenius:ontology:kind_of",
                cls("urn:eigenius:umlscui:C1821973"),
            ),
        );
        assert_eq!(structure_links(&flat, &vb), vec![for_("relationship")]);
        // A governed preposition is the adjective's complement (D97 decision 6).
        let dependent_on = app2(
            "urn:eigenius:ontology:deg_a00725772_rel_on",
            app1(
                "urn:eigenius:ontology:kind_of",
                cls("urn:eigenius:umlscui:C0388246"),
            ),
            Exp::Var("x0".into()),
        );
        assert_eq!(
            structure_links(&dependent_on, &vb),
            vec![link(
                Function::AdjectiveComplement("on".into()),
                "WRN",
                "dependent"
            )]
        );
    }

    /// An adjective is attributive inside a noun phrase's restrictor and predicated outside it.
    #[test]
    fn an_adjective_in_a_restrictor_is_attributive() {
        let l = layer();
        let names: BTreeMap<String, String> = [
            ("n00029378", "events"),
            ("a02734544", "genetic"),
            ("C0388246", "WRN"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let vb = Vb::surface(&names, &l);
        let events = sig(
            cls("urn:eigenius:wn:n00029378"),
            app1("urn:eigenius:wn:a02734544", Exp::Var("x0".into())),
        );
        assert_eq!(
            structure_links(&events, &vb),
            vec![link(Function::Adjective, "genetic", "events")]
        );
        let wrn_is_genetic = app1(
            "urn:eigenius:wn:a02734544",
            cls("urn:eigenius:umlscui:C0388246"),
        );
        assert_eq!(
            structure_links(&wrn_is_genetic, &vb),
            vec![link(Function::PredicateAdjective, "genetic", "WRN")]
        );
    }

    /// A verb's arguments are links to it — read once, with the whole application, so the object
    /// is never also taken for a one-argument verb's subject. A PP-oblique verb's object is a
    /// prepositional object.
    #[test]
    fn a_verb_links_its_object_and_subject_once() {
        let (l, names) = (layer(), surface_names());
        let vb = Vb::surface(&names, &l);
        let with_frame = |frame: &str| {
            app2(
                &format!("urn:eigenius:ontology:v00644583_{frame}"),
                app1(
                    "urn:eigenius:ontology:kind_of",
                    cls("urn:eigenius:umlscui:C0005516"),
                ),
                cls("urn:eigenius:wn:n13780719"),
            )
        };
        let verb = "v00644583";
        assert_eq!(
            structure_links(&with_frame("t"), &vb),
            vec![
                link(Function::Subject, "relationship", verb),
                link(Function::Object, "biomarkers", verb),
            ]
        );
        for frame in ["p", "p_to", "p_out_of"] {
            assert_eq!(
                structure_links(&with_frame(frame), &vb),
                vec![
                    link(Function::Subject, "relationship", verb),
                    link(Function::PrepositionalObject, "biomarkers", verb),
                ],
                "{frame}"
            );
        }
        // The gloss says a named preposition (D97 slice 2); the any-preposition frame has none to say.
        let named = verbalize(&with_frame("p_to"), &vb);
        let any = verbalize(&with_frame("p"), &vb);
        assert_eq!(named.replacen(" to ", " ", 1), any, "{named}");
        // A tag that only starts like a named one is not a PP frame.
        assert_eq!(named_preposition("p_nowhere"), None);
        assert_eq!(named_preposition("p_out_of"), Some("out of"));
    }

    fn and(a: Exp, b: Exp) -> Exp {
        let and = Exp::Const(Iri::parse("urn:eigenius:logic:And").expect("iri"), vec![]);
        Exp::App(Box::new(Exp::App(Box::new(and), Box::new(a))), Box::new(b))
    }

    /// «We ascertained MSI status with sequencing.»: the verb-adjunct encoding conjoins the PP to
    /// the clause with the subject as its first argument; the PP is an adverbial of the verb. Beside
    /// a predicated adjective it is the predicate's adverbial, beside a copula a second predicate.
    #[test]
    fn a_conjoined_pp_is_about_its_clause() {
        let l = layer();
        let names: BTreeMap<String, String> = [
            ("v00920000", "ascertained"),
            ("n13945000", "status"),
            ("n00649000", "sequencing"),
            ("n13780719", "we"),
            ("a01000000", "essential"),
            ("n00039021", "interaction"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let vb = Vb::surface(&names, &l);
        let we = || cls("urn:eigenius:wn:n13780719");
        let status = || {
            app1(
                "urn:eigenius:ontology:kind_of",
                cls("urn:eigenius:wn:n13945000"),
            )
        };
        let sequencing = || {
            app1(
                "urn:eigenius:ontology:kind_of",
                cls("urn:eigenius:wn:n00649000"),
            )
        };
        let with = |s: Exp| app2("urn:eigenius:ontology:prep_with", s, sequencing());
        let adjunct = and(
            app2("urn:eigenius:ontology:v00920000_t", status(), we()),
            with(we()),
        );
        let adverbial = link(
            Function::Adverbial("with".into()),
            "sequencing",
            "ascertained",
        );
        assert_eq!(
            structure_links(&adjunct, &vb),
            vec![
                link(Function::Subject, "we", "ascertained"),
                link(Function::Object, "status", "ascertained"),
                adverbial.clone(),
            ]
        );
        // A second adjunct of the same clause, nested in another conjunction, is the verb's too.
        let nested = and(adjunct.clone(), with(we()));
        assert!(structure_links(&nested, &vb).contains(&adverbial));
        // Surface keeps its words.
        assert_eq!(
            verbalize(&adjunct, &Vb::surface(&names, &l)),
            "we ascertained status and we with sequencing"
        );
        // A PP whose first argument is not the verb's subject is no adjunct.
        let not_adjunct = and(
            app2("urn:eigenius:ontology:v00920000_t", status(), we()),
            with(status()),
        );
        assert!(structure_links(&not_adjunct, &vb).contains(&link(
            Function::Postmodifier("with".into()),
            "sequencing",
            "status"
        )));
        // Beside a predicated adjective, the predicate's adverbial.
        let essential = app2(
            "urn:eigenius:ontology:gt",
            app1("urn:eigenius:ontology:deg_a01000000", we()),
            Exp::EigonAxiom(Iri::parse("urn:eigenius:ontology:std_a01000000").expect("iri")),
        );
        assert!(
            structure_links(&and(essential, with(we())), &vb).contains(&link(
                Function::PredicateAdverbial("with".into()),
                "sequencing",
                "essential"
            ))
        );
        // Beside a copula, a second predicate of the subject.
        let is_a = app2(
            "urn:eigenius:ontology:is_a",
            we(),
            cls("urn:eigenius:wn:n00039021"),
        );
        assert!(structure_links(&and(is_a, with(we())), &vb).contains(&link(
            Function::SecondPredicate("with".into()),
            "sequencing",
            "we"
        )));
    }

    /// «Synthetic lethality is an interaction between two genetic events.»: the counted,
    /// adjectival restrictor renders as relations of the head, not as a bracketed fragment.
    #[test]
    fn a_counted_relation_and_an_adjective_render_on_the_head() {
        let l = layer();
        let names: BTreeMap<String, String> = [
            ("C4280020", "Synthetic lethality"),
            ("n00039021", "interaction"),
            ("n00029378", "events"),
            ("a02734544", "genetic"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let two = Exp::InductiveCtor(
            Iri::parse("urn:eigenius:units:Quantity").expect("iri"),
            "mk_quantity".to_string(),
            vec![
                Exp::LitRat(crate::numeric::Rational::new(2.into(), 1.into()).expect("q")),
                Exp::LitInt(0),
            ],
        );
        let genetic_events = Exp::Sig(
            Patt::Var("y0".into()),
            Box::new(cls("urn:eigenius:wn:n00029378")),
            Box::new(app1(
                "urn:eigenius:ontology:a02734544",
                Exp::Var("y0".into()),
            )),
        );
        let between = Exp::Lam(
            Patt::Var("y".into()),
            Box::new(app2(
                "urn:eigenius:ontology:prep_between",
                Exp::Var("x0".into()),
                Exp::Fst(Box::new(Exp::Var("y".into()))),
            )),
        );
        let counted = Exp::App(
            Box::new(app2(
                "urn:eigenius:ontology:has_count",
                genetic_events,
                between,
            )),
            Box::new(two),
        );
        let reading = app2(
            "urn:eigenius:ontology:is_a",
            app1(
                "urn:eigenius:ontology:kind_of",
                cls("urn:eigenius:umlscui:C4280020"),
            ),
            sig(cls("urn:eigenius:wn:n00039021"), counted),
        );
        assert_eq!(
            verbalize(&reading, &Vb::expanded(&names, &l)),
            "«Synthetic lethality» [C4280020] is a «interaction» [n00039021] + between 2 «events» \
             [n00029378] + is «genetic» [a02734544]"
        );
        assert_eq!(
            predication(&reading),
            Some("a statement about the subject's kind as a whole")
        );
    }

    /// A document's named individual keeps its whole local name: «Project Achilles», not «ni».
    #[test]
    fn a_named_individual_is_named_whole() {
        assert_eq!(atom_key("ni_project_achilles"), "ni_project_achilles");
        assert_eq!(atom_key("v00618878_as"), "v00618878");
        assert_eq!(
            sense_key("doc:project_achilles").as_deref(),
            Some("ni_project_achilles")
        );
        assert_eq!(
            sense_key("wn:identify.v.00618878").as_deref(),
            Some("v00618878")
        );
        assert_eq!(sense_key("umls:C0388246").as_deref(), Some("C0388246"));
    }
}
