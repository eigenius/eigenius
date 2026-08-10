# Shape-rule amortisation — measurement, and why frame keying does not fix it

*2026-08-09. Investigation for issue #111. Every number below is reproducible from files in
this repo; the commands are in §7.*

Grades follow the working protocol. **Derived** = a script or the compiler produced it.
**Observed** = read in the source at the cited `file:line`. **Declared** = judgment, marked
as such.

---

## 1. The evidence cited in #111 does not support its claim

#111 opens with: `demo/prose-to-formulas/rules.esl` contains `rule_1` and `rule_2` for a
two-sentence paragraph — "a 1:1 ratio, no amortisation, on the corpus the mechanism was
designed to amortise over." It attributes this to syntactic variety.

The two sentences have **identical syntax**. *(Derived)* Their active pins in
`demo/prose-to-formulas/pins.tsv` are byte-identical sense-erased skeletons:

```
§(the(ΣG#0:§. And(compound_kind(G#0, §), prep_of(G#0, kind_of(§)))).1,
  kind_of(ΣG#0:§. compound_kind(G#0, ΣG#1:§. compound_kind(G#1, §))))
```

The two generated rules differ in **exactly two tokens** *(Derived — unified diff over the
whitespace-normalised `type_expr`)*:

| | `rule_1` | `rule_2` |
|---|---|---|
| verb axiom | `wn:v02203362_t` | `wn:v02627934_t` |
| consequent | `onco_typed:HasActivity` | `onco_typed:RequiresActivity` |

The rule key is `(predicate, abstracted-proposition)` — `crates/eigenius-encoding/src/emit.rs:350`
*(Observed)*. The predicates differ, so the key differs no matter what the shape component does.
**Two rules is the floor for this corpus under every possible keying scheme**, and a rule
concluding `HasActivity` could not be shared with one concluding `RequiresActivity` in any case.

The 2/2 ratio measures the demo's choice of two different domain predicates. It is not a
measurement of amortisation.

## 2. The defect is real. Here is a measurement that shows it

`experiments/parsing/expected-readings.tsv` — 62 sentences of the WRN paper, each with a
human-verified reading. *(Derived)*

- **62 sentences → 61 distinct sense-erased skeletons.** One pair shares a skeleton
  («…WRN is a synthetic-lethal vulnerability for MSI cancers.» /
  «…WRN is a promising drug target for MSI cancers.»).

Two propositions with the same shape are identical except at the argument-class positions, and
those positions hold WordNet synsets or UMLS CUIs, which `erase_senses` collapses to the same
`§`. So shape-equality implies skeleton-equality, and the current scheme yields **at least 61
rules for these 62 sentences**. That is the measurement #111 needs.

## 3. The diversity is structural, not lexical — which rules out the proposed fix

`erase_senses` (`kernel/src/dcg/skeleton.rs:53`) replaces every token carrying a run of ≥4
digits with `§` *(Observed)*. WordNet synsets (`v02203362_t`, `n13440063`) and UMLS CUIs
(`C0920269`) are all erased; only structural relation names survive (`compound_kind`,
`prep_of`, `kind_of`, `And`, `the`, `gt`, `is_a`).

**So the 61 count already assumes perfect lexical abstraction.** Every verb sense and every
noun sense is a hole in it. VerbNet, FrameNet, PropBank and the Predicate Matrix all group
*lexical items*; the most any of them can achieve is what total erasure achieves, and total
erasure leaves 61 of 62.

**No lexical-abstraction resource can reduce the rule count below 61 on this corpus.** This
answers #111's coverage question without acquiring any lexicon: biomedical VerbNet coverage
does not need measuring, because even 100 % coverage buys nothing on the axis that is
actually driving the count. It also defers the licensing question (#111's second open
question) — there is nothing yet to license.

## 4. Where the diversity does live

Truncating each reading to depth *d* and holing out everything below *(Derived)*:

| truncation depth | distinct classes | sentences / rule |
|---|---|---|
| 1 — matrix frame only, all arguments holed | **10** | 6.20 |
| 2 | 39 | 1.59 |
| 3 | 48 | 1.29 |
| 4 | 57 | 1.09 |
| 5 … full | **61** | 1.02 |

Reuse is all-or-nothing. Only truncating the argument *entirely* (depth 1) gives real sharing;
one level of argument structure gives back 29 of the 51 collapsed distinctions.

Depth 1 is exactly #111's proposal — "the verb sense plus its argument structure", arguments as
opaque role fillers.

## 5. Depth 1 deletes the variables the rule binds

The path from `rule_1`'s root to its abstracted variable `v0` *(Derived, from
`eigenius compile` output)*:

```
Pi → Pi → Pi → App → App → Fst → App → Sig → App → App → App → Var v0
```

The three `Pi`s are the two `∀ (v : Set)` binders and the implication arrow. The two `App`s
are the verb applied to its two arguments. Everything after that is **inside** the first
argument: a projection off a definite description (`Fst`, `the`), then an existential binder
(`Sig`), then the `And` of `compound_kind` and `prep_of`. `v0` and `v1` sit six constructors
below the argument's root.

`v0` and `v1` are the classes the consequent names — `HasActivity(v0, v1)`. Truncating the
argument to get the depth-1 collapse deletes them.

