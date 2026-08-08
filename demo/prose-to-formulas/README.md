# prose-to-formulas

A paragraph of the WRN paper, parsed into typed propositions, committed to a chain — and then edited
until the kernel refuses it.

```bash
./demo/prose-to-formulas/run.sh
```

To run including parsing:
```bash
EIGENIUS_DB_SNAPSHOT="$PWD/../db-snapshot/wordnet-umls-aligned-2026-08-03-specpoly" ./demo/prose-to-formulas/run.sh --reparse
```

**Prerequisite: a lexicon snapshot.** The encoded claims' propositions are built from lexicon axioms
(`wn:v02627934_t` is the verb sense of *require*), so the chain they commit to must be the one that
*defines* those axioms — a bare core+domain chain fails at the D47 decode with `ConstRef references
unresolved IRI`. `run.sh` stages it into the kernel's docker volume and treats the snapshot itself as
read-only.

```
<repo-parent>/db-snapshot/wordnet-umls-aligned-2026-08-03-specpoly   (1006 MB)
```

Override with `EIGENIUS_DB_SNAPSHOT`. To build one: `scripts/reseed-lexicon-db.sh`, then
`scripts/build-alignment-snapshot.sh`.

## What it shows

Two sentences of controlled prose:

```text
MSI cancer models had the exonuclease activity of WRN.        ← a measurement
MSI cancer models required the helicase activity of WRN.      ← an activity claim
```

and one rule **pinned from the literature**, not from this document:

```text
HasActivity(WRN, exonuclease) → RequiresActivity(WRN, helicase)
```

The parser turns each sentence into a closed, felicity-gated `Prop`, committed as an
`enc:EncodedClaim` under a `reflection:ProgramTrace` that mints `IsDerivedAs claim_i P_i`. A shape
rule lifts each into domain vocabulary. Then the pinned rule is applied to the measurement's claim,
and the result is:

> **`RequiresActivity(WRN, helicase)` is justified twice.**
> Once because sentence 2 asserts it — the document says so.
> Once because it **follows** from sentence 1 plus a published rule.
>
> Two `ReasoningSentence`s carrying a byte-identical proposition, with entirely different
> justification terms. (Nothing on chain records that they coincide — see *What is still thin*.)

**The derived route is not the better-warranted one**, and it is worth being exact about that:

| route | rests on |
|---|---|
| asserted (`s2:sentence`) | one Declared lift + sentence 2's parse |
| derived (`inferred:sentence`) | one Declared lift + sentence 1's parse, **plus** a Declared literature rule |

Both commit at grade `Declared`. The derived route carries strictly more assumptions — everything
the assertion needs, over a *different* sentence, and a published rule on top. "Independent of the
document stating the conclusion" is a claim about what it does not depend on, not about strength.

What makes it the interesting one is that it **knows what it depends on**, which the next section
makes visible.

That is what "a sentence becomes justified" means here: not that it was written down, but that a
chain of witnesses the kernel checks ends at its proposition.

Then the demo negates the measurement:

> MSI cancer models ~~had~~ **did not have** the exonuclease activity of WRN.

The two routes then come apart, in the same run:

```text
sentence 2's lift  (ASSERTED)     ✓ still commits — nothing about sentence 2 changed
sentence 1's lift  (MEASUREMENT)  ✗ REJECTED — and with it the derivation
```

Sentence 1 parses to a different proposition, so no `IsDerivedAs` witness matches the one its lift
names. The inferred claim cited that lift as its antecedent, so it has nothing left to stand on —
while the document's own assertion of the same conclusion is untouched.

That asymmetry is the point. A conclusion the graph produced carries a live dependency on what it
was produced from; a document repeating it does not. Nothing compared the two texts.

(The citations layer is deliberately split per sentence in `run.sh` — loaded as one layer the edited
run fails atomically, which hides exactly this.)

## The three ways a claim gets justified here

| | what warrants it | grade | authoring cost |
|---|---|---|---|
| **shape rule** | a Declared rule quantified over the parse's argument classes, instantiated per sentence with `spec_poly` | Declared | one rule per parse SHAPE |
| **pinned literature rule** | a published `A → B` on the chain, applied to a claim an earlier sentence established | Declared | one rule, reused |
| **prose modus ponens** | `A` and `A → B` **both parsed from sentences** — the grammar renders `if` as native implication | **Derived** | none |

The third is the strongest: nothing is Declared, because the implication *is* a sentence. The
grammar's `if` entry is `λs₂. λs₁. (s₂ → s₁)`, and its design note says encoding it opaquely "would
forfeit modus ponens in the checker". `"S₁ if S₂"` parses to a genuine top-level implication whose
antecedent is verbatim the premise sentence's own parse — verified against the real snapshot.

