# D63 — Denominal-suffix adjectives: aligning `X-E` with `E link X`

**Status:** design (short). The **shared spec** for the productive class of biomedical denominal adjectives
(`-based`, `-like`, `-mediated`, `-dependent`, `-derived`, `-related`, `-induced`, `-specific`, …): the
`DenominalElement` table (§3) + the alignment invariant (§2) guaranteeing compound `X-E` and phrasal
`E link X` generate the *same* proposition. Generalizes the `X-based` ≡ `based on X` convergence
([d63-compound-morphology.md](d63-compound-morphology.md) §2a). **The two implementation sites are separate
tracks:** the **compound half** closes out compound-morphology (§3b — generalize the recognizer + the `-like`
fix); the **phrasal half** rests on [d63-passive-voice-handling.md](d63-passive-voice-handling.md)
(promotion / agent / roles). This note owns the shared table + the invariant that ties them.

## 1. Problem

Two surface forms express the same relation and must share one encoding:

- compound: `PCR-based method`, `RecQ-like genes`, `cell-mediated response`
- phrasal:  `method based on PCR`, `genes like RecQ`, `response mediated by cells`

`X-based` ships (compound half, [d63-compound-morphology.md](d63-compound-morphology.md) §3 Slice 2), but
it is hardcoded to the single suffix `based` + the verb lemma `base`. Two gaps:

1. **No alignment structure** — the compound rule and the (deferred) phrasal rule (Step 2b) have no shared
   object forcing them to the same proposition; each is written independently.
2. **`-like` is mis-handled today.** `like` is a WordNet adjective (synset `01409581` "like, similar",
   Derived), so the Slice-1 hyphen-head rule ([`adjective_bases`](../../kernel/src/dcg/lookup.rs)
   `lookup.rs:3040`) fires on `RecQ-like`, seeds `like` with an **identity** sem, and **drops `RecQ`** →
   `Σg:Gene. like(g)` (contentless). Every `X-<suffix>` whose suffix is also a free adjective over-generates
   this way. The Slice-1 exclusion list `SLICE2_TAILS` (`lookup.rs:3045`) contains only `"based"`.

## 2. The structure — one relation per element, two ways to saturate it

Each table entry is a record, not just `{suffix → relation}`:

```
DenominalElement {
  element:   "based" | "like" | "mediated" | …    // the participle / adjective surface
  suffix:    "-based" | "-like" | …               // the compound (hyphen) form
  relation:  rel_E : Entity → Entity → Prop        // grounded to a WordNet verb / adjective axiom
  link:      Some("on"|"by"|"from"|"to") | None    // the phrasal linking preposition (None = direct)
  voice:     PassiveParticiple(verb) | Adjective   // selects the phrasal machinery
}
```

Both surface forms must produce **the same proposition** `rel_E(theme, ground)`:

- **Compound `X-E`** — the morphology rule seeds `λθ. rel_E(θ, kind_of(X))` on the whole hyphen token
  (synthesize-on-token, [d63-compound-morphology.md](d63-compound-morphology.md) §3).
- **Phrasal `E link X`** — the argument-PP machinery: `E : (S[adj]\NP)/cat_pp_arg`, `link : cat_pp_arg/NP`,
  sem `λθ. rel_E(θ, X)`. When `link = None` (`like`), `E` takes the NP directly.

**Alignment invariant** (one per row; run as a test): `⟦X-E⟧ = ⟦E link X⟧ = rel_E(theme, kind_of(X))`.

## 3. The table

The `relation` is always a **2-place verb axiom** — adjective-voice suffixes route to the corresponding
*verb* (`resemble`/`depend`/`relate`), since a 1-place adjective (`like`) is not a relation:

| suffix / element | relation `rel` ← verb | phrasal | link | voice (θ) |
|---|---|---|---|---|
| `-based` / based | `base` ← base.v.01 (`v00636888`) ✓ | based **on** X | on | passive-ptcp (θ obj) |
| `-mediated` / mediated | `mediate` ← mediate.v | mediated **by** X | by | passive-ptcp (θ obj) |
| `-derived` / derived | `derive` ← derive.v | derived **from** X | from | passive-ptcp (θ obj) |
| `-induced` / induced | `induce` ← induce.v | induced **by** X | by | passive-ptcp (θ obj) |
| `-like` / like | `resemble` ← resemble.v | like X / similar **to** X | ∅ / to | adjective (θ subj) |
| `-dependent` / dependent | `depend` ← depend.v | dependent **on** X | on | adjective (θ subj) |
| `-related` / related | `relate` ← relate.v | related **to** X | to | adjective (θ subj) |
| `-specific` / specific | *(no verb — deferred; needs a minted `specific_to`)* | specific **to** X | to | adjective (θ subj) |

