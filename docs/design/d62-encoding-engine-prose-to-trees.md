# D62 — The encoding pipeline: prose → typed reasoning (the driver)

*Status: design (not yet implemented). **Rewrite** (the prior version is preserved at
`d62-encoding-engine-prose-to-trees.old.md`). Rationale: D63 settled and **implemented** the
grammar/composition core, D64 specified reference resolution, D65 the lexicon runtime — so D62 is
no longer the engine internals. It is now the **document-level driver**: the pipeline that turns a
whole scientific document into typed, witnessed reasoning by orchestrating the settled core over the
kernel. The formal spine (TTR / MTT / Carpenter / DCG / EigenTT) and the lexicon-bootstrap mapper
are harvested into D63 + the `.old.md`; this doc references, it does not re-specify them. **D61 (the
method/faithfulness framework) is deferred** — to be settled against real pipeline output, not
upfront; D62 keeps its D61 touchpoints as named, deferred seams and remains buildable without it,
because the kernel felicity gate (D63) is the present oracle of structural truth.*

## 1. Scope & relation to the engine

D62 owns the **driver**: segmentation, lazy lexical recovery, the three resolution modes, assembly,
and the institution wrapper. It references — and does not re-specify — the settled core:

- **D63** — the DCG grammar engine: `parse_scoped` (string → ranked forest of felicity-gated
  `Prop` trees), the felicity gate, the categorial spine. *The composition/target the old §2–3
  described is realized here.*
- **D64** — reference (anaphora) resolution: the **open (hole-bearing) forest** and the
  resolver **component** that fills holes and re-gates — a *step* in this pipeline (S3), not its
  own institution; the institution is the whole-pipeline wrapper (§8). *The first concrete
  resolver stage (S3).*
- **D65** — the lexicon runtime (lazy, scoped) backing parse-time lookup and scope.
- **D61** — the encoding method & faithfulness framework. **Deferred** (§11); D62 emits the
  raw material D61 will later formalize.

Strict dependency, unchanged from the old §1: the engine is the **untrusted** "prose → typed trees"
step; its output enters **Derived/candidate** and the kernel oracle (felicity gate) — not the LLM —
admits it. An engine without the oracle produces *prose with false precision*; the oracle is present
(D63), so the pipeline is sound today even with D61 deferred.

## 2. Goal & litmus

Formalize scientific prose into typed reasoning committed to the chain. **Litmus: the WRN *Nature*
paper** (and similar) — encode its claims as graded, provenanced, witnessed `Prop`s, with every
unencodable span surfaced as an explicit, justified gap. Success is measured both by what encodes
*and* by the quality of the gap stream it produces (§9).

## 3. Governing principle

- **Deterministic core = oracle.** The parser + felicity gate (D63) decides what is a valid `Prop`.
  Every committed encoding type-checks by construction.
- **LLM = proposer / selector**, never load-bearing for correctness: it proposes segments and
  paraphrases, *selects* among kernel-valid parses, and proposes bindings (D64) — all re-gated.
- **Search (D43) = grounding / target-finder**: binds surface terms to existing typed entries and
  retrieves prior knowledge; it never decides meaning.
- **Witnessed, provenanced, graded.** Each unit carries source span, the parse as witness, its
  decisions, and a grade (Observed / Declared / Derived / Verified).
- **Fail-closed.** Anything unencodable within the current grammar/vocabulary becomes an explicit
  `CutItem` (or a `LexicalGap` / unresolved finding) — never silently dropped or force-fit.

## 4. Pipeline overview

Eight stages over a per-unit state machine (§5). Each has a typed resource contract (§6) and is
tagged deterministic-core vs LLM-proposer.

