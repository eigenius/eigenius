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

//! **The lexicon** — everything the engine knows about words, and nothing about parsing them.
//!
//! Two layers, and the second is built on the first:
//!
//! 1. **Entry handling.** Resolve an entry's `sem` reference to a value ([`resolve_sem`]), build a parse
//!    [`Item`] from a committed entry ([`entry_to_item`]), and the **felicity gate** ([`gate_entry`]) —
//!    the trusted filter every LLM- or import-produced entry must pass. An entry is admitted iff
//!    `⟦cat⟧ ≡ sem_type` and its `sem` actually inhabits `⟦cat⟧`. The kernel is the oracle.
//! 2. **The index.** [`LexicalIndex`] is a `form → entries` map over a layer's committed
//!    `lexicon:LexicalEntry` resources, resolving each through `entry_to_item` above. Lazy (a probe of
//!    an active `core:ValueIndex` — the production path at WordNet's 325k entries) or eager (a full
//!    chain scan) — behaviour-identical.
//!
//! [`LexicalLookup`] is the trait the parser sees: **two methods, and nothing else**. That is
//! deliberate. `LexicalIndex` began as exactly what its name says and grew a chart parser, a beam, a
//! sense cap, an LLM reranker, a felicity gate, and an anaphora resolver — because nothing structurally
//! prevented it. A concrete type invites accretion; a narrow trait refuses it. The parser
//! (`super::parse`) holds this as `Arc<dyn LexicalLookup>` and *cannot* call anything else.

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::check::{check, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::term::Exp;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::Iri;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use crate::layer::{normalize_value, resolve_active_value_indexes};
use crate::program::eigentt_type_mirror::decode_type;

use super::category::{denote_cat, type_eq};
use super::item::Item;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
}

/// Resolve an entry's `sem` reference to its EigenTT *value*: an axiom →
/// `EigonAxiom`, a class → `EigonClass`, an instance → `EigonResource`. Chain
/// entities become values the checker can type, not unbound `Var`s.
pub fn resolve_sem(layer: &Arc<Layer>, target: &Iri) -> Exp {
    let r = layer
        .resolve(target)
        .unwrap_or_else(|| panic!("sem target not found: {target}"));
    if r.is_instance_of(&iri("urn:eigenius:eigentt:Axiom")) {
        Exp::EigonAxiom(target.clone())
    } else if r.is_instance_of(&iri("urn:eigenius:core:Class")) {
        Exp::EigonClass(target.clone())
    } else {
        Exp::EigonResource(Box::new((*r).clone()))
    }
}

/// Resolve an entry's `sem` field *value* to its EigenTT term. `sem` is an
/// EigenTT term with two surface forms:
/// - a **reference** to a chain entity — the common case (a noun's class, a
///   verb's axiom, a named entity's resource), resolved by [`resolve_sem`]; the
///   reference is shorthand for that entity's `ConstRef` term;
/// - an **inline `type_expr` term** — a function word's λ-semantics, e.g. a
///   determiner's `λA:Set. λV:A→Prop. ∀x:A. V(x)`, which has no chain entity to
///   point at; decoded through the D47 codec.
pub fn resolve_sem_value(layer: &Arc<Layer>, sem_v: &Value) -> Result<Exp, String> {
    let target = match sem_v {
        Value::ResourceRef(i) => i.clone(),
        Value::String(s) => Iri::parse(s).map_err(|e| format!("sem iri: {e}"))?,
        // An inline EigenTT term value (rare — references are the norm).
        other => return decode_type(other, layer).map_err(|e| format!("sem decode: {e:?}")),
    };
    // A `lexicon:SemTerm` reference holds an inline λ-term: decode its `term`
    // field. (Any other reference — class / axiom / instance — goes through
    // `resolve_sem`'s entity dispatch.)
    let r = layer
        .resolve(&target)
        .ok_or_else(|| format!("sem target not found: {target}"))?;
    if r.is_instance_of(&iri("urn:eigenius:lexicon:SemTerm")) {
        let term_v = r
            .get(&iri("urn:eigenius:lexicon:term"))
            .ok_or("lexicon:SemTerm has no `term`")?;
        return decode_type(term_v, layer).map_err(|e| format!("sem term decode: {e:?}"));
    }
    Ok(resolve_sem(layer, &target))
}

