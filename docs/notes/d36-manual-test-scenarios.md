# D36 manual test scenarios

Pre-close-out walk-through for the Phase 15 / D36 merge-resolution UX in the notebook. Each scenario is self-contained: paste the ESL into cells, follow the click steps, check the expected result. All scenarios assume the stack is running locally (orchestrator + kernel) and the notebook is open at `http://localhost:8080/notebooks/`.

The scenarios share a small ontology stub on `main` to avoid re-typing the same boilerplate. Run **Scenario 0** first.

---

## Scenario 0 — Shared ontology stub on `main`

**Goal.** Seed `main` with classes / properties that subsequent scenarios will conflict on.

**Branch.** `main` (the default branch on a fresh database).

**Steps.**

1. Open a fresh notebook (toolbar → New notebook).
2. Add an ESL cell containing:

   ```esl
   namespace core    = "urn:eigenius:core";
   namespace project = "urn:project";

   class project:Profile {
       description = "Patient profile, references Patient by IRI.";
       requires project:profile_for;
   }

   property project:profile_for : core:resource {
       description = "Reference to a Patient resource.";
   }

   property project:weight : core:float {
       description = "Patient weight in kilograms.";
   }

   // Placeholder so subsequent scenarios can reference Patient
   // (Scenario 6's witness in particular needs Patient to exist
   // in the chain at commit time). Branches re-declare with
   // different descriptions to drive the IriCollision conflict.
   class project:Patient {
       description = "Placeholder Patient class — branches re-declare.";
   }
   ```
3. Run the cell. Expect "Loaded 4 resources" in the cell footer; the commit-status badge shows `TRIVIAL_MERGE` or `BRANCH_ADVANCED`.
4. Note the resulting branch tip layer id (visible in the Topology panel) — call it **L₀**. You'll need it as the divergence point.

---

## How to manufacture a conflict

Every D36 scenario needs **two divergent layers** off the same ancestor. The pattern is:

