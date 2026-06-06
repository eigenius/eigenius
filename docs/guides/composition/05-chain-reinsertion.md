# 5. Chain reinsertion of comorphism outputs

[D14 §9.3](../../design/d14-institution-realisation.md) step 4: the
comorphism's reify output isn't a transport-only return value — it becomes a
**first-class chain resource** that downstream queries can match, gates can
fire on, and provenance can trace back to.

This chapter is the practical reference for how chain reinsertion works
through both the ESL and EigenQL surfaces, why deterministic content-hash
IRIs are the right shape for the default IRI, and what the audit trail
looks like.

## 5.1. Why chain reinsertion matters for composition

A comorphism without chain reinsertion is a function call: input goes in,
output comes back, and the output exists only for the duration of the
caller's scope. That's fine for a one-shot translation embedded inside an
EigenQL FIBER param coercion, where the caller wants the translated value
*as the FIBER input slot* and nothing more.

But many compositional patterns need the output to **persist on the chain**:

- **Multi-step pipelines.** Catalyst → DiffEq → IntervalArithmetic only
  works as a chain of comorphisms if each step's output commits as the next
  step's input.
- **Downstream gates.** AutoOnLoad gates fire on commit. A comorphism that
  produces an `OdeProblem` should be able to trigger DiffEq's
  `validate_solution` gate on the produced problem the same way a
  hand-authored `OdeProblem` would.
- **Audit and replay.** A Verdict's `verdict_subject` points at the gated
  resource. If that resource is a comorphism output that was never
  committed, the audit trail has a dangling reference.
- **Cross-fibre identity.** When two callers run the same comorphism with
  the same input, they should see the same output — at the same IRI. The
  chain's role as the source of truth depends on this.

Chain reinsertion is what closes those needs. Phase 19i wired both surfaces
through the same `commit_with_validation` machinery; the result is that a
comorphism-produced resource is indistinguishable, from the chain's
perspective, from a hand-authored resource of the same class.

## 5.2. ESL surface: `comorphisms:foo(input)` → `Exp::InstitutionInvoke`

In ESL, a comorphism is invoked as a qualified-name function call in
expression position. The compiler classifier resolves the qualified name
through the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs);
if it classifies as a `Comorphism`, the call lowers to:

```rust
Exp::InstitutionInvoke {
    comorphism_iri: <full IRI>,
    source: <the argument expression>,
    target_iri: None,                  // None → kernel mints a content-hash IRI
}
```

Example from the kinase notebook (cell 30):

```esl
namespace symbolics   = "urn:eigenius:symbolics";
namespace jump        = "urn:eigenius:jump";
namespace comorphisms = "urn:eigenius:comorphisms";
namespace nb          = "urn:eigenius:notebook:kinase_demo";

program nb:produce_problem
    : symbolics:SymbolicsToJuMPInput -> jump:OptimisationProblem
{
    comorphisms:symbolics_to_jump(input)
}
```

