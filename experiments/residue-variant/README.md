# Residue versus variant — UAB Round 1, experiment 01

The toy from `experiments/uab/UAB Round 1/01-toy-residue-variant/residue-versus-variant.md` (the UAB
material is not in this repository; line references below are to that file). The material asks:
"Encode the above and show the kernel refusing the substitution" (:58), with a two-sided success
criterion — a refusal is the result; if there is none, "the reason why is more interesting than the
demo" (:60-61).

## Result

The survey sentence is parsed and lands as a claim. A declared bridge lifts the survey's report to
the variant-level predicate. Four conclusions are checked by the kernel's commit-time certificate
check (Rule 21):

| Case | Conclusion | Grounds offered | Verdict | Reason the kernel reports |
|---|---|---|---|---|
| A | `DDAVPResponsive(Leu44)` | bridge at `Leu44` + the parsed claim | **refused** | `resource … avpr2:Leu44 (is_a = [avpr2:Residue]) does not inhabit class urn:eigenius:uab:avpr2:Variant` — the conclusion's type is not a type |
| B | `DDAVPResponsive(c.131T>C)` | bridge at `c.131T>C` + the parsed claim | **refused** | `no admitted IsDeclaredAs witness for IRI …:claim_1 with the supplied proposition` |
| C | `SurveyReportsResponsive(Leu44)` | the parsed claim | commits | — |
| D | `DDAVPResponsive(c.131T>C)` | a declared inverse of `HasResidue` + `HasResidue(c.131T>C, Leu44)` + the parsed claim | commits | — |

A is the "unstatable" refusal (:41): the predicate is indexed by `Variant`, and the sentence's subject
is a `Residue`. B is the substitution itself: the sentence grounds a proposition about `Leu44`, and
the witness key hashes the proposition, so it grounds nothing about `c.131T>C`. C shows the true
sentence lands. D is the "does not refuse" case: the substitution commits once a rule inverting
`HasResidue` is declared, and that rule carries `prov:was_attributed_to` and its own trace.

`crates/eigenius-encoding/tests/residue_variant.rs` asserts all four, including A's and B's reasons.

## Mapping the material onto Eigenius

| Material | Here |
|---|---|
| `DDAVPResponsive : Variant -> Prop` (:29) | `data avpr2:DDAVPResponsive : avpr2:Variant -> Prop` (`avpr2.esl`) |
| `residueOf : Variant -> Residue`, not injective (:35-36) | `data avpr2:HasResidue : avpr2:Variant -> avpr2:Residue -> Prop` — a relation, so there is no function to invert. Its non-injectivity is stated in the vocabulary's comments; only `HasResidue(c.131T>C, Leu44)` is declared, in `residue-bridge.esl` |
| "Keep the subject at nucleotide resolution … distinct terms that never unify" (:41) | `avpr2:c_130C_T` and `avpr2:c_131T_C` are distinct individuals of `avpr2:Variant` |
| The true sentence (:18) | Parsed by `prose-to-esl` into `claims.esl` `claim_1`, committed under a `prov:DeclarationTrace` |
| The lossy identifier `Leu44` (:47) | A proper-noun lexical entry naming the individual `avpr2:Leu44 : avpr2:Residue` (`avpr2.esl`). The parse carries `Leu44` as the residue it denotes |
| "show the kernel refusing the substitution" (:58) | Cases A and B |
| "If it does not refuse, the reason why" (:61) | Case D |
| Retrieval prior: `v1` in ClinVar, `v2` absent (:48) | Not modeled |
| Open question: does nucleotide resolution forbid real reports? (:67) | The residue-level sentence lands (C); only the variant-level predicate requires a `Variant` |
| Open question: is "absent from ClinVar as of T" Observed? (:68) | Not modeled |
| Open question: where does a family's goal live? (:69) | Not modeled |

### Departures from the source sentence

The source is «A nationwide survey lists Leu44 among the desmopressin-responsive *AVPR2* genotypes,
and responders achieved 12.6 to 31.6 percent reductions in urine output.» `sentences.txt` holds:

> A nationwide survey reports Leu44 as a desmopressin-responsive AVPR2 genotype. Responders achieved
> 12.6–31.6% reductions in urine output.

| Change | Why |
|---|---|
| `lists … among the … genotypes` → `reports … as a … genotype` | With `list`, all 48 readings attach `among …` to the survey, not to `Leu44`: both WordNet senses of `list` carry only transitive frames (8, 11; 9) and SPECIALIST records `tran=np`, so `among …` can only be an adjunct. `X as Y` needs a verb in the curated essive set (`crates/eigenius-wordnet/src/convert.rs`, `ESSIVE_VERBS`); `report` is in it, `list` is not (`lists Leu44 as …` has no parse) |
| `12.6 to 31.6 percent` → `12.6–31.6%` | The parser reads `%` as a unit and joins a range with an en-dash or hyphen; `percent` as a word and `X to Y` ranges have no parse (D95) |
| One sentence → two | The one-sentence form with the en-dash range had no parse with `WRN` standing in for `Leu44`; not localized further |
| `*AVPR2*` → `AVPR2` | Markdown emphasis is not prose |

