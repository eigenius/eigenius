# Claims audit — kernel follow-ups

*2026-08-08. Produced by auditing every load-bearing claim in a SIGMOD/VLDB paper draft
against the kernel and storage trees, claim by claim, with `file:line` for each. This
note collects what the audit surfaced about the **code**, not about the paper.*

Provenance discipline: everything below was read in the source during the audit unless
marked *(swept, not re-read)*. Two items the sweep reported were wrong on closer reading
and are corrected in place — see A5 and the note under C2.

Severity: **P1** correctness or durability · **P2** misleading or silently wrong ·
**P3** hygiene, headroom, or deferred work.

---

## A. Correctness and durability

### A1 — Index writes are not atomic with the layer write · **P1**

`storage/rocksdb/src/lib.rs:548` calls `eigenius_kernel::layer::populate_layer_indexes(layer)`
**before** `run_blocking` opens the layer's `WriteBatch`. Each index then creates and
commits **its own** batch — `triple_index.rs:145-151`, `text_index.rs:417-423`,
`value_index.rs:144-150` — and none of those is sync-flagged. Only the layer batch is
(`lib.rs:654-657`).

A crash between the index batches and the layer batch leaves index entries pointing at a
layer that was never persisted.

The fix already exists as unused API. Every index exposes `extend_into_batch`, and the
**delete** path already does the right thing: `delete_layer` folds `drop_into_batch` for
all four indices into the single layer batch (`lib.rs:930-943`). The write path needs the
same treatment.

Note the batch comment at `lib.rs:550-556` promises atomicity — it is accurate about the
layer batch and misleading about the commit as a whole.

### A2 — Index population errors are silently discarded · **P1**

`kernel/src/layer/mod.rs:1165` and `:1177`:

```rust
let _ = layer.storage.triple_index.extend_layer(layer.id(), &borrowed);
```

A resource can be committed, pass the full gate, and be unfindable, with no signal
anywhere. Text indexing compounds it by swallowing unknown analyzers and silently
skipping non-string values (`query/text/indexing.rs:26-33`, `:113-116`).

At minimum these should be logged through the `kernel.commit.*` operation table; better,
an index-population failure should fail the commit, since the alternative is a store that
disagrees with itself.

### A3 — EigenQL fixpoint truncates silently · **P1**

`kernel/src/query/evaluate/mod.rs:126,137-153`. The stratum loop is bounded at 1001
passes; if it has not converged it simply exits, leaving a **partial relation and no
error**. A query returns fewer rows than it should and reports success.

This is a truncation, not a termination guarantee. It should raise a `QueryError`.

### A4 — n-ary `DEFINE` heads are write-only past column 0 · **P1**

`parse_define` uses `parse_variable_list()` (`parser.rs:132`), so `DEFINE Pair(?x, ?y)`
parses. `project_onto_head` (`evaluate/mod.rs:66-78`) dutifully stores columns `"0"` and
`"1"`. Nothing can ever read column `"1"`: `parse_pattern` accepts exactly one variable
between the parens (`parser.rs:481-484`), and `collect_candidates` reads only
`b.get("0")` (`evaluate/pattern.rs:145`).

So an n-ary derived relation is silently truncated to its first column at every use site,
and there is no relational typing of derived heads to catch it.

Two options: reject multi-variable heads at parse time (cheap, honest, immediate), or
implement n-ary consumption. The second is a **prerequisite** for any justification-graph
work, since `justifies(term, prop)` and `depends_on(prop, assumption)` are irreducibly
binary — see G1.

### A5 — Branch CAS is process-local · **P2**

`kernel/src/lattice.rs:344,359,757-775` — a process-wide `RwLock` snapshot gate plus a
per-branch `Mutex`, then a plain get/compare/put. Not a RocksDB transaction;
`TransactionDB` and merge operators are unused. Correct under the single-writer-process
assumption RocksDB already enforces, but the assumption is not stated at the API surface.
Worth documenting rather than changing.

---

## B. Documentation and comments that claim more than the code does

This is the defect class that made the audit necessary, and it is worth treating as a
class rather than as five unrelated bugs. It is *worse* than a `waits for M5` marker,
because a deferred-work note is honest about its state while a present-tense description
of unimplemented behaviour reads as fact.

