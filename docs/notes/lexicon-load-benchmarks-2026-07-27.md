# Lexicon load + parse benchmarks — configuration and measurements (2026-07-27)

Measured during the reseed that baked in `lexicon:scope_bearing`. Every figure below is either read
from a run log or measured with the command shown. Figures that are *inferred* rather than measured
are labelled as such; §6 lists what was not measured at all.

**Unit discipline.** Sizes are `du -sb` bytes, with GiB and GB (decimal) both given. `du -sh` reports
rounded GiB and is not used as an input to any derived figure — a rounded GiB back-multiplied into a
bytes-per-resource figure was the source of one error in this session.

---

## 1. Hardware

| | |
|---|---|
| CPU | Intel Core Ultra 9 185H — 22 logical CPUs visible, L3 24 MiB |
| RAM | 31 GiB to the VM (`MemTotal` 32,583,844 kB) |
| swap | 8 GiB (`/dev/sdc`), swappiness 60 |
| storage | `/dev/sdd`, ext4, 1 TB virtual disk, 30% used |

**The CPU topology visible from inside is fabricated.** `lscpu` reports 11 cores × 2 threads,
homogeneous, with `MAXMHZ` empty and no distinct frequency tiers. The 185H is physically *hybrid*
(6 P-cores + 8 E-cores + 2 LP-E). WSL2 has flattened it, so **which core type executed a given
workload cannot be determined from inside the VM** — a material caveat for any single-threaded
figure, since P and E cores differ substantially.

## 2. Software

| | |
|---|---|
| kernel | `6.18.33.2-microsoft-standard-WSL2`; Hypervisor: Microsoft, VT-x, full virtualization |
| Docker | 29.6.1, storage driver `overlayfs`, runtime `runc`, root `/var/lib/docker` |
| Docker mode | **native in this WSL distro**, not a separate Docker Desktop VM (`/var/lib/docker` is visible from the distro) |
| resource limits | **none** — `docker-compose.yml` sets no `cpus`, `mem_limit`, or `deploy.resources`; the container sees all 22 CPUs and 31 GiB |
| toolchain | rustc 1.97.0 (2d8144b78), cargo 1.97.0; kernel image built with `--profile ci`, harness with `--release` |
| `.wslconfig` | sets only `networkingMode=mirrored`, `dnsTunneling=false` — WSL2 resource defaults apply |

**The I/O path is deep and both ends share one device.** RocksDB → docker volume → overlayfs →
ext4 → VHDX → host NTFS → physical disk. `/var/lib/docker` and the snapshot output directory are
both on `/dev/sdd`, so the snapshot copy-out reads and writes the same virtual disk. No figure here
characterises hardware storage; they characterise this virtualization stack.

## 3. Corpus size

Counted from the emitted ESL chain (`grep`/`sort -u` over `wordnet-chain/`, `umls-chain/`), which is
what the kernel actually loads.

| | WordNet | UMLS | closed-class |
|---|---|---|---|
| lexical entries | 465,554 | 6,168,891 | 160 |
| distinct surface forms | 176,690 | 4,606,180 | 80 |
| distinct sem targets | 138,736 | 2,372,308 | — |
| distinct sense keys | 201,204 | 2,372,308 | — |
| entries per form | 2.64 | 1.34 | 2.00 |

Totals: **6.63 M entries** over **4.78 M distinct forms**. No `sem` value is an inline `type_expr` —
every target is a `wn:` or `umlscui:` reference.

Sense keys exceed sem targets in WordNet only (201,204 vs 138,736): a WordNet sense key is
lemma-specific, so several keys share one synset. In UMLS the two are identical — one `umls:CUI` per
concept class.

WordNet sem targets by what they denote — these sum to 138,736 exactly:

```
noun    82,111      deg_a…            15,406   (adjective degree functions)
verb    15,831      pos_sem_a…        15,405   (positive-form sems)
adj      3,651      cmp_sem_a… +
                    cmp_attrib_sem_a…  6,332   (comparatives)
```

Adjective machinery therefore generates ~37 k derived sem targets — 27% of the WordNet total —
against only 3,651 bare adjective axioms.

