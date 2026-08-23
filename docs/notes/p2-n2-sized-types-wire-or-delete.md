# P2 · N2 — Sized types: wire or delete

Note N2 of the [P2 plan](p2-type-theory-soundness-plan.md) §2. Settles
[#139](https://github.com/eigenius/eigenius/issues/139), and supplies the cost input
[#66](https://github.com/eigenius/eigenius/issues/66) needs. Written `2026-08-22` from `155edd1`.

**Recommendation: delete the solver half.** Keep the comparison pair as the sized-types surface.

## 1. What sized types are FOR, and what is wired

**The motivation is coinductive, not inductive.** D19's decisions table is explicit — *"Sized types?
In scope (#16) | Required for complete termination story when combining inductive recursion with
**codata corecursion**"* (D19:491), and §302 makes the same point about producing coinductive streams
of results. Codata productivity is the reason the machinery exists; sized inductives are the other
half of the same story, not the point of it.

That matters here because the **productivity path is a wired site**, and it is the one a reader
scanning `sized.rs`'s callers would miss — it does not call the comparison pair at all. It *feeds*
it:

| site | what it does |
|---|---|
| `check/mod.rs` — `(Exp::Lam(p, e), Val::SizedPi(upper, g))` | **Productivity.** Opens the size binder and registers `j < upper` in the TSO, so a corecord's observations are forced by type to produce at sizes strictly below the outer one (`check/codata.rs:783-791`, "Productivity via sized codata", Phase 11b step 15f) |
| `check/conv.rs:373` | subtyping of an inductive's size parameters, `sub_pᵢ ≤ sup_pᵢ` — consults the TSO the arm above populates |
| `check/inductive.rs:493` | a bounded size argument is strictly below its binder's upper bound |
| `check/mod.rs:913` | the same check on the `CtorArg::Size` path |

The unsized productivity check, `check_guarded` (`codata.rs:189`, D11 §3 / Phase 9b-i), is the
weaker companion: it rejects a corecursive reference observed inside its own definition, and its own
doc names the gap sized types close — *"Sized types would close that gap — out of scope"*
(`codata.rs:185`), for cases like `broken(n).head` that guardedness cannot see through.

All of this works, and **nothing here proposes changing any of it.**

## 2. What is not wired, and why "add a caller" understates it

The other ~390 lines are a port of MiniAgda's `Warshall.hs`: `Weight`, `Rigid`, `Node`,
`Constraint`, `arc`, `SizeExpr`, `solve`, `Solution`, and the private `Graph` / `warshall`. Nothing
outside the module and its own tests references any of them. The module header names what it is
waiting for — *"constraint emitters elsewhere in the kernel (Phase 11b step 16 onward) can plug in
by maintaining their own ID namespace."*

**Those emitters cannot be written today, because there is nothing to solve for.** `solve` computes
assignments for **flexible** size variables, and the term language has no flexible size:
`grep FlexId` outside `sized.rs` returns nothing, and a `Val` of size sort is only ever `SizeInf`,
`SizeSucc(_)` or a rigid `Neut::Gen`. Introducing size metavariables means touching `Exp`/`Val`,
`eval`, `readback`, `conv` and the checker *before* a single constraint is emitted. D48 Phase C's
`Neut::Meta` could plausibly host them, but nothing does that today and no design says it should.

So #139's framing — *"That work is unstarted, not blocked: the entry point, the constraint algebra,
and the tests are all in place, and what is missing is a caller"* — is optimistic. What is missing
is the representation the caller would range over.

## 3. The finding that decides it: nothing uses sized types at all

The unwired half is not the only dead part.

- **`core:Size` appears in no ontology, experiment, demo or crate test.** Not one sized inductive is
  declared anywhere. It is also not a chain-resident class: the ESL parser mints a bare
  `Size` name and the compiler special-cases it as a built-in (`esl/compile.rs:1064,1126,1138,2145`)
  alongside `Inf`, so it never had to be declared.
- **No chain term, ESL or JSON, carries a size form.** `"ctor": "SizedPi" | "SizeSort" | "SizeInf" |
  "SizeSucc"` matches nothing across `ontologies/`, `experiments/`, `demo/`.

The wired comparison pair therefore has no chain-resident exercise either. It runs against kernel
unit tests and nothing else. That does not argue for deleting it — it is small, correct, and on the
path where a sized declaration would land the moment one is written — but it does mean the solver is
not "one caller away from useful". It is two absent features away from a first user.

## 4. The reference is not vendored, and the pointer is dangling

`sized.rs`'s header links `../../../references/miniagda/src/Warshall.hs`. **`references/` contains no
`miniagda`** — it has EngiBench, FraCaS, Oxen, ScienceAgentBench, WordNet-3.0, lightblue,
nanoda_lib, ncbi, openccg, publications, umls, wiktionary.

Same staleness class as the nanoda citations corrected in the plan's §3, and worse in consequence:
nanoda is vendored, so its line numbers could be re-pointed and its algorithm checked. This port
cannot be diffed against its source at all. A faithful port whose faithfulness is unverifiable is a
liability on the TCB, not an asset.

## 5. Recommendation, and the counter-argument

**Delete `solve` and its supporting types**; keep the comparison pair as the sized-types surface, and
correct the module header to describe what the module actually is.

**Does the codata motivation change the answer? No, and the reason is worth stating.** Productivity
works from sizes that are *written* — the `SizedPi` binder carries its upper bound, and the arm above
reads it off the type. `solve` computes assignments for sizes that are *unknown*. Inference would
make sized codata (and sized inductives) writable without annotating every size, which is a real
ergonomic gain and exactly parallel to `inductive.rs:482`'s refusal to elide a size argument. But
that gain is unreachable for the same reason as before: there is no unknown size in the term language
to infer. Deleting `solve` costs productivity nothing.

The counter-argument is that this throws away a careful port. Weighed against:

- it has no consumer, and cannot acquire one until size metavariables exist;
- its faithfulness is unverifiable, because MiniAgda is not vendored here;
- it carries review surface on the trusted computing base with no behaviour behind it;
- **and it is recoverable** — the port is in git history, and this note records where it came from
  and what it did.

#139 itself offers this as the alternative: *"If that plan has lapsed, the alternative is deleting
the unreachable half so the comparison pair stands alone as the sized-types surface."* The plan has
lapsed; nothing since Phase 11b step 15 moved toward step 16.

**To revisit, three things must be true**, and the note exists so that whoever revisits does not
rediscover them: a chain declares a sized inductive; size metavariables exist in the term language;
and MiniAgda is vendored, or the algorithm is re-derived from the paper rather than trusted as a
port.

## 6. What this means for #66

#66 weighs restricting the ESL surface to sanctioned recursion forms (option 1) against documenting
the bare-`Decl::Drec` divergence hazard (option 2). Its option 1 lists three sanctioned forms:
`Map`/`Reduce`, `Match` on a sized inductive scrutinee, and codata guarded by `check_guarded`.

**None of the three has a chain user, and neither does the hazard.** Verified across `ontologies/`,
`experiments/` and `demo/`, in ESL source and in JSON chain terms:

| form | chain users |
|---|---|
| `Map` / `Reduce` | none (`program:Map` / `program:Reduce` are program COMPONENTS, a different thing) |
| `Match` on a sized scrutinee | none — no sized inductive is declared anywhere |
| codata — `check_guarded`, or sized productivity | none — `core:CodataType` is declared in the core ontology and nothing instantiates it |
| bare `letrec` (the hazard) | none |

So the earlier reading — that option 1 is expensive because it pushes authors onto the sized-`Match`
path while sizes must be hand-written — was wrong in its premise. It is not that the sanctioned path
is onerous; it is that **no recursion of any kind is exercised by any chain in this repository.**

That makes #66 cheap *now* and expensive later, which is an argument for deciding it now rather than
on the trigger its acceptance criteria imply. Deleting the solver does not change #66's cost either
way: size inference was never what made the sized path usable.

## 7. Exit gate

- `solve`, `Solution`, `Constraint`, `arc`, `SizeExpr`, `Node`, `Rigid`, `Weight`, `Graph`,
  `warshall` and their tests removed; nothing else in `sized.rs` changes.
- The module header describes the comparison pair, states that sized types exist for **codata
  productivity** first (D19:491) rather than reading as an inductive-termination utility, names
  MiniAgda as the *origin* of the deleted half rather than linking a path that does not exist, and
  points at this note.
- `cargo test --workspace`, `clippy -D warnings`, `fmt` all clean.
- #139 closed with the deletion recorded; #66 updated with §6.