Sentence 2 lands as `claim_2`; no conclusion uses it.

## The bridge

### How it was made

`bridge.esl` is hand-written: a `justification:Declaration` whose `eigentt:proposition` is

```
∀ (v : avpr2:Variant). rv:SurveyReportsResponsive(v) → avpr2:DDAVPResponsive(v)
```

attributed to `agent:eigenius_core_team`, with `rv:warrant_survey` as its primary source. Its
`prov:DeclarationTrace` mints the `IsDeclaredAs` witness that each certificate's
`declared("…:bridge", …)` leaf consumes; without the trace every certificate citing the bridge is
refused.

The antecedent is not restated in the bridge. `SurveyReportsResponsive` is a definition
(`survey-typed.esl`) whose body is `claim_1`'s parsed term with `Leu44` abstracted to `x`, so the
step from the prose to the antecedent is computed by unfolding, not declared. The declared step is
the lift from «the survey reports x as a desmopressin-responsive AVPR2 genotype» to «x responds to
desmopressin». It has to be declared because `DDAVPResponsive` is an opaque predicate over variants,
not a definition over the parse — unlike v2 (`demo/prose-to-formulas-v2`), whose domain predicates
are defined over the parse and need no declared lift.

Two choices in the bridge decide the verdicts: the quantifier ranges over `Variant`, which refuses A;
and the antecedent is the survey's report about `v`, which refuses B, because the survey reports only
about `Leu44`.

### Toward realistic domain reasoning

The bridge takes the survey's report to a statement about a variant in one declared step. A chain
with one ground per step, composed with `app`, would separate at least these:

| Step | From → to | Ground | Machinery |
|---|---|---|---|
| 1 | the survey reports x as G → the survey's authors assert G(x) | Declared, with the survey as cited source | `reference:Citation`, `justification:Declaration` |
| 2 | «desmopressin-responsive» → a measured response | A definition over the survey's threshold on urine-output reduction (sentence 2: 12.6–31.6%) | D95 quantities; a `stats:` analysis plan if the survey's data were available |
| 3 | the cohort's responders → the response of a genotype | Population-level claim | `stats:PopulationLevel` epistemic scope (D52 §7.4) |
| 4 | a residue-level genotype (`Leu44`) → nucleotide-level variants | `∃ v. HasResidue(v, Leu44) ∧ DDAVPResponsive(v)` — some variant at Leu44, not every one | A definition; nothing declared |
| 5 | a variant → its functional class | Observed assay results per variant: c.130C>T reaches the membrane at wild-type levels; c.131T>C has under 4% of wild-type binding sites (:13-14) | `prov:ObservationTrace` |
| 6 | variant and functional class → the genetic condition and its treatment class: AVPR2 loss of function causes nephrogenic diabetes insipidus (:9), desmopressin-responsive or not | Declared clinical rule, attributed | `justification:Declaration` |

With the steps separate, step 4 yields only an existential, and `DDAVPResponsive(c.131T>C)` needs
variant-specific evidence (step 5) rather than the survey. A refusal then names the step that has no
ground; with a single bridge, a refusal or an admission concerns the whole lift. None of these steps
is built.

## Files

| File | Written by | Role |
|---|---|---|
| `sentences.txt` | hand | The prose input |
| `avpr2.esl` | hand | Vocabulary: `Variant`, `Residue`, `HasResidue`, `DDAVPResponsive`, the two variants, `Leu44`, and `Leu44`'s lexical entry. Chain-loaded at parse time (the snapshot does not carry it) and loaded first in the test |
| `selections.json` | `prose-to-esl`, live LLM, once | The recorded reading choice per sentence (80 and 20 candidates). Replayed on every later run; a run whose candidate pool differs fails closed instead of choosing |
| `claims.esl` | `prose-to-esl` | The parsed claims: units, `EncodedClaim`s, `DeclarationTrace`s, `DecisionPoint`s with the ranker's rationale |
| `survey-typed.esl` | hand, from `claims.esl` | `def rv:SurveyReportsResponsive(x : lexicon:Entity)` — `claim_1`'s term with `Leu44` abstracted. `SurveyReportsResponsive(Leu44)` unfolds to `claim_1`'s proposition. Rebuild it whenever `claims.esl` changes |
| `bridge.esl` | hand | The declared bridge `∀ (v : Variant). SurveyReportsResponsive(v) → DDAVPResponsive(v)`, its trace, and the survey's warrant (the material does not cite the survey) |
| `conclusions.esl` | hand | Cases A, B, C |
| `residue-bridge.esl` | hand | Case D: the declared inverse of `HasResidue`, the declared `HasResidue(c.131T>C, Leu44)` (HGVS numbering: c.131 is in codon 44), and the conclusion. Loaded on its own branch |
| `crates/eigenius-encoding/tests/residue_variant.rs` | hand | Loads the files onto a working copy of the snapshot and asserts the four verdicts |