`build_shape_rule` fails closed on exactly this: an argument class that does not occur in the
proposition is `GradeError::ArgumentNotInProposition`
(`crates/eigenius-reasoning/src/grade.rs:551-559`, test at
`crates/eigenius-reasoning/tests/shape_rule.rs:233`) *(Observed)*. A depth-1 normal form does
not merely lose precision — it does not build.

## 6. The calculus blocks the obvious repair

The natural repair is to keep the rule general and bridge the gap: derive the frame-level
antecedent `A` from the parse-level proposition `P`, then apply the general rule.

`JustifiedBy` has nine constructors (`ontologies/reasoning/reasoning.esl:97-175`): four
groundings, `app`, `sum_l`, `sum_r`, `spec_str`, `spec_poly`. **None produces
`JustifiedBy(_, A -> B)`** — stated at `reasoning.esl:26-31` *(Observed)*:

> No rule PRODUCES `JustifiedBy(_, A -> B)`: `app` yields `B`, `sum_*` yield `P`, `spec_*`
> yield `P(t)`. An implication therefore enters only through a grounding — asserted as a
> resource, witnessed by a trace. There is no deduction theorem […]

So getting from `P` to `A` requires a `JustifiedBy(_, P -> A)`, which must be **Declared** —
one per distinct `P`, i.e. **one per parse shape**. That is precisely the cost #111 exists to
remove. *(Derived: the constructor list is exhaustive and closed; no arm has an implication in
its conclusion.)*

**Generalising the antecedent as logic cannot reduce the Declared-artifact count. The cost is
conserved.**

## 7. The one route that does change the count

Make the frame representation a **second derived witness**, not a logical consequence.

The parser already emits `P` under a `ProgramTrace`, so `derived(claim_iri, P)` grounds. Have
the same program also emit a normal form `F` as its own resource with its own
`canonical_proposition` and `ProgramTrace`. Then `derived(normal_iri, F)` grounds directly, and
the shape rule is written against `F` from the start:

```
parse ──[parser]────────> P   IsDerivedAs   (unchanged)
parse ──[normaliser]────> F   IsDerivedAs   (new, same program run)
                          ∀v. F[v] → Pred(v)   Declared, keyed on the NORMAL FORM
```

No `P -> A` implication is needed, because `F` is never derived from `P` inside the calculus —
it is witnessed alongside it. Rule count then tracks distinct normal forms, and the authoring
cost is one normaliser rather than N Declared artifacts.

Consequences, stated so they are not discovered later:

- **#111's witness-key question is answered.** Normalisation **adds** a witness; it does not
  replace one. `P`'s hash is untouched, so every committed citation keeps resolving. The new
  requirement is that `F` be deterministic and stable across runs, the same requirement `P`
  already carries.
- **The normaliser becomes the trusted component**, and it is trusted for *faithfulness*
  (oracle #2), which the commit gate cannot check. Per the working protocol a mechanised
  faithfulness score is **Derived**, never auto-Verified.
- **The normal form must preserve the logical skeleton.** `demo/prose-to-formulas` turns on
  deleting one negation being invisible structurally and visible in the proposition; the
  `→ False` and the quantifier scoping must survive normalisation, or the demo's central
  property is lost. This is testable and should be a test before it is a design.
- **Generality is bought with strength.** A rule keyed on the matrix frame asserts that *any*
  sentence with that verb and those fillers warrants the predicate — dropping "the exonuclease
  *activity of* WRN" down to two role fillers. That is a materially stronger Declared claim
  than today's near-tautological one, and it is a place an unjustified lift can hide.
  *(Declared — this is a judgment about risk, not a measurement.)*

## 8. Recommendation

*(Declared.)*

1. **Correct #111's problem statement** — replace the 2/2 figure with the 61/62 figure, and
   restate the cause as argument-internal structural variation rather than syntactic variety.
2. **Close out the lexical-resource line of work** (VerbNet / FrameNet / PropBank / SemLink /
   Predicate Matrix). §3 shows it cannot move the number on this corpus. Nothing to license,
   nothing to measure.
3. **Reframe the issue** around the normalisation design of §7: what the normal form is, what
   it is allowed to discard, and how its faithfulness is scored. The generality/strength
   trade-off in §7 is the real open question, and it is a semantic one, not a lexicon-sourcing
   one.
4. **#111's ordering claim still holds.** It says this should land before #112 (`TypeExpr`
   interpretation), on the grounds that authoring against raw DCG skeletons is the thing to
   stop doing. §7 does not change that: the normal form is what #112 would give an authoring
   surface to.

## 9. Reproducing the numbers

```bash
# §2, §4 — skeleton count, the shared pair, and the truncation curve.
# Re-prints every skeleton and compares it against the corpus before counting;
# a reading that does not round-trip aborts the run.
python3 experiments/parsing/skeleton-abstraction.py

# §1 — identical pins
grep -F 'MSI cancer models had the exonuclease'   demo/prose-to-formulas/pins.tsv
grep -F 'MSI cancer models required the helicase' demo/prose-to-formulas/pins.tsv

# §1, §5 — the two-token rule diff, and the path to v0
cargo build -p eigenius-cli
./target/debug/eigenius compile demo/prose-to-formulas/rules.esl
```
