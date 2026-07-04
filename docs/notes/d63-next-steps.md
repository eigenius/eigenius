# D63 — next steps (phases & stages order)

Short running to-do for the two interleaved tracks. Authoritative detail lives in the source docs:
**Stages** = the document→encoding pipeline (`d63-document-preprocessing-scope.md` §1);
**Phases** = the kind-predication reshape (`d63-kind-predication-reshape.md` §6). Mapping: reshape
Phase A = the tail of Stage B (done); Phase C = a slice of Stage C; Phase B = off-timeline cleanup.

## Done
- **Stage A (emission)** — abbreviation extract/ground/emit alias model (built; not yet wired into the
  served parse path).
- **Reshape Phase A** — `kind_of` axiom + unified `kind_raised_nps` (bare mass + plural, incl.
  compounds). Committed `04bab3d`; validated over full-UMLS re-measure (**OPEN 35 → 0**, any-parse ~61%).
- **Reshape Phase B** — retired the `Quantification` hole carrier (`freshen_quant` + 4 call sites,
  `quant_hole_type`/`quant_hole_base`, `QUANT_SENTINEL`, the 2 per-span registrations, the
  `HoleKind::Quantification` variant, the de-risk probe); `EntityRef`/anaphora untouched;
  `d62-bare-plural-quantification.md` marked superseded. Full suite + fmt + clippy green. *(uncommitted)*

## To do (in order)
- [ ] **Wire Stage A into the parse path** — inject the document glossary so `MSI`/`MMR` mass-mark and
      the kind shift fires on them (served `ParseSentence(branch="doc:<id>")`). Closes the biggest bucket
      of re-measure grammar-gaps; makes Stage A true end-to-end, not just emission.
- [ ] **Stage C — post-parse resolution**, in this internal order:
  - [ ] **Reshape Phase C** — grade attachment: a closed prop enters the reasoning layer as `Declared`;
        a `reference:Citation` witness climbs the grade.
  - [ ] **Anaphora / referent resolution (D64)** — pronouns / "these X" / referents.
  - [ ] **Figure / table / citation binding** — `document:FigureRef` / `TableRef` / `reference:Citation`
        (Phase 2 of the preprocessing note).

## Separate / pre-existing (not blocking the above)
- [ ] Sense-crowding → clean single (`encoded`) parses — everything parses ambiguous (×256) over full
      UMLS; the reshape does nothing for this (diagnosis lever #2).
- [ ] Residual grammar constructions — comparatives / `than` / "as a biomarker".
- [ ] OOV — hyphenated/`-based`/`recq` (double-stranded, hypermutable, pcr-based, recq).
- [ ] Named-disease handling (Lynch syndrome — `cat_np` injection for named entities).
