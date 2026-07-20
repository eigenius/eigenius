# D63 — Right-node raising over lexicalized compounds (head distribution)

**Thesis.** A coordinated pre-nominal modifier list with a shared head noun ("colon, gastric,
endometrial and ovarian cancers") is faithfully a **union of lexicalized kind concepts**
`Or(⟦colon cancer⟧, ⟦gastric cancer⟧, …)`, not a generic head with a disjunctive modifier. Reaching
those concepts requires **head distribution**: split the head onto each conjunct, re-consult the
lexicon per distributed compound, and build the union **directly**. This supersedes the `cat_mod`/`Or`
approach ([d63-coordinated-modifier-category.md](d63-coordinated-modifier-category.md) §9f) for the
shared-head case; `cat_mod`/`Or` remains the fallback where no distributed compound lexicalizes.

## 1. The problem (Derived — probes via `EIGENIUS_TRACE_SENTENCE`)

The distributed compounds are lexicalized concepts, but the coordination hides them. Each "X cancer"
traced as "X cancer is common":

| compound | own entry |
| --- | --- |
| colon cancer | WordNet `n14247239` |
| gastric cancer | UMLS `C0024623` |
| endometrial cancer | WordNet `n14247458` |
| colorectal cancer | UMLS `C0009402` |
| insertion mutation | UMLS `C1512796` |
| deletion mutation | UMLS `C1511760` |

`ovarian cancer` has **no** bigram entry (composes). A multiword lexeme needs **adjacency**; in
"colon, gastric, endometrial and ovarian cancers" the head is one token at the end, separated from
`colon`/`gastric`/`endometrial` by the list, so none of the lexicalized concepts are reachable. `cat_mod`/`Or`
(D63 M3) unions the right *modifiers* — `Σx:cancer. Or(colon x, gastric x, …)` — and throws away the
right *kinds*.

**Page prevalence:** three shared-head coordinations are affected — "colon, gastric, endometrial and
ovarian cancers" (para 2), "colorectal, endometrial, gastric and ovarian cancers" (para 4, a live
measurement unit), "insertion or deletion mutations" (para 2, both compounds lexicalized). Not a
one-off.

## 2. The reduction that does NOT work (Derived — fail-closed)

The tempting design is **un-elision**: rewrite "M₁, M₂, … and Mₙ Head" to the head-repeated
"M₁ Head, M₂ Head, … and Mₙ Head" and let existing coordination + adjacency-multiword-lookup handle it.
**Invalidated by probe** — even the head-repeated form fails:

- "Colon cancer and endometrial cancer are common." → **32 readings / 6 skeletons**; reading[0] is junk
  (`cancer`-as-verb `v02604760`, zodiac `n08686658`).
- "Colon cancer, gastric cancer and endometrial cancer are common." → **80 / 4**, same junk.
- "Colon cancers and ovarian cancers are common." → 2 / 1, but **mis-parsed as a nested compound**
  `compound_kind(x, compound_kind(y, n14247239))`, not a union.

Two facts fall out: **(i) multiword-preference does not survive coordination** — the coordination
context re-opens `cancer`'s bare-noun senses (verb/zodiac) that the standalone "Colon cancer is common"
(1 reading, `n14247239`) suppresses; **(ii) coordinating compound NPs is itself broken** — the `and`
gets absorbed into a compound instead of coordinating. So RNR cannot reduce to NP-coordination; it must
build the union **directly** from re-looked-up atomic concepts, sidestepping both failures.

## 3. Target semantics (Declared)

For "M₁, …, Mₙ Head" over head class `H`:

```text
Σx:H. Or(δ₁, …, δₙ)
  δᵢ = is_a(x, Kᵢ)                      if "Mᵢ Head" lexicalizes to concept Kᵢ  (Kᵢ ⊆ H)
  δᵢ = compound_kind(x, ⟦Mᵢ⟧)  (noun)   otherwise — the M3 compositional restrictor
  δᵢ = ⟦Mᵢ⟧(x)                 (adj)    otherwise
```

Example: `Σx:cancer. Or(is_a(x, n14247239), is_a(x, C0024623), is_a(x, n14247458), compound_kind(x, ovarian))`.

Two semantic sub-questions, deferred (they do not change the mechanism):

