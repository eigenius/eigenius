> # ⚠️ RETIRED `2026-08-17` — superseded by [`demo/prose-to-formulas-v2`](../prose-to-formulas-v2/)
>
> **The files are gone.** Only this README remains, as the record of what the demo was and why it was
> retired. The runnable demo, and the artifacts the acceptance test reads, are v2's.
>
> **Why it was retired.** Selection here was by SKELETON PIN (`pins.tsv`), and a pin is sense-erased,
> so it cannot break a tie between readings that differ only in sense — `PinReadingRanker` abstains
> (`kernel/src/dcg/reading_ranker.rs`: *"no match, or ≥2 readings share the pinned skeleton — abstain"*)
> and the run fails closed. That made the demo INVENTORY-DEPENDENT: any lexicon change adding a sense
> to a word in its paragraph could defeat it, and twice did.
>
> * On the d67 inventory the EDITED variant's pin matched 3 readings; it was moved to a recorded
>   selections draw.
> * On `2026-08-15` (D70) `C0043119` «Werner Syndrome» — a T047 disease — gained a bare-standing
>   `name` entry, so «the exonuclease activity of **WRN**» acquired a second kind reading (the
>   syndrome beside `C0388246` «WRN protein, human»). The INTACT variant's pin then matched 2
>   readings and abstained. Note the sense ranker had ALREADY eliminated the syndrome and put the
>   protein at rank 0 — the pin simply cannot act on that, being sense-erased.
>
> Fixing it would have meant giving the intact variant a selections draw too, leaving `pins.tsv`
> unused and the demo differing from v2 only in prose and in lacking the anaphora and claim-kind
> stages. v2 selects with the reading ranker, which survives inventory changes.
>
> **Where its role went.** `crates/eigenius-encoding/tests/acceptance.rs` — the in-process D67 §3.5
> acceptance — read this demo's artifacts and now reads v2's (paths AND the IRI namespace: v1 used
> `urn:eigenius:demo:formulas:`, v2 uses `urn:eigenius:demo:v2:`). It passes: intact `Holds`, edited
> `Fails` with the missing-witness diagnostic.
>
> **What was NOT lost.** The pin MECHANISM is alive and covered. `PinReadingRanker` / `load_pins` /
> `prose-to-esl --pins` remain a first-class selection arm, and the parse-rate sweep uses it as its
> DEFAULT: with no `EIGENIUS_SELECTIONS` the harness reports *"reading ranker: pin-backed
> (expected-readings corpus)"* and selects from `experiments/parsing/expected-readings.tsv` — a
> different file from this demo's, and the one behind the tracked 62/62. The abstain-on-tie behaviour
> that retired this demo is the same code path, exercised there on every deterministic run.
>
> What went with the deletion is only this demo's WORKED EXAMPLE of a pins file — there is now no
> committed `pins.tsv` anywhere. Anyone adding one should also add a test, since a shell script was
> the only thing exercising this one.

# prose-to-formulas

A paragraph of the WRN paper, parsed into typed propositions, committed to a chain — and then edited
until the kernel refuses it.

```bash
./demo/prose-to-formulas/run.sh
```

To run including parsing:
```bash
./demo/prose-to-formulas/run.sh --reparse
```