Each `instantiate` instance in `conclusions.esl` and `residue-bridge.esl` is ascribed its class,
`(avpr2:c_131T_C : avpr2:Variant)`. `instantiate` fixes its domain `T` from the instance's inferred
type, and an individual's inferred type is its principal one (a record of its properties refined by
its classes), which does not unify with the rule's `forall (v : avpr2:Variant)`. The kernel checks
the ascription by `is_a`.

## Running it

The lexicon snapshot is `../db-snapshot/uab-compound-aligned-2026-10-01` (base
`uab-compound-2026-10-01`). It carries the `responsive → to` frame below; older snapshots do not, and
on them `desmopressin-responsive` has no reading.

```bash
# The toy (in-process, no docker). The path must be ABSOLUTE: the test runs from the crate
# directory, and a relative path that does not resolve SKIPs — and reports `ok`.
EIGENIUS_DB_SNAPSHOT=/abs/path/db-snapshot/uab-compound-aligned-2026-10-01 \
  cargo test --release -p eigenius-encoding --test residue_variant -- --ignored --nocapture

# Regenerate claims.esl, replaying selections.json (no LLM; byte-identical to the committed file as of
# 2026-10-01). Run from the repo root: the source path lands in the artifact verbatim.
cargo build --release -p eigenius-encoding --features use-llm
target/release/prose-to-esl --snapshot /abs/path/db-snapshot/uab-compound-aligned-2026-10-01 \
  --source experiments/residue-variant/sentences.txt \
  --chain-load experiments/residue-variant/avpr2.esl \
  --selections experiments/residue-variant/selections.json \
  --ns urn:eigenius:uab:residue-variant \
  --out experiments/residue-variant/claims.esl
```

## What the runs produced

Parse probes ran cap-only (no sense reranker) through `scripts/measure-parse-rate.sh --page` or
`prose-to-esl --partial`; reading choices, where recorded, came from the live reading ranker.

