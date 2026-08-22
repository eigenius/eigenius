# D68 — claim kinds: the two-axis claim model (gates D67 slice 5)

**Status: BUILT (2026-08-12) — §7 records the implementation and the measured close-out; the
design sections stand as written (one §5 refinement noted there).** Parent: [d67-pipeline-unification.md](d67-pipeline-unification.md)
§8 (the claims-ontology question raised in review, 2026-08-11) and §4 (claim antecedents).
Settled inputs: «These findings» refers to prior claims of the same document (user, 2026-08-11);
plural demonstratives get REAL set antecedents, not a most-recent-single approximation (user
decision, same review); the binding target is the reified claim resource (the proposition's
carrier — a bare `Prop` cannot fill an Entity slot).

## 1. The two axes

- **Epistemic source** — how the claim is warranted: the existing `reflection:` lattice
  (`{Declared,Observed,Derived,Verified}Resource`), the trace classes, the `JustificationTerm`
  constructors. Unchanged by this note.
- **Discourse kind** — what the claim IS in the document's own terms: a finding, an
  observation, a classification, a hypothesis, a suggestion, or a plain assertion. This is the
  axis the corpus refers BY («these findings» 19/20/49, «These observations» 60, «These
  classifications» 45) and marks on the way in («We **hypothesized** that…» 9/35). Orthogonal
  to source: a hypothesis and a finding both land Derived.

D67 §8's finding stands: one axis cannot carry both. `enc:EncodedClaim ⊑ finding-class` would
type every claim — including the hypotheses — as a finding, and the checker would admit «these
findings» → a hypothesis. Wrong resolutions must stay INEXPRESSIBLE, not merely unranked.

## 2. Kind is a second `is_a` class, not a property

