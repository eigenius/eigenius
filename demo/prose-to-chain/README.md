# prose-to-chain

A paragraph of the WRN paper, parsed into typed propositions, committed to a chain — and then
edited until the kernel refuses it.

```bash
./demo/prose-to-chain/run.sh          # stages the lexicon snapshot, brings the kernel up, runs both halves
```

**Prerequisite: a lexicon snapshot.** The encoded claims' propositions are built from lexicon axioms
(`wn:v02627934_t` is the verb sense of *require*), so the chain they commit to must be the one that
*defines* those axioms — a bare core+domain chain fails at the D47 decode with `ConstRef references
unresolved IRI`. `run.sh` stages it into the kernel's docker volume; the snapshot itself is treated
as read-only.

```
<repo-parent>/db-snapshot/wordnet-umls-aligned-2026-08-03-sigmaproj   (993 MB)
```

Override with `EIGENIUS_DB_SNAPSHOT`. To build one:
`scripts/reseed-lexicon-db.sh` then `scripts/build-alignment-snapshot.sh`.

## What it shows

Two sentences of [CNL-v3](../../references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v3.txt)
prose:

```text
MSI cancer models required the helicase activity of WRN.
MSI cancer models did not require the exonuclease activity of WRN.
```

(A third — *"We found that WRN was selectively essential in MSI models"* — was dropped: on this
snapshot it yields **two readings sharing the pinned skeleton**, differing only in sense, and
`select_pinned` refuses to choose. That refusal is the design working, not a defect.)

The DCG parser (D63) turns each into a closed, felicity-gated `Prop`. Each lands on the chain as an
`enc:EncodedClaim` under a `reflection:ProgramTrace`, which mints the witness
`IsDerivedAs claim_i P_i`. A recorded argument then lifts each to a domain proposition through a
Declared bridge, as two `reasoning:ReasoningSentence`s whose certificates the kernel checks at
commit.

Then the demo deletes one negation from the second sentence:

> MSI cancer models ~~did not require~~ **required** the exonuclease activity of WRN.

The edited sentence still parses. It parses to the *same structural skeleton* as the helicase
sentence — same bracketing, and the skeleton eraser drops sense identity — so nothing syntactic
notices. The **proposition** is different, the certificate that cites the old one no longer
type-checks, `ValidateJustification` returns `Fails`, and the commit is rejected.

Nothing compared the two texts. The kernel rejected an argument that no longer follows from what the
document says.

## Layout

| file | generated? | what it is |
|---|---|---|
| `paragraph.txt` | — | the two sentences, verbatim from CNL-v3 |
| `paragraph-edited.txt` | — | the same, minus one negation |
| `pins.tsv` | — | the human-verified reading per sentence (see *Reading selection* below) |
| `ranks.json`, `ranks-edited.json` | recorded once each | the sense reranker's decisions, replayed — no LLM, no network, no key. One per variant: the replay key includes each word's candidate senses, so the edited paragraph is a different question. |
| `claims.tsv` | — | which domain proposition each sentence is taken to warrant, and on whose authority |
| `claims-intact.json` | `prose-to-eigon` | units + encoded claims + ProgramTraces + decision points |
| `claims-edited.json` | `prose-to-eigon` | the same, from the edited prose |
| `argument.json` | `prose-to-eigon --argument-out`, **once** | 2 Declared bridges + 2 ReasoningSentences |

The domain vocabulary is the WRN case study's own
[`01-onco.esl`](../../experiments/publications/wrn-helicase/chain/01-onco.esl), loaded unchanged.

`run.sh --reparse` re-derives the two claims layers from a lexicon snapshot
(`EIGENIUS_DB_SNAPSHOT`) instead of using the committed fixtures.

## Why the argument layer is generated once and committed

`argument.json` is the **recorded argument**: what someone concluded, at a point in time, from the
prose as it then read. The claims layers are a function of the prose. Regenerating the argument on
every run would re-derive it around any edit, and nothing would ever fail to commit — which is the
one thing the demo exists to show.

