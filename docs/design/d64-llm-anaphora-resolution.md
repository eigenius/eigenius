# D64 — LLM-based anaphora resolution: pronouns as resolved resource references

*Status: design (not yet implemented). The decision — anaphora is resolved by an **LLM proposer behind
the kernel felicity oracle**, not a core engine dependency nor a compositional-dynamic-semantics rewrite.
This doc specifies the three-layer subsystem: the grammar's referent **holes** (D63), the **resolver
component** — a *step in the D62 `FormalizeDocument` pipeline institution* (§8 of D62), **not its own
institution** — and the kernel **re-gate** + faithfulness verdict (D61). Builds directly on D63 §5.3
(anaphora → committed-resource IRI references; the donkey-anaphora Σ-truncation escape hatch) and is the
first concrete consumer of the D61 faithfulness machinery.*

> **Layering note.** The *dispatched-institution* property (untrusted LLM proposer behind the kernel
> felicity boundary, like Lean/R/Julia) belongs to the **whole encoding pipeline** — the single D62
> `FormalizeDocument` institution wraps all of S0–S7. The reference resolver is **one component/step
> (S3)** inside that pipeline, not a separate institution. Earlier wording in this doc that calls the
> resolver itself "a dispatched institution" is superseded by this note.

## 1. The problem, and the decision

D63 builds the sentence grammar; 6-tail's headline item is **pronouns**, whose two halves are very
different:
- **Case** (he/him, who/whom) — the cheap *syntactic* half (a `Case` feature; English overt case is
  essentially the pronoun paradigm). Prevents `*him affects he`. Folds into the pronoun lexicon.
- **Anaphora** — the *semantic* half: *what the pronoun denotes*. A pronoun has **no fixed denotation**;
  it refers to an antecedent in the surrounding discourse. This breaks the assumption every slice so far
  has relied on — that meaning is **sentence-level and context-free** (lookup → CKY → felicity yields a
  closed `Prop` per sentence, no cross-sentence state). Anaphora is inherently discourse-level.

**Decision (this doc): resolve anaphora with LLM-based machinery, as a post-parse component of the D62
`FormalizeDocument` pipeline institution (not a separate institution — see the layering note above).**
The two rejected alternatives:
- *Compositional dynamic semantics* (DRT / dynamic predicate logic): thread a discourse context through
  composition, changing the sem type of **everything** to context-passing. Powerful (handles donkey
  anaphora natively) but invasive and research-grade — disproportionate to a phenomenon that rides on top
  of extensional facts. D63 §5.3 already declines this: "no proof-search engine [in the core] … a prover
  is needed only downstream … and fits as a *dispatched institution*."
