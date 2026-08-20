# Kernel OOM during a notebook session — open investigation

**Status:** CAUSE FOUND `2026-08-20` — retroactive validation on a redefining load over the
lexicon chain. Triggers removed at the call sites; the underlying unbounded scan is still open.

## The observation

A kernel serving the full lexicon chain (`wordnet-umls-aligned-2026-08-20`, ~7.6M resources) was
killed by the **host** OOM killer during an interactive notebook session:

```
oom-kill: constraint=CONSTRAINT_NONE, global_oom, task=eigenius
Out of memory: Killed process 794089 (eigenius)
  total-vm:30203128kB  anon-rss:27788664kB
```

27.8 GB resident on a 31 GB host. `docker inspect` reports `OOMKilled: false` and exit 137 — that
flag tracks the container's *own* cgroup limit, and the service sets none, so a global OOM shows up
only as the signal.

The kernel logged nothing for the ~7 minutes before the kill. The session included: a D71
formalize cell run, several failed EigenQL queries, and layer views opened on small
notebook-result layers.

## CAUSE FOUND (`2026-08-20`) — retroactive validation on a redefining load

**Reproduced twice, deliberately the second time.** `commit.retroactive.start` is the last line in
the kernel log before both kills (27.8 GB, then 27.2 GB).

Loading a layer that REDEFINES existing resources triggers Rule 22's retroactive validation across
the chain. On the 7.6M-resource lexicon chain that scan is what allocates ~27 GB. This is the
**already-known open issue** recorded against the reference-integrity work: retroactive validation
was scoped to redefinitions, and a 7.6M-chain retroactive-scan OOM was fixed once by gating the
property-key case — but the full-chain-resident OOM was left open, and this is it.

Both observations fit:

- The original session loaded `encoding.esl` and then `claim-kind-alignment.esl`, and the alignment
  file REDECLARES each discourse-kind class (that is its whole mechanism — it adds lexicon parents).
  Five redefinitions, full-chain retroactive scan, dead kernel.
- The second was `demo/prose-to-formulas-v2/run.sh` loading `encoding.esl` after it had been moved
  into the bootstrap chain, which makes *every* resource in it a redefinition.

**Fixed at the call sites** by not re-loading what already arrives: `encoding.esl` is bootstrapped
and the alignment rides in the snapshot, so neither the demo nor the notebook loads them any more.
That removes the trigger; it does not remove the hazard. **Any** load that redefines a resource on a
lexicon-scale chain can still do this, and there is nothing between a user and that outcome — no
cap, no diagnostic, no refusal. The kernel dies with no log line.

What remains open is therefore the underlying scan, not the incident: retroactive validation needs
to be index-driven (the fix pattern `typed_resource_iris` established for `build_axiom_env` and
`draws_from_layer`), or bounded with a typed refusal when the affected set is too large.

## What was RULED OUT on the way (measured, not argued)

Peak RSS on the same chain, same build, sampled with `docker stats` at 2–3s intervals:

| operation | peak |
|---|---|
| idle baseline | 2.15 GiB |
| 3-`MATCH` join query over `enc:` classes | 2.56 GiB |
| formalize, 1 sentence, live proposers | 2.44 GiB |
| formalize, 3 sentences, live proposers + anaphora + claim landing | **2.43 GiB** |

And by inspection:

- **Class patterns ARE index-driven.** `collect_candidates` takes the Phase-14h indexed path when
  the class is bound and `is_a` is indexable. Probed on the persisted chain: `core:is_a` and
  `core:subclass_of` both resolve with `data_type: resource_array`, so `is_indexable_predicate` is
  true and the `iter_all_resources` fallback never fires.
- **`LayerTopology` depth is bounded correctly.** `walk_layer` recurses only when
  `max_depth == 0 || depth + 1 < max_depth`, so the notebook's `maxDepth: 1` walks the root layer
  ONLY and never reaches the lexicon parent.
- **The per-fetch taxonomy count is streaming.** `taxonomy_counts_by_layer` accumulates a
  `HashMap<LayerId, (u64, u64, u64)>` and never collects subjects, as its doc comment claims.

## The one live hazard found, and why it is probably not this

`LayerStackView` fetches `{ rootLayer, maxDepth: 1, includeResources: true }`, and the
`include_resources: true` branch emits **one proto node per resource in that layer** with no cap.
Drilling into a lexicon layer would allocate on the right order. The operator reports opening only
small notebook-result layers, so this is a real unbounded path but likely not the observed kill.
It is worth capping regardless (a typed refusal or a bounded page with an explicit `truncated`
marker — never a silent cap, D62's rule).

## What would actually settle it

Static analysis is exhausted; the same wall as
[reseed-oom-memory-investigation.md](reseed-oom-memory-investigation.md), whose §6 next-action is
the tool to use here too:

1. **jemalloc heap profile.** The kernel supports `--features jemalloc-prof` (referenced in
   `docker-compose.yml`'s `MALLOC_ARENA_MAX` note). Build the image with it, run a notebook session,
   `jeprof` the dump, and name the owner of the ~25 GB.
2. **Catch it in the act.** Sample container RSS throughout a session so the growth curve points at
   the interaction, rather than reconstructing from a corpse. The kill left no log line.

## Note on method

Four causes were proposed and eliminated in sequence before this note was opened. Each was
plausible and unrefuted at the moment it was proposed, and "plausible and unrefuted" was repeatedly
mistaken for "found it". Only the measurements moved anything. Whatever is next: measure first.