The two halves run on two branches off one base, rather than as two loads onto one chain, because a
redefinition does not retract the earlier layer's witness: `IsDerivedAs claim_2 P_original` would
still be reachable and the certificate would still resolve.

## Where the failure actually comes from

A false proposition, on its own, commits fine. `qc_consistency_check` returns Undecidable for any
non-trivial input ([`reasoning.esl`](../../ontologies/reasoning/reasoning.esl)), so nothing checks a
standalone claim against anything. What rejects is the **certificate**, and it is worth spelling out
exactly which pieces meet, because the demo is only interesting if the failure is a type error rather
than a comparison.

### The three properties on a ReasoningSentence

| property | what it holds | encoding |
|---|---|---|
| `reasoning:proposition` | the domain claim `C`, e.g. `DispensableActivity("WRN","exonuclease")` | D47 `eigentt:TypeExpr` |
| `reasoning:justification` | the *reason shape*: `App(DeclaredEvidence(bridge), DerivedEvidence(claim))` | D32 §3.7 tagged dict — a `JustificationTerm` **value** |
| `reasoning:certificate` | the proof that the reason actually warrants the claim | D47 `eigentt:TypeExpr` |

The justification and the certificate are two different things, and both are needed: the
justification says *which evidence*, the certificate says *why that evidence suffices*. A sentence
can name perfectly real evidence and still fail, because the certificate is what has to type-check.

### What the gate does

[`do_validate_justification`](../../crates/eigenius-reasoning/src/validate.rs) runs at commit, via
the `AutoOnLoad` hook on every `reasoning:ReasoningSentence`:

1. **Decode** `proposition` and `certificate` through the D47 codec against the current chain — this
   is where `ConstRef`s (`onco:DispensableActivity`, `wn:v…`, `umlscui:C…`) re-resolve to real
   inductives and axioms.
2. **Lift** `justification` into a `Val::InductiveVal` typed at `reasoning:JustificationTerm`.
3. **Resolve** the `reasoning:JustifiedBy` inductive declaration from the layer.
4. **Type-check the proposition at `Prop`** (`Sort(0)`) and evaluate it to a `Val`.
5. **Construct the expected type** — and this is the crux:

   ```text
   JustifiedBy(justification_val, proposition_val)
   ```

   `JustifiedBy : JustificationTerm -> Prop -> Type 0` is an **indexed** inductive, so the
   justification and the proposition sit in its *index slots*. The expected type is therefore
   parameterised by exactly the reason given and exactly the claim made.
6. **Type-check the certificate against that type** with the kernel's NbE checker.
7. `Holds`, or `Fails` carrying the kernel's type-error string.

### Where the prose enters

The certificate the demo builds is one application of the Artemov rule:

```text
app( P, C,
     DeclaredEvidence(bridge),  DerivedEvidence(claim),
     declared(bridge, P → C, _),        ← the human's lift, Declared
     derived (claim,  P,     _) )       ← the parser's output, Derived
```

`app` is a constructor of `JustifiedBy`: given `j1 : JustifiedBy(_, A → B)` and
`j2 : JustifiedBy(_, A)`, it yields `JustifiedBy(App(j1,j2), B)`. Note that **`A` must be the same
term on both sides** — the bridge's antecedent and the claim's proposition must be *identical*, not
merely compatible. (That invariant has its own test:
`bridge_antecedent_and_derived_grounding_embed_the_same_subtree`.)

The trailing `_` on `declared` and `derived` is a `UnitVal`. The kernel **discards whatever is there
and synthesises the real witness itself**, by looking up the chain witness index:

```text
WitnessKey { category: Derived, iri: claim_iri, prop_hash: sha256(encode(P)) }
```

