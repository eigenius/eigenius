# D66 slice 3 — hand-off

*2026-08-10. Written to move this work to another machine. Design is
[`docs/design/d66-definitional-lifting-and-witness-normalization.md`](../design/d66-definitional-lifting-and-witness-normalization.md)
(decision-complete, D1–D10). Slices 0–2 are landed and verified; slice 3 is code-complete but has
never run, because running it needs a reseed.*

---

## 1. Where it stands

| slice | what | state |
|---|---|---|
| 0 | witness index → direct lookup, constant footprint; `has_witness_candidates` on `LayerHandle` | **done** |
| 1 | emit side decodes before hashing; `kernel.layer.witness_decode` diagnostic | **done** |
| 2 | `nbe::subst`, `eigentt:Definition`, decode peel-and-substitute, Rule 24, ESL `def` | **done** |
| 3 | the demo rewrite — definitions replace generated shape rules | **code-complete, unrun** |

Everything is committed (HEAD `91d34c9`). `cargo test --workspace` 2776 passed / 0 failed, clippy
clean under `-D warnings`, as of the last full run.

## 2. The blocker: a reseed is required, and a layer patch cannot substitute

`ontologies/eigentt/eigentt-type-fragment.json` gained four resources — `eigentt:Definition`,
`definition_type`, `definition_body`, `definition_opaque`. That file is a **bootstrap** ontology:
the kernel carries it via `include_str!` and seeds it as a root-adjacent layer
(`kernel/src/bootstrap/mod.rs:256`).

Bootstrap is **content-verified**. Booting against a store seeded from a different version of an
embedded ontology fails:

```
Server error: persistent bootstrap failed: seed manifest drift —
refusing to boot against a DB seeded with different embedded ontologies.
stored:
  eigentt-type-fragment:ea4bdba301a2c272b5742b2164c0864bf12fddc4f868028196448ffee658002c
  …
```

So **every pre-D66 snapshot is unbootable with the current binary.** This was learned the expensive
way: `ontologies/eigentt/definition-layer.esl` and the 990M snapshot
`../db-snapshot/wordnet-umls-aligned-d66-definitions` were built to add the four resources as an
additive layer via `scripts/add-layer-to-snapshot.sh`. That addresses "the class is not on the
chain", but the drift check fires **before** it, on the seed manifest. The approach cannot work for
a change to a bootstrap ontology.

**Both artifacts are dead. Delete them** once the reseed lands:

```bash
git rm ontologies/eigentt/definition-layer.esl
rm -rf ../db-snapshot/wordnet-umls-aligned-d66-definitions
```

## 3. Step 1 — reseed

```bash
scripts/provision-wordnet.sh                       # if references/WordNet-3.0/dict is absent
scripts/reseed-lexicon-db.sh --snapshot-dir wordnet-umls-d66
```

Needs, per that script's header:
- WordNet 3.0 dict at `references/WordNet-3.0/dict`
- UMLS Level-0 META at `references/umls/2026AA/META` (own licence; `scripts/provision-umls.sh`)

It rebuilds the kernel image by default (`--no-build` skips it — **do not** skip; see §6). Budget
~20 min plus the UMLS load; memory profile in `docs/notes/2026-08-03-reseed-memory-profile.md`.

**Take timings while you are there.** Two measurements have been deferred to this reseed since slice
0, both recorded in D66's slice Status blocks:

1. **The witness-index skip.** Slice 0 replaced a materialised per-layer index with direct lookup
   plus a stamped `has_witness_candidates` bit. On a pre-D66 snapshot every layer decodes that bit
   as `true` (the conservative serde default), so the skip has never actually been exercised. A
   reseed stamps it. Compare the demo's rejecting branch against the figures the old code recorded:
   **0.75 s committing, 127 s rejecting** (`kernel/src/layer/witness_index.rs`, pre-slice-0).
2. **Decode cost on the commit path** (D7). Slice 1 added a D47 decode per `canonical_proposition`.
   Record the reseed's own wall-clock as the baseline.

## 4. Step 2 — run the demo

```bash
EIGENIUS_DB_SNAPSHOT="$PWD/../db-snapshot/wordnet-umls-d66" ./demo/prose-to-formulas/run.sh
```

Expected, and unchanged from before D66:

- intact branch: claims commit, `inference.esl` commits, conclusion justified twice
- edited branch: claims commit, `inference.esl` **refused** with
  `qc_validate_justification returned Fails`

What differs is what it takes to get there: **two load steps instead of five**, and **one
`DeclarationTrace` on the branch instead of three**.

Add `--reparse` to re-derive the claims layers from the snapshot. It now emits only
`claims-intact.esl` / `claims-edited.esl`; there are no shape rules or bridges to generate.