All three are exercised by [`shape_rule.rs`](../../crates/eigenius-reasoning/tests/shape_rule.rs),
including `one_rule_serves_two_different_sentences` (one rule, two sentences, both `Holds`) and two
fail-closed cases.

### Why the literature rule can be hand-authored when a bridge cannot

Its antecedent is `HasActivity(WRN, exonuclease)` — plain class references. A rule whose antecedent
had to be a *parse* would be impractical to hand-author: the Σ-binders and projections a DCG term
contains do have an ESL surface (`exists`, `eigentt:fst` — `claims-intact.esl` in this directory is
nothing but), but the term for one sentence runs to hundreds of characters of nested applications
that no one writes by hand correctly. That is exactly why the parse-shaped step is a **generated**
shape rule and the domain-vocabulary step is not.

## Where the rejection actually comes from

A false proposition, on its own, commits fine. `qc_consistency_check` returns Undecidable for any
non-trivial input ([`reasoning.esl`](../../ontologies/reasoning/reasoning.esl)), so nothing checks a
standalone claim against anything. What rejects is the **certificate**.

### The three properties on a ReasoningSentence

| property | what it holds | encoding |
|---|---|---|
| `reasoning:proposition` | the domain claim `C` | D47 `eigentt:TypeExpr` |
| `reasoning:justification` | the *reason shape*: `App(DeclaredEvidence(bridge), DerivedEvidence(claim))` | D32 §3.7 tagged dict — a `JustificationTerm` **value** |
| `reasoning:certificate` | the proof that the reason warrants the claim | D47 `eigentt:TypeExpr` |

The justification says *which evidence*; the certificate says *why that evidence suffices*. A
sentence can name perfectly real evidence and still fail, because the certificate is what has to
type-check.

### What the gate does

[`do_validate_justification`](../../crates/eigenius-reasoning/src/validate.rs) runs at commit, via the
`AutoOnLoad` hook on every `reasoning:ReasoningSentence`:

1. **Decode** `proposition` and `certificate` through the D47 codec against the current chain — where
   `ConstRef`s (`umlscui:C…`, `wn:…`) re-resolve to real classes and axioms.
2. **Lift** `justification` into a `Val::InductiveVal` typed at `reasoning:JustificationTerm`.
3. **Resolve** the `reasoning:JustifiedBy` inductive declaration from the layer.
4. **Type-check the proposition at `Prop`** (`Sort(0)`) and evaluate it.
5. **Construct the expected type** — the crux:

   ```text
   JustifiedBy(justification_val, proposition_val)
   ```

   `JustifiedBy : JustificationTerm -> Prop -> Type 0` is an **indexed** inductive, so the
   justification and the proposition sit in its *index slots*: the expected type is parameterised by
   exactly the reason given and exactly the claim made.
6. **Type-check the certificate against that type** with the kernel's NbE checker.
7. `Holds`, or `Fails` carrying the type-error string.

### Where the prose enters

The certificate is one application of the Artemov rule:

```text
app( P, C,
     DeclaredEvidence(bridge),  DerivedEvidence(claim),
     declared(bridge, P → C, _),        ← the human's lift, Declared
     derived (claim,  P,     _) )       ← the parser's output, Derived
```

`app` is a constructor of `JustifiedBy`: from `j1 : JustifiedBy(_, A → B)` and
`j2 : JustifiedBy(_, A)` it yields `JustifiedBy(App(j1,j2), B)`. **`A` must be the same term on both
sides** — the bridge's antecedent and the claim's proposition must be *identical*, not merely
compatible. (That invariant has its own test:
`bridge_antecedent_and_derived_grounding_embed_the_same_subtree`.)

The trailing `_` on `declared` and `derived` is a `UnitVal`. The kernel **discards it and synthesises
the real witness itself**, from the chain witness index:

```text
WitnessKey { category: Derived, iri: claim_iri, prop_hash: sha256(encode(P)) }
```

`lookup_chain_witness` walks the layer and every ancestor
([`witness_index.rs`](../../kernel/src/layer/witness_index.rs)). The key is minted on the other side
by the parser's `reflection:ProgramTrace`: a trace whose `reflection:resource` points at the claim
emits `IsDerivedAs claim_iri P`, where `P` is the claim's own `canonical_proposition`.

**The author of a certificate cannot supply the witness. Only the chain can.**

### Why the edit fails

