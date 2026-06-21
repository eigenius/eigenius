# D58 harvest — what framing D57 actually needed

The point of running D57 as the first dogfood is to let real friction, not
speculation, specify D58's `objective:` ontology (D58 §5 is full of open questions
precisely because they shouldn't be guessed). Each finding below is a place where
the **D58-lightweight** framing (`Prop` decls + prose comments, on branch
`obj-d57`) forced prose where a typed construct belongs. They are the grounded
requirements for D58.

## Findings

**H1 — No typed thesis / milestone / axiom / edge.** The obligation graph is a
set of `data … -> Prop` declarations distinguished only by comment headers
("THESIS", "MILESTONE m1", …). D58 needs `objective:Objective` (root, → thesis
Prop), `objective:Milestone`, `objective:Axiom`, and a typed **intended-warrant
edge** (`intends` / `depends_on`) — the thesis comment "Intended warrant: m1 ∧ m2
∧ m3 ∧ m4" should be machine-readable, not prose. (Settles D58 §5 "Ontology
shape" toward: a thin typed wrapper over Props, not a parallel structure.)

**H2 — Acceptance criteria are prose.** Each milestone's grade + witness-kind +
falsifier live in a comment ("acceptance: Derived-by-load; witness = the layer
commit"). D58's "acceptance-criterion encoding" question is real: these must be
typed properties on the Milestone so the gate dispatch can read them. Confirmed
shape: `acceptance_grade` (Observed|Declared|Derived|Verified) + `witness_kind`
(layer-commit | query | generator-output | citation) + `falsifier` (string).

**H3 — Frames need first-class revision.** The v1→v2 reframe (slice → full
mapping, on user correction) was done by **rewriting the ESL and reloading**. The
v1 props (`DescriptiveMetadataTyped`, `SliceExpressible`, `FileBound`) are now
orphaned on an earlier layer with **no supersession link** to v2. D58 needs frame
**versioning** — a `supersedes` edge between objective versions — and the four
gates must **re-run on revision**. This was the single biggest gap: reframing is
the *normal* case (the frame⇄ground loop *is* repeated reframing), yet it had no
representation.

**H4 — OPEN/frontier status is untyped.** m1's union-range decision was the gating
unknown at framing time; it could only be recorded as a prose "OPEN (frontier)"
admissibility note. D58's "frontier representation" question confirmed: OPEN nodes
+ their pending/failing gate need a typed status (`frontier_gate: Anchored|…`,
`status: open|blocked|admissible`) so the loop — and a human — can see the
remaining grounding work by query, not by reading comments.

**H5 — Gate assessment was by hand.** The four gates (Expressible / Anchored /
Reachable / Checkable) were assessed in a comment block. The pass confirmed they
map to **real EigenQL**: e.g. *Anchored* = "every axiom IRI has an
`IsDeclaredAs`/`IsObservedAs` witness or a `reference:Citation`" — a concrete
query the kernel can run. D58's gate-query encodings are buildable now; this pass
gives the worked examples.

**H6 — Milestone ⇆ discharge is a viable identity.** m1 was discharged by a
`reasoning:ReasoningSentence` (`concl_discipline`) whose proposition is *the same*
`Prop` as the milestone (`MappingDisciplineDefined("schema_org")`). The "milestone
as a `ReasoningSentence` stub, completed when it Holds" candidate (D58 §5) works:
the stub carries the proposition + acceptance metadata; discharge fills in
justification + certificate. So `objective:Milestone` can *be* a ReasoningSentence
subtype rather than a parallel class. Verified: `concl_discipline` → `Holds`.

**H7 — Branch-per-objective is clean.** `branch create obj-d57 --from <main>` +
`load --branch obj-d57` isolated the whole frame and kept it inspectable; no
collision with `main`. D58 "objective isolation" → branch-per-objective is the
recommended default.

**H8 (skill, not D58) — the `reasoning` anchor template over-specifies.** It
authors an explicit `DeclarationTrace` for each citation, but `reference:Citation`
(a DeclaredResource) has its witness auto-stamped on commit (per the working WRN
literature layer). The skill's Anchor template should drop the explicit-trace step
for citations. → fix in `.claude/skills/reasoning.md`.

## Net recommendation for D58

`objective:Objective` = a typed wrapper holding the thesis Prop + the intended
edges (H1); `objective:Milestone` = a `ReasoningSentence` stub + acceptance
metadata (H2, H6); a `supersedes` edge for revisions with gate re-run (H3); a
typed frontier status (H4); the four gates as committed `QueryClass`es (H5);
branch-per-objective isolation (H7). This is enough real grounding to write D58's
ontology section without guessing — which is the next D58 step.
