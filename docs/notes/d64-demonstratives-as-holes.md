# D64 — demonstratives as referent holes (Stage 2 of the parser-pipeline plan, §2.1)

**Status: design note for review — precedes any code.** Parent map:
[parser-pipeline-plan.md](parser-pipeline-plan.md) Stage 2; the resolver machinery this feeds is
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
   adjudicated (one CORRECTED a prior wrong pick). Skeleton ledger: 45 kept + 20 migrated + 55
   stale dropped + **74 new-unadjudicated pending a wave** (see §5a).

   **§5a — migration finding: hole types are invisible to skeletons.** A demonstrative NP's
   internal restrictor structure ("data sets **for genes that…**") now lives in the hole's TYPE
   (`HoleInfo.ty`); the carrier's `Exp::Lam` binders are untyped, so `pretty_term`/skeletons
   cannot print it. Consequences: (a) pins on such units discriminate attachment *inside* the
   demonstrative NP less than before (recorded in the affected pins' notes); (b) most old ledger
   rows for these units could not be carried mechanically — hence the 74-row wave. Instrument
   fix, future: print each hole's type alongside the skeleton (the `OpenParse.holes` carry it) in
   the dump/ledger keys.
4. **Discourse close-out** — with §2.2/§2.3 in place: the corpus page through DB-backed
   `resolve_document`, demonstrative units resolving to entity/kind antecedents; claim-referent
   units documented as pending §2.3's claim candidates.
