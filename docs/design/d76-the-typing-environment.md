# D76 — The typing environment

**Status: skeleton.** The design is not written. This file exists so the decisions already made, and
the obligations other work has parked here, have a home rather than living in another document's
appendix.

Implements **Seam A** of `docs/design/d75-fusing-eigentt-and-the-knowledge-graph.md`.

## 0. Scope

The chain of layers *is* the environment `Γ_env` of the judgment `Γ_env; Γ ⊢ e : T`. Today the layer
is an **effect capability** on `EvalCtx`, and the type theory has no environment on two of its three
surfaces (D75 §2):

| surface | environment today |
|---|---|
| `check` | partial — `CheckCtx.layer: Option<Arc<Layer>>` + `type_cache` + `CheckHooks::resolve_class`, classes only |
| `eval` | none |
| `eq_nf` / `subtype_of` | none — no context parameter at all |

## 1. Already decided (D75 §8)

Carried in so D76 starts from them rather than reopening them.

- **Q1 — the environment belongs to `check` and `conv`, not `eval`.** `eval` already produces opaque
  values for chain references (`Val::EigonClass(iri)`, `Neut::EigonAxiom(iri)`), so it builds
  neutrals and defers — nanoda's shape. 195 `eval` sites are untouched; 23 `eq_nf` sites are the gap.
  Not a materialised projection: that is the full-chain-scan antipattern behind two prior OOMs. The
  memo at `layer/mod.rs:678` is already the right shape — a lazy cache keyed by `(LayerId, Iri)`.
- **Q1 — the `Option` goes.** "No layer access in pure check mode" is what let the three surfaces
  diverge.
- **Q2 — δ is decided per kind**, so no transparency annotations are needed: definitions
  **transparent** (pinned by D66 §4 and the proposition hash), axioms **opaque**, classes **opaque**
  (unfolding them would make class identity structural — and 749 of 894 shipped classes already
  resolve to the same `Val::One`, so opacity is the sole mechanism distinguishing most of the
  ontology), inductives deferred to Q3.
- **Q2 — §3.3 reconciles by fixing `check`, not `eq_nf`.** `find_sigma_field`'s unfolding must become
  a **projection rule** that consults the environment to type a field access without asserting the
  class equals its unfolding.
- **`Val` does not capture the environment.** The neutral carries the IRI; `Γ_env` lives in the
  conversion context, as nanoda's `Tc` holds `Env` while `Const` holds name + levels.

## 2. Inbound obligations — work parked here by other documents

**Each of these is blocked until D76 lands. This section is the reason this file exists.**

### 2.1 D78 §3.1 — the complete `Refine` subtyping rule

`Refine(R, S) <: Refine(R′, S′)` requires `⋀S ⊨ D` for every `D ∈ S′`. Entailment resolves class IRIs
against the layer chain, and `subtype_of(level, sub, super_)` / `eq_nf(level, v1, v2)`
(`nbe/check/conv.rs:290`, `:30`) take **no context at all**.

D78 Phase A shipped **set inclusion** (`S ⊇ S′`) instead — sound, because a constraint present in `S`
is trivially entailed by `⋀S`; **incomplete**, because it rejects the case where `S` entails `D`
without containing it. Strengthening it to the full rule is a **one-arm change** in
`subtype_of_inner` once conversion carries `Γ_env`.

**Signal that this is done:** `entailment_beyond_set_inclusion_is_not_yet_decided`
(`kernel/src/nbe/readback.rs`) currently asserts the *rejection*. It must **flip** — it pins the
limitation, not the semantics.

### 2.2 Q4 / 4c — the recursor motive's codomain (#228)

The motive codomain is the constant `Exp::sort(2)` (`nbe/check/inductive.rs:589-594`), which caps
large elimination at **Set**: a motive returning `Sort(k)` has type `Sort(k+1)`, and only `k ∈ {0,1}`
pass. 4c makes it a level parameter, `I.rec.{u}` with motive `I(params) → Sort u`, which needs
`Const(iri, levels)`. `large_elim_admitted` keeps its exact meaning and call site — the two-way choice
between `sort(0)` and `sort(2)` becomes *`u` pinned to 0* vs *`u` free*.

**Signal:** `large_elimination_is_capped_at_set_not_type_n` (`nbe/level.rs`) must flip.

### 2.3 #188's residual — declaration-level `uparams` and level arguments

Affordable only after consolidation to one `Const`, which is what removes the self-reference stub and
`PartialEq`-by-IRI. Without that, levels land on five variants and `List.{0}` compares equal to
`List.{1}`.

### 2.4 D77 — `InvalidatedSignature`

D20 names the missing merge cascade kind **type-checker driven** (`layer/merge/cascade.rs:24`). The
rule-driven half of D77 needs nothing from here (`validation/retroactive.rs` discharges the linear
form without touching the type checker), but the *designed* backstop is a type-level check and does.

## 3. What D76 must settle

From D75 §9, unchanged:

- **The trait's signature.** What are the methods; what does a lookup return — `Arc<Resource>`, a
  `Decl` enum, a `Val`; who owns the `(LayerId, Iri)` memo once two surfaces share it. `CheckHooks`
  (`nbe/check/hooks.rs:34`) is already the boundary and should absorb the role.
- **δ mechanics, not δ policy.** Conversion must know a `Const`'s kind *without* resolving on the hot
  path — the lazy-δ shape, compare IRIs syntactically first and resolve only on mismatch — plus
  unfolding order when both sides are transparent.
- **The layer under construction.** During `check` of one declaration, are *later* declarations in the
  same layer visible? nanoda extends `Env` declaration-by-declaration. Forward references and
  intra-layer self-reference both turn on this.
- **The consolidation migration.** How ~583 sites move: `InductiveType(decl, args)` →
  `App(Const(iri), args)`, what happens to the inlined `Arc<InductiveDecl>` at readback, which D47
  codec arms change. **Persisted terms change shape, so this is a chain-format change** — unlike D78,
  which is additive to the chain throughout.

## 4. References

- D75 §2 (the measurement), §4 (the chain as `Γ_env`), §5 (what it forces and costs), §8 Q1–Q4
- D78 §3.1 (the parked obligation), §7 (why D78 did not wait)
- nanoda_lib at `6ae1f0c` — `env.rs:37 DeclarInfo`, `expr.rs:54 Const{name,levels}`
- #188, #215, #228