**Unresolved discrepancy (recorded, not explained).** The WordNet importer self-reports
`465642 entries`; the emitted files contain **465,554** — a gap of **88** (0.019%). The base file
contains zero entries, so it is not a chunking artifact, and UMLS matches its self-report exactly
(6,168,891 both ways). Smaller gaps appear per category (noun 82,111 measured vs 82,115 implied;
verb 15,831 vs 15,841). The importer's counter and its writer disagree on WordNet; one of the two is
wrong. Not chased — it does not affect parsing.

## 4. Load throughput

### Headline

**5,227 resources/s** of *validated* commits — 627,182 resources (UMLS chunks 26+27) in a directly
measured 120 s window. Every resource passes the commit gate: Rule 22 reference integrity, `is_a`
class resolution, type-checking. This is not a bulk-import path.

Per-minute: **313,591 resources/min**. (Do not confuse this with the mean chunk size of 294,853 —
they are different quantities that coincided because the chunk rate was ≈1/min.)

### Whole reseed run

```
elapsed              1,989 s   (33 m 09 s)
resources loaded     9,192,394   =  WordNet 641,537
                                 +  UMLS base 128
                                 +  UMLS 29 chunks 8,550,729   (mean 294,853/chunk)
loads                34          =  4 WordNet + 30 UMLS (1 base + 29 chunks)
whole-run average    4,622 res/s   ← CONFLATES build + convert + load + copy; do not cite as throughput
store               3,316,537,602 B = 3.09 GiB = 3.32 GB
density             360 bytes/resource
```

Phases in order: release build (49 s) → kernel image (47 s) → clean volume → WordNet convert → UMLS
convert (MRSTY/MRCONSO/MRDEF scan) → WordNet load → UMLS load → snapshot copy.

*Inferred, not measured:* 9,192,394 resources at 5,227 res/s is 1,758 s of pure loading, leaving
≤231 s for build + convert + copy within the 1,989 s run. Against ~96 s of builds read from the log,
that leaves little room for early chunks to have been much faster, so the sampled rate is taken as
representative of the whole load. The sample was late in the run (largest store), and RocksDB does
not get faster as it grows.

### Cost asymmetry: classes vs entries

Same single core, clean store, measured separately:

| layer | resources | rate |
|---|---|---|
| WordNet base (classes) | 113,954 | ~28,000 res/s |
| WordNet entries (`wordnet-001.esl`) | 246,178 | **4,088 res/s** |

**Entries are ~7× more expensive.** They pay the felicity gate (`dcg::lexicon::gate_entry`): check
`⟦cat⟧ ≡ sem_type`, then check the `sem` actually inhabits `⟦cat⟧` — real EigenTT type-checking per
entry. Classes pay only reference resolution. This is where reseed wall-clock goes.

The 4,088 figure is not the same measurement as the headline 5,227: different chunk (WordNet vs
UMLS) and a nearly empty store versus one with ~7.5 M resources already committed.

### Alignment step — not a throughput figure

```
40,027 resources in 207 s
aligned snapshot: 2,851,439,106 B = 2.66 GiB = 2.85 GB
```

Dominated by staging 3.09 GiB in and copying 2.66 GiB out; the merge-layer load itself is seconds of
it. Useful only as wall-clock for "apply a merge layer" (≈3.5 min).

## 5. Parallelism actually used

**One thread. 1.05 cores of 22 — about 5% of the machine.**

Per-thread sampling inside the container, TID-keyed, over a 5 s window mid-load:

```
busy threads: 1
  tid 114   tokio-rt-worker   527 jiffies  =  105% of one core
aggregate: 1.05 cores
```

The container runs **27 threads**; exactly one does work. `docker stats` agrees: 104% mean while
active, brief peaks to 162% during RocksDB flush. (105% is jiffy-rounding over a 5 s window at
100 Hz; it is 1.0 core.)

Cause: `#[tokio::main]` with `tokio` features `"full"` gives a multi-thread runtime sized to the CPU
count, so 22 workers exist and idle. A `load` is one gRPC request → one task, and the work inside is
sequential — **no `rayon`, no `par_iter` anywhere** in `kernel/`, `crates/`, `storage/`, and **no
RocksDB parallelism knobs** set (`increase_parallelism`, `max_background_jobs` are defaults). The
only concurrency is RocksDB background flush/compaction.