`✓` = axiom Derived (witnessed). The rest are Declared — verify each verb axiom + its frame at
implementation (a missing verb → the token stays OOV, fail-safe). The **compound half of every row above is
implemented** ([compound-morphology.md](d63-compound-morphology.md) §3b, `2026-07-05`); the phrasal column
awaits [d63-passive-voice-handling.md](d63-passive-voice-handling.md). Voice sets the argument order:
`passive-ptcp` → θ is the object → `rel(θ, X)`; `adjective` → θ is the subject → `rel(X, θ)`.
X's thematic role varies (`on` = ground, `by` = agent, `from` = source, `like` = standard-of-comparison),
but that role is carried by each `rel_E`'s own meaning, so every entry collapses to a **2-place
`rel_E(theme, ground)`**; the linker only names the role.

## 4. Conceptual unifier — the hyphen is an elided linker

The deepest framing: `X-E` **is** `E` with its ground-argument saturated by `X`, pre-posed, the linking
preposition realized as the hyphen instead of a word:

```
X-based  ≡  based [on := hyphen] X          X-like  ≡  like [∅] X
```

So compound and phrasal are the *same functor* `E : λg.λθ. rel_E(θ, g)`, saturated morphologically (hyphen)
or phrasally (preposition) — the **affix-as-functor** view (the "style b" deferred in
[d63-compound-morphology.md](d63-compound-morphology.md) §3). Two implementations of the same alignment:

- **Style-a (current + shared table).** Keep two rules — the compound suffix rule and the phrasal
  argument-PP frame — both dispatching through the *same* `DenominalElement.relation`. Alignment enforced by
  the shared relation + the §2 invariant as a test. No pre-tokenizer, no new crowding. **Recommended.**
- **Style-b (functor).** One lexical entry per element, saturated two ways; alignment automatic. Needs the
  affix-aware pre-tokenizer + chart-crowding management. Revisit only if the inventory grows large.

## 5. Role/order convention (load-bearing)

`rel_E(theme, ground)` — **theme first** (the modified noun), ground second (X). Both halves must obey it, or
the invariant fails. The phrasal half therefore requires **passive-participle promotion** — promote the
verb's object (theme) to the subject slot and put X in the ground slot — which is the role correction from
[d63-compound-morphology.md](d63-compound-morphology.md) §2a ("Faithful target": `base_on(theme, ground)`,
not the coarse `v00636888_t(theme, agent)`).

## 6. Implementation sites (this note = the shared spec)

This note owns the **shared `DenominalElement` spec** (§2/§3) and the **alignment invariant** (§2, tested in
§7). The two halves are implemented in separate notes:

- **Compound half `X-E`** — [d63-compound-morphology.md](d63-compound-morphology.md) §3b: generalize the
  `-based` recognizer to the full suffix set + the `-like` over-generation fix
  ([`lookup.rs`](../../kernel/src/dcg/lookup.rs): `denominal_suffix_item` `:1007`, `SLICE2_TAILS` `:3045`).
  Self-contained; ships without any passive work.
- **Phrasal half `E link X`** — [d63-passive-voice-handling.md](d63-passive-voice-handling.md): the
  passive-participle promotion + agent suppression + `rel(theme, ground)` roles (importer/grammar), plus the
  argmarker linkers ([closed-class.esl](../../ontologies/lexicon/closed-class.esl), incl. the new `by_arg`).
  For `like` (link=∅) the phrasal is the preposition/adjective `like` taking NP directly.

**Where the table lives.** Declare `DenominalElement` in the lexicon ontology (a resource class + one
instance per row) — data, not code, per "everything is a Resource" — in a new
`ontologies/lexicon/denominal.esl` alongside [closed-class.esl](../../ontologies/lexicon/closed-class.esl).
Both implementation sites read it. Each `relation` references a WordNet verb/adjective axiom by sense
(base.v.01 `v00636888` ✓, like/similar `01409581` ✓, mediate.v, derive.v, induce.v, relate.v, depend.v,
dependent.a, specific.a) — never a freshly-minted predicate. A Rust const table in `lookup.rs` is the
fallback if declaring it is premature.

## 7. Verification — the invariant as a test

- **Demo-fixture** — [`kernel/tests/closed_class_determiners.rs`](../../kernel/tests/closed_class_determiners.rs):
  extend the `BASED_FIXTURE` pattern (a fixture verb/adjective per element); for each element parse `X-E` and
  `E link X`, assert **identical** pretty sem. This is the §2 alignment invariant, mechanized.
- **Full lexicon** — [`crates/eigenius-wordnet/tests/db_backed_encoding.rs`](../../crates/eigenius-wordnet/tests/db_backed_encoding.rs):
  the `show_based_on_x_reading` diagnostic pattern, per element, over the snapshot.
- **Over-generation guard** — assert `RecQ-like`-shaped tokens no longer take the Slice-1 identity reading
  (the §1.2 fix).

## 8. Cross-references

- [d63-compound-morphology.md](d63-compound-morphology.md) — the `X-based` slice this generalizes (§2a
  convergence, §3 Slice 2, the `[[gene_family_lexicon_gap]]` for `RecQ` itself).
- Prior art (from compound-morphology §4): CG lexical rules (affix as functor); finite-state / two-level
  morphology (Koskenniemi 1983; Beesley & Karttunen 2003).
- Note `RecQ-like genes` is **double-blocked**: it also needs the `RecQ` gene-family entity (HGNC:1049) —
  the gene-family track, separate from this one.
