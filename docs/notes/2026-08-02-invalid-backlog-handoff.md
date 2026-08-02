# Driving the `invalid` backlog to zero — state and next steps (2026-08-02)

Handoff note. The goal is unchanged: eliminate **fundamentally incorrect** parses on the WRN-helicase
first page (CNL-v3, 62 units), measured as the adjudication ledger's `invalid` rows, with
`grammar-gap 0` **non-negotiable** and no loss of a pinned correct reading.

## Where things stand

Snapshot `wordnet-umls-aligned-2026-08-01-prednom`, tracked replay
`experiments/parsing/ranks/2026-07-29-demonstratives.json`. Steps 1 and 2 below are **done** —
their results are folded in here; the section text is kept because it records how each was measured.

| | `f89717e` | + adverb binding |
|---|---|---|
| grammar-gap | 0 | **0** |
| expected-hits | 60/62 | **60/62** (same two documented misses) |
| encoded | 14 | 14 |
| total-readings | 953 | 895 |
| total-skeletons | 164 | **160** |
| `invalid` | 11 rows / 5 units | **6 rows / 4 units** |
| unadjudicated | 0 | **0** |

Same snapshot on both sides, so this is a clean A/B — the only variable is the category.

## The 6 remaining `invalid` rows

| unit | rows | blocked on |
|---|---|---|
| We found that WRN was selectively essential in MSI models. | 2 | enumeration (step 2) — **still costs a pin** |
| These libraries define genes that were essential for proliferation and survival. | 2 | enumeration (step 2) |
| We ascertained MSI status with sequencing. | 1 | enumeration (step 2) |
| We hypothesized that other DNA repair defects would give rise to synthetic-lethal relationships. | 1 | **scope island** (step 3), its own axis |

The five rows on «These classifications were highly concordant…» are gone — all five, where step 1
was predicted to clear three. That unit fell 8 skeletons → 3.

---

## Step 1 — make adverbs feature-preserving — **DONE**

**The defect.** Under `pred ⊑ adj`, an adverb typed `(S[adj]\NP)/(S[adj]\NP)` *accepts* a `pred`
argument and *returns* a fixed `adj`, so it launders a predicate nominal back into an
attributive-capable adjective and re-opens exactly the hole `lexicon:pred` was introduced to close.
Structurally identical to the way `fin_any` launders finiteness (`3ae672d`) — that pattern is now
confirmed twice, and it is worth treating "a rule shaped `X → X` on a feature" as a laundering
suspect on sight.

**WHERE THE NOTE WAS WRONG.** This said the adverbs were emitted by the WordNet importer, needing an
importer change and a reseed. Both are false, and `baseline.json`'s previous note repeated it. The
importer defers adverbs entirely (`Pos::Adv => {}`, `crates/eigenius-wordnet/src/convert.rs`) and
emits no `(S[adj]\NP)/(S[adj]\NP)` anywhere. The categories are built in the KERNEL by
`dcg::category::adverb_modifier_cats` and seeded productively for `-ly` forms and for the
lexicalized list (`dcg::parse::seed`). The set really is open — `-ly` recognition is data-driven off
the adjective base, which is why enumeration could not close it — but the fix is kernel-side and
needs **no reseed**. Cost: two builds instead of a ~40-minute reseed.

**The fix, as landed.** One polymorphic forward category `(S[f]\NP[n])/(S[f]\NP[n])` replacing the
two fixed ones (`adj` for the adjective modifier, `fin` for the forward VP modifier); the bound `f`
covers `fin` too, so keeping a separate `fin`-fixed forward category would only duplicate every
VP-adverb derivation. The BACKWARD VP modifier stays fixed at `fin` — it never laundered (accepts
`fin`, returns `fin`) and post-adjectival adverbs are not attested on this page, so binding it would
widen coverage on no evidence. Pinned by
`the_forward_adverb_modifier_binds_the_clause_feature_it_consumes` (`category.rs`), which asserts the
shared variable and that no seeded adverb category fixes `adj`.

**Measured:** gap 0, pins 60/62 held, skeletons 164 → 160, readings 953 → 895, `invalid` 11 → 6.
All five `classifications` rows cleared where three were predicted.

**The widening is real.** A variable admits every `Fin` value, not just the two that were enumerated,
and one new skeleton appeared: «MSI is most commonly observed …» gains the passive reading of `is
observed` with the agent abstracted. Adjudicated `available` from its gloss — it differs from that
unit's pin only in dropping the `gt(…, std)` degree, the transparent-adverb family the grammar
supports deliberately.

**Still to do in the next reseed:** drop the five now-redundant `_prednom` entries from
`ontologies/lexicon/closed-class.esl` (`is/are/was/were_copula_prednom`, `not_adj_prednom`).
Subsumption admits `pred` at the plain `adj` slot, so they only add duplicate derivations. The
current 160 includes them, so this number should improve — treat any change as a finding, not noise.

---

## Step 2 — recover the enumeration's lost pin (clears 5 rows) — **RE-MEASURED, still blocked**

**Result on the `pred` baseline (2026-08-02).** gap 0, skeletons 164 → 150, readings 953 → 817,
encoded 14 → 15, expected-hits 60 → **59**. The lost pin is «MSI is associated with notable responses
to immune checkpoint blockade.» — the SAME one as before, so the 1-pin cost did not change under the
`pred` split and adoption stays blocked. The open question below is therefore unchanged, and the
forest trace is the next instrument.

