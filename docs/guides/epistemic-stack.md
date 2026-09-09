# The epistemic stack

Four ideas hold this system up: **constructive type theory**, **justification logic**, **institution
theory**, and — to get from prose into any of it — **dependent categorial grammar**. None is ours.
What is ours is the claim that they compose, and a chain that runs the composition end to end.

This document explains each idea, why it is here rather than an alternative, and what it looks like
in the tree. Every example is lifted from something that runs in CI.

The design paper is [*Judgements, Warrants, and Logics*](../design/judgements-and-warrants.tex); its
conformance record is the implementation companion in `../publications/`. This is neither: it is the
tour.

---

## The problem, stated once

A system that records how a fact is known has to hold two different objects: a **proof** that a
proposition holds, and a **record of the grounds** on which it is asserted. They obey different
rules. A proof is factive — if it is well-formed, the proposition holds — and it transports: hand it
to a stranger with a checker and they reach your verdict. A record of grounds is neither. It reports
what materials somebody had.

Use one representation for both, and a system that evaluates *these are the grounds* can report
*this is proved*. So the transition from grounds to proof must be **inexpressible**, not discouraged.

That single constraint selects all four ideas below.

---

# Part I — The four ideas

## 1. Constructive type theory

### The move

Martin-Löf's intuitionistic type theory identifies propositions with types and proofs with terms: a
proof of `P` **is** a term `t : P`. Not a certificate about a proof, not a record that a proof
existed — the proof object itself, in the language.

That identification is why this foundation and not another. A classical proof of `P ∨ ¬P` hands you
no object. A constructive one hands you a term you can **store, transport, and re-check**, which is
exactly what a chain needs to hold. When the design says a claim's warrant is `Verified`, what makes
that more than an assertion is that a term is sitting in the chain and a checker will re-run.

### What the kernel implements

`eigentt` is MLTT in the ordinary sense: Π and Σ types, inductive families, a universe hierarchy, and
normalization-by-evaluation deciding definitional equality. `Prop` is the universe of propositions,
with proof irrelevance.

**Dependency is not decoration here — it is the mechanism.** A dependent type may mention a value,
and every load-bearing family in this system is indexed by one:

```esl
data justification:Grounds : Prop -> Type 2 { … }
```

`Grounds` is indexed **by the proposition it grounds**. That one choice is what makes the central
rule enforceable: `Grounds(P)` and `P` are different types, so no term of one is a term of the other,
and no rule relates them. The separation the design demands is not a policy the validator applies —
it is a typing fact.

The witness predicates are indexed the same way:

```
witness:IsDeclaredAs(iri, P)     -- zero constructors, in Prop
```

Zero constructors means **ESL cannot inhabit it**. The kernel synthesizes an inhabitant from the
layer's witness index, or the surrounding term fails to check. An author cannot write down that a
claim is declared; they can only fail to compile until the chain carries the trace that admits it.

### The universe discipline, visible at a use site

`Grounds` sits at `Type 2` rather than `Prop`, for two stacked reasons worth following because they
show the type theory doing real work.

It inhabits a `Type` rather than `Prop` so a grounds term is **stored and re-checkable** — proof
irrelevance would make it uninspectable, and the whole warrant algebra reads the term.

It is `Type 2` rather than `Type 0` because `instantiate` binds `T : Type 1`. A constructor
argument's sort may not exceed the inductive's own, or a large type is smuggled into a small one and
Girard's paradox follows. The constraint is enforced by nanoda's `check_ctor` universe check. The
cost at use sites is nothing: the *type* sits a universe higher, the *values* are unchanged.

### Σ-types are why a class is a record

`resolve_class_type` on a class returns the **Σ-chain of its required and recommended properties**. A
class is not a tag; it is a dependent record type. Hold that — it is what lets a common noun in
English be a type in §4.

---

## 2. Justification logic — the bridge into real science

### What type theory alone cannot reach

Almost nothing in empirical science has a proof term. There is no proof that this compound inhibits
this enzyme, that this gene is selectively essential, that this assay ran correctly. Demanding
`t : P` for scientific claims would leave the system able to record almost nothing.

The tempting move is to weaken — add a confidence score, a provenance note, a grade. Every one of
those is a scalar that can contradict its own evidence, and none survives the question *what does
this rest on?*

