# P2 · N1 — Positivity criterion and declaration routing

Note N1 of the [P2 plan](p2-type-theory-soundness-plan.md) §2. Settles
[#92](https://github.com/eigenius/eigenius/issues/92)'s fork. Written `2026-08-22` from `6744c9a`.

**Working hypothesis, adopted `2026-08-22`: arm 1 — extend the criterion.** Keep `lexicon:Cat` as
declared; make the kernel admit what it already relies on, rather than rewriting the parser's
category algebra to fit a criterion narrower than the type theory requires.

## 1. What is actually wrong

The positivity pass exists and is correct; it is **not reachable from the path that matters**.
`check_positivity` runs from `check_type`'s `Exp::Inductive` arm (`kernel/src/nbe/check/mod.rs:335`),
and `Exp::Inductive(` is constructed nowhere in `kernel/src/esl/compile.rs`. An ESL `data`
declaration reaching the chain through `Validator::validate()` is never checked. That is why #92's
probe reported 0 errors.

Wiring it in unchanged rejects the bootstrap: `cat_forall`, `cat_fin_forall` and `cat_num_forall`
(`ontologies/lexicon/lexicon-ontology.esl:271,280,281`) are higher-order positive, and
`positivity.rs` refuses that shape (`rejects_higher_order_positive`, `:507`).

## 2. The criterion

Today `check_arg_positivity` (`positivity.rs:168`) has three cases: no occurrence → accept; direct
application `D(params)(indices)` → accept; **anything else → reject**. The generalization replaces
case 3's blanket rejection with the classical strictly-positive shape:

> A constructor argument type is **strictly positive** in `D` when it is a Π telescope
> `(a₁ : A₁) → … → (aₖ : Aₖ) → D(params)(indices)` in which `D` occurs in **no** `Aᵢ`,
> the parameter prefix is passed through unchanged, and `D` occurs in none of the `indices`.

`k = 0` is today's direct case, so this is a strict extension. `D` occurring in any `Aᵢ` stays
rejected — that is the negative occurrence, and it is the thing positivity exists to stop. The
uniform-parameter check (`check_params_uniform`, a port of nanoda's `ctor_app_params_ok`) and the
no-`D`-in-own-indices check are unchanged and apply to the telescope's conclusion.

`cat_forall`'s second argument is `(Set -> lexicon:Cat)`: `k = 1`, `A₁ = Set`, no occurrence of
`lexicon:Cat` in `A₁`. It fits exactly, as do the other two.

nanoda implements this criterion — `check_positivity1` (`references/nanoda_lib/src/inductive.rs:758`
at `6ae1f0c`), with `which_valid_ind_app` `:867` and `is_rec_argument` `:1082`. Port it rather than
re-deriving; the plan's §3 discipline applies, and the citations in `positivity.rs` (`:24`, `:129`,
`:584`) are against the old pin `f58f2f6` and need repointing in the same commit.

## 3. One predicate, spelled in three places

The criterion is not confined to the positivity checker. `InductiveDecl::is_direct_recursive_ref`
(`kernel/src/nbe/term.rs:465`) is

```rust
matches!(typ, Exp::InductiveType(d, _) if d.iri == self.iri)
```

and three sites consume that same notion of *"this argument is a recursive occurrence"*:

| site | what it does with it |
|---|---|
| `positivity.rs:168` `check_arg_positivity` | admits the argument |
| `recursor.rs:~178` `derive_minor_type` | emits one IH binder per recursive argument |
| `eval/iota.rs:88` | applies one induction hypothesis per recursive argument |

Widening the criterion in one place and not the others is how the halves come to disagree — which is
#138's defect in a different pair of functions. **The change is therefore a single shared analysis,
not three edits**: a function returning the recursive-argument *shape*

```
RecArgShape { binders: Vec<(Patt, Exp)>, indices: Vec<Exp> }
```

`None` for a non-recursive argument, `Some` with `binders` empty for today's direct case. All three
sites consume it. That is the structural version of this change and the reason to do it once rather
than to special-case higher-order arguments at each site.

## 4. What the eliminator has to become

With `arg : (a₁ : A₁) → … → (aₖ : Aₖ) → D(params)(idx…)`, the two derived forms generalize the way
the recursor module already predicts (`recursor.rs:36` — *"Higher-order recursion would need IHs of
function type (`Π x:T. C(arg(x))`)"*):

- **Minor premise IH** — `Π a₁:A₁ … Π aₖ:Aₖ. motive idx… (arg a₁ … aₖ)`, instead of
  `motive idx… arg`.
- **Iota** — the induction hypothesis becomes `λ a₁ … aₖ. rec … (arg a₁ … aₖ)`, instead of
  `rec … arg`.

nanoda builds both from one `local_indices` — `mk_motive_dep` `inductive.rs:1058`, `mk_minors1group`
`:1161`, with the IH abstraction at `:1177-1178`.

## 5. The finding that changes the sequencing

`positivity.rs`'s module doc says admitting a higher-order positive occurrence *"would create a
soundness gap"*, because iota cannot construct the induction hypothesis. **The evidence says
otherwise, and the difference is worth a paragraph because it decides whether arm 1 can be staged.**

`derive_minor_type` and `iota_reduce_impl` filter on the **same** predicate — `is_direct_recursive_ref`
at `recursor.rs:~178` and `iota.rs:88`. So if a higher-order argument were admitted today, both would
skip it identically: the minor's declared type would carry no IH binder, and iota would apply none.
The two halves agree. The failure mode is a **missing** induction hypothesis, not a wrong one — an
eliminator too weak to do induction through that argument, which is incompleteness, not unsoundness.

That is a claim to **verify before relying on**, not to assume. The check is cheap and belongs in the
first commit: declare a reflexive inductive, derive its minors, iota-reduce a constructor application,
and assert the derived minor type and the reduction still agree. If they do, arm 1 splits into two
independently landable steps:

1. **Widen the criterion and route declarations through the pass.** Closes #92. The bootstrap keeps
   its three constructors, and the eliminator is weaker for them than for direct recursion — a
   documented limitation, not a hole.
2. **Function-typed IHs in the minor derivation and iota.** Makes induction through a reflexive
   argument possible.

If instead the two halves are found to disagree, step 1 cannot ship alone and #92 waits on the full
eliminator work — which is the reading that makes #138 a hard prerequisite.

## 6. Consequences for the package

- **#138 first either way.** Step 2 above rewrites `derive_minor_type`'s IH construction, which is the
  same function #138 fixes; doing them in the other order means writing the IH generalization twice.
  Under the staged reading, #92 step 1 can land before #138 — the plan's suggested order does not
  have to serialize entirely.
- **No reseed from #92.** Arm 1 is kernel-only. The bootstrap ontologies are untouched, the manifest
  does not move, and the package's reseed accounting reduces to #188 (certain) and #194 (uncertain,
  on #136's precedent).
- **`lexicon:Cat` is unchanged**, so nothing in the parser's category representation moves and D63
  §8.2 stands as written.

## 7. Mutual and nested

The criterion above is for a **single** declaration. Neither #20 nor #21 is in scope, and the note
records the posture rather than leaving it to omission:

- **#20 (mutual)** — positivity must range over every declaration in the block simultaneously.
  nanoda threads `st.all_inductives_incl_specialized` through the walk (`inductive.rs:88,181,183`).
  The `RecArgShape` analysis in §3 takes a single `decl`; making it take a block later is a signature
  change, not a redesign, **provided** the shape type is introduced now rather than the widening being
  inlined at each of the three sites.
- **#21 (nested)** — neither positive nor negative; Lean reaches it only through specialize / check /
  unspecialize (`st.nested_to_unspecialized_ty_nofvars`, `inductive.rs:180`), so positivity does not
  apply until after the expansion. Nothing in §3 needs to anticipate it.

So: design for a single declaration, but introduce the shared shape type, because that is the one
decision that would be expensive to reverse when #20 is picked up.

## 8. Open questions

1. **Do `derive_minor_type` and `iota_reduce_impl` agree on a skipped higher-order argument?** §5. The
   answer decides whether #92 lands in one step or two. Verify by construction, first commit.
2. **Does anything eliminate `lexicon:Cat` with `InductiveRec` today?** If the parser only constructs
   `Cat` values and matches them, the weaker eliminator from §5 step 1 costs nothing at all in
   practice. Unmeasured.
3. **Large elimination.** D46 §7's singleton-elim rule interacts with reflexive constructors in ways
   this note has not examined. Out of scope for the criterion, but N1 should not be read as clearing
   it.

## 9. Exit gate

- ESL `data` declarations reach `check_positivity`.
- A negative-occurrence declaration is rejected through `Validator::validate()` — the path #92's probe
  took, not just through `check`.
- The bootstrap loads, with `cat_forall`, `cat_fin_forall` and `cat_num_forall` unchanged.
- `positivity.rs` cites `6ae1f0c` and its line numbers are correct.
- The §5 agreement check exists as a test, whichever way it comes out.
