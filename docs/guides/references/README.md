# References

A consolidated bibliography for the Eigenius platform. Four lists, organized by what each reference *does* for the project — what we cite, what we depend on, what came before us, and what we share contemporary ground with.

The lists are generated from the `.bib` files in [`docs/references/`](../../references/) by [`scripts/bib-to-md.py`](../../../scripts/bib-to-md.py); within each list, entries are sorted alphabetically by citation key. The same citation keys are used in the LaTeX papers under [`docs/papers/`](../../papers/), so anything you find here can be cited verbatim with `\cite{key}`.

## How to read this guide

If you're looking up a specific work, the citation key is the lookup index — every entry uses the same `key` it has in the `.bib` files. If you're orienting yourself, read by category instead:

- **What does Eigenius actually rely on as published prior art?** → [Cited references](01-cited.md) and [Foundational works](02-foundational.md).
- **What is Eigenius positioned against in the longer arc of formal-knowledge research?** → [Precursors](03-precursors.md).
- **What is Eigenius's contemporary peer set?** → [Related work](04-related-work.md).

## Lists

1. **[Cited references](01-cited.md)** — works explicitly cited from the design documents, papers, or guides. The single source of truth for `\cite{...}` calls in the LaTeX papers; new entries follow the same lowercase-key convention. **60 entries.**

2. **[Foundational works the system relies on](02-foundational.md)** — foundational publications that Eigenius depends on conceptually (type theory, codata, Datalog, knowledge representation, LLMs, WebAssembly, SMT, RPC) but does not yet cite from any design doc, paper, or guide. **32 entries.**

3. **[Philosophical and methodological precursors](03-precursors.md)** — works that situate Eigenius within a longer arc of research on Mathematical Knowledge Management, verified-at-scale formalization, Suppes-style structuralism, formal ontologies in science, the reproducibility movement, and adjacent contemporary projects. **14 entries.**

4. **[Contemporary related work](04-related-work.md)** — contemporary work in applied formal reasoning for science and engineering: institution theory in physics and systems engineering, higher-order logic for the natural sciences, formal ontologies for engineering and chemistry, Homotopy Type Theory and its directed and dynamic extensions, and the epistemology of formal proof. **30 entries.**

## Source files

The four lists above are rendered from these BibTeX files. Edit there, then run the regeneration command below.

| List | Source bibtex |
|---|---|
| 1. Cited references | [`docs/references/eigenius.bib`](../../references/eigenius.bib) |
| 2. Foundational works | [`docs/references/eigenius_additional.bib`](../../references/eigenius_additional.bib) |
| 3. Precursors | [`docs/references/eigenius_precursors.bib`](../../references/eigenius_precursors.bib) |
| 4. Related work | [`docs/references/eigenius_related_work.bib`](../../references/eigenius_related_work.bib) |

## Tooling

Two stdlib-only Python scripts maintain this collection:

- **[`scripts/bib-to-md.py`](../../../scripts/bib-to-md.py)** — regenerates the four markdown files from the bibtex sources. Run with no arguments after editing any `.bib` file:
  ```bash
  scripts/bib-to-md.py
  ```
  Use `--check` to fail if any output is out of date (CI mode), or `--stdout NAME.bib` to preview a single file without writing.

- **[`scripts/verify-citations.py`](../../../scripts/verify-citations.py)** — verifies each `.bib` entry against authoritative sources. For entries with a `doi` field it cross-checks title / year / first-author surname against Crossref; for `eprint` fields it queries the arXiv API; for entries with only a `url` it confirms the URL resolves. Run periodically:
  ```bash
  scripts/verify-citations.py                 # all four bib files
  scripts/verify-citations.py --only-flagged  # entries whose note contains "verif"
  scripts/verify-citations.py --key KEY       # one specific entry, with --verbose
  ```

## Adding a new reference

1. Decide which list it belongs to (cite-it-now → `eigenius.bib`; rely-on-but-uncited → `eigenius_additional.bib`; precursor → `eigenius_precursors.bib`; contemporary peer → `eigenius_related_work.bib`).
2. Add the `@kind{key, ...}` block to that bib file. Prefer including a `doi` (or `eprint` for arXiv) so the verifier can check it.
3. Run `scripts/verify-citations.py --key your-new-key --verbose` — fix anything it flags.
4. Run `scripts/bib-to-md.py` to regenerate this guide.

---

Start with → **[1. Cited references](01-cited.md)**.