### Artemov's move

Justification logic replaces the modality *it is known that F* with an explicit typing `t : F` — *the
term `t` is a justification for `F`*. The justification becomes **an object in the language** rather
than metadata beside it, and terms compose: application `s·t`, sum `s+t`, and in LP a proof checker
`!t`.

That is the bridge. A scientific claim gets a justification *term* even when it has no proof term,
and because the term is structured it can be asked questions.

### Three grounds, and the premise that makes them honest

Every warrant establishes a **specific** proposition, usually not the claimed one:

| ground | what it establishes | premise needed |
|---|---|---|
| `Verified` | `t : P` | none — the same proposition |
| `Observed` | a recording occurred | supplied by the consumer |
| `Declared` | agent `a` asserted `P` | trust in `a` |

An instrument reading does not establish *this compound inhibits this enzyme*. It establishes that a
specific assay on a specific sample yielded a specific number. **When the established proposition
differs from the claim, an accountable party must declare the premise bridging them** — and that
declaration is a chain resource with an owner, not a gap in the argument.

This is the move that lets formal machinery touch real science. The chain records only what was
established; every inference across the gap is a declared premise, attributed:

```esl
resource wrn:bridge_msi_selective : justification:Declaration {
    prov:was_attributed_to  = wrn:authors;
    prov:had_primary_source = wrn:warrant_selective_essentiality_criterion;
    prov:rationale   = "If a sample set measuring WRN dependency by MSI status has its MSI-group
                        mean strictly below its MSS-group mean (MSI more dependent), WRN is
                        selectively essential in MSI. The directional statistical claim is the
                        warrant.";

    eigentt:proposition = type_expr(
        stats:lt(stats:mean_diff_of("urn:eigenius:pub:wrn:wrn_dep_sampleset"), 0.0)
        ->
        onco:SelectivelyEssential("WRN", "MSI")
    );
}
```
<sub>`experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl:135`</sub>

An implication from a **statistical** proposition to a **domain** proposition. Nothing in statistics
licenses that step; a named party does, on a stated basis, in a resource anyone can cite or refuse.
`prov:rationale` carries the human reason, `eigentt:proposition` the machine-checkable half, and
keeping them apart is the point.

### Computed and Sampled are shapes, not grounds

A tempting fourth and fifth category dissolve on inspection. **Computed** is
`app(declared(f : I → O), observed(input))` — the plan declared to denote a function, applied to the
observed input. **Sampled** is a bare `observed` leaf. Neither is stored as a state name, and neither
reaches `Verified`, because in `Computed` the typing `f : I → O` is *itself* declared.

The asymmetry has a reason. If a plan denotes a function, its output is determined by its input and
the execution record is provenance rather than evidence. For a stochastic process the output is not
determined, so the execution record **is** the evidence — an observation, not an application.
Determinism for an external procedure is an empirical fact about the environment, not a property of
the code, which is why somebody has to assert it rather than the system infer it.

### The regress terminates, and cannot close into a circle

A declaration establishes that an agent asserted something; an observation establishes that a
recording occurred. Neither needs a further premise. The chain of justification stops at a named
party or a physical event.

Circularity is excluded separately: a premise's support graph may not transitively include the
premise itself, checked at commit as ordinary cycle detection over the `core:mentions` index. A
declared premise is exempt, and vacuously — it has no support graph, because its bridge rests on
institutional trust rather than on a derived proposition. Artemov's constant specifications permit
self-referential axioms `c : A(c)` for the same reason: postulated self-reference is sound, derived
circularity is not, and only the second has a graph to inspect.

---

## 3. Institution theory — one size does not fit all

### Why not just pick a logic

Statistics has a satisfaction relation and no proof language. Lean has proof terms and no notion of a
sample set. A reaction-network solver has neither. Any single logic rich enough for all of them is
either so weak it says nothing or so strong nothing checks it.

Goguen and Burstall's **institutions** formalize *a logic* as a structure — signatures, sentences,
models, and a satisfaction relation coherent under signature change — precisely so that many logics
can be treated uniformly and related to one another. That is the shape this system needs: not one
logic, but a way to hold several and stitch them.

### The reframing that made it operational

