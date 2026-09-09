# D89 rename — execution plan

`2026-09-08`. Companion to [`../design/d89-the-justification-vocabulary-refactoring.md`](../design/d89-the-justification-vocabulary-refactoring.md),
which decides *what* changes. This decides *how*, per substitution, and in what order.

Surface measured `2026-09-08` against the working tree.

## 0. What the survey found

| identifier | occurrences | full-IRI | prefixed | ontologies | artifacts | src | tests | docs |
|---|---|---|---|---|---|---|---|---|
| `justification:Certificate` | 311 | 17 | 294 | 3 | 19 | 26 | 14 | 27 |
| `reflection:canonical_proposition` | 262 | 17 | 245 | 7 | 33 | 14 | 13 | 30 |
| `justification:Conclusion` | 191 | 13 | 178 | 2 | 22 | 11 | 11 | 20 |
| `spec_poly` | 185 | 0 | 185 | 1 | 8 | 5 | 7 | 23 |
| `justification:Claim` | 158 | 7 | 151 | 1 | 20 | 3 | 3 | 13 |
| `justification:judgement` | 93 | 10 | 83 | 2 | 15 | 4 | 3 | 12 |
| `justification:subject_iri` | 66 | — | — | 1 | 15 | 0 | 2 | 8 |
| `justification:proof` | 38 | 1 | 37 | 2 | 1 | 1 | 1 | 14 |
| `justification:refutes` | 7 | — | — | 1 | 0 | 0 | 0 | 4 |

Three facts decide most of the technique choices.

**No renamed token is a prefix of another live token.** The full `justification:*` inventory is
`Certificate, Claim, Conclusion, ConsistencyRequest, Declared, EntailmentRequest, Projection, Term,
app, certificate, counterfactual_iri, declared_grounds, fully_verified, judgement, observed_grounds,
proof, proposition, refutes, subject_iri, subject_sentence, support_count, survives_without, term,
verified_grounds`. `proof` is not a prefix of any of them; neither is `judgement`, `Claim` or
`Conclusion`. So a plain substring replace on the **prefixed** form is safe, and it covers the
full-IRI form too, since `urn:eigenius:justification:Certificate` contains `justification:Certificate`.
One rule per identifier, not two.

**Several names in that inventory are not declared anywhere** — `justification:proposition`,
`justification:Assertion`, `justification:certificate`, `justification:Projection`,
`justification:Term`. They survive only in Rust comments and `prov.esl` prose. `proposition` is
therefore free to take, and the comments naming it are stale and should be corrected in the same
pass. See §2.

**The notebooks are test-covered now.** `kernel/tests/stats_notebook_cells_compile.rs` and
`crates/eigenius-lean/tests/notebook_fixture_test.rs` compile their cells. D88's rewrite recorded the
notebook as *not* compiled by the suite, and that is no longer true — every authored ESL surface in
the tree is reached by some test.

## 1. Technique per substitution

### T1 — substring replace on the prefixed form

Safe because of the no-prefix-collision fact above. Applies to five identifiers:

| from | to | occurrences |
|---|---|---|
| `justification:Certificate` | `justification:Grounds` | 311 |
| `justification:Claim` | `justification:Declaration` | 158 |
| `justification:judgement` | `justification:grounds_judgement` | 93 |
| `justification:proof` | `justification:proof_judgement` | 38 |
| `reflection:canonical_proposition` | `justification:proposition` | 262 |

Run per file class, not tree-wide, so the docs policy (§4) can differ. Verify by residue grep: after
the pass, no occurrence of the old token outside `docs/design/` and `docs/notes/`.

The one wrinkle is `justification:Certificate` in *prose*, where "certificate" is also an ordinary
noun ("a certificate records grounds"). The replace only touches the qualified token, so prose
survives untouched and reads wrong afterwards — `Grounds(P)` described as "the certificate". Prose
needs a human pass, sized at 27 doc files and the ontology comments.

### T2 — paren-aware transformer

`spec_poly` → `instantiate` **and** deletion of its first two arguments, now implicit. 10 call sites
in 6 files. The rename half is a T1-style replace on a unique token; the argument deletion is not,
and it is the same shape D88 used for the certificate merge: an arity survey first, then a
paren-aware transformer deleting argument spans **right-to-left** so earlier spans keep their
offsets, leaving formatting and comments untouched.

Arity survey to run first, as D88's did: confirm all 10 sites are at arity 4 and that every
off-pattern `spec_poly(` occurrence is inside a `//` comment. Two of the six files
(`universal_rule.esl`, `spec_poly_set_domain.esl`) exist only to test the constructor and are
renamed with it.

### T3 — Rust identifiers, compiler-verified

`kernel/src/ontology/well_known.rs` constants (`CANONICAL_PROPOSITION` and the
`justification:*` IRIs), plus the stale names from the last two renames:
`emit_from_reasoning_sentence` and 17 occurrences of `sentence` in
`kernel/src/layer/witness_admission.rs`, named for a `justification:Sentence` class and a
`reasoning:` namespace that no longer exist. Rename these with the compiler as the check; nothing
here needs a script.

### T4 — declaration edits, by hand

Small, high-attention, one file each:

- `ontologies/justification/justification.esl` — the renames above plus the new `requires` clauses,
  `instantiate implicit(T, P)`, and deletion of `subject_iri` and `refutes`.
- `ontologies/encoding/encoding.esl` — `enc:EncodedClaim` subclasses `Declaration`.
- `ontologies/reflection/reflection-ontology.json` — remove `canonical_proposition`, which moves
  namespace.

### T5 — deletions from instances

