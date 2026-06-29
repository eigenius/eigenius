# D62 — the encoding output contract: graded propositions + typed obligations

*Design contract. Fixes what the encoding pipeline **returns**, so the institution that wraps it
(§5) and the `ParseSentence`/encoding service surface (§6) are designed to it rather than retrofitted.
Grading: **Derived** = grounded in current code / the WRN encoding; **Declared** = a design choice.*

## 1. The contract (Declared)

Encoding prose does **not** yield a flat set of true propositions. It yields a **partial theory** — a
**reasoning-chain fragment**:

> **`encode(text) → ({ graded propositions }, { typed obligations })`**

- a set of **propositions** the text asserts (each a felicitous `Prop`, **Declared** grade by
  default), and
- a set of **obligations** — the outstanding warrants those propositions require to be *grounded*
  (raised above Declared).

A claim that merely parses is only **Declared**: the kernel certifies it *type-checks*, not that it is
*grounded*. Surfacing the obligations as first-class output is the mechanism that closes the
faithfulness gap (kernel-passes ≠ faithful — D61): the grounding conditions are made **explicit and
gated**, never silently assumed.

## 2. Typing (Declared)

- **Proposition** — a kernel-gated `Prop`, carrying a **grade** (Declared on emit) and **zero or more
  attached obligations**. Obligations are indexed *to the proposition they warrant*, not a free pile.
- **Obligation** — a **typed slot**: the `Prop` it must establish + the **witness kind** that
  discharges it, which sets the grade it unlocks (§4). Discharging an obligation raises its
  proposition's grade (e.g. Declared → Derived).

A measurement adverb (`docs/notes/d62-adverb-semantics-decision.md` §4a) yields **one** Declared
differential proposition **carrying** a measurement obligation; discharge it and the same proposition
climbs to Derived.

## 3. Two hole families — only one is an output obligation (Derived)

The D64 open-parse carrier produces two kinds of hole (`kernel/src/dcg/lookup.rs`, `HoleKind`), and
they have **different fates** in the contract:

- **Reference holes (`EntityRef`** — pronouns/possessives) are **resolution** obligations — *internal
  completions*. The resolver (D64 proposer-behind-oracle, `resolve_with`) binds an antecedent and
  **closes** the parse into a finished proposition. They do **not** appear in the output's obligation
  set; they feed the *proposition* set.
- **Proof/measurement holes (`ProofObligation`** — factive presupposition, measurement adverbs) are
  the genuine **output** obligations. They **survive** parsing and are discharged downstream by
  measurement / derivation / proof.

So: `EntityRef` holes resolve *into* the proposition set; `ProofObligation` holes *are* the obligation
set. (Today only `EntityRef` is implemented; `ProofObligation` is the planned arm — see the adverb
note §4a and the carrier note.)

## 4. Witness-kind → grade → discharging institution (Derived from the WRN chain)

| Obligation witness | Discharger | Grade unlocked |
| --- | --- | --- |
| **measurement** | statistics institution → `stats:StatisticalAnalysisResult` | Derived |
| **derivation / computation** | ProgramTrace → `IsDerivedAs` | Derived |
| **proof** | Lean / kernel | Verified |
| **grounding / discovery** | retrieval (D43) | an anchored fact |

The WRN chain (`experiments/publications/wrn-helicase/chain/`) is this contract fully discharged: a
Declared claim (`SelectiveViabilityDependence(WRN, MSI)`) + a measurement obligation discharged by a
recomputed result + a justification-logic certificate (`DeclaredEvidence`/`DerivedEvidence`/
`VerifiedEvidence`) raising the grade.

## 5. What this means for the institution wrapping the pipeline (Declared)

The parser is **untrusted proposer**; the **encoding institution** is the trusted wrapper (§8.8.2–3 in
`lookup.rs` — "selecting one parse and committing it is the institution's job"). The contract shapes
it as follows:

1. **Consume both forests.** The parser returns `(closed, open)` (`parse_scoped_open`). The
   institution disambiguates/selects among closed parses and runs the resolver over `EntityRef`-open
   parses to close them — both feed the **proposition** set.
2. **Commit a proposition as a graded witness** — a `lexicon:Sentence` / reasoning claim at
   **Declared** grade, carrying references to its attached obligations.
3. **Commit each `ProofObligation` as a first-class open node** — an objective-shaped milestone /
   `objective:CompetencyQuestion`-style discovery target (D61), or a `stats:StatisticalAnalysisPlan`
   for a measurement — **open until discharged**. The institution **never drops an obligation
   silently** (fail-closed; the reasoning protocol). An undischarged obligation is *recorded as open*,
   not ignored.
4. **Route obligations to the discharging institution** (§4) and, on discharge, **upgrade the
   proposition's grade** via the witness (the justification-logic certificate). This makes the
   encoding institution a **discourse-level proposer-behind-oracle**: it proposes claims+obligations;
   the measurement/reasoning/proof institutions are the oracles that ground them.

Net: the institution's commit surface is **"a graded proposition + its typed obligations,"** and it is
inherently a *bridge* to the statistics / reasoning / Lean / grounding institutions — not a terminal
"prose → Prop" sink.

## 6. Surface impact (Declared)

`ParseSentence` today projects only the **closed** forest to the wire (`kernel/src/server/parse.rs`).
To honor the contract the encoding surface must return **both**: the graded propositions **and** the
typed obligations (each: the `Prop` to establish, witness kind, and the proposition it warrants) — so
a client/institution can commit the Declared claims and open the obligations. This is the extension
flagged in the prototype findings ("`ParseSentence` must report the open forest + missed tokens").

## 7. Relation to existing structure (Derived)

- **Grade ladder** (Observed/Declared/Derived/Verified) — the contract *is* the grade ladder applied
  to encoding output: parse ⇒ Declared; discharge an obligation ⇒ Derived/Verified.
- **D64 carrier** — supplies the hole machinery; §3 assigns the two families their roles in the
  contract.
- **D61** — an open obligation *is* an open competency question / discovery target; the contract is
  how encoding emits them.
- **WRN chain** — the worked, fully-discharged instance (§4).