The theoretical question — *is this participating logic really an institution?* — turned out to be
the wrong gate. Meseguer's *general logics* shows institutions and proof systems are not exclusive:
an entailment system without models is a legitimate instantiation, and the kernel is that
**degenerate case** — proofs are the entire content — rather than something outside the framework.

So the operative question became: **can the host hold and re-check a witness for what this logic
establishes?** That is decidable by inspecting the system, and it determines what a logic can reach:

| institution | supplies a proof term? | ceiling |
|---|---|---|
| statistics | no — satisfaction relation, no proof language | `Computed` |
| Lean 4 | yes — an export the host re-checks | `Verified` |
| the kernel | it *is* the checker | `Verified` |

A participating logic contributes vocabulary, a decision procedure returning a tri-state verdict,
derivation resources, and optionally a judgement in its own logic. It does **not** assign a warrant,
define a witness type, or establish `Verified` on its own authority. It may veto a commit; it may not
grant a grade. A verification institution that returns `Holds` but supplies no term reaches
`Computed` and no further.

### Stitching: comorphisms

Institutions are related by **comorphisms** — structure-preserving maps that let a sentence in one be
read in another. Here a comorphism is a declared triple `(ExportFormat, transformation, ImportFormat)`
dispatched in four steps at commit.

The honest example is Catalyst → DiffEq, from the five Julia institutions cooperating in
`notebooks/examples/kinase-institutions.json`. Catalyst speaks `ReactionNetwork`, DiffEq speaks
`OdeProblem`, and compiling one into the other is real work — so you would expect the comorphism's
middle to do it. It doesn't: the shipped transformation is an identity `program:Lambda`, and the
compilation lives in the ExportFormat's own procedure. Worth knowing before authoring one.

For a **verification** institution the comorphism carries the risk. The danger in admitting a Lean
proof is not that Lean's kernel accepts falsehoods; it is that the translated proposition `P'` fails
to denote the chain proposition `P`. The comorphism is therefore a trusted-computing-base element,
named as one alongside the checkers themselves.

---

## 4. Dependent categorial grammar — the bridge from prose

Science is written in prose. If the formalism is reachable only by hand-authoring type theory, it
reaches nothing at scale. So there is a deterministic path from English into typed trees, with **no
LLM in the loop** and the kernel as the arbiter.

### Four traditions, one stack

| tradition | contributes |
|---|---|
| Carpenter, type-logical semantics | *how words compose* — categorial slots, derivation-as-term by Curry–Howard |
| Chatzikyriakidis & Luo, MTT-semantics | *the categories are dependent types* — common nouns as **types**, coercive subtyping |
| Cooper, TTR | *those types are records* — which is our `Class`-as-Σ-typed-record |
| Luo, dependent categorial grammars | the glue between them |

Bekki's Dependent Type Semantics (`lightblue`) is the working instance of the family and the closest
prior art: a CCG parser producing Σ-types with a native type check.

**The second row is the key idea.** In classical Montague semantics a common noun is a *predicate*,
`cell line : e → t`. In MTT-semantics it is a **type**: `CellLine : Set`. That is §1's
propositions-as-types move applied to grammar, and it is why the grammar composes with the rest of
the system instead of sitting beside it. A noun denotes a type; a name denotes an inhabitant of it; a
verb denotes a typed constant.

### The realization

The categorial type is a kernel inductive, carried as a `type_expr` like any other term:

```
data lexicon:Cat : Type 1 { cat_s ; cat_n ; cat_np(Set) ; fwd(Cat,Cat) ; bwd(Cat,Cat) }
```

with a homomorphism `⟦·⟧ : Cat → EigenTT type`:

| category | `⟦·⟧` | role |
|---|---|---|
| `cat_s` | `Prop` | a proposition |
| `cat_n` | `Set` | a common noun = a **type** |
| `cat_np(T)` | `T` | a name = an **entity** of that type |
| `fwd(A,B)` / `bwd(A,B)` | `⟦B⟧ → ⟦A⟧` | a functor; direction drives the parser, not the type |

Four archetypes, each onto a kernel constructor. As real lexicon entries:

```esl
// common noun → a TYPE
resource lexicon:e_cell_line : lexicon:LexicalEntry {
    lexicon:form     = "cell line";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:CellLine, lexicon:num_any) );
    lexicon:sem      = lexicon:CellLine;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "wn:cell_line.n.01";
}

