# WRN study — completion plan: the DSB-mechanism recompute backlog

> **Why this exists.** The mechanism arm was originally lifted to *one*
> recomputed DSB marker (53BP1) and the rest deferred as "redundant same-shape
> corroboration." That was an effort-minimizing misread. Scientifically the
> markers are **distinct measurements of distinct steps**: γH2AX/53BP1 report the
> DSB *lesion*; phospho-ATM(S1981)/Chk2(T68) report the DSB *response* (the DDR
> signaling axis the paper uses to bridge DSB → p53). Reproducing the paper's
> mechanism claim properly requires both halves. This plan closes the genuinely
> closeable gaps and records, with reasons grounded in the source data, the two
> that cannot be recomputed from what we hold.

## 1. Validated gap inventory (against the publication)

Paper text: *"WRN silencing in MSI but not MSS cells substantially increased
γH2AX and 53BP1 foci, markers of DSB (Figs 4a–c, ED 6a–h). These findings were
corroborated by increased phospho-ATM(S1981) foci formation and Chk2(T68)
phosphorylation, indicating DSB responses known to activate p53 and
anti-proliferative signaling (ED 7a–e)."* (nihms-1522798 ll. 304–313). Source-data
sheets confirmed by inspection of the pinned `.xlsx`.

| Panel | Reading | Paper stat | Source sheet | Recompute shape | Role | Action |
|---|---|---|---|---|---|---|
| ED 6a | γH2AX foci/cell (colon) | MSI-selective | MOESM9 `ED Fig 6a` ✓ | — | lesion → `CausesDSBs` | **NOT recomputed** — pan-nuclear confound (see below); intensity is the valid γH2AX readout |
| ED 6c | γH2AX **intensity**/cell | contrast-LSM **P<2×10⁻¹⁶**; mean logFC **0.147 (OVK18) / 0.055 (ES2)** | MOESM9 `ED Fig 6c` ✓ | emmeans lsmeans (if_ed5 shape) | lesion → `CausesDSBs` | **recompute** (this is the published γH2AX quantification) |
| ED 6d | γH2AX foci/cell (ovarian) | MSI-selective | MOESM9 `ED Fig 6d` ✓ | — | lesion → `CausesDSBs` | **NOT recomputed** — pan-nuclear confound; subsumed by ED 6c intensity |
| ED 6f/6h | 53BP1 foci/cell | interaction +1.82, p≈2.6e-142 | MOESM9 `6f/6h` ✓ | interaction lm | lesion → `CausesDSBs` | **done** (`concl_dsb_foci`, F8) |
| ED 7b | pATM(S1981) foci/cell (colon) | MSI-selective DDR | MOESM10 `ED Fig 7b` ✓ | interaction lm | **DDR signaling** → p53 bridge | **recompute** |
| ED 7d | pATM(S1981) foci/cell (ovarian) | MSI-selective DDR | MOESM10 `ED Fig 7d` ✓ | interaction lm | DDR signaling → p53 bridge | **recompute** |
| ED 7e | Chk2(T68) + γH2AX (shRNA, in vitro/vivo) | western blot (band levels) | **none** (blot, no numeric sheet) | — | corroboration + CRISPR-artifact control | **stays linked** (no recomputable data) |
| ED 8a | telomere PNA-FISH metaphase | qualitative; *no* telomeric-specific defect | **none** (no sheet in MOESM11) | — | `NotViaTelomereDefect` (rejected hyp.) | **stays Declared/linked** (no data + qualitative null) |
| ED 8d | WRN–fibrillarin coloc | two-tailed t P=1e-3/4.3e-5/0.014 | MOESM11 `ED Figure 8d` ✓ | t-test | `ReducedNucleolarColocalization` | **done** (`concl_coloc_recomputed`) |

**Net actionable gaps:** γH2AX **intensity** (6c) and pATM(S1981) **foci** (7b/7d).

**Marker → valid readout (the load-bearing correction).** Each DSB marker has a
*single* statistically valid quantification, set by its biology:

| marker | valid readout | why the other is invalid |
|---|---|---|
| γH2AX | **intensity** (6c) | γH2AX is the *diffuse* marker — at high damage it goes pan-nuclear, those cells have uncountable (blank) foci and drop out; the dropped cells are disproportionately MSI WRN-KO, so foci-counting shows a **spurious decrease** in MSI (measured interaction −0.43, MSI ×0.96 vs MSS ×1.10). The authors quantify γH2AX by intensity for exactly this reason. |
| 53BP1 | **foci** (6f/6h) | discrete foci, pan-nuclear rare → unbiased (done, F8) |
| pATM(S1981) | **foci** (7b/7d) | discrete foci, 94–100% countable → unbiased |

So ED 6a/6d (γH2AX *foci*) are **not** recomputed — not for effort, but because
foci-counting is confounded for γH2AX; ED 6c intensity is the faithful readout and
carries the published statistic. Chk2(T68) and telomere-FISH are **not**
recomputable from vendored data (no numeric sheet); the telomere "no telomeric
defect" finding is a qualitative negative anyway (a null the encoding does not
derive). All stay linked/Declared **on stated grounds**, not as avoidance.

## 2. What the closure changes epistemically

