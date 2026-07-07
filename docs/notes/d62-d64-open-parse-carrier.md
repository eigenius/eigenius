# Open-parse carrier — one mechanism, two resolver dispatches (D62 factives ⊕ D64 anaphora)

**Status:** Design note unifying two extensions that were being scoped separately: the
**factive-subordinator** proof-obligation extension (D62 §11.5 item 7;
`docs/notes/d62-subordinator-design-findings.md` §5) and **pronoun/anaphora** referent holes
(D64 §3–4). Two findings: (1) these are **one carrier with two resolver dispatches**, not two
mechanisms; (2) the carrier is a **DCG-engine (elaboration-layer) extension** that leaves the
**kernel term language and the committed chain hole-free** — it needs *no* new `Exp` node. This
**supersedes** D64 §3's `Exp::Anaphor`-as-kernel-node lean (see §2, §7).

## 1. The convergence finding

A parse stops being a closed `Prop` in two places, and they are the same shape:

- **Referential pronoun** (`it`/`they`/`its`/`their`): sem carries a free **`Entity`
  referent** — *"it affects HeLa"* ⇒ `affects(hela, ?ref)`, `?ref : Entity` unbound.
- **Factive subordinator** (`because`/`although`/`while`): sem carries free **proof
  obligations** — *"… because S₂"* ⇒ `Because(p, q, ?h_p, ?h_q)`, `?h_p : p`, `?h_q : q`.

Both are **a parse whose `sem` is open under a context of typed holes, carried out of the
parser for downstream resolution.** The decisive reason to unify rather than build twice: a
**single sentence carries both at once** — *"**it** died **because** the gene was deleted"*
has an `Entity` hole (`it`) **and** proof obligations (the `because` clauses) in one parse. So
there must be **one** carrier; two parallel mechanisms cannot represent that sentence.

## 2. The carrier — engine-side holes, kernel stays hole-free

**A hole is a metavariable, and metavariables are an *elaboration*-layer concept — they do not
belong in the kernel term language.** This matches the Lean/`nanoda_lib` split: the kernel
`Expr` there has bound `Var`, free `Local` (FVar), `Sort`, `Const`, `App`, `Lambda`, `Pi`,
`Let`, `Proj`, literals — **no metavariable, no `sorry`, no hole**. Holes live in Lean's
*elaborator* (`Expr.mvar`); the kernel only ever checks fully-elaborated terms. So our `Hole`
is the analogue of an elaborator metavariable, and it should live in the **DCG engine**, not in
`Exp`.

Concretely, an **open parse** is:

```
OpenParse {
    term: Exp,                 // an ordinary kernel term whose hole positions are
                               //   FRESH FREE VARIABLES (?h₁ … ?hₙ) — already neutrals in NbE
    holes: Map<VarId, HoleInfo {
        ty,                    // the hole's type — Entity (pronoun) or a parse-computed Prop p
        kind,                  // resolver dispatch tag: EntityRef | ProofObligation (see §3)
        meta,                  // morphosyntactic features (pronoun) / clause provenance (oblig.)
    }>,
}
```

The hole-ness (`id`, `kind`, `features`) lives in the **engine-side `holes` context**, keyed by
variable — **not** in the kernel term. The kernel sees only free variables, which it already
handles. So:

1. **No new `Exp` node.** A hole position is a fresh free variable; in NbE a free variable is
   already a **neutral** (reduces to itself, readback is the variable). Zero new `eval` /
   `readback` / `check` arms — the opposite of adding an `Exp::Hole`, which would force a new
   case into every NbE function. D64's objections to "just a free `Var`" are met without a node:
   *collision* → a reserved fresh-var namespace (the kernel already keeps free vars distinct);
   *nowhere for features* → the engine-side `HoleInfo`, keyed by var; *NbE special-casing* →
   none, free vars are already neutral.
2. **Felicity = type-check under Γ; non-empty `holes` ⇒ OPEN.** The kernel already type-checks a
   term *under a typing context* (`CheckCtx` carries one; that is how it goes under every
   binder). The gate checks `term` under the context built from `holes` — so
   `Because(p, q, ?h_p, ?h_q)` and `affects(hela, ?ref)` type-check. The **engine** classifies a
   parse with non-empty `holes` as **open** and does not admit it as a final closed parse.
   (No kernel change: "open vs closed" is an engine judgment — `holes` empty or not.)