`lookup_chain_witness` walks the layer and then every ancestor
([`witness_index.rs`](../../kernel/src/layer/witness_index.rs)). The key is minted on the other side
by the parser's `reflection:ProgramTrace`: a trace whose `reflection:resource` points at the claim
emits `IsDerivedAs claim_iri P`, where `P` is the claim's own `canonical_proposition`.

So the author of a certificate cannot supply the witness. Only the chain can.

### Why the edit fails

Deleting the negation changes nothing syntactic — the edited sentence parses to the *same skeleton*
as the helicase sentence. What changes is the term:

```text
intact   claim_2.canonical_proposition = P    →  index key (Derived, claim_2, sha256(P))
edited   claim_2.canonical_proposition = P′   →  index key (Derived, claim_2, sha256(P′))

recorded certificate still contains        derived(claim_2, P, _)
kernel therefore looks up                  (Derived, claim_2, sha256(P))
```

The proposition is **hashed into the key**, so this is exact structural identity — no similarity
metric, no threshold. `sha256(P) ≠ sha256(P′)`, the lookup misses, no inhabitant exists for
`JustifiedBy(DerivedEvidence(claim_2), P)`, the `app` constructor cannot be applied, and the
certificate fails to check against `JustifiedBy(justification, C)`. `ValidateJustification` returns
`Fails` and the commit is rejected.

Two things follow. The paragraph has to be an **argument**, not a list of facts — the edit only bites
because some certificate names the premise. And the rejection is indifferent to *how* the prose
changed: a synonym swap that produced a genuinely different proposition would fail identically, while
a whitespace change would not fail at all.

## What a `Holds` does NOT mean

Two witnesses stand behind the intact commit, and neither is about biology.

| witness | grade | what it actually attests |
|---|---|---|
| `IsDerivedAs(claim_i, P)` | **Derived** | the DCG parser, run over bytes with a recorded sha256 at a recorded span, produced proposition `P` — a fact about **the text** |
| `IsDeclaredAs(bridge_i, P → C)` | **Declared** | a human asserts that a sentence meaning `P` warrants the domain claim `C`, with author and rationale |

`app` composes them and `BridgedClaimGrader` stamps the result **Declared** — a conclusion is no
better than its weakest link, and the lift is the weak one.

So the chain says *"the paper says it, and we declare that its saying so warrants the claim."*
**Nothing here witnesses that WRN's exonuclease activity is in fact dispensable.** A `Holds` proves
the certificate type-checks against admitted witnesses — D61's oracle #1, structural validity. It is
not oracle #2 (faithfulness), and it is certainly not truth.

That is exactly why the *failure* case is the interesting half: the gate catches an argument no
longer following from its recorded premise, which is real and useful, and is not "the kernel knows
the biology."

A stronger witness is possible in principle. `SelectivelyEssential("WRN","MSI")` is also warranted by
`wrn:concl_wrn_selective_recomputed`, whose certificate cites an `IsDerivedAs` the **statistics
institution** minted by recomputing a Wilcoxon over 37 vs 91 cell lines from pinned data — a witness
about the world, not the text. The helicase/exonuclease claims have no such leg; in the real WRN
chain they are narrative, so "the authors report it" genuinely is the strongest available warrant.

## What this build uncovered

Three defects, none demo-specific, all found by the demo failing to commit:

**D47 had Σ-introduction but no Σ-elimination.** `Pair` existed; `Fst`/`Snd` did not. The DCG renders
every definite description as `the(Σx:C. P(x)).1`, so **no parsed sentence containing a definite noun
phrase could be committed to a chain at all.** Fixed by adding both constructors and their codec arms
([`eigentt-type-fragment.json`](../../ontologies/eigentt/eigentt-type-fragment.json),
[`eigentt_type_mirror.rs`](../../kernel/src/program/eigentt_type_mirror.rs)). `eigentt` is
bootstrapped, so this also forced a lexicon reseed.