```text
intact   claim_2.canonical_proposition = P    →  index key (Derived, claim_2, sha256(P))
edited   claim_2.canonical_proposition = P′   →  index key (Derived, claim_2, sha256(P′))

recorded certificate still contains        derived(claim_2, P, _)
kernel therefore looks up                  (Derived, claim_2, sha256(P))
```

The proposition is **hashed into the key**, so this is exact structural identity — no similarity
metric, no threshold. `sha256(P) ≠ sha256(P′)`, the lookup misses, no inhabitant exists for
`JustifiedBy(DerivedEvidence(claim_2), P)`, `app` cannot be applied, and the certificate fails against
`JustifiedBy(justification, C)`.

Two things follow. The paragraph has to be an **argument**, not a list of facts — the edit only bites
because a certificate names the premise. And the rejection is indifferent to *how* the prose changed:
a synonym swap producing a genuinely different proposition fails identically, while a whitespace
change does not fail at all.

## Layout

| file | generated? | what it is |
|---|---|---|
| `paragraph.txt` | — | the two sentences, verbatim from CNL-v3 |
| `paragraph-edited.txt` | — | the same, minus one negation |
| `onco-typed.esl` | — | domain predicates at `Set -> Set -> Prop` |
| `claims.tsv` | — | which domain proposition each sentence warrants, with **class IRI** arguments |
| `pins.tsv` | — | the human-verified reading per sentence (see *Reading selection*) |
| `ranks.json`, `ranks-edited.json` | recorded once each | the sense reranker's decisions, replayed — no LLM, no network, no key. One per variant: the replay key includes each word's candidate senses, so the edited paragraph is a different question. |
| `literature-rules.esl` | — | the pinned `A → B`, cited, in domain vocabulary |
| `claims-intact.esl` | `prose-to-esl` | units + encoded claims + ProgramTraces + decision points |
| `claims-edited.esl` | `prose-to-esl` | the same, from the edited prose |
| `rules.esl` | `--rules-out`, **once** | shape rules — one per distinct (predicate, parse shape) |
| `bridges.esl` | `--citations-out`, **once** | one `ReasoningSentence` per sentence, citing a shape rule |
| `inference.esl` | `--inference-out`, **once** | the CONCLUDED claim — the literature rule applied to the measurement |

The generated chain artifacts are **ESL, not Eigon-JSON**. `prose-to-esl` runs the same pipeline as
`prose-to-eigon` and prints the record as source, so what is committed is the formula a reviewer
reads rather than a D47 encoding of it:

```
resource formulas:claim_1 : encoding:EncodedClaim {
    reflection:canonical_proposition = type_expr(wn:v02203362_t(eigentt:fst(ontology:the(
        (exists x0 : wn:n13440063 => logic:And(ontology:compound_kind(x0, wn:n14606137),
        ontology:prep_of(x0, ontology:kind_of(umlscui:C0388246)))))), …));
}
```

`eigenius decompile <file>.json` prints any Eigon-JSON document this way; `--verify` recompiles it
and checks every term is alpha-equal under the normalisation the witness index hashes.
`kernel/tests/esl_round_trip.rs` runs that check over this directory on every build.

`run.sh --reparse` re-derives the two claims layers from the snapshot instead of using the committed
fixtures.

The domain namespace is `urn:eigenius:demo:onco-typed` rather than the WRN case study's `onco:`,
whose predicates are string-typed and whose committed conclusions depend on that; migrating them is
separate work.

## Why the argument layer is generated once and committed

`bridges.esl` is the **recorded argument**: what someone concluded, at a point in time, from the
prose as it then read. The claims layers are a function of the prose. Regenerating the argument on
every run would re-derive it around any edit, and nothing would ever fail to commit — which is the
one thing the demo exists to show.

The two halves run on two branches off one base, rather than as two loads onto one chain, because a
redefinition does not retract the earlier layer's witness: `IsDerivedAs claim_2 P_original` would
still be reachable and the certificate would still resolve.

## What a `Holds` does NOT mean

Two witnesses stand behind the intact commit, and neither is about biology.

| witness | grade | what it actually attests |
|---|---|---|
| `IsDerivedAs(claim_i, P)` | **Derived** | the DCG parser, run over bytes with a recorded sha256 at a recorded span, produced proposition `P` — a fact about **the text** |
| `IsDeclaredAs(bridge_i, P → C)` | **Declared** | a human asserts that a sentence meaning `P` warrants the domain claim `C`, with author and rationale |

`app` composes them and the result commits at **Declared** — a conclusion is no better than its
weakest link, and the lift is the weak one.

