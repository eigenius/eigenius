# D63 — Compound & derived words (hyphen / concatenation) — implementation note

**Status: COMPLETED** (`2026-07-05`). Closed the derived-adjective OOV from the parse-gap triage
([d63-parse-gap-closure.md](d63-parse-gap-closure.md) §2): `pcr-based`, `double-stranded` (hyphenated),
`hypermutable` (concatenated) — parsed morphologically **without ballooning the lexicon**, purely symbolic
and felicity-gated. Shipped: Slice 1 (prefix + hyphen-head identity), Slice 2 (`X-based`), and §3b (the
full denominal-suffix table `-based`/`-like`/`-mediated`/… + the `-like` over-generation fix), all
witnessed by tests over the snapshot (missing-lexeme 6 → 2). **Everything deferred was extracted to its own
track:** the phrasal `E link X` convergence → [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md);
the passive-voice machinery it needs → [d63-passive-voice-handling.md](d63-passive-voice-handling.md);
`recq` (a gene-*family* name — HGNC gene group 1049, no base) → the gene-family track ([[gene_family_lexicon_gap]]).
Nothing derivational remains here.

## 1. Approach — analyze against the lexicon, then synthesize

A recognizer strips the affix / splits at the hyphen, probes the base against the lexicon, and — if the
base is known — **seeds the derived word directly as a typed `ADJ` `Item`** (§3, "synthesize-on-token").
The affix's functor *category* (`-based : ADJ\N`, `hyper- : ADJ/ADJ`) characterizes the result, but we do
**not** seed the affix as a separate lexeme and let CKY compose it — that split-and-compose variant is the
deferred alternative (§3). This is the analyze-against-the-lexicon family (finite-state / two-level
morphology; Koskenniemi 1983, Beesley & Karttunen 2003), and it generalizes the mechanism we already ship
for `-ly` adverbs:

