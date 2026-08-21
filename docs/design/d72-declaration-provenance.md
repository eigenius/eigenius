# D72 — Declaration provenance: agent and warrant

*Status: BUILT `2026-08-20` in [#177](https://github.com/eigenius/eigenius/pull/177) (`a40c952`). §3.1, §3.3 and §4 record what was designed; the **As built** notes below record where the implementation departed from it and why.*

*Arises from [#141](https://github.com/eigenius/eigenius/issues/141) / [#167](https://github.com/eigenius/eigenius/issues/167).
PR #177 stopped `stamp_declared` overwriting an author's `declared_by`; that fix hit a wall which is
this note's subject. Depends on [D6b](d6b-reasoning-trace-schema.md) (the epistemic cluster).*

## 0. The decision

`reflection:declared_by` conflates two independent facts. Split them:

| axis | question | property | type |
|---|---|---|---|
| agent | **who** asserted this | `reflection:declared_by` (retyped) | `core:resource` → `agent:Agent` |
| warrant | **what grounds** it | `reflection:warranted_by` (new) | `core:resource` |
| rationale | **why**, in prose | `reflection:rationale` (unchanged) | `core:string` |

The compiler stops inferring epistemic category from syntax.

## 1. The finding

`declared_by` is documented "Who declared this resource", typed `core:string`, and required by
`reflection:DeclaredResource` — "a resource asserted by a **human** as an axiom, definition, or
design decision."

Every authored value in the tree names something other than a person:

| value | count | what it is |
|---|---|---|
| `urn:schema_org`, `urn:obo:obo-foundry` | 2140 | vocabularies |
| `wrn-literature` | 55 | a source |
| `agent:sab18-method`, `agent:sab16-method` | 100 | methods |
| `wrn-paper:*-criterion`, `*-thesis`, `*-readout` | ~120 | criteria |
| `literature:smith_et_al_2024_3.2` | 30 | a citation |
| `methodology:ic50-classification-convention` | 20 | a convention |
| `d57:m3-generator` | 20 | a program |

Zero name an agent — **except** `urn:schema_org` and `urn:obo:obo-foundry`, which on closer
reading are the agent axis after all: those terms genuinely *are* declared by those projects.
As built they stay in `declared_by` and became `reflection:Organization` resources, so 2140 of
the ~2400 values kept their meaning and gained a referent instead of moving. D6b's illustrative `"Eigenius core team"` is the only agent-shaped value in the
design corpus, and it is an organization. The property is documented as the agent axis and used as
the warrant axis.

Three consequences follow from the string typing:

1. **Nothing validates it.** `"esl-compiler"` sat in 74 slots on the WRN chain undetected (#167). A
   `core:string` has no referent to check.
2. **`agent:`-prefixed values resolve to nothing.** 100 values write an `agent:` prefix; there is no
   `urn:eigenius:agent:` namespace and no `Agent` class anywhere outside the imported schema.org
   vocabulary.
3. **One declarer cannot be recognised across two strings.** An agent with two email addresses or a
   changed affiliation is two unrelated values.

## 2. The second conflation: category from syntax

`stamp_declared` runs at nine ESL call sites: `axiom`, `def`, `class`, `property`, `data`, `codata`,
`macro`, `program` — and `resource`.

The first eight are theory forms, and the stamp is sound for them: writing `axiom` **is** a human
assertion. `resource` is the general instance form and carries anything. From the D71 demo artifact:

```
resource v2:trace_1 : reflection:ProgramTrace {
    reflection:source = "eigenius-reasoning lander: DCG parse (D63) of …";
}
```

`stamp_declared` adds `DeclaredResource` and `declared_by = "esl-compiler"`, so the resource asserts
both *a program produced this* and *a human asserted this as an axiom*. The second is false twice
over: no human asserted it, and the compiler is not who.

Inferring the epistemic category from the keyword is the same category error as the placeholder
value — the compiler answering a question it has no standing to answer, one level up. The author
knows where knowledge came from; the compiler does not.

## 3. The shape

### 3.1 `agent:Agent`

> **As built: the vocabulary lives in `reflection`, not its own layer.** A separate `agent`
> layer below `reflection` is unconstructible: `stamp_declared` injects a
> `reflection:DeclaredResource` reference into every resource the ESL compiler emits, so no
> ESL layer can sit below `reflection` — the trial layer failed `UnresolvedClassReference` on
> all nine of its own resources — while `declared_by`'s `class_types` must resolve
> same-or-lower (Rule 22). The circularity is created by the defect this note exists to fix.
> The classes are therefore `reflection:Agent` / `Person` / `Organization` / `orcid`, and the
> two bootstrap agents are `reflection:agent:unattributed` and
> `reflection:agent:eigenius_core_team`.

A new root-layer namespace. `Agent` with `Person` and `Organization` subclasses (aligned to the
imported schema.org vocabulary rather than duplicating it).

Identity is an opaque stable IRI: `urn:eigenius:agent:<id>`. Email, name, and affiliation are
**properties of that resource**, never the identity — they change and get reassigned, and this
repository's own history already carries one contributor under two addresses.

`agent:orcid` carries the persistent external identifier for a person. ORCID appears nowhere in the
tree today; the pattern is established — [D-reference](../../ontologies/reference/reference.esl)
already handles DOI and PMID as persistent external identifiers, and the same rule applies: real,
validated identifiers, never fabricated.

### 3.2 `declared_by` becomes resource-typed

```
property reflection:declared_by : core:resource {
    description = "The agent who asserted this resource.";
    class_types agent:Agent;
}
```

Rule 8 (`class_types`) and Rule 22 (reference integrity) then require the declarer to **exist in the
chain, same-or-lower**. The #167 defect class stops being a bug to fix and becomes unrepresentable:
`"esl-compiler"` is not an IRI, and an `agent:` IRI that resolves to nothing fails at commit.

### 3.3 `warranted_by` is the new property

> **As built: the 48 warrant targets are `reflection:DeclaredResource`, not a new class.**
> §8.2 asked whether `warranted_by` should carry `class_types`; a `Warrant` marker class was
> rejected because it would be a fifth category cutting across the four epistemic types, and a
> criterion *is* declared by a human. Each stub's description says it is a stub: those
> identifiers existed only as unresolvable strings, and writing a plausible account of what
> `wrn-paper:mmr-restoration-criterion` asserts would fabricate domain claims into a
> publication chain.

```
property reflection:warranted_by : core:resource {
    description = "The criterion, convention, source, or prior result that grounds this declaration.";
}
```

This is what the ~2400 existing values actually mean. Deliberately un-`class_types`'d in v1: the
warranting resources are heterogeneous (a `reference:Citation`, a criterion resource, a methodology
convention, a generator program) and no single class covers them yet.

`reflection:rationale` — "Why this resource was declared", `core:string` — stays as the prose axis.
Three axes, three properties: who, what grounds it, why in words.

## 4. `DeclaredResource.requires` — the fork

`DeclaredResource` requires `declared_by`. The `class` and `property` ESL grammars **reject** a
`ref:declared_by` field, so those forms have no slot for an author and can only ever carry a
placeholder. That is the wall PR #177 stopped at.

Resolved by the split. A theory form has no *agent* the compiler can know, but the human running the
compiler is an agent the **session** can know. Two options, decided here as (b):

- **(a)** `requires` → `recommends`. The placeholder disappears; the epistemic claim weakens.
- **(b) Keep `requires`; give the theory forms a slot.** The ESL grammar gains an optional
  `declared_by` on `class`/`property`/`axiom`/`def`/…, and the compiler fills an unset one from a
  configured session agent rather than a literal. A build with no configured agent fails closed
  rather than inventing one.

> **As built: (b), with the fail-closed clause relaxed to a default agent.** The compiler reads
> `EIGENIUS_DECLARED_BY`; absent or malformed, it writes `reflection:agent:unattributed` rather
> than refusing to build. The marker is deliberately not a real party — naming one for
> unattributed content would be a worse falsehood than the `"esl-compiler"` literal it replaces,
> because it would name someone who exists. §8.1's question is answered: an explicit env var,
> **not** the git committer identity.
>
> The ESL grammar slot on the theory forms is **not built**; the session agent covers the need
> and a per-declaration override remains available.

(b) preserves D6b's enforcement model and keeps "who asserted this" answerable, which is the point
of the epistemic cluster. It costs a grammar change and a compiler configuration surface.

## 5. `stamp_declared` after the split

Stamp `DeclaredResource` on the **eight theory forms only**. `resource` gets its epistemic category
from the author's `is_a`, like any other class membership.

A `resource` block declaring `: reflection:DeclaredResource` explicitly still requires
`declared_by`, now agent-typed and validated. A `ProgramTrace` is a `ProgramTrace` and nothing more.

## 6. Migration

Every change here moves content hashes.

- **Ontology.** `reflection` (retype `declared_by`, add `warranted_by`) and a new `agent` layer are
  bootstrap-resident ⇒ `ManifestDrift` on every persisted store ⇒ reseed, and
  `bootstrap_manifest_pinned` updates.
- **Data, ~2400 values.** The 2135 `urn:schema_org` and 5 `urn:obo:obo-foundry` values are already
  IRI-shaped and are *warrants*, not agents — they move to `warranted_by` and need an importer
  change, not a data edit. The ~200 `.esl`-authored values likewise move to `warranted_by`; their
  targets must be minted as resources for Rule 22 to pass.
- **The 74 `"esl-compiler"` values (#167)** are not migrated: they are recompiled. The `.esl` sources
  hold the correct values, so the chain is rebuilt with the fixed compiler.

Pre-production posture applies — drop-and-reseed is acceptable and there are no deployed consumers.

## 7. Build order

> **All seven steps landed in #177**, plus two the order did not anticipate: the example
> notebook `stats-and-reasoning.json` (four ESL cells writing string warrants — no test
> compiles notebook JSON, so nothing caught it), and `DeclaredClaimGrader` / `DerivedClaimLander`,
> which wrote `declared_by` as a String defaulting to `"encoding-pipeline"` — the same shape as
> `"esl-compiler"`. Also folded in: `core:short_name` gained `pattern: \S+`.

1. `agent` ontology layer + `Agent`/`Person`/`Organization` + `orcid`. Independent of everything else.
2. `reflection:warranted_by`, un-typed target. Additive; no existing value moves yet.
3. Importer change: schema.org and OBO write `warranted_by` instead of `declared_by`.
4. ESL: move the ~200 authored values to `warranted_by`; mint the warrant resources they name.
5. Retype `declared_by` to `core:resource`/`agent:Agent`. Only safe once (3) and (4) have vacated it.
6. ESL grammar slot + session agent (§4b); `stamp_declared` narrowed to the eight theory forms (§5).
7. Reseed; re-derive the demo artifacts and both parse baselines.

Steps 1–2 are additive and can land immediately. Step 5 is the breaking one and must not precede 3–4.

## 8. Open questions

> **Resolved:** (1) the session agent is `EIGENIUS_DECLARED_BY`, not the git committer;
> (2) `warranted_by` carries no `class_types` and the stubs are `DeclaredResource` (§3.3).
> **Still open:** (3) whether `ObservedResource.source` needs the same treatment — measured and
> filed as [#183](https://github.com/eigenius/eigenius/issues/183): 141 values, 84 distinct, 70
> containing spaces, so it is prose for half its uses and needs its own split before any retype.

1. **Session agent configuration** (§4b) — env var, a config file, or the git committer identity?
   The last is tempting and wrong: a commit author is who *committed*, not who *asserted*.
2. **Should `warranted_by` eventually carry `class_types`?** A `Warrant` marker class would let Rule 8
   check it, at the cost of tagging every criterion, citation, and convention resource.
3. **Does `ObservedResource.source` want the same treatment?** It is `core:string` describing an
   external source — the same shape defect, unmeasured here.