**Prerequisite: an ALIGNED lexicon snapshot.** The encoded claims' propositions are built from
lexicon axioms (`wn:v02627934_t` is the verb sense of *require*), so the chain they commit to must
be the one that *defines* those axioms — a bare core+domain chain fails at the D47 decode with
`ConstRef references unresolved IRI`. And it must be the aligned chain: on a raw (unaligned) reseed,
WordNet and UMLS carry duplicate senses of the same words, the recorded ranks replay (keyed on each
word's candidate senses) misses and falls back to cap-only, and `--reparse` fails closed with two
readings sharing the pinned skeleton. `run.sh` stages the snapshot into the kernel's docker volume
and treats the snapshot itself as read-only.

```
<repo-parent>/db-snapshot/wordnet-umls-aligned-d66   (993 MB)
```

Override with `EIGENIUS_DB_SNAPSHOT`. To build one:
`scripts/reseed-lexicon-db.sh --snapshot-dir wordnet-umls-d66`, then
`scripts/build-alignment-snapshot.sh --base <that> --out …/wordnet-umls-aligned-d66`.

## What it shows

Two sentences of controlled prose:

```text
MSI cancer models had the exonuclease activity of WRN.        ← a measurement
MSI cancer models required the helicase activity of WRN.      ← an activity claim
```

and one rule **pinned from the literature**, not from this document:

```text
∀m. HasActivity(m, WRN, exonuclease) → RequiresActivity(m, WRN, helicase)
```

The parser turns each sentence into a closed, felicity-gated `Prop`, committed as an
`enc:EncodedClaim` under a `reflection:DeclarationTrace` that mints `IsDeclaredAs claim_i P_i`. There is
no lift step: `onco-typed.esl` *defines* the domain predicates over the parser's own lexicon, so
each parsed proposition already **is** a domain formula. Then the pinned rule is specialized at the
model and applied to the measurement's claim, and the result is:

> **`RequiresActivity(MSI, WRN, helicase)` is justified twice.**
> Once because sentence 2 asserts it — the document says so.
> Once because it **follows** from sentence 1 plus a published rule.
>
> One proposition, two entirely different warrants: sentence 2's own parse witness, and a
> `ReasoningSentence` whose certificate applies the rule. (Nothing on chain records that they
> coincide.)

**The derived route is not the better-warranted one**, and it is worth being exact about that:

| route | rests on |
|---|---|
| asserted (`claim_2`) | sentence 2's parse — a `Derived` witness, nothing Declared |
| derived (`inferred:sentence`) | sentence 1's parse, **plus** a Declared literature rule |

The derived route carries strictly more assumptions — a parse of a *different* sentence, and a
published rule on top, so it commits at `Declared`, its weakest link. "Independent of the document
stating the conclusion" is a claim about what it does not depend on, not about strength.

What makes it the interesting one is that it **knows what it depends on**, which the next section
makes visible.

That is what "a sentence becomes justified" means here: not that it was written down, but that a
chain of witnesses the kernel checks ends at its proposition.

Then the demo negates the measurement:

> MSI cancer models ~~had~~ **did not have** the exonuclease activity of WRN.

The two routes then come apart, in the same run:

```text
sentence 2's claim  (ASSERTED)       ✓ still commits — nothing about sentence 2 changed
the derivation on sentence 1         ✗ REJECTED — inference.esl refused at commit
```

Sentence 1 parses to a different proposition, so no `IsDeclaredAs` witness matches the one the
recorded certificate cites for its antecedent. The inferred claim has nothing left to stand on —
while the document's own assertion of the same conclusion is untouched.

That asymmetry is the point. A conclusion the graph produced carries a live dependency on what it
was produced from; a document repeating it does not. Nothing compared the two texts.

(The edited run's rejection lands on `inference.esl` itself — the recorded argument is the only
resource that cites the measurement.)

## The two ways a claim gets justified here

| | what warrants it | grade | authoring cost |
|---|---|---|---|
| **pinned literature rule** | a published `∀m. A → B` on the chain, specialized with `spec_poly` and applied to a claim a sentence established | Declared | one rule, reused |
| **prose modus ponens** | `A` and `A → B` **both parsed from sentences** — the grammar renders `if` as native implication | **Derived** | none |

The lift from prose to domain vocabulary is not a third way: `onco-typed.esl` *defines* the domain
predicates over the parser's own lexicon, so `HasActivity(m, g, a)` and the parsed sentence are the
same term and the lift is definitional equality (D66). The kernel computes it; nothing declares it.
What matters is how many **unchecked assertions** a chain rests on, and definitions take that to
one: the literature rule, which genuinely is an assertion and is graded as one.

The second is the strongest: nothing is Declared, because the implication *is* a sentence. The
grammar's `if` entry is `λs₂. λs₁. (s₂ → s₁)`, and its design note says encoding it opaquely "would
forfeit modus ponens in the checker". `"S₁ if S₂"` parses to a genuine top-level implication whose
antecedent is verbatim the premise sentence's own parse — verified against the real snapshot.

Both are exercised by
[`justification_routes.rs`](../../crates/eigenius-reasoning/tests/justification_routes.rs), each
with a `Holds` case and a fail-closed case.

### Why the literature rule is hand-authored, and nothing else is

Its antecedent reads `HasActivity(m, WRN, exonuclease)` — a call anyone can type. That call *is* the
parse: `HasActivity` is a definition whose body is the DCG term, so the readable surface and the
formula are one thing rather than two connected by an assertion.

The term itself runs to hundreds of characters of nested applications that no one writes by hand
correctly — which is why nobody writes it. What is authored is the abbreviation and the claim that
uses it: the answer to an unwritable term is to name it, not to generate an assertion about it.

## Where the rejection actually comes from

A false proposition, on its own, commits fine. `qc_consistency_check` returns Undecidable for any
non-trivial input ([`reasoning.esl`](../../ontologies/reasoning/reasoning.esl)), so nothing checks a
standalone claim against anything. What rejects is the **certificate**.

### The three properties on a ReasoningSentence

| property | what it holds | encoding |
|---|---|---|
| `reasoning:proposition` | the domain claim `C` | D47 `eigentt:TypeExpr` |
| `reasoning:justification` | the *reason shape*: `App(SpecStr(DeclaredEvidence(rule), tag), DerivedEvidence(claim))` | D32 §3.7 tagged dict — a `JustificationTerm` **value** |
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

The certificate is one application of the Artemov rule, over an implication obtained by
specializing the ∀-quantified literature rule at the model:

```text
app( A, B,
     SpecStr(DeclaredEvidence(rule), tag),  DerivedEvidence(claim_1),
     spec_poly( Set, (fun m => A(m) → B(m)), DeclaredEvidence(rule),
                «MSI cancer models», tag,
                declared(rule, ∀m. A(m) → B(m), _) ),   ← the pinned rule, Declared, specialized
     derived (claim_1, A, _) )                          ← the parser's output, Derived
```

`spec_poly` eliminates the quantifier: from `JustifiedBy(j, ∀y:T. P(y))` it yields
`JustifiedBy(SpecStr(j, tag), P(x))`. `app` then composes: from `j1 : JustifiedBy(_, A → B)` and
`j2 : JustifiedBy(_, A)` it yields `JustifiedBy(App(j1,j2), B)`. **`A` must be the same term on both
sides** — the specialized rule's antecedent and the claim's proposition must be *identical*, not
merely compatible. Both reach it through the `HasActivity` definition, which unfolds to exactly the
committed parse (`has_activity_unfolds_to_exactly_the_committed_parse`), so the match holds by
construction rather than by an assertion kept in step by hand.

The trailing `_` on `declared` and `derived` is a `UnitVal`. The kernel **discards it and synthesises
the real witness itself**, from the chain witness index:

```text
WitnessKey { category: Derived, iri: claim_iri, prop_hash: sha256(encode(P)) }
```

`lookup_chain_witness` walks the layer and every ancestor
([`witness_index.rs`](../../kernel/src/layer/witness_index.rs)). The key is minted on the other side
by the parser's `reflection:DeclarationTrace`: a trace whose `reflection:resource` points at the
claim emits `IsDeclaredAs claim_iri P`, where `P` is the claim's own `canonical_proposition`.

Declared, not Derived, since eigenius#201 (D73 §6). The parser establishes that the text parses to
this well-typed term — not that the term is faithful to what the author wrote (D61, unbuilt), nor
that what the author wrote is true. A `derived` witness would have said a program established the
domain proposition, which it did not. What is unchanged is the part that makes the demo work: the
key hashes the PROPOSITION, so editing the prose still breaks the citation.

**The author of a certificate cannot supply the witness. Only the chain can.**

### Why the edit fails

```text
intact   claim_1.canonical_proposition = P            →  index key (Derived, claim_1, sha256(P))
edited   claim_1.canonical_proposition = P → False    →  index key (Derived, claim_1, sha256(P → False))

recorded certificate still contains        derived(claim_1, P, _)
kernel therefore looks up                  (Derived, claim_1, sha256(P))
```

The proposition is **hashed into the key**, so this is exact structural identity — no similarity
metric, no threshold. `sha256(P) ≠ sha256(P → False)`, the lookup misses, no inhabitant exists for
`JustifiedBy(DerivedEvidence(claim_1), P)`, `app` cannot be applied, and the certificate fails against
`JustifiedBy(justification, C)`.

Two things follow. The paragraph has to be an **argument**, not a list of facts — the edit only bites
because a certificate names the premise. And the rejection is indifferent to *how* the prose changed:
a synonym swap producing a genuinely different proposition fails identically, while a whitespace
change does not fail at all.

## Layout

| file | generated? | what it is |
|---|---|---|
| `paragraph.txt` | — | the two sentences, verbatim from CNL-v3 |
| `paragraph-edited.txt` | — | the same, with the measurement negated («had» → «did not have») |
| `onco-typed.esl` | — | domain predicates DEFINED over the parser's lexicon — `Set -> Set -> Prop`, model explicit. The activity concept is fixed IN each definition (C1148824 exonuclease / C1149627 helicase): abstracting it as a third parameter makes the body untypable, since `fst(the(Σ x0:a. …))` has type `a` and an abstract `a : Set` has no subsumption path to the verb axiom's `Entity` slot. |
| `pins.tsv` | — | the human-verified reading per sentence (see *Reading selection*) — the INTACT variant's authority |
| `ranks.json`, `ranks-edited.json` | recorded once each | the sense reranker's decisions, replayed — no LLM, no network, no key. One per variant: the replay key includes each word's candidate senses, so the edited paragraph is a different question. |
| `selections-edited.json` | recorded once | the reading-selection draw for the EDITED variant, replayed. It selects by draw, not by pin, because the negated sentence's pinned skeleton matches three readings differing only in sense — a tie a sense-erased pin cannot break, so the pin arm fails closed there (correctly). |
| `literature-rules.esl` | — | the pinned `∀m. A → B`, cited — the ONLY DeclarationTrace on the branch |
| `claims-intact.esl` | `prose-to-esl` | units + encoded claims + DeclarationTraces + decision points |
| `claims-edited.esl` | `prose-to-esl` | the same, from the edited prose |
| `inference.esl` | hand-authored | the CONCLUDED claim — the literature rule specialized at the model and applied to sentence 1's own parse |

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

## Why the argument is committed, not regenerated

`inference.esl` is the **recorded argument**: what someone concluded, at a point in time, from the
prose as it then read. The claims layers are a function of the prose. Re-deriving the argument on
every run would reshape it around any edit, and nothing would ever fail to commit — which is the one
thing the demo exists to show. `--reparse` regenerates only the claims.

The two halves run on two branches off one base, rather than as two loads onto one chain, because a
redefinition does not retract the earlier layer's witness: `IsDeclaredAs claim_1 P_original` would
still be reachable and the certificate would still resolve.

## What a `Holds` does NOT mean

Two witnesses stand behind the intact commit, and neither is about biology.

| witness | grade | what it actually attests |
|---|---|---|
| `IsDeclaredAs(claim_i, P)` | **Declared** | the named agent asserts `P`; the trace records that the DCG parser, run over bytes with a recorded sha256 at a recorded span, arrived at that FORM — a fact about **the text**, not a warrant for `P` |
| `IsDeclaredAs(lit_rule, ∀m. A(m) → B(m))` | **Declared** | a human asserts the published dependency between the two activities, with author and rationale |

Both are Declared since eigenius#201, and that is the honest reading: nothing in this demo derives
a claim about biology. What differs is only who asserts and on what basis.

`spec_poly` and `app` compose them and the result commits at **Declared** — a conclusion is no
better than its weakest link, and the rule is the weak one.

So the chain says *"the paper says the measurement, and we declare a rule under which it entails
the conclusion."*
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
exactly this reason: on this snapshot it yields two readings sharing the pinned skeleton, and the
pin arm refused to choose. That refusal is the design working — and the edited variant now hits the
same wall (three sense-variant readings under one skeleton), which is why it selects by the recorded
draw in `selections-edited.json` instead. The two arms are the same seam: a `ReadingRanker`.

**The literature rule is Declared, and has to be.** Definitions make the parse and the domain
formula one term, so the lift Declares nothing — but a rule relating two propositions is a claim
about the world, graded Declared.

That is a property of the logic, not an authoring shortcut: **there is no implication introduction.**
No rule produces `JustifiedBy(_, A → B)` — `app` yields `B`, `sum_l`/`sum_r` yield `P`,
`spec_poly` yields `P(x)`. An implication enters only through a grounding, so a rule
connecting propositions must be asserted; it cannot be proved.

## Appendix: `spec_poly`

The elimination this demo's certificate uses:

```esl
spec_poly :
    forall (T : Type 1, P : T -> Prop, j : JustificationTerm, x : T, tag : core:string) =>
    JustifiedBy(j, forall (y : T) => P(y)) -> JustifiedBy(SpecStr(j, tag), P(x)),
```

The only specialization rule in [`reasoning.esl`](../../ontologies/reasoning/reasoning.esl) (landed
2026-08-03; `reasoning.esl` is bootstrapped, so adding it cost a lexicon reseed). A monomorphic
`spec_str` over `core:string` stood beside it until 2026-08-21, when it was retired as strictly
subsumed (eigenius#203). `spec_poly` binds the domain type and the instance on the *proof* side
while the justification term carries only a string tag, so `JustificationTerm` needed no change;
only `JustifiedBy` gained a constructor. The domain binder is `Type 1`, not `Set`: a rule whose own
quantifier ranges over `Set` would otherwise need `Set : Set` to instantiate (eigenius#136).
`inference.esl` applies it at `T := Set` to specialize the literature rule at «MSI cancer models» —
a nested compound-kind term, not a class IRI. (D66 §9 records an undiagnosed universe question
about that instantiation; it type-checks today.)

Two further gaps, for completeness:

- **Anaphora with propositional antecedents.** Resolution is implemented (D64 — referent holes, LLM
  proposer, kernel re-gate), but `entity_candidates` collects only named-entity IRIs, so *"these
  findings"* resolves to a definite description over a *findings* kind rather than to the propositions
  it denotes. The paper's own conclusion sentence needs exactly that.
- **The inference structure.** *"These findings show that X"* yields a proposition; nothing extracts
  the justification term from the connective. Here the argument's shape is hand-authored in
  `inference.esl` and only the propositions come from the prose.
