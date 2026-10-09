# D100 — Certifying the Mayo report: three claims against an existing ledger

*Design note. The plan for UAB round 1, survey step 3 — the first rung of the collaborator's own
item 3, "certify the Mayo report in Eigenius, against a ledger that already exists", which he names
as the one he would most like to do. Written `2026-10-09`. It changes no code.*

## What the material is

`experiments/uab/UAB Round 1/05-certification-log/` holds a complete, replicating certification of a
~25-page clinical report: **475 claims**, each with a declared kind and a certificate recording
method, evidence and verdict, over three passes.

| claim kind | count | | entry kind | count |
|---|---|---|---|---|
| `observed` | 239 | | `extracted` | 475 |
| `declared` | 80 | | `certified` | 441 |
| `derived` | 74 | | `retired` | 48 |
| `marked` | 49 | | `rejected` | **25** |
| `compressed` | 20 | | `note` | 20 |
| `exempt` | 13 | | `blocked` | 6 |
| | | | `replicated` | 4 |
| | | | `overturned` | 2 |

The 25 rejections and 6 blocks are the part that matters. The collaborator's own denominator:
**nineteen of twenty-five first-pass rejections sat on numbers that re-derived exactly.** Every
database tally, score and identifier survived. What failed were *labels, quantifiers and scope
words*. His reading of that, which this note adopts: "the computational core verifies; the failures
cluster at the institutional bridge where a verified statement becomes a domain claim."

## Decision 1 — three claims, not one

The survey's §4 step 3 names C-0020 alone. **That tests the half that already works.** C-0020 is an
`observed` claim whose count re-derives (`exome.ac_hemi + genome.ac_hemi = 2 + 2 = 4`), reconciled
independently against the gnomAD v4.1 release VCFs. Landing it proves the machinery and, by the
denominator above, is expected to succeed.

So three rungs, chosen because each exercises a different axis and two of them are *recorded
failures with their causes*:

**C-0020 · `observed` · certified.** «p.Ile380Val is carried by 4 hemizygous males.» A gnomAD GraphQL
sweep, five sha256-archived files, four `observed_at` timestamps, an independent VCF reconciliation,
and a `reading` field naming subject and label. The rung that should work.

**C-0011 · `marked` · rejected → overturned.** «Ile380Thr is very likely tolerated, and — more
decisive than that — the risk it carries is bounded rather than open-ended.» Its certificate:

```
premise_failure: C-0100 (rejected: gnomAD cannot establish health), C-0024 (blocked on 'ever')
reading: Two limbs, and they fail differently. 'Bounded rather than open-ended' rests on
         hemizygosity (C-0013 certified) and loss-of-function mechanism (certified), and it HOLDS.
         'Very likely tolerated' rests on …
verdict: rejected
E3 · overturned · cause: amendment
     superseded_premises: C-0023 -> C-0398; C-0024 -> C-0399; C-0100 -> C-0406
```

This is the gap the collaborator names first: *"There is no verdict for 'the inference is sound but a
premise failed.' I hit that twice and had to route around it."* He routed around it by adding a
`premise_failure` FIELD rather than a verdict. The inference is sound, one limb holds, and the claim
is rejected because a premise of the other limb is not established. That is what
`justification:Conclusion`'s two-judgement split is for — `grounds_judgement` =
`holds(kernel, c, Grounds(P))` against `proof_judgement` = `holds(logic, t, P)`.

**C-0078 · `compressed` · rejected → retired.** The section heading «Finding 2 — Healthy hemizygous
men carry substitutions here, and ClinVar calls one of them Benign»:

```
compresses: C-0080, C-0082, C-0083, C-0084, C-0086, C-0087, C-0089, C-0091, C-0092, C-0093
            (the Finding 2 table rows), with C-0018/C-0019 … and C-0100 supplying 'healthy'
qualifiers: 'that call is single-submitter with criteria provided, which is one star. Do not
            present it to a reviewer as settled expert consensus'
reading:    Rejected under the compression rule (one dropped qualifier is a rejection), NOT on any
            number: every ClinVar value the heading compresses is certified in this pass
verdict:    rejected
```

A heading rejected for dropping exactly one qualifier, with all thirteen claims beneath it
certified. This is the collaborator's third-pass finding — "a section heading that pointed the wrong
way while every number underneath it was correct… The heading was a judgment about what the numbers
establish. No amount of re-verifying the numbers would have caught it" — and the survey's §3 records
that Eigenius has **no counterpart at all** for `compressed`. So this rung's deliverable may be the
gap rather than the landing, which is the survey's stated goal and the half experiment 02's README
calls "probably the more useful half".

## Decision 2 — read the log directly

