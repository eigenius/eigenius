# D63 — Passive-voice handling: promotion, agent, roles

**Status:** design (short). General grammar/importer infrastructure for the English passive. Extracted from
[d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md) (where it was the phrasal-half
dependency) because it is **broader than denominals** — it serves every passive clause; the denominal
phrasal half is only one consumer.

## 1. The three coupled gaps

All Derived this session — from [convert.rs](../../crates/eigenius-wordnet/src/convert.rs), a grammar grep,
and the `db_backed_encoding::show_based_on_x_reading` witness (`2026-07-05`):

1. **No object→subject promotion.** The importer's passive-participle entry reuses the *active* category:
   `cat_pss = kind.cat("pss", …)` ([convert.rs:494](../../crates/eigenius-wordnet/src/convert.rs), emitted
   `:557`), and `cat` (`:193`) varies only the `Fin` feature — so a transitive verb's `pss` form is
   `(S[pss]\NP)/NP`, object slot retained. And there is **no passive rule in the grammar** (grep of
   parser.rs/category.rs for `pss`/passive/promote → nothing; only a `ger` fronted-adjunct rule). Nothing
   turns `(S\NP)/NP` into the promoted `S[adj]\NP`.

2. **No agent demotion / suppression.** The passive backgrounds the agent — "X is based on Y" asserts no
   agent; "represented **by** Z" makes it an oblique. Nothing existentially-closes or drops the agent slot.
   Witnessed: `The method is based on sequencing` → the verb reading is `(Πg. base_v(method, g)) ∧
   prep_on(…)` — object slot Π-bound, inert; `…were represented by these data sets` sits in grammar-gap.

3. **Role mis-assignment.** The coarse 2-place transitive axiom is `v00636888_t(theme, agent)` — slot 2 is
   the *agent*; the passive-with-PP wants slot 2 = ground/oblique (`rel(theme, ground)`). Reusing it puts the
   wrong entity in the wrong role (the "faithful target" correction,
   [d63-compound-morphology.md](d63-compound-morphology.md) §2a).

## 2. Target

For a passive clause, produce `rel(theme, ground)` — theme promoted to subject, the `by`/`on`/… object as
the ground, agent existentially closed or dropped:

- `X is based on Y` → `base_on(X, Y)`
- `X is mediated by Y` → `mediate(X, Y)`
- `X was represented by Y` → `represent(X, Y)` (or `∃a. represent(a, X)` when there is no `by`-phrase)

## 3. Touchpoints

**Importer — [`crates/eigenius-wordnet/src/convert.rs`](../../crates/eigenius-wordnet/src/convert.rs):**

| change | what | anchor |
|---|---|---|
| object+PP `FrameKind` | `((S\NP)/cat_pp_arg)/NP`, 3-role axiom `Entity→Entity→Entity→Prop`; mirror `PpOblique` (commit `2b22705`). Only frames **20/21** are genuine object+PP (13/22 are subject+PP → `PpOblique`). | `FrameKind` `:145`, `tag`/`arrow`/`cat` `:161`/`:174`/`:193`, `classify` `:229` |
| **passive-participle promotion** (the missing piece) | emit the promoted passive `(S[adj]\NP)` (transitive) / `(S[adj]\NP)/cat_pp_arg` (object+PP): object→subject, agent closed, sem `λθ. rel(θ, ground)`. Replaces the active-valency `cat_pss`. | `push_verb` `:478`, `cat_pss` `:494`/`:557` |
| adjective-voice PP-complement | adjectives that subcategorize a PP (`dependent on`, `specific to`, `related to`) need `(S[adj]\NP)/cat_pp_arg`; `push_adj` emits only bare `S[adj]\NP` | `push_adj` `:596` |

**Grammar alternative — [`kernel/src/dcg/`](../../kernel/src/dcg):** instead of (or besides) importer-emit, a
**passive type-changing rule** that promotes `(S\NP)/NP` (or its `pss` form) → `S[adj]\NP` at composition
(object→subject, agent existentially closed). Decide importer-emit (per-verb data) vs grammar-rule (one
rule); the grammar rule generalizes to every verb without re-seeding.

**Linkers — [`ontologies/lexicon/closed-class.esl`](../../ontologies/lexicon/closed-class.esl):** the
argmarker set (`to_arg`/`from_arg`/`on_arg`/`with_arg`, `:1124+`, sem `argmarker_sem` `λy.y`, cat
`cat_pp_arg/NP`) needs **`by_arg`** added (the agent/by-passive). `cat_pp_arg` denote is
[`category.rs:68`](../../kernel/src/dcg/category.rs).

## 4. Consumers

- [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md) — the phrasal half (`E link X`) of
  every passive-participle-voice denominal element (`-based`/`-mediated`/`-derived`/`-induced`).
- **Ordinary passive clauses** on the WRN page (`were represented by`, `is associated with`, …) — several
  in the current grammar-gap list. This track closes them, independent of any denominal work.

## 5. Prior art

Passive as a category-changing lexical rule — CCG's unary type-changing rules; LFG / HPSG lexical rules.
Pick importer-emit vs grammar-rule at implementation.
