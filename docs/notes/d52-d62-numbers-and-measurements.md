# Numbers & measurements: how prose quantities flow (or don't) into typed trees

*Open-question / design note, 2026-06-29. Cross-cuts the kernel (numeric literals), **D52** (the
measurement/statistics institution, `docs/design/d52-measurement-statistics-institution.md`), and
**D62/D63** (the prose→tree encoding/grammar). Written before extending the grammar further, to pin
down how quantitative content is — and is not — represented today, and what "doing numbers properly"
would require. Grading: the current-state map is **Derived** (read from code, file:line cited); the
proposed work and open questions are **Declared**.*

## TL;DR

There are **two disconnected number worlds**. The kernel and the D52 *verifier* compute with real
`f64`/`i64`; the prose→tree *parser* drops numbers at the door, drops cardinality, treats units as
strings, and builds gradable adjectives on opaque axioms that never reduce. The kernel-side targets
all exist; **the missing thing is the prose→number extraction path.**

## 1. World 1 — kernel + D52 verifier: real numbers (Derived)

- Kernel numeric literals/types: `Exp::LitInt(i64)` / `Exp::LitFloat(f64)`
  (`kernel/src/nbe/term.rs:122,126`), `PrimitiveType::Integer|Float` (`:391-392`), values
  `Val::LitInt|LitFloat` (`kernel/src/nbe/val.rs:75,78`). Full eval/readback/check support
  (`eval.rs:208-209`, `readback.rs:195-196`, `check.rs:1102-1105`). `MinValue`/`MaxValue`
  constraints compare real `f64` at check time (`eval.rs:2024-2042`). `core:float`/`core:integer`
  are core types (`ontologies/core/core-ontology.json:70,191,258`).
- **D52** lives in `ontologies/statistics/statistics.esl` (`namespace stats =
  "urn:eigenius:measurements"`, `:55`). It is a **statistics institution**: a `Claim` schema with
  `alpha`, `EffectSize` (Absolute magnitude+units, Relative/fold-change, Cohen's d, Hedges' g, η²/ω²,
  `:140-152`), experimental-design types, `Replicate{value:core:float, unit_id:core:string}`
  (`:310-321`), float-ordering predicates `stats:lt/le/gt/ge : core:float → core:float → Prop`
  (`:266-269`), and statistical functionals (`mean_of`, `variance_of`, … `:252-264`). Its **verifier
  recomputes test statistics in real `f64`** (`crates/eigenius-statistics/src/numerics.rs`,
  `tests/wilcoxon_wrn.rs`).

## 2. World 2 — the prose→tree parser: numbers dropped / opaque (Derived)

- **Numbers in prose are filtered out.** `is_nonprose` (`kernel/src/dcg/segment.rs:107-110`) routes
  any token starting with a digit or with no alphabetic char out of the parse: `14`, `0.56`,
  `10−13`, `37`, `=` never reach the grammar. `tokenize` (`kernel/src/dcg/lookup.rs:91-128`) trims
  non-alphanumeric edge chars, so `P =`→`p`, `n =`→`n` (1-char "stat-symbol leaks",
  `crates/eigenius-wordnet/tests/db_backed_encoding.rs:548-562`), and `10−13` splits into `10`+`13`
  (scientific notation not even reassembled).
- **Cardinality is dropped.** Cardinal numerals `two`..`ten` (`ontologies/lexicon/closed-class.esl`,
  added 2026-06-29) are plain existential plural determiners — `two genes` ≡ `∃ genes` (`exists_sem`).
  **No `Card`/counting predicate exists anywhere** (only comments promising one).
- **Gradable adjectives are opaque.** "X is large" = `measurements:gt(deg_large(x), std_large)`;
  `deg_large : Entity → core:float`, `std_large : core:float`, and `measurements:gt` are **axioms
  with no reduction rule** (`experiments/lexicon/lexicon.esl:158-176`; an `EigonAxiom` evals to a
  blocked neutral, `eval.rs:430-431`). So the float is never an actual number and `gt(…)` is an
  inert `Prop`, not a computed truth. (`measurements:gt` and `stats:gt` are the same IRI.)
- **Units are untyped strings** — `EffectSize.Absolute(magnitude, units:core:string)`,
  `Replicate.unit_id:core:string`. No dimensions, no unit algebra, no conversion. D52 does not plan a
  unit type.

## 3. Faithfully represented today? (Derived)

| Thing | Status |
|---|---|
| A `core:float`/`core:integer` *value* in the kernel | Real — computed, range-validated |
| A D52 `Claim` (hand-authored) | Real — verifier recomputes in `f64` |
| A number **in prose** (`14 cell lines`, `n=37`, `P=4.2×10⁻¹³`, `0.56-fold`) | **Dropped** (non-prose filter) |
| Cardinality of a numeral NP | **Not represented** (count dropped) |
| Degree of a gradable adjective | **Opaque axiom**, never a number |
| Units / dimensions | **Bare strings**, no algebra |