The mechanism already exists: the checker's intensional inhabitation rule consults the **full
`is_a` array** (#91 — multi-class individuals type against EACH of their classes). So a landed
claim carries BOTH axes as classes:

```
is_a = [ enc:EncodedClaim, enc:Finding ]
```

and «these findings» (restrictor = the lexicon's *finding* class) resolves iff `enc:Finding`
subsumes into that restrictor — the existing subsumption walk, **no new kernel rule and no
reshaping of the reflection lattice**. A kind-as-property (`enc:claim_kind = …`) is invisible
to the checker and would need a new rule; rejected.

New vocabulary in `encoding.esl` (chain-loaded, no reseed):

- **`enc:Claim`** — the abstract discourse-referable claim root. This also answers §8's
  "missing root" finding at the `enc:` level: the root the source lattice lacks is named HERE,
  where discourse needs it, without touching `reflection:` (whose per-source classes keep their
  required-field roles; the three-representation redundancy of the source axis is recorded as a
  separate cleanup, not blocking).
- **The kind classes**, each `subclass_of enc:Claim`: `enc:Finding`, `enc:Observation`,
  `enc:Classification`, `enc:Hypothesis`, `enc:Suggestion`, `enc:Assertion`. A CLOSED set in
  the same spirit as the enumerations (adding a kind is a vocabulary edit); chosen from the
  corpus's own reference nouns + the two matrix-frame verbs, with `Assertion` the unmarked
  default.

## 3. The lexicon alignment is per-kind, curated, and lives in its own layer

The bridge facts — `enc:Finding subclass_of <the lexicon's finding sense-class>`, one per kind
— reference WordNet sense classes, which exist only on the SEEDED chain. So they cannot live in
`encoding.esl` (whose validation chain has no lexicon data) and must not be bootstrap (no
reseed). They go in a small **`ontologies/encoding/claim-kind-alignment.esl`**, loaded onto the
doc/interactive chain after the lexicon — the same pattern as the wordnet↔umls alignment
layers. The exact target sense-classes are picked at implementation against the page's actual
restrictors (the demonstrative holes name them), one reviewable `subclass_of` fact per pair;
`enc:Assertion` is deliberately aligned to NOTHING — an unmarked claim is not discourse-referable
(fail-closed). `encoding_validates` extends to pin the kind lattice; a second chain-level check
validates the alignment file over a seeded fixture.

## 4. Kind assignment at landing

Where the kind comes from, per claim:

1. **Matrix-frame evidence (deterministic).** A sentence whose parse is headed by a marked
   frame — `hypothesize(P, we)`, `suggest(P, x)`, `show(P, x)` — carries its kind on its
   sleeve; the lander maps frame → kind (`hypothesize` → Hypothesis, …) from a small closed
   table. Note the object of study: units 9/35 land as Hypothesis via this route.
2. **The recorded classifier (untrusted) for unmarked declaratives.** Units 13–18 — the plain
   experimental results «these findings» refers to — have no frame; something must call them
   findings. That is a judgment, and it gets the same treatment as every other judgment in this
   pipeline: an LLM classifier behind a trait, in document context, RECORDED (a
   `kinds.json` sibling of ranks/selections/proposals; replay-only in artifact generation),
   with the assignment emitted on the claim (the `is_a` itself is the record) and adjudicable.
   There is no kernel veto on kind assignment — same trust story as reading selection: the
   recording, the audit, and the adjudication ledger are the controls, and the honest statement
   is that the restrictor check is sound RELATIVE to assigned kinds.
3. **Default `enc:Assertion`** — no frame, no classifier (or classifier abstains): the claim
   lands unmarked and unreferable. Fail-closed; a later re-classification is a vocabulary-level
   re-land, not a mutation.

## 5. Set antecedents (plural demonstratives)

The term language has NO group-entity former: coordination DISTRIBUTES predication
(`coordinate_np`/`distribute` — unit 37 lands as `And(§(achilles,…), §(drive,…))`), and that is
the shape to reuse.

- **Distributive set resolution** (built in slice 5): a plural demonstrative hole may bind a
  SET of claims `{c₁…cₙ}`; resolution produces the conjunction of the per-member β-applications
  — `And(body[h:=c₁], …, body[h:=cₙ])` — each member passing the restrictor veto, the whole
  re-gated closed. «These findings show that X» becomes `And(show(X, c₁₃), …, show(X, c₁₈))` —
  exactly what the grammar itself would build for the spelled-out coordination, which is the
  correctness argument. Mechanism: a set-binding arm in `resolve_open` (per-member veto +
  conjunction before the closed re-gate); `Candidate::ClaimSet` carries the member resources
  (the `Candidate::Kind`/`Claim` carries-its-content pattern); the binding audit records the
  membership.
- **Candidate assembly**: sets are not searched combinatorially. The discourse threads, per
  kind, the MAXIMAL run of consecutively-landed same-kind claims; the candidate set offers that
  run (plus each single claim). The proposer ranks; the kernel vetoes per member. If the
  maximal-run heuristic proves too coarse on the page, the recorded proposer is the place to
  refine — not the search.
- **Collective predication is out of scope and stays Open**: «The co-occurrence of these two
  events…» (unit 1) needs the group AS an entity (`co-occurrence of each event` is wrong), i.e.
  a real group term — deferred with its own design question, alongside the hole NUMBER feature
  (a plural demonstrative should not bind a singular individual; `HoleInfo` carries no number
  until the felicity gate captures cat features — D64 §2.4 deferral). Units that need either
  stay honestly Open.

## 5a. Literature check (review question, 2026-08-12)

The §5 split is the plurals literature's own split, and the design sits where the singular
machinery already sits (Ranta/Bekki type-theoretically; Kamp/Heim dynamically; Bos/MRS
architecturally; Gundel et al. and Elbourne for the demonstrative/`the` division of labor):

- **Distributive arm = Link's D operator, finitely instantiated** (Link 1983; Kamp & Reyle
  1993 ch. 4's duplex condition). For a KNOWN finite plurality, `∀x∈X. P(x)` is definitionally
  `P(c₁) ∧ … ∧ P(cₙ)` — and constructively the conjunction is the better form: the proof is
  the tuple of per-member proofs, so the resolved claim's justification decomposes into
  per-member `IsDerivedAs` witnesses.
- **The deferred collective arm has a name**: Link's i-sum `⊕` / Landman's group `↑` — a
  second entity-forming device beside the existing `∩` (`kind_of`). The checker rule it will
  need ("a sum of Findings is findings for a restrictor") is Link's `*` closure — a third
  check-mode coercion parallel to the DKP arm.
- **The one real divergence**: DRT's Summation/Abstraction introduce a PERSISTENT set-valued
  discourse referent; we assemble the set per binding site and no term denotes it, so
  plurality IDENTITY across sentences («These findings… They also…») is unrepresented. Chosen
  limitation, not accident: once the sum former exists, plural resolution can mint the sum
  term, which then is the persistent referent (harvestable like any entity).
- **The maximal-run heuristic is the maximality preference** (Kamp & Reyle's Abstraction is
  maximal; Nouwen on plural pronouns for the exceptions, e.g. complement anaphora) — the
  recorded proposer is the designed home for the exceptions.

## 6. What slice 5 then is

1. `enc:Claim` + kind classes in `encoding.esl`; `claim-kind-alignment.esl`; validates
   coverage.
2. Kind at landing: the frame table (deterministic) + the `KindClassifier` trait with recorded
   replay arms + `Assertion` default; `DerivedClaimGrader::cluster` takes the kind class list.
3. Incremental landing into the discourse loop: `Candidate::Claim { resource, surface }`
   (carries the built resource — D67 §4's amended mechanics) + `Candidate::ClaimSet`; the
   same-kind-run assembly; the distributive set arm in `resolve_open`.
4. Measure: the discourse close-out re-run — the 4 claim-referent units are the target;
   re-ratchet with provenance. The pinned recency arm stays the deterministic floor (the
   classifier's recorded draw joins ranks/selections/proposals in the replay set).

Not in slice 5: collective/group terms, hole number features, the reflection-lattice
source-axis cleanup (recorded in D67 §8), any bootstrap edit.

## 7. Implementation record (D67 slice 5, 2026-08-12)

- **Vocabulary**: `enc:Claim` root + the six kind classes in `encoding.esl`;
  `encoding_validates` pins the lattice. The curated alignment is
  `ontologies/encoding/claim-kind-alignment.esl` — SHADOWING redeclarations adding the lexicon
  parents (multi-parent `class X : A, B` syntax), with the full curation record in its header:
  aligned = wn:n09279458 + umlscui:C2825141 (Finding), wn:n01002956 + C0302523 + C5890437
  (Observation), wn:n01012712 (Classification); NOT aligned = the act/watching/group/process
  senses and the clinical concepts ("Signs and Symptoms", "Patient observation") — readings
  whose restrictor carries an unaligned sense do not resolve to claims, so the alignment
  doubles as sense discrimination. Targets came from a restrictor probe over the page
  (`probe_claim_unit_restrictors` + `probe_restrictor_class_labels`).
- **Kernel**: `Candidate::Claim { resource, surface }` and `Candidate::ClaimSet { kind,
  members, surface }` (both carry their content — no layer lookup); the internal `Ante`
  (One/Each) machinery with per-member vetoes; the DISTRIBUTIVE arm in the resolution core
  (per-member full application, `logic:And` right-fold, single closed re-gate; at most one set
  binding per parse — a second fails closed pending the collective/group design); the
  `ClaimLander` seam in `resolve_document` (the Proposer/ReadingRanker inversion — the
  reasoning-side impl owns the clusters, the loop threads resource + surface); maximal
  same-kind-run assembly keyed off the resource's kind class (broken by any non-landing
  sentence). Fixture tests: a landed claim resolves through multi-class inhabitation with the
  alignment analog, the wrong kind is vetoed, and a plural reference distributes over a 2-run
  with the set membership in the binding audit.
- **Reasoning**: `claim_kind.rs` — the frame table (`hypothesized/suggest that` → deterministic
  kinds), the `KindClassifier` trait with Recording (memoizing) / Replay (miss = Assertion,
  counted) arms and the live `AnthropicKindClassifier` (use-llm, document context);
  `ClaimSource.kind_classes` → `ParsedClaimGrader::cluster` writes `is_a = [EncodedClaim,
  <kinds…>]`; `DerivedClaimLander` composes frame → classifier → Assertion default and
  accumulates the clusters.
- **§5 refinement at implementation**: sets resolve at the plural hole with EVERY member
  passing the restrictor veto individually, and the run breaks on any sentence that lands
  nothing — including `Ambiguous` sentences (only `Encoded` ones land under no ranker), which
  is what made the finding-run land exactly on the units «these findings» refers to.
- **Measured** (the discourse close-out, dem→d67 snapshot + ranks replay, recency proposer,
  no ranker):
  - Deterministic floor (no classifier): 12 claims land, all `Assertion` — the pre-D68 pin
    (12/35/15/0) HOLDS exactly with the whole machinery active.
  - With the recorded kind draw (`experiments/parsing/kinds/2026-08-12-reference.json` — 12
    verdicts: 2 Finding, 4 Observation, 3 Classification, 1 Suggestion, 2 Assertion, live
    Anthropic, replay 12/0): **ALL FIVE claim-referent units close** — 19/20 «These findings
    show…» AMBIG(12), 45 «These classifications…» AMBIG(6), 49 «These findings remained
    true…» AMBIG(2), 60 «These observations suggest…» AMBIG(24) — **open 15 → 10** (encoded
    12, ambiguous 40, gap 0), each a fail-open pool awaiting the Stage-1 ranker. Both arms
    PINNED in the close-out test.
  - The isolated-sentence sweep is untouched (the chain-loaded layers add classes, no lexical
    entries) — baselines replay verified.
- **Kind verdicts are model-adjudicated pending human sign-off**
  (`experiments/parsing/kinds/README.md`), like the reading ledger.