| # | Stage | Contract (in → out) | Kind | Spec |
|---|---|---|---|---|
| S0 | **Segment** | Document → ordered `DiscourseUnit` (provenance); route equations→FormulaTerm, citations→Reference | LLM-assisted, inspectable | D62 §7.1 |
| S1 | **Scope** (thin) | `DiscourseUnit` → `ScopedUnit` — set lexicon scope only; **no upfront term binding** | deterministic / often no-op | D62 §7.2, D65 |
| S2 | **Parse** | `ScopedUnit` → forest = {closed-one, closed-many, **open**, empty(+unresolved tokens)} | deterministic | D63 |
| S3 | **Reference resolution** | open `Item` → closed `Item` \| unresolved finding | LLM-proposer + re-gate | D64 |
| S4 | **Structural disambiguation** | closed-many → chosen `Item` + `DecisionPoint` | rank + LLM-select among *valid* parses | → D67 |
| S5a | **Lexical recovery** | empty (missing lexeme) → bind/inject entry → re-parse | search + LLM-propose + felicity gate | D62 §7.6a |
| S5b | **Reformulation** | empty (grammar gap) → paraphrase/decompose → re-parse \| `CutItem` | LLM-proposer + back-translation gate | → D66 |
| S6 | **Assemble & ground** | encoded `Prop`s → `ReasoningStructure`, bound to facts; graded | deterministic + grounding | D62 §7.7 (D61 deferred) |
| S7 | **Faithfulness back-stop** | per claim → fidelity verdict; review surface | LLM-judge, kernel-committed | **deferred → D61** |

**Parser-driven recovery (the lazy-discovery flow).** Target-discovery is *not* an upfront pass;
the parser tells you exactly which tokens are missing, so recovery is keyed off its own signal:

```
S2 parse_scoped ─▶ closed-one ─────────────────────────────▶ S6 encode
                ├▶ closed-many ──▶ S4 disambiguate ─────────▶ S6 encode
                ├▶ open (holes) ─▶ S3 reference (D64) ─re-gate▶ S6 encode
                └▶ empty ────────▶ diagnose:
                                     ├─ missing lexeme(s) ─▶ S5a lexical recovery ─▶ re-parse
                                     └─ grammar gap       ─▶ S5b reformulation     ─▶ re-parse
```

The four parse outcomes drive **three distinct resolution modes** — referential (S3),
structural (S4), and failure (S5a/S5b) — which must not be conflated.

## 5. Pipeline state model

- **Per-unit state machine:** `segmented → scoped → parsed → {resolved | disambiguated | recovered
  | reformulated | cut} → assembled`. Failures stay local to a unit; the document encodes
  incrementally and is **resumable**.
- **Discourse state = the committed chain prefix.** Cross-sentence context (antecedents for D64,
  prior `Prop`s for S4 context-consistency) is the monotonically-growing committed prefix — not a
  dynamic-semantics rewrite (per D64 §4). Each new unit resolves against what is already committed.
- **Provenance throughout:** every `DiscourseUnit` and derived resource carries its source span +
  section; every decision/recovery/cut is a committed, auditable record.

## 6. Typed contracts

Each stage is `resource → resource`, so stages are independently testable with fixture resources and
the pipeline state is chain-committed and auditable. New resource classes (encoding ontology;
**build these first**):

- `DiscourseUnit` — a parsing unit + provenance (span, section, kind: prose / equation / citation).
- `ScopedUnit` — `DiscourseUnit` + lexicon scope (thin; binding deferred to S5a).
- `LexicalGap` — an unresolved token + disposition (bound-to-IRI / injected / unresolvable).
- `DecisionPoint` — a disambiguation choice + warrant (minimal record now; typed-content
  formalization → D61).
- `CutItem` — an unencodable span + reason + disposition (minimal record now; → D61).
- `EncodedClaim` — a committed `Prop` + provenance + grade + parse witness.
- `ReasoningStructure` — the assembled per-document reasoning graph.

## 7. The stages

### 7.1 Segment (S0)
Document → `DiscourseUnit`s at clause/claim granularity (the grammar parses clauses best).
**Route non-prose out of the parser:** equations → FormulaTerm/EigenTT, citations → Reference /
Citation (reference ontology), tables/figures → structured capture. LLM assists by classifying
claim-bearing vs boilerplate and splitting compound sentences; units stay explicit/inspectable.