/// The felicity gate: admit a lexical entry iff its category and semantics
/// agree. Checks `⟦cat⟧ ≡ sem_type` and that the entry's `sem` actually
/// inhabits `⟦cat⟧`. Returns the derived `⟦cat⟧` on admit; a reason on reject.
/// This is the trusted filter every drafted/imported entry must pass.
pub fn gate_entry(layer: &Arc<Layer>, entry: &Resource) -> Result<Exp, String> {
    let cat_v = entry
        .get(&iri("urn:eigenius:lexicon:cat"))
        .ok_or("entry has no `cat`")?;
    let st_v = entry
        .get(&iri("urn:eigenius:lexicon:sem_type"))
        .ok_or("entry has no `sem_type`")?;

    let cat = decode_type(cat_v, layer).map_err(|e| format!("cat decode: {e:?}"))?;
    let denoted = denote_cat(&cat)?;
    let sem_type = decode_type(st_v, layer).map_err(|e| format!("sem_type decode: {e:?}"))?;
    if !type_eq(&denoted, &sem_type) {
        return Err(format!(
            "⟦cat⟧ ≠ sem_type: ⟦cat⟧ = {denoted:?}, sem_type = {sem_type:?}"
        ));
    }

    // The sem must actually inhabit ⟦cat⟧. **Check-mode** (not `check_infer` +
    // exact `type_eq`): a lambda determiner sem checks against its `Pi` type, and a
    // (possibly multi-class) resource checks against its class via the full `is_a`
    // (#91) — neither of which `check_infer` can synthesize.
    let sem_v = entry
        .get(&iri("urn:eigenius:lexicon:sem"))
        .ok_or("entry has no `sem`")?;
    let sem = resolve_sem_value(layer, sem_v)?;
    let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(layer));
    let denoted_val = eval(&denoted, &Rho::Nil).map_err(|e| format!("⟦cat⟧ eval: {e}"))?;
    check(&mut ctx, &sem, &denoted_val).map_err(|e| format!("sem does not inhabit ⟦cat⟧: {e}"))?;
    Ok(denoted)
}