### What this run is actually testing

Everything below has only ever been checked offline, against stand-in classes carrying the real
IRIs. This is the first contact with the real WordNet/UMLS resources:

- **Rule 24 against the real lexicon.** It decodes `definition_body` and `check`s it against
  `definition_type`. If `wn:v02203362_t`, `wn:n13440063` &c. resolve to something other than what
  the stand-ins modelled, it fails here.
- **`spec_poly` at a Σ-term.** `inference.esl` instantiates `∀ (m : Set)` at the nested
  compound-kind term for «MSI cancer models». Related: D66 §9 records an undiagnosed oddity —
  `spec_poly` binds `T : Set` yet is applied at `T := Set`, which the universe rules do not
  obviously admit. It worked in the old generated `bridges.esl`, so it should work here, but the
  usage is new.
- **The whole chain committing at all.** Slice 2's near-miss (§6) came from testing the mechanism
  and never the gate.

## 5. Step 3 — finish slice 3

Only after the run is green:

1. **D6 — the naming question.** D66 defers arity and naming to this capstone. Arity is settled
   (ternary, model explicit). Naming is not: `onco-typed.esl` records that `HasActivity` duplicates
   **RO:0002215 `capable of`**, and that grounding is capped by the parser's lexicon (WordNet +
   UMLS), so reaching RO or GO is a lexicon change rather than a rename. Decide and record.
2. **D66 slice 3 Status block** — add one, matching slices 0–2: what was verified, what was not, and
   the measured numbers from §3.
3. **`demo/prose-to-formulas/README.md`** — updated for D66 but not re-read end to end against the
   new run. Its narration and the script's output should agree.
4. **`docs/guides/`** — nothing has been updated for `def`, `eigentt:Definition`, or Rule 24. The
   ESL chapters (`docs/guides/esl/03-lexical-structure.md` keyword tables,
   `docs/guides/esl/11-appendix.md` EBNF) list declaration keywords and will now be wrong.

## 6. Traps found the hard way

**The kernel image is pinned and `up` will not rebuild it.** `docker-compose.yml` carries
`image: eigenius-kernel:local` beside its `build:` stanza, so `docker compose up` reuses whatever
was last built. A demo run exercises **that** kernel, not the working tree — a lexer, validator or
codec change simply is not there, and the failure reads as a bug in the ESL. `cargo test` passing
says nothing about it. `run.sh`'s prerequisites now say so, and
`scripts/add-layer-to-snapshot.sh` now runs `docker compose build kernel` itself — it loads layers
through the kernel *specifically* to validate them, so a stale image made that claim false and could
have baked a rejected layer into a snapshot.

**Test the gate, not just the mechanism.** In slice 2 every test built layers directly and passed,
while `def` could not commit at all: Rule 21 ends in `check_infer`, and a lambda chain has no
inferable type, so every well-formed definition was rejected with `cannot infer type of: Lam(…)`.
Found only when a test finally ran the commit path. The exemption and its three-leg soundness
argument are in D66 slice 2.

**`git checkout <file>` discards uncommitted work.** Used it to undo a bad edit and lost a test that
had not been committed. Commit before experimenting.

## 7. Files slice 3 touched

Rewritten: `demo/prose-to-formulas/{onco-typed,literature-rules,inference}.esl`, `run.sh`,
`README.md`.
Deleted: `demo/prose-to-formulas/{rules,bridges}.esl`, `claims.tsv`.
Removed from the emitter: `emit_shape_rules` and the `--rules-out` / `--citations-out` flags
(`crates/eigenius-encoding/src/{emit,pipeline}.rs`).
Retargeted: `kernel/tests/esl_round_trip.rs` corpus now compiles `onco-typed.esl` and
`literature-rules.esl` in place of the deleted files.

`build_shape_rule` / `ShapeRuleCitation` remain in `crates/eigenius-reasoning/src/grade.rs`, reachable
only from `crates/eigenius-reasoning/tests/shape_rule.rs`. **Open:** whether to delete them. D66 does
not require it; they are now dead outside their own tests.

## 8. Offline evidence already in place

`crates/eigenius-reasoning/tests/witness_hash_agreement.rs` — the slice 3 premise, verified against
the committed demo artifacts:

- `HasActivity(msi, WRN, exonuclease)` unfolds to **exactly** `claim_1`'s parse, and hashes identically
- `RequiresActivity(msi, WRN, helicase)` likewise for `claim_2`
- the emit and check sides agree on the definite description, on its negation, and across binder
  renaming — with a fourth test asserting the comparison is not vacuous

Separately verified: `inference.esl`'s `declared(…)` sub-proof carries the literature rule's
`canonical_proposition` **verbatim** as a subtree, which is the match the witness key depends on.
