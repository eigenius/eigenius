# D66 — Definitional lifting: transparent definitions, explicit context, and symmetric witness normalization

*Status: design — **decision-complete** (§6 D1–D8 all ✅); ready to implement (§5 slices). No code yet. Motivated by the shape-rule
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
the mechanism `demo/prose-to-formulas/bridges.esl:44` already uses.

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
| **emit** (layer build) | `kernel/src/layer/witness_index.rs:206,223,249` — `hash_proposition_value(encoded_prop)` on the stored `Value::Json` | **no** — hashes what the author typed |

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

`prop_hash` stays a hash of a *term*, and there is one normalization path instead of two kept in step
by hand. **The stored form stays folded**, so `eigenius decompile` still prints `HasActivity(m, g, a)`
and the readability the definition buys is not lost at the printer.

This is a smaller change than "evaluate in the layer": the emit side must decode anyway to obtain an
`Exp`, and decode is where the layer already is.

### 4.2 This makes a latent fail-open live

`hash_proposition_value` is infallible today, so witness indexing cannot fail. Decoding can. And index
population errors are **discarded** — `let _ = …` at `kernel/src/layer/mod.rs:1165` and
`kernel/src/layer/mod.rs:1176` (item A2 of
[`docs/notes/2026-08-08-claims-audit-followups.md`](../notes/2026-08-08-claims-audit-followups.md)).
A proposition that fails to decode would
silently not be indexed, its witness would silently not resolve, and the citing sentence would fail with
no signal naming the cause.

**A2 is a prerequisite of this work, not adjacent hygiene.**

## 5. Implementation plan (slices)

Ordering is forced: 0 before 1 before 2, or the first committed definition produces witnesses that do
not resolve.

**Slice 0 — fail closed on index population.** Replace the discarded `Result`s at `kernel/src/layer/mod.rs:1165,1176`
with a commit-failing (or at minimum logged, via the `kernel.commit.*` operation table) error path.
Claims-audit A2. *Gate:* an induced index-population failure fails the commit with a diagnostic.

**Slice 1 — symmetric witness normalization.** Emit side decodes `canonical_proposition` → `Exp`,
encodes, hashes — instead of hashing the stored JSON. *Gate:* a proposition stored in one form and cited
in a definitionally-equal other form resolves. *Also:* recompute the index over an existing snapshot and
diff keys against the current scheme — expected to be a no-op, since no chain today has definitions and
binder-name differences are already α-canonicalized. **Verify, do not assume.** *And:* record a
lexicon-reseed timing before and after as a baseline — per D7 this does **not** gate the slice; it
exists so a later optimisation has a number.

**Slice 2 — the `def` declaration and δ-at-decode.** A new declaration form (D5), *not* `Decl::Def` on
the `Let` token — `Let` stays reserved for the scoped type-position let (§1.2a). Three parts:

1. **Resource shape** (D8) — IRI, declared type, λ-body, opacity flag. Arity and parameter types are
   read off the type; a commit check rejects a recursive body.
2. **Capture-avoiding substitution on `Exp`** — new, and the one genuinely novel piece (§2.4). Total, no
   fail-soft. Property-test it against `eval`+`readback` on closed terms.
3. **Decode: peel and substitute** (§2.4) — one more arm on the head-aware `"App"` handling, alongside
   the `InductiveType` folding already there. `eval` is untouched and stays layer-free; `axiom` stays
   rigid.
4. **Opacity modes** (#95) — a branch condition at the head. **Not deferrable**: once unfolding is a
   decode-time question about a specific definition, the mode must exist for decode to answer it.

*Gate:* a `def` whose body is a parse-shaped `Prop` compiles and commits; `HasActivity(m, g, a)` converts
with the parse; **the decoded term contains no `App(Lam, _)`**; a partial application decodes to a
β-normal `Lam`; an `opaque` def does **not** unfold; a recursive body is rejected at commit;
`eigenius decompile` still prints the folded form.

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
| **D6** | Arity and naming of the domain predicates once the context is explicit | ✅ settled **as sequencing** — the question is deliberately *not* answered here. `demo/prose-to-formulas` is rewritten as the **capstone** (slice 3) once slices 0–2 are implemented, and the arity and names are chosen there with the mechanism in hand. Answering it now would fix a vocabulary against a `def` form that does not yet exist. Note `demo/prose-to-formulas/onco-typed.esl` already records that `HasActivity` duplicates **RO:0002215 `capable of`**, which is *binary*, gene-to-process, and carries the same context-free assumption — so the honest ternary form will not map onto it cleanly, and the capstone settles **arity and naming, not grounding** (§7) |
| **D7** | Cost of decoding on the commit path | ✅ settled — **absorb it**. Every `canonical_proposition` gains a D47 decode at layer build. Correctness comes first; the alternative is two normalization paths kept in step by hand, which is the defect being fixed. Efficiency is follow-up work, taken only if measurement warrants it — see below |
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
| `spec_poly` already applied at `Set` | `demo/prose-to-formulas/bridges.esl:44,74,194` |
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

`spec_poly` binds `T : Set` and is applied at `T := Set` (`demo/prose-to-formulas/bridges.esl:44`).
The kernel has
`Sort(n) : Sort(n+1)` (`kernel/src/nbe/check/mod.rs:616,1136`) and cumulativity `Sort(m) <: Sort(n)`
iff `m ≤ n` (`kernel/src/nbe/check/conv.rs:292`), which does not obviously admit `Set : Set`. The demo
commits and loads, so either the universe rule is being read wrongly here or something is lenient at
that site. **Not diagnosed.** D66 §2.2 reuses this instantiation, so it inherits whatever the answer is;
worth settling independently of this document.