- *Symbolic resolution* (centering theory; Hobbs' algorithm): brittle, hand-tuned salience heuristics,
  weak on the world-knowledge cases ("the drug … it inhibited the enzyme" — which noun?).

The LLM approach is **architecturally native**: Eigenius is *untrusted-proposer + kernel-as-felicity-
oracle* throughout (D62: the LLM proposes trees, the kernel admits/rejects). Anaphora resolution is
pragmatic inference over discourse and world knowledge — exactly where an LLM is strong and a symbolic
resolver is brittle — and the LLM **never gets the last word**: it proposes a binding, the kernel
re-checks the *resolved* tree. We get the LLM's reach without trusting it.

## 2. Architecture — three layers, one trust boundary

```
prose ──parse(D63)──▶  tree with referent HOLES        (felicitous-modulo-resolution; NOT a closed Prop)
                              │
                  resolve(S3 component, LLM)            hole → antecedent  (chain IRI / bound var)
                              │  substitute
                              ▼
                       resolved tree ──re-gate(kernel)──▶  closed Prop ✓   (LLM proposes, kernel disposes)
                              │
                       faithfulness(D61): Derived verdict + over-resolution back-stop
```

The **felicity oracle stays the trusted boundary** (D62). The LLM lives entirely in the resolver step;
its output is re-checked by the kernel before anything commits. An ill-typed resolution is rejected by
the *kernel*, not the LLM.

This is exactly the **untrusted-proposer-behind-the-felicity-oracle** shape D63 §5.3 anticipated
(anaphora resolution "fits as a dispatched institution, like the Lean/R/Julia computations, never a core
engine dependency" — realized here as a *component* of the one pipeline institution, per the layering
note above), and the
**antecedent is a committed resource referenced by IRI** ("our antecedents are committed resources
referenced by IRI … linguistic anaphora resolves to a *resource reference*"). D64 makes the resolver an
LLM.

## 3. The grammar side (D63): referent holes + case

> **Carrier note (revises the `Exp::Anaphor`-node recommendation below).** The referent hole here
> is the **entity instance** of a general open-parse carrier shared with D62's factive-subordinator
> proof obligations — one carrier, two resolver dispatches (`EntityRef` → this D64 LLM resolver via
> substitution; `ProofObligation` → grounding via witness), and a single sentence can carry both at
> once. **But the carrier is a DCG-engine (elaboration) extension, not a kernel term node.** The
> `nanoda_lib`/Lean precedent is decisive: the kernel `Expr` has *no* metavariable — holes are an
> elaborator concept and the kernel only checks fully-elaborated terms. So represent a hole as a
> **fresh free variable** (already a neutral in NbE) plus an **engine-side context** carrying its
> `id`/`kind`/features, type-checked under that context and resolved before commit — **no
> `Exp::Anaphor` node, the kernel and chain stay hole-free.** This addresses the bare-`Var`
> objections below (fresh-var namespace; features in the engine-side context; free vars are already
> neutrals — no NbE special-casing). D64 Phase A is the carrier MVP. See
> `docs/notes/d62-d64-open-parse-carrier.md` (§2, §7).

**Pronouns are case-marked NPs whose sem is a referent hole.** A pronoun does not denote; it marks a slot
the resolver fills.

- **Hole representation — SUPERSEDED (see the carrier note above).** *Original recommendation: a new
  `Exp::Anaphor { id, ty, features }` kernel term node.* The revised design keeps the **kernel
  hole-free** (per the `nanoda_lib`/Lean precedent — metavariables are an elaborator concept, absent
  from the kernel `Expr`): a hole is a **fresh free variable** (already a neutral in NbE) plus an
  **engine-side context** carrying `id`/`ty`/`features`, keyed by the variable. The pronoun lexical
  entry's `sem` introduces such a hole-var. This meets the original objections to a bare `Var` without
  a node — *collision* → a reserved fresh-var namespace; *nowhere for features* → the engine-side
  context; *NbE special-casing* → none (free vars are already neutrals) — and adds **zero** kernel
  surface. See `docs/notes/d62-d64-open-parse-carrier.md` §2/§7.
- **Case feature.** Add `Case` (`nom` / `acc` / `case_any`) to `cat_np` (mirrors the `Num`/`Fin`
  `feat_meets` machinery — `*_any` meets anything). Verb **subject** slot requires `nom`, **object**
  slots `acc`; **full NPs** (HeLa, the gene) are `case_any` (unchanged from today); **pronouns** are
  case-marked (`he` = nom, `him` = acc, `it`/`they` invariant ⇒ `case_any`/number-marked). This keeps
  `*him affects he` out and is the *entire* syntactic content of "case" (English has no productive case
  elsewhere).
- **Lookup returns hole-bearing parses as a distinct category.** A tree containing an `Anaphor` is
  **open** — not a closed `Prop` — so the current felicity filter (`reduced_felicitous`, which checks a
  closed term inhabits `⟦cat⟧`) must *not* admit it as a final parse. `parse` returns two forests: the
  ordinary closed forest, and an **open (hole-bearing) forest** awaiting resolution. An open parse is a
  first-class, non-error outcome.

The grammar layer is otherwise unchanged: pronoun entries are ordinary closed-class lexical entries.

## 4. The resolver (S3 component of the D62 pipeline institution): the LLM step

A new orchestration component (sibling of `complete_json.ts`, using `llm/adapter.ts` and the kernel
bridge `kernel_client.ts`):

1. **Assemble the candidate set = the in-scope antecedents.** The committed chain resources from the
   prior discourse (recent `lexicon:Sentence`s and the entities they reference) + entities introduced in
   the current parse. Recency/salience is **not** modelled symbolically — candidates are passed (ordered
   by recency) and the LLM ranks. A cheap **feature pre-filter** (number/gender/type) narrows candidates
   before the call, shrinking the hallucination surface.
2. **Call the LLM (structured output).** Input: the sentence, each hole (type + features), and the
   ordered candidate antecedents (IRI, type, surface form, recency). Output (JSON-schema-constrained):
   per hole, `{ antecedent_iri | "introduce-new" | "unresolvable", confidence, rationale }`.
3. **Substitute + re-gate.** Substitute each `Anaphor.id → resolved Exp` (an `EigonResource` IRI ref for
   a named antecedent; a bound `Var` for the donkey case — Phase B). Then **the kernel re-checks
   felicity** (`kernel_client.ts`): the resolved term must type-check to `Prop`. A type mismatch — "it"
   bound to a `Gene` where the predicate needs a `CellLine` — is **rejected by the kernel**; the resolver
   retries with the next-best candidate or marks the hole unresolvable.
4. **Fail closed.** An unresolved pronoun ⇒ **no committed sentence**, recorded as a finding (an open
   discovery gap, D61), never silently dropped or guessed.

## 5. The trust boundary and faithfulness (D61)

The kernel re-check verifies the resolution **type-checks** — **necessary, not sufficient**. The residual
risk is **over-resolution**: a binding that is well-typed but *wrong* (two cell lines in scope; "it"
could be either, both type-check). This is precisely the D61 faithfulness gap (*checker-passing ≠
faithful*). Therefore:

- The resolved sentence commits with a **Derived** grounding verdict (the kernel re-checked it) —
  **never auto-Verified** (the LLM-judge is inflated; D61). Low-confidence or genuinely-ambiguous
  resolutions are surfaced for human review, not committed.
- D61's **back-stop** applies: back-translate the resolved proposition ("HeLa is primary") and score
  consistency against the source ("… it is primary") — a faithfulness check on the binding itself.
- Anaphora resolution is thus the **first concrete consumer of the D61 machinery** and should be built in
  coordination with it (the CQ-runner / `faithfulness_check.ts`).

## 6. Scope and roadmap

- **Phase A — referential anaphora to chain IRIs (recommended first).** Resolve pronouns to **named
  entities already committed as chain resources** ("HeLa … it" → `it := hela`). The 80/20 for scientific
  prose; the kernel re-check is a clean type test; the antecedent is a resource reference exactly as
  D63 §5.3 specifies. This is the tractable, high-value core.
- **Phase B — bound-variable / quantificational (donkey) anaphora (deferred).** "every gene that affects
  a cell line … it" — the pronoun is bound by a quantifier in scope. Handled via the **D63 §5.3 / D46
  escape hatch**: compose *that sentence* in `Set` (genuine Σ, proof-relevant), and **truncate to `Prop`
  at the sentence boundary** (`‖Σx:N. P‖ := ΠC:Prop. (Σx:N. P → C) → C`). The resolver proposes the
  binder; the Σ gives the reusable witness; proof-relevance stays local to one sentence and the reasoning
  layer only ever sees `Prop`. No whole-grammar dynamic-semantics rewrite.
- **Case** folds into Phase A (the pronoun lexicon).
- **Minor tail:** `who`/`whom` (wh-pronoun case — low value; `that`/`which` are case-invariant and
  dominate scientific text); **possessives** (`its`/`their` — genitive, a separate determiner-like
  construction). Both deferred, demand-driven.

## 7. Decisions — resolved and open

*Resolved:*
- LLM resolver as a **post-parse component of the D62 `FormalizeDocument` pipeline institution** (not
  its own institution; not dynamic semantics, not symbolic).
- Hole = a **fresh free variable + engine-side context** (`id`/`ty`/`features`), **kernel hole-free** —
  *revised from the original `Exp::Anaphor` kernel node* per the carrier note (`nanoda_lib`/Lean: no
  kernel metavariable). See `docs/notes/d62-d64-open-parse-carrier.md`.
- Antecedent = committed chain-resource IRI (Phase A); bound `Var` via Σ-truncation (Phase B).
- Kernel re-gates the resolved term; verdict **Derived**, never auto-Verified; unresolved ⇒ fail-closed
  finding.
- Resolution runs **per sentence at commit time** — the discourse context grows monotonically as
  sentences commit, so each new sentence resolves against the committed prefix.

*Open (decide at build time):*
- Candidate-window policy (how far back the discourse context reaches; whole-document vs. recency window).
- Multi-hole joint resolution vs. independent per-hole (joint is more faithful for "X … Y … they"; more
  LLM surface).
- Whether the feature pre-filter is hard (drop type/number-mismatched candidates) or soft (down-rank).

## 8. Implementation plan (phased; each independently verifiable)

1. **Grammar holes + case (D63 engine).** Open-parse carrier — a hole = fresh free var + engine-side
   context, **kernel hole-free** (carrier note; *not* an `Exp::Anaphor` node); `Case`
   feature on `cat_np` + the verb-slot case constraints; pronoun lexical entries (`it`/`they`/`he`/`him`/
   `she`/`her`/…); `parse` returns the open (hole-bearing) forest distinctly. *Verify:* "it is primary"
   parses to a hole-bearing tree (open, not admitted closed); `*him affects he` fails the case-meet;
   existing closed sentences are unchanged (regression).
2. **Resolver component (orchestration/D62).** Candidate assembly from committed discourse + feature
   pre-filter + structured LLM call + substitution + kernel re-gate via `kernel_client.ts`. *Verify:*
   "HeLa affects BRCA1. It is primary." → `it := hela`, re-gated `is_primary(hela) : Prop`; an ill-typed
   forced binding is rejected; an ambiguous case fails closed.
3. **Faithfulness link (D61).** Derived verdict; back-translation back-stop; confidence/ambiguity →
   human-review surface. *Verify:* a planted wrong-but-typed resolution is flagged by the back-stop; a
   faithful one commits Derived.

Phase B (donkey via Σ-truncation) is a later slice, gated on a real corpus need.

## 9. References and cross-references

- **In-repo / internal:** D63 §5.3 (the dispatched-institution stance, resource-IRI antecedents, the
  donkey Σ-truncation escape hatch); D62 (the encoding institution, proposer/oracle split, felicity
  boundary); D61 (the faithfulness gap; back-translation; Derived-not-Verified); D46 §… (Prop universe,
  the `‖Σ‖`-truncation); D56 (kernel bridge); D8 (`complete_json` component pattern). Code touch-points:
  `kernel/src/dcg/{category,lookup}.rs` (the open-parse carrier: free-var holes + engine-side context,
  Case, open forest — **no `nbe/term.rs` node; kernel hole-free**),
  `ontologies/lexicon/{lexicon-ontology,closed-class}.esl` (Case feature + pronoun entries),
  `orchestration/src/components/` (the resolver), `orchestration/src/llm/adapter.ts`.
- **Prior art (named; bib entries to be verified before citing as load-bearing anchors, per the D61
  grounding discipline — never fabricate):** Discourse Representation Theory (Kamp) and dynamic semantics
  (Groenendijk & Stokhof) — the compositional alternative we decline; **Centering theory** (Grosz, Joshi
  & Weinstein) and Hobbs' resolution algorithm — the symbolic alternative we decline; **lightblue** DTS
  (in `references/`) — the entities-as-witnesses contrast that motivates our resource-IRI antecedents
  (D63 §5.3); contemporary LLM coreference resolution — the empirical basis for the proposer choice.
