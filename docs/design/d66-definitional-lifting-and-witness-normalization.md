# D66 — Definitional lifting: transparent definitions, explicit context, and symmetric witness normalization

*Status: design — **decision-complete** (§6 D1–D10 all ✅); ready to implement (§5 slices). No code yet. Motivated by the shape-rule
amortisation investigation ([`docs/notes/2026-08-09-shape-rule-amortisation.md`](../notes/2026-08-09-shape-rule-amortisation.md),
issues #111/#112): every lift from a parsed sentence to domain vocabulary is currently a **Declared**
bridge, one per parse shape — measured at ≥61 bridges for 62 sentences. The cause is not the bridge
generator; it is that domain predicates are declared as **opaque axioms**, so nothing but an assertion
can connect them to a parse. Resolution: declare them as **δ-transparent definitions** over lexicon
vocabulary, make the hidden context parameter **explicit**, and eliminate the resulting `∀` with the
existing `spec_poly`. The blocker is a **normalization asymmetry in the witness index** (§4), which must
land first.*

## 1. Problem

### 1.1 Every parse→domain lift is a Declared assertion, one per parse shape

`demo/prose-to-formulas` lifts each parsed sentence into domain vocabulary through a generated
**shape rule** — `∀ v… : Set. <parse shape>(v…) → Pred(v…)` — committed as a
`reflection:DeclaredResource` (`crates/eigenius-reasoning/src/grade.rs::build_shape_rule`). Two
sentences share a rule only when their parses coincide apart from the argument classes.

Measured on `experiments/parsing/expected-readings.tsv` (62 sentences of the WRN paper, human-verified
readings; reproduce with `python3 experiments/parsing/skeleton-abstraction.py`):

| | count |
|---|---|
| sentences | 62 |
| distinct sense-erased skeletons | **61** |

Two propositions with the same shape are identical except at the argument-class positions, and those
hold WordNet synsets or UMLS CUIs, which `erase_senses` collapses to the same `§`. So shape-equality
implies skeleton-equality, and the current scheme yields **at least 61 Declared bridges
for 62 sentences**. The skeletons are sense-erased (`kernel/src/dcg/skeleton.rs`), so 61 is what a
*perfect lexical abstraction* would reach — no lexicon resource (VerbNet, FrameNet, PropBank,
Predicate Matrix) can reduce it. The diversity is structural, not lexical.

**The count is not the real cost.** A Declared bridge is an unchecked assertion carrying a
`declared_by`. Sixty-one of them is sixty-one epistemic leaps that the kernel accepts on authority.

### 1.2 The cause: domain predicates are opaque axioms

`demo/prose-to-formulas/onco-typed.esl` declares

```esl
data onco2:HasActivity : Set -> Set -> Prop { }
```

A zero-constructor inductive is **opaque by construction** — the same device `reasoning.esl` uses
deliberately for the `ChainWitness` predicates ("Zero ctors enforces opacity"). Nothing relates
`HasActivity` to any parse, so the only available connection is an assertion. The bridge is forced by
the *declaration form*, not by the domain.

ESL offers no alternative. Three ways exist to introduce a `Prop`-valued name, and none is transparent:

| form | result |
|---|---|
| `data P : … -> Prop { }` | zero ctors — opaque |
| `axiom P : …` | type only, no body — opaque |
| `alias x = e in body` | compile-time substitution inside one `type_expr(…)`; inlines and vanishes, no shared name |

`Decl::Def` — the kernel's real δ-binding — exists in NbE but is emitted from exactly one place,
`kernel/src/program/expr.rs:358`, the executable-program body path. **ESL never emits it in declaration
position**, and it would not be the right thing if it did (§1.2a).

### 1.2a A chain-resident definition is a third binder

Neither existing binder can be a definition, and the axis is not syntactic position — it is how the
name resolves:

| | `Let` / `Decl::Def` | `axiom` / `EigonAxiom` | needed |
|---|---|---|---|
| name | local `Patt::Var` | IRI | IRI |
| resolved by | position in `Rho` | nothing | the layer chain |
| extent | the `let`'s body, one term | every resource | every resource |
| unfolds | yes | **never** | yes |

`Exp::EigonAxiom(iri)` evaluates to `Val::Nt(Neut::EigonAxiom(iri))` — a **neutral**, stuck by
construction (`kernel/src/nbe/eval/mod.rs:509`). That rigidity is correct for a genuine axiom:
`ontology:kind_of` and `ontology:the` must not unfold. And `eval(exp, rho)` takes only a `Rho`
(`kernel/src/nbe/eval/mod.rs:155`) — **there is no layer in scope during evaluation at all.**

So a definition needs an IRI-named constant the system *can* unfold, which neither form provides.
`kernel/src/esl/lexer.rs:48-54` reserves the `Let` token for a **scoped type-position let** —
`let x : T = e in <type expr>`, the real-δ counterpart to `alias`'s compile-time substitution, inside a
single `type_expr(…)`. That is a different feature and keeps the token. See D5.

### 1.3 The predicates hide the context in their arity

`HasActivity : Set -> Set -> Prop` has slots for a gene and an activity, and none for the model the
activity was measured in. The parse does mention it: measured on `rule_1`, the antecedent's second
argument holds `umlscui:C0920269` (MSI cancer models) and the consequent holds only `v0`/`v1`.

So the lift discards the experimental context, and a discarding step is an implication. Since
**no `JustifiedBy` constructor produces `JustifiedBy(_, A -> B)`** (`ontologies/reasoning/reasoning.esl:26-31`;
the nine constructors are four groundings, `app`, `sum_l`, `sum_r`, `spec_str`, `spec_poly`), that
implication can only enter as a grounding — i.e. Declared.

This is worse than an extra artifact: the generalisation from *"MSI cancer models had WRN's exonuclease
activity"* to a context-free *"WRN has exonuclease activity"* is a **silent universal quantification
hidden in an arity**. It is invisible in the predicate's type and never stated as a claim.

### 1.4 Why this is not #111's or #112's problem

Both alternatives were investigated and neither addresses §1.2.

- **#111 — key rules on verb frames.** Ruled out three ways: sense-erased skeletons already assume
  perfect lexical abstraction (§1.1); generalising a rule's antecedent needs a Declared `P → A` per
  parse shape, conserving the cost; and the consequent's arguments are *intra*-argument co-occupants
  related by `prep_of`, not role fillers, so a frame-keyed rule cannot express the conclusion.
- **#112 — interpret `eigentt:TypeExpr` in the theory.** Needed for rules that *quantify over shapes*
  (case analysis on syntax). A parse-shaped proposition is already a `Prop` and already usable as an
  implication antecedent; what is missing is **abbreviation**, not reification.

## 2. The design

### 2.1 Domain predicates become definitions over lexicon vocabulary

```esl
def onco2:HasActivity (m : Set) (g : Set) (a : Set) : Prop =
    wn:v02203362_t(
        eigentt:fst(ontology:the(exists x : wn:n13440063 =>
            logic:And(ontology:compound_kind(x, a),
                      ontology:prep_of(x, ontology:kind_of(g))))),
        ontology:kind_of(m))
```

`HasActivity(MSI_cancer_models, WRN, exonuclease)` then **δ-unfolds to the parse** — not to something
weaker, to the same term. Nothing is discarded, so there is nothing to justify: the lift is
definitional equality. Per D5 the unfolding happens at **decode**, so the two are literally the same
`Exp` by the time anything type-checks or hashes them.

The surface stays as readable as it is today. The definition is an abbreviation, not a new opaque
thing to bridge to.

### 2.2 The context parameter is explicit; the literature rule quantifies over it

```esl
∀ (m : Set). onco2:HasActivity(m, umlscui:C0388246, wn:n14606137)
          -> onco2:RequiresActivity(m, umlscui:C0388246, umlscui:C0920283)
```

Declared once — correctly, it *is* a literature claim — and now honestly general rather than a claim
about WRN in no particular context. Application to «MSI cancer models» is `spec_poly` at `m`, which is
the mechanism `demo/prose-to-formulas/inference.esl` uses to eliminate the quantifier.

### 2.3 What this does to the accounting

| | today | with D66 |
|---|---|---|
| parse → domain | Declared shape rule, one per parse shape | δ-conversion, free |
| literature rule | context-free, silently universal | `∀ m`, Declared once |
| instantiation | — | `spec_poly` at `m` (existing) |
| **Declared artifacts** | **≥61 + 1** | **1** |

The residual cost is one **definition** per parse shape — the 61 does not drop. But a definition is
content-preserving and the kernel checks the equality, so N definitions is a vocabulary-size problem,
not N unchecked leaps. **The number worth minimising is the Declared count, and it goes to one
independent of corpus size.**

### 2.4 Instantiating a definition: peel and substitute (D8)

The definition is stored as a λ-body — `Lam(m, Set, Lam(g, Set, Lam(a, Set, B)))`, reusing `TypeExpr`'s
existing `Lam` (3 args: name, dom, body; `kernel/src/program/eigentt_type_mirror.rs:453-465`). Arity and
parameter types come from the declared type, so nothing is stored twice.

**`B` is stored already normalized** (D9): `def` commit normalizes the right-hand side, so a use decodes
straight to a normal term and nothing downstream has to evaluate.

`HasActivity(MSI, WRN, exo)` encodes as the spine `App(App(App(ConstRef(HasActivity), MSI), WRN), exo)`.
Naïve δ would resolve the head to the λ-body and leave three β-redexes, which is why the emit side would
then need an evaluator (§4). **Decode never constructs them.** Its `"App"` arm is already head-aware — it
decodes head and arg and, when the head is an `InductiveType` / `CodataType` / `InductiveCtor`, folds the
arg onto the head instead of building an `App` (`kernel/src/program/eigentt_type_mirror.rs:474-482`). One
more arm, for "head is a transparent definition body": peel a leading `Lam`, substitute, return.

| step | head after decode | action | result |
|---|---|---|---|
| 1 | `Lam(m, Lam(g, Lam(a, B)))` | peel, `m := MSI` | `Lam(g, Lam(a, B[m:=MSI]))` |
| 2 | `Lam(g, Lam(a, …))` | peel, `g := WRN` | `Lam(a, B[m:=MSI, g:=WRN])` |
| 3 | `Lam(a, …)` | peel, `a := exo` | `B[m:=MSI, g:=WRN, a:=exo]` |

Out comes the parse-shaped term, β-normal, no redex ever existing.

- **This is not evaluation.** Each step consumes one leading `Lam` and one spine argument; it terminates
  by structural decrease on the spine, bounded by `min(#Lams, #args)`. No fixpoint, no `Rho`, no readback.
  D5 holds: decode substitutes, eval evaluates.
- **Under-application falls out.** `HasActivity(MSI)` peels once and stops at
  `Lam(g, Lam(a, B[m:=MSI]))` — still β-normal. Over-application cannot arise in a well-typed term.
- **Opacity is "don't take the arm."** An opaque definition resolves like an axiom, the spine stays
  `App(ConstRef(f), …)`, which is exactly today's behaviour. #95's mode is a branch condition at the
  head, not a separate mechanism.
- **Definitions are non-recursive.** A recursive body puts decode in `Decl::Drec` territory where it is
  no longer total (issue #66). Parse abbreviations do not need recursion; forbidding it is what keeps
  decode terminating, and it is a commit-time check on the definition resource.

**What this needs that does not exist: a total, capture-avoiding substitution on `Exp`.** There is none in
`kernel/src/nbe/term.rs` or the mirror. The only one in the tree is `beta_normalize`'s private helper
(`kernel/src/dcg/rules/combinators.rs:1592`), which is **deliberately partial** — it declines to reduce
when the argument shares a name with a binder in the body rather than freshening, because it feeds a sort
key where a missed reduction costs nothing. Decode cannot fail soft that way: a declined substitution
leaves a redex and silently breaks the §4 hash agreement.

### 2.5 The same primitive already serves anaphora resolution (D64)

D64 resolves a pronoun by **applying** the sentence's open sem to its antecedent. From
`kernel/src/dcg/parse/resolve.rs`:

> The open sem is `λ(h₀:T₀)…(hₙ:Tₙ). body` (D64 — a parametric proposition). Resolution is
> APPLICATION: apply each hole's antecedent in binder order […], then β-reduce.

and it does so the redex-forming way — builds `App(App(sem, a₀), a₁)`, then `readback_val(0, &eval(…))`.
A definition is also a parametric proposition and instantiating it is also application: **one primitive,
two consumers.** That D64 reached the same shape independently is evidence the §2 structure is not
invented for this document.

Routing `resolve_open` through the same substitution is **follow-up, not a slice** — it would replace an
NbE round-trip with bounded structural work and stop `readback_val` renaming every binder in the sentence
to `G#n` when only the hole was touched. Two things to check first, so this is not assumed:

1. **`eval` does more than β.** The comment says "then β-reduce", but the evaluator also performs δ and ι.
   Confirm nothing at that call site depends on full normalisation before swapping in a β-only step.
2. **`resolve_open` runs a closed re-gate afterwards** — `check(&mut ctx, &nf, &expected_val)`, *"the
   kernel veto that keeps the LLM from having the last word"*: a type-mismatched antecedent fails, and a
   leftover unbound hole fails closed. For definitions the equivalent falls out of ordinary type-checking
   on the result, but the *timing* differs and the difference should be deliberate.

## 3. Why the lift must not be a normalization step

An earlier draft of this work proposed emitting a lossy "normal form" of each parse as a second
derived witness, and keying rules on that. Recorded here so it is not re-proposed:

- A lossy map `P ↦ F` makes the *normaliser* the trusted component, for **faithfulness** — the oracle
  the commit gate cannot give. Its faithfulness would be graded Derived and never Verified.
- The classes a rule's consequent names sit **six constructors inside the verb's argument**
  (`App Fst App Sig App App App Var`, from `rule_1`), under a definite description and an existential.
  Discarding argument structure deletes the variables the rule binds, and `build_shape_rule` fails
  closed on it (`GradeError::ArgumentNotInProposition`).
- `ontology:kind_of : Set -> lexicon:Entity` and `ontology:the : forall (A : Set) => A` are the only
  routes from a class to an entity, while a transitive verb axiom is `Entity -> Entity -> Prop`
  (`crates/eigenius-wordnet/src/convert.rs:210`). **The scaffolding that must be discarded for reuse is
  the scaffolding that types the argument.**

A definition avoids all three because it preserves content: there is no faithfulness question, nothing
is deleted, and the types are unchanged.

## 4. The prerequisite — symmetric normalization at the witness index

The witness key is `(category, iri, prop_hash)` (`kernel/src/witness/mod.rs`). The two sides that must
agree on `prop_hash` do not compute it the same way:

| side | path | normalizes? |
|---|---|---|
| **lookup** (type-check) | `kernel/src/program/check_hooks.rs:76` — `readback_val(level, &indices[1])`, then `WitnessKey::from_exp` | **yes** — the proposition arrives as a `Val`, already evaluated by NbE, so δ has happened |
| **emit** (witness-index build) | `kernel/src/layer/witness_index.rs:206,223,249` — `hash_proposition_value(encoded_prop)` on the stored `Value::Json` | **no** — hashes what the author typed |

**Where the emit side actually runs.** Not at layer build or persist. `build_witness_index` is called
lazily from `Layer::chain_witness_index` through a `OnceLock` (`kernel/src/layer/mod.rs:541-546`), and
the trigger is `check_layer_with_coercion` during **type-checking**, when a certificate needs a witness
(`kernel/src/layer/witness_index.rs:356`). The index is a pure function of the layer's resources and is
**not persisted** — content-addressing covers it transitively through the Trace resources. So it is
built at most once per layer per process, and only for layers something asks about.

`alpha_canonicalize_proposition_json` (`kernel/src/witness/mod.rs:181`) is not a general normalization policy. It
is a targeted patch for the one symptom of this asymmetry that already bit — NbE readback freshens
binder names, so author-supplied names never matched. Its doc comment says exactly that
(`kernel/src/witness/mod.rs:130-136`). *Verified*: two α-variants of `∀(x:Set). Eq(x,x)` hash identically and
produce equal `WitnessKey`s.

**δ-divergence is the same bug in the same seam**, latent only because ESL has no definitions to
diverge over. It fires on the first definition committed.

Fixing δ does **not** retire α. The binder-name asymmetry it patches is permanent under §4.1: the check
side reads back and freshens to `G#n`, decode carries the author's name through unchanged
(`kernel/src/program/eigentt_type_mirror.rs:431-439`), so the two never agree on names by themselves.
α-canonicalization stays as mechanism — D4.

### 4.1 Fix the seam, not the hash

Making `hash_proposition_value` δ-normalize is the wrong move:

- It needs a layer to resolve `ConstRef`, so `prop_hash` becomes **layer-relative**. Today it is a
  content hash of stored bytes; then it is a hash of a normal form relative to a chain state, and a
  descendant layer that adds or shadows a definition changes the normal form of an already-committed
  proposition. Witnesses silently stop resolving.
- δ-normalization is not guaranteed total (`Decl::Drec`, issue #66), and hashing is on the commit path
  for every witness.
- Whether to unfold is a **per-definition policy**: an opaque definition is an abstraction barrier you
  want citations keyed to; a transparent one you want unfolded. That is issue **#95**.

**Instead, make the emit side decode.** Per D5, δ happens at *decode* — the pass that already resolves
`ConstRef` against the layer — not at eval. So the two ends agree as soon as the emit side stops
hashing raw stored JSON and decodes first, exactly as the check side already does:

| | stored | decoded / hashed |
|---|---|---|
| author writes | `HasActivity(m, g, a)` | the unfolded parse |
| check side | — | already unfolded (decode → eval → `Val`) |
| emit side, today | hashes the folded JSON | — |
| emit side, fixed | keeps the folded JSON | decodes, then hashes the unfolded form |

Decode is what makes the decoded term the normal form: it performs δ (recursively, D9) and
peel-and-substitute forms no redex, while Rule 24 has already refused any body carrying one. So
"decode, then hash" and "hash the normal form" are the same operation, and the two sides agree by
construction rather than by coincidence.

`prop_hash` stays a hash of a *term*, and there is one normalization path instead of two kept in step
by hand. **The stored form stays folded** — see D10 for why, and for what that does and does not buy.

This is a smaller change than "evaluate in the layer": the emit side must decode anyway to obtain an
`Exp`, and decode is where the layer already is.

### 4.2 This makes a latent fail-open live

`hash_proposition_value` is infallible today, so witness-index construction cannot fail. Decoding can —
and there is nowhere for the failure to go:

- `build_witness_index(layer) -> BTreeMap<WitnessKey, ()>` (`kernel/src/layer/witness_index.rs:75`) has
  **no error channel**.
- Its per-resource emitters return `Option<WitnessKey>` and `None` is dropped without a trace
  (`emit_from_reasoning_sentence`, `emit_from_institution_derivation`, `emit_from_trace`).
- `Layer::chain_witness_index` builds it inside `OnceLock::get_or_init`
  (`kernel/src/layer/mod.rs:541-546`), which cannot return a `Result`.

So a proposition that failed to decode would produce no key, the lookup would miss, and the citing
sentence would fail as *"no witness"* rather than *"this witness exists but its proposition did not
decode"* — the two are indistinguishable to the author.

**Correction (2026-08-09).** An earlier draft attributed this to claims-audit **A2**, the discarded
`Result`s at `kernel/src/layer/mod.rs:1165,1176`. That is wrong: those govern the **triple / text /
value** indexes, which are persisted and populated at `populate_layer_indexes`. The witness index is a
different mechanism — lazy, in-memory, never persisted (§4). **A2 is a real defect but not a
prerequisite of this work**, and slice 0 targets `build_witness_index`, not `populate_layer_indexes`.

## 5. Implementation plan (slices)

Ordering is forced: 0 before 1 before 2, or the first committed definition produces witnesses that do
not resolve.

**Slice 0 — replace the witness index with direct lookup (constant footprint).**

Today `build_witness_index` materialises **every** witness key in a layer into a
`BTreeMap<WitnessKey, ()>` cached in a `OnceLock`, then answers a lookup by membership test. Two
consequences: memory is O(trace resources) per layer, held for the layer's lifetime; and a miss is a
bare `false` that cannot say why.

Neither is necessary, because **the key already contains the IRI** — `WitnessKey { category, iri,
prop_hash }`. Given a key, go straight to the resource that would produce it:

| the key's origin | how to reach it directly |
|---|---|
| self-attesting (`ReasoningSentence` → Verified, `InstitutionEmittedDerivation` → Derived) | the key's IRI **is** the resource: `layer.resolve(iri)` |
| trace-attested (`DeclarationTrace` / `ObservationTrace` / `ProgramTrace`) | `scan_predicate_object(reflection:resource, key.iri)` — `reflection:resource` is `core:resource`-typed, so it is in the triple index |

Then read that one resource's proposition (or the `Asserts(iri)` default), hash, compare. **No index is
built and nothing is retained** — constant footprint, and the persistence question disappears with it:
there is no derived structure to keep in step, so it can neither go stale nor be half-written.

This **subsumes the diagnostic goal**. The miss now happens while holding the specific resource, so it
distinguishes *no trace at all* / *trace exists, wrong category* / *category matches, proposition hashes
differently* / (after slice 1) *proposition failed to decode*. Surfacing the third case is given as the
reason `prop_hash` is in the key at all (`kernel/src/witness/mod.rs:86-89`), and it is invisible today
because the answer is one membership bit.

Scope: `check_layer_with_coercion` (`kernel/src/layer/witness_index.rs:356`) is the only consumer of the
whole map — it calls `contains_key`. Every other call site is `let _ = layer.chain_witness_index();`,
force-population that becomes a no-op and is deleted. `Layer::chain_witness_index`, its `OnceLock`
field, `build_witness_index`, and `chain_witness_index_for_test_set` all go.

Preserve exactly: the Derived→Verified coercion, the `Asserts(iri)` default, and the parent-chain walk.
In-flight and no-backend layers have no populated triple index — `witness_candidates` already
special-cases them with `iter_resources()`; the direct version keeps that fallback but **short-circuits
on first hit** instead of collecting, so it stays constant memory and linear time, on those layers only.

**Keep the layer skip — it was the index's real job.** The index did not answer the match; it *avoided*
layers that could never answer it. Five `scan_predicate_object(is_a, …)` probes returned nothing on a
lexicon layer, and the empty `BTreeMap` was then cached in the `OnceLock`, so every later lookup on that
layer was one membership test on an empty map — free. The measured 127 s was the *first* visit to each
ancestor; the cache is what stopped it recurring. Direct lookup alone is cheaper on first visit (one
targeted probe instead of five) and unboundedly worse after, because a certificate makes many lookups
and each walks the chain: past roughly *k > 5* lookups the cached design wins.

So the skip is preserved as **one stamped bit on `LayerHandle`**, not as a cache:

```rust
/// false iff this layer defines no resource that could ever admit a ChainWitness.
#[serde(default = "witness_candidates_unknown")]   // absent ⇒ true
pub has_witness_candidates: bool,
```

- **Constant footprint** — one bool per layer, in metadata already held in memory ("bounded by the
  number of layers, not by graph size") and already carrying derived hints stamped at write time
  (`resource_count`, `byte_size`).
- **Cheaper than caching** — computed once *ever*, at commit, folded into the resource walk
  `store_layer` already performs for `byte_size`. A `OnceLock<bool>` would recompute per process.
- **Cannot go stale** — layers are immutable, and it rides the handle's own write rather than a
  separate index batch, so claims-audit **A1** does not apply. This is why the earlier
  "don't persist derived witness data" argument does not extend to it: that argument was about an
  O(traces) structure in its own unsynced batch, and this is neither.
- **Defaults to `true`, and must.** Handles written before the field decode without it; `true` means
  "no information, go and look". `false` would mark every pre-existing layer witness-free and silently
  break every citation into history — a performance hint turned into a correctness bug.

**No `SCHEMA_VERSION` bump.** D24's criterion is whether a kernel built before the change fails to read
a DB written after it. `LayerHandle` is CBOR via ciborium, which writes structs as named maps, and serde
ignores unknown keys. Both directions are pinned by tests rather than assumed:
`handle_without_witness_flag_decodes_as_unknown` (old DB → new kernel yields `true`) and
`handle_with_unknown_field_still_decodes` (new DB → old kernel skips the field).

*Gate:* every existing witness test passes unchanged; no per-layer allocation proportional to trace
count; a layer stamped witness-free is skipped without probing; a miss caused by a proposition mismatch
names the resource and the mismatch.

**Status (2026-08-09): implemented; correctness verified, cost not yet measured.**

Verified — `cargo test --workspace` 2745 passed / 0 failed, clippy clean under `-D warnings`; and
`demo/prose-to-formulas/run.sh --reparse` end to end against
`wordnet-umls-aligned-2026-08-03-specpoly`: the intact branch commits all four layers and the
inference, the edited branch refuses `bridge-s1` and `inference.esl` with `qc_validate_justification`
→ `Fails` while `bridge-s2` still commits. Regenerating the claims/rules/bridges artifacts reproduced
the committed files exactly (modulo one trailing newline).

**Not yet measured, and the demo run could not measure it.** That snapshot predates the handle field,
so every lexicon layer decodes via the serde default to `true` and is *probed, not skipped* — the run
exercised direct lookup with the skip inactive on the whole deep part of the chain. It therefore
establishes correctness under the pessimistic condition and says nothing about the skip's value.

**Deferred to the next reseed** (a reseed is what stamps the lexicon layers). Two timed runs of the
demo settle it, against the figures the predecessor recorded — 0.75 s committing, **127 s** rejecting:

1. **legacy handles, skip inactive** — the upper bound; every ancestor probed on every lookup.
2. **after reseed, skip active** — expected fastest of the three designs, since it costs neither the
   five `is_a` probes per layer nor a per-process rebuild.

If (1) is already at or below the old figures the skip is belt-and-braces; if (1) regresses and (2)
recovers, the stamped bit is load-bearing and should be treated as such.

Note the demo does **not** stress the conservative serde default: a `false` default would have passed
it too, because the legacy lexicon layers genuinely hold no witnesses and every trace-bearing layer in
that run was freshly stamped. `handle_without_witness_flag_decodes_as_unknown` is what covers it.

*If repeated lookups on layers that **do** hold traces later prove too slow*, the next step is a
fixed-size bloom over witnessed target IRIs on the same handle — skipping per-IRI rather than per-layer,
still constant footprint, and reusing the per-layer bloom pattern that already exists.

**Slice 1 — symmetric witness normalization.** Emit side decodes `canonical_proposition` → `Exp`,
encodes, hashes — instead of hashing the stored JSON. *Gate:* a proposition stored in one form and cited
in a definitionally-equal other form resolves.

**Correction (2026-08-09).** An earlier draft gated this on "recompute the index over an existing
snapshot and diff keys" — i.e. on `encode(decode(stored)) == stored`. That is the wrong property:
neither necessary nor sufficient. What matters is only that **the two ends land on the same hash**, and
under D9 they do by construction. Reproducing the stored bytes is irrelevant, and in one case
impossible — see the `Lam` boundary below.

*Known boundary:* `Exp::Lam` carries no type slot, so decode discards a `Lam`'s domain
(`kernel/src/program/eigentt_type_mirror.rs:456`) and re-encoding a bare `Lam` is a hard error
(`EncodeError::LamWithoutAnnotation`, `:129`). A stored proposition containing one can never round-trip
— but `WitnessKey::from_exp` already routes through `encode_type`, so the **check** side cannot key such
a proposition today either. Slice 1 turns an asymmetric failure (emit succeeds, check fails) into a
symmetric one; nothing that resolves today stops resolving. Pinned by
`lam_bearing_propositions_cannot_round_trip_on_either_side`.

*Also:* record a lexicon-reseed timing before and after as a baseline — per D7 this does **not** gate
the slice; it exists so a later optimisation has a number.

**Status (2026-08-09): implemented and verified.**

The three emit sites (`emit_from_trace`, `emit_from_reasoning_sentence`,
`emit_from_institution_derivation`) now route through one helper that decodes against the layer and
hashes the resulting `Exp`. Two needed the layer threaded in. No evaluation on this side — per D9 the
decoded term is already the normal form. A decode failure logs through the operation table under
`kernel.layer.witness_decode`, naming the resource, so a miss caused by an undecodable proposition is
no longer indistinguishable from an absent witness (§4.2).

Verified:
- `cargo test --workspace` — 170 suites, **2747 passed, 0 failed**; clippy clean under `-D warnings`.
  Slice 1 is a no-op on everything currently on chain, which is the point.
- `crates/eigenius-reasoning/tests/witness_hash_agreement.rs` — the emit and check sides agree on the
  **definite description** `Fst(the(Σx. …))` that every parsed sentence contains, on its negated form
  `⟨parse⟩ → False`, and across binder renaming. The negated and un-negated forms hash **differently**,
  which is what makes the demo's one-word edit detectable. A fourth test asserts the comparison is not
  vacuous — `eval` + `readback` genuinely rewrites the term, so the agreement is doing work rather than
  comparing a term with itself.
- `demo/prose-to-formulas/run.sh --reparse` against `wordnet-umls-aligned-2026-08-03-specpoly` — both
  branches behave exactly as before: intact commits through the inference, edited refuses `bridge-s1`
  and `inference.esl` while `bridge-s2` still commits. Regeneration is byte-identical to the committed
  artifacts.

**Timing is still not the measurement D7 wants.** That run took 56.9 s wall for the *whole script* —
volume staging, kernel boot, two reparses, all loads — which is not comparable to the 0.75 s / 127 s
figures, since those measured a branch's loads alone. It rules the 127 s pathology out, and nothing
more: there is no timing from the pre-slice-0 run, and the skip is still inactive on that snapshot
(every lexicon layer decodes to "unknown"). The comparison remains deferred to the next reseed, per
slice 0's Status.

**Slice 2 — the `def` declaration and δ-at-decode.** A new declaration form (D5), *not* `Decl::Def` on
the `Let` token — `Let` stays reserved for the scoped type-position let (§1.2a). Three parts:

1. **Resource shape** (D8) — IRI, declared type, λ-body, opacity flag. Arity and parameter types are
   read off the type; a commit check rejects a recursive body.
1a. **Normalize the RHS at commit** (D9) — store the normal form, and reject a body that will not
   normalize. This is what lets every later use skip evaluation. Pin the condition it rests on:
   substituting normal closed arguments into a normal body yields a normal term.
2. **Capture-avoiding substitution on `Exp`** — new, and the one genuinely novel piece (§2.4). Total, no
   fail-soft. Property-test it against `eval`+`readback` on closed terms.
3. **Decode: peel and substitute** (§2.4) — one more arm on the head-aware `"App"` handling, alongside
   the `InductiveType` folding already there. `eval` is untouched and stays layer-free; `axiom` stays
   rigid.
4. **Opacity modes** (#95) — a branch condition at the head. `eigentt:definition_opaque`; absent means
   transparent. An opaque definition decodes to `EigonAxiom` — rigid, never unfolded — and its
   identity is the folded name rather than the normal form of its body (the D9 carve-out).

   **Correction (2026-08-09).** An earlier draft called this "not deferrable: once unfolding is a
   decode-time question about a specific definition, the mode must exist for decode to answer it."
   That is circular — the mode is needed only if opaque definitions exist; were every definition
   transparent, decode would unfold unconditionally and need no flag. Opacity **is** deferrable, and
   is carried here for #95 rather than required by this slice.

   **What distinguishes an opaque definition from an axiom, and what does not.** Today: nothing.
   Both decode to a rigid `EigonAxiom`, both carry a type and a name, and the opaque definition's
   body is inert. The distinction only becomes real with **step 1a's commit-time check**: an axiom is
   *asserted* — nothing inhabits it and the kernel takes it on trust — whereas an opaque definition
   has a body that was **type-checked against `definition_type` and then sealed**. That is Coq's
   `Qed`: downstream depends on the name, not on which inhabitant, but the inhabitant was verified.
   In a system built on epistemic grading that is not a small difference — unchecked assertion versus
   checked construction — but it is entirely carried by the check. **Until 1a lands, an opaque
   definition is an axiom with a decorative body**, and a body the kernel ignores is exactly the
   "documentation claiming more than the code does" pattern the claims audit names. Do not expose
   `opaque` in the ESL `def` surface before then.

*Gate:* a `def` whose body is a parse-shaped `Prop` compiles and commits; `HasActivity(m, g, a)` converts
with the parse; **the decoded term contains no `App(Lam, _)`**; a partial application decodes to a
β-normal `Lam`; an `opaque` def does **not** unfold; a recursive body is rejected at commit;
`eigenius decompile` still prints the folded form.

**Status (2026-08-10): implemented.**

| piece | where |
|---|---|
| capture-avoiding substitution | `kernel/src/nbe/subst.rs` (new) — exhaustive over the definition-body fragment, **refuses** anything outside it rather than passing it through unsubstituted |
| `Definition` class + `definition_type` / `definition_body` / `definition_opaque` | `ontologies/eigentt/eigentt-type-fragment.json` |
| decode: unfold transparent, peel-and-substitute; opaque stays rigid | `kernel/src/program/eigentt_type_mirror.rs` |
| Rule 24 — non-recursive, decodes, β-normal, inhabits its type | `kernel/src/validation/mod.rs` |
| `def` surface | `kernel/src/esl/{lexer,ast,parser,compile}.rs` |

Three things the implementation settled that the design had left loose:

- **Parameters use `TypedParam`, not `DataParam`.** `data`'s parameter kind is an `IndexKind` — a
  class or a sort only — so it cannot express `(P : T -> Prop)`. The `forall` production can.
- **No printer change was needed.** The printer emits generic `resource X : Class { … }` blocks, the
  same treatment `axiom` gets, and that round-trips. Verified rather than assumed
  (`a_definition_round_trips_through_the_printer`), because a definition that printed but did not
  reparse would break `eigenius decompile --verify` on the first chain holding one.
- **`opaque` is not accepted by the parser.** The property exists for #95, but until the body is
  type-checked at commit an opaque definition is an axiom with a decorative body. Rule 24 now does
  that check, so exposing it is a small follow-up rather than a design question.

**Rule 24's check order is load-bearing.** Recursion is checked **first, on the encoded form**, before
any decode. Decode unfolds a definition by substituting its body at the use site, so a body naming its
own IRI recurses *inside decode* — a guard placed after decoding never runs. A test caught this.

**`eigentt:definition_body` is exempt from Rule 21, and this is the argument.** Without the exemption
**no `def` can commit at all**: Rule 21 ends in `check_infer`, and a definition body is a lambda chain,
which has no inferable type — a lambda is *checked against* an expected type, never inferred from
itself. Every well-formed definition was rejected with `cannot infer type of: Lam(…)`. The exemption is
sound on three legs:

1. **Rule 21 contributed nothing here.** For a lambda it produced only a spurious rejection, so
   exempting loses no coverage.
2. **Rule 24 checks the body in the correct mode** — against the declared `definition_type`, strictly
   stronger than inference. And `definition_type` is **not** exempt: it still passes through Rule 21,
   so the type the body is checked against is itself validated.
3. **The exemption is keyed on the property IRI, not the class**, which would be an escape hatch if
   `definition_body` could ride on a non-`Definition` resource — Rule 24 would not run and Rule 21
   would be exempt. It cannot: **Rule 10 (domain) is restrictive** and `definition_body`'s
   `core:domain` is `[eigentt:Definition]`. Pinned by
   `definition_body_cannot_escape_checking_by_riding_on_another_class`, which smuggles an ill-typed
   body onto a `DeclaredResource` and asserts the refusal. If that domain is ever relaxed, or Rule 10
   made advisory, the test fails and names the reason.

This was found only because a test exercised the **commit gate** rather than the mechanism. Every
earlier slice-2 test built layers directly and passed; the feature would have failed on first real
use. Worth carrying into slice 3, where the demo rewrite goes through the real load path.

**Slice 3 — the capstone: rewrite the demo.** Not a cleanup pass — this is the acceptance test for the
whole design, and it is where **D6** is answered (arity and names, chosen with the `def` form in hand
rather than guessed against one that does not exist yet).

Rewrite `demo/prose-to-formulas/onco-typed.esl` as definitions with the context parameter explicit,
retiring the `urn:eigenius:demo:onco-typed` namespace as far as the parser's lexicon allows; rewrite
`literature-rules.esl` with the `∀ m`; delete `rules.esl` and the `--rules-out` / `--citations-out`
generation path.

*Gate:* `run.sh` still shows the intact branch committing, the edited branch's measurement lift refused,
and the inference dying with it — with **one** Declared resource on the branch, down from ≥62. The
negation-visibility property (`→ False` distinguishing the edited sentence) must survive unchanged; it
is the demo's whole point and the definitions must preserve it.

**Status (2026-08-10): implemented, run, and verified — first contact with the real lexicon.**

The slice was code-complete on 2026-08-09 but unrunnable until a reseed: slice 2 grew a **bootstrap**
ontology (`eigentt-type-fragment.json` gained `Definition` + its three properties), and the seed
manifest is content-verified, so every pre-D66 snapshot fails the drift check. An attempt to add the
four resources as an additive layer (`definition-layer.esl` + a patched snapshot) could not work —
the drift check fires on the seed manifest before any layer loads — and those artifacts are deleted.

Environment: `reseed-lexicon-db.sh --snapshot-dir wordnet-umls-d66` — **11 m 41 s** wall on a cold
machine (includes a from-scratch kernel-image build; this is the D7 baseline number). WordNet 4
layers / ~648 k resources + UMLS 11 layers / ~2.65 M resources, all clean.
`build-alignment-snapshot.sh` over it: 38 389 merges, **3 m 07 s** → `wordnet-umls-aligned-d66`.

Verified, committed fixtures (gate items):

- Intact branch commits every layer and the inference; the conclusion is justified twice. Edited
  branch: claims commit, `inference.esl` refused with `qc_validate_justification` → `Fails` — the
  negation-visibility property survived the move to definitions.
- **One** DeclarationTrace on the branch (the literature rule), down from ≥62 Declared.
- **Rule 24 against the real WordNet/UMLS resources** (`wn:v02203362_t`, `wn:n13440063`, …, not the
  stand-ins): `onco-typed.esl`'s two definitions commit, 673 ms.
- **`spec_poly` at the Σ-term** for «MSI cancer models» type-checks on the real chain (§9's
  universe oddity remains undiagnosed but does not bite here either).

Verified, `--reparse` against the **aligned** snapshot: both variants parse each sentence to exactly
1 closed reading, and the regenerated `claims-intact.esl` / `claims-edited.esl` are **byte-identical**
to the committed fixtures. Against the **raw** (unaligned) snapshot `--reparse` fails closed:
sentence 1 yields 2 readings sharing the pinned skeleton — WordNet/UMLS duplicate senses are
unmerged, so the recorded ranks replay (keyed on candidate senses) misses and falls back to
cap-only, and `select_pinned` refuses. The demo's prerequisite is the aligned chain, as before D66;
the fixture path is indifferent (no parsing).

**The slice-0 deferred measurement**, from the kernel's own `commit.pipeline_run` `duration_ms`
against the freshly stamped snapshot: intact `inference.esl` commit **343 ms** (pre-slice-0:
0.75 s); edited `inference.esl` refusal **366 ms** (pre-slice-0: **127 s**). Rejection now costs
the same as commit — the 127 s materialised-index pathology is gone. Run (1) of the protocol
(current code, legacy unstamped handles) is no longer obtainable: pre-D66 snapshots fail the drift
check and any current reseed stamps the bit; the nearest legacy datapoint stays slice 1's 56.9 s
whole-script run. Whether the stamped skip specifically, as opposed to direct lookup alone, is
load-bearing was therefore not isolated.

Found by the run (slice 2's mechanism-vs-gate lesson, again):

- `narrate()` still referenced `$SPLIT` — the bridge-splitting scratch dir whose definition was
  deleted with the bridges — so under `set -u` the first narration killed the script. A pipe on the
  invoking side masked the non-zero exit. Restored as `SCRATCH`, now only narrate's compile cache.
- `run.sh`'s default snapshot pointed at the dead pre-D66 `…-2026-08-03-specpoly`; now
  `…-aligned-d66`.
- The edit-direction narration in `run.sh` and `inference.esl` said a negation was *deleted from
  sentence 2*; the fixtures *insert* one into sentence 1 («had» → «did not have»). Corrected.

D6 is recorded in §6 and `onco-typed.esl`. The ESL guides gained `def` (03 keyword table,
04 §4.4c with Rule 24 and the opacity boundary, 11 EBNF + keyword reference).

*(Opacity control, #95, was a fourth slice in an earlier draft. It is folded into slice 2: once δ is a
decode-time question about a specific definition, decode cannot answer it without the mode, so it
cannot be deferred.)*

## 6. Decisions

| # | Decision | Status |
|---|---|---|
| **D1** | The parse→domain lift is **definitional equality**, not a Declared implication | ✅ settled — §2.1; a Declared bridge is forced only by the opaque declaration form |
| **D2** | Domain predicates carry the **context parameter explicitly**; the general claim is `∀`-quantified and eliminated with the existing `spec_poly` | ✅ settled — §2.2; removes a silent universal quantification, adds no new machinery |
| **D3** | Normalization is fixed at the **emit side of the witness index**, not inside `hash_proposition_value` | ✅ settled — §4.1; keeps `prop_hash` layer-independent |
| **D4** | Does α-canonicalization survive as mechanism, or degrade to a leveller? | ✅ settled — **it stays load-bearing; keep it.** Under D5 only one side reads back: the check side's `readback_val` freshens binder names to `G#n`, while decode carries the author's name straight through as `Patt::Var(name)` (`kernel/src/program/eigentt_type_mirror.rs:431-439` for `Pi`, `:443-451` for `Sig`). The names therefore always differ and α-canonicalization is what makes the two hashes meet. This holds under either D8 branch — option (b) leaves the emit side with no readback at all, option (a) adds one at a level that need not match `ctx.rho.len()`. An earlier draft had the emit side *evaluating*, which is where the "both sides read back, so maybe it is redundant" doubt came from; §4.1 no longer says that |
| **D5** | A definition is a **separate declaration form** (`def`), not `Decl::Def` on the `Let` token; unfolding happens **at decode**, not at eval | ✅ settled — §1.2a. A chain-resident definition is a *third binder*: `Let` is local and `Rho`-resolved and cannot mint an IRI; `EigonAxiom` mints an IRI but evaluates to a rigid neutral (`kernel/src/nbe/eval/mod.rs:509`), correctly so for `kind_of`/`the`. `eval` has no layer (`:155`), so δ belongs in decode, which already resolves `ConstRef` against the layer — and #95 independently frames δ-control as decode modes. `Let` stays reserved for the scoped type-position let |
| **D6** | Arity and naming of the domain predicates once the context is explicit | ✅ settled — **slice 3 (2026-08-10): ternary with the model explicit, and the invented names stay.** Arity: `HasActivity(m, g, a) : Set -> Set -> Set -> Prop`, the model a `Set` parameter carrying the nested compound-kind term the sentences are about (§2.2). Naming: `HasActivity` / `RequiresActivity` remain in `urn:eigenius:demo:onco-typed`. Under `def` a name is a transparent abbreviation — the body is the meaning and the kernel computes it — so the name asserts nothing and an invented one is no longer an unchecked leap. Borrowing **RO:0002215 `capable of`**'s name was rejected: that relation is *binary*, gene-to-process, and context-free, so the explicit-model ternary form contradicts the identity the name would claim. Grounding stays a lexicon change (§7): when RO/GO enter the parser's lexicon, the move is to `def` RO-named predicates over those entries, not to rename these. Recorded in `onco-typed.esl` |
| **D7** | Cost of decoding on the commit path | ✅ settled — **absorb it**. Every `canonical_proposition` gains a D47 decode at layer build. Correctness comes first; the alternative is two normalization paths kept in step by hand, which is the defect being fixed. Efficiency is follow-up work, taken only if measurement warrants it — see below |
| **D10** | Is a definition's body stored δ-**folded** or δ-**expanded**? | ✅ settled — **folded**. Both satisfy D9: a use decodes to the fully unfolded term before anything hashes it, so identity is the normal form either way. Only storage differs. Measured on two nested definitions in the demo's own shape (`ActivityOf` referenced once by `HasActivity`): folded **17 nodes / 554 bytes**, expanded **32 nodes / 1104 bytes** — roughly double at one level, and a body referencing another twice inlines it twice. Expanded storage also has a sharper edge: `encode_type` refuses a bare `Exp::Lam` (`LamWithoutAnnotation`), because decode discards a `Lam`'s domain, so anything reading a stored body and writing it back must carry the parameter types separately. **Corrected 2026-08-10:** an earlier draft justified folded storage by `eigenius decompile` printing the folded call. That readability requirement was never set and is not a basis for this decision; it is at most an observation about the printer |
| **D9** | What is a definition's **identity** for equality and hashing? | ✅ settled — **the normal form of its right-hand side**. *(Refined 2026-08-10 against the implementation: the earlier wording said "normalized once at commit and stored that way", which is not what the kernel does and not what it should do. **Nothing normalizes.** β-normality is enforced at commit by **rejection** — Rule 24 refuses a redex-bearing body rather than rewriting it, because a compiler silently rewriting an author's body is worse than telling them it has a redex. **δ is performed at decode**, recursively, so a body may reference another definition and keep it FOLDED in storage. The storage half of that split is D10. Both ends of the witness key still agree, because both go through the same decode — pinned by `nested_definitions_unfold_all_the_way_at_decode`.)* Anything computing a `prop_hash` therefore hashes the normal form, and the two ends of the witness key agree *by construction* rather than by an argument that decode-only happens to coincide with decode-plus-eval. Normalizing at commit rather than per use matters because slice 0 moved emission to **per lookup**: normalizing at each use would put a full NbE evaluation on every witness probe on every layer of a chain walk, far beyond the "a D47 decode at layer build" D7 accepted. With the RHS already normal, D8's peel-and-substitute drops closed arguments into a normal body without forming a redex, so a use decodes straight to a normal term and neither side evaluates. **Carve-out:** this defines identity for *transparent* definitions; an opaque one does not unfold, so its identity stays the folded name (#95). **Condition to pin, not assume:** substituting normal closed arguments into a normal body yields a normal term |
| **D8** | Does decode form a redex or substitute through — and what is stored? | ✅ settled — **store the λ-body; decode peels and substitutes; definitions are non-recursive.** §2.4. The real axis is decode behaviour, not storage: forming `App(Lam…, x)` would force the emit side to replicate the evaluator, which is §4's defect relocated from α to β. Peel-and-substitute is bounded and structural, so D5 and D4 both hold. Storage is the λ-body — arity and parameter types come from the declared type, so nothing is duplicated and no new consistency rule is needed; and it avoids a second binding convention in `TypeExpr` that `alpha_canonicalize_proposition_json` would mis-handle, since that function deliberately preserves free `Var`s. Decode distinguishes a definition by the resolved resource's class, as `resolve_const_ref` already does for axiom / class / individual; **no new `Exp` variant** — after substitution the definition leaves no trace. Opacity (#95) hangs off the same resource and is a branch condition at the head. Requires a total capture-avoiding substitution on `Exp`, which does not yet exist (§2.4) |

## 7. Out of scope

- **Rules that quantify over parse shapes.** `⟦_⟧`/`match` over `eigentt:TypeExpr` — issue #112. D66
  covers rules over a *fixed* shape, which is what `literature-rules.esl` is. The two are independent;
  #112's stated ordering dependency on #111 does not survive §1.4.
- **Reducing the definition count.** D66 moves 61 Declared bridges to 61 definitions and 1 Declared
  rule. Collapsing the 61 definitions needs shape quantification (#112) and is not attempted here.
- **Grounding the domain vocabulary** in GO/RO. Capped by the parser's lexicon (WordNet + UMLS), so it
  is a lexicon change, not a bridge change — as `onco-typed.esl` already records.
- **Optimising the decode cost** (D7). Deferred deliberately, not overlooked. Record a before/after on a
  lexicon reseed when slice 1 lands — as a **baseline, not a gate** — so a later optimisation has a
  number to work against. The lever is already identified and does not require revisiting this design:
  index population is **per-resource independent work on a single thread**, and claims-audit E4 measures
  ingest at 1.05 of 22 cores with no `rayon` anywhere in `kernel/`, `crates/`, or `storage/`. If the
  measurement warrants it, parallelising index population recovers far more than this change costs.
  Two adjacent items compound and are worth folding into the same pass: `Rule 21` already decodes two
  `eigentt:TypeExpr`-ranged properties per lexical entry (claims-audit B8), and RocksDB is untuned with
  Bloom filters off (E5).

## 8. Source anchors (verified against the tree)

| Claim | Anchor |
|---|---|
| Shape rule is a Declared resource, one per (predicate, shape) | `crates/eigenius-reasoning/src/grade.rs:546`; key at `crates/eigenius-encoding/src/emit.rs:350` |
| 62 sentences → 61 distinct sense-erased skeletons | `experiments/parsing/skeleton-abstraction.py` over `expected-readings.tsv` |
| Skeletons erase every open-class sense | `kernel/src/dcg/skeleton.rs:53` (`erase_senses`, ≥4-digit token → `§`) |
| Zero-ctor inductive is opaque | `ontologies/reasoning/reasoning.esl:52` |
| No implication introduction | `ontologies/reasoning/reasoning.esl:26-31`, ctors at `:97-175` |
| `Decl::Def` never emitted from ESL | only `kernel/src/program/expr.rs:358` |
| `Let` reserved for type-position δ-binding | `kernel/src/esl/lexer.rs:48-54` |
| Lookup side normalizes | `kernel/src/program/check_hooks.rs:76` |
| Emit side does not | `kernel/src/layer/witness_index.rs:206,223,249` |
| α-canonicalization is a targeted patch | `kernel/src/witness/mod.rs:130-136,181` |
| Index errors discarded | `kernel/src/layer/mod.rs:1165,1176` |
| `spec_poly` applied at `Set` | `demo/prose-to-formulas/inference.esl` (slice 3; previously the generated `bridges.esl`) |
| Decode preserves author binder names | `kernel/src/program/eigentt_type_mirror.rs:431-439` (`Pi`), `:443-451` (`Sig`) |
| `Lam` is `(name, dom, body)`; dom validated then dropped | `kernel/src/program/eigentt_type_mirror.rs:453-465` |
| Decode's `"App"` arm is already head-aware and folds args | `kernel/src/program/eigentt_type_mirror.rs:474-482` |
| `resolve_const_ref` discriminates by resolved class | `kernel/src/program/eigentt_type_mirror.rs:141-149` |
| D64 resolves anaphora by application + `eval`/`readback` | `kernel/src/dcg/parse/resolve.rs` (`resolve_open`) |
| Holes are λ-bound free variables, span-keyed | `kernel/src/dcg/holes.rs:33-44` |
| The only `subst` in the tree is deliberately partial | `kernel/src/dcg/rules/combinators.rs:1592` |
| Committed parses are β-normal | 0 `Lam`, 0 `App(Lam, _)` across 76 nodes in `demo/prose-to-formulas/claims-intact.esl` |
| Class→entity routes | `ontologies/ontology/ontology.esl:41,52`; verb arrow `crates/eigenius-wordnet/src/convert.rs:210` |
| Consequent drops the model | `rule_1` antecedent arg2 holds `umlscui:C0920269`; consequent holds only `v0`,`v1` |

## 9. Unresolved observation

`spec_poly` binds `T : Set` and is applied at `T := Set` (`demo/prose-to-formulas/inference.esl`).
The kernel has
`Sort(n) : Sort(n+1)` (`kernel/src/nbe/check/mod.rs:616,1136`) and cumulativity `Sort(m) <: Sort(n)`
iff `m ≤ n` (`kernel/src/nbe/check/conv.rs:292`), which does not obviously admit `Set : Set`. The demo
commits and loads, so either the universe rule is being read wrongly here or something is lenient at
that site. **Not diagnosed.** D66 §2.2 reuses this instantiation, so it inherits whatever the answer is;
worth settling independently of this document.
