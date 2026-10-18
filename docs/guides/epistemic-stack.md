# The epistemic stack

Four ideas hold this system up: **constructive type theory**, **justification logic**, **institution
theory**, and — to get from prose into any of it — **dependent categorial grammar**. None is ours.
What is ours is the claim that they compose, and a chain that runs the composition end to end.

This document explains each idea, why it is here rather than an alternative, and what it looks like
in the tree. Every example is lifted from something that runs in CI.

The design paper is [*Judgements, Warrants, and Logics*](../design/judgements-and-warrants.tex); its
conformance record is the implementation companion in `../publications/`. This is neither: it is the
tour.

**Who this is for.** Someone who needs to understand *why* the system is shaped this way — a new
engineer, a reviewer, or someone building teaching material from it. It assumes you are comfortable
with types and functions, and have a rough idea of what a proof assistant does. It does **not**
assume you know this codebase, Martin-Löf type theory, justification logic, institution theory, or
categorial grammar. Each is introduced from the beginning.

**If a word is unfamiliar, it is in [Appendix A](#a-glossary).** That glossary has two halves: the
theory vocabulary the four ideas bring with them (*factive*, *inhabit*, *universe*, *warrant*), and
the system vocabulary particular to Eigenius (*resource*, *layer*, *chain*, *commit*). Symbols such
as `t : P` and `S\NP` are in [Appendix B](#b-notation). Nothing later in the document depends on
having read them first — but they are there the moment a term bites.

**Two appendices matter especially if you are teaching from this.** [Appendix C](#c-status--built-and-not)
separates what is built from what is only designed, so material drawn from here does not present the
second as the first. [Appendix E](#e-terms-that-mislead-and-how-to-say-it-instead) lists the terms
that mislead — including two this document's own authors got wrong.

**What Eigenius is, in one paragraph.** A typed knowledge graph. Facts live as *resources* — records
with an identity (an IRI), a set of classes, and typed properties — organised into immutable
*layers* that stack into a *chain*. Adding a layer is a *commit*, and validation runs at commit time:
a layer that violates the rules is rejected rather than stored. What distinguishes it from an
ordinary graph database is that the type system is a full dependent type theory, so a property can
hold a *proposition* or a *proof*, and the database can check them.

---

## The problem, stated once

A system that records how a fact is known has to hold two different objects: a **proof** that a
proposition holds, and a **record of the grounds** on which it is asserted.

They behave differently, in two ways that matter.

A proof is **factive**: if it is well formed, the thing it proves is true. It also **transports** —
hand it to a stranger who has a checker, and they reach the same verdict you did, without having to
trust you.

A record of grounds is neither. *"Two labs measured this, and an expert says it means X"* can be a
perfectly accurate record while the claim itself is false. And it does not transport: a reader who
distrusts the labs, or the expert, has to evaluate the evidence again from the start.

Now suppose you store both in the same slot. A query that means *these are the grounds* can be read
as *this is proved*, and nothing in the system objects. The fix is not a warning or a naming
convention. **Going from grounds to proof has to be impossible to express at all** — not merely
discouraged.

That one constraint is what selects all four ideas below.

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

`eigentt` is Martin-Löf type theory in the ordinary sense. The pieces, and what each one is for:

| piece | what it is |
|---|---|
| **Π types** | dependent functions: the *result* type may mention the argument's value |
| **Σ types** | dependent records: a later field's type may mention an earlier field's value |
| **inductive families** | data types indexed by a value — one `Grounds(P)` for each proposition `P` |
| **universe hierarchy** | `Prop`, `Set`, `Type 1`, … Types are values too, so they need types of their own; the levels stop that from looping back on itself |
| **normalization by evaluation** | how the kernel decides whether two types are *the same* type |

`Prop` is the universe of propositions, and it carries **proof irrelevance**: any two proofs of the
same proposition count as interchangeable. A proof in `Prop` can be checked but not inspected.

**Dependency is the mechanism here, not decoration.** A *dependent* type is one that may mention a
value — `Grounds(P)` is not one type but a whole family, one for each proposition `P`. Every
load-bearing family in this system is indexed that way:

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

To *inhabit* a type is to have a value of it — an inhabitant of `P` is a proof of `P`. These
predicates have **zero constructors**, meaning the surface language offers no way to build one. So an
author cannot write down that a claim is declared.

The kernel supplies the inhabitant itself. A citation names the resource it relies on, so the kernel
goes to that resource and asks whether the layer carries a trace establishing the fact. If it does
not, the surrounding term simply fails to type-check. You do not assert that the evidence exists; you
fail to compile until it does.

### The universe discipline, visible at a use site

Every type lives at a level. `Prop` holds propositions, `Set` holds ordinary types, `Type 1` and
above hold the types of *those*. Where a family is placed is a real decision, and `Grounds` is placed
at `Type 2` for two separate reasons.

**Why not `Prop`.** Proof irrelevance applies inside `Prop`: two proofs of the same proposition are
interchangeable, so nothing can look at one. But the entire warrant algebra works by *reading* a
grounds term — walking it to find the leaves. Putting `Grounds` in a `Type` keeps it inspectable.

**Why level 2 and not 0.** The `instantiate` constructor takes a type argument `T : Type 1`. A rule
of the theory says a constructor's argument may not sit at a higher level than the family itself —
without it you could hide a large type inside a small one, and Girard's paradox lets you prove
anything. `nanoda`'s `check_ctor` enforces it. Nothing is paid at a use site: the *type* moves up a
level, the *values* written in ESL are unchanged.

### Σ-types are why a class is a record

Ask the kernel for a class's type and you get back a **Σ-chain built from its required and
recommended properties**. So a class is not a label attached to a resource; it is a record type, and
"this resource is a `SampleSet`" means "it has these fields, of these types".

Remember this one — it is what lets an English common noun be a type in §4.

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

Two more categories look natural and turn out not to be grounds at all. They are **shapes** a
justification term can have, which is a different thing.

**Computed** is the shape `app(declared(f : I → O), observed(input))`: somebody declared that the
plan `f` is a function from inputs to outputs, and that function was applied to an observed input.
**Sampled** is the simpler shape of a lone `observed` leaf.

Neither is stored anywhere as a label, and neither can reach `Verified` — because in `Computed` the
claim that `f` *is* a function is itself only declared by somebody.

There is a reason the two shapes differ. If a plan really is a function, the input determines the
output, so the record that it ran tells you nothing you did not already have — it is provenance, not
evidence. If the process is random, the input does *not* determine the output, so the record of the
run is the only evidence there is — an observation, not an application.

Which of the two you have is not something the system can work out. Whether an external procedure is
deterministic is a fact about the world it runs in — the machine, the library versions, the seed —
not a property readable from the code. So somebody has to assert it, and be named as having done so.

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

Goguen and Burstall's **institutions** answer this by making *a logic* into a structure with four
parts:

| part | what it is |
|---|---|
| signatures | the vocabulary a theory is written in |
| sentences | what you can say in that vocabulary |
| models | the situations a sentence could describe |
| satisfaction | which sentences hold in which models — *is this true here?* |

plus one law tying them together: translate the vocabulary, and truth has to travel with it
consistently. Package a logic that way and many logics become comparable, and translatable into one
another. That is the shape this system needs — not one logic, but a way to hold several and stitch
them together.

### The reframing that made it operational

The obvious gate — *is this participating logic really an institution?* — turned out to be the wrong
one. It invites an argument about definitions that no amount of engineering settles.

Meseguer's *general logics* dissolves it. Institutions and proof systems are not rival categories: a
system with rules of proof and no models is a legitimate member of the family, not an outsider. The
kernel is exactly that limiting case — proofs are the whole of its content, and it has no separate
notion of a model at all.

So the question was replaced with one the system can answer about itself: **can the host hold a
witness for what this logic establishes, and re-check it later?** That is settled by looking at the
code, and it fixes how strong a claim each logic can support:

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

A worked example, from the five Julia institutions cooperating in
`notebooks/examples/kinase-institutions.json`: Catalyst speaks `ReactionNetwork`, DiffEq speaks
`OdeProblem`, and turning one into the other is real computation.

You would expect that computation to live in the middle of the comorphism. It does not. The shipped
transformation is an identity function, and the actual compiling happens earlier, inside the
ExportFormat's own procedure. Anyone authoring a comorphism should know that before starting: the
middle slot is often a pass-through.

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

**The second row is the key idea, and it is worth slowing down on.**

The older tradition, Montague semantics, treats a common noun as a *predicate*: `cell line` becomes a
function `e → t`, taking any entity at all and answering true or false. Everything is an entity
first; being a cell line is a fact you then assert about it.

MTT-semantics makes the noun a **type** instead: `CellLine : Set`. Now "HeLa is a cell line" is not
an assertion to check but the *typing* `hela : CellLine` — the same propositions-as-types move from
§1, applied to grammar. Nonsense stops being false and starts being ill-typed.

That is why this grammar composes with the rest of the system rather than sitting next to it. A noun
denotes a type, a name denotes an inhabitant of that type, a verb denotes a typed constant — all
three are things the kernel already understands.

### The realization

The categorial type is a kernel inductive, carried as a `type_expr` like any other term:

```
data lexicon:Cat : Type 1 { cat_s ; cat_n ; cat_np(Set) ; fwd(Cat,Cat) ; bwd(Cat,Cat) }
```

A grammatical category is written `⟦·⟧` when read as a type. The translation is structure
preserving — a category built from parts maps to a type built from the matching parts:

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

A dictionary entry is admitted only when two things hold: the type its category translates to is the
same type its meaning claims (`⟦cat⟧ ≡ sem_type`), and its meaning really is a value of that type.
Both checks are the kernel's own — the same equality test and the same type checker that check a
certificate elsewhere in the system.

The consequence is worth stating plainly: an entry whose category and meaning disagree is not a bad
parse to be ranked low. It cannot be committed at all.

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

---

# Appendices

*These exist so the document stands on its own. If you are working in the tree you can skip them; if
you are writing teaching material from it, start here.*

## A. Glossary

### Theory terms

The vocabulary the four ideas bring with them. Each is introduced in the body, collected here.

| term | meaning |
|---|---|
| **factive** | if it is well formed, what it asserts is true. A proof is factive; a record of grounds is not. |
| **transports** | a stranger with a checker reaches the same verdict, without trusting the sender. |
| **term** | a value of the type theory. Under propositions-as-types, a proof *is* a term. |
| **inhabit** | to have a value of a type. An inhabitant of `P` is a proof of `P`. |
| **dependent type** | a type that may mention a *value* — `Grounds(P)` is a different type for each `P`. |
| **indexed family** | a collection of types generated by such a value, one per index. |
| **universe** | a level in the tower `Prop`, `Set`, `Type 1`, … Types need types; levels stop the loop. |
| **proof irrelevance** | inside `Prop`, any two proofs of one proposition are interchangeable, so neither can be inspected. |
| **constructor** | one of the listed ways to build a value of an inductive type. Zero constructors means no value can be written. |
| **judgement** | a checked triple `holds(L, t, P)`: a checker for logic `L` verified term `t` against proposition `P`. |
| **grounds** | the record of what a claim rests on. Non-factive by construction. |
| **warrant** | the answer to *what does this rest on?*, computed from a grounds term, never stored. |
| **witness** | the kernel-supplied evidence that the chain carries a trace for a cited claim. |
| **satisfaction relation** | which sentences hold in which models — a logic's *is this true here?* |
| **Curry–Howard** | proofs correspond to programs, propositions to types; so a parse derivation *is* a term. |
| **regress** | the chain of "and what justifies that?" A declaration or an observation ends it. |

### System terms

| term | meaning |
|---|---|
| **resource** | a record: an identity (IRI), a set of classes, typed properties. The unit of storage. |
| **IRI** | the identifier of a resource, e.g. `urn:eigenius:pub:wrn:discovery_rule`. |
| **class** | a type of resource. Resolves to a **Σ-type** of its required and recommended properties — a dependent record, not a tag. |
| **property** | a typed field. Declares a data type and optionally `class_types` (which inductive its values inhabit). |
| **layer** | an immutable set of resources with a parent pointer. |
| **chain** | the stack of layers, ordered by those pointers. Lookup walks it. |
| **commit** | adding a layer. Validation runs here; a bad layer is rejected, never stored. |
| **bootstrap chain** | the ~20 layers defining the system's own vocabulary, loaded before any user data. |
| **ESL** | the surface syntax you author resources in (the code blocks above). |
| **EigenTT** | the kernel's dependent type theory, and the `eigentt:` vocabulary mirroring it onto the chain. |
| **term** | a value of the type theory — a proposition, a proof, a lambda. Stored via `type_expr(…)`. |
| **institution** | a participating logic, in Goguen & Burstall's sense (§3). **Not** an organisation. |
| **comorphism** | a declared translation between two institutions. |
| **AutoOnLoad** | the hook by which a commit dispatches to an institution — how a proof gets checked when it lands. |
| **witness admission** | the check that decides whether a layer supports a `witness:Is*As`. A **decision procedure over that layer's Trace resources, not a stored index** — the key names the resource, so the lookup goes straight to it. Keyed `(category, iri, hash(P))`. |
| **certificate** | the older name for a `Grounds` term. Renamed because *certificate* implies factivity; you may still meet it in `docs/design/`. |

## B. Notation

| written | read as |
|---|---|
| `t : P` | `t` is a proof of `P` / a term of type `P` |
| `Π`, `forall (x : A) => B` | dependent function type |
| `Σ` | dependent pair / record type |
| `Prop`, `Set`, `Type 1` | universes: propositions, small types, larger types |
| `Grounds(P)` | the type of grounds for `P` — an **indexed family**, `P` is the index |
| `holds(L, t, P)` | a judgement: a checker of logic `L` verified `t` against `P` |
| `⟦·⟧` | the map from a grammatical category to its type (§4) |
| `S\NP`, `(S\NP)/NP` | categorial slots: a thing wanting an NP to its left / left then right |
| `s·t`, `s+t`, `!t` | justification-logic application, sum, proof checker |

## C. Status — built, and not

**Slideware must not present the unbuilt as shipped.** Verified against the tree on `2026-09-08`.

| capability | state |
|---|---|
| the type theory, `Prop`/proof irrelevance, indexed families | built, load-bearing |
| the `Grounds` algebra and its seven constructors | built; every example above type-checks in CI |
| warrant projections (`support`, `leaves_of`, `is_fully_verified`, `survives_without`) | built, as a **Rust API** |
| witness admission from a trace | built |
| the statistics institution (recompute → `Holds` → `Computed`) | built |
| Lean → chain: proof checked, `Verified` witness admitted | built, end to end |
| the DCG grammar engine and lexicon | built; measured numbers in §4 |
| **a conclusion carrying its own `proof_judgement`** | **declared, populated nowhere** |
| EigenTT **proposition** → Lean `Expr` | built, load-bearing — `externalize.rs`, and step 3 of §6 depends on it |
| EigenTT **proof term** → Lean | **not built**; the externalizer covers the `Prop` fragment, i.e. statements, not proofs |
| **a declared comorphism resource for Lean** | **not built**; the Lean institution declares an ExportFormat and query classes, no `(ExportFormat, transformation, ImportFormat)` triple |
| **warrant as a query** | **not built.** The projections are Rust. A justification term is opaque to the query language — no pattern binds one. "Warrant becomes a query" must not be read as "an EigenQL query" |
| `instantiate` with implicit `T`/`P` | prototyped, not landed |

The `proof_judgement` and *warrant as a query* rows are the ones most likely to be over-claimed.
**Verified is reachable
today only through the Lean route** — an external checker's judgement on a trace. The configuration
where the kernel itself proves a chain claim is designed and empty.

## D. Where the numbers come from

Every quoted figure, with its method, so it can be cited or re-run.

- **62 units, grammar-gap 0, missing-lexeme 0, 30/41 reading-correct** — `scripts/measure-parse-rate.sh`
  over the WRN paper's first page, release build with the reranker, scored by `eval-parse-rate.sh`
  against committed baselines. Run `2026-09-08`, 42.90s. Identical to the run of `2026-09-07`.
- **The WRN projection table (§7)** — assertions in `kernel/tests/justification_projection.rs`, run
  by CI on every commit. Not a measurement; a test that fails if the answers change.
- **~8.6M resources** — WordNet plus a UMLS domain import, loaded as a chained layer stack.

Two cautions for anyone quoting these. The parse measurement **must** run release with the reranker:
a debug build overflows the stack in normalization and the harness reports the dead parse as a
grammar gap indistinguishable from a real one, and a cap-only run inflates gaps by construction.
And 30/41 is *reading-level* correctness on ambiguous units — not "97% accurate" or any such
compression.

## E. Terms that mislead, and how to say it instead

Traps this document's own authors have fallen into.

- **"Trace" means two unrelated things.** `prov:*Trace` records how a *resource came to exist*;
  `program:traces:*Trace` records how a *program evaluated*. They shared a namespace once and the
  confusion was real enough to force a rename.
- **"Institution" is Goguen & Burstall's technical term** — a logic packaged with signatures,
  sentences, models and satisfaction. Not an organisation, not an institution in the everyday sense.
  A slide that says "institutions like universities" is wrong.
- **"Declaration" is overloaded.** `justification:Declaration` is an assertion by an accountable
  agent. A *declaration* in type theory is a named constant. Context disambiguates; a slide may not.
- **Do not say a certificate "proves" anything.** It records grounds. The entire design exists to
  keep those apart, so the vocabulary has to hold the line: grounds *ground*, proofs *prove*.
- **Do not call `Declared` weak or `Verified` strong.** They answer different questions. A declared
  bridge from a domain expert may be exactly the right warrant; the system's contribution is making
  it *visible and attributable*, not grading it down.
- **"The kernel proves your claims" is false today.** See Appendix C.

## F. What would make good figures

For teaching material, the four that carry the most:

1. **The three strata as a stack** — `Judgement(kernel, c, Grounds(P))` over `Grounds(P)` over `P`,
   with a struck-through arrow from `Grounds(P)` to `P` labelled *no rule*. That one image is the
   design.
2. **A justification term as a tree** — the §7 WRN conclusion, leaves coloured by ground, with the
   projection questions as callouts on the same picture.
3. **The Lean round trip** — proof lands → nanoda re-checks → correspondence check → trace with
   judgement → witness admitted → downstream certificate type-checks. Six boxes; the near-miss
   fixture as a red branch off the correspondence check.
4. **A parse as a derivation** — "HeLa depends on BRCA1" with categories underneath and the
   resulting `Prop` term at the root, showing that the derivation *is* the term.