- **kind-membership predicate.** `is_a(x, Kᵢ)` is a placeholder; the exact form must match how a
  lexicalized compound denotes as a common-noun restrictor over its supertype (Kᵢ ⊆ H is the typing
  constraint). To confirm against the grammar's existing kind representation.
- **distributed predication variant.** "X are common" may mean *each kind is common*
  `And(P(kind_of K₁), …)` rather than *the union is common* `P(Σx. Or …)`. The union-restrictor form
  above is primary (direct analogue of M3); the distributed-predication reading is a possible second
  reading, gated on need.

## 4. Architecture (Declared)

Head distribution needs **surface + lexicon** access (to reconstruct "Mᵢ Head" and re-look-it-up), which
the pure combinators (`apply`, `coordinate_mod`) do not have and must not gain (the packed-forest
soundness invariant — rules decide on `(cat, sem)` only). It **does** belong where multiword lookup
already lives: a **cell-level, token-and-lexicon-accessing rule**, run by both chart drivers, exactly as
`multiword_protected_splits` / `lookup_span` are. So:

**A `distribute_head` cell rule** at a span `[i, j]` whose tokens are `[modifier coordination] head`:

1. re-tokenize `tokens[i..j-1]` on the coordination connectives (`,` / `and` / `or`) → modifier
   surfaces `s₁ … sₙ`; the head is `tokens[j]` (with its singular/plural variants);
2. for each `sᵢ`, look up the multiword `"sᵢ ‹head›"` in the existing **surface** index (no new index) —
   trying head plural and singular; → concept `Kᵢ` or `None`;
3. build `Σx:H. Or(δ₁ … δₙ)` **directly** (§3) — not via `coordinate_np`/`coordinate_mod`, so the broken
   NP-coordination path (§2) is bypassed;
4. seed it as one item at `[i, j]`.

