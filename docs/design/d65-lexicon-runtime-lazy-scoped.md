# D65 — The lexicon runtime: lazy form-indexed lookup, per-parse scoping, and lexicon identity

*Status: design — **decision-complete** (§6 D1–D6 all ✅); ready to implement (§5 slices). Motivated by
D63 §8.7 (WordNet scale-up) and the domain-lexicon injection track ([[domain_lexicon_injection]],
D62 §8.7.8): the current `LexicalIndex` materialises the whole lexicon eagerly, which does not survive
injecting domain corpora (UMLS, NCBI) at their real scale, and there is no way to (a) load lexica on
demand, (b) select which lexica a given parse uses, or (c) name/expose the available lexica. Resolution:
a **lazy lexicon cache** over a new declared **`core:ValueIndex`** (exact value index on `lexicon:form`,
populated at layer-build like the text index — so lazy works uniformly, committed or not), **per-parse
scoping** by **`Lexicon`-as-Resource** (stable IRI) with entries bound via a `lexicon:in_lexicon` property.
No code yet — this settles the decisions before touching the parse hot path; the `ValueIndex` is a general
kernel/storage capability (§5 slice 0) and its own dependency.*

## 1. Problem

The DCG engine (D63) parses against a `LexicalIndex` built by [`LexicalIndex::build`]
(`kernel/src/dcg/lookup.rs`): it scans the **entire** layer chain (`iter_all_resources`) into a
`form → Vec<Item>` map up front, pre-resolving every `lexicon:LexicalEntry`. Three problems:

1. **It does not scale to injected domains.** WordNet alone is 325k entries (~14 s build, the warm-start
   cost flagged in D63 §8.7). UMLS is *millions* of terms; NCBI taxonomy is large. Eagerly materialising
   "everything ahead of time" to parse a 5-word sentence is infeasible once domain lexica are stacked on.