`justification:subject_iri` (66 occurrences, 15 artifact files) and `justification:refutes` (7, none
authored) are dropped, not renamed. Line-wise deletion, and **not** by bare-token matching:
`subject_iri` is also an ordinary local variable name in `kernel/src/layer/index.rs` and
`kernel/src/query/vector/indexing.rs`. Match `justification:subject_iri` only.

### T6 — semantic reclassification, by hand

Two `justification:Claim` instances in the tree carry no `prov:was_attributed_to`:
`kernel/tests/fixtures/universal_rule.esl:82` (`screen:m_eig0291`) and
`kernel/tests/fixtures/spec_poly_set_domain.esl:53` (`poly:obs`). Both are cited by `observed(…)`.

**They are exactly the two the refactoring reclassifies.** D89 puts no class on the observed side, so
these become plain measurement resources rather than `Declaration`s. 75 of 77 Claims rename cleanly
and the 2 that do not are the 2 that should not have been Claims — the corpus validates the new
requirement set rather than fighting it.

## 2. Settled: `Conclusion` gets no proposition slot

43 `justification:Conclusion` instances exist and **2** carry a proposition. An earlier draft of
D89 §2 gave `Conclusion` a slot so every container would bear `P` the same way, which meant authoring
the value at **41 sites**. Reading the code settles it against that.

`target_proposition_hash` (`kernel/src/layer/witness_admission.rs:459`) tries the explicit
`canonical_proposition` slot, then — **gated on `is_a` containing `justification:Conclusion`** —
projects `P` out of the judgement, then falls back to `Asserts(target_iri)`. The class test at line
471 is the defect: a reader dispatching on class to locate a property, in an ontology whose premise
is that warrant-bearing is a property. Adding a slot would make that test redundant; it would not
make it wrong-headed, and it would buy the removal with 41 authored copies of `P` plus an equality
rule to keep them from drifting.

Nor can the two emitters simply share one lookup. `emit_from_trace` reads a **grounds** judgement,
whose type is `Grounds(P)` and needs unwrapping; `emit_from_reasoning_sentence` reads a **proof**
judgement, whose type is `P` already. Those are different unwrappings, so a single merged arm would
be wrong.

**Decision: replace the class test with property dispatch, add no slot.** One accessor, tried in
order — `justification:proposition`; else `grounds_judgement` unwrapped through `Grounds(P)`; else
`proof_judgement` taken as `P`; else the `Asserts(iri)` default. `emit_from_reasoning_sentence`
becomes a caller rather than a sibling path, which is what G3 asks for.

Cost: **zero authored sites**, no second copy of `P`, no equality rule, no commit-time mutation, and
the class test is deleted rather than left redundant. This is a one-function change in
`witness_admission.rs` plus its tests, and it belongs in step 3 of §3 below rather than in the
artifact pass.

## 3. Order

Renames and structural edits touch the same 60-odd files, so the aim is one pass per file.

1. **Declarations** (T4) — the ontology is the source of truth and everything else is checked against
   it.
2. **Rust** (T3, plus §2's accessor) — compiler-verified, and it fails loudly if step 1 missed
   something. The property-dispatch accessor lands here, before any artifact is touched, so the
   artifact pass is a pure rename.
3. **Mechanical renames** (T1 + T5) across ontologies, artifacts, src, tests. One script, one commit,
   residue-grepped.
4. **`instantiate` transformer** (T2) — separate commit, because it is the only step that changes
   argument counts and it wants its own review.
5. **Reclassification** (T6) — 2 fixtures by hand.
6. **Prose and living docs** (§4).
7. **Reseed and gates** (§5).

Steps 3 and 4 are separate commits deliberately: a textual rename that compiles is easy to review as
a diff; an argument deletion is not, and mixing them hides the second in the noise of the first.

## 4. Docs policy

`docs/design/` and `docs/notes/` record intentions at the time and are **not** rewritten — renaming
inside D48, D73, D87 or D88 would falsify the record of what was decided when. D89 carries the
mapping, and that is the pointer a reader follows.

Living documentation is rewritten: `docs/guides/` and `docs/method/`, sized at 14 files for
`Certificate`, 10 for `canonical_proposition`, 7 for `Claim`, 2 for `spec_poly`. Plus
`docs/notes/work-stack.md` and `next-steps-after-d88.md`, which describe current work rather than
past decisions — the latter's C1 entry needs the rewrite D89 §5a describes.

## 5. Verification

| stage | what catches a mistake |
|---|---|
| declarations | `cargo test -p eigenius-kernel` bootstrap + manifest tests |
| Rust renames | the compiler |
| artifact renames | the validator at commit, via the fixture-compiling suites |
| WRN chain | `wrn_phase3` / `wrn_phase5`, which assert grounds reach `verified` with witnesses chain-resident |
| notebooks | `stats_notebook_cells_compile.rs`, `notebook_fixture_test.rs` |
| `instantiate` arity | `universal_rule.esl`, `spec_poly_set_domain.esl`, and every certificate that type-checks |
| residue | grep for each old token outside `docs/design` and `docs/notes` |

Then the reseed: `CARGO_FEATURES=use-llm scripts/reseed-lexicon-db.sh --umls-all`, the alignment
snapshot, and `measure-parse-rate.sh` against the committed baselines. The bootstrap manifest moves,
so nothing persisted is resumable until it runs — the same position A3 described before B4.

`instantiate implicit(T, P)` needs the unifier work of D89 §5a, which is still a prototype with an
open naming fragility. If that is not ready, land `instantiate` with all four arguments explicit and
add `implicit(T, P)` later — it is a declaration-only change, but it moves the manifest, so deferring
it costs a second reseed.