This preserves token offsets (unlike un-elision's rewrite → provenance to the source text is kept), and
adds no new index (it reuses the surface multiword lexemes).

**Rejected alternative — pre-parse un-elision rewrite.** Detect the pattern, rewrite the input to the
head-repeated form, parse that. Rejected: (a) inherits the §2 breakage (head-repeated coordination
explodes); (b) loses source offsets; (c) detection is a fragile pre-parse pass outside the forest.

## 5. Preference & multiplicity (Declared)

`distribute_head` is **multiword-preference for the distributed case**. When ≥1 conjunct lexicalizes,
the union reading must be **preferred and the compositional `cat_mod`/`Or` reading suppressed** at that
span — the same principle `66d84de` applies to *adjacent* compounds (prefer the lexeme over composition),
extended to the *coordination-shared-head* case. Without suppression the two readings (union-of-kinds vs
generic-head-with-Or-modifier) are semantically distinct, do not pack, and double the count. **Fallback:**
if no conjunct lexicalizes, `distribute_head` yields nothing and `cat_mod`/`Or` stands — coverage is never
reduced.

## 6. Invariants & constraints

1. **Differential oracle** (packed ≡ unpacked): `distribute_head` must be applied identically by both
   drivers — it is cell-level like multiword lookup, so both call it on the same spans.
2. **Felicity:** the union `Σx:H. Or(…)` must type-check; each `Kᵢ ⊆ H` (a colon-cancer is a cancer), so
   the disjuncts are well-typed at `x:H`.
3. **Coverage NON-NEGOTIABLE:** grammar-gap 0 held — the rule only *adds* the union reading and
   *suppresses a duplicate*; it never removes the sole parse of a sentence.
4. **Offsets preserved:** the reading spans the original tokens; no rewrite.
5. **Multiword-preference-through-coordination** (§2.i) is a prerequisite the rule *embodies* rather than
   fixes globally — it builds the union directly, so it does not depend on the bare-noun senses being
   suppressed elsewhere. (The general "coordination re-opens senses" bug (§2) is logged separately; this
   rule routes around it for the shared-head case.)

## 7. Detection — when it fires (Declared)

Fire on `[i, j]` iff: `tokens[i..j-1]` is a **pre-nominal modifier coordination** (≥2 items separated by
`,`/`and`/`or`, each a modifier-eligible surface — adjective or noun, not a determiner/verb) and
`tokens[j]` is a **common-noun head**. Do **not** fire on: head-repeated coordinations ("MSI lines,
microsatellite-stable lines and indeterminate lines" — each conjunct already has its head; there is no
elision); bare-NP/name coordinations ("project Achilles and project DRIVE"); predicate coordinations.
Over-firing is bounded — a mis-fire that finds no lexicalized `Kᵢ` produces nothing (§5 fallback).

## 8. Staging (Declared)

- **M-RNR-1 — surface re-lookup primitive.** A cell helper: given a modifier-coordination span + a head
  token, return `[(sᵢ, Kᵢ_or_None)]`. Unit-tested against the §1 table (no snapshot: mock lexemes).
- **M-RNR-2 — `distribute_head` rule, union builder + felicity.** Build `Σx:H. Or(δ₁…δₙ)`; confirm it
  type-checks and the oracle holds. Measure on "insertion or deletion mutations" (the cleanest case:
  both lexicalize → `Or(C1512796, C1511760)`).
- **M-RNR-3 — preference/suppression.** Suppress the compositional reading when the union fires; confirm
  no doubling and the two cancer units collapse. Re-measure cap-only + reranked; re-baseline.
- **M-RNR-4 — kind-membership predicate.** Resolve `is_a` against the grammar's kind representation
  (§3); until then M-RNR-2/3 may carry a placeholder predicate flagged in the term.

## 9. Risks & open questions

- **§2 coordination-reopens-senses bug** is deeper than RNR and may need its own fix; RNR routes around
  it for the shared-head case but the head-*repeated* case ("X cancer, Y cancer") stays broken. Scope:
  is the head-repeated case on the page? (Grep says no — para 3's "… lines" is the only repeated head,
  and "line"/"lines" is not the exploding surface.) So defer.
- **Head morphology.** "cancers" (pl) must re-look-up "colon cancers" AND "colon cancer"; the lexicon
  carries both singular (`n14247239`) and plural (`C0007600` for cell line) surfaces. Try both, prefer
  the number-matching hit.
- **Multi-token modifiers** ("microsatellite-stable"): re-tokenizing on connectives keeps them intact
  (they contain no connective); verify the compound "microsatellite-stable X" lookup.
- **`ovarian cancer` coverage gap** (surface absent; concept exists as "ovarian carcinoma" `C0919267`):
  logged, not fixed here — that conjunct composes.
- **Semantics of union-of-kinds** (§3): the `is_a` predicate and the distributed-predication variant.

## 10. Validation criteria

1. oracle packed ≡ unpacked holds with `distribute_head` in both drivers;
2. "insertion or deletion mutations" → `Or(C1512796, C1511760)` (both lexicalized — the clean case);
3. both cancer units carry the union reading with the three lexicalized `Kᵢ` + the composed `ovarian`;
4. no doubling — the compositional `cat_mod`/`Or` reading is suppressed where the union fires;
5. coverage grammar-gap 0; cap-only does not rise; reranked encoded does not fall.

## 11. Update — implemented, default-on (Derived)

Built as `Parser::distribute_head` (seed-time, `parse/seed.rs`) + a two-line extension of
`multiword_spans` (`chart/mod.rs`); default-on, on top of the M3 baseline. The design held; four things
were resolved in the build.

**Semantics — (B), and (A)/(B) are not exclusive.** `core:is_a` is a resource-model **Property**, not an
`Entity→Class→Prop` axiom, so (A)'s `Or(is_a(x,Kᵢ))` has no predicate the felicity gate can check. (B)
references each whole kind **directly** (`kind_of(Kᵢ)` group members) — no `is_a` — and reuses the
existing distributive machinery, which (verified) handles **both** subject and object predication. So
(A)'s supposed edge (uniform `cat_kind` predication) is not actually needed — (B)'s distribution already
covers both positions — and (A) still needs `is_a`. (B) is the encoding; §3's kind-membership
sub-question is moot for it.

**As built:** split the conjuncts; look up each "conjunct + head" **in isolation**; build the bare-kind NP
`cat_np(H, kind_of(Kᵢ))` — typed by the **head** class `H`, sem the specific kind `Kᵢ`; fold
`coordinate_np` → `cat_group`; the distributive rules (subject and object) predicate over it.

**Wall 1 — the group would not combine (two causes).**

| cause | fix |
| --- | --- |
| `common_super(Kᵢ) = Entity` — UMLS-CUI compounds lack a loaded ancestor narrower than the top, so `group_member_fits` fails against any concrete slot | type the group by the **head class `H`** (each `Kᵢ ⊆ H`), not `common_super` |
| ("1b", a MISDIAGNOSIS) the pre-`H`-typing debug showed the subject VP slot as `cat_kind` | after `H`-typing the group distributes in **both** object AND subject position (verified — see §11 corrections); no `group→kind` shift needed. The `cat_kind` slot was one of several VP forms; the group matches a `cat_np` one |

**Wall 2 — coverage gap.** The sense **cross-product** blew up: 4 conjuncts × ~4 senses on
"colorectal, endometrial, gastric and ovarian cancers" → up to `4⁴` = 192 group seeds → a forest blow-up
that parsed unit 53 to 0 readings. Fix: take the **top-ranked sense per conjunct** (`.take(1)`) — the union is one
structural reading; sense choice is the cap/reranker's job downstream.

**M-RNR-3 suppression (§5).** A seeded `cat_group` in the LEAVES is necessarily an RNR seed (compositional
groups are built later in the CKY), so `multiword_spans` counts it and its span is protected exactly like a
lexicalized multiword — the coordination's guessed cat_mod/composition is pruned **with the multiword widen
fallback**, so coverage is safe by construction (grammar-gap 0). Replace, not add.

**Results.** "MSI occurs in colon, gastric, endometrial and ovarian cancers" → **1 reading**:
`And(occurs(MSI, K)…)` over `colon n14247239 · gastric C0024623 · endometrial n14247458 · ovarian C1140680`
(was 30 readings / 5 skeletons — §1). "insertion or deletion mutations" → the `Or` union, 2 sense-variants.
Deterministic cap-only 2321→2304 (−17), **encoded 2→4**; reranked encoded 12→13, total 1124→1141 (the
+17 is `SENSE_CAP`/single-draw entanglement — cap-only is the clean A/B, the same pattern M3 documents).
grammar-gap 0; differential oracle holds; §7's all-lexicalized gate keeps RNR off non-compound
coordinations (verified: "MSI and MMR deficiency" does not fire).

Two corrections to earlier claims. (i) The "ovarian cancer does not lexicalize" note was wrong — the
plural surface *does* (`C1140680`). (ii) **Subject position is NOT deferred** — that was a misdiagnosis
(Wall "1b" above): after the `H`-typing fix the group distributes in subject position too (verified:
"Insertion or deletion mutations are common" → `Or(common(kind_of C1512796), common(kind_of C1511760))`;
"Colon, gastric and endometrial cancers are common" → the 3-kind `And` union — each a clean 1-reading).
The genuine remaining gap is the **head-REPEATED** coordination of lexicalized compounds ("Colon cancer,
gastric cancer and endometrial cancer are common" — 80 readings of junk), which is NOT the shared-head
case RNR handles: `distribute_head` does not fire (there is no shared head to distribute — each conjunct
is already a full "X cancer"), and coordinating the compound NPs directly reopens the head's junk senses
(the §2 multiword-preference-through-coordination problem). It is not on the WRN page (which uses the
shared-head form). Also still open: head morphology (sg/pl re-lookup).

## References

- Supersedes for the shared-head case: [d63-coordinated-modifier-category.md](d63-coordinated-modifier-category.md)
  (`cat_mod`/`Or`, M1+M3, committed `e7c1b24`); §9 there records the survey and this reframing.
- Multiword-preference this extends: baseline history `66d84de`
  ([experiments/parsing/baseline.json](../../experiments/parsing/baseline.json)).
- Code touch-points: `lookup_span` / `multiword_protected_splits`
  ([chart/mod.rs](../../kernel/src/dcg/chart/mod.rs)); the seed path
  ([parse/seed.rs](../../kernel/src/dcg/parse/seed.rs)); `coordinate_mod` / `complete_coord` fallback
  ([rules/constructions.rs](../../kernel/src/dcg/rules/constructions.rs)).
- Instruments: `EIGENIUS_TRACE_SENTENCE` / `EIGENIUS_TRACE_SKELETONS` on `trace_one_sentence`
  ([db_backed_encoding.rs](../../crates/eigenius-wordnet/tests/db_backed_encoding.rs)).