### 7.2 Scope (S1) — thin, optional
Sets only the **lexicon scope** for ranking (default = the chain's lexica, or a named
`LexiconProfile`, D65). **No upfront per-term target-discovery** — that is lazy (S5a). May be
deferred to S4, since scope affects ranking, not whether a parse succeeds. Often a no-op annotation.

### 7.3 Parse (S2)
`parse_scoped` (D63) → the forest, classified into the four outcomes. The empty result **must report
the unresolved tokens** (lookup already knows them) so S5 can diagnose missing-lexeme vs grammar-gap.

### 7.4 Reference resolution (S3) — D64
Fill referent holes in open parses (antecedent = committed resource IRI), substitute, **kernel
re-gate** to a closed `Prop`; unresolved ⇒ fail-closed finding. Per D64.

### 7.5 Structural disambiguation (S4) — → D67
Closed-many → one. All candidates already type-check, so this is *selection*: start from the
parser's `(lexicon_order, sense_rank)` ranking, add **context-consistency** against the committed
prefix (no contradiction; references resolve), and an LLM judge picks among valid readings. Emit a
`DecisionPoint`. *(Warrants its own spec, D67, for the context-consistency model.)*

### 7.6a Lexical recovery (S5a) — the on-demand target-discovery
Triggered only by missing-lexeme failures. Per unresolved token: **search** (D43 text/vector) the
in-scope lexica for a target (synonym / hypernym / inflection) → bind it; if none, **propose +
inject** a new entry (LLM proposes → felicity gate validates → witness) — the domain-lexicon
injection path. Then **re-parse**. Records a `LexicalGap`. *(This is where the old upfront S1 work
now lives, bounded to the words that actually blocked the parse.)*

### 7.6b Reformulation (S5b) — grammar-gap only → D66
The lexemes resolved but the grammar can't compose them. Paraphrase / decompose into supported
constructions → re-parse, with a **back-translation faithfulness gate** (parse the rewrite →
back-translate the resulting `Prop` → compare to the original span). If still unencodable → `CutItem`.
*(Warrants its own spec, D66 — the rewrite-and-verify loop is research-grade.)*

### 7.7 Assemble & ground (S6)
Encoded `Prop`s → `ReasoningStructure`, bound to discovered facts (grounding via the existing
`grounding` skill; the gated grounding-discovery formalization is **deferred to D61**). Each
`EncodedClaim` carries provenance + grade + parse witness.

### 7.8 Faithfulness back-stop (S7) — deferred → D61
A named seam. Until D61 is settled, the pipeline commits claims with provenance + grade + the kernel
re-gate (structural truth), marked *"fidelity check deferred."* When D61 lands, S7 back-translates +
scores → a **Derived** verdict (never auto-Verified), with a human-review surface — slotting into
this seam without reshaping the driver.

## 8. The engine as an institution

The driver's home is a **dispatched institution** (D14), the same pattern as Julia/Lean/R and the
reasoning checker. Harvested from the old §6, with the D61 half deferred:

- **Generation = OnDemand.** `FormalizeDocument` / `EncodeProse` QueryClass, invoked via a
  commit-capable `FIBER … INTO`; the engine *generates*, it does not gate arbitrary commits.
