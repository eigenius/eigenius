# B2 — arity checking turned on, and what it caught

D76 §8 Phase B deferred B2 as *"verdict-affecting over the whole chain, so it follows the #194/#92
protocol: instrument to log without rejecting, run the suites and the shipped ontologies, count, then
enforce."*

**Run in the stronger form instead, on the user's call:** enforce first, then reseed. With no deployed
users a violation costs a failed reseed and names the offender directly — the load stops and reports
the inductive with its expected and actual counts — where logging-without-rejecting would have
produced a count needing a second pass to interpret.

## What was suppressed

Two dispatch sites tested `decl.indices.is_empty()` as a proxy for *"is this a stub"*, taking a
lenient path when true: every argument treated as a parameter, **no arity check**. A stub had empty
indices — and so does a genuine un-indexed inductive, so `Nat` was indistinguishable from one. **All
ten shipped inductives are un-indexed**, so the lenient path was taken by every inductive application
in the chain. `Nat(x, y, z)` type-checked.

The stub is gone (Phase B), so the conflation went with it. Both sites now check uniformly:
`check_inductive_type_args` for under- and over-application, `Val::app_impl` for over-application
during evaluation.

## Result: nothing breaks

Reseed from `b848461`, `--umls-all`:

| | E2 (`b0127d4`) | B2 (`b848461`) |
|---|---|---|
| resources | 9,439,633 | **9,439,633** |
| loads | 35 | **35** |
| errors | 0 | **0** |
| **arity violations** | — | **0** |
| **all 35 layer hashes** | — | **byte-identical to E2** |

Identical hashes are the strongest form this result could take: the chain is not merely *valid* under
the stricter check but **unchanged**. The narrowing rejected nothing, so nothing was being masked.

**The finding is negative and stated as such:** the suppressed check had no live violations. Importers
generate chain content programmatically and apply type formers correctly; the bootstrap ontologies do
too — including `logic:And(P, Q)`, which is exactly the shape B2 checks.

## What it did catch

**One test, written during Phase B, by the author of the check.**
`positivity::level_slot::a_de_fused_application_reaches_the_same_value_as_the_fused_one` applied
`core:Level` — a former with **no parameters and no indices** — to `Sort(1)`. The leniency swallowed
that argument as a parameter and the test passed, asserting `params.len() == 1` on a type that cannot
take one.

That is the case for the check surviving a zero-violation reseed: the leniency hid a wrong application
from someone who had just read the surrounding code. Rewritten against `List`, with an added assertion
that the nullary former now refuses an argument.

## Hand-authored ESL — the WRN demo, and a correction

An earlier version of this section said the reseed *"says nothing about hand-authored ESL"* and left
it there. That was too weak a claim to leave standing when a sample was available: the WRN demo loads
**five hand-written files** — `claims-intact.esl`, `claims-edited.esl`, `inference.esl`,
`literature-rules.esl`, `onco-typed.esl` — whose argument lists a person typed, including
`reasoning:SpecStr(x, y)`, `reasoning:DeclaredEvidence(…)` and `ontology:compound_kind(x, m)`.

**Run against the B2 kernel: both acceptance checkpoints pass, 0 arity violations.** Intact justifies
twice; edited rejects. So the hand-authored corpus is arity-correct too.

**What that does and does not credit to B2.** Most applications in those files are **constructor**
applications, which route through `check_inductive_ctor_args` and have always had their own arity
handling — a miscount there would have been caught before this change. B2 changed **type former**
applications. So the demo establishes the files are correct; only part of that is the check enabled
here.

The alignment rebuild was skipped, deliberately and not for speed: B2 edits no ontology, the manifest
pin still passes, and all 35 layer hashes matched the E2 run — so the existing aligned snapshot is
both resumable and provably identical in content. Rebuilding it would reproduce it byte-for-byte.

## Parse gate — the third population

The parser builds type applications **programmatically at parse time** — `logic:And(P, Q)` for
coordination, `List(C)` for groups, `cat_n(Σ…, num)` for refined nouns — from combinator rules rather
than reading them from a file. That is a third population, covered by neither the reseed (importer
output) nor the demo (authored files).

`--replay` of the same recorded draw against the same aligned snapshot, so it is directly comparable
to the E2 run:

```
COVERAGE: PASS — every unit parses (grammar-gap 0, missing-lexeme 0)
SELECTION-VALIDITY: PASS — no invalid-adjudicated skeleton was selected
expected-hits 62/62 → 62/62, miss-set unchanged
```

**Every number identical to E2**: encoded 1, ambiguous 41, open 20, total-readings 674, skeletons 171.
0 arity violations, 0 malformed replies.

**Skeletons holding at 171 is the load-bearing one.** A rule assembling a former with the wrong
argument count would turn a silently-accepted reading into a rejected one, showing as *fewer*
skeletons or a grammar gap. Neither moved.

## The result across all three populations

| population | how applications arise | result |
|---|---|---|
| chain content, 9.4M resources | importers, programmatic | 0 violations, all 35 layer hashes identical |
| hand-authored ESL, 5 demo files | a person typed them | 0 violations, both checkpoints pass |
| grammar rules, 62 units | combinators, at parse time | 0 violations, skeletons unmoved |

**Uniformly negative.** The `indices.is_empty()` conflation disabled arity checking system-wide, and
nothing in the system was exploiting it.

## What this still does not establish

Hand-authored ESL beyond these five files and the bootstrap ontologies. Both samples pass; neither is
a corpus.