- **[`is_derived_adverb`](../../kernel/src/dcg/lookup.rs) / [`adverb_items`](../../kernel/src/dcg/lookup.rs)**
  (`lookup.rs:900–937`): a recognition gate (strip the affix via [`adverb_bases`](../../kernel/src/dcg/lookup.rs)
  `:2897`, probe the base's cat) shared with [`has_token`](../../kernel/src/dcg/lookup.rs) so a derived
  form counts as **known** (not routed to missing-lexeme), plus a seeder that produces `Item`s (cat + sem).

The derived adjectives then modify nouns through the **existing refine rules** —
[`RefineKind::Attrib`](../../kernel/src/dcg/parser.rs) (`parser.rs:265`, attributive adjective `S[adj]\NP`
+ `cat_n` → `Σx:C. adj(x)`) and [`RefineKind::PpMod`](../../kernel/src/dcg/parser.rs) (`:272`, `cat_pp`
noun-modifier) — so **no new composition machinery**, only affix lexemes + a recognizer.

## 2. The three patterns (cats + sems)

`ADJ = bwd(cat_s(dcl, adj), cat_np(Entity, num_any))` (predicative adjective, `S[adj]\NP`; as in the demo
`primary`/`large`).

| pattern | affix category | sem | reuses |
|---|---|---|---|
| **`X-based`** (suffix) | `-based : ADJ\N` = `bwd(ADJ, cat_np(Entity))` | `λn. λx. base(x, n)` — the WordNet **`base` verb axiom** (§2a), a predicate over the head, like the `cat_pp` noun-mod sem `λy.λx. prep_P(x,y)` | `Attrib`/`PpMod` refine |
| **`hyper-X`** (prefix) | `hyper- : ADJ/ADJ` — the **existing** adjective-modifier cat `(S[adj]\NP)/(S[adj]\NP)` (`adverb_modifier_cats`, `category.rs:368`) | v1: **identity** `λp. p` (like the `-ly` adverbs — degree deferred); v2: a degree operator | adj-mod cat + `Attrib` (noun-mod) |
| **`A-stranded`** (hyphen compound-adj) | head is the **known** `stranded : ADJ`; `double- : ADJ/ADJ` = the same **existing** adjective-modifier cat | `λp. p` (v1) or a compound predicate | adj-mod cat + `Attrib` |

**Machinery check (`2026-07-04`):** the adjective-modifier cat `(S[adj]\NP)/(S[adj]\NP)` — the compose step
for `hyper-X` / `A-stranded` — already exists (`category.rs:368`) and is proven: `WRN was **selectively
essential**…` parsed in the baseline via it (adverb premodifying `essential`). So those two patterns reuse
existing composition; only the *seeding* of the affix/modifier at that cat is new. `X-based`'s `-based :
ADJ\N` is the one genuinely new affix category.

Two constraints from the discussion:
- **Typed sem / felicity gate = the semantic over-generation guard.** Each synthesized sem must
  type-check at `Prop` and reference a **real relation** — for `X-based`, **reuse the WordNet `base` verb
  axiom** (§2a), not a freshly-minted `based_on`; a degree `hyper` operator (v2) would need declaring. The
  gate rejects an ill-typed synthesis, so the ontology bounds what the rule can produce. (Dependency:
  `X-based` needs the base `X` grounded — `pcr` may itself be an abbreviation, pulling in the same glossary
  path as `MSI`.)
- **Closed affix inventory, not frequency splitting.** For biomedical text the productive prefixes are a
  small closed set `{hyper, hypo, poly, multi, mono}`; a declarative FST-style list is more precise and
  cheaper than a corpus-frequency splitter (Koehn & Knight 2003 — built for open German compounds, and it
  injects statistical noise into an otherwise symbolic, gate-checked parser). Use the list.

## 2a. `X-based` ≡ `based on X` — reuse the verb axiom, don't mint a relation

The compound and the phrasal are the same proposition and must share a representation:

- `pcr-based method` → `Σx:method. base(x, PCR)`
- `method based on PCR` → `Σx:method. base(x, PCR)`

`based on X` is the passive of *base X on Y* — the participle `based` taking an `on`-PP **argument**, which
is exactly the verb+PP machinery already built: `based : (S[adj]\NP)/cat_pp_arg`, `on : cat_pp_arg/NP`,
`based on X : S[adj]\NP`, sem `λx. base(x, X)`. The relation is the WordNet **`base` verb's own axiom**
(`base.v.01`, "use as a basis; found on") — **not** a declared `based_on`.

**The convergence has two independent halves — one per surface form. They are built by different work:**
- **Compound `X-based` → `base(x, X)`:** the affix rule with sem `λn.λx. base(x, n)` over the `base` axiom.
  **Built by the morphology impl (§3, Slice 2). Needs no object+PP work.** (The "declare `based_on`"
  prerequisite dissolves — reuse the verb.)
- **Phrasal `based on X` → `base(x, X)`:** built by the **object+PP frame extension (Step 2b)** in the
  closure plan. Today the phrasal *parses* but with a different sem (adjective + adjunct — see the caveat).

So the object+PP extension resolves **only the phrasal half**; the compound half is Slice 2. General shape
for participial `-Ved` denominal adjectives (`PCR-based`, `cell-mediated`, `receptor-bound`) **and the
adjectival `-like`/`-dependent`/`-specific` class**: the affix maps to the axiom of the verb/adjective it
derives from, and compound `X-E` must align with phrasal `E link X`. That generalization — the shared
`DenominalElement` table + the `⟦X-E⟧ = ⟦E link X⟧` invariant, the `-like` over-generation fix, and the
per-file code touchpoints — is [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md).
- **Caveat (checked + corrected twice, `2026-07-04`).** `base` has two relevant verb senses: **"situate"**
  (WN3.0 02756196, frames 8–11 transitive) and **"found on"** (WN3.0 00636888 = OEWN 00638550, *"base a
  claim ON an observation"*), which **does** carry an object+PP frame — WN3.0 **frame 21** ("Somebody
  ----s somebody PP"), OEWN "----s something PP" (`vtai-pp`). So WordNet *does* encode `base X on Y`. But
  our importer routes **frame 21 → Transitive** (an object+PP frame handled coarsely — the PP dropped), so
  the argument reading is **not produced**. What `based on X` produces today (witnessed over
  `wordnet-umls-2026-07-04`, `db_backed_encoding::show_based_on_x_reading`, `2026-07-05`) is a
  **noun/adjective/verb pile-up**, and in **every** reading `on X` is a separate `prep_on(x, X)`
  **adjunct**, never the argument:
  - `Cells are based on genes` → **×8, all the NOUN reading** `subclass_of(Cell, Σx:Basis.
    prep_on(x, kind_of(Gene)))` (`based` → the noun *basis*, C1527178);
  - `The method is based on sequencing` → **×176**, `based` resolving as a gradable **adjective**
    (`gt(deg_based(x), std_based) ∧ prep_on(x, …)`), as the **verb** with its object slot inert
    (`(Πg. base_v(x, g)) ∧ prep_on(x, …)`, `base_v` = `v00636888_t`), and as a noun-compound.

  (An earlier draft here said the adjective reading is *the* current reading and that `Cells are based on
  genes` shows it — wrong on both: it is one of several, and that example actually gives the noun reading.)
  **Consequence:** the convergence is the *desired* representation and needs the **object+PP frame
  extension** (Step 2b, §3) so both `-based` and `based on X` resolve through the `base`-verb axiom.

**Faithful target (witnessed `2026-07-05`) — what both surface forms should land on:**

| sentence | faithful encoding |
|---|---|
| `Cells are based on genes.` | `base_on(kind_of(Cell), kind_of(Gene))` |
| `The method is based on sequencing.` | `base_on(m, kind_of(Sequencing))`, `m : Method` (definite referent) |

- **Relation:** `base_on` = the **stative** "found on / rests on" projection of `base.v.01` (WN 00636888) —
  grounding to the verb, still **not** a free-minted relation.
- **Arity = 2-place `base_on(theme, ground)`** (theme = subject, ground = the on-object). The active
  `base.v.01` is 3-role `base(agent, theme, ground)`; the stative passive **drops the agent** (it asserts a
  foundational relation, not an event), so `∃agent. base(agent, X, Y)` is *less* faithful.
- **Role correction to Slice 2's shorthand `base(x, X)`:** the coarse 2-place axiom is
  `v00636888_t(theme, **agent**)` — its second slot is the *agent*, so Slice 2 puts the ground in the agent
  slot (pragmatic, role-imprecise). Faithful Step 2b emits `base_on(theme, ground)` with the right roles.
- **Subject term by determiner:** definite `the method` → a specific **referent** `m : Method` (D64 hole),
  *not* a kind; bare `Cells`/`genes`/`sequencing` → `kind_of(·)`.
- **Sense caveat:** the parse mis-resolves `sequencing` → C0004793 *"Base Sequence"* (the DNA base
  sequence); the faithful `ground` is the sequencing *technique* — a grounding/sense-selection fix,
  orthogonal to the relation.
- *(History: I first said `based on X` gaps — wrong; then that WordNet lacks the subcat — wrong (frame 21);
  then that today's reading is the adjective one — wrong (noun/adjective/verb pile-up). Each corrected by
  witnessing. Lesson: parse it before characterizing it.)*

## 3. Implementation plan

**Parse-time, no reseed.** Like the `-ly` handling, this lives in `lookup.rs` (the seeder), not the
importer — so it works over the existing `wordnet-umls-2026-07-04` snapshot immediately. The bases
(`mutable`, `stranded`, `based`) are already in the lexicon.

**Style: synthesize-on-token** (the `-ly` mechanism, generalized). The recognizer produces **one derived
`ADJ` `Item` per token** — it does *not* split the token or seed affix-functors. New code:
`adjective_bases` / `is_derived_adjective` / `derived_adjective_items` (mirroring the `adverb_*` trio),
wired into seeding (`lookup.rs:~1827`) and `has_token`; plus one affix category `-based : ADJ\N`. Reused:
the adjective-modifier cat `(S[adj]\NP)/(S[adj]\NP)`, `RefineKind::Attrib`, the `base` verb axiom.

### Scope — all four OOV ship here
- **Slice 1 — `hyper-X` + `A-stranded`** (identity sem): recognize (prefix from `{hyper,hypo,poly,multi,mono}`
  or hyphen-split), probe the head/base is `ADJ`, **seed the base/head's own `Item`s on the whole-token
  span** — so `hypermutable ≡ mutable`, `double-stranded ≡ stranded`. Test: demo fixture, then snapshot
  units 15 & 21 → parse.
- **Slice 2 — `X-based`**: recognize (hyphen-split, suffix `based`, `X` a known `N`), seed an `ADJ` with sem
  `λx. base(x, X)` via the `base` verb axiom. Dependency: `X` grounded — `pcr` is an abbreviation, so
  `pcr-based` needs the glossary; test via the glossary path or a `pcr : N` fixture. Snapshot units 45, 49.
- **`recq`** — a named-entity entry (gene-*family* name, no base to decompose); separate from the
  morphology (anchor + the general gene-family gap in §Source touchpoints).

**Gate:** over the snapshot, missing-lexeme 6 → 0; re-measure.

### Source touchpoints

**Slices 1 & 2 — parse-time, `kernel/src/dcg/lookup.rs`** (mirror the `-ly` trio; no importer change, no reseed):

| new / change | what | model on |
|---|---|---|
| `adjective_bases(surface)` — **new fn** | strip a closed prefix `{hyper,hypo,poly,multi,mono}` / split at `-` → base candidate(s) | `adverb_bases` (`:2897`); prefix-exception list like `NON_ADVERB_LY` (`:2901`) |
| `is_derived_adjective(&self, surface)` — **new fn** | recognition gate: base resolves to `ADJ` (Slice 1) / `N` (Slice 2). Reuse `is_adjective_cat` (`:2972`) | `is_derived_adverb` (`:912`) |
| `derived_adjective_items(&self, surface)` — **new fn** | seed the `ADJ` `Item`(s): Slice 1 copies the base/head's own `Item`s; Slice 2 builds sem `λx. base(x, X)` from the `base` axiom | `adverb_items` (`:921`) |
| `has_token` (`:569`) — **add clause** | count a derived adjective as *known* (not missing-lexeme) | the adverb clause at `:588` |
| seed loop (`:1862`) — **add call** | invoke `derived_adjective_items(&surface)` beside `adverb_items` | `self.adverb_items(&surface)` (`:1862`) |

**No new grammar rule or category:** the derived `ADJ` reuses `S[adj]\NP` + `RefineKind::Attrib`;
`hyper-`/`double-` reuse the adjective-modifier cat. (`-based : ADJ\N` is the categorial *analysis*;
synthesize-on-token builds the `ADJ` item directly, so nothing new is declared.)

**`recq` (separate — gene-family named entity):** `RecQ` is not a single gene but a gene *family* —
authoritatively **HGNC gene group 1049** ("RecQ like helicases": BLM, RECQL, RECQL4, RECQL5, WRN); the
parallel UMLS/MeSH concept is **C0084304** ("RecQ Helicases" / "RecQ Family of DNA Helicases"). The
document uses it as a bare modifier (`a RecQ DNA helicase`, `the four other RecQ DNA helicases` = WRN +
the other four members). It is OOV because a family has no bare-token lexical entry: the NCBI importer
reads per-gene `gene_info` (no gene-*group* records), and UMLS carries the concept only as ≥2-token
strings (shortest `RecQ Helicase` / `Helicase, RecQ`), so the single-word surface `recq` matches nothing
(verified against `MRCONSO.RRF` 2026AA + `Homo_sapiens.gene_info`). The fix is a named-entity `cat_np`
entry for `RecQ` **aligned to HGNC:1049** (primary — the nomenclature authority for families),
cross-referenced to UMLS C0084304 — grounding, not a minted symbol.

The five members are **already imported** in `Homo_sapiens.gene_info` (per-gene, each carrying an HGNC
dbXref and a `… RecQ like helicase` description) — so the family is derivable from data on hand, only the
group record is missing:

| member | NCBI GeneID | HGNC | description |
|---|---|---|---|
| RECQL (RECQL1) | 5965 | HGNC:9948 | RecQ like helicase |
| WRN | 7486 | HGNC:12791 | WRN RecQ like helicase |
| BLM | 641 | HGNC:1058 | BLM RecQ like helicase |
| RECQL4 | 9401 | HGNC:9949 | RecQ like helicase 4 |
| RECQL5 | 9400 | HGNC:9950 | RecQ like helicase 5 |

**General gap (follow-on, beyond this note):** gene *families* used as bare modifiers are a recurring
class the current lexicon cannot resolve — the per-gene `gene_info` import has the members but no
family/group record, and UMLS holds the family only as multi-token strings. Two data paths for a
family-entity source: **(a) structured** — import HGNC gene groups (group id → label + member HGNC IDs),
the authority; **(b) derivable** — the members are already imported, each with its HGNC dbXref and a
shared `RecQ like helicase` descriptor, so a family layer can be synthesized from data on hand. Its own
domain-lexicon track, not the morphology and not a per-token handler.

**Step 2b (deferred, importer) — `crates/eigenius-wordnet/src/convert.rs`** (mirror the `PpOblique` change,
commit `2b22705`; then reseed):

| change | what |
|---|---|
| `FrameKind` (`:145`) | add a `TransitivePp` variant |
| `tag`/`arrow`/`cat` (`:166`/`178`/`209`) | arms for `TransitivePp` → `((S\NP)/cat_pp_arg)/NP` |
| `classify` (`:229`) | route the object+PP frames {13,20,21,22} → `TransitivePp` |

### Explicitly not this work (and not blocking the OOV)
- **Affix-functor split** (the alternative to synthesize-on-token) — split the token into morpheme-spans
  and compose via functors. Not built: it needs an affix-aware **pre-tokenizer** (concatenation has no
  boundary; hyphen-splitting fights the intact-multiword rule `give rise` relies on) *and* it **adds chart
  items → worse crowding** (the `give rise` beam pressure). Synthesize-on-token seeds one item per word and
  reuses the existing pattern. Revisit only if the affix inventory grows large.
- **Phrasal `based on X` argument reading** — the §2a *phrasal* half, built by the object+PP frame
  extension (**Step 2b**, closure plan: importer change + reseed). Independent of the compound OOV; the
  compound half is Slice 2. The phrasal half + its passive machinery are their own tracks —
  [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md) (spec + invariant) and
  [d63-passive-voice-handling.md](d63-passive-voice-handling.md) (promotion/agent/roles).

## 3b. Close-out — generalize the compound half to the full denominal-suffix set

The shipped `-based` slice (Slice 2) is **one row** of a productive class (`-like`, `-mediated`,
`-dependent`, `-derived`, `-related`, `-induced`, `-specific`, …). Closing out the compound-morphology work
= generalize the **compound recognizer** to the whole set — the compound half only; the phrasal `E link X`
alignment and the passive machinery are separate parked tracks
([d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md),
[d63-passive-voice-handling.md](d63-passive-voice-handling.md)). Two changes in
[`kernel/src/dcg/lookup.rs`](../../kernel/src/dcg/lookup.rs):

**Status: implemented** (`2026-07-05`). Three changes in
[`kernel/src/dcg/lookup.rs`](../../kernel/src/dcg/lookup.rs):

| change | what | anchor |
|---|---|---|
| `DENOMINAL_SUFFIXES` const table | `&[(suffix, relation_lemma, theta_is_object)]` — `based`/`mediated`/`derived`/`induced`→their verb (θ object); `like`/`dependent`/`related`→`resemble`/`depend`/`relate` (θ subject). `-specific` deferred (no verb relation). | new const `:3166` |
| `denominal_based_item` → `denominal_suffix_item` | table-driven: rsplit hyphen → `(X, suffix)`; fetch the element's 2-place relation axiom; build `λθ. rel(first, second)` with the arg order set by `theta_is_object`. | `:1008` |
| `adjective_bases` — drop `SLICE2_TAILS` | exclude every `DENOMINAL_SUFFIXES` tail from Slice-1 hyphen-head identity — **fixes the `-like` over-generation** (`like` is a WN adjective, so Slice-1 otherwise seeds identity `like` and drops `X`). | `:3048` |

**Two deviations from the plan above** (surfaced, not silent):
1. **Adjective-voice suffixes route to a *verb*, not an adjective fetch.** A 1-place adjective (`like`) is
   not a 2-place relation, so `-like`/`-dependent`/`-related` use the corresponding verb `resemble`/`depend`/
   `relate`. `is_transitive_verb_cat` was broadened to **`is_binary_relation_cat`** (`:3138`) to also accept
   argument-PP verbs `(S\NP)/cat_pp_arg` (e.g. `depend on`) — both carry a 2-place axiom.
2. **Argument order flips by voice.** Passive-participle (`θ is based on X`) → θ is the object, `rel(θ, X)`;
   adjective/active (`θ resembles X`) → θ is the subject, `rel(X, θ)`. The `theta_is_object` flag drives it.

Reused unchanged: `is_derived_adjective` (`:957`), `derived_adjective_items` (`:979`), `kind_of`,
`predicative_adjective_cat`, `has_token`, the seed loop. A suffix whose relation verb is absent from the
lexicon fails the probe → the token stays OOV (fail-safe), never a wrong reading. **Verified:**
`denominal_x_based_adjective_predicates_via_the_base_axiom` (verb voice, θ-first) +
`denominal_like_routes_to_the_verb_relation_and_does_not_drop_x` (adjective voice, X-first, + the
over-generation guard) in `closed_class_determiners.rs`; full kernel suite green. The `⟦X-E⟧ = ⟦E link X⟧`
equivalence is the *alignment* note's test, gated on the phrasal half.

## 4. Prior art

- **Compound splitting** — Koehn & Knight, *Empirical Methods for Compound Splitting*, EACL 2003
  ([ACL E03-1076](https://aclanthology.org/E03-1076/); geometric-mean frequency split). Cited as the
  approach we **reject** for `hyper-` (open-domain, statistical).
- **Finite-state / two-level morphology** — Koskenniemi (1983); Beesley & Karttunen, *Finite State
  Morphology* (CSLI, 2003). The analyze-against-a-lexicon family our recognizer sits in. *(Canonical;
  verify exact bib details before adding to `eigenius_related_work.bib`.)*
- **Affixes / derivation as lexical rules in categorial grammar** — the CG lexical-rules tradition (an
  affix as a functor category; the derived word's cat+sem produced by rule, not stored). Our `-ly`
  handling and this note are instances.