The log is the input, unmodified: `claims/*.md` with YAML front matter (`id`, `document`,
`first_seen`, `location`, `text`, `kind`, `depends_on`, `supersedes`) and `## E<n> · <entry-kind> ·
<pass> · <date> · <agent>` sections. No import step and no transformation. Two reasons: it
replicates offline today and that property should not be disturbed, and a transformation would make
every later disagreement ambiguous between our reading and our rewriting.

## Decision 3 — the grade is computed, not asserted

The survey's §3 says "*Exists:* the epistemic lattice the analyst note reaches for, with concrete
obligations (`d81-the-epistemic-stack.md:275`): `DeclaredResource` requires `declared_by`;
`ObservedResource` requires `source` …". **Those four classes were REMOVED, deliberately**, in #239
(`c4a4eb7`), whose own comment gives the reason: "that is the separation the grade classes collapsed:
`DeclaredResource` and `ObservedResource` made the ground a KIND OF RESOURCE."

`judgements-and-warrants.tex` §"Deprecated Architectural Patterns" governs here, and it deprecates the
pattern outright:

> **Grades assigned by class membership, by a trace declaring its own grade, or by the importer that
> wrote the resource.** Replaced by computation from stored evidence. **No path exists by which
> asserting a class confers evidential standing.**

> **A verified resource class declared a subclass of a derived resource class.** Replaced by the
> two-axis separation.

So there is no "land it as an `ObservedResource`" to do, and reinstating those classes so a validator
could enforce D81's obligations would restore the deprecated pattern. D81 is an interpretation of the
paper; the paper governs.

**What ships instead, and it is a better answer to the collaborator's complaint.**
`justification:Grounds : Prop -> Type 2` — "an inhabitant of `Grounds(P)` IS the justification term",
with `declared`, `observed` and `verified` at its leaves, `app` for Artemov application, `sum_l`/`sum_r`
and `instantiate` above them. Two properties decide this plan's shape:

- **The witness is synthesized, not written.** `declared`/`observed`/`verified` each consume a
  `witness:Is*As` "that the kernel synthesizes from the layer's traces rather than from anything an
  author writes". We do not assert that C-0020 is observed; we commit its evidence with provenance and
  the kernel computes the ground.
- **`Grounds(P)` does not assert `P`.** "No rule turns `Grounds(P)` into `P`." That is exactly the
  seam the collaborator's failures sit on — 19 of 25 rejections had sound numbers and broken labels,
  quantifiers and scope. A verified computation grounding a domain claim is `Grounds(P)`; the domain
  claim is `P`; the bridge is a `justification:Declaration`, which "REQUIRES BOTH halves", the
  proposition and the attribution — and the note records that "requiring only the first is what the
  predecessor class did".
- **An observed ground cites the measurement resource itself.** `justification:Declaration` is
  explicitly "NOT a container for observations: an observed ground cites the measurement resource
  itself — a SampleSet, an ingested file — which carries its own domain structure and needs no class
  here." So C-0020's five sha256-archived files are committed as what they are, and the ground cites
  them.

**The shape for each rung, therefore:** commit the claim's proposition and its evidence as chain
resources with provenance; let the kernel synthesize the witness; build the `Grounds` term; and keep
the proposition itself unasserted until something bridges it. This is "a kernel checks what a Makefile
currently checks" in the paper's own terms, and it needs no bootstrap edit and no reseed — which also
removes the (a)-versus-(b) trade I had wrongly posed here.

## What this measures

The deliverable is not three landed claims. It is the **divergence** between the kernel's verdict and
the ledger's, on claims whose true status was established three times over:

1. **C-0020** should agree — `certified` both ways. A disagreement here is a defect in our encoding,
   not a finding.
2. **C-0011** should expose the missing vocabulary. Does the `Grounds(P)` / `P` split let us say
   "sound inference, failed premise" as a *type* rather than a field? And does one limb holding while
   the other fails come out as a conjunction with a per-conjunct grounds verdict?
3. **C-0078** should expose the hole. `compressed` has no counterpart; the question is what it would
   take — plausibly an entailment obligation from a heading to the conjunction of what it compresses,
   plus a no-dropped-qualifier side condition. Naming that precisely is the result.

## Not in scope

- The other 472 claims. This is three rungs, not a corpus run; the corpus run is the survey's step 4
  and wants the parse-coverage measurement first.
- Error injection (the collaborator's item 4), which depends on this.
- The real-patient case, blocked on IRB with affiliate-faculty status offered as a route.
- `exempt`, which has no Eigenius counterpart either and no claim among the three.

## Answered

- *"Is item 1 a morning's work, or does it want the rewrite?"* — done and merged as **#267**
  (`a8d3e45`); `ontologies/variant/variant.esl` carries `NucleotideAllele`/`ProteinAllele` and a
  `TranslatesTo` documented as non-injective. The collaborator already knows.
- *"Would you take my certification log as-is?"* — yes, directly, per Decision 2.
