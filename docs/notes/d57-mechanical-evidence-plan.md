# D57 — encoding the process as mechanical evidence

> **Status (2026-06-20): Levels 1 + 2 implemented.** Level 1: the mechanical evidence
> is in `crates/eigenius-schemaorg/tests/output_validates.rs` (input-pinned → convert →
> kernel `Validator` 0 errors + property checks); m3 composes **two content-hashed
> Observed artifacts** (`obj:gen_input` / `obj:gen_output`, kernel-emitted
> `IsObservedAs`); m4's partition is independently recounted. **Level 2 (D60):** the
> `GeneratorConforms` leg is now genuinely **Derived** — `eigenius run` dispatches the
> generator through the kernel's generic `oci` tool runtime
> (`eigenius-schemaorg-worker` in a pinned image; kernel-tracked `runtime:BuildRecipe`),
> committing `generate_result` + a `ProgramTrace → IsDerivedAs(generate_result,
> GeneratorConforms)` that `concl_generator` discharges via `derived(...)`. So m3 is now
> **Observed (artifacts) + Derived (conformance)** — the kernel attests the run, no new
> institution. Verified: `cargo test -p eigenius-oci --test oci_e2e -- --ignored` (real
> container conversion → report carries the proposition) + `tests/d57_chain_validates.rs`
> (`concl_generator` Holds via `derived(...)`); the live compose run is the deployment
> confirmation (`experiments/objectives/d57-schema-org/programs/README.md`). See
> [d60-native-runtime-and-tracked-env-build.md](../design/d60-native-runtime-and-tracked-env-build.md).
> Witnessing surfaced two findings (F1 transitive-closure bug, fixed; F2 four
> genuinely-open enumerations, accounted) — see
> [d57-mapping-decisions.md](d57-mapping-decisions.md).


> The chain (`experiments/objectives/d57-schema-org/chain/`) records the objective's
> thesis + milestones but discharges m1–m4 as **Declared** conclusions
> (`declared()` over a rationale). The research, derivations, and decisions captured
> in [d57-mapping-decisions.md](d57-mapping-decisions.md) are prose, not graded
> witnessed propositions — and several claims have **stronger mechanical evidence
> available (Derived / Verified) that we left on the table**. This plan says, for
> each step, what witness we *can* provide and how to encode it.
>
> Governing principle (the `reasoning` skill): *don't assert — witness.* A claim is
> Declared only when no run/check/proof can establish it. We over-used Declared.

## 1. Mechanical-evidence sources Eigenius actually offers

| Mechanism | Grade | What it witnesses |
|---|---|---|
| `ingest:PinnedExternalFile` + `content_hash` | **Observed** | the input (V30.0) and the deterministic output exist, byte-identified |
| Loading a layer (commit accepted, 0 errors, no `Fails`) | **Verified** (structural) | the resources are well-formed / Expressible; a rejected load is a fail-closed finding |
| EigenQL query returning the expected result | **Verified** (check) | a correspondence rule or a completeness claim, evaluated over the *real* output |
| Generator run *through the kernel* (D56 wrapped-program / `RunRuntimeScript` — no new institution) → `DerivedResource` + `ProgramTrace` + `IsDerivedAs` | **Derived** | the output was computed by an attested program from a content-hashed input |
| Re-run → identical output `sha256` | **Verified** | determinism / reproducibility |
| `ReasoningSentence` that `Holds`; well-posedness gates | **Verified** | kernel-checked reasoning |
| `reference:Citation` with a resolvable source | Declared (anchored) | an imported external claim (e.g. the conformance stance) |

## 2. Per-proposition evidence map

What each intermediate claim *currently* is, the strongest witness *available*, and
the grade it could reach.

