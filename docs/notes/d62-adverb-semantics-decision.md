# D62 — adverb semantics: a decision (scope to science; transparent bulk + justification-routed measurement)

*Design-decision note. Records how the encoding engine (D62/D63) handles adverbial modifiers, why we
**do not** build Davidsonian event semantics for them, and what the load-bearing minority routes to
instead. Grading per the working protocol: **Derived** = witnessed by the run/ontology/grammar cited;
**Declared** = a design choice; **Deferred** = explicitly out of scope with a revisit trigger.*

## Decision (the short version)

For encoding **science**, adverbs are mostly **not load-bearing** and are handled **transparently**
(parse-to-compose, contribute nothing to the claim `Prop`). The minority that *are* load-bearing are
**measurement/quantification-associated** (`selectively, preferentially, significantly, highly,
predominantly, …`). For those, the WRN study's own encoding gives the pattern (§4a): the adverb is a
**prose trigger for a typed differential/graded domain predicate over a contrast, warranted by
measured evidence and carried in a justification-logic certificate** — *not* an adverbial operator,
and *not* a manner predicate on a reified event. **Davidsonian event semantics is therefore not built
for adverbs** — it is decoupled and deferred (see §5).

The only mechanical prerequisite that stands either way is the **`-ly` reverse-derivation rule**
(Phase 3) — needed to *recognize* the token at all (WordNet doesn't store productive `-ly` adverbs).

## 1. Why this came up (Derived)

After Lever A/B cleared parsing scale and the fresh-DB run measured encoding over the full
WordNet+UMLS lexicon (`docs/notes/d62-encoding-prototype-findings.md`, 2026-06-28), `-ly` adverbs are
a standing residual on the OOV gate — and, critically, **OOV even over full WordNet**
(`has_token("commonly") = false`): WordNet's adverb file is small/lexicalized and Morphy is
inflectional, so productive `-ly` adverbs are neither stored nor derived. Recognizing them needs the
derivational rule; *interpreting* them is this note's question.

## 2. The cut, against the real data (Derived)

The WRN-page `-ly` OOV set splits cleanly along the science-load line:

- **Inert for a scientific claim (~64%):** `commonly, correspondingly, favourably, simply, typically,
  respectively, independently`. No truth-conditional load — "WRN is *commonly* mutated" asserts, for
  encoding purposes, the same fact as "WRN is mutated".
- **Measurement / quantification-laden (~36%):** `selectively, preferentially, predominantly, highly`.
  Load-bearing: "**selectively** kills MSI cells" is a *differential/contrastive* claim; "**highly**
  expressed" / "**predominantly**" are degree/majority quantifications; "**preferentially**" is a
  comparative. Dropping these loses the science.

The load-bearing ones are not *manner facts about an event*; they are **second-order, evidential
claims about the predication** (the killing is *selective* = a measured differential vs MSS). That is
the semantic content justification logic exists to carry.

## 3. What the reference grammar does (Derived — `references/openccg/grammars/core-en/adv.xsl`)

core-en (OpenCCG CCG) gives three adverb families (`Adverb`, `Loc-Adverb`, `Transitional-Adverb`),
each with positional categories: **Initial** `s/s` (sentence-fronted), **Forward** `(s\np)/(s\np)`
(pre-VP manner), **Backward** `s\s` (post-sentential), and a transitional variant taking a trailing
comma. Its **semantics is HLDS** — `@E:situation(<HasProp>(P ∧ [*DEFAULT*]))` — i.e. **Davidsonian
event-decoration**: the adverb adds a `HasProp` predication to the event, defaulted to the lexical
stem (locatives use `<Location>`). We adopt core-en's **categories** but re-target its semantics to
EigenTT; the HLDS event-decoration is exactly the part we are deciding *not* to replicate (§5).

## 4. The decision in detail (Declared)

The `-ly` derivation rule (Phase 3) recognizes the adverb; the **sem is a two-way routing** rather
than an event-manner record:

1. **Recognize** — reverse-derive `-ly` → adjective base (`-ily→-y`, `-ally→-ic/-al`, `truly→true`,
   else strip); if the adjective resolves, emit an adverb-categorized item (category from `adv.xsl`).
2. **Inert bulk → transparent.** Sem = **identity** (`λV. V`) at the modifier category, so the
   sentence composes and the adverb contributes nothing to the claim `Prop`. This is a **deliberate,
   recorded cut** (these carry no scientific load), not a silent drop — captured as a CutItem-style
   disposition, reversible if a downstream task needs manner.
3. **Measurement / quantification subset → the WRN measure+evidence pattern (§4a).** A small
   **curated class** that triggers a differential/graded **domain predicate**, raises a **measurement
   obligation**, and is **graded via a justification-logic certificate** — detailed below.

The one genuinely new artifact is the **inert-vs-measurement classification** — a science-tuned list,
crossed with `adv.xsl`'s category split (manner VP-modifier vs sentential vs transitional).

## 4a. The measurement subset, grounded in the WRN encoding (Derived)

The WRN study (`experiments/publications/wrn-helicase/chain/`) already encodes the very claims our
measurement adverbs name — *"WRN is **selectively** essential"*, *"**preferential** dependency"* — and
it does **not** use an adverbial form. It encodes them as a **typed domain relation over a contrast,
warranted by measured evidence**:

- **Claim = a differential domain predicate.** `onco:SelectivelyEssential(gene, "MSI")`,
  `onco:TopDifferentialDependency("WRN", "Achilles_MSI")`, `SelectiveViabilityDependence(WRN, MSI)` —
  the selectivity is a **differential between two populations** (MSI vs the complement MSS), baked
  into the predicate. Not `selectively(kills(…))`.
- **Warrant = a measured statistical result.** A recomputed test (limma moderated-t / Wilcoxon /
  Spearman) emitted as a `stats:StatisticalAnalysisResult` (`reflection:InstitutionEmittedDerivation`)
  via a ProgramTrace → `IsDerivedAs` witness (e.g. *WRN rank = 1, Q = 4.81e-24*).
- **Grade = a justification-logic certificate.** `DeclaredEvidence` / `DerivedEvidence` /
  `VerifiedEvidence` + `app / declared / derived / verified` / `reasoning:App` tie claim ↔ evidence;
  the grade *is* the strength of the measured evidence.

So a measurement adverb is the **prose trigger** for that pattern, contributing the *contrast/degree
dimension* and an *obligation* — not its own content (the numbers come from the study's data). The
encoding of "X **selectively** kills MSI cells" is therefore:

1. **Select a differential/graded predicate template** (small curated map): `selectively` /
   `preferentially` → a contrast-dependence predicate over (target population, complement);
   `significantly` → a significance-qualified claim; `highly` / `predominantly` → a degree/magnitude
   claim.
2. **Raise a measurement obligation.** The claim is **Declared** with an open obligation that a
   `stats:StatisticalAnalysisResult` (the measured contrast) must discharge to **Derived/Verified**.
   This is a **measurement obligation of the same family as the D64 `ProofObligation` hole** (the
   factive/presupposition arm, deferred in `docs/notes/d62-d64-open-parse-carrier.md`): the adverb
   opens an obligation on the predication that grounding/measurement discharges — reusing the
   open-parse carrier, not a new mechanism.
3. **Carry it in a justification certificate** — the grade + the claim→evidence link (the chain's
   `…Evidence` + certificate combinators).

The kernel stays the felicity oracle throughout (the predicate is typed, the obligation is a hole, the
certificate is the reasoning institution's); the adverb adds no opaque kernel operator.

## 5. Why Davidsonian events are deferred (Declared + Deferred)

The faithful-manner route is **neo-Davidsonian event semantics**: reify the verb's event and let the
adverb be an intersective predicate on it. We investigated the full path and it is *coherent* with our
foundations, but **not justified by adverbs for science**:

- **Root type — Act, not Event (Derived).** `schema:Action` is already mapped (D57) and *is* the
  neo-Davidsonian frame in its own words ("an action performed by a direct agent and indirect
  participants upon a direct object … may produce a result"), with the role inventory already present
  as **advisory `recommends`**: `agent, object, instrument, participant, result, location, startTime`,
  plus a ~14-branch action taxonomy. `schema:Event` is *scheduled happenings* (concert/lecture) — the
  wrong root. Verb synsets would root under `schema:Action` exactly as noun synsets root under
  `Entity`.
- **The import is cheap (Derived).** Verb `sem`/`subclass_of` are emitted strings in the converter;
  rooting verbs under `schema:Action` + an event sem is a converter-rule change + reseed
  (`scripts/reseed-lexicon-db.sh`), not 325k edits.
- **The real seam — property-as-relation (Derived).** A resource-typed `core:Property` carries
  `class_types` (range) and the kernel **validates resource-valued property assertions** against it —
  so a reified event *resource* with `schema:agent = <Person>` edges already type-checks (the
  graph-assertion direction works). But a property is **not a term**: `resolve_sem` maps it to a
  (non-applicable) `EigonResource`; there is **no property-as-relation path**, so the event-record
  *Prop* (`∃e:Action. affect(e) ∧ agent(e,subj) ∧ object(e,obj)`) cannot use `agent` as a `→Prop`
  relation. (And `core:domain` is absent — D57 put domain on the class as `recommends` — so the
  relation signature is split: range on the property, domain on the class.) Closing this needs either
  **(a)** a small set of paired role-relation **axioms** `Action → Entity → Prop` (reuses the
  verb-axiom machinery, zero kernel change) or **(b)** a **property-as-relation kernel feature**
  (unified, but a real type-theory extension).

**Decision:** none of that is on the adverb critical path. Events / `schema:Action` / property-as-
relation are **decoupled from adverbs and deferred.** They remain the right answer *if* we later want
**verb-event reification for knowledge-graph alignment** (events as reified nodes with typed role
edges) — but that stands on its own merits, as a separate decision, with the (a)/(b) fork captured
here for that day. **Revisit trigger:** a task that needs to query/relate the *internal structure* of
events (roles, manner, time) rather than the propositional claim.

## 6. Cost / benefit (Declared)

- **No events, no property-as-relation, no kernel type-theory change** for adverbs. The `-ly` adverbs
  leave the OOV gate via derivation + transparent/justification routing.
- **Faithfulness:** dropping the inert bulk is justified (no scientific load, recorded as a cut);
  the measurement subset is routed *more* correctly than manner-on-event would (evidential/graded,
  which is what those adverbs mean).
- **Remaining work is bounded:** the `-ly` derivation rule (Phase 3, unchanged), the curated
  inert-vs-measurement classification + `adv.xsl` categories, and — for the measurement subset — the
  justification-logic encoding of a graded qualifier (likely its own slice, gated on the
  justification-logic surface).

## 7. Open questions (Declared)

- The exact `adv.xsl` category assignment per class (manner `(s\np)\(s\np)` / sentential `s\s` / `s/s`
  / transitional-with-comma) — pull when implementing.
- The **differential-predicate template map** (§4a.1): which predicate each measurement adverb selects
  (`selectively`/`preferentially` → contrast-dependence; `significantly` → significance-qualified;
  `highly`/`predominantly` → degree). Do these reuse existing `onco`/`stats` predicates or need a few
  general ones?
- **Contrast-complement inference**: `selectively` is differential *vs what?* The WRN encoding names
  the complement explicitly (MSI vs MSS). From prose alone the complement is often implicit — likely
  part of the measurement obligation to resolve, not derivable from the sentence.
- Boundary cases in the classification (`predominantly`, `independently` — judgment calls).
- Provenance for any future events work: the synthesis note that prompted this (Davidsonian +
  dependently-typed CCG with records) is an **unverified proposal**; anchor on primaries (Davidson
  1967; Parsons neo-Davidsonian; Luo MTT-semantics; Cooper TTR) before committing — do not cite the
  proposal as authority.