// name → an INHABITANT of that type
resource lexicon:e_hela : lexicon:LexicalEntry {
    lexicon:form     = "HeLa";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:CellLine, lexicon:sg) );
    lexicon:sem      = lexicon:hela;
    lexicon:sem_type = type_expr( lexicon:CellLine );
}

// transitive verb → a typed PREDICATE, category (S\NP)/NP
resource lexicon:e_depends_on : lexicon:LexicalEntry {
    lexicon:form     = "depends on";
    lexicon:sem      = lexicon:depends_on;
    lexicon:sem_type = type_expr( lexicon:Gene -> lexicon:CellLine -> Prop );
    lexicon:sense    = "wn:depend.v.01";
}
```
<sub>`experiments/lexicon/lexicon.esl`. "HeLa depends on BRCA1" composes these three.</sub>

### Felicity is the type checker, not a heuristic

An entry is admitted **iff** `⟦cat⟧ ≡ sem_type` and its `sem` inhabits `⟦cat⟧`. That is the kernel's
own conversion check and its own type checker — the same ones that check a certificate. A lexicon
entry whose category and meaning disagree is not a low-scoring parse; it does not exist.

Subtyping comes free and means something linguistically: CN-as-types subsumption honors
`core:subclass_of`, so a predicate typed at a supertype accepts subclass-typed arguments. *Gene* is a
subclass of *Entity*, so a predicate over entities applies to a gene — because the type theory
already says so, not because the grammar was told.

### What it produces, and how well

A parse is a derivation, and by Curry–Howard the derivation **is** the term — so a successful parse
yields a `Prop`-typed EigenTT term, which is exactly what `eigentt:proposition` holds. The prose path
and the hand-authored path converge on the same object.

Measured on the WRN paper's first page, the gated harness reports 62 units with **grammar-gap 0** and
**missing-lexeme 0** — every unit parses — and the ranker selects the correct reading for 30 of the 41
ambiguous ones. The lexicon behind that is WordNet plus a UMLS domain import, millions of entries,
loaded as a chained layer stack.

---

# Part II — How they compose

## 5. Four strata, and the rule between them

| stratum | form | reads | factive |
|---|---|---|---|
| proposition | `eigentt:proposition` | *this resource asserts P* | — |
| grounds | `justification:Grounds(P)` | *these ground a claim to P* | **no** |
| proof | `eigentt:Judgement` = `holds(L, t, P)` | *a checker of L verified t against P* | **yes** |
| warrant | computed, never stored | *what does this rest on?* | — |

```
Judgement(kernel, c, Grounds(P))   a checker verified that the grounds c ground a claim to P
Grounds(P)                         the grounds
P                                  the proposition
```

`Verified` is the configuration with **no middle layer** — the chain holds `Judgement(L, t, P)`
directly. A proof judgement is not stronger grounds; it skips the stratum.

Two slots on a conclusion hold judgements, and conflating them is the substitution the design
forbids: `grounds_judgement` is `holds(kernel, c, Grounds(P))` and does **not** say `P`;
`proof_judgement` is `holds(L, t, P)` and does, which is why only it admits a `Verified` witness.

The grounds algebra has seven constructors — `declared`, `observed`, `verified`, `app`, `sum_l`,
`sum_r`, and `instantiate`. The first six are Artemov's. `instantiate` narrows a universal to an
instance and adds no ground: in Artemov's own systems specialisation is not primitive, because
instantiation is a logical axiom justified by a constant and applied with `·` — and FOLP can leave
that constant implicit only by taking the constant specification to be **total**. A chain cannot,
since every constant here is a committed resource. `instantiate` is the chain-resident stand-in for
that totality, and it is what lets one declared bridge serve a whole campaign.

## 6. Lean 4 end to end

1. A `lean:LeanProofTerm` lands; AutoOnLoad fires `qc_proof_check`.
2. **Proof validity** — `nanoda_lib`, a Lean kernel reimplementation linked into the host, re-checks
   every declaration in the verbatim export, refusing any axiom outside the permitted set.
3. **Statement correspondence** — the claim's `eigentt:proposition` is externalized to a Lean `Expr`
   and compared with `def_eq`. Without this, `Holds` would mean only *a theorem with this name
   type-checks*.
4. The institution emits a `prov:VerificationTrace` carrying `prov:judgement` —
   `holds(logic_lean4, Checked(payload), P)` — plus the permitted axioms, checker identity and
   checked declaration.
5. `witness_admission` reads that judgement's **type** and admits a `Verified` witness on `hash(P)`.
6. A downstream certificate writes `verified(claim_iri, P)`; the kernel synthesizes the witness and
   the certificate type-checks.

<sub>`crates/eigenius-lean/tests/notebook_fixture_test.rs` runs this, including a **near miss** — the
same proof and target bound to the claim about the *other* individual, which must be refused. Without
a fixture that fails, a demo shows plumbing running rather than a check discriminating.</sub>

**An author cannot forge step 4.** `Exp::Checked(iri)` names a proof an external checker verified, and
the kernel refuses it: it has no proof of the proposition and will not manufacture one. The
institution's own emission is never asked, because `structural_validate` runs before
`autoonload_dispatch` and the followup slice has no validation phase. "Kernel-only, refused from
input" falls out of the pipeline's shape rather than from a guard placed to enforce it.

**Where trust actually sits:** not in the refusal, in the record. Export bytes, target name,
proposition, permitted axioms and checker identity are all on the chain, so any party can re-run the
check and reach the same verdict. A receipt says *I checked*; this says *check it yourself*.

## 7. The whole thing, on a real claim

The WRN chain encodes a published result. Its flagship discovery conclusion projects like this, and
these are assertions from `kernel/tests/justification_projection.rs`:

| question | API | answer |
|---|---|---|
| how many independent routes? | `support(t).len()` | **1** — no `Sum` |
| what does it rest on unproved? | `leaves_of(t, Declared)` | `discovery_rule`, `dd_achilles`, `dd_drive` |
| what measurements of its own? | `leaves_of(t, Observed)` | **none** |
| fully verified? | `is_fully_verified(t)` | **false** |
| survives losing any one? | `survives_without(t, …)` | **no**, for all three |

Read that as the design working. **The flagship claim of a published paper rests on three
declarations, no observations of its own, and no proof — and the chain says so out loud.** A stored
grade would say "Derived" and stop.

The counterfactual is the argument for keeping the term rather than a scalar. Authored with a
fallback — `Sum(dd_achilles, dd_drive)` instead of needing both — the same conclusion answers
`survives_without` differently while grading identically. Only the term separates them. And
committing a `Sum` now requires certificates for **both** branches, so an alternative that `support`
reports is one that was actually grounded rather than one an author asserted.

---

## Where to look next

| you want | read |
|---|---|
| to author a conclusion | [justification-logic tutorial](platform/justification-logic/) |
| the statistics half in detail | [composition §7](composition/07-stats-and-reasoning-walkthrough.md) |
| the Lean half in detail | [lean-institution guide](platform/lean-institution/) |
| comorphisms in detail | [composition §3](composition/03-comorphisms.md) |
| the grammar engine | [D63](../design/d63-dcg-engine-english-grammar.md) |
| the type theory | ESL guide §4–§7; D46, D47, D48 |
| what is specified but unbuilt | the implementation companion in `../publications/` |

### Where these ideas come from

Several of these sit in `references/publications/`, so a claim above can be checked against its
source rather than taken on trust:

| idea | source | local |
|---|---|---|
| constructive type theory | Martin-Löf, *Intuitionistic Type Theory* (1984) | — |
| justification logic | Artemov & Fitting, *Justification Logic: Reasoning with Reasons* | `justification-logic-artemov-fitting-2020.txt` |
| institutions | Goguen & Burstall, *Institutions* (1992) | — |
| institutions ∩ proof systems | Meseguer, *General Logics* (1989) | — |
| MTT-semantics | Chatzikyriakidis & Luo | `TT Appendices/` |
| types as records | Cooper, TTR | `Cooper-2023-TTR-chaper-1.pdf` |
| CCG | Baldridge | `Baldridge_dissertation.pdf` |
| normal-form parsing | Eisner | `Eisner-Efficeint Normal Form Parsing.pdf` |

The WRN chain is not a toy either — it encodes a published *Nature* result, and the paper it encodes
is at `references/publications/WRN-Helicase-Nature.pdf`. §4's parse numbers are measured against its
first page.