| # | Snapshot | Input | Result |
|---|---|---|---|
| 1 | `wordnet-umls-aligned-2026-09-30-governed-preps` | the source sentence | No parse; one out-of-vocabulary token, `Leu44` |
| 2 | same | `WRN` for `Leu44`, clauses split | Clause 1 parses (several readings). Clause 2 has no parse with every word known. `12.6–31.6%` parses; `12.6 percent` and `12.6 to 31.6%` do not |
| 3 | same | both clauses, `avpr2.esl` chain-loaded (`Leu44 : Residue`) | 48 and 20 readings. `Leu44 : Residue` raised no type error. All 48 attach `among …` to the survey. `desmopressin-responsive` became plain `responsive` (`wn:a01999306`) |
| 4 | same | `lists`/`reports`/`identifies` `Leu44 as a …` | `lists`: no parse. `reports`, `identifies`: 48 readings each, every one `report_as(Leu44, g, s)` with `g` a responsive AVPR2 genotype — `desmopressin` still dropped |
| 5 | `uab-compound-aligned-2026-10-01` (after the compound fix and reseed) | `lists … among` and `reports … as` | 32 readings each; 32 of 32 keep desmopressin as `deg_responsive_rel(kind_of(C0011701), g)` |
| 6 | same | `sentences.txt` | 32 and 20 readings; the ranker chose survey `umlscui:C0038951`, `report` `v00966809` ("announce as the result of an investigation"), `responsive` `a01999306` |
| 7 | same | `WRN is pan-essential.` / `WRN is helicase-dead.` (full WRN paper words, not on the gated page) | `pan-essential`: «essential for the genus *Pan*» (`n02481629`, the chimpanzees), 64 readings. `helicase-dead`: «unresponsive to DNA helicases» (`a02107386`, `dead(p)`, «followed by `to'»), 4 readings. The rule of runs 5-6 read every noun left half as the head's governed complement |
| 8 | same, after the compound rework (below) | runs 7 and 6 again | `pan-essential`: 10 readings, all a plain sense of `essential`. `helicase-dead`: 38 readings, the vague `dead(x) ∧ compound_kind(x, DNA Helicases)` among them beside the «dead to» ones. `sentences.txt`: 80 and 20 readings; the ranker chose survey `wn:n00644503` and the vague reading `responsive(g) ∧ compound_kind(g, desmopressin)` over «responsive to desmopressin» |

Toy test runs, in order:

| Run | Outcome | What it exposed |
|---|---|---|
| 1 | A, B, C all fail to decode | `subst` (definition instantiation) refused `Exp::Const`, which an inductive's name decodes to since D76 — any definition body naming an inductive (`logic:And`) could not be instantiated |
| 2 | A refused (its reason), C commits; B, D fail in `instantiate` | `T` inferred from the instance's principal type — fixed by the ascription above |
| 3 | B, D fail solving `instantiate`'s `P` | The unifier's scope check treated a named individual (`umlscui:C1332124`, the AVPR2 gene) as able to hide a variable |
| 4 | A, B refused for their reasons; C, D commit | — |
| 5 | After the compound rework: replaying `selections.json` failed closed (sentence 1's pool grew from 32 to 80 readings). Re-recorded; `survey-typed.esl` rebuilt from the new `claim_1`. A, B refused for their reasons; C, D commit | — |

## Changes made outside this directory

| Change | Files |
|---|---|
| Hyphen compound `L-H` with an adjective head: a bound prefix is never looked up as a word — `pan-` (and `hyper-` … `mono-`) reads as `H`, `non-`, `anti-`, `pre-` and the other listed prefixes have no reading; an adjective `L` modifies `H` (`double-stranded`, `L` dropped as before), also when `L` is a noun too; a noun or name `L` that is not an adjective keeps its content as `H(x) ∧ compound_kind(x, L)`, and where `H` governs a preposition also as the phrase `H P L` (`responsive to desmopressin`), selection choosing between them | `kernel/src/dcg/parse/seed.rs`, `kernel/tests/closed_class_determiners.rs` |
| `has_token` counts a hyphenated surface whose spaced form is an entry (`microsatellite-stable`) as known — it had been known only through the dropped-left-half rule | `kernel/src/dcg/parse/mod.rs` |
| `responsive` governs `to` (SPECIALIST E0053052 `compl=pphr(to,np)`) | `crates/eigenius-wordnet/adjective-frames.tsv`, `crates/eigenius-wordnet/src/convert.rs` (test) |
| `subst` passes `Exp::Const` as a leaf | `kernel/src/nbe/subst.rs` |
| The unifier's scope walk treats a `Val::ResourceVal` WITH an `@id` — a stored chain individual — as variable-free: its values are strings, numbers, booleans, embedded resources and JSON, none of which can hold a `Neut::Gen`. An embedded resource (no `@id`) stays undecidable: `Exp::Construct` builds those, and its marshalling replaces a field that depends on a bound variable with an empty resource (`eval/marshal.rs`) | `kernel/src/nbe/unify.rs` |
| Snapshot builds run isolated per checkout: the kernel image tag is `EIGENIUS_KERNEL_TAG` (default `local`); the scripts look up the kernel container and the project's volume instead of hardcoding `eigenius-kernel-1` and `eigenius_eigenius_db` | `docker-compose.yml`, `scripts/reseed-lexicon-db.sh`, `scripts/add-layer-to-snapshot.sh` |

The reseed ran as `COMPOSE_PROJECT_NAME=eigenius-uab EIGENIUS_KERNEL_TAG=uab`, so it used its own
volume and image. Its snapshots are named outside the `wordnet-umls-*` pattern that
`scripts/measure-parse-rate.sh` and the `justfile` pick up by modification time.

Reference-page gate on `uab-compound-aligned-2026-10-01` (replaying
`experiments/parsing/ranks/2026-09-29-d95-slice8.json` and the matching selections): grammar-gap 0,
expected-hits 62/62, total-readings 652, total-skeletons 212, selection 29/41 correct. HEAD's parser
on the baseline snapshot gives the same 29/41; `baseline.json` records 30/41. A first version of the
compound rework gave a left half its nominal readings even when it is also an adjective; the gate
measured total-readings 652 → 796 (ceiling 700) and 29 of 41 recorded selections abstaining. The
page's adjective-left compounds, `double-stranded` and `large-scale`, have left halves that WordNet
also lists as nouns. With the adjective reading taking precedence, the gate figures are those above.

## Open

- `instantiate` at an individual needs the ascription; solving `T` from the rule instead is the
  higher-order unification question D88 §6.1 leaves open.
- `DDAVPResponsive(c.130C>T)` from its published evidence is not encoded, so there is no case where
  the variant-level predicate is grounded at nucleotide resolution.
- `selections.json` was recorded against the cap-only parse (no sense ranks); the reading pool and
  therefore the choice depend on the snapshot.
- The ranker chose the vague reading of «desmopressin-responsive» over «responsive to desmopressin»;
  its rationale does not compare the two.
- `non-`, `anti-` and the other opaque prefixes have no reading: their semantics (negation,
  reversal) is not built.