So the 33-minute reseed ran on one core of a 22-CPU machine. Headroom ~21×, untouched. The felicity
gate of §4 is per-entry independent, which is where that headroom would be recovered.

**Keying matters when measuring this.** A first attempt joined thread samples on `comm`; all workers
share the name `tokio-rt-worker`, so the join produced a cross-product and reported 314% per thread.
Join on TID.

## 6. Parse benchmark (same snapshot)

`scripts/measure-parse-rate.sh`, WRN-helicase first page, CNL-v3, 62 units.

| | |
|---|---|
| snapshot | `wordnet-umls-aligned-2026-07-27-scopebearing` |
| config | `pos_prune=0 combinatory_core=0 attribution=0 context_window=0 SENSE_CAP=2 CELL_BEAM=64` |
| profile | `release` (load-bearing: a debug build overflows the stack in NbE readback and the harness reports phantom grammar gaps) |
| result | 62 units, **grammar-gap 0**, missing-lexeme 0, encoded 10, ambiguous 50, open 2 |
| | **total-readings 1078, total-skeletons 234** (sense× 4.61), expected-hits 44/45 |

## 7. Not measured

- **CPU utilisation and disk I/O during the reseed itself.** The reseed was not instrumented; §5 is
  a separate, later measurement on the same configuration. Whether the reseed was CPU-bound,
  I/O-bound, or client-stream-bound is **not established** — with 22 CPUs idle and one client, the
  last is plausible and unverified.
- **Bare-metal comparison.** Every figure includes the WSL2 + Docker + VHDX stack.
- **P-core vs E-core placement** — not observable from inside the VM (§1).
- **Swap attribution.** 4.1 GiB of swap was in use, but `sort -S 400M` passes over the 2.8 GB UMLS
  chain were run in the same session for §3. Attributing swap to the reseed would be unsupported.

## 8. Reproduction

```bash
# reseed — NOTE: --snapshot-dir takes an ABSOLUTE PATH, not a name.
# A bare name makes `docker -v <name>:/dst` a NAMED VOLUME rather than a bind mount:
# the store lands in a docker volume, the local dir stays empty, and the script still
# prints "reseed complete" with "size: 4.0K". Verified failure mode, 2026-07-27.
scripts/reseed-lexicon-db.sh --umls-all --snapshot-dir /abs/path/wordnet-umls-<date>

scripts/build-alignment-snapshot.sh \
  --base /abs/path/wordnet-umls-<date> \
  --out  /abs/path/wordnet-umls-aligned-<date> \
  --merges experiments/lexicon-align/merges-lemma-keyed.json

EIGENIUS_DB_SNAPSHOT=/abs/path/wordnet-umls-aligned-<date> \
  scripts/measure-parse-rate.sh --replay experiments/parsing/ranks/<recording>.json
```

**FIXED after this run** (`scripts/reseed-lexicon-db.sh`): `--snapshot-dir` now accepts either a path
or a bare name (resolved under `SNAPSHOT_ROOT`), and is made ABSOLUTE before it reaches `docker -v`.
The snapshot copy is then verified — `CURRENT` present and size ≥ 512 MiB — and the script EXITS 1
with recovery instructions instead of printing `size: 4.0K` and reporting success. The sibling scripts
(`add-layer-to-snapshot.sh`, `build-alignment-snapshot.sh`) already passed `readlink -f` paths and
were never affected.

For a lexicon change that does **not** touch `BOOTSTRAP_CHAIN`, test with a patch layer first and
skip the reseed entirely: `scripts/add-layer-to-snapshot.sh --base <snap> --out <snap>-x patch.esl`
keeps the base immutable and loads through the kernel, so the validator and commit gate still run.
Editing a chain layer (e.g. `ontologies/lexicon/lexicon-ontology.esl`) changes `current_manifest()`,
which sha256s every chain layer's source — all prior snapshots then fail to resume (`ManifestDrift`,
fail-closed) and a full reseed is unavoidable.