- `CausesDSBs(WRN, MSI)` gains a **second independent recomputed marker**:
  `concl_dsb_gh2ax` (γH2AX intensity, ED 6c) joins `concl_dsb_foci` (53BP1, ED
  6f/6h). These are added as standalone reproduced warrants — the same shape and
  status as the existing `concl_dsb_foci` (which is *not* consumed by a downstream
  certificate). `concl_dsb` (the linked full-panel conclusion) and `concl_mech`
  are left intact: rewiring them to cite the recomputed lemmas was considered and
  rejected as risky certificate surgery on the validated spine for no epistemic
  gain — the markers are already first-class recomputed `Holds` warrants on chain.
  `mech_dsb` is updated (comment only) to record that γH2AX/53BP1/pATM are now
  recomputed and only Chk2(western) + the in-vitro/vivo shRNA confirmation remain
  linked within it.
- A **new** proposition represents the DDR-signaling step the paper makes
  load-bearing for p53 activation: `onco:ActivatesDSBResponse("WRN","MSI")`
  (working name), recomputed from pATM(S1981) foci. This is the upstream evidence
  `concl_p53_activation` rests on; it is presently absent from the chain entirely.

## 3. Ontology additions

- `onco:ActivatesDSBResponse : core:string -> core:string -> Prop` — "WRN loss
  activates the ATM-driven DSB response in MSI." (Name is a small modeling
  choice; alternative: `ActivatesDamageResponseSignaling`.)
- γH2AX reuses the existing `onco:CausesDSBs`.

## 4. Steps

1. **Extract** (in `extract/`, same discipline as the 53BP1/IF recipes) — **DONE**:
   - `gh2ax-ed6c-extract.R` → `gh2ax_intensity_long.csv` (ED 6c, 32,882 cells) from
     MOESM9; `patm-ed7-extract.R` → `patm_foci_long.csv` (ED 7b/7d, 191,241 cells)
     from MOESM10. (γH2AX *foci* 6a/6d deliberately not extracted — pan-nuclear
     confound, §1.)
   - pinned in `data/sources.tsv` + `data/MANIFEST.md`; `fetch.sh --check` green;
     ED-6c fidelity reproduced (ES2 0.055 / OVK18 0.144 log10, interaction p<2e-16).
2. **Programs** (`programs/mechanism/`) — **DONE**:
   - `gh2ax-intensity-{program,input,files}.json` + `.R` — emmeans interaction on
     log10 intensity; emits `CausesDSBs`.
   - `patm-foci-{program,input,files}.json` + `.R` — interaction lm on foci counts;
     emits `ActivatesDSBResponse`.
   - each carries `runtime:image_digest` (the existing R image; emmeans + base `lm`
     already baked) + `additional_inputs` = Supp Table 1 (MSI genotype join).
3. **Chain** (`chain/08-phase3-invivo-mechanism.esl`):
   - add standalone recomputed warrants `concl_dsb_gh2ax` (CausesDSBs via γH2AX
     intensity) and `concl_ddr_signaling` (ActivatesDSBResponse via pATM) — parallel
     to `concl_dsb_foci`.
   - leave `concl_dsb` / `concl_mech` intact (see §2); update the `mech_dsb` comment
     to record γH2AX/53BP1/pATM as recomputed and only Chk2/FISH linked.
   - reference `ActivatesDSBResponse` in `mech_rule`'s rationale as the recomputed
     DDR-signaling bridge to p53.
4. **Keep-linked, documented:** add chain comments at `mech_dsb` and
   `fish_readout` stating *why* Chk2(T68) (western, no numeric data) and the
   telomere FISH (no source sheet; qualitative null) are not recomputable.
5. **Tests:** extend `crates/eigenius-statistics/tests/wrn_phase1_recompute.rs` /
   the reasoning phase3 test counts; add `#[ignore]`d R-runtime cases mirroring
   the existing foci/IF ones (covered live by the demo).
6. **Demo:** add steps 3j/3k (γH2AX foci+intensity, pATM foci) to
   `demo/wrn-helicase/run.sh`, staging the new slices into the extfile-cache.
7. **Docs:** update `04-review.md` §5 (move γH2AX/pATM from "backlog" to recomputed;
   restate Chk2/telomere as justified linked), `02-dependency-graph.md` (new
   recomputed nodes + the DDR-signaling thread), and add findings to
   `03-recompute-findings.md` (F14 γH2AX, F15 pATM; note the ED-6c fidelity target).

## 5. Recompute-fidelity targets (from the paper)

- **ED 6c** (the one panel with an exact published statistic): reproduce
  contrast-of-LSM **P < 2×10⁻¹⁶**, mean log fold-change **0.147 (OVK18, MSI)** vs
  **0.055 (ES2, MSS)**. This is a Class-A exact-match check.
- **ED 6a/6d, 7b/7d**: directional MSI-selective effect — positive, significant
  condition×MSI interaction in MSI lines; MSS lines n.s. (the `lt(mean_diff,0)` /
  positive-interaction proposition the existing bridges consume).

## 6. Verification

- `python3 extract/*.py` / `Rscript extract/*-extract.R` reproduce the new slices'
  pinned sha256 byte-for-byte (`fetch.sh --check`).
- `cargo test -p eigenius-statistics -p eigenius-reasoning` green (path + count
  assertions).
- Clean-DB `docker compose` demo: all verdicts Hold, now including
  `concl_dsb_gh2ax` and `concl_ddr_signaling`; the ED-6c value matches the paper.
- After this, every **per-cell quantitative** DSB-mechanism readout in the paper
  is recomputed; the only linked mechanism items are the Chk2 western (no numeric
  data) and the telomere-FISH negative (no data + a non-derivable null) — each a
  recorded, defensible decision rather than an open gap.