- **Derived by construction.** An institution dispatch emits a `DerivedResource` under a
  `ProgramTrace → IsDerivedAs` (D56) — so output is **Derived** ("the kernel attests the engine
  computed this"), never Verified. The provisional-until-checked discipline is framework-enforced.
- **Felicity = AutoOnLoad.** The commit-time type-check on the emitted `lexicon:`/encoding classes is
  a deterministic, fail-closed D14 gate — structural truth, present today (D63).
- **Faithfulness = a separate (deferred) verification institution (D61).** The clean
  *generation institution (D62, Derived) + verification institution (D61, Verified)* pair; the
  second is deferred.
- **The comorphism reading (intended direction, not settled):** autoformalization is a translation
  from the natural-language source into the EigenTT/reasoning institution (D10; cf. D37). The
  faithfulness gap *is* that satisfaction-preservation is not guaranteed by construction (the LLM
  proposers make it approximate) — which is exactly why the verification half (D61) is mandatory,
  not optional.

## 9. Coverage feedback loop

The `CutItem` (S5b), `LexicalGap` (S5a), and unresolved-finding (S3) streams are a **first-class
output** — each is precisely a construction or word the corpus needed. They feed D63 grammar
extension and lexicon injection. Treat gap-harvest (counts, exemplars, dispositions) as a product,
not just a failure log; on the WRN litmus it is likely the most valuable early signal.

## 10. Build & test roadmap

> **Empirical reprioritization** (see `docs/notes/d62-encoding-prototype-findings.md`): a
> core-algorithm prototype run against real WordNet + the WRN paper shows the *vocabulary* is
> already covered by WordNet+UMLS+NCBI (≈87% single-word, ~3 truly-novel content words), and the
> actual gating blockers are **S0 tokenization** and **closed-class/grammar coverage of ordinary
> English** — not domain vocabulary or disambiguation. Sequence accordingly: S0 + closed-class
> first; bring the lexica into scope (and tackle S4 ambiguity) only once sentences parse.

**Contracts first** (define the §6 encoding-ontology classes), then slices, each independently
verifiable behind its contract:

1. **Grammar open-forest + open-parse carrier** (D63/D64 grammar side; engine-side free-var holes +
   context, kernel stays hole-free — see `docs/notes/d62-d64-open-parse-carrier.md`) — unblocks S2's
   open outcome + S3.
2. **S3 reference resolver** (D64) — lays the reusable **proposer → re-gate → (deferred faithfulness)**
   spine that S4/S5 reuse.
3. **S5a lexical recovery** — high-value early: lets the parser make progress on real prose without a
   full upfront lexicon (keyed off the parser's unknown-token signal).
4. **S0 segment + S1 scope** — the document front-end.
5. **S4 disambiguation** (D67), then **S5b reformulation** (D66) — the hardest; the gap-harvest loop.
6. **S6 assemble & ground**; **S8 institution wrapper** (`FormalizeDocument`).
7. **S7 faithfulness** — last, with D61.

**Test discipline (three layers, mirroring D63):** (1) deterministic-core exact tests on toy fixture
resources; (2) seeded-real-lexicon tests (the `wordnet_scale.rs::stage_a` pattern — a real seeded
WordNet slice + a real-shaped UMLS concept); (3) `#[ignore]`d heavy E2E (full-document, real LLM).
**LLM stages are mock-by-default** (recorded proposer) so CI is deterministic — but the **kernel
re-gate is always real**, so even a mocked proposer's output is truth-checked. The litmus E2E runs a
WRN-paper slice and asserts on both encoded claims and the gap stream.

## 11. Implementation architecture (one level deeper)

This section grounds the stages in the actual codebase: the core algorithm per stage, where new
code lives, which existing interfaces to reuse, and the adjustments required. (Interfaces verified
against the tree; signatures abbreviated.)

### 11.1 The trust boundary, in code

Two homes, one boundary:

- **Kernel (Rust) = the deterministic oracle.** Parse, felicity gate, search, commit/witness,
  institution dispatch. Reachable from orchestration only through gRPC (`proto/eigenius.proto`).
- **Orchestration (Deno/TS) = the LLM proposers.** Each LLM stage is a **component**
  (`orchestration/src/components/*.ts`) following the `complete_json.ts` pattern, calling the LLM
  (Vercel AI SDK `generateObject`, JSON-schema-constrained) and the kernel via
  `client/kernel_client.ts`. Components **propose**; the kernel **re-gates**.

The driver is hosted as a D62 institution; its reasoner runs in orchestration via the existing
external-dispatch path (`server/component_executor.ts` `DispatchExternal`, the same host the
schema.org / R / Julia institutions use).

### 11.2 Core algorithm + placement per stage

| Stage | Core algorithm | Side | Reuses |
|---|---|---|---|
| S0 Segment | LLM split→clauses + classify claim/boilerplate + route non-prose (equation/citation regex+LLM); emit `DiscourseUnit` | TS component `segment.ts` | `generateObject`; `kernel_client.load` |
| S1 Scope | resolve `LexiconProfile`→scope IRIs or default | kernel | `resolve_lexicon_profile` (`dcg/lookup.rs`) |
| S2 Parse | `parse_scoped` → forest, classify {one/many/open/empty}, **report missed tokens** | kernel RPC | `LexicalIndex::parse_scoped` (`dcg/lookup.rs`); `ParseSentence` (`server/parse.rs`) |
| S3 Reference | feature-prefilter candidates from committed prefix → LLM bind → substitute → **re-gate** | TS `reference_resolve.ts` + kernel re-gate | D64; `reduced_felicitous` (`dcg/lookup.rs`) |
| S4 Disambiguate | start from `Cost` rank → context-consistency via EigenQL over committed prefix → LLM select → `DecisionPoint` | TS `disambiguate.ts` | `kernel_client.query`; `generateObject` |
| S5a Lexical recovery | per missed token: search candidates → bind, else LLM-propose entry → construct → commit (felicity-gated) → re-parse | TS `lexical_recovery.ts` + kernel | EigenQL `~` via `query` **or** `run_text_search`/`top_k_subjects`; `gate_entry`; `kernel_client.load` |
| S5b Reformulate | LLM paraphrase/decompose → re-parse → **back-translate + compare** → accept/`CutItem` | TS `reformulate.ts` (→D66) | `ParseSentence`; `generateObject` |
| S6 Assemble | collect `EncodedClaim`s → `ReasoningStructure`; ground via search | kernel + TS | `kernel_client.{query,load}` |
| S7 Faithfulness | *deferred → D61* | — | — |

The disambiguation/context-consistency check (S4) and back-translation (S5b) both run **over the
committed prefix via EigenQL** — no new kernel surface, just `kernel_client.query`.

### 11.3 New components to add — and where

**Kernel (Rust):**
- `ontologies/encoding/encoding-ontology.esl` — the §6 contract classes (`DiscourseUnit`,
  `ScopedUnit`, `LexicalGap`, `DecisionPoint`, `CutItem`, `EncodedClaim`, `ReasoningStructure`) +
  the `FormalizeDocument` Institution and its OnDemand `QueryClass`. Insert into `BOOTSTRAP_CHAIN`
  (`kernel/src/bootstrap/mod.rs`) **after `lexicon`** (it references `lexicon:`/EigenTT vocab).
- D64 grammar side (`kernel/src/dcg/`): the **open-parse carrier** (engine-side free-var holes +
  context — *no* new `nbe/term.rs` node; the kernel stays hole-free), `Case`, the **open forest**
  (`docs/notes/d62-d64-open-parse-carrier.md`).
- A small **lexical-entry constructor** reusing the importer shape (`construct_lexical_entry(form,
  cat, sem, sem_type, rank) -> Resource`) for S5a — factor from `eigenius-wordnet::convert`.

**Orchestration (TS):** `components/{segment,reference_resolve,disambiguate,lexical_recovery,
reformulate}.ts` (the S5b/S4 ones map to D66/D67), plus `components/encoding_resources.ts` (typed
constructors stamping `is_a`). Register each in `main.ts` (`components.register(IRI, handler)`).

### 11.4 Existing interfaces to reuse (no change)

- **Parse / felicity:** `LexicalIndex::{parse_scoped, reduced_felicitous}`, `gate_entry`
  (`dcg/lookup.rs`, `dcg/lexicon.rs`) — the parse + re-gate path D64 needs is already implemented.
- **Search:** `run_text_search` (`query/text/search.rs`), `top_k_subjects` (`query/vector/search.rs`)
  — or, preferred, reach them through **EigenQL `~`** via `kernel_client.query` to avoid new surface.
- **Institution/dispatch:** `InstitutionIndex` + `QueryClassEntry` + `DispatchRole`
  (`institution/registry.rs`), FIBER eval (`query/evaluate/fiber.rs`), AutoOnLoad
  (`institution/dispatch.rs`) — `FormalizeDocument` is declared as data, no new dispatch code.
- **Commit + Derived witness:** `CommitPipeline` (`commit/pipeline.rs`) — felicity runs in
  `structural_validate`; `WitnessKey`/`IsDerivedAs` (`witness/mod.rs`, `program/trace.rs`) gives the
  Derived grade by construction.
- **Orchestration:** the component contract + registry (`components/registry.ts`,
  `complete_json.ts`), `generateObject`, `kernel_client.{load,query,inspect,runProgram}`, and the
  `DispatchExternal` host (`server/component_executor.ts`).

### 11.5 Adjustments required to existing components

Small, localized — the spine exists; these are the seams:

1. **Surface missed tokens on parse failure** (`dcg/lookup.rs`): `lookup_span` silently drops a
   miss. Add `parse_with_failures() -> (forest, missed_spans)` so S5a can target only the blocking
   words. *(Required for lazy lexical recovery.)*
2. **`ParseSentence` response carries the open forest + missed tokens** (`proto/eigenius.proto`,
   `server/parse.rs`): today it returns the closed forest only. Add the open (hole-bearing) parses
   (D64) and the missed-span list. Regenerate the TS client (`buf generate`).
3. **`kernel_client.parseSentence(...)`** (`orchestration/src/client/kernel_client.ts`): the RPC and
   generated schemas exist, but there's **no client wrapper** — add one (mirrors `query`/`load`).
4. **Lexical-entry constructor + commit-from-pipeline** for S5a (kernel): the construction shape
   lives in the importers; expose it as a callable so a runtime entry can be built and committed
   (commit + `gate_entry` already validate it).
5. **(If not reusing EigenQL `~`) expose search as an API** — `run_text_search`/`top_k_subjects` are
   currently internal to the similarity pre-pass. Recommendation: **reuse EigenQL** and skip this.
6. **D64 re-gate reachable from the resolver** — the resolver substitutes a binding and needs the
   kernel to re-check the resolved `sem : Prop`. `reduced_felicitous` already does this; expose it
   on the resolved-tree path (a focused RPC, or fold into the S3 resolver component of the
   `FormalizeDocument` pipeline institution).
7. **Open parses carrying proof obligations** (the *factive-subordinator* engine extension —
   `dcg/parser.rs`, `dcg/lexicon.rs`, the bridge in `dcg/lookup.rs`, and the felicity gate). This is
   **not small** — it is the substantive prerequisite for factive connectives (`because`/`although`/
   `while`, D62 §2d) and, more broadly, for **presupposition-as-felicity**. Today a parse produces a
   **closed** sem term and the felicity gate `check_infer`s it in a fixed context. A factive entry has
   the dependent signature `Π(p q:Prop) → p → q → Prop`, so its derivation must introduce **fresh
   proof hypotheses** `h_p : p`, `h_q : q` into the typing context and build `Because(p, q, h_p, h_q)`.
   The parse item's sem is then **no longer closed** — it carries free proof variables, the gate must
   type-check it in a context that *binds* them, and the result shape changes from "a closed `Prop`"
   to "**a `Prop` in a context of proof obligations**" that travel out of the parser to the
   grounding/reasoning layer (where they are discharged: `Holds`/`Open`/`Fails`, with *local
   accommodation* = local discharge — the same mechanism as attitude-verb **plugs**, which also makes
   the existing intensional `shows` complement-verb (D63 §8.11) need plug-binding rather than its
   current opaque treatment). The presupposition-projection account (free-in-Γ = projected through
   negation/modals; locally λ-bound = filtered) is recorded in
   `docs/notes/d62-subordinator-design-findings.md` §5.

   **This is one capability, two instances — and it also subsumes pronominal reference (S3/D64).** A
   referential pronoun (`it`/`they`/`its`/`their`) is categorially a plain NP (core-en `ProNP` = NP)
   whose semantics is a **free referent variable** — "it affects HeLa" ⇒ `affects(hela, ?ref)` with
   `?ref : Entity` unbound. That is the *same* open-parse shape as a factive's proof obligation,
   differing only in the **hole's type and resolver**: an `Entity`-typed hole resolved by **D64**
   (anaphora), vs. a `Prop`/proof-typed hole resolved by **grounding**. (Our existing finding — a bare
   chain `ResourceRef` lowers to an unbound `Var`; "named-entity references need explicit
   binding/resolution" — is the same gap.) So the real foundational piece is: **a parse whose `sem`
   is open under a context of typed holes, carried out of the parser for downstream resolution.**
   Building it once unblocks both the factive connectives *and* referential pronouns. The unified
   design — one **engine-side** carrier (free-var holes + context; **kernel stays hole-free**, no new
   `Exp` node, per the `nanoda_lib`/Lean elaborator-vs-kernel split), two resolver dispatches
   (`EntityRef`→D64 substitution; `ProofObligation`→grounding witness), pronouns (D64 Phase A) as the
   carrier MVP — is in `docs/notes/d62-d64-open-parse-carrier.md`.

   **Sequencing:** the *closed-term* lexicon ships without it — determiners, modals, `if` (native
   `→`), `but` (→ `logic:And`) are done; the deictic/non-anaphoric `we` and the discourse
   connectives `however`/`thus` may admit a closed first cut, but `it`/`they`/`its`/`their` and the
   factive subordinators both require this capability. Scope it as its own deliberate engine piece.

Net new kernel surface is small *except* for item 7: the parse-failure/open-forest fields on
`ParseSentence`, a lexical-entry constructor, and the D64 grammar nodes are minor; the
open-parse/proof-obligation extension (item 7) is the one substantive engine change, and is gated and
scoped on its own. Everything else is **data** (the encoding ontology + `FormalizeDocument`
declaration) and **orchestration components** over existing RPCs.

## 12. Deferred D61 seams

D61 is deferred and settled against real pipeline output. Until then:
- **S7 faithfulness back-stop** is a named stub (structural truth only; fidelity deferred).
- **`DecisionPoint` / `CutItem` / `LexicalGap`** are emitted as **minimal provenanced records** — not
  the full typed-`Prop` content treatment. These records are the **harvest material** for the D61
  rewrite.
- **Grounding-discovery** uses the existing `grounding` skill; its gated formalization waits for D61.

This keeps D62 self-contained against the *existing* (un-rewritten) D61 and the grounding skill.

## 13. Open decisions

- Parsing granularity (clause vs claim) — the dominant coverage lever.
- Discourse-context window for S4 (whole-document vs recency).
- How aggressively S5b reformulates before declaring a `CutItem`.
- Whether S1 scope is ever needed upfront, or always deferrable to S4.
- Multi-unit batching vs strict per-unit commit (resumability vs throughput).

## 14. References & cross-references

- **Settled core:** D63 (grammar/composition/felicity), D64 (reference resolution), D65 (lexicon
  runtime). **Method (deferred):** D61. **Prior version:** `d62-encoding-engine-prose-to-trees.old.md`
  (formal spine — TTR / MTT / Carpenter / DTS-`lightblue` — and the lexicon-bootstrap mapper).
- **Platform:** D43 (text/vector retrieval), D14 (institution realisation), D31 (external-institution
  lifecycle), D56 (kernel bridge / `IsDerivedAs`), D10/D37 (institution comorphisms), D8
  (`complete_json` component pattern), reference ontology (Reference / Citation).
- **Litmus:** the WRN *Nature* paper.
- **New stage specs (to author):** D66 (reformulation), D67 (structural disambiguation).