/// Build a parse item (category + resolved sem) from a committed lexical entry. The
/// leaf's **cost** is the entry's `lexicon:sense_rank` (D63 §8.7 Stage B) — a 0-based
/// WordNet sense-frequency rank (sense 1 → 0); absent ⇒ 0 (closed-class / demo
/// entries). The parser sums leaf costs, so a parse using more-frequent senses has a
/// lower cost and ranks higher.
pub fn entry_to_item(layer: &Arc<Layer>, entry: &Resource) -> Result<Item, String> {
    let cat_v = entry
        .get(&iri("urn:eigenius:lexicon:cat"))
        .ok_or("entry has no `cat`")?;
    let cat =
        strip_feature_binders(decode_type(cat_v, layer).map_err(|e| format!("cat decode: {e:?}"))?);
    let sem_v = entry
        .get(&iri("urn:eigenius:lexicon:sem"))
        .ok_or("entry has no `sem`")?;
    let sense_rank = entry
        .get(&iri("urn:eigenius:lexicon:sense_rank"))
        .and_then(Value::as_integer)
        .unwrap_or(0)
        .max(0) as u32;
    // `lexicon:scope_bearing` — the entry is a scope-bearing operator (sentential negation, a modal,
    // do-support), so its leaf carries [`Combinator::ScopeOperator`] and the combinator can tag the
    // operator's OUTPUT `Modal` without sniffing the category. Read HERE, exactly once, next to
    // `sense_rank`: the parse then never re-derives the property, and the firing decision downstream
    // reads only provenance. Absent ⇒ an ordinary leaf. See [`Combinator::ScopeOperator`] for why the
    // property is declared rather than inferred.
    let prov = if entry
        .get(&iri("urn:eigenius:lexicon:scope_bearing"))
        .and_then(|v| match v {
            Value::Boolean(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false)
    {
        super::item::Combinator::ScopeOperator
    } else {
        super::item::Combinator::Other
    };
    Ok(Item::from_parts(
        cat,
        resolve_sem_value(layer, sem_v)?,
        prov,
        super::item::Cost::from_sense_rank(sense_rank),
    ))
}

/// Peel `cat_fin_forall` / `cat_num_forall` binders off a leaf category (D63 §8.10),
/// leaving its feature variables FREE — so the parser's unifier binds them
/// (call-locally) from the consumed verb's real features and `subst_cat` propagates
/// them into the produced VP. Felicity gating reads the resource's full (binder-
/// wrapped) cat, where `⟦·⟧` erases the binder; only the parse item uses the
/// stripped form. The binder's `Exp::Lam` bound name appears as `Exp::Var` in the
/// body — which is exactly the free feature variable the parser then unifies.
fn strip_feature_binders(cat: Exp) -> Exp {
    if let Exp::InductiveCtor(_, name, args) = &cat {
        if (name.as_str() == "cat_fin_forall" || name.as_str() == "cat_num_forall")
            && args.len() == 1
        {
            if let Exp::Lam(_patt, body) = &args[0] {
                return strip_feature_binders((**body).clone());
            }
        }
    }
    cat
}

/// The **lexical lookup fence** (the parsing backbone's one dependency on the lexicon).
///
/// This is the ENTIRE surface the parser is allowed to ask of a lexicon: give me the entries for a
/// surface form, and tell me how far a lexical span may reach. It is a trait, and [`Parser`] holds it
/// as `Arc<dyn LexicalLookup>`, precisely so that parsing, ranking, seeding, and policy **cannot**
/// re-accrete onto the index — the compiler will not let the parser call anything else.
///
/// That fence is the lesson of how this module got here. `LexicalIndex` began as exactly what its name
/// says (see below) and grew a chart parser, a beam, a sense cap, an LLM reranker, a felicity gate, and
/// an anaphora resolver, because nothing structurally prevented it. A concrete type invites accretion;
/// a two-method trait refuses it.
pub trait LexicalLookup: Send + Sync {
    /// Every resolved entry for one exact, already-lowercased surface form.
    fn entries_for(&self, form_lc: &str) -> FormEntries;
    /// How far a lexical span may reach from a token — the multiword-seeding window, given a sentence
    /// of `n` tokens.
    fn span_limit(&self, n: usize) -> usize;
}

/// A `form → entries` lookup over a layer's committed `lexicon:LexicalEntry`
/// resources, each resolvable to a parse [`Item`] (category + sem). Built once per
/// layer; a [`Parser`] borrows it. Keys are **lowercased** forms (case-insensitive
/// lookup, the v1 choice; case-sensitive acronym disambiguation is a refinement).
///
/// This does lookup and **nothing else** — no parsing, no ranking, no policy. Those live on [`Parser`],
/// behind the [`LexicalLookup`] fence. (They did once live here; see that trait's note.)
///
/// Two backing modes (D65 §2.2, decisions D1/D2):
/// - **Lazy** — when a `core:ValueIndex` on `lexicon:form` is active at the layer
///   head (the production path: a shared-storage chain rooted at `bootstrap`). Form
///   lookups probe that exact index on demand and memoise per form, so `build` is
///   O(1) and a parse touches only the forms its sentence mentions — essential at
///   WordNet scale (325k entries), where the eager full-chain scan dominated.
/// - **Eager** — the fallback when no such index is active (e.g. an isolated-storage
///   chain, where [`scan_chain`](crate::layer)'s shared-index requirement keeps the
///   schema's declaration invisible from a child layer). Scans the whole chain once
///   into a materialised `form → items` map. This is the pre-D65 implementation,
///   retained verbatim as the no-index path; the two modes are behaviour-identical.
pub struct LexicalIndex {
    layer: Arc<Layer>,
    source: Source,
    /// **Document-augmentation overlay** (D63 lexicon-augmentation §6a): an in-memory `form → entries`
    /// map of a document's OOV groundings, consulted by [`Self::entries_for`] ALONGSIDE the persisted
    /// value-index probe. It lets a document's grounded aliases (`LexiconAugmentation`) be seeded WITHOUT
    /// committing them to the store — they are proposals, not committed lexicon
    /// ([`Self::with_document_augmentation`]). Each entry's cat/sem was resolved over the Arc chain
    /// (storage-independent), so the overlay works over a DB-backed head where the value-index probe
    /// cannot see uncommitted entries (§7-2). Empty by default (no behaviour change).
    overlay: BTreeMap<String, FormEntries>,
}

/// One resolved lexical entry — what [`LexicalLookup::entries_for`] returns: its parse [`Item`], its
/// `lexicon:in_lexicon`
/// membership (the scope filter, D65 §4), and its `lexicon:sense` label (for contextual reranking).
/// The sense rides *only* here — once a leaf enters the chart its [`Item`] carries no sense (a
/// composed item has none), so the sense never bloats the hot CKY structure.
#[derive(Clone)]
pub struct LexEntry {
    pub item: Item,
    pub in_lexicon: Option<Iri>,
    pub sense: Option<String>,
    /// The entry's `lexicon:form` in its **ORIGINAL CASE** — the index keys are lowercased, so this
    /// is the only place the source's capitalisation survives lookup.
    ///
    /// It exists for ACRONYM DISAMBIGUATION, the refinement this type's index documents as deferred
    /// from v1. A case-insensitive index is right for retrieval — `Cell lines…` sentence-initially
    /// must still reach the lemma `cell` — but it makes an all-caps nomenclature symbol reachable
    /// from the lowercase common noun it happens to spell. Measured over MRCONSO 2026AA: 4,319
    /// distinct all-caps English atoms lowercase onto a WordNet common-noun lemma, 24 of them on the
    /// WRN reference page. `CELL` (HGNC `NS` for the CELP pseudogene, C1413337, plus OMIM `ACR` for
    /// CEL, C1413336) is why `MSI cell lines` had 16 skeletons reading `cell` as a GENE.
    ///
    /// The drop set cannot fix this and should not: `drops.rs` keeps clean all-caps collisions
    /// DELIBERATELY (`CAT` the catalase gene, `SET` the oncogene are real symbols) and its
    /// `DROP_TTYS` excludes `ACR`/`NS`. The symbol is legitimate; reaching it from lowercase prose is
    /// not. So the fix belongs at the point of use, keyed on the OBSERVED token — see
    /// [`super::parse::all_caps_symbol`].
    pub form: String,
    /// The entry's own `core:description` — **the gloss the reranker reads**.
    ///
    /// It cannot come from the `sem`: a function word's `sem` is an inline λ-term, with no IRI and
    /// no resource to carry a description, so [`LexicalIndex::sem_gloss`] returns `None` and the
    /// prompt renders the entry as a BLANK LINE. Measured 2026-07-12 — the reranker was asked to
    /// choose between `""` and a full NCI definition, and eliminated the determiner `each` and the
    /// focus particle `alone`, exactly as one would expect. The description therefore lives on the
    /// ENTRY (`ontologies/lexicon/closed-class.esl`), where a grammatical reading can actually say
    /// what it means.
    pub gloss: Option<String>,
}

/// The resolved entries for one surface form (each a [`LexEntry`]) — the unit a scope
/// filter (D65 §4) consumes to keep + rank entries by lexicon, and the sense cap /
/// contextual reranker consume to keep entries by sense.
pub type FormEntries = Vec<LexEntry>;

/// The two backings behind [`LexicalIndex`] (lazy probe vs eager materialisation).
enum Source {
    /// Materialised `form → (item, in_lexicon)` — used when no form `ValueIndex`
    /// is active. The per-item `in_lexicon` (D65 §3) is the entry's `lexicon:Lexicon`
    /// membership, consumed at seed time for scope filtering + precedence ranking.
    Eager {
        by_form: BTreeMap<String, FormEntries>,
        /// Word count of the longest indexed form — the multi-span seeding window.
        max_words: usize,
    },
    /// On-demand probe of the active `lexicon:form` `ValueIndex`, memoised per form.
    Lazy {
        /// The `core:ValueIndex` Resource IRI its entries are keyed under.
        index_iri: Iri,
        /// The normalizer it declares — applied to a lookup key so it matches how
        /// the index was populated (D65: `lowercase`).
        normalizer: Iri,
        /// `normalized_form → resolved (item, in_lexicon)`. Presence = probed (an
        /// empty `Vec` records a probed miss, so a missing form is never re-probed).
        cache: Mutex<BTreeMap<String, FormEntries>>,
    },
}

/// The `lexicon:in_lexicon` membership of an entry resource (D65 §3), or `None`
/// for an untagged entry (always-available — e.g. the grammatical closed class).
fn read_in_lexicon(r: &crate::ontology::resource::Resource) -> Option<Iri> {
    r.get(&iri("urn:eigenius:lexicon:in_lexicon"))
        .and_then(|v| v.as_iri_str())
        .and_then(|s| Iri::parse(s).ok())
}

/// The `lexicon:sense` label of an entry resource — the sense key (e.g. `wn:bank.n.05`) the
/// contextual reranker reorders by. `None` for an entry that carries no sense (closed class).
fn read_sense(r: &crate::ontology::resource::Resource) -> Option<String> {
    match r.get(&iri("urn:eigenius:lexicon:sense")) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// The human-readable gloss of a chain entity (its `core:description`) — what the reranker reasons
/// over for a candidate sense. `None` if the entity has no description.
pub(super) fn read_description(r: &crate::ontology::resource::Resource) -> Option<String> {
    match r.get(&iri("urn:eigenius:core:description")) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Resolve a `lexicon:LexiconProfile` IRI to its ordered scope — the
/// `lexicon:lexica` array of `lexicon:Lexicon` IRIs, in declaration order =
/// resolution precedence (D65 §4.1). The result is ready to pass as the `scope`
/// to [`LexicalIndex::parse_scoped`]. Returns `None` if the IRI doesn't resolve or
/// carries no `lexica`. Resolved against `layer`'s chain so a profile committed
/// anywhere below the parse head is visible.
pub fn resolve_lexicon_profile(layer: &Layer, profile: &Iri) -> Option<Vec<Iri>> {
    let r = layer.resolve(profile)?;
    match r.get(&iri("urn:eigenius:lexicon:lexica"))? {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|v| v.as_iri_str().and_then(|s| Iri::parse(s).ok()))
                .collect(),
        ),
        v => v
            .as_iri_str()
            .and_then(|s| Iri::parse(s).ok())
            .map(|i| vec![i]),
    }
}

impl LexicalIndex {
    /// Build the lookup over `layer`. Prefers the **lazy** path — a declared, active
    /// `core:ValueIndex` on `lexicon:form` — and falls back to the **eager** full-
    /// chain scan when none is active. Entries whose `cat`/`sem` fail to resolve are
    /// skipped (the felicity gate caught them at import; a parse cannot use them).
    ///
    /// Builds a LEXICON, not a parser: no beam, no cap, no reranker, no chart. To parse, wrap it in a
    /// [`Parser`] (or use [`Parser::build`], which does both).
    pub fn build(layer: Arc<Layer>) -> Self {
        let form_prop = iri("urn:eigenius:lexicon:form");
        if let Some(active) = resolve_active_value_indexes(&layer)
            .into_iter()
            .find(|a| a.target_property == form_prop)
        {
            return LexicalIndex {
                layer,
                source: Source::Lazy {
                    index_iri: active.iri,
                    normalizer: active.normalizer,
                    cache: Mutex::new(BTreeMap::new()),
                },
                overlay: BTreeMap::new(),
            };
        }
        let (by_form, max_words) = Self::scan_eager(&layer);
        LexicalIndex {
            layer,
            source: Source::Eager { by_form, max_words },
            overlay: BTreeMap::new(),
        }
    }

    /// Overlay a document's [`LexiconAugmentation`](crate::dcg::LexiconAugmentation) (D63 §6a) — its
    /// grounded alias entries become seedable via an in-memory `form → entries` map consulted alongside
    /// the persisted index, so a DB-backed parse SEES the document's OOV groundings without those
    /// (proposal-grade) entries being committed to the store. Each alias's cat/sem is resolved over a
    /// throwaway doc chain (Arc parent = this index's head) — Arc-walk resolution is storage-independent,
    /// so a committed concept and a doc-local minted class (a grounding miss, carried in
    /// `LexiconAugmentation::supporting`) both resolve. Entries whose cat/sem don't resolve are skipped
    /// (fail-closed, as at import). Builder-style; default (unset) is the persisted index alone.
    pub fn with_document_augmentation(
        mut self,
        aug: &crate::dcg::augment::LexiconAugmentation,
    ) -> Self {
        use crate::layer::{LayerBuilder, LayerStorage};
        let form_prop = iri("urn:eigenius:lexicon:form");
        // Doc chain purely for RESOLUTION: the supporting resources (miss-minted classes) sit on this
        // index's head, so an alias's `sem` resolves whether the concept is committed (head) or doc-local.
        let mut b = LayerBuilder::new("doc-overlay", Some(Arc::clone(&self.layer)));
        for r in aug.supporting.iter().cloned() {
            let _ = b.add_resource(r);
        }
        let doc = Arc::new(b.build(LayerStorage::in_memory()));
        let mut overlay: BTreeMap<String, FormEntries> = BTreeMap::new();
        for binding in &aug.added {
            let entry = &binding.proposed;
            let Some(Value::String(form)) = entry.get(&form_prop) else {
                continue;
            };
            let key = form.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let Ok(item) = entry_to_item(&doc, entry) else {
                continue;
            };
            overlay.entry(key).or_default().push(LexEntry {
                item,
                in_lexicon: read_in_lexicon(entry),
                gloss: read_description(entry),
                sense: read_sense(entry),
                form: form.trim().to_string(),
            });
        }
        self.overlay = overlay;
        self
    }

    /// The pre-D65 eager scan: walk the chain (`iter_all_resources`, which follows
    /// parent `Arc` pointers — storage-sharing independent) and materialise
    /// `form → items`, tracking the longest form's word count for span seeding.
    fn scan_eager(layer: &Arc<Layer>) -> (BTreeMap<String, FormEntries>, usize) {
        let entry_class = iri("urn:eigenius:lexicon:LexicalEntry");
        let form_prop = iri("urn:eigenius:lexicon:form");
        let mut by_form: BTreeMap<String, FormEntries> = BTreeMap::new();
        let mut max_words = 1;
        for (_id, r) in layer.iter_all_resources() {
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            let Some(Value::String(form)) = r.get(&form_prop) else {
                continue;
            };
            let Ok(item) = entry_to_item(layer, r.as_ref()) else {
                continue;
            };
            let key = form.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            max_words = max_words.max(key.split_whitespace().count());
            by_form.entry(key).or_default().push(LexEntry {
                item,
                in_lexicon: read_in_lexicon(r.as_ref()),
                gloss: read_description(r.as_ref()),
                sense: read_sense(r.as_ref()),
                form: form.trim().to_string(),
            });
        }
        (by_form, max_words)
    }

    /// Probe the active value index for a normalized form key (lazy path).
    fn probe_form(&self, index_iri: &Iri, normalizer: &Iri, norm_key: &str) -> FormEntries {
        let entry_class = iri("urn:eigenius:lexicon:LexicalEntry");
        let form_prop = iri("urn:eigenius:lexicon:form");
        let mut seen: BTreeSet<Iri> = BTreeSet::new();
        let mut items = Vec::new();
        for hit in self.layer.storage().value_index.lookup(index_iri, norm_key) {
            let Ok((subject, _defining)) = hit else {
                continue;
            };
            if !seen.insert(subject.clone()) {
                continue; // a subject can be hit once per defining layer; resolve once
            }
            // Resolve the chain-nearest definition (None ⇒ out of this head's chain).
            let Some(r) = self.layer.resolve(&subject) else {
                continue;
            };
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            // Shadow safety: the resolved (nearest) definition's form must still
            // normalize to the queried key — a closer layer may have redefined it.
            let Some(Value::String(form)) = r.get(&form_prop) else {
                continue;
            };
            if normalize_value(normalizer, form) != norm_key {
                continue;
            }
            let Ok(item) = entry_to_item(&self.layer, r.as_ref()) else {
                continue;
            };
            items.push(LexEntry {
                item,
                in_lexicon: read_in_lexicon(r.as_ref()),
                gloss: read_description(r.as_ref()),
                sense: read_sense(r.as_ref()),
                form: form.trim().to_string(),
            });
        }
        items
    }

    /// Number of distinct indexed forms. **Eager**: the total materialised forms.
    /// **Lazy**: the forms probed into the cache so far (forms are discovered on
    /// demand, so the full count is not known without enumerating the value index).
    pub fn len(&self) -> usize {
        match &self.source {
            Source::Eager { by_form, .. } => by_form.len(),
            Source::Lazy { cache, .. } => cache.lock().expect("LexicalIndex cache poisoned").len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The fence, implemented over the committed lexicon (lazy value-index probe or eager chain scan).
impl LexicalLookup for LexicalIndex {
    /// Items for one exact, already-lowercased form key. **Eager**: a map lookup.
    /// **Lazy**: a memoised probe of the active `lexicon:form` `ValueIndex` —
    /// `value_index.lookup(index, normalize(form))` yields candidate `(subject,
    /// layer)` across the DAG; each distinct subject is resolved chain-nearest (via
    /// [`Layer::resolve`](crate::layer::Layer::resolve), which filters to the head's
    /// chain and shadow-resolves), re-checked to be a `LexicalEntry` whose form
    /// still normalizes to the key, then turned into an [`Item`].
    fn entries_for(&self, form_lc: &str) -> FormEntries {
        let mut out = match &self.source {
            Source::Eager { by_form, .. } => by_form.get(form_lc).cloned().unwrap_or_default(),
            Source::Lazy {
                index_iri,
                normalizer,
                cache,
            } => {
                let key = normalize_value(normalizer, form_lc);
                // Bind the cache hit to a local so the `MutexGuard` temporary drops HERE — before
                // `probe_form` and the re-`lock()` below. (Holding it across the `else`, as an `if let
                // Some(hit) = cache.lock()…get()` would, deadlocks on the re-lock — the guard lives to the
                // end of the `if let`.)
                let cached = cache
                    .lock()
                    .expect("LexicalIndex cache poisoned")
                    .get(&key)
                    .cloned();
                if let Some(hit) = cached {
                    hit
                } else {
                    let items = self.probe_form(index_iri, normalizer, &key);
                    cache
                        .lock()
                        .expect("LexicalIndex cache poisoned")
                        .insert(key, items.clone());
                    items
                }
            }
        };
        // Merge the document-augmentation overlay (§6a): a doc's OOV groundings, seeded alongside the
        // persisted entries so a DB-backed parse sees them without their being committed.
        if let Some(extra) = self.overlay.get(form_lc) {
            out.extend(extra.iter().cloned());
        }
        out
    }

    /// The multi-span seeding window: how far a lexical span may reach from token
    /// `i`. **Eager** knows the longest indexed form (`max_words`); **lazy** seeds
    /// every span up to the sentence length `n` (D65 §2.3 / D3 — no `max_words`
    /// stat; an over-long span is a cheap empty probe, memoised).
    fn span_limit(&self, n: usize) -> usize {
        match &self.source {
            Source::Eager { max_words, .. } => *max_words,
            Source::Lazy { .. } => n,
        }
    }
}

#[cfg(test)]
mod referential_definite_tests {
    use super::{LexicalIndex, LexicalLookup};
    use std::sync::Arc;

    /// The definite-referential fix (`experiments/parsing/near-encoded-bucket-analysis.md`,
    /// `2026-07-16`): the definite / demonstrative determiners denote a REFERENTIAL definite —
    /// `ontology:the`, the ι operator — NOT the existential CPS (`obj_exists_sem`/`exists_sem`)
    /// they once reused as a first-cut. A definite is scopeless, so `¬require(the(A), s)` is a
    /// single reading; the existential encoding spuriously bifurcated it into `¬∃x.P` / `∃x.¬P`
    /// under negation (the WRN-paper "did not require the exonuclease activity of WRN" dup).
    ///
    /// This pins the wiring **in CI, no snapshot** (the behavioural 2→1 collapse is the
    /// snapshot-gated `definite_negation_collapses_referential` in
    /// `crates/eigenius-wordnet/tests/db_backed_encoding.rs`). It catches a **reversion**
    /// (a definite re-pointed at the existential drops `ontology:the`) and an **over-correction**
    /// (a genuine existential made referential gains it).
    #[test]
    fn definites_reference_ontology_the_and_existentials_do_not() {
        const THE: &str = "Iri(\"urn:eigenius:ontology:the\")";
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let lex = LexicalIndex::build(Arc::clone(ctx.head()));
        let mentions_the = |form: &str| -> Vec<bool> {
            let es = lex.entries_for(form);
            assert!(!es.is_empty(), "no lexical entries for `{form}`");
            es.iter()
                .map(|e| format!("{:?}", e.item.sem()).contains(THE))
                .collect()
        };
        // Definites + demonstratives: their determiner reading is referential. `any` (not `all`)
        // because some forms are polysemous (`that` is also a complementizer, `this`/`these` also
        // pronouns) — those extra readings are legitimately not `ontology:the`.
        for d in ["the", "this", "that", "these", "those"] {
            assert!(
                mentions_the(d).iter().any(|&b| b),
                "`{d}` must have a referential-definite reading (references ontology:the)"
            );
        }
        // Genuine existentials + a cardinal: NEVER referential — their negation-scope split is real
        // and must be preserved.
        for q in ["a", "an", "some", "two"] {
            assert!(
                mentions_the(q).iter().all(|&b| !b),
                "`{q}` must stay quantificational (no ontology:the)"
            );
        }
    }
}

#[cfg(test)]
mod scope_bearing_tests {
    use super::{LexicalIndex, LexicalLookup};
    use crate::dcg::item::Combinator;
    use crate::dcg::rules::combinators::is_modal_functor;
    use std::sync::Arc;

    /// **Every entry with the auxiliary category shape must DECLARE `lexicon:scope_bearing`.**
    ///
    /// `combinators::build` tags a scope-bearing operator's output `Combinator::Modal` from the
    /// DECLARED property alone (this flag → `Combinator::ScopeOperator` on the leaf). It used to also
    /// INFER the property from the category via `is_modal_functor`; that inference was retired once
    /// this test showed the declaration covers it. `is_modal_functor` survives as a `#[cfg(test)]`
    /// predicate for exactly this check — it is now the COMPLETENESS OBLIGATION, not a fallback: an
    /// auxiliary added to `closed-class.esl` without the flag would silently lose its `Modal` tag, so
    /// it fails here instead.
    ///
    /// Negation is the part no category test can reach: `not` is `fwd(VP[bse], VP[bse])` /
    /// `fwd(VP[adj], VP[adj])`, and the latter is byte-identical to the adverb adjective-modifier
    /// category (`dcg::category::adverb_modifier_cats`). Declaring it is the point.
    ///
    /// CI-runnable, no snapshot (the behavioural check is `negation_scope_blocks_adjunct_escape` in
    /// `crates/eigenius-wordnet/tests/db_backed_encoding.rs`).
    #[test]
    fn scope_bearing_covers_the_modal_category_sniff() {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let lex = LexicalIndex::build(Arc::clone(ctx.head()));

        // (1) COMPLETENESS: every entry with the auxiliary category shape declares the flag. Since
        // `build` no longer infers from the category, a miss here means that entry has NO `Modal`
        // tag at all — a VP-adjunct could attach above it and escape its scope.
        for form in [
            "does", "do", "did", "can", "could", "may", "might", "must", "will", "would", "should",
            "not", "is", "are", "was", "were", "affects", "the", "in",
        ] {
            for e in lex.entries_for(form) {
                if is_modal_functor(e.item.cat()) {
                    assert_eq!(
                        e.item.prov(),
                        Combinator::ScopeOperator,
                        "`{form}` has an entry the category sniff catches but which does NOT \
                         declare lexicon:scope_bearing — retiring the sniff would drop its Modal tag"
                    );
                }
            }
        }

        // (2) NEGATION is declared, and is exactly what the sniff MISSES — all of its entries.
        // THREE since the `lexicon:pred` split: a predicate nominal is `S[dcl,pred]\NP`, not
        // `S[dcl,adj]\NP`, so "is not a drug target" needs its own negation entry alongside the
        // adjectival one. Every selector of a predicative complement is enumerated over {adj, pred}.
        let nots = lex.entries_for("not");
        assert_eq!(
            nots.len(),
            3,
            "expected the three `not` entries (verbal + adjectival + predicate-nominal)"
        );
        for e in &nots {
            assert_eq!(
                e.item.prov(),
                Combinator::ScopeOperator,
                "sentential negation must declare lexicon:scope_bearing — without it a VP-adjunct \
                 attaches ABOVE the negation and escapes its scope"
            );
            assert!(
                !is_modal_functor(e.item.cat()),
                "if the sniff started catching `not`, this test's premise is stale — the flag was \
                 introduced precisely because no category test can single negation out"
            );
        }

        // (3) NOT over-applied: an ordinary content word and a determiner stay untagged, so the
        // flag cannot silently spread to categories that legitimately host an adjunct above them.
        for form in ["affects", "the", "in"] {
            for e in lex.entries_for(form) {
                assert_ne!(
                    e.item.prov(),
                    Combinator::ScopeOperator,
                    "`{form}` must not be scope-bearing"
                );
            }
        }
    }
}
