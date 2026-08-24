# P2 · N4 — Should the EigenTT term representation live in core?

Written `2026-08-23` from `c271213`. Arose while implementing
[#188](https://github.com/eigenius/eigenius/issues/188) but is **not part of it** — universe
polymorphism needs none of this.

**Answer: yes, and nothing blocks it.** The bootstrap-cycle objection does not hold for the decoder
(hard-coded, not schema-driven) and does not hold for the validator either — §4 settles both. The
move additionally closes a **fail-open in Rule 16** that the P2 audit missed, §4a.

## 1. The symptom, twice

`core` owns the inductive metamodel — `InductiveType`, `InductiveCtor`, `InductiveArgType`,
`InductiveParam`, and the properties `param_kind`, `type_name`, `result_sort`, `indices`,
`type_params`. It does **not** own the term language those slots need to describe a type. So every
type-valued slot in core degrades to a string, and each has decoded lossily:

| slot | encoding | consequence |
|---|---|---|
| `core:result_sort` | was `"Prop"` / `"Set"` / `"Type:N"` | could not express a level variable — **fixed** by #188 slice 5b, now a `core:Level` value |
| `core:param_kind` | IRI \| bare param name \| sort keyword | a class-typed parameter falls through `decode_param_kind_str` to `Sort(1)` and is **silently typed `Set`**, which accepts anything |
| `core:type_name` (on `InductiveArgType`) | IRI \| bare param name | handled correctly — `decode_arg_type` has the `EigonClass` arm `param_kind` lacks |

Two of the three were defective, in the same way, for the same reason. `decode_arg_type`'s doc
records the asymmetry without naming its cause: *"Any other class IRI: emitted as
`Exp::EigonClass(iri)`"* — the arm the parameter path never got. #199's comment names it too, from
the other side.

**The `param_kind` `EigonClass` arm is a live bug and should be fixed regardless of this note.** It
needs no layering change and no ontology edit.

## 2. What the layer actually contains

`eigentt-type-fragment` is not one thing. Its dependency edges run one way:

| resource | kind | depends on |
|---|---|---|
| `eigentt:TypeExpr` | `core:InductiveType` | — |
| `eigentt:Axiom` | `core:Class` | — |
| `eigentt:axiom_statement` | property | `class_types [eigentt:TypeExpr]` |
| `eigentt:axiom_justification` | property | — |
| `eigentt:Definition` | `core:Class` | — |
| `eigentt:definition_type` | property | `class_types [eigentt:TypeExpr]` |
| `eigentt:definition_body` | property | `class_types [eigentt:TypeExpr]` |
| `eigentt:definition_opaque` | property | — |

So the layer bundles **the term language** with **resources that carry terms**. `Axiom` and
`Definition` are consumers of `TypeExpr`; nothing depends the other way. Moving `TypeExpr` down does
not break a cohesive layer — it separates two strata that were bundled.

That also disposes of the one argument this note started with against the move. There is no cohesion
to protect.

## 3. Why `core` is already the type-theory layer

The objection "core should stay a minimal, type-theory-free metamodel" describes a `core` that does
not exist:

- it owns the entire inductive **declaration** metamodel (§1);
- it **declares inductives** — `core:Asserts`, `core:Option`;
- it now carries a **universe level algebra**, `core:Level`, because `result_sort` needed one and a
  lower layer cannot reference a higher one (#188 slice 5b);
- D47 §211 already reasons about what must be in core *and in what order* — *"the core ontology needs
  the following declarations prior to committing the axiom Resources (in the same core-ontology
  layer, ordered before the axioms)"*.

No recorded rationale places `TypeExpr` above `core`. On the evidence the boundary is where it is by
accident of when D47 was written, not by design.

## 4. The bootstrap-cycle question, answered

The objection worth taking seriously: `eigentt:TypeExpr` is itself declared **as a
`core:InductiveType`**, its constructor arguments described by `core:type_name`. Retype `type_name`
to carry `TypeExpr` values — which is the reason to move it — and TypeExpr's own declaration
describes itself in its own language. Does that cycle?

**For the decoder, no.** `decode_type_json` (`program/eigentt_type_mirror.rs:496`) dispatches on
hard-coded ctor-name strings — `match ctor { "Sort" => …, "Var" => …, … }`. It never reads
`TypeExpr`'s chain declaration. Decoding a `TypeExpr` value therefore does not require having
decoded `TypeExpr`'s schema, and the self-reference is inert at decode time.

The recursion the design *does* worry about is already solved, and by a mechanism that generalises:
`decode_arg_type` resolves a cross-inductive reference to a **stub** rather than the full decl,
*"which would risk infinite recursion for mutually-referential inductives"*. A `TypeExpr`-valued
`type_name` would resolve through the same stub path.

**For the validator, also no.** Rule 16 (`validation/rules/inductive.rs:245`) *is* schema-driven —
it reads the target `InductiveType`'s `ctors` array to check a value's ctor name and arity. But its
recursion is structural on the **value tree**, never on the schema: `check_inductive_value` reads
`ctors` (a resource lookup, not a validation), matches the ctor, zips `args` against the declared
`arg_types`, and recurses via `check_inductive_arg` on **each argument value**, with
`child_path = "{path}.args[{i}]"`. Every step consumes one node of a finite JSON value. The schema
resource is re-read at each level and never descended into as data.

So validating `TypeExpr`'s own declaration would read `TypeExpr`'s `ctors` while checking a value
that happens to live inside `TypeExpr`'s `ctors` — a bounded read, not a loop. Depth is bounded by
the value's own nesting. **The question this note opened is closed: there is no cycle.**

## 4a. What the retype would close: Rule 16 fails open on parameter-typed arguments

`check_inductive_arg` reads `type_name` as a **string** (`.and_then(Value::as_str).unwrap_or("")`)
and, when it does not parse as an IRI, **returns `Ok` — admitting the value unchecked**:

```rust
let type_iri = match Iri::parse(type_name) {
    Ok(i) => i,
    Err(_) => return, // Bare parameter name; deferred per v1.
};
```

A bare `type_name` is a **type-parameter reference**, so every parameter-typed constructor argument
of every parametric inductive is admitted without validation. The declarations exist:
`core:Option.some(A)`, `logic:And.conj(P, Q)`, `logic:Or.inl/inr`.

**Latent, not live — but nearer than the comment suggests.** Measured `2026-08-23`: no `conj`,
`inl`, `inr` or `Some` values on any committed chain, so nothing is currently mis-validated. The
deferral's stated premise, *"v1 callers use only monomorphic inductives"*, is half stale: the
declarations are parametric already, and `closed-class.esl:858` gives English *"but"* the semantics
`λs₂:Prop. λs₁:Prop. logic:And(s₁, s₂)`. **Any parsed sentence containing "but" produces an `And`
value**, whose two arguments are typed `P` and `Q`. It is one prose encoding away, not hypothetical.

This is the shape P2 spent its length fixing — the commit gate accepting what it cannot check, with
no diagnostic. #92 was declarations never reaching the positivity pass; #194 was check mode
discarding the expected universe; this is Rule 16 discarding the argument type. Each reads as if it
checks and does not. The audit did not look at Rule 16.

**The retype subsumes it.** A `TypeExpr`-valued `type_name` makes a parameter reference an
`Exp::Var` — a decodable, checkable shape — instead of a string that fails to parse as an IRI. The
fail-open exists *because* the slot is stringly typed, which is this note's whole thesis in one call
site.

Fixing it independently is still worth doing, and is smaller: make the unparseable case produce a
diagnostic rather than silent admission, and correct the comment to say what is actually true.

## 5. If it goes ahead

### The ontology

- **Move the declaration, keep the IRI.** `urn:eigenius:eigentt:TypeExpr` declared in
  `core-ontology.json` is legal — the namespace prefix is a naming convention, not a layer
  assertion. That leaves **all 22 chain references and 59 Rust references untouched**. Renaming to
  `core:TypeExpr` would change 81 references and break a widely-referenced IRI for cosmetics.
- **`TypeExpr` references nothing outside `core` except itself.** Checked: its only non-`core:*`
  reference is `urn:eigenius:eigentt:TypeExpr`, its own self-reference. `Sort` already points at
  `core:Level`, the literals at `core:string` / `core:integer` / `core:float` / `core:boolean`. So
  the move introduces no upward reference — it is a relocation, not a rewiring.
- **Order it after `core:Level`** inside `core-ontology.json`. D47 §211 already establishes that
  core declarations are order-sensitive.
- `Axiom` / `Definition` and their four properties stay where they are. They are the higher stratum
  (§2), and their `class_types [eigentt:TypeExpr]` keeps resolving — a higher layer referencing a
  lower one.
- Retype `param_kind` and `type_name` to `data_type core:resource` / `class_types [TypeExpr]`, the
  pattern `reflection:canonical_proposition` and now `core:result_sort` both use.
- **`TypeExpr` needs a `SizeSort` ctor** — `param_kind` accepts `Size`, and `TypeExpr`'s 19
  constructors have no representation for it.

### The code — the site count was wrong

The estimate below was **six sites, three producers and three consumers**. The build found
**fifteen**, and the four it missed are the interesting ones, because each was a *silent* reader:
code that read `type_name` or `param_kind` as a `Value::String`, got `None`, and carried on.

| site | role | found by |
|---|---|---|
| `esl/compile.rs` — codata params, data params, data indices | producer | in the estimate |
| `esl/compile.rs` — `compile_ctor_binder`, `compile_ctor_arg_type`, the `Option` site | producer | in the estimate |
| `program/ground.rs` — `decode_param_kind` | consumer | in the estimate |
| `program/ground.rs` — `decode_arg_type` | consumer | in the estimate |
| `validation/rules/inductive.rs` — `check_inductive_arg` (Rule 16) | consumer | in the estimate |
| `esl/print.rs` — the printer | consumer | in the estimate |
| **`program/ground.rs` — `decode_indices`** | consumer | bootstrap |
| **`program/ground.rs` — `decode_codata_observation_type`** | consumer | test suite |
| **`program/expr.rs` — `decode_program_type`** | consumer | test suite |
| **`validation/mod.rs` — `is_option_of_class`** | consumer | test suite |
| **`crates/eigenius-julia/src/mirror_gen.rs` ×2** | consumer | grep, after the suite went green |

The Julia pair is the one worth pausing on: **its tests passed the whole time.** They build the
`InductiveArgType` fixture by hand, so they kept writing strings and kept reading them. A green
suite said nothing about whether the generator worked on a real chain declaration, and would not
have said anything until someone ran it. Hand-built fixtures for a property whose encoding is
changing are not evidence.

**Three head-readers had already been written by the time the suite was green** — one in
`decode_arg_type`, one in Rule 16 (`type_name_head`), one about to be written for the codata path.
That is N1 §3's shape exactly, so the reader is now a single `program::ground::arg_type_head`, and
all five consumers call it.

### Four defects the retype surfaced, none of them anticipated here

1. **Every index kind on every chain was `EigonClass(core:Set)`.** `decode_indices` read
   `core:param_kind` as `Some(Value::String(s))` with `_ => "urn:eigenius:core:Set"` as the
   fallback. `urn:eigenius:core:Set` is not a declared resource — it decoded to a class type nothing
   can inhabit. Once the producer emitted a `TypeExpr`, the fallback fired for *every* index, and
   `reasoning:JustifiedBy.declared` failed with `EigonPrimitive(String) ≠ EigonClass(core:Set)`.
   Before the retype, the same fallback fired for every index whose `param_kind` was absent, silently.
2. **`check_type`'s fallback was `check(a, &Val::sort(1))`** — "is a type" spelled as "inhabits
   `Set`". `reasoning:JustifiedBy.spec_poly` binds `T : Type 1` and then writes `P : T -> Prop`;
   checking `T` against `Set` fails `Sort(2) </: Sort(1)`. Now `ensure_sort(infer(a))`
   (`references/nanoda_lib/src/tc.rs:244`). Same defect as the `Level` `Ord` derive removed earlier
   in #188: a universe comparison written as a constant.
3. **The D47 codec had no size form at all.** Not `SizeSort`, `SizedPi`, `SizeInf` or `SizeSucc`. A
   `Size` kind used to travel as the literal string `"Size"` and never entered the codec. `SizeSort`
   is now encoded and decoded; the other three still are not, and nothing on any chain carries one
   (N2 §3).
4. **The three telescope producers had already drifted.** The codata-param site called `var_value`
   where the data-param and index sites called `bare_kind_value`, so a `Size`-kinded codata
   parameter lowered to `Var("Size")` — a reference to a binder that does not exist. One
   `Compiler::lower_kind` now serves all three.

`core:Set` and `core:Size` were being written in ESL fixtures as though they were resources. Neither
is declared on any chain; `wk::SET_KIND` and `wk::SIZE_KIND` were unreferenced residue of the string
encoding and are deleted. The sorts are spelled `Set` and `Size`.

### What it buys

- `data Vec (A : Sort u)` becomes expressible. This is the loose end #188 left: `core:result_sort`
  was retyped in slice 5b so `data X : Sort u` works, while `param_kind` stayed a string, so a
  **parameter** still cannot carry a level. TTR record types parameterised over an arbitrary
  universe want exactly that shape (N3 §5a).
- A class-typed parameter stops being silently typed `Set` — `Exp::EigonClass(iri)` is just what
  the decoded `ConstRef` yields, with no special arm needed.
- Parameter-typed constructor arguments become checkable: a parameter reference decodes to
  `Exp::Var`, so Rule 16 has a shape to check instead of a string it cannot parse (§4a).
- Both string unions disappear.

### Migrating the hand-authored values — 89 of them, by script

ESL-declared inductives re-encode at bootstrap, so they are free. Hand-authored JSON is not:

| file | `type_name` / `param_kind` strings |
|---|---|
| `ontologies/lean/lean-expressions.eigon.json` | 36 |
| `ontologies/eigentt/eigentt-type-fragment.json` | 29 |
| `ontologies/core/core-ontology.json` | 13 |
| `ontologies/formulas/formulas-ontology.json` | 11 |
| **total** | **89** |

**The retype cannot be scoped to `core` + `eigentt`.** `core:type_name` is one property with one
`data_type`; changing it changes its contract for every `InductiveArgType` on every chain. So the
Lean mirror's 36 are in scope whether or not D30 is otherwise touched, and `lean` and `formulas`
layers move along with `core` and `eigentt` — four layers, not two.

**Do it with a one-shot script, not by hand and not via the compiler.** Decompile-then-recompile is
the obvious mechanism and does not work: `eigenius decompile` flattens a `data` declaration into a
generic `resource` block, so recompiling never reaches `compile_data` and the old string survives
(eigenius#217). Fixing the decompiler is worth doing and is **not** a prerequisite here.

The script walks each `type_name` / `param_kind` string and rewrites it to the value the new encoder
would produce — the mapping is `decode_param_kind_str`'s dispatch inverted, written once.

**Guard it with an equivalence check rather than trusting it.** For every rewritten value, decode
the new form and assert it yields the same `Exp` the old string yielded. The migration is correct
**iff** old-decode and new-decode agree, which is a property a test can hold, not a claim to
believe. Same discipline as the round-trip tests.

## 5a. The open-term question, and where it landed

`param_kind` and `type_name` values are **open terms** — they name the declaration's own
parameters. `core:Option`'s `some(value : A)` encodes as `Var("A")`, and `A` is bound by `Option`'s
parameter list. Rule 21 (`check_type_expr_well_typed`) type-checks every `eigentt:TypeExpr`-valued
property as a **closed** term, so the retype made it report `unbound variable in type context: A`
for a value that is well-formed where it lives. Every other TypeExpr-valued property —
`canonical_proposition`, `axiom_statement`, `definition_type` — carries a closed term. These two do
not, and §5 did not anticipate it.

**Resolved by giving the check a binding context — supplied by the rule that owns the telescope.**
Rule 21 declines these two properties, and Rule 23 routes the whole `core:InductiveType` resource
through `check_type`'s `Exp::Inductive` arm, which now type-checks the type former
`Π params. Π indices. sort` and each constructor's full `Π params. Π args. Self(params)` chain
(`check_inductive_decl_telescopes`). Every value of these two properties is a domain in one of
those Π chains, so the Π typing rule checks each in the scope of the binders before it. That is
strictly stronger than what Rule 21 could have established, and it is the relationship
`eigentt:definition_body` already has with Rule 24.

**The arm was not doing this before.** `Exp::Inductive` ran `check_positivity` and
`validate_indexed_ctor_conclusions` over the declaration's structured fields and never applied the
Π rule to them, so a parameter kind or constructor argument type that was not a type at all was
admitted. `references/nanoda_lib` gets the check for free — a declaration there is a single Π-chain
`Expr`, so `check_declar_info` (`src/tc.rs:165`) inferring its type checks every parameter kind, and
`check_ctor` (`src/inductive.rs:881`) walks the constructor chain calling `ensure_infers_as_sort` on
each binder domain (`:900`). EigenTT fused the telescope into structured fields and did not
re-implement the check they displaced — the same omission the `Exp::InductiveType` arm documents for
a type former's *arguments*.

**The constructor-argument universe constraint is ported too.** nanoda's `check_ctor` requires each
argument's sort to be `≤` the inductive's unless the inductive is in `Prop` (`src/inductive.rs:904`,
with `is_zero(l)` = `leq(l, Zero)`, `src/level.rs:264`). An inductive at `Sort n` storing something
from `Sort m > n` smuggles a large type into a small one and Girard's paradox follows; `Prop` is
exempt because a proposition has no computational content to smuggle. The parameter prefix is
exempt as well — nanoda strips it before the loop starts (`:888-897`), and it must, or
`List (A : Type u) : Type u` would reject itself.

**Measured before it rejected anything**, per the #194 / #92 protocol: the check logged instead of
failing, and the whole workspace produced **one violating declaration** —
`reasoning:JustifiedBy.spec_poly`, three of its arguments. `JustifiedBy` was declared at `Type 0`
and binds `T : Type 1`; `Type 1` is `Sort 2`, so that argument sits at `Sort 3`. **`spec_poly`'s own
comment had predicted it** — *"a rule quantifying over `Type 1` domains would need `Type 2`, and so
on up the ladder"* — while the declaration said `Type 0`. Nothing enforced it, so the two sat
contradicting each other in the same file.

The fix is one token: `JustifiedBy : … -> Type 2`. A certificate's TYPE moves up a universe, its
VALUES do not change, and nothing checks `JustifiedBy(j, p)` against a fixed sort — the full suite
is green with no other edit. **This adds `reasoning` as a FIFTH moved layer.**

**The parameter-prefix agreement check is ported too, in #219.** nanoda `assert_def_eq`s each prefix
binder against the declared parameter (`:892`). EigenTT's `build_ctor_type` builds the prefix FROM
`decl.params`, so the positional path agrees by construction — but a `core:ctor_type` payload
carries an AUTHOR-WRITTEN prefix, which is what the ESL typed constructor form `mk : <type-expr>`
emits. Measured first: zero disagreements across the bootstrap and the whole workspace. Now enforced
in the same walk, so nanoda's `check_ctor` is fully ported bar the mutual/nested handling that
belongs to #20 / #21.

## 6. Scope

**Folded into P2 on `eigentt-improvements`** (maintainer decision, `2026-08-23`). It is still a
distinct change from #188 with its own gate (§7) — universe polymorphism needed `core:Level` and has
it — but it lands on the same branch and rides the same reseed.

**But it SHOULD ride #188's reseed if it is ready in time.** An earlier draft of this section said
the opposite, and the reasoning was wrong twice over:

- *"The open validator question could surface mid-reseed."* It cannot. Rule 16 validates the
  embedded ontology set in `cargo test -p eigenius-kernel --lib bootstrap`, ~2s. A retype that makes
  it diverge or reject fails there, on a green-tree check. A reseed only runs against an
  already-green tree; risk attaches to the change, not to the reseed step.
- *"Folding it in trades a bounded cost for an unbounded one."* Hand-waving with no mechanism. A
  reseed is a fixed cost — ~40 minutes plus the snapshot, the demo artifacts and both parse
  baselines. A change landing before it is carried free; a change landing after buys a **second**
  reseed. Batching is cheaper, which is precisely why #188's slice 5 was sequenced before the
  reseed, citing #196 — which paid two reseeds by discovering a second bootstrap edit after the
  first had gone.

The one real consideration is weaker and about attribution, not cost: the parse baselines and demo
artifacts are **measurements**, and two structural changes in one reseed make a drifted baseline
ambiguous. That does not justify a second reseed — #188 already moves `core` twice, the baselines
are re-derived once regardless, and attribution is recoverable by re-running a gate.

**So the gate is readiness, not sequencing:** settle §4's validator question, land the change with
its own green gate, and if that happens before the reseed runs, it rides along.

## 7. Exit gate

- ✔ `TypeExpr` declared in `core-ontology.json`, ordered after `core:Level`; removed from
  `eigentt-type-fragment.json`; **IRI unchanged**, so no reference in any chain or crate moves.
- ✔ `param_kind` and `type_name` are `data_type core:resource` / `class_types [TypeExpr]`.
- ✔ `TypeExpr` has a `SizeSort` ctor, and the D47 codec encodes and decodes it (it had no size form
  at all — §5).
- ✔ All **fifteen** code sites (§5) produce and consume `TypeExpr` values; no site dispatches on a
  kind string. The five consumers share one head-reader, `program::ground::arg_type_head`.
- ✔ **`decode_param_kind_str`'s silent `Sort(1)` default is gone** — the function is deleted, as is
  `decode_index_kind_str` — and `check_inductive_arg` no longer returns `Ok` for an unparseable
  `type_name` (§4a), nor `.unwrap_or_default()`s it into a report naming a parameter that does not
  exist. A parameter reference decodes to `Exp::Var` and is checked.
- ✔ `data Vec (A : Sort u)` compiles, with a test —
  `universe_polymorphic_parameter_kind_survives_the_round_trip` (`program/ground.rs`), which pins
  the decoded kind as `Sort(Param("u"))` rather than `Sort(_)`, because `Sort(_)` would have passed
  against the old silent `Sort(1)` default. The telescope check has its own commit-path test,
  `parameter_kind_referring_to_a_later_parameter_is_rejected_at_commit`.
- ✔ The hand-authored values are migrated by `scripts/migrations/188-type-name-to-typeexpr.py`,
  each guarded by the old-decode/new-decode equivalence check — **85 values across 3 files**, not
  the 89 across 4 estimated in §5: `eigentt-type-fragment.json`'s 29 are `eigentt:type_expr` values,
  a different property, and were never in scope.
- ✔ Manifest moves on **five** layers: the four predicted (`core`, `eigentt-type-fragment`,
  `formulas`, `lean-expressions`) plus **`reasoning`**, which was not predicted — the
  constructor-argument universe constraint (§5a) forced `reasoning:JustifiedBy` from `Type 0` to
  `Type 2`. Re-pinned in `kernel/tests/bootstrap_manifest_pinned.rs`.
- ✔ Rule 21 declines `param_kind` / `type_name`, and Rule 23 — renamed from a positivity rule to
  `check_inductive_declaration`, `rules/inductive_decl.rs` — routes the declaration through
  `check_type`, which type-checks both telescopes (§5a).
- ✔ `cargo test --workspace` (185 binaries, 0 failed), `clippy -D warnings`, `fmt` clean.
- Then the reseed this rides (§6).

> **LANDED `2026-08-23`.** All of the above except the reseed. Manifest re-pinned on the four
> layers. The build's findings are in §5 and §5a; two of them — `check_type`'s `Sort(1)` fallback
> and `decode_indices`' `EigonClass(core:Set)` default — were latent defects the retype exposed
> rather than caused.