| Proposition (intermediate step) | now | Mechanical evidence available | Achievable grade |
|---|---|---|---|
| Input is the pinned schema.org V30.0 vocabulary | prose (MANIFEST) | `sha256:0f0c97a4…` → `PinnedExternalFile` | **Observed** |
| schema.org is recommendation-based (the conformance fact #4) | Declared (note) | `datamodel.html` verbatim quote → `reference:Citation` carrying the claim | Declared, **anchored** |
| The generator emits the `urn:schema_org:` vocabulary | Declared (`concl_generator`) | (L2) run through kernel → `DerivedResource`+`ProgramTrace`; (L1) output `content_hash` + the load | **Derived** (L2) / Observed+Verified (L1) |
| The output loads + validates (Expressible) | rationale prose | the actual load: 2114 resources, **0 errors** | **Verified** (structural) |
| Conversion is deterministic | asserted | re-run, compare output `sha256` (byte-identical) | **Verified** |
| Enumeration closed set is enforced | prose in `concl_probe` | load a member (accepted) + a non-member (`AllowedValueViolation`) | **Observed** (pos+neg) |
| `domainIncludes`→`recommends`, no `core:domain` (#9) | prose | query: `count(properties with core:domain) = 0`; classes carry `recommends` | **Verified** (query=expected) |
| Every enumeration-ranged property has `allows_only` | implied | query over the output | **Verified** |
| Round-trip identity (`source_irl` ↔ `@id`) (#13) | Declared | query: every `source_irl` reverses to its `@id` via the prefix substitution | **Verified** |
| The cut is complete (m4) | Declared (`concl_cut`) | partition check: `mapped ∪ folded ∪ excluded ∪ residual = |schema: nodes|` | **Derived** + **Verified** |
| Each correspondence rule (URL→format, enum→allows_only, …) | prose | the crate's 9 unit tests (one per tier) | **Derived** (test) |
| Bugs #16 (datatype seeding) / #17 (ref filtering) | decision log | the **failed load** before the fix → fix → load succeeds + regression test | **fail-closed finding** (Observed `Fail` → Verified fix) |

## 3. Two levels of rigor

- **Level 1 — no new infrastructure (do now).** Pin input+output as content-hashed
  `ingest` resources (Observed); **load the generated 2114-resource ontology onto
  the objective branch** (the commit is the Verified-structural witness); author the
  **EigenQL property-checks** (§2 rows) as `ReasoningSentence`s / `QueryClass`
  verdicts (Verified); record the two bugs as fail-closed findings; anchor the
  conformance claim (citation). This alone moves m2/m3/m4 from *Declared assertion*
  to *Observed artifact + Verified checks*.
- **Level 2 — the gold standard (a build).** Run the generator as a program *through
  the kernel* — the **D56 wrapped-program path** (`RunRuntimeScript`), the same
  mechanism WRN uses for its wrapped-R programs. **No new institution is needed**: the
  kernel dispatches the program over a content-hashed input and emits a real
  `DerivedResource` + `ProgramTrace` → `IsDerivedAs` over the content-hashed output.
  Then `concl_generator` discharges `GeneratorConforms` (and the artifact legs) via
  `derived(output, …)` instead of `declared(...)`. This is the genuine "the kernel
  attests the computation" witness.

## 4. Encoding the *process*, not just the conclusions

Beyond upgrading grades, add the missing process nodes so the chain mirrors the
decision log:

- **Anchors** — the two schema.org docs as `reference:Citation`s (the conformance
  claim, the distribution facts); the pinned input as an Observed resource. These
  are the *premises* the discipline builds on.
- **Derivations** — the closure computations (DataType set, enumeration set,
  correspondence partition) are computed by the generator and **checkable by query**
  over the output; encode the load-bearing ones as Verified checks rather than
  trusting the prose.
- **Decisions** — correctly Declared (they are design choices: entity-first,
  scope, recommends-not-domain), but each should **link to its motivating evidence**
  (the union rule → the probe; recommends-vs-domain → the over-enforcement it would
  cause) so the *why* is on chain, not only the *what*.
- **Findings (fail-closed)** — the two generator bugs and the earlier D59
  ResourceRef bug were each a *load that Failed → fix → load that Holds*. That
  failing-then-passing trace is the strongest evidence the discipline works; record
  it (the `recompute-findings.md` pattern, generalized) instead of only narrating it.
- **Decompose m3** — `GeneratorProduces` is really a conjunction: *output loads*
  (Verified) ∧ *deterministic* (Verified) ∧ *tier counts as reported* (Derived) ∧
  *no `core:domain`* (Verified) ∧ *enum closed set enforced* (Observed). Each
  sub-proposition gets its own witness; `concl_generator` composes them (the way the
  thesis composes m1–m4).

## 5. Recommended order

1. **Level 1 checks first** (highest value / least cost): pin input+output, load the
   generated ontology onto `obj-d57`, author the four EigenQL property-checks
   (no-domain, enum-allows_only, round-trip, cut-completeness) + the
   member/non-member enforcement loads, and re-discharge m2/m3/m4 citing these
   Verified/Observed witnesses (decomposed per §4).
2. **Findings + anchors**: record the two bugs as fail-closed findings; add the
   `datamodel.html` conformance citation.
3. **Level 2** (when warranted): wrap the generator as a kernel-dispatched program
   for a true `Derived` witness — the WRN wrapped-R pattern applied to the generator.

The meta-point: this is the difference between an objective whose milestones are
*asserted* and one whose milestones are *witnessed*. The infrastructure to witness
most of D57 already exists (loads, queries, content hashes, the gate verdicts) — we
just declared instead.