3. **Chart threading.** When items combine (`apply`/compose/coordinate), their `holes` maps
   union into the result (fresh var ids keep them distinct). Engine-level bookkeeping; the
   kernel terms just compose as usual.
4. **Open forest + resolve→re-gate seam.** `parse` returns a closed forest **and** an open
   forest (each entry an `OpenParse`) — D62 §11.5 item 2. The resolver (engine/orchestration)
   fills holes; the kernel **re-gates** the resulting *closed* term (`reduced_felicitous`,
   §11.5 item 6). Only closed terms are ever committed.

> **A hole (metavariable) is not an `Opaque` constant.** They share only "inert under
> `eval`/`readback`" and even that is incidental (a free var is inert because it is *unbound*,
> not because it is a declared constant). Their roles are opposite: an **opaque constant**
> (`EigonAxiom`, or the GH #95 unfold-suppressed `Opaque`) is **determined, shared, terminal** —
> one referent, and a term using it is a **closed, admissible** final parse. A **hole** is
> **undetermined, per-occurrence, provisional** — it exists to be *removed* (substituted) or
> *discharged* (witnessed), and a parse still carrying one is **open**, never committed.

## 3. The divergence (dispatch by `kind` — keep separate)

The carrier is unified; the **resolvers are not**, and must not be.

| | `EntityRef` (pronoun) → **D64** | `ProofObligation` (factive) → **grounding** |
|---|---|---|
| **`ty`** | `Entity` (or narrower) — non-dependent | a parse-computed `Prop p` — **dependent** |
| **discharge** | **substitute** the free var with the antecedent term, drop the `holes` entry, re-gate (term rewriting → closed term) | **witness** the proposition — `Holds`/`Open`/`Fails`; *no* term rewrite, the obligation gets a grounding verdict (committed-form caveat §5) |
| **selection** | anaphora accessibility/salience + feature match (LLM-ranked, D64 §4) | retrieve/derive a grounding witness (D43/D49) |
| **binding theory** | c-command/recency/agreement; donkey = bound `Var` (D64 Phase B) | presupposition projection: free ⇒ projects through ¬/modals; local λ-discharge under plugs ⇒ filtered (findings §5) |
| **fail-closed** | unresolved pronoun ⇒ no committed sentence, recorded finding (D64 §4.4) | `Fails` ⇒ presupposition-failure finding; never a quietly-false claim |

So: **unify the carrier, dispatch the resolver on `kind`.** Anaphora accessibility and
presupposition projection stay entirely separate theories layered above the shared carrier.

## 4. Dependent, heterogeneous holes — design for both from day one

- **Dependent `ty`.** A proof obligation's `ty` is `p`, a Prop *computed during the parse*, not
  a constant. The pronoun's `Entity` is the **non-dependent special case**. `HoleInfo.ty` is
  therefore an arbitrary term, and the gate checks the open `term` under a context that may bind
  earlier holes referenced by later ones. (Free vars + a typing context already give this — it
  is exactly the kernel's under-binder discipline.)
- **Heterogeneous coexistence.** One open parse may hold holes of *both* kinds (§1). The open
  forest and the resolve loop route each hole to its `kind`'s resolver — pronouns to D64,
  obligations to grounding — within the same sentence, re-gating once all are discharged (or
  failing closed on the first unresolvable one).

## 5. Build order — pronouns first as the carrier MVP

`EntityRef` is the strictly simpler instance (non-dependent `ty`, substitution discharge → a
clean closed term), and **D64 Phase A** (referential anaphora to chain IRIs) already specifies
it end-to-end. So:

1. **Build the carrier on the entity case = D64 Phase A.** The `OpenParse` structure
   (free-var holes + engine-side `holes` context) + `Case` feature + open forest + the D62
   resolver *component* (a step in the D62 `FormalizeDocument` pipeline institution, not its own
   institution) + kernel re-gate. This *is* the carrier MVP, de-risked on the simpler
   hole. Carry `kind` in `HoleInfo` from the start (only `EntityRef` used initially) so adding
   `ProofObligation` is a new resolver arm, not a representation change.
2. **Layer the factive case on the validated carrier.** Add `ProofObligation` holes (the
   factive subordinator entries, dependent `ty`), the grounding resolver dispatch, and the
   presupposition-projection/plug handling — including the attitude-verb **plug** fix to the
   existing intensional `shows` (D63 §8.11), which must *bind* its complement's obligations
   rather than treat them opaquely (findings §7).

**Committed-form caveat (factive only).** An entity hole resolves by substitution → an
unambiguous closed term. A proof obligation resolves to a *grounding judgment* (witness/finding)
**outside** the term — so the committed form of `Because(p, q, …)` is a real design question:
proof-carrying (witnesses substituted for the proof vars, closed) vs. erased (a non-dependent
`Because(p,q) : Prop` committed, the obligations recorded as separate grounding requirements).
Either way the *metavariable artifact is transient*; which committed form is right is part of
the factive-arm design (ties to the expert round-3 questions). The entity case has no such
ambiguity.

Deictic `we` (a designated speaker/author referent, not a hole) and the propositional-anaphoric
`however`/`thus` (an `EntityRef`-like hole over a *Prop* antecedent) slot onto the same carrier
once it exists.

## 6. Touch points

- **`kernel/src/nbe/term.rs` — NO new node.** Holes are fresh free variables; the kernel stays
  hole-free. (This is the change from the earlier `Exp::Hole`/`Exp::Anaphor` plan.)
- `kernel/src/dcg/{category,lookup,parser}.rs` — the `OpenParse` carrier (an `Item`/forest
  entry gains a `holes: Map<VarId, HoleInfo>`); `Case` feature (D64 §3); holes union through the
  chart; `parse` returns the open forest distinctly; pronoun + factive lexical entries whose
  `sem` introduces a fresh hole-var (resp. applies `Because` to hole-vars).
- the felicity gate (`reduced_felicitous` / `gate_entry`) — type-check the open `term` **under
  the context built from `holes`** (reusing `CheckCtx`'s context); the **engine** marks
  non-empty-`holes` parses as open and never emits them as final closed parses.
- `proto/eigenius.proto`, `server/parse.rs`, `orchestration` — open forest + holes on
  `ParseSentence` (D62 §11.5 item 2); the resolver *component* (a step in the `FormalizeDocument`
  pipeline institution, not its own institution) dispatches per `kind`
  (D64 §4 for `EntityRef`; grounding for `ProofObligation`); kernel re-gate of the resolved
  closed term (§11.5 item 6). **The kernel and the chain only ever see closed terms.**

## 7. Resolved: free-var-in-Γ, not a kernel node (and why)

The earlier draft (and D64 §3) leaned on an inline `Exp::Anaphor`/`Exp::Hole` kernel node. The
`nanoda_lib`/Lean precedent settles this the other way: the kernel has **no** metavariable
construct *by design* — holes are an elaborator concept, and the kernel only checks
fully-elaborated terms. Putting a hole node in `Exp` would (a) leak an elaboration concept into
the trusted term language and the committed chain, and (b) force a new case into every NbE
function — whereas free-var-in-Γ reuses the existing neutral + typing-context machinery and adds
**zero** kernel surface. So: **the carrier is a DCG-engine extension; the kernel stays
hole-free.**

Remaining field-idiom checks (representation *within* the engine):

- **Free-var store (adopted) vs. variable-free.** We carry holes as free vars + an engine-side
  store (a Steedman-style unbound-dependency store, kept out of the kernel). The alternative is
  Jacobson-style **variable-free** semantics (pronouns as identity functions, composition via
  `g`/`z` — no holes at all). Worth a deliberate check that free-var-store doesn't under-generate
  for cross-clausal/donkey cases before D64 Phase B.
- **Joint vs. independent multi-hole resolution** (D64 §7) — and whether an `EntityRef` and a
  `ProofObligation` hole in the *same* sentence ever need *joint* resolution (likely not —
  different resolvers — but confirm the heterogeneous case is independent).

## 8. References

- D62 §11.5 item 7 (the engine extension; generalized here), §7.4 (S3 reference resolution).
- D64 §3–4 (`Case`, open forest, the resolver, re-gate, fail-closed) — the entity instance this
  note generalizes; its `Exp::Anaphor`-as-kernel-node recommendation is **superseded** here
  (free-var-in-Γ, engine-side).
- `references/nanoda_lib/src/expr.rs` — the Lean-kernel `Expr` (no metavariable/`sorry`): the
  precedent that holes are elaborator-level and the kernel stays hole-free.
- `docs/notes/d62-subordinator-design-findings.md` §5 (presupposition = proof obligation;
  projection = hole scoping), §7 (plugs / attitude verbs / `shows`).