So the chain says *"the paper says it, and we declare that its saying so warrants the claim."*
**Nothing here witnesses that WRN's exonuclease activity is in fact dispensable.** A `Holds` proves
the certificate type-checks against admitted witnesses — D61's oracle #1, structural validity. It is
not oracle #2 (faithfulness), and it is certainly not truth.

That is why the *failure* case is the interesting half: the gate catches an argument no longer
following from its recorded premise, which is real and useful, and is not "the kernel knows the
biology."

A stronger witness is possible in principle. `SelectivelyEssential("WRN","MSI")` is also warranted by
`wrn:concl_wrn_selective_recomputed`, whose certificate cites an `IsDerivedAs` the **statistics
institution** minted by recomputing a Wilcoxon over 37 vs 91 cell lines from pinned data — a witness
about the world, not the text. The helicase/exonuclease claims have no such leg; in the WRN chain they
are narrative, so "the authors report it" genuinely is the strongest available warrant.

## Two limits, stated

**Reading selection is declared, not solved.** The reference page runs 60 of 62 units ambiguous. This
demo does not decide which reading is right — it takes the one whose sense-erased skeleton equals a
human-verified pin in `pins.tsv`, and **fails closed** if the pin matches zero readings (stale pin, or
the grammar moved) or several (they differ only in sense, and choosing between them is not the
pipeline's call). Every selection is recorded on chain as an `enc:DecisionPoint` with the pin as its
rationale. Both sentences here reach a single reading.

A third sentence — *"We found that WRN was selectively essential in MSI models"* — was dropped for
exactly this reason: on this snapshot it yields two readings sharing the pinned skeleton, and
`select_pinned` refused to choose. That refusal is the design working.

**Bridges are Declared and do not amortise.** Typing the arguments makes the lift checkable in one
respect — the classes must be present — but it does not make it *derivable*. The bridge is still a
ground implication, one per claim, graded Declared.

That is a property of the logic, not an authoring shortcut: **there is no implication introduction.**
No rule produces `JustifiedBy(_, A → B)` — `app` yields `B`, `sum_l`/`sum_r` yield `P`, `spec_str`
yields `P(t)`. An implication enters only through a grounding, so a bridge between vocabularies must
be asserted; it cannot be proved.

## On amortisation, honestly

`--rules-out` prints the ratio it achieved. This paragraph's two sentences use two different
predicates, so it reports **2 rules for 2 sentences** — no saving visible here. The saving is real
but needs repetition to show: `one_rule_serves_two_different_sentences` demonstrates one rule
covering two sentences, and the mechanism groups automatically by `(predicate, abstracted
proposition)`. A page with recurring constructions would show it; a two-sentence paragraph with two
constructions cannot, and inventing prose to make the number look better would be dishonest.

## Appendix: the quantified form

Shape rules are quantified over the classes rather than naming them:

```esl
reflection:canonical_proposition = type_expr(
    forall (g : Set, a : Set) => onco2:ParsedShape(g, a) -> onco2:RequiresActivity(g, a)
);
```

**This already commits** — a `Set`-quantified implication is a `Prop` and mints its `IsDeclaredAs`
witness like any other declaration (verified 2026-08-03). What is missing is the *elimination*:
`spec_str` is monomorphic on `core:string`, so it cannot instantiate a `Set`-quantified rule. A
polymorphic form is expressible —

```esl
spec_poly :
    forall (T : Set, P : T -> Prop, j : JustificationTerm, x : T, tag : core:string) =>
    JustifiedBy(j, forall (y : T) => P(y)) -> JustifiedBy(SpecStr(j, tag), P(x)),
```

— and loads as a well-formed constructor (also verified). It binds the domain type and the instance
on the *proof* side while the justification term carries only a string tag, so `JustificationTerm`
needs no change; only `JustifiedBy` gains a constructor.

That is a `reasoning.esl` edit, which is bootstrapped, so it costs a lexicon reseed — worth batching
with any other ontology change. With it, one rule per parse *shape* replaces one bridge per sentence,
and the shape inventory is small: 46 distinct `cat` shapes across 242,938 lexical entries.

Two further gaps, for completeness:

- **Anaphora with propositional antecedents.** Resolution is implemented (D64 — referent holes, LLM
  proposer, kernel re-gate), but `entity_candidates` collects only named-entity IRIs, so *"these
  findings"* resolves to a definite description over a *findings* kind rather than to the propositions
  it denotes. The paper's own conclusion sentence needs exactly that.
- **The inference structure.** *"These findings show that X"* yields a proposition; nothing extracts
  the justification term from the connective. Here the argument's shape is Declared in `claims.tsv`
  and only the propositions come from the prose.