**Indexed inductives' constructors could not be inferred.** The `Exp::InductiveCtor` inference arm
passed empty expected indices and answered `indices: []`, so any such term failed validation Rule 21
with `index arity mismatch (actual has 2, expected has 0)`. `reasoning:JustifiedBy` is indexed —
therefore **no `reasoning:certificate` could pass commit validation**, including the WRN case study's
own `chain/04-phase1-recompute-conclusions.esl`, which was confirmed failing identically. `nanoda_lib`
settled the fix: Lean has no constructor node, so `infer_app` walks the Pi telescope and returns
`inst(fun, ctx)` — indices fall out of substitution, no expected type needed. The equivalent value
was already computed in `check_inductive_ctor_args` and discarded.

**`ontologies/encoding/encoding.esl` did not compile.** Three references used undeclared
sub-namespaces (`institution:runtimes:external`), and `requires_environment` pointed at a stub
resource nothing defines. The D62 ontology had never been loaded.

A common thread worth noting: the in-process grader tests build layers with `LayerBuilder` directly,
which **does not run the validator**. Both the Rule 21 failure and a missing `DeclarationTrace`
timestamp were invisible until a real `eigenius load`.

## Two limits, stated

**Reading selection is declared, not solved.** The reference page runs 60 of 62 units ambiguous.
This demo does not decide which reading is right — it takes the one whose sense-erased skeleton
equals a human-verified pin from
[`expected-readings.tsv`](../../experiments/parsing/expected-readings.tsv), and **fails closed** if
the pin matches zero readings (stale pin, or the grammar moved) or several (they differ only in
sense, and choosing between them is not the pipeline's call). Every selection is recorded on chain
as an `enc:DecisionPoint` with the pin as its rationale, so the chain always says on whose authority
the reading was taken. Under the reranked replay, sentences 2 and 3 reach a single reading anyway;
sentence 1 offers 4.

**Bridges do not generalise.** A parsed proposition is in WordNet/UMLS vocabulary
(`wn:v01234…`, `umlscui:C…`); a domain conclusion is in `onco:`. The bridge between them is a ground
implication `P → C` between two closed propositions, which is why no term translation is needed —
and why it buys nothing for the next sentence. One Declared bridge per claim, authored in
`claims.tsv` with its author and grounds. That is the same bargain the WRN chain already makes for
its statistical→domain lifts
([`wrn:bridge_msi_selective`](../../experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl)).

Sentence 1 has a third wrinkle: it parses under a reporting attitude — `found(P, speaker)`, not `P`
— so its bridge licenses passing from *the authors report it* to *it holds*, and says so in its
rationale. Sentences 2 and 3 are attitude-free.

## What is still missing

- **Anaphora with propositional antecedents.** Resolution itself is implemented (D64 — referent
  holes, LLM proposer, kernel re-gate). But `entity_candidates` collects only named-entity IRIs, so
  *"these findings"* resolves to a definite description over a *findings* kind rather than to the
  two propositions it denotes. The paper's conclusion sentence — «These findings show that WRN is a
  synthetic-lethal vulnerability for MSI cancers» — needs exactly that, which is why it is not in
  this paragraph.
- **The inference structure.** *"These findings show that X"* yields a proposition; nothing extracts
  the justification term from the connective. Here the argument's shape is Declared in `claims.tsv`
  and only the propositions come from the prose.
- **Structural disambiguation** (D62 S4) — see *Reading selection* above. The dropped third
  sentence is this gap concretely: two readings, same skeleton, and nothing in the pipeline entitled
  to pick.
- **Implication introduction.** Justification logic here has none: no rule *produces*
  `JustifiedBy(_, A → B)`, so a bridge can only ever be Declared, never derived. That is why bridges
  do not amortise — it is a property of the logic, not an authoring shortcut. The next step is
  quantified shape-rules specialised with `spec_str`, which needs a class-typed sibling of `SpecStr`
  (it is monomorphic on `core:string` today).