The kernel-side targets all exist (`LitInt`/`LitFloat`, `EffectSize`, `Claim`); the verifier numbers
in `crates/eigenius-statistics` come from a hand-built `SampleSet`, **not from parsing the paper**.
The missing piece is the prose→number extraction path connecting World 2 back to World 1.

## 4. What is deliberate vs a gap (Derived — recorded decisions)

- **By design:** "Route non-prose out of the parser: equations → FormulaTerm/EigenTT, citations →
  Reference" (`docs/design/d62-encoding-engine-prose-to-trees.md:117,62`). Dropping `n=37`/`P=…` from
  the *grammar* is intended — they should be **routed to a typed equation/stat term**. The gap:
  **that routing target is not built** — they are currently filtered, not re-injected.
- **Numerals existential, count dropped** — "a faithfulness refinement (a `Card`/measure predicate,
  tying to D52)" (`closed-class.esl` numerals block; `docs/notes/d62-grammar-gap-analysis.md` §2 #4).
- **Digit numerals deferred** — "need a generative numeral tokenizer hook."
- **Degrees reuse D52, opaque** (`docs/design/d63-dcg-engine-english-grammar.md:403-408`).

## 5. Proposed work — three roughly-independent pieces (Declared)

1. **Numeral / measure-phrase grammar → real `LitInt`/`LitFloat`.** A digit-numeral tokenizer hook;
   a `Card(set, n:core:integer)` counting quantifier (closes the count-dropped gap); `N-fold` / `N %`
   measure phrases feeding D52 `EffectSize.Relative`/`Absolute`. Mechanical once the target shapes are
   fixed. Parser + small ontology; the `Card` predicate is a new core/measurements axiom.
2. **Stat-expression routing → typed `stats` nodes.** Implement the D62 S0 "route to a typed stat
   resource" step so `n = 37`, `P = 4.2×10⁻¹³` become D52 `stats` resources carrying real
   `LitInt`/`LitFloat` (reassembling scientific notation), instead of being dropped — connecting the
   prose path to the existing verifier (`crates/eigenius-statistics`). Mostly S0/tokenizer + a small
   equation grammar; the kernel/verifier targets already exist.
3. **A real quantity / unit type (dimension + unit algebra).** The biggest and most genuinely
   design-first piece. D52 currently punts units to `core:string`; unit-checked measurements need a
   `Quantity`/`Unit`/`Dimension` model (SI base dimensions, unit algebra, conversion). Deserves its
   own deliberation and prior-art grounding (the D61 method) — *not* a grammar slice.

## 6. Open design questions (Declared)

- **Counting quantifier semantics.** Is `Card` exact ("exactly n") or lower-bound ("at least n", the
  usual numeral semantics)? Does `two genes affect X` mean `∃≥2` or `∃=2`? (Linguistics says
  ≥-with-scalar-implicature; a faithful first cut is likely "≥n" with the implicature out of scope.)
  How does it compose with the existing existential determiner machinery (a `Card` conjunct on the
  restrictor vs a distinct counting quantifier)?
- **Stat-resource shape from prose.** What typed node does `P = 4.2×10⁻¹³` become — a bare
  `stats:value` literal, or a partial `stats:Claim` to be completed/verified? How does an inline stat
  bind to the clause it qualifies (apposition/parenthetical attachment, ties to #1 apposition)?
- **Units: type vs annotation.** Full dimensional type system (unit errors are type errors) vs a
  lighter typed-annotation that the institution checks? Adopt/borrow an existing standard (QUDT, UCUM,
  OM) rather than mint — a grounding question for the D61 method.
- **Where do general (non-statistical) measurements live?** D52 is a *statistics* institution; a
  general `Quantity` may belong in its own layer, not under `urn:eigenius:measurements` (which is
  currently just the stats namespace).

## 7. Recommendation

Pieces **1** (cardinality + digit numerals → real `LitInt`) and **2** (stat-expression routing) are
mechanical slices once their target shapes are pinned — do them when ready. Piece **3** (quantity/unit
type) is a **D52-extension design task**, design-first, with prior-art grounding; it should not be
rushed as a grammar slice. Until then, the honest status is: *Eigenius can hold and verify real
numbers, but does not yet extract any number from prose* — every quantitative claim in the WRN page
is, on the prose→tree path, currently dropped or represented only as an opaque degree.

Related: `[[db_backed_encoding_finding]]`, `[[reference_ontology_modeling]]`; D52 spec; D62 §2 #4
(numerals) in `d62-grammar-gap-analysis.md`.
