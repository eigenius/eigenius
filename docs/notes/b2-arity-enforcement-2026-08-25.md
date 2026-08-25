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

## What this does not establish

The reseed covers **chain content**, which is machine-generated. It says nothing about hand-authored
ESL, which is where a human writes an argument list and is the population most likely to contain a
miscount. The bootstrap ontologies are the sample of that population, and they pass — but they are a
few hundred declarations, not a corpus.
