# Reseed memory: 40% of it was glibc arena fragmentation

*2026-08-03. Four A/B runs over the same WordNet chain, one variable each. **This note was twice
rewritten by its own later measurements** — the corrections are kept in place (§"What I got wrong")
rather than edited out, because the failure mode they share is the interesting part.*

## The answer

`MALLOC_ARENA_MAX=2` on the kernel service — now set in `docker-compose.yml`.

| run | variable | peak RSS | idle after |
|---|---|---|---|
| baseline | glibc default arenas | **15.66 GB** | 15.39 GB |
| cache budget 20k | resource cache ÷12.5 | 17.45 GB | — |
| **`MALLOC_ARENA_MAX=2`** | allocator only | **9.41 GB** | 9.05 GB |

The kernel runs on the **system allocator** (jemalloc is opt-in via `--features jemalloc-prof`).
glibc hands a tokio/tonic server up to 8×ncores per-thread arenas; a bulk load's allocation churn —
JSON decode plus a Rule 21 type-check on every one of millions of `lexicon:cat` values — fragments
them, and freed memory is never returned to the OS. Capping arenas at 2 removes ~6.25 GB of the
15.66. The effect compounds with churn: only −12% after chunk 001 (6.64 → 5.87 GB), −40% by the end.

**Still unexplained: the residual 9.41 GB**, which also does not come back at idle (9.41 → 9.05 over
30 s). Fragmentation is a large component, not the whole story. The instrument for the rest is a
`--features jemalloc-prof` build, which reports *live* bytes instead of leaving liveness to be
inferred from RSS. Open, and no longer blocking.

## What was measured

Load **only** the WordNet chain (base + 3 chunks, 225 MiB of ESL, ~430k resources) into a clean
volume through the served kernel, sampling the kernel process's `VmRSS` every **1 s**, with a **15 s
idle gap after every commit** — the gap is the instrument: if the memory a commit uses is transient,
that is when it comes back.

| | |
|---|---|
| chunk 001 (100 MiB) | → **6.7 GB**; 15 s idle → **no release** |
| chunk 002 (101 MiB) | → **15.4 GB**; 15 s idle → **no release** |
| chunk 003 (24 MiB) | (small) |
| all loads done, container idle >1 min | still **15.39 GB** |
| peak | **15.66 GB** |
| `docker compose down` | host 21 GB used → 5 GB — confirms the RSS was the container |

RSS does not fall in those windows. At the time this read as *"memory is retained per layer"* —
**which was wrong**, and wrong in a specific way worth naming: RSS not falling shows only that the
allocator has not returned pages, not that the data is live. Roughly 40% of it was glibc holding
fragmented arenas (see the table at the top).

**There is a chain-depth term.** Chunk 002 is the same size as 001 and cost more on both axes:
**+8.7 GB vs +6.7 GB** and **111 s vs 55 s**. Loading against a deeper chain is more expensive in
time and in memory.

## Consequences

**Smaller chunks do not help.** `scripts/reseed-lexicon-db.sh --split-bytes` was added on the theory
that peak RSS was a per-commit transient that finer slicing would cap. RSS not falling in the idle
gaps refutes that regardless of *why* it does not fall, and the depth term says more layers is worse.
The flag stays — chunk size is a legitimate thing to vary for the gRPC limit or a partial-load retry
— with its default restored to the importer's and a comment saying not to reach for it for memory.

**A full reseed ran at the edge of a 32 GB host — before the arena cap.** 15.66 GB for WordNet alone
is consistent with the ~26 GB observed on a `--umls-all` run. It had completed on this host before,
so it fit, but only with the machine to itself: three runs failed on 2026-08-03 and the proximate
cause each time was competition for RAM (concurrent `cargo build` / `cargo clippy --all-targets` / a
test suite, one of which was SIGKILLed; and once the IDE), not the workload changing. With
`MALLOC_ARENA_MAX=2` the same WordNet load peaks at 9.41 GB, which should put a full reseed near
16 GB — headroom rather than a knife edge.