Running this program against `nb:fit_input` (cell 31's program-run cell)
produces an `OptimisationProblem` and commits it to the chain at a
deterministic content-hash IRI (§5.4 below). The program's return type
(`jump:OptimisationProblem`) is statically checked against the comorphism's
target class — if the user wrote `program nb:produce_problem : ... -> jump:OdeProblem` (the wrong target), the type checker would reject it.

The ESL form is right for compositional code that *consumes* the
comorphism's output downstream — the program's return value is the
committed resource, accessible via its content-hash IRI from the next cell
or the next program.

## 5.3. EigenQL surface: `FIBER ... AS ?var INTO "<iri>"`

In EigenQL, the same chain-reinsertion semantics are available via the
`INTO` keyword on a `FIBER` clause:

```eigenql
USING INSTITUTION "urn:eigenius:institutions:symbolics" AS symb

FIBER symb:"urn:eigenius:symbolics:query_classes:qc_symb_to_jump" {
    "urn:eigenius:symbolics:objective":               "urn:eigenius:notebook:kinase_demo:sse_expr",
    "urn:eigenius:symbolics:variable_names":          ["Ki"],
    "urn:eigenius:symbolics:framing_variable_bounds": ["urn:eigenius:notebook:kinase_demo:ki_bound"],
    "urn:eigenius:symbolics:sense":                   "Min"
} AS ?problem INTO "urn:eigenius:notebook:kinase_demo:fiber_problem"
RETURN [] { iri: ?problem }
```

The dispatch is identical to a plain `FIBER ... AS ?var` clause (FIBER
calls the OnDemand handler, gets back a typed response). The difference is
**chain reinsertion**: the response resource commits to the regular chain
at the named IRI, rather than living in a transient overlay. Both surfaces
share the same `commit_with_validation` machinery — AutoOnLoad gates bound
to the response's class fire on commit; the response is reachable by every
subsequent query; the audit trail (Verdict + RuntimeInvocation) is the
standard one.

Without `INTO`, the response stays overlay-only (the v0 behaviour, kept
for queries that don't want side effects). With `INTO`, the caller picks
the IRI explicitly — useful when the IRI needs to match a known external
identifier (e.g. an enterprise system's deterministic key for the
resource).

The EigenQL form is right for *interactive* composition: a notebook author
or a dashboard query that wants to run a comorphism live, pin the result at
a known IRI, and have downstream queries find it by that IRI.

## 5.4. Deterministic content-hash IRIs

When the ESL form omits `target_iri` (the common case), the kernel mints
the IRI deterministically:

```
urn:eigenius:comorphism-output:<comorphism-tail>:<hex16>
```

where `<comorphism-tail>` is the last colon-separated segment of the
comorphism's IRI (e.g. `symbolics_to_jump`) and `<hex16>` is the first 16
hex characters of SHA-256 over the canonical Eigon-CBOR of the produced
resource (with `@id` cleared so the hash is over the *content*, not the
IRI).

Two properties this gives:

1. **Idempotence.** Running the same comorphism with the same input
   twice produces the same content (because the comorphism is a EigenTT
   evaluation — pure modulo the institution handlers' purity), which
   produces the same IRI. The second run dedupes to the first.
2. **Cross-fibre identity.** Two different callers running the same
   comorphism with structurally-identical inputs see the same IRI. The
   "which resource is this?" question has a single answer; the chain is
   the source of truth.

Property (1) is what makes chain reinsertion safe — re-running a notebook
cell doesn't proliferate near-duplicate `OptimisationProblem` resources;
the second run finds the first one already there.

Property (2) is what the Grothendieck construction wants. A comorphism's
output, viewed as a fibre over its input, has a canonical representative
on the chain — not a fresh "copy" each time. Cross-cutting queries that
ask "what comorphism outputs do I have?" get a clean answer.

The hex tail is 16 characters (64 bits of entropy) — enough to make
collisions astronomically unlikely (~10^-10 with millions of resources)
without bloating IRIs to full 64-character SHA-256 hex. If collisions
matter for your use case, the EigenQL `INTO` form lets you pick a longer
or domain-specific identifier.

## 5.5. Audit trail: `Trace::Comorphism` + `RuntimeInvocation`

Chain reinsertion produces three audit artifacts per dispatch:

1. **The reified resource itself** — at the content-hash IRI (or
   caller-named IRI under `INTO`).
2. **A `Trace::Comorphism` audit variant** on the program trace (or query
   trace) of the invoking call. From
   [`kernel/src/program/trace.rs`](../../../kernel/src/program/trace.rs):

   ```rust
   Trace::Comorphism {
       comorphism_iri: String,
       source_trace:   Option<Box<Trace>>,
       target_iri:     String,
       target_class:   String,
   }
   ```

   `comorphism_iri` names which bridge ran; `source_trace` carries the
   trace of the source-expression evaluation (so the audit walk can
   reconstruct what produced the input); `target_iri` and `target_class`
   identify the produced resource on the chain.

3. **A `RuntimeInvocation`** if either institution involved is
   external-runtime hosted (D31 §6.3). Carries the env image digest, the
   numerical metadata (BLAS / FMA / determinism flags) captured at worker
   bootstrap, the started_at / completed_at timestamps, and the per-method
   provenance for the substrate-side dispatch.

Together these three give a complete chain-resident answer to "where did
this resource come from?" Walking back from the resource: its
`Trace::Comorphism` event names the comorphism; the comorphism resource
names its three constituent pieces (export, transformation, import); the
RuntimeInvocation pins the exact runtime that executed each handler. A
re-run on the same image with matching `numerical_metadata` is reproducible
by construction.

## 5.6. Worked example: the kinase notebook's Part C end to end

Cells 29–35 of the kinase notebook walk through both surfaces in sequence.
This section traces the flow.

### ESL program form (cells 29–32)

**Cell 29 (markdown).** Frames the whole part: "Part B's cell 20
hand-committed an `OptimisationProblem`; Part C runs the comorphism live."

**Cell 30 (ESL).** The wrapper program:

```esl
program nb:produce_problem : symbolics:SymbolicsToJuMPInput -> jump:OptimisationProblem {
    comorphisms:symbolics_to_jump(input)
}
```

Compiles to `Lam(input, InstitutionInvoke(symbolics_to_jump, input))`. The
return-type annotation `jump:OptimisationProblem` is checked against the
comorphism's import-side target class at compile time — if they didn't
match the program would refuse to compile.

**Cell 31 (program-run).** Invokes the program with `nb:fit_input` as the
input. The kernel:

1. Resolves `comorphisms:symbolics_to_jump` to its triple.
2. Calls Symbolics's `extract_typed` on `nb:fit_input` → gets the SSE
   FormulaTerm.
3. Applies the identity transformation Component → FormulaTerm unchanged.
4. Calls JuMP-HiGHS's `reify` → produces a fresh `OptimisationProblem`.
5. Computes the content-hash IRI:
   `urn:eigenius:comorphism-output:symbolics_to_jump:<hex>`.
6. Commits the produced resource to the chain at that IRI.
7. Returns the IRI as the program's value.

**Cell 32 (EigenQL).** Verifies the produced resource exists:

```eigenql
MATCH "urn:eigenius:jump:OptimisationProblem"(?p) {
    "urn:eigenius:jump:sense":          ?sense,
    "urn:eigenius:jump:variable_names": ?vars
}
WHERE ?p LIKE "urn:eigenius:comorphism-output:symbolics_to_jump:%"
RETURN [] {
    iri:   ?p,
    sense: ?sense,
    vars:  ?vars
}
```

Returns one row — the comorphism-produced `OptimisationProblem`, identified
by its content-hash IRI prefix. The `MATCH` pattern uses only the
properties JuMP-HiGHS's `reify` actually sets; including `core:short_name`
in the pattern would AND-filter to zero rows because substrate-produced
resources don't carry one.

### EigenQL `FIBER ... INTO` form (cells 33–35)

**Cell 33 (markdown).** Introduces `INTO` as the interactive surface for
the same chain-reinsertion semantics.

**Cell 34 (EigenQL).** The FIBER + INTO call:

```eigenql
FIBER symb:"urn:eigenius:symbolics:query_classes:qc_symb_to_jump" {
    ...
} AS ?problem INTO "urn:eigenius:notebook:kinase_demo:fiber_problem"
RETURN [] { iri: ?problem }
```

Calls the operational backing of the comorphism — the OnDemand
`qc_symb_to_jump` QueryClass — and pins the produced `OptimisationProblem`
at a *caller-named* IRI (`fiber_problem`).

**Cell 35 (EigenQL).** Inspects the FIBER-INTO result:

```eigenql
MATCH "urn:eigenius:jump:OptimisationProblem"(?p) {
    "urn:eigenius:jump:sense":           ?sense,
    "urn:eigenius:jump:variable_names":  ?vars
}
WHERE ?p = "urn:eigenius:notebook:kinase_demo:fiber_problem"
RETURN [] { iri: ?p, sense: ?sense, vars: ?vars }
```

The exact-match WHERE filters to the one IRI the previous cell pinned. The
returned row has the same shape as cell 32's content-hash row — confirming
the two surfaces produce structurally-identical resources, just at
different IRIs.

## 5.7. Idempotence and the cross-fibre identity property

Two notes worth flagging for compositional reasoning:

**Idempotence.** Re-running cell 31 against the same `nb:fit_input` produces
*the same* `OptimisationProblem` content, which hashes to *the same*
content-hash IRI. The second commit dedupes to the first; no proliferation,
no chain bloat. This makes notebook authoring forgiving — cells can be
re-run without polluting the chain.

The dedup works because:
1. The comorphism's transformation is a EigenTT evaluation, which is pure.
2. The institution handlers (`extract_typed` / `reify`) are required to be
   pure functions of their inputs (D14 §8); deterministic-host runs hash
   to the same bytes.
3. The IRI is a function of the produced bytes, so equal bytes → equal IRI.

**Cross-fibre identity.** Two callers running the same comorphism on the
same input — say, a notebook user and a TypeScript program against the
same kernel — see the *same IRI*. Subsequent queries asking "do we have a
comorphism output for this input?" get a clean yes/no answer.

This is the property the Grothendieck construction wants: the fibres of
the comorphism (the comorphism viewed as an indexed family of mappings)
have a canonical representative on the chain, not a per-caller copy. The
chain becomes the canonical record; the comorphism is the canonical
mapping; together they realise the categorical structure D14 sketches.

The `INTO` form opts out of this when the caller wants a custom identifier
— useful when the produced resource needs to match an external system's
key, but loses idempotence (re-running the same `INTO` overwrites or
errors, depending on validator state).

---

Next: **[6. Walkthrough: reading the kinase notebook end-to-end →](06-kinase-walkthrough.md)**