| # | Claim | Reality |
|---|---|---|
| B1 | "seminaive" — **22 occurrences across 10 files** (`README.md`, `architecture-v0.3.md`, `d2-eigenql-specification.md`, `d59-…`, `implementation-plan.md`, two EigenQL guides, two website copies, and `evaluate/mod.rs:104`) | The evaluator is **naive**: `evaluate/mod.rs:140` re-evaluates each rule's full body against the full derived set every iteration. No delta relations exist |
| B2 | `FIBER … INTO` fires AutoOnLoad gates on commit (`docs/guides/eigenql/08-fiber-clauses.md:139`, `docs/guides/composition/05-chain-reinsertion.md:101-107`) | `server/query.rs:101-108` commits via `WithRetroactive` with `institutions: None`, **deliberately bypassing** them |
| B3 | `parser.rs:471` advertises `Derived relation: Name(variable, ...)` | Exactly one variable is accepted (`:481-484`). The comment claims the feature of A4 |
| B4 | Four comments assert index atomicity — `storage/rocksdb/src/lib.rs:158-180`, `triple_index.rs:23-28`, `text_index.rs:31-34`, `kernel/src/layer/text_index.rs:118-122` | See A1 |
| B5 | `layer/mod.rs:1047` — `LayerId` is the SHA-256 of the canonical CBOR encoding of resources | Superseded by the two-hash scheme at `:1208` (content) and `:1257` (position) |
| B6 | `layer/value_index.rs:29-30` — value entries are pre-populated at `LayerBuilder::build` | Build populates only when there is **no** persistent backend (`layer/mod.rs:1141-1144`) |
| B7 | `ontology/eigon_cbor.rs:138-141` — RFC 8949 §4.2 Core Deterministic Encoding | Deterministic, but not §4.2: `@id` is hoisted first (`:155-160`) and remaining keys sort by `BTreeMap<Iri,_>`, i.e. UTF-8 on the *unencoded* key, where §4.2.1 requires bytewise on the *encoded* key |
| B8 | `docs/notes/lexicon-load-benchmarks-2026-07-27.md` attributes the 7× lexical-entry ingest cost to `dcg::lexicon::gate_entry` | `gate_entry` is called only from importer binaries and tests, never the kernel commit path. The commit-path cost is **Rule 21** decoding and `check_infer`-ing the two `eigentt:TypeExpr`-ranged properties each entry carries (`lexicon:cat`, `lexicon:sem_type`). The *measurement* stands; the mechanism named for it does not |

**Suggested convention.** A check that is specified but not implemented should say so in
the imperative — "not implemented; see M5" — rather than describing intended behaviour in
the present tense. A cheap standing grep over `validation/`, `commit/`, `query/`, and the
guides for capability-claiming prose would catch recurrences.

---

## C. Dead or misleading code

**C1 — `cf_embed_cache` is dead. · P3** The column family is created on every DB open
(`storage/rocksdb/src/lib.rs:140,147`) and there is no other reference to it anywhere in
the tree. Its doc comment also claims a blake3 key, while the actual in-process embedding
cache uses SHA-256 (`program/embedding_cache.rs:25-30,65`). Either wire it or drop it.