**Not caused by the D47 `Fst`/`Snd` addition.** That change is on the bulk-load hot path —
`lexicon:cat` and `lexicon:sem_type` are ranged at `eigentt:TypeExpr`
(`ontologies/lexicon/lexicon-ontology.esl`), so Rule 21 decodes and type-checks every one of the
millions of entries — but it adds 2 constructors to 16 on an inductive whose new arms no
`lexicon:Cat` value ever reaches. It cannot turn 6 GB into 26.

## Ruled out

| candidate | why not |
|---|---|
| in-memory backend used by accident | `serve --db` → `bootstrap_persistent` → `LayerStorage::with_persistent` (`kernel/src/bootstrap/mod.rs`) |
| unbounded resource cache | `with_persistent` builds `BoundedResourceCache` at `cache_budget()` |
| `PendingStage` never drained | RocksStore drains it after the write batch (`storage/rocksdb/src/lib.rs`) |
| RocksDB write buffers / block cache | `Options::default()`, 4 column families — a few hundred MB |

## The resource cache — investigated, REFUTED

Chunk 001 loads **243,537 resources**; `DEFAULT_CACHE_BUDGET_ENTRIES` is **250,000**
(`kernel/src/layer/storage.rs`), and `BoundedResourceCache` calls `moka`'s `max_capacity` with **no
weigher** — so the budget counts entries, not bytes, which the struct's own doc flags as open
*"pending Phase 12 workload data"*. The constant is sized for *"a ~1 KiB mean, a few hundred MB
resident"*; 6.7 GB / 243k gives ~27 KiB per entry. Compelling, and wrong:

    cache budget 250,000 → peak 15.66 GB
    cache budget  20,000 → peak 17.45 GB      (÷12.5, and peak went UP)

The cache is not the retention, and the derived ~27 KiB/entry and ~64× surface-to-resident figures
go with it — both assumed the 6.7 GB was cached resources.

The entry-vs-byte budget is still a genuine latent bug and should be fixed on its own merits
(`moka` takes a `.weigher`), but it is not what makes a reseed expensive. Filed, not urgent.

## Still open: the residual 9.41 GB

With arenas capped, WordNet alone still peaks at 9.41 GB and still does not release at idle
(9.41 → 9.05 over 30 s). Something beyond fragmentation holds or churns it, and **RSS cannot answer
that question** — the whole of this note is a demonstration of why. The instrument is a
`--features jemalloc-prof` build (`cli/src/main.rs`), which reports live bytes directly.

Two facts a live-heap profile has to account for: the chain-depth term (chunk 002 costs
**+8.7 GB / 111 s** against 001's **+6.7 GB / 55 s** for the same bytes), and the sheer duplication
in the input — measured on `wordnet-001.esl`, **242,938 entries carry only 46 distinct `cat`
shapes**, and 56% of the `cat` values are byte-identical to another entry's. Nothing interns them.

## Method note — the same error three times

Every wrong turn in this note has one shape: **a conclusion about liveness drawn from a measurement
of RSS**, or from a window in which the thing being claimed could not have shown up.

1. *"Memory is transient per commit, so smaller chunks cap it"* — from samples 20 s apart with
   chunks loading back-to-back, so no trough could appear even if one existed. Led to a
   `--split-bytes` knob that does nothing for memory.
2. *"Memory is retained per layer"* — from RSS staying flat across idle gaps. The gaps were the
   right instrument and the observation was sound; the inference was not. ~40% of it was the
   allocator, not the kernel.
3. *"It has plateaued"* — from four flat samples that were the gap between two load phases.

The measurements were fine each time. The claims outran them. What eventually worked was making each
hypothesis predict a number before the run: *cache is the cause ⇒ a 20k budget peaks near 540 MB*
(it peaked at 17.45 GB — refuted), *fragmentation is the cause ⇒ arena cap drops peak well below
15.66* (9.41 GB — confirmed, partially).
