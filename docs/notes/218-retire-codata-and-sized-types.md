# #218 — Retire sized types and codata

Decision note for [#218](https://github.com/eigenius/eigenius/issues/218), which was filed
`needs:decision` with two arms. **Decided `2026-08-23` by the user: remove both**, and the scope
widened from the sized encoding half to the whole codata subsystem.

This note exists so the deletion is recoverable and so nobody re-derives the evidence.

## 1. What was decided, and on what grounds

> "If we do not need the sized halfs, we should remove them" … "also prune codata, which is
> consistent with nanoda not having it."

**The nanoda argument is verified and is stronger for sized types than for codata.** Checked at
`6ae1f0c`:

- `references/nanoda_lib`'s entire term language is `StringLit · NatLit · Proj · Var · Sort · Const ·
  App · Pi · Lambda · Let · Local`. Its declaration forms are `Axiom · Quot · Theorem · Definition ·
  Opaque · Inductive · Constructor · Recursor`.
- Zero occurrences of `Sized`, `SizeSort`, `SizedPi`, `SizeInf`, `SizeSucc`.
- Zero occurrences of `codata`, `coinduct`, `corecurs`, `observation`.

**Sized types are absent from Lean 4 entirely** — not the kernel, not the frontend. Lean does
termination by structural and well-founded recursion, elaborated to `WellFounded.fix` before the
kernel sees anything. Sized types are an Agda/MiniAgda lineage, and N2 §4 established our
implementation is a port of MiniAgda's `Warshall.hs` from a reference **that is not vendored here**,
so its faithfulness was never checkable.

**Codata is a different case and the note should not blur them.** EigenTT added codata *knowing*
Lean lacks it — D11 is a design document with `Status: Implemented (Phase 9b)`. So "nanoda does not
have it" describes a **deliberate divergence**, not an accidental one. What justifies removing it is
not the divergence but the absence of a user (§2), and the divergence is what makes it cheap to
remove: nothing downstream of the kernel is shaped around it.

## 2. The evidence: nothing uses either

| | chain users | kernel implementation | tests |
|---|---|---|---|
| sized types | **0** — no chain term, ESL or JSON, carries a size form | incomplete: `SizedPi`/`SizeInf`/`SizeSucc` have **no D47 codec path at all** | yes |
| codata | **0** — zero `codata` declarations in `ontologies/` `experiments/` `demo/`; `core:CodataType` is declared in core and has **zero instances** | complete across ~15 files; round-trips D47 via `ConstRef` + `App` | 11 sites compiling real ESL |

So sized types are *incomplete and unused*; codata is *complete and unused*. Neither has ever been
exercised by anything but its own tests.

The dependency runs one way and is why the two go together: **sized types exist for codata
productivity first** (D19:491 — "required for complete termination story when combining inductive
recursion with **codata corecursion**"), not for inductive termination. Removing codata removes the
only thing sized types were for.

**Nothing live depends on either.** Verified before proposing the deletion:

- The task / trace subsystem does **not** reference any codata form. D11 §5's "Tasks as Codata" is
  conceptual framing; `RocksTraceStore`, task tracking and resumption are independent of `Exp::Codata`.
- `check_guarded`'s only caller is `check/mod.rs:247`, the `Decl::Drec` arm — and #66 established
  ESL has **no `letrec`** and nothing in production constructs a `Decl::Drec`.

## 3. Deletion surface

```
whole files          kernel/src/nbe/check/codata.rs      1012
                     kernel/src/nbe/sized.rs              410
                     kernel/src/nbe/sized_rigid.rs        347
                                                        -----
                                                        1,769 lines
scattered            ~700 lines across 27 further files, led by
                     program/ground.rs (157), nbe/check/mod.rs (131),
                     nbe/check/inductive.rs (37), nbe/term.rs (36),
                     nbe/eval/mod.rs (35), nbe/check/conv.rs (32),
                     esl/compile.rs (30), nbe/val.rs (25), esl/parser.rs (20)
term language        Exp/Val: Codata, CodataType, SizedPi, SizeSort, SizeInf, SizeSucc
ESL surface          `codata` keyword + observation syntax; bounded binder `{j < i}`;
                     the `Size` / `Inf` compiler built-ins
ontology (core)      core:CodataType, core:Observation, core:observation_name,
                     core:observation_type, core:observations
```

**The ontology edit means another reseed** (~35 min + alignment + demos). One reseed was just paid on
`2026-08-23`; this is a second. Batch anything else that touches a bootstrap ontology into it —
#219 and #220 do not, so #218 owns this reseed alone.

## 4. Design docs that stop describing the implementation

- **D11 (Codata, Streams, and Resumable Execution) — DEPRECATED** (user, `2026-08-23`). Its status
  was `Implemented (Phase 9b)`. §2, §3, §4, §5.1 and §6 are codata and become a record of something
  the platform no longer has. The document is **not deleted**: it is the account of what was built
  and why, and deleting it would destroy the only written reason the machinery existed.
  §5.3/§5.4 (persistent task state, concurrent tasks) describe the trace/task subsystem, which is
  live and never depended on codata — the deprecation header must say so explicitly, or the next
  reader will assume tasks went with it.
- **D19 §8** — the sized-types sections, including `:491`'s decisions-table row.
- Eight further design docs mention codata in passing and need a sweep for claims that stop being
  true.

The N2/#139 deletion is the precedent: it exposed two stale docs with dangling
`references/miniagda/…` paths, and fixing those was part of the change, not a follow-up.

## 5. What would have to be true to bring either back

Recorded so a future reader does not rediscover it, same discipline as N2 §5:

**Sized types** — (a) a chain declares a sized inductive or codata type; (b) size metavariables exist
in the term language (`FlexId` appears nowhere outside `sized.rs` today); (c) MiniAgda is vendored,
or the algorithm is re-derived from the paper rather than trusted as an unverifiable port.

**Codata** — a chain-resident consumer that genuinely needs infinite structure with productivity
guarantees, rather than a Rust-side stream. The kernel is not the only place a stream can live, and
D11's three motivating capabilities were traces, task tracking and incremental processing — the
first two are built and independent of codata, and the third has never been attempted through it.

Both are in git history, and this note records what they were and what removed them.

## 6. Exit gate

- The three files are gone; no `Codata`/`Sized`/`Size*` variant remains in `Exp`, `Val`, the ESL AST,
  the parser, the compiler, the ground decoder or the D47 codec.
- `core:CodataType`, `core:Observation` and the three observation properties are gone from
  `core-ontology.json`; the manifest is re-pinned and the reseed run.
- D11's status corrected and its codata sections marked retired with a pointer here; D19 §8 likewise;
  the eight further docs swept for now-false claims.
- No test is deleted merely because its subject is gone **without checking what else it covered** —
  the sized-codata tests exercise the ground decoder and the ESL round-trip too, and any coverage
  that is not about codata is re-pointed rather than dropped.
- `cargo test --workspace`, `clippy -D warnings`, `fmt` clean; then reseed, alignment, WRN demo and
  the parse gate, as `2026-08-23`.