1. Create a feature branch `feat-A` starting at L₀ (Branches panel → **New branch** → start from `main` → name = `feat-A`, **Switch to branch** = on).
2. Run a cell on `feat-A` that defines the conflicting IRI one way. Note the new tip — **L_A**.
3. Switch back to `main` (Branches panel → click `main`).
4. Create a second feature branch `feat-B` starting at L₀ (Branches panel → **New branch** → Start from → **Specific layer id** → paste L₀'s hex). Switch to `feat-B`.
5. Run a cell on `feat-B` that defines the *same* IRI a *different* way. Note the tip — **L_B**.
6. The two branches have diverged. There are now two entry points into the resolution flow:
   - **Explicit merge:** open the Merge rail, source = `feat-B`, target = `feat-A` (or vice versa), click **Refresh preview** → expect *Predicted: conflict*; click **Merge** → expect the warning result block with the **Resolve conflicts** button.
   - **Cell race:** open the notebook in a second tab, both pointed at `feat-A`. Have tab 1 commit a conflicting cell while tab 2's tip is stale; tab 2's next commit hits `NEEDS_WITNESSED_MERGE` and the cell badge shows **Resolve in Merge rail**.

The explicit-merge path is the easier one to script and is used as the default in every scenario below. Pick the cell-race path only for **Scenario 8** (cell auto-clear) and **Scenario 9** (race recovery).

---

## Scenario 1 — IriCollision resolved by Rename

**Tests.** Strategy picker · `RenameEditor` · cascade ack · success card.

**Branch setup.** Two divergent branches off L₀ (see recipe above).

**Cell on `feat-A`:**
```esl
namespace core    = "urn:eigenius:core";
namespace project = "urn:project";

class project:Patient {
    description = "Medical-records Patient.";
    requires project:weight;
}
```

**Cell on `feat-B`:**
```esl
namespace core    = "urn:eigenius:core";
namespace project = "urn:project";

class project:Patient {
    description = "Billing-system Patient.";
}
```

**Resolution.**
1. Merge rail → source `feat-B`, target `feat-A` → **Refresh preview** → expect *Predicted: conflict* listing `urn:project:Patient`.
2. Click **Merge** → result block shows *Conflict — target unchanged* with **Resolve conflicts** button. Click it.
3. Picker mounts with one conflict card (kind: `IriCollision`).
4. Pick **Rename**. Side = B. New IRI = `urn:project:billing:Patient`. Preview button enables.
5. Click **Preview cascade**. Expect one `OrphanedReference` item (`project:Profile → project:Patient` at `project:profile_for`) because the shared stub references `project:Patient`. *(If your Profile resource list is empty, the cascade may be empty — that's fine.)*
6. Tick the checkbox. Commit button enables. Click **Commit merge**.
7. Expect success card: *Merge committed*. New merge-layer id shown.
8. Switch to `feat-A` in the Branches panel — confirm the tip advanced.

---

## Scenario 2 — PropertyDataType resolved by KeepOne

**Tests.** `QuotientEditor` · winner selection.

**Branch setup.** Two divergent branches off L₀.

**Cell on `feat-A`:**
```esl
namespace core    = "urn:eigenius:core";
namespace project = "urn:project";

property project:weight : core:float {
    description = "Weight in kg.";
}
```
*(redeclaring the property carries the new data_type)*

**Cell on `feat-B`:**
```esl
namespace core    = "urn:eigenius:core";
namespace project = "urn:project";

property project:weight : core:integer {
    description = "Weight in grams as an integer.";
}
```

**Resolution.**
1. Drive the explicit-merge path. One conflict card; kind = `PropertyDataType`.
2. Pick **Keep one**. Winner = A. Preview cascade → likely empty (no downstream references in the stub). Acknowledge if any items appear. Commit.
3. Expect success.

---

## Scenario 3 — PropertyDataType: KeepNeither

**Tests.** `QuotientEditor` keep-neither path · ancestor-fallback / tombstone semantics.

**Branch setup.** Same as Scenario 2 (re-run the recipe from L₀).

**Resolution.** Drive to the picker, choose **Keep neither** in the QuotientEditor. Preview → commit. Verify with a follow-up cell on the merged branch that `project:weight` resolves to *the ancestor's `core:float` definition from L₀* (ancestor fallback path; tombstone path only triggers when the ancestor had no body).

---

## Scenario 4 — KeepBoth greyed-out rationale (D36 §15.5)

**Tests.** Applicability-greyed UI · inline rationale copy.

**Branch setup.** Same as Scenario 2.

**Steps.**
1. Open the resolution flow.
2. Locate the **Keep both** radio in the strategy list.
3. Expect: radio **disabled** with the generic inline caption *"Not applicable to this conflict kind."* — D36 §15.5's kind-specific verbiage (e.g. "data_type is single-valued…") was deferred and is not in v1; the longer story is one click away via the **Strategy reference** help link.
4. Do not commit — close with **Cancel**.

---

## Scenario 5 — SubclassConflict resolved by Restructure

**Tests.** `RestructureEditor` · new-parent definition · multi-select.

**Branch setup.**

**Cell on `main` first** (extend the stub):
```esl
namespace project = "urn:project";

class project:Mammal { description = "Warm-blooded vertebrate."; }
class project:Reptile { description = "Cold-blooded vertebrate."; }
```
Run on `main`, note the new tip → **L₀′**. Use this as the new divergence point for the two feature branches.

**Cell on `feat-A`:**
```esl
namespace project = "urn:project";

class project:Dog : project:Mammal {
    description = "Domestic dog (mammal lineage).";
}
```

**Cell on `feat-B`:**
```esl
namespace project = "urn:project";

class project:Dog : project:Reptile {
    description = "Dog (reptile lineage — for testing only).";
}
```

**Resolution.**
1. Drive the explicit-merge path. One conflict card; kind = `SubclassConflict`.
2. Pick **Restructure**.
3. In the editor: affected class = `urn:project:Dog`. New parent IRI = `urn:project:Animal`. Toggle **Introduce new parent class** = on.
4. Fill the sub-form: short_name = `Animal`, description = `Common parent for Mammal and Reptile.`.
5. Classes-under-new multi-select: tick `urn:project:Mammal` and `urn:project:Reptile`.
6. Toggle **Re-parent affected class under new parent** = off (Dog keeps its existing parent through the lineage; the test exercises the toggle being available, not its semantics).
7. Preview → ack → Commit. Expect success.

---

## Scenario 6 — Witness, happy path (D37)

**Tests.** D37 PRs 2+3 end-to-end: `merge_comorphism` ESL declaration → commit-time validator (Rule 18 + Rule 19) → `WitnessEditor` Combobox of applicable witnesses → resolver's apply-time class check → successful merge layer.

**Kernel status.** D37 PR 1 wired apply-time class enforcement (`resolve_merge_comorphism` early-rejects on `merge_target_class` mismatch). PR 2 added the ESL surface (`merge_comorphism`, `lambda`, `pi`, `for`) plus commit-time validators for `MergeComorphism` shape (Rule 18) and standalone Lambda well-typedness (Rule 19, NbE-backed). PR 3 turned the `WitnessEditor` into an EigenQL-driven Combobox of applicable comorphisms. PR 4's notebook scenario closes the loop.

**Witness vs. other strategies — what kind of conflict it fits.** Witness operates at the **instance** level. `merge_comorphism for A` means "applies when both sides of an IriCollision are instances of class A." The WitnessEditor's Combobox query filters on the conflict's `is_a[0]` — i.e., the class the conflicting instances belong to.

- **Instance-level conflicts** (this scenario): both branches commit `resource project:patient_42 : project:Patient { … }` with different field values. The conflict's `is_a[0]` is `urn:project:Patient`, so `merge_comorphism for project:Patient` resolves.
- **Class-level conflicts** (Scenario 1): both branches re-declare `class project:Patient`. The conflict's `is_a[0]` is `urn:eigenius:core:Class`, so witnesses authored `for project:Patient` don't surface in the Combobox. Class-level conflicts are usually resolved structurally via Rename or Restructure.

**Branch setup.**

1. Create branches `s6-a` and `s6-b`, both off `d36-base`. The fixture already commits `class project:Patient` + `property project:weight` — that's everything needed for Patient instances.
2. On `s6-a`, run the **branch-A** cell:

   ```esl
   namespace project = "urn:project";

   resource project:patient_42 : project:Patient {
       project:weight = 75.0;
   }
   ```
3. Switch back to `main`, create `s6-b` off `d36-base`, switch to it. Run the **branch-B** cell — same IRI, different weight:

   ```esl
   namespace project = "urn:project";

   resource project:patient_42 : project:Patient {
       project:weight = 80.0;
   }
   ```
4. Switch back to `s6-a` (the merge target). Run the **witness cell**:

   ```esl
   namespace core    = "urn:eigenius:core";
   namespace project = "urn:project";

   // D37 §9.1 — take Branch B's body unchanged.
   merge_comorphism project:patient_take_b for project:Patient {
       (a, b, opt) => b
   }
   ```

The witness cell compiles to two chain resources:

- A synthesised standalone `Lambda` at `urn:eigenius:auto:lambda:<sha256>` carrying its declared Pi-type (`pi a : Patient, b : Patient, opt : Option<Patient> => Patient`) and `parameter_type` annotations on each binder.
- A `MergeComorphism` at `urn:project:patient_take_b` with `merge_target_class = urn:project:Patient` and `merge_transformation` pointing at the synthesised lambda.

The commit-time validators run on both. Reject conditions: missing `merge_target_class`, transformation isn't a 3-binder Lambda chain, body fails NbE check against the declared Pi-type. The witness lives on `s6-a` and is reachable at merge time — `resolve_merge_comorphism` searches both branch tips' contributions, not just the ancestor (per `merge.rs:1118-1126`).

**Resolution.**

1. Open the Merge rail. Source = `s6-b`, target = `s6-a`. Drive to the picker. One conflict card; kind = `IriCollision` on `urn:project:patient_42`.
2. Pick **Witness**. The `WitnessEditor` extracts the conflict's class from `branchABodyJson` (`urn:project:Patient`) and queries the chain for `MergeComorphism` resources matching.
   - **Expected**: `patient_take_b` appears as a single option in the Combobox.
3. Select `patient_take_b`. The Combobox commits its IRI as the `WitnessStrategy`'s `comorphismIri`.
4. Click **Preview cascade**. Expect: cascade may be empty.
5. Click **Commit merge**. Expect success card. The merged `urn:project:patient_42` is **branch B's body** (weight = 80.0) — the witness's `Var "b"` returned the second argument unchanged.
6. Switch to `s6-a` in the Branches panel — confirm the tip advanced. Verify with an EigenQL cell:

   ```eigenql
   MATCH "urn:project:Patient"(?p) {
       "urn:project:weight": ?w
   }
   RETURN ?w
   ```

   Should return `80.0`.

**Negative path — wrong class.** Author a second witness `merge_comorphism project:visit_take_b for project:Visit { (a, b, opt) => b }` (requires a `class project:Visit` to exist; add `class project:Visit { description = "A visit."; }` to a setup cell first). For the Patient conflict, the Combobox filters by class and should NOT surface `visit_take_b`. Manually pasting its IRI into the free-form fallback lands on `MergeComorphismWrongClass` from the resolver.

**Negative path — unresolved IRI.** Pasting an IRI that doesn't resolve to a `MergeComorphism` (e.g., `urn:project:nonexistent`) still surfaces `MergeComorphismNotFound` → `MALFORMED_RESOLUTION`. The Combobox prevents this in the happy path; the free-form fallback (for conflict kinds without a target class) preserves the error surface.

---

## Scenario 7 — Malformed Rename (collision)

**Tests.** `MALFORMED_RESOLUTION` error path · error MessageBar rendering.

**Branch setup.** Same as Scenario 1.

**Resolution.**
1. Drive to the picker.
2. Pick **Rename**. Side = B. New IRI = `urn:project:Profile` — an IRI **already taken** by the shared stub.
3. Preview cascade → expect `MALFORMED_RESOLUTION` from the kernel with a message pointing at the colliding IRI.
4. Confirm **Try again** drops to `picking`; fix the new IRI to `urn:project:billing:Patient` and proceed.

---

## Scenario 8 — Cell auto-clear on successful resolution

**Tests.** D36 §15.3 cell-badge auto-dismissal · cellId thread-through.

**Branch setup.** Use the **cell-race path** (two browser tabs).

**Steps.**
1. Tab 1: on `main`, run the Scenario 1 `feat-A` ESL. Note tab 1's branch tip.
2. Tab 2 (do **not** reload after tab 1's commit; tab 2 must hold a stale view of `main`): paste the `feat-B` Scenario 1 ESL into a new cell and run it.
3. Tab 2's cell should commit but hit `NEEDS_WITNESSED_MERGE` — badge shows `Needs witnessed merge` with **Resolve in Merge rail**.
4. Click the button. Drive Rename to a successful commit (as in Scenario 1).
5. After the success card appears, look at the cell that triggered the resolution — its badge should now show a normal merged status (no error indicator). Internally the cell's commit meta was rewritten from `NEEDS_WITNESSED_MERGE` to `TRIVIAL_MERGE`.

---

## Scenario 9 — Race recovery (BRANCH_CAS_RACE)

**Tests.** §11 error-recovery path · race-diff banner · keyed-by-conflict-id preservation of picks.

**Branch setup.** Use the cell-race path; you need a second commit landing on the branch mid-resolution.

**Steps.**
1. Tab 1: drive the Scenario 1 setup until you have an open resolution session in the `acknowledging` state (cascade previewed, boxes not yet ticked).
2. Tab 2 (same notebook, same branch as the resolution target): run an ESL cell that **introduces a new conflicting IRI** distinct from `project:Patient`. E.g., a new `class project:Visit { ... }` that also exists with a different body on the resolution's candidate head. Commit succeeds in tab 2, advancing the branch.
3. Tab 1: click **Commit merge**. Kernel returns `BRANCH_CAS_RACE`.
4. Expect the flow to drop back to `picking` with the **race-diff banner** showing `New conflicts: project:Visit` (or `+1 new conflict, …`). Your prior Rename pick for `project:Patient` should be **preserved**.
5. Pick a strategy for the new conflict and continue. Confirm commit succeeds the second time.

---

## Scenario 10 — Cancel + localStorage persistence

**Tests.** Reload-recovery (D36 §10) · cancel cleans up storage.

**Branch setup.** Any conflict setup (Scenario 1 is easiest).

**Steps.**
1. Drive to the picker, fill in a Rename pick, but **don't preview**.
2. Reload the page (F5).
3. Notebook should re-mount the resolution flow at the same `picking` state, with the Rename pick intact. (Branches panel may need a moment to repopulate.)
4. Click **Cancel**. The Merge rail returns to its default view.
5. In devtools → Application → Local Storage → `http://localhost:8080`, confirm the `eigenius.mergeResolution.v1` key has been deleted.

---

## Scenario 11 — Empty cascade short-circuit (D36 §7.1)

**Tests.** Cascade-preview shortcut when there are no consequences.

**Branch setup.** Two branches that conflict on a Property's `data_type` (Scenario 2 setup) — but **delete the shared stub's `Profile`/`profile_for` first** by starting over from a fresh database, so the conflicting IRI has no downstream references.

**Resolution.**
1. Drive the explicit-merge path. One conflict card.
2. Pick **Keep one**, winner = A.
3. Click **Preview cascade**. Because there are no downstream references, the kernel returns an empty cascade.
4. Expect the flow to skip the acknowledging state and short-circuit to `committing` (or surface the *"Resolutions are self-contained — no downstream consequences."* empty banner with Commit immediately available — verify which one happens; both are valid per §7.1).

---

## Scenario 12 — Fold-after-20 in the cascade preview

**Tests.** Section fold UI for large cascades.

**Setup.** Hard to manufacture organically. Skip in routine testing; instead, in devtools, set a breakpoint in `CascadePreviewPane.tsx`'s `Section` and temporarily lower `SECTION_FOLD_THRESHOLD` to 2 while you have a 3+ item cascade open (Scenario 1 with a richer Profile stub). Confirm:
- Only the first 2 items render initially.
- A **Show all N** button appears.
- Clicking it expands; the inverse **Show first 2** appears.

Revert the threshold change before continuing other scenarios.

---

## Scenario 13 — Help link

**Tests.** §15 in-app docs affordance.

**Steps.** Open any picker (Scenario 1 setup), click **Strategy reference** in the picker header. Expect a new tab to `https://eigenius.io/`.

---

## Scenario 14 — Telemetry events

**Tests.** §15 telemetry sink.

**Steps.**
1. Open browser devtools console.
2. Drive any successful Scenario 1 run end-to-end.
3. Expect to see, in order:
   - `[merge-resolution] open { state: "loading", branch, candidateHeadShort }`
   - `[merge-resolution] preview { state: "previewing", conflictCount }`
   - `[merge-resolution] commit-success { state: "done", mergeLayerShort }`
4. Drive a Cancel from `picking`. Expect `[merge-resolution] cancel`.
5. Drive an error path (Scenario 7). Expect `[merge-resolution] error { errorKind, rpc }`.

---

## Scenario 15 — Provenance records in the Layer Inspector (D38 §3)

**Tests.** D38 PR 1 end-to-end: `MergeResolutionRecord` resources are committed alongside the resolved bodies and surface in the History panel's Layer Inspector.

**Branch setup.** Any successful merge run leaves a record per resolved conflict on the merge layer. Easiest path: re-run **Scenario 1** (IRI-collision rename) end-to-end and capture the resulting merge-layer id.

**Steps.**
1. Drive Scenario 1 to a successful merge. Note the merge-layer id from the `done` card.
2. Open the History panel. Locate the merge layer in the timeline — its name is `merge:<head_a>+<head_b>`.
3. Expand the Layer Inspector for that layer. Among the layer's contributions you should see a resource at IRI `urn:eigenius:auto:merge-record:<sha256>`.
4. Inspect that resource. Expect:
   - `is_a` includes `urn:eigenius:core:MergeResolutionRecord`.
   - `merge_record_strategy = "Rename"`.
   - `merge_record_conflict_id` matches the conflict id the classifier emitted (e.g. `iri_collision:urn:project:Patient`).
   - `merge_record_rename_side`, `merge_record_rename_from_iri`, `merge_record_rename_to_iri` populated with the picker's choices.
   - `merge_record_branch_a_source_layer` / `_branch_b_source_layer` are present (cycle-shaped conflicts wouldn't carry these — but Scenario 1's IriCollision does).
5. Optional EigenQL query — the chain-resident records are also queryable:

   ```eigenql
   USING "urn:eigenius:core:MergeResolutionRecord"

   MATCH MergeResolutionRecord(?r) {
       "urn:eigenius:core:merge_record_conflict_id": ?conflict,
       "urn:eigenius:core:merge_record_strategy":    ?strategy
   }
   RETURN [] { record: ?r, conflict: ?conflict, strategy: ?strategy }
   ORDER BY ?conflict
   ```

   Expect one row per resolved conflict across the chain's merge history.

**Per-strategy variants.** Re-run with Scenario 2 (KeepOne), Scenario 3 (KeepNeither), Scenario 5 (Restructure), Scenario 6 (Witness) to verify each strategy's optional slots:
- KeepOne: `merge_record_quotient_kind = "KeepOne"` + `merge_record_quotient_winner`.
- KeepNeither: `merge_record_quotient_kind = "KeepNeither"` (no winner).
- Restructure: `merge_record_restructure_new_parent` + `merge_record_restructure_affected_class`.
- Witness: `merge_record_witness` (comorphism IRI) + `merge_record_witness_source_layer` (the original committing layer's id).

**Idempotency check.** Re-running the same resolution against the same span (re-driving from the picker, picking the same strategy + parameters) must produce a merge layer with the same id. The record's `@id` is content-hashed (`urn:eigenius:auto:merge-record:<sha256>`), so identical resolutions deduplicate naturally.

---

## Scenario 16 — Witness on a sibling branch (D38 §4)

**Tests.** D38 PR 2 end-to-end: `WitnessEditor`'s "Search additional branches" disclosure surfaces witnesses on branches outside the merge span; the kernel's fourth-tier walk applies them; the off-span witness gets copied into the merge layer (D38 §3.2 step-4 off-span path) so the merge layer stays self-contained.

**Branch setup.**

1. Re-run Scenario 6's setup through step 3 (so both `s6-a` and `s6-b` exist with `patient_42` and a weight conflict), but **skip the witness cell on `s6-a`** — we want the witness to live elsewhere.
2. Create a new branch `witness-library` off `d36-base`. On it, run the witness cell from Scenario 6 step 4 (`merge_comorphism project:patient_take_b for project:Patient { (a, b, opt) => b }`). Tag this commit if you want a stable name; otherwise the branch ref alone is enough.
3. Switch back to `s6-a` (the merge target).

**Resolution.**

1. Open the Merge rail. Source = `s6-b`, target = `s6-a`. Drive to the picker. One conflict on `urn:project:patient_42`, kind = `IriCollision`.
2. Pick **Witness**. The Combobox is empty — `patient_take_b` lives on `witness-library`, not on `s6-a`'s chain.
3. Click **▸ Search additional branches** (below the Combobox). Type `witness-library` and press Enter (or click Add).
4. The Combobox re-queries. Expect: `patient_take_b` now appears as the single option.
5. Select it. Click **Preview cascade** (empty cascade expected). Click **Commit merge**.
6. Expect the merge to succeed and `urn:project:patient_42` to resolve to weight 80.0 on `s6-a` (branch B's body, per the `λa.λb.λopt.b` witness).

**Off-span copy verification.**

1. Note the merge-layer id from the `done` card. Open the History panel's Layer Inspector for that layer.
2. Among the layer's contributions you should see both:
   - The `MergeComorphism` resource at `urn:project:patient_take_b` — copied from `witness-library` into the merge layer per D38 §3.2's off-span guard.
   - The synthesised transformation `Lambda` at `urn:eigenius:auto:lambda:<sha256>` — same.
3. Inspect the `MergeResolutionRecord` for the conflict. `merge_record_witness_source_layer` should be the `witness-library` tip's layer id (the *original* committing layer, preserved after the copy).
4. **GC stability sanity check.** Delete the `witness-library` branch ref (`eigenius db branch delete witness-library` or via the Branches panel). Re-open the merge layer's Layer Inspector. The `MergeComorphism` and Lambda are still resolvable through the merge layer — the off-span copy made the merge layer self-contained.

**In-span no-copy sanity check (contrast with above).** Re-run Scenario 6 verbatim (witness on `s6-a`, the merge target — *in* the span). After the merge commits, open the merge layer's Layer Inspector and confirm the witness IRI is **not** in the layer's own contributions — the merge layer reaches it through its parent chain instead. D38 §3.2's guard skips the duplication for in-span witnesses (transitively pinned by reachability).

**Stale-branch tolerance.** With the picker open and the scope set to a name that doesn't exist (`bogus-branch`), the query silently skips it — the Combobox still surfaces whatever's reachable from the default scope + any valid extras. No hard error.

---

## Post-run cleanup

The scenarios produce a small but non-trivial chain. To reset:

```bash
# Wipe the local db
eigenius db gc --force   # or just blow away the storage dir
```

Or, if testing against the docker-compose stack: `docker compose down -v && docker compose up -d`.

---

## Coverage summary

| Scenario | D36 surface tested |
|---|---|
| 1 | StrategyPicker + RenameEditor + cascade ack + success path |
| 2 | QuotientEditor → KeepOne + winner radio |
| 3 | QuotientEditor → KeepNeither + ancestor-fallback semantics |
| 4 | KeepBoth greyed-out rationale (§15.5) |
| 5 | RestructureEditor + new-parent definition (heaviest editor) |
| 6 | Happy-path Witness via D37 — `merge_comorphism` ESL → commit-time validators (Rule 18 + Rule 19) → WitnessEditor Combobox → successful merge |
| 7 | MALFORMED_RESOLUTION + Try-again loop |
| 8 | Cell auto-clear on success (§15.3) |
| 9 | BRANCH_CAS_RACE recovery + race-diff banner (§11) |
| 10 | localStorage persistence + cancel cleanup (§10) |
| 11 | Empty-cascade short-circuit (§7.1) |
| 12 | Fold-after-20 section UI |
| 13 | In-app help link |
| 14 | Telemetry events |

Scenarios 1, 5, 8, 9, and 10 are the **must-pass** set before closing Phase 15 — they cover at least one editor of each shape, the cell-side entry point, the most subtle error-recovery path, and the persistence story.
