# D64 — demonstratives as referent holes (Stage 2 of the parser-pipeline plan, §2.1)

**Status: design note for review — precedes any code.** Stage 2 of the four-stage build map
(retired `2026-08-19`; the as-built record is work-stack entry 0, and Stage 4 became
[D71](../design/d71-document-formalization-service.md)). The resolver machinery this feeds is
built (`kernel/src/dcg/parse/resolve.rs`, the Π-carrier in `felicity.rs`).

## 0. Problem

`this / that / these / those` are wired as **referential definites**: their sems denote
`ontology:the(A)` — the ι operator, a fixed presupposed referent — exactly like plain `the`
(`ontologies/lexicon/closed-class.esl` §definites; the sems are `the_ref_subj_sem` /
`the_ref_obj_sem`, shared by all five surfaces). A demonstrative NP therefore parses **CLOSED**
and never reaches the D64 resolver.

On the corpus page this is the dominant anaphoric device: ~19 units ("These findings…", "This
state…", "these data sets…", "These libraries…"). Each parses to `the(N)` with **no discourse
link** — *which* findings is unrepresented. Meanwhile the machinery that would resolve them
(typed holes, proposer, kernel re-gate, discourse loop) sits complete and reaches only `it` /
`they` / possessors / elided comparative standards.

The current ι reading of a demonstrative is, for this corpus, a **misparse we have been
blessing**: in scientific prose "these X" is discourse-anaphoric, full stop. (Plain `the` is
different — "the MLH1 promoter" is genuinely unique-referential — and **stays ι**; only the four
demonstrative surfaces move.)

## 1. The type problem, and why the pronoun protocol doesn't transfer as-is

A pronoun hole is **Entity-typed**: `lexicon:anaphor : lexicon:Entity` is a placeholder axiom the
seed stage freshens into a span-keyed free variable (`$anaphor$i_j`, `kernel/src/dcg/holes.rs`);
the felicity gate abstracts it into the Π-carrier `λ(h:Entity). body : Π(h:Entity). ⟦cat⟧`
(`felicity.rs::classify_felicitous`), and resolution β-applies an antecedent, re-gated closed.

A demonstrative hole must be **restrictor-typed**: "these findings" may resolve only to
findings. The Π-carrier already supports arbitrary hole types (`HoleInfo.ty` is an `Exp`), and a
restrictor-typed hole gives the FULL kernel veto — `resolve_open`'s re-gate rejects a non-finding
antecedent by type, no side conditions needed. The blocker is purely **where the type comes
from**: hole specs are assembled at SEED time with the fixed `Entity` type
(`parse/paths.rs` pushes `(hole_base(i,j), Entity, EntityRef)` per span), but a determiner's hole
learns its type only when the determiner **combines with its noun**, mid-derivation.

## 2. Mechanism: a polymorphic placeholder, freshened at the felicity gate

New bootstrap axiom (sibling of `lexicon:anaphor`):

```
axiom lexicon:anaphor_of : forall (A : Set) => A
```

Demonstrative sems (replacing the ι sems on the four surfaces, subject and object forms):

```
dem_subj_sem = λA. λV. V(lexicon:anaphor_of(A))      : ∀(A:Set) → (A → Prop) → Prop
dem_obj_sem  = λT. λTV. λs. TV(lexicon:anaphor_of(T), s)
```

The applied placeholder `anaphor_of(A)` is a **closed constant application** — it gate-passes
through the whole derivation like any sem (the kernel stays hole-free at rest), and by the time
the top-span felicity gate runs, β-reduction has made `A` concrete (the restrictor class the
noun supplied).

**Freshening moves to the felicity gate** for this placeholder: `classify_felicitous` walks the
candidate sem for `App(anaphor_of, A)` subterms, replaces each occurrence with a fresh variable
(deterministic traversal order names them; skeleton α-normalization already handles binder
names), and records `HoleInfo { var, ty: A, kind: EntityRef }`. The carrier abstraction and
everything downstream — `resolve_open`'s β-apply + closed re-gate, `resolve_with`'s
propose-then-search, `resolve_document`'s threading — is **unchanged**: the demonstrative hole is
just a hole whose `ty` is `Finding` instead of `Entity`.

Why not seed-time freshening (the pronoun protocol)? At seed time the restrictor is unknown — the
determiner hasn't met its noun. Why is felicity-time freshening safe where the pronoun needed
span-keying? Span-keying exists because *seed-time* variables must survive unary shifts across
spans; a *top-span* traversal has no such hazard, and two occurrences of `anaphor_of(Finding)`
within one reading are distinct subterm sites → distinct fresh variables.

**Number**: `these/those` keep `pl`, `this/that` keep `sg` in their `cat` — untouched; the
number feature continues to do its categorial work.

## 2a. Slice-2 findings (review points, answered empirically 2026-08-11)

Three interactions flagged in review, monitored as explicit slice-2 tests
(`kernel/tests/closed_class_determiners.rs`, the `yonder` fixture — a novel surface so the
bootstrapped ι demonstratives don't interfere; no lexicon change):

1. **The restrictor veto did not exist until enforced at application time — β-erasure.**
   `resolve_open` β-applies the antecedent and checks the *reduced* term, and evaluation ERASES
   the Π-binder's type annotation; after substitution the antecedent only meets the body's own
   argument types (typically a verb slot's wide `Entity`). The slice-2 veto test caught it: a
   `Gene` antecedent for a `CellLine`-typed hole resolved. Fix (landed): `resolve_open` checks
   `antecedent : Tᵢ` explicitly before applying each binding — the hole's declared type is now
   enforced by the checker's intensional resource-inhabits-class rule. This also strengthens the
   pronoun holes (their `Entity` check now runs on the same path).
2. **Subtyping/coercion — the veto is subsumption-aware, not over-strict.** The intensional rule
   (`nbe/check`: a resource inhabits `sup` iff one of its `is_a` classes is a
   reflexive-transitive subclass via `Layer::is_subclass_of`) accepts an antecedent typed by a
   SUBCLASS of the restrictor — verified: `hela_s3 : HeLaSubline <: CellLine` resolves a
   `CellLine`-typed hole. Semantic near-misses that are NOT subclasses in the ontology
   (`Result` vs `Finding` as unrelated classes) are still vetoed — bridging those is the
   proposer's job (§2.4 candidate pre-filter) or an ontology alignment fact, never a silent
   coercion.
3. **Multiple occurrences.** Two `anaphor_of(A)` sites in one reading are two distinct holes
   (verified: "yonder cell line affects yonder cell line" carries two independently-resolved
   holes; coreference = binding both to one antecedent). Resolution cost is `search_resolve`'s
   existing bounded per-hole depth-first search — plural/group reference to a SET of antecedents
   remains the §4 open point.
4. **Measurement shock** (§5) stands as planned: the ~19-unit flip to Open is a deliberate
   re-ratchet to honest numbers, executed as one migration with the reseed.

## 2b. Kind antecedents — the derived-kind-predication coercion (slice 4, kernel rule)

A kind referent is a TERM, `kind_of(K) : Entity` (Chierchia's ∩) — so a restrictor-typed hole
(`Library`, `CellLine`) vetoed every kind: inference gives `Entity`, and `Entity ⋢ C`. But the
grammar already indexes a bare-kind NP by `base(K)` so it sits in the subsumption lattice
(`LexicalIndex::kind_raised_nps`) — the sem-level check disagreed with the categorial system's
own indexing, and "…shRNA **libraries** … These libraries…" could never resolve. The fix is a
third intensional rule in the CHECKER (`nbe/check`), beside resource-inhabits-class (#91) and
CN-as-types subsumption: in check mode, `kind_of(K)` coerces into a class position `C` iff
`base(K)` — the Σ-spine-peeled base class — is a reflexive-transitive subclass of `C` (the
type-shift half of derived kind predication). Properties, each pinned by a test:
check-mode-ONLY (inference still gives `Entity`; definitional equality stays exact); it only
ADDS acceptances (on a miss the term falls back to plain inference, so `kind_of(K) : Entity`
keeps typing through the axiom's codomain — the first cut errored instead and broke a
witness-hash test); directional (a superclass kind does not narrow); a non-class base falls
through. End to end: "yonder gene" resolves to the harvested ⟦genes⟧ kind, and the CellLine
individual is still vetoed for the Gene-typed hole.

## 2c. The resolution search is explicitly bounded (slice 4 finding)

`resolve_with`'s docs claimed the assignment search was "bounded by the proposer's list
lengths" — a bound the kernel does not own: the proposer is UNTRUSTED input, and the
deterministic recency proposer (propose everything, let the veto filter) drove the first
close-out run to 50 minutes inside one cross-product (two-hole parses × every candidate pair ×
a full re-gate each). Two changes, both structural: (1) the restrictor veto is a
per-(hole, candidate) fact, so it now PRE-FILTERS each hole's candidate list linearly before
the search — the cross-product only ever enumerates hole-wise-typed assignments (this is also
where a hole with zero surviving candidates fails closed early); (2) the search caps full
re-gates at `MAX_REGATE_ATTEMPTS = 64` per open parse, fail-closed on exhaustion — the kernel
self-protects instead of trusting the proposer's list to be short. The full-page discourse pass
dropped to 25 s.

## 3. Alternatives considered

- **(B) Dual entries** — keep the ι reading and ADD the hole reading; the Stage-1 ranker +
  pooled competition (plan §2.2) choose. Migration-friendly (no measured unit flips outcome; the
  pins hold), but it makes the wrong reading a **standing competitor** the ranker must reject
  forever, on every demonstrative NP, at forest-growth cost. The house rule is to kill wrong
  readings at the source (copula-as-grammar, importer skips), not to let ranking hide them.
  Rejected as the end state; not worth building as a stage.
- **(C) Demonstrative marker + post-parse substitution** — parse closed as a distinct
  `ontology:dem(A)`, then rewrite marked sites against the discourse and re-gate. Keeps the
  single-sentence measurement almost unchanged (a mechanical skeleton rename), but invents a
  **second resolution pathway** — structural substitution inside arbitrary terms, outside the
  Π-carrier discipline — duplicating what holes already do, with a subtler binder-safety story.
  Rejected: the carrier IS the trust boundary; one resolution mechanism.

**Decision: (A) replace.** Demonstratives become restrictor-typed holes; plain `the` stays ι.

## 4. What the antecedents are — the 2.3 dependency

The corpus's demonstrative units split by referent kind:

- **entities/kinds**: "these data sets" (Achilles+DRIVE), "These libraries", "these lines",
  "This state", "these four lineages" — resolvable once `Candidate` carries kinds (plan §2.3);
  some already resolve as named individuals.
- **prior claims**: "These findings show…", "These observations suggest…", "These
  classifications…" — the antecedent is a *committed claim* (or a group of them), which needs
  §2.3's claim-IRI candidates and, for groups, plural reference to a SET of antecedents — noted
  as the successor of D64's deferred joint-multi-hole question, not solved here.

So the lexicon change lands **with** §2.2 (pooled closed∪resolved-open competition) and §2.3
(candidate enum) — a demonstrative unit only leaves `Open` when its referent kind is
representable. Units whose referents are not yet representable stay honestly `Open` (fail-open,
never a wrong closed parse) until 2.3 catches up.

*Status (slice 4, 2026-08-11):* entities and kinds are live (§2b coercion; measured in the
slice-4 record below); claims, plural sets, quantifier witnesses, and Σ-restrictor discharge
are the four residual referent kinds, each named in the close-out's residual list.

## 5. Measurement migration (bootstrap edit ⇒ reseed; batch it)

`closed-class.esl` and `ontology.esl` are both bootstrap-embedded — the sems change forces a
reseed (~15 min + alignment) and retires current snapshots. Consequences, planned rather than
discovered:

- **Single-sentence sweep**: the ~19 demonstrative units lose their closed ι readings; where no
  other closed reading exists they flip to **Open** — which the harness already measures and
  which is skeletonizable/pinnable (the Π-carrier is a well-typed parametric proposition).
  Coverage (`grammar-gap 0`, `missing-lexeme 0`) is untouched — Open is not a gap. `encoded` /
  `ambiguous` / readings / skeletons totals shift; ceilings re-ratchet to the honest numbers.
- **Pins**: affected units re-pin (their correct skeleton becomes the hole-carrying form —
  same bracketing, `the(§)` → the abstracted-hole shape). Mechanical for units whose structure
  is otherwise unchanged; the pin corpus records the migration in its notes.
- **Ledger**: changed skeletons re-adjudicate (`audit-skeletons` fails closed until done — by
  design).
- **Ranks**: closed-class surfaces are not sense-ranked; the recording's keys should HOLD.
  Verified by replay, not assumed.
- **Selections**: the 19 units' candidate sems change → those keys MISS → a fresh reference
  draw + reading-level adjudication of the new decisions → `selection-baseline.json`
  re-derived. (The parse `baseline.json` and the selection baseline re-derive together in this
  one migration — the two-file split keeps the provenance of each honest.)
- **Discourse measurement**: the DB-backed `resolve_document` corpus test (plan §2.5) becomes
  the place where demonstrative units are expected to CLOSE — the single-sentence sweep
  correctly reports them Open (no discourse, no referent).

## 6. Slices

1. **This note** — review gate.
2. **Kernel mechanism** — DONE (2026-08-11). `holes::freshen_anaphor_of` (felicity-time
   freshening of applied polymorphic placeholders, `$demref$k_0` naming — skeleton-normalizer
   compatible), restrictor-typed `HoleInfo` via the felicity gate, and the **hole-type veto in
   `resolve_open`** (the β-erasure finding, §2a). Four tests over the `yonder` fixture: typed
   open parse, type-wrong veto, subclass acceptance, two independent holes. No lexicon change —
   the bootstrapped ι demonstratives are untouched; full suite green.
3. **Lexicon swap + reseed** — DONE (2026-08-11). `anaphor_of` axiom + `dem_ref_*_sem` in
   `closed-class.esl`; all 8 demonstrative entries swapped (`the` untouched; the `that`
   complementizer untouched); reseed → `wordnet-umls-aligned-2026-08-11-dem`. The §5 migration,
   measured: ranks replay **62/0 held**; open 2→20, ambiguous 46→31, encoded 14→11,
   total-readings **761→226** (the retired ι readings carried most of the page's sense
   multiplicity), skeletons 144→139; **19 units re-pinned** by the mechanical ι→hole transform of
   their verified skeletons, the 2 documented misses kept — **expected-hits 60/62 holds**;
   ceilings re-ratcheted 1900→500 / 400→250. Selection re-drawn (eligible 46→31 — flipped-Open
   units left the pool): chose 31/31, reading-correct 21/31, invalid 0; 3 novel decisions
   adjudicated (one CORRECTED a prior wrong pick). Skeleton ledger: rebuilt from the pre-migration
   commit; with the typed instrument (§5a) ledger + pins cover ALL skeletons — no wave.

   **§5a — migration finding: hole types are invisible to skeletons — FIXED same day.** A
   demonstrative NP's internal restrictor structure ("data sets **for genes that…**") lives in
   the hole's TYPE (`HoleInfo.ty`); the carrier's `Exp::Lam` binders are untyped, so the plain
   sem skeleton cannot print it — pins stopped discriminating attachment *inside* the NP and
   ledger rows could not be carried. The fix: **`OpenParse::skeleton()`** prints
   `λ(h : ⌈T⌉). ⌈body⌉` through ONE `erase_senses` pass (binder names and body occurrences
   co-normalize); the harness keys open readings on it. Results: skeletons 139 → **144** (the 5
   splits are RECOVERED structure — open readings previously merged though differing in hole
   type); the 19 re-pins re-keyed mechanically by untyped projection; the one genuine split
   ("We analysed these data sets…" — FIVE attachment variants inside the NP, now visible)
   adjudicated to the fully-nested restrictor matching the pre-migration verified pin verbatim;
   the ledger rebuilt from the pre-migration commit via ι/typed detransforms. **Coverage is
   complete: 84 ledger rows + the 60 pinned correct skeletons cover all 144 — the provisional
   "74-row wave" was an artifact of the untyped instrument (and of counting pin-covered
   skeletons as ledger debt) and is discharged.**
4. **Discourse close-out** — DONE (2026-08-11). Plan §2.2 + §2.3 landed:
   - **§2.2 pooled competition** in `resolve_document`: pool = closed readings ∪ open readings
     whose holes resolve (EVERY open parse tried, sem-level dedup); pool of one → `Encoded`,
     several → ranker or `Ambiguous` (fail-open), none → `Open`/`Gap`. A closed reading no
     longer silently kills the anaphoric one (pinned by the `yonder cell line`
     named-entity-vs-demonstrative competition test); an unresolvable anaphoric reading leaves
     the closed one alone.
   - **§2.3 candidates**: `Candidate` is an enum — `Individual { iri, surface }` /
     `Kind { term, surface }` — with READABLE surfaces (the individual's layer label via
     `resource_label`, the kind's verbalized gloss over its sentence's own sense names). The
     proposer now selects BY INDEX among the assembled candidates (it can no longer introduce
     its own IRIs — which is also what lets a kind, a term with no IRI, be a candidate at all).
     Kinds harvest as CLOSED `kind_of(…)` subterms of each resolved sem; the candidate set
     dedups by identity, most-recent-first. Landed-claim candidates stay pending Stage 3
     (undecided: claim-resource typing, plural/group reference) — documented on the enum.
   - **Kind typing** became a kernel rule (§2b) and the resolution search an explicitly bounded
     one (§2c).
   - **Measured** (`resolve_document_discourse_close_out`, DB-backed over the page: dem
     snapshot + ranks replay, recency proposer, no ranker — PINNED in the test): encoded
     11→12, ambiguous 31→35, **open 20→15**, gap 0; discourse pass 25 s; sense-rank replay
     faithful. The 5 closures: «These data sets are project Achilles and project DRIVE» →
     ENCODED, its demonstrative resolved to the harvested kind ⟦data from large-scale
     silencing screens⟧ — unit 12's referent, the intended antecedent; «This state…», «These
     groups…», «These cell lines contained fewer…» → Ambiguous pools for the Stage-1 ranker;
     and «The lines from rare lineages…» (a PRE-migration Open unit — its elided comparative
     standard) → all 48 readings resolve into the pool. The isolated-sentence sweep is
     untouched: the full replay holds every baseline exactly (readings 226, skeletons 144,
     hits 60/62, selection 21/31, eval exit 0).
   - **Residual 15 Opens, by referent kind** — each a named deferral, none a defect:
     claim referents («These findings/observations…», 4 units — §2.3 claim candidates on
     Stage-3 landing); plural/group sets («these two events», «These libraries…» — D64 §4's
     deferred set-antecedent question); quantifier-introduced witnesses («This impairment…» —
     an existential's witness is not yet harvestable as a candidate); Σ-restrictored holes
     («…these data sets for genes that…» — resolving one means discharging its restrictor
     content against the referent, a presupposition-accommodation decision not yet taken; the
     page splits 51 plain-class vs 25 Σ-typed demref binders).
5. **Proposer upgrades (plan §2.4) + doc sync (§2.5)** — DONE (2026-08-11). `ProposeCtx` now
   carries the SAME `DocumentContext` as the reading ranker (document + target sentence + prior
   selections, threaded unconditionally through the loop) plus the hole's restrictor type;
   number features await a felicity-gate carrier change (noted on the struct). The trait answers
   a `Proposal { ranked, rationale, confidence }`; `AnthropicProposer` prompts with the document
   + the type-pre-filtered candidates (`EIGENIUS_DUMP_PROPOSE_PROMPT` dumps it). Record/replay
   landed as `RecordingProposer`/`ReplayProposer` (`dcg/proposer_record.rs` — key covers
   sentence, document sha, priors, hole var+type+kind, presented candidates; a recorded refusal
   replays as a hit; a miss answers empty, fail-closed, counted; the recorder MEMOIZES repeat
   questions — the worst unit carries 48 same-hole open parses). The close-out harness gained
   the `EIGENIUS_PROPOSALS` three-arm discipline (exists→replay / absent+live→record /
   unset→recency); the pinned recency baseline (12/35/15/0) holds under all of it.
   `docs/design/d64-llm-anaphora-resolution.md` §3/§4 synced to the as-built Π-carrier and
   in-process resolver. A live reference draw + referent-level adjudication is future
   measurement work — the recency floor is the tracked deterministic arm.