2. **No per-parse lexicon selection.** The index is the whole composed chain. A genomics sentence and a
   general-English sentence get the *same* lexicon — so domain senses pollute non-domain parses (more
   felicitous-but-irrelevant ambiguity, the D63 #93 theme) and there is no way to say "use WordNet + NCBI,
   not UMLS, for this parse."
3. **No stable identity for a lexicon.** A layer has a content-hash `LayerId([u8;32])` (version-specific,
   opaque) and a non-unique short `name`; branches/tags are mutable `name → LayerId` string pointers;
   EigenQL has no layer addressing. So there is no stable, queryable handle for "the WordNet lexicon" to
   expose available lexica or to name them in a parse scope.

These are one problem: the lexicon needs a **runtime** (load on demand — backed by a new exact value
index), a **scope** (pick lexica per parse), and an **identity** (name lexica stably) — addressed in
§§2–4 with the platform's existing declared-index and "everything is a Resource" patterns.

## 2. The lazy lexicon runtime

### 2.1 The lookup gap, and the primitive that fills it

There are four ways to find a resource in a committed layer, and **none** does exact lookup by a string
property value:

1. **By IRI** — `Layer::resolve` (resource backend + per-layer bloom). The primary key; fast.
2. **By IRI-valued property `(p,o)→s`** — `TripleIndex::scan_predicate_object` (Phase 14h) — but *only* for
   `resource`/`resource_array` predicates (`is_indexable_predicate`; `Triple` is all-IRI). `lexicon:form`
   is `core:string`, so it is **not in the triple index at all**.
3. **By literal property, exact** — *no index*: a free-subject pattern `MATCH ?e { form: "cell line" }`
   scans `Layer::all_resources()` and filters by `values_equal` — an O(n) chain scan (EigenQL guide §5.7).
4. **By string content, fuzzy** — the `~` operator over a declared `core:TextIndex` (BM25, tokenised,
   ranked). Wrong semantics for an *exact, multiword, case-folded* key.

So form→entries needs a new **exact value index**, and — per the platform's own pattern — it should be a
**declared, first-class index Resource**, exactly like `TextIndex`/`VectorIndex`. (The core ontology spells
out the rationale: an index is "a first-class Resource separate from the Property it targets so indexing
decisions don't modify the data-model identity … index lifecycle independent of Property lifecycle." A
`core:indexed` *flag on the property* would contradict this.)

**`core:ValueIndex`** (new — see §5/§dependency): declares that a target Property's values are
exact-indexed for `value → [(subject, layer)]` lookup.
- targets a property via the existing shared **`core:target_property`** slot (as Text/Vector do);
- carries an optional **`core:value_normalizer`** (the exact-index analogue of `text_analyzer`, but it
  *normalizes, never tokenizes*): `identity` / `lowercase` / `lowercase-trim`. For `lexicon:form` →
  `lowercase`, so case-insensitivity is **declarative** — no shadow `form_norm` property needed;
- discovered at load via `resolve_active_value_indexes(head)` (mirrors `resolve_active_text_indexes`,
  `kernel/src/layer/index_discovery.rs:154`), with the same one-active-per-property-per-head multiplicity rule;
- **pre-populated at `LayerBuilder::build`** — exactly as the existing triple and text indexes already are
  (`kernel/src/layer/mod.rs:1016` and `:1044`), so reads against a freshly-built, not-yet-persisted layer
  work identically to reads after restart. (The persistent backend re-writes the same entries at
  `store_layer`, idempotently.) See §2.3.

The lexicon schema declares one `ValueIndex` on `lexicon:form` (normalizer `lowercase`); the lazy lookup is
`value_index.lookup(normalize(surface)) → [(entry IRI, layer)]`. Scoping is then a filter on each entry's
`lexicon:in_lexicon` (§3.1, §4). Bonus: any exact string/value lookup (gene symbols, accessions, codes)
gets the same path, and EigenQL `MATCH { prop: "literal" }` on a value-indexed property can use it instead
of the full scan — exactly parallel to the triple index for IRI-property patterns.

### 2.2 `LexicalIndex` becomes a cache, not a materialisation

`LexicalIndex` holds the layer + a `form → Vec<Item>` cache filled **on demand**: when `parse` needs a
span's surface, it queries the active `ValueIndex` on `lexicon:form`, resolves the returned entries to
`Item`s (reusing `entry_to_item`), and caches them. A 5-word sentence touches ~5–15 forms, not the whole
lexicon.

**Immutable layers make the cache trivially correct.** A `Layer` is immutable (`Arc<Layer>`, parent
pointers), so a cache keyed by `(layer-head-id, form)` never needs invalidation within a layer version —
the "in-memory cache" model fits the data model exactly.

### 2.3 Remaining gaps

1. **Lazy vs eager boundary — simplified by build-time population.** Because a `ValueIndex` is pre-populated
   at `LayerBuilder::build` (as the triple and text indexes already are, `mod.rs:1016`/`:1044`), the form
   index is present for **both committed and uncommitted in-memory** layers — so the lazy path works
   uniformly (the importer's in-memory validate, tests, the harness all get it). The eager scan survives
   only as the degenerate fallback for a layer with **no active `ValueIndex` on `lexicon:form`** (legacy /
   a lexicon that didn't declare one), not as an in-memory carve-out. (This is the D1 simplification — see
   §6.)
2. **The multiword window (`max_words`) — dissolved, no stat needed.** Today `build` derives `max_words`
   (the longest multiword form) by scanning all entries, to cap the MWE seeding window. But that cap is a
   pure micro-optimization, not a correctness need: the seeding loop is already `.min(n)`, and **trying
   every span up to the sentence length `n` gives identical results** — an over-long span just *misses*
   (a span matches only if an entry exists for that exact normalized string; no spurious matches). In the
   lazy model each over-long span is one cheap empty `ValueIndex` probe (~µs), so for sentence-scale `n`
   the saving is sub-millisecond and not worth a precomputed stat. **Decision:** drop `max_words` as a
   lexicon stat — seed all spans `tokens[i..=j]`, `i ≤ j < n`. (An optional *generous fixed* safety cap
   may bound work on adversarially long inputs, but never the exact lexicon longest-form, which would
   silently drop longer MWEs.)

## 3. Lexicon identity and EigenQL exposure

The open question (raised in review): give **layers** stable IRIs, or organise differently?

A `Layer` is a *generic versioning/commit unit*, not specifically a lexicon — putting a lexicon-flavoured
IRI on it conflates two concepts and means surgery on the core layer-identity machinery. The better fit,
consistent with "everything is a Resource" and EigenQL-native, is to model a **lexicon as a first-class
Resource**:

- **`lexicon:Lexicon`** instances with **stable IRIs** — `urn:eigenius:lexicon:wordnet`,
  `…:umls`, `…:ncbi` — carrying metadata: `source`, `version`, `language`, `domain`, `license`, and a
  binding to its content (see below). The *logical* identity ("the WordNet lexicon") lives on the resource;
  the *physical* version lives in the layer it binds — decoupled, so re-importing WordNet doesn't change its
  IRI.
- **Available lexica** = an ordinary EigenQL query over `Lexicon` instances — no new query machinery, no
  layer addressing in the grammar.
- **A parse scope** is then a set of `Lexicon` IRIs (§4), which the runtime resolves to content layers.

### 3.1 How a `Lexicon` binds to its entries — DECIDED: entry property (B2)

**Each `LexicalEntry` carries `lexicon:in_lexicon = <Lexicon IRI>`** (an IRI-valued property), and the
parse scope filters on it. The `Lexicon` resource holds the metadata; membership is the inverse of this
property. Chosen over the layer-aligned alternative *because* of the two decisions above:

- **It binds to the stable Lexicon IRI we are already minting**, not to a layer. The layer-aligned option
  (B1: `Lexicon` → a content layer) would have to reference the layer by its version-specific `LayerId`
  (we decided **not** to give layers stable IRIs) — which churns on every re-import, hits a self-reference
  snag (a `Lexicon` committed in its own layer can't name that layer's hash before it is built), and forces
  *one lexicon = one layer*. B2 has none of that.
- **It's flexible:** a layer may hold several lexica; a lexicon may span layers / accrue across re-imports.
- **Cost is modest:** one extra triple per entry, the object a single interned `Lexicon` IRI (small
  triple-index footprint) — ~15–20% on top of each entry's existing `form`/`cat`/`sem`/`sem_type`/`grade`.
  The importer emits it for free (it knows which lexicon it is building).
- **Scope filter is free:** entries are resolved to `Item`s anyway, so `in_lexicon` is read at resolve time
  — keep those with `in_lexicon ∈ scope`. Enumerating a lexicon (stats / "what's in UMLS") is a direct
  `scan_predicate_object(lexicon:in_lexicon, <LexiconIRI>)`.

(So the answer to "stable IRIs for layers?" is: don't IRI the layer — a generic commit unit; IRI a
`Lexicon` Resource and point each entry at it via `lexicon:in_lexicon`. Importers already namespace entries
(`wn:`, `umls:`, …), so membership is *also* derivable from the IRI prefix — a corroborating signal, but
`in_lexicon` is the explicit, queryable source of truth.)

## 4. Per-parse scoping

`parse` gains an optional **lexicon scope**: an **ordered list** of `Lexicon` IRIs, or a **named profile**
(a `lexicon:LexiconProfile` Resource — §4.1). The lazy lookup keeps only entries whose `lexicon:in_lexicon`
is in the scope set (read free at resolve time, §3.1).

- **Default** (no scope) = the whole composed chain, unordered (today's behaviour) — backward compatible.
- **Composition vs selection.** The layer chain still provides *composition* (stack WordNet + domain
  layers); the scope adds *per-parse selection* over what's composed.
- **Ambiguity control.** Scoping out irrelevant domains directly shrinks the felicitous-but-irrelevant
  forest (the D63 #93 / selectional-restriction theme) — a non-medical sentence need not carry UMLS senses.
- The scope is itself a candidate for a `ReasoningSentence`/institution input: an encoding institution can
  *choose* the scope for a document (e.g. from its domain tags) rather than hard-coding it.

### 4.1 `LexiconProfile` and order = resolution precedence

A **`lexicon:LexiconProfile`** is a minimal Resource — one ordered `resource_array` of `Lexicon`
references — selectable as a scope:

```
class lexicon:LexiconProfile { requires lexicon:lexica; }
property lexicon:lexica : core:resource_array { class_types lexicon:Lexicon; domain lexicon:LexiconProfile; }

resource lexicon:profile_biomedical : lexicon:LexiconProfile {
    lexicon:lexica = [ lexicon:ncbi, lexicon:umls, lexicon:wordnet ];   -- order = precedence
}
```

`class_types lexicon:Lexicon` gives referential integrity (no dangling/non-Lexicon members), and
`resource_array` is triple-indexable so "which profiles include WordNet?" is a free
`scan_predicate_object(lexicon:lexica, <wordnet>)`.

**The array order is the order of resolution — soft precedence, not shadowing.** When a form appears in
several scoped lexica ("cell" in a domain lexicon *and* WordNet), all readings stay in the forest, but
entries from earlier-listed lexica **rank first**; the D63 §8.7 cap drops the low-precedence tail. (Hard
shadowing — suppressing later lexica's entries for a form an earlier one defines — is rejected: it drops
valid parses and breaks the §6 forest-returns boundary.)

### 4.2 Precedence folds into the rank+cap as the primary sort key

The D63 §8.7 Stage-B rank already orders the forest by an additive `Item.cost` (the summed sense-frequency
ranks). Lexicon precedence becomes the **primary** component of that cost, sense-frequency the secondary —
so `Item.cost` goes from a scalar to a **2-component `(lexicon_order, sense_rank)`**:

- each **leaf**'s `lexicon_order` = the index of its entry's `in_lexicon` in the scope's ordered list
  (0 = first/most-preferred; 0 for the unordered default);
- the combinators **sum** both components (as they already sum `sense_rank`) — so a parse's
  `lexicon_order` is **the sum of its leaves' positions** (a parse using *more* preferred-lexicon entries
  ranks higher; this is the confirmed **sum** aggregation, granular, matching the additive model — vs a
  coarser per-parse `max`);
- the forest sorts **lexicographically by `(Σ lexicon_order, Σ sense_rank)`** then caps. The tuple makes
  precedence dominate cleanly with no magic offset constants; sense-frequency tie-breaks within a
  precedence level.

So "priority lives in the rank" still holds — the **scope supplies the lexicon-precedence input to the
rank**, which now has two inputs (scope order + sense frequency). The ordering only bites on forms present
in *multiple* scoped lexica; single-lexicon forms carry a fixed component either way.

## 5. Implementation plan (slices)

0. **`core:ValueIndex` — a general exact-value index (kernel/storage capability; its own sub-design).**
   This is broader than the lexicon and parallels D43's introduction of Text/Vector indices, so it is an
   explicit **dependency**, sequenced first:
   - `core:ValueIndex` class + `core:value_normalizer` slot in the core ontology (reusing `core:target_property`);
   - a `ValueIndex` storage trait (`lookup(normalized_key) → [(subject, layer)]` + `extend_layer`/
     `drop_layer`) with **memory** and **rocksdb** impls — mirroring `TripleIndex`;
   - **build-time population** (a `populate_value_indexes` at `LayerBuilder::build`, like
     `populate_text_indexes`) + GC drop-on-sweep;
   - `resolve_active_value_indexes(head)` discovery + the one-active-per-property-per-head multiplicity rule;
   - *(optional, later)* teach the EigenQL planner to use it for exact literal-property patterns — a free
     win, not required for the lexicon runtime.
1. **Declare the lexicon `ValueIndex`** — a `core:ValueIndex` on `lexicon:form` (normalizer `lowercase`) in
   the lexicon schema, so it is active for every lexicon layer.
2. **Lazy `LexicalIndex`** — cache over the active `ValueIndex`; seed all spans up to the sentence length
   (no `max_words` stat, §2.3); fall back to the eager scan only when no `ValueIndex` is active on
   `lexicon:form`. Behaviour-preserving for existing tests (default scope).
3. **`lexicon:Lexicon` resource + `lexicon:in_lexicon`** (§3) — schema for the `Lexicon` descriptor and the
   `in_lexicon` entry property; the importer emits one `Lexicon` and tags every entry with it; "available
   lexica" is an EigenQL query over `Lexicon` instances.
4. **Parse scope + ordered precedence** (§4) — the scope parameter (ordered `Lexicon` IRIs or a
   `LexiconProfile`); filter resolved entries by `in_lexicon ∈ scope`; add `lexicon:LexiconProfile` +
   `lexicon:lexica` to the schema; and extend the D63 §8.7 rank `Item.cost` from a scalar to the 2-component
   `(lexicon_order, sense_rank)` summed by the combinators, with the forest sorted lexicographically (§4.2).
   `lexicon_order = 0` for the unordered default ⇒ behaviour-preserving for existing single-lexicon parses.
5. **Domain injectors consume it** — UMLS/NCBI importers commit their layers, each emitting its `Lexicon` +
   tagging entries; scoped parses select them. (This slice is the *reason* for the rest — see
   [[domain_lexicon_injection]].)

## 6. Decisions

- **D2 — form lookup index. ✅ DECIDED:** a declared **`core:ValueIndex`** (exact value index) on
  `lexicon:form`, with a `value_normalizer = lowercase` for case-insensitivity — the same first-class
  declared-index pattern as Text/Vector, *not* a `core:indexed` flag on the property (which the core
  ontology's own rationale rejects) and *not* a bespoke lexicon-only index. New kernel/storage capability;
  its own sub-design (§5 slice 0).
- **D1 — lazy vs eager boundary. ✅ DECIDED (simplified by D2):** because a `ValueIndex` populates at
  `LayerBuilder::build`, lazy works **uniformly** (committed *and* uncommitted in-memory); the eager scan
  survives only as the fallback when **no `ValueIndex` is active** on `lexicon:form` — not as a
  committed-vs-uncommitted carve-out.
- **D4 — lexicon identity. ✅ DECIDED:** `Lexicon`-as-Resource (stable IRI + metadata). *Not* stable layer
  IRIs (a layer is a generic commit unit) and *not* bare branches/tags.
- **D5 — Lexicon↔entries binding. ✅ DECIDED:** entry property `lexicon:in_lexicon = <Lexicon IRI>` (B2) —
  binds to the stable Lexicon IRI rather than a version-specific layer; see §3.1.
- **D3 — `max_words`. ✅ DISSOLVED:** not needed — seed all spans up to the sentence length `n`; an
  over-long span just misses (a cheap empty probe). No per-lexicon stat. (Optional generous safety cap for
  adversarial input only; never the exact lexicon longest-form.)
- **D6 — scope surface. ✅ DECIDED:** an **ordered** list of `Lexicon` IRIs *or* a `lexicon:LexiconProfile`
  (a Resource with one ordered `lexica` array); default (absent) = whole chain, unordered. **Array order =
  resolution precedence (soft)** — earlier lexica rank first, later stay in the forest (no shadowing).
  Realized by extending the D63 §8.7 rank cost to a 2-component **`(Σ lexicon_order, Σ sense_rank)`** sort key,
  summed by the combinators (**sum** aggregation across a parse's leaves). See §4.1–§4.2.

## 7. Out of scope / future

- Sense-frequency rank + cap (D63 §8.7 Stage B, built) — orthogonal; scoping reduces the *input* to the
  forest, ranking orders the *output*.
- Selectional restrictions via verb argument types (issue #93) — a different ambiguity lever.
- A persisted/serialised parse index (skip the per-load rebuild entirely) — the lazy cache already removes
  the *full* rebuild; a serialised warm index is a further optimisation, not required here.
- Cross-lexicon entry de-duplication / alignment (the same lemma in WordNet and a domain lexicon) — a
  grounding/alignment concern, deferred.

## 8. Source anchors (verified against the tree)

Each design element mapped to the code it touches or builds on. Verified at authoring; treat line numbers
as approximate (they drift).

| Design element (§) | Source — read / extend | Status |
|---|---|---|
| Eager `LexicalIndex` (whole-chain scan) → make lazy (§2.2) | `kernel/src/dcg/lookup.rs:95` (`build`), `:100` (`iter_all_resources`) | read → replace |
| MWE seeding window; `max_words` dissolves (§2.3) | `kernel/src/dcg/lookup.rs:203` (`(i + max_words).min(n)`), `:87`/`:99`/`:114` (the field) | read → drop field, seed to `n` |
| Leaf `Item` build + `cost` → 2-component (§4.2) | `kernel/src/dcg/parser.rs:64` (`Item`), `:68` (`cost: u32`) | read → widen to `(u32,u32)`; combinators sum both |
| Leaf cost source (`sense_rank` + `lexicon_order`) (§4.2) | `kernel/src/dcg/lexicon.rs:124` (`entry_to_item`) | read → add `lexicon_order` from scope |
| Lazy form lookup primitive (§2.1) | `kernel/src/layer/index.rs:101` (`scan_predicate_object` trait), `:330` (mem impl) | pattern to mirror for `ValueIndex` |
| Why `lexicon:form` isn't triple-indexable (§2.1) | `index.rs:46` (`Triple` all-IRI), `:123` (`is_indexable_predicate` = resource/array only) | read (motivates `ValueIndex`) |
| Build-time index pre-population (§2.1, §2.3) | `kernel/src/layer/mod.rs:1016`–`1038` (triple), `:1044` (text); `index.rs:180` (`extract_indexable_triples`); `query/text/indexing.rs:72` (`populate_text_indexes`) | mirror with `populate_value_indexes` |
| Declared-index discovery to mirror (§2.1) | `kernel/src/layer/index_discovery.rs:154`/`:187` (`resolve_active_{text,vector}_indexes`), `:51` (`ActiveTextIndex`) | add `resolve_active_value_indexes` |
| Declared-index schema pattern (§3, §5.0) | `ontologies/core/core-ontology.json:862` (`TextIndex`), `:892` (`index_target`), `:899` (`text_analyzer`) | add `core:ValueIndex` + `value_normalizer` |
| `lexicon:form` (string) + `LexicalEntry` (§2.1) | `ontologies/lexicon/lexicon-ontology.esl:211` / `:205` | read; declare a `ValueIndex` on it |
| `LayerBuilder::build` (population hook) (§5.0) | `kernel/src/layer/mod.rs:950` | extend with `populate_value_indexes` |

**Discrepancy flagged + corrected.** An earlier draft (and the original D1 reasoning) claimed the triple
index is populated *only at commit* (`store_layer`), making lazy work for committed layers but not
uncommitted in-memory builds. **That is false:** `LayerBuilder::build` pre-populates *both* the triple
index (`mod.rs:1016`) and the text index (`:1044`) up front (the persistent backend re-writes the same
entries idempotently at `store_layer`). The mis-claim came from a storage-module doc comment describing the
*persistent write*, not the in-memory pre-population. Corrected in §2.1/§2.3 — the D1 conclusion (lazy works
**uniformly**, eager only as the "no `ValueIndex` declared" fallback) is unchanged and is in fact *better*
supported: the `ValueIndex` simply mirrors the build-time pre-population the other two indexes already do.

No other discrepancies found: `scan_predicate_object` returning `(subject, defining_layer)`, the IRI-only
indexability rule, the declared-index pattern (separate Resource via `index_target`), and the build-time
text-index population all match the doc as written.