**C2 — `RocksTripleIndex::stats()` reports operational counters only. · P3**
*(Correction: the audit sweep called this "hardcoded zeros"; on reading, `triple_index.rs:204-208`
carries a deliberate comment — "Live `triples` and `layers` would require a full scan to
count exactly. For RocksDB v1 we only report the cumulative operational counters".)* Not a
defect. Worth a doc note so nobody cites `triples`/`layers` from it.

**C3 — `extend_into_batch` is built and unused on the write path.** See A1.

---

## D. Deferred work with a named milestone

These are honest debt — they say what they are. Listed so the inventory is in one place.

- **D1 · P2 — Comorphism signature equality "waits for M5."**
  `validation/mod.rs:374-378` and `:423-425`. Commit-time Rule 15 checks only that the two
  format references resolve to the right classes and that `transformation` resolves to
  *some* resource. Nothing compares the transformation's type against
  `payload_type(export) → payload_type(import)`, though both are already parsed into
  `ExportFormatEntry`/`ImportFormatEntry` (`institution/registry.rs:81,92`).
  **The pattern already exists one door down:** Rule 18 does exactly this check for
  `MergeComorphism`, verifying the 3-binder shape and binder-type agreement against
  `target_class` (`validation/mod.rs:484-507`). Mismatches currently surface at dispatch
  instead of at commit.
- **D2 · P2 — `Exp::InstitutionInvoke` has no typing rule.** No arm in `check/mod.rs`; it
  falls through to the `CannotInfer` catch-all at `:1214-1216`.
- **D3 · P3 — `qc_which_axioms` returns `NotImplemented`**
  (`crates/eigenius-lean/src/institution.rs:270`). The QueryClass is declared and
  dispatch-bound; the handler is a stub. See G2 — this is the cheapest real win in the
  whole list, since nanoda already walks a proof term's dependencies.
- **D4 · P3 — `qc_consistency_check` returns `Undecidable`** for any non-trivial input;
  the propositional-fragment decision procedure is unbuilt (`reasoning.esl:358`).
- **D5 · P3 — Vector-sweep `TaskRecord` is in-process only** (`task/sweep.rs:27-33`,
  `task/sweep_registry.rs:28-31`, issue #59). `GetTaskStatus` cannot observe sweep
  progress and the state does not survive restart, so a client has no way to poll for
  when the vector index has caught up with a commit.

---

## E. Performance and operational headroom

**E1 · P2 — HNSW default connectivity is too low.** Measured 2026-08-08 (12-vCPU
i7-1265U under KVM; synthetic 64-dim clustered vectors, K=10, 100 queries):

| N | flat | m=16 | m=32 | m=48 |
|---|---|---|---|---|
| 1,000 | 0.071 ms | 1.000 | 1.000 | 1.000 |
| 10,000 | 0.476 ms | 0.722 | 0.927 | 0.890 |
| 50,000 | 4.882 ms | 0.618 | 0.797 | **0.949** |

The v1 schema default is `HNSW_M = 16` (`layer/index_discovery.rs:118`). At N=50k that
yields recall 0.62; m=48 yields 0.949 while a query still costs 0.064 ms against brute
force's 4.882 ms. **Raise the default, or make it size-adaptive.** The poor recall
reported previously is a default-tuning problem, not an algorithmic one.

**E2 · P2 — `ef` barely affects recall, and connectivity is non-monotonic.** Raising `ef`
from 16 to 256 moves recall from 0.596 to 0.618 at (N=50k, m=16) and not at all at m=48.
And m=32 beats m=48 at N=10k. Neither is how a correctly pruned HNSW behaves; both are
consistent with the builder omitting the Malkov–Yashunin §4.3 neighbour-selection
heuristic. This is the actual algorithmic defect behind E1.

**E3 · P3 — HNSW build is superlinear.** 0.21 s / 10.2 s / 368.7 s at m=16 for
1k/10k/50k — roughly 50× per 10× in N.

**E4 · P3 — Ingest is single-threaded and not by design.** No `rayon` or `par_iter`
anywhere in `kernel/`, `crates/`, or `storage/`, and RocksDB's parallelism knobs are at
defaults. Measured at 1.05 of 22 cores during a lexicon load. The felicity/type-check work
is per-resource independent, which is where the ~21× headroom is.

**E5 · P3 — RocksDB is effectively untuned.** `storage/rocksdb/src/lib.rs:194-205` sets
three options: `create_if_missing`, `create_missing_column_families`, LZ4. No
`BlockBasedOptions` at all, so **RocksDB's own Bloom filters are not enabled** and point
lookups pay full per-level SST probes. (The hand-rolled per-layer Bloom filters in
`layer/bloom.rs` are a different mechanism — they skip *layers* during a chain walk and do
nothing for the KV lookup underneath.) No block-cache sizing, write-buffer sizing, or
`max_background_jobs`.

**E6 · P3 — The Bloom cache is unbounded in the production configuration.**
`layer/cache.rs:431` (`MemoryBloomCache`) is used by `with_persistent_bounded`
(`layer/storage.rs:222`), sitting beside a resource cache carefully bounded at 250,000
entries. Bounded eviction is an unlanded TODO at `cache.rs:406-407`. *(swept, not
re-read.)*

**E7 · P3 — No benchmark harness.** `criterion` is declared at `Cargo.toml:79` but no
crate depends on it and there are zero `[[bench]]` targets; all benchmarks are `#[ignore]`d
integration tests timing with `std::time::Instant`. No CI performance tracking in
`.github/workflows/`. Every number in `docs/notes/` is a hand-run measurement, which is
why this audit could not tell whether figures still held without re-running them.

---

## F. Security posture

**F1 · P2 — No authentication, authorization, or transport security.** A grep across
`kernel/src/server/` for authn/authz/TLS terms returns zero hits. `kernel/src/capability/`
is *institution backend registration*, not an authorization system
(`capability/mod.rs:15-21`), and `ExecutionMode::{ReadOnly, ReadWrite}`
(`context/mod.rs:48-53`) is a process-local write guard, not a principal-based permission.

This is defensible for a kernel deployed behind a trusted boundary, but it is currently
implicit. It should be stated as a deployment constraint somewhere a reader will find it,
because the word "capability" in the module tree actively suggests otherwise.

---

## G. Extensions surfaced (not defects)

**G1 — n-ary derived relations.** See A4. Prerequisite for G2.

**G2 — Implement `qc_which_axioms`.** Cheapest real win in this document. nanoda already
walks a proof term's transitive dependencies; the QueryClass is declared and
dispatch-bound. Unlocks "what does this conclusion actually rest on."

**G3 — Semiring provenance over `JustifiedBy`, weighted by `WitnessCategory`.** Annotate
derived propositions with support polynomials over base facts (Green, Karvounarakis &
Tannen, PODS 2007), then weight by the `Declared | Observed | Derived | Verified` grade —
which derives `Ord`, so support quality is computable. Answers *"which unwitnessed
assertion carries the most verified conclusions?"* — load-bearing from the polynomial,
questionable from the grade. Requires G1 and benefits from G2.

**G4 — A search tier with `Declared` admission.** Merge auto-resolution (D20 §11.2),
comorphism synthesis (declined on Fact 14.9), and `qc_entailment_query` are the same
feature: the system proposing something the universal property does not force. Each was
declined separately. The admission policy already exists and is unconnected — a proposal
is `Declared` and promotes only when witnessed. One policy would be better than three
refusals.

**G5 — `BranchMergePolicy` (D20 §11.1).** Per-conflict-kind default strategies as chain
resources, with contributor allowlists, so CI bots can auto-resolve known-safe conflicts.
Deferred for want of usage data, not theory.

**G6 — Semilattice-typed properties.** For a property whose value type carries a lawful
join — set-valued, counters, LWW registers — the merge is canonical and needs neither a
witness nor a human. Worth noting that `lattice.rs` is named for the layer DAG and
contains no join or meet, while this is the place a real join would eliminate conflicts.

**G7 — Prove merge-witness associativity (D20 §11.7).** "Likely yes… but not formally
established," and sequential-merge automation leans on it hardest. It is a proof
obligation, and Lean 4 already runs in-process as a verification institution. Related open
question: comorphism composition was *declined* on Fact 14.9 (composing left adjoints
yields isomorphism, not equality) while witness composition is *assumed* fine. Same shape,
two answers — worth arguing rather than assuming.

**G8 — Magic-set transformation for `DEFINE`.** Gives bottom-up evaluation the
query-directedness of top-down SLD resolution while keeping guaranteed termination — the
standard answer to "why does a reachability query materialize the whole relation."
Alongside semi-naive evaluation (A-adjacent) and a hash-set replacement for the
`Vec::contains` dedup at `evaluate/mod.rs:144`.

---

## Suggested order

1. **A1, A2** — silent corruption and silent index loss are the only items here that can
   produce a store that disagrees with itself.
2. **A3, A4** — silent wrong answers. Both are small; A4 can be a parse-time rejection
   today and a real fix later.
3. **B1–B8** — a documentation sweep, cheap, and it is what makes every future audit
   cheaper. B2 and B3 are the dangerous ones because they describe behaviour that does not
   exist.
4. **E1** — a one-line default change that moves recall from 0.62 to 0.95 at 50k.
5. **D1** — closes the last commit-gate hole, with Rule 18 as the working template.
6. **E2, E5, E4** — the algorithmic and tuning headroom, in that order.
7. **G1 → G2 → G3** — the extension path, in dependency order.

## Method note

Nothing here required new tooling: it came from reading the code against a set of written
claims and checking each one. The classes that recurred — docs claiming more than the code
(B), and a permissive generic path beside a strict specific one where only the specific
one is wired (A1, A4, D1) — are both invisible from inside any single function and both
cheap to find once you are comparing against an external statement of intent. A periodic
pass of that kind, against the design docs rather than a paper, would likely be worth its
cost.