**What exists.** `experiments/parsing/probes/vpadj-enumerate-verbal.esl` — 66 entries replacing the
11 VP-adjunct prepositions' `fin_any` with the six verbal `Fin` values, one concrete feature on both
sides per entry. On the OLD baseline it measured gap 0, skeletons 179 → 165, and removed 7 `invalid`
rows — but cost the pin on «MSI is associated with notable responses to immune checkpoint blockade.»

**Why it is not adopted.** A lost pin is a lost *correct* reading, which outranks backlog reduction.

**What is already ruled out.** All six concrete-feature options were measured (table in
`experiments/parsing/probes/vpadj-crossproduct.esl`). The 7×7 cross-product makes every
(result, argument) pair expressible and STILL loses that pin, so `fin_any` does not stand for "some
pair" — it ERASES the feature, and the erasure licenses a derivation no concrete assignment does.
Beam was excluded with a control: all arms bit-identical at `CELL_BEAM` 64 and 256, base included.

**How the re-measurement was run** (repeat it after any change to the base):

```bash
scripts/probe-mode-layer.sh \
  --base /home/hm/src/eigenius/db-snapshot/wordnet-umls-aligned-2026-08-01-prednom \
  --layer experiments/parsing/probes/vpadj-enumerate-verbal.esl
```

The open question is narrow and specific: **what does `fin_any`'s erasure license in that one
derivation?** The instrument is
`EIGENIUS_TRACE_FOREST=deriv:i..j` on that span, base vs enumeration, comparing which rule and split
build the surviving constituent.

---

## Step 3 — the scope-island row (1 row, separate axis)

«We hypothesized that other DNA repair defects would give rise to synthetic-lethal relationships.»
The `to`-PP escapes both the `that`-clause and the `Would` modal and lands on the matrix subject,
asserting `we to a synthetic-lethal relationship`. Untouched by everything above. Not yet diagnosed
to a rule — start with the reading dump and the forest trace, as in step 2.

---

## Step 4 — housekeeping

- **`probe-mode-layer.sh` image guard.** It brings the kernel up with the existing Docker image and
  never checks it matches the base snapshot's bootstrap. A mismatch surfaces as `error: kernel
  exited before becoming healthy` with a hash dump — opaque. Cost a debugging cycle on 2026-08-01
  after a reseed rebuilt the image and the source was then reverted.
- **22 stale ledger rows.** Was 2, then 10; the `pred` change and the adverb binding each removed
  skeletons that carried verdicts. Nine of them are `invalid` rows for defects now FIXED (the five
  `classifications` rows and the four `findings` rows) — those carry the diagnosis of the
  false-identity family, part of which is still open, so deleting them loses evidence. Deleting is a
  deliberate act — decide, don't prune silently. **Note:** 2 of the stale entries are PINS, not
  ledger rows (`audit-skeletons.sh` merges `expected-readings.tsv` in as `correct` verdicts), and
  `expected-readings.tsv` explicitly forbids re-pinning either.

---

## Instruments (re-derived 2026-08-01, easy to lose)

| instrument | gives | caveat |
|---|---|---|
| `EIGENIUS_DUMP_READINGS=1` (`EIGENIUS_READINGS_MAX`, dflt 40) | raw term + verbalized English, on the sweep WITH the document overlay | the only one like-for-like with the ledger; **over-counted skeletons 166 vs 164** — do not use it for a census |
| `scripts/audit-skeletons.sh` | the validity ledger: correct / available / **invalid**; fail-closed on unadjudicated | authoritative skeleton set; needs `EIGENIUS_DB_SNAPSHOT` + `EIGENIUS_SENSE_RANKS` matching the baseline |
| `EIGENIUS_TRACE_FOREST=deriv:i..j` / `top` | hyperedge tree — which split, which rule, which child cells | reps, not readings |
| `EIGENIUS_DUMP_CELL` | flat item bag per cell | no derivation structure |
| `EIGENIUS_TRACE_SENTENCE` | single-sentence trace | **cap-only, no document overlay** — different reading set |

**The recorded trap:** skeletons are sense-erased (`§`) and glosses are verbalized English, so
grepping either for a predicate name matches nothing and reads as "0 occurrences" rather than as the
category error it is. This has produced three wrong analyses in this corpus.

## Standing constraints

- `grammar-gap 0` is **non-negotiable**; a lost pin outranks any skeleton reduction.
- `--release` is load-bearing (debug overflows the stack in NbE readback → phantom GRAMMAR-GAP).
  Use `scripts/measure-parse-rate.sh`; do not hand-roll `cargo test`.
- `EIGENIUS_WRN_PAGE` / `EIGENIUS_SENSE_RANKS` / `EIGENIUS_DB_SNAPSHOT` must be ABSOLUTE paths.
- Any reseed needs `--umls-all`. The default is a subset (3.29M resources vs 9.19M) and the mismatch
  is SILENT — it once produced a fake catastrophic regression.
- UMLS data is licensed; `/references` is gitignored and UMLS content is never committed.
- A bootstrap edit (`lexicon-ontology.esl`, `closed-class.esl`) cannot be resumed on an old store —
  ManifestDrift, fail-closed. Mode assignments and lexical shadowing are patchable via
  `scripts/add-layer-to-snapshot.sh`; type declarations are not.
