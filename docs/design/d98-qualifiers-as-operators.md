# D98 — Qualifiers as logical operators

**Status: proposed** (2026-09-28). Supersedes the inert/measurement split of
`docs/notes/d62-adverb-semantics-decision.md` for the qualifiers below. Decisions 1–4 are open, their
proposals revised 2026-09-29; decision 5 is decided (2026-09-29).

## The gap

Every adverb the parser reads is transparent: its meaning is `λV. V`, so the proposition does not
contain it. The WRN claims corpus (`experiments/publications/wrn-helicase/claims/`) measures the
cost: 14 of its 62 claims parse only by dropping a qualifier, and in four the proposition then says
something else.

| The paper says | The parse says |
|---|---|
| p53 activity contributes to, but is not solely responsible for WRN dependence | p53 activity is not responsible for WRN dependence |
| dMMR alone contributes to but does not fully explain this synthetic lethal interaction | dMMR does not explain it |
| DSBs … are not merely a consequence of cell death | DSBs are not a consequence of cell death |
| Ch3+5 transfer … partially rescued viability from shWRN | the transfer rescued viability |

The first two contradict the paper's own `contributes to`. None of these qualifiers is vague. Each
has an exact logical form, which this document states and proposes to represent.

D62 split adverbs into an inert bulk, made transparent, and a measurement subset (`selectively`,
`preferentially`, `significantly`, `highly`, `predominantly`) whose semantics was never built. The paper shows the
split is wrong in both directions: `commonly`, `favorably` and `independently`, filed as inert, carry
checkable content, and the exclusives (`solely`, `merely`, `alone`) and completeness qualifiers
(`fully`, `partially`) are in neither list.

## The paper's qualifiers

Counted in the PMC author manuscript (`data/slices/PMC6580861.txt`), whole words:

| Qualifier | Uses | Class |
|---|---|---|
| `respectively` | 29 | distributive pairing |
| `preferentially` | 8 | contrastive |
| `alone` | 4 | exclusive |
| `only` | 3 | exclusive |
| `selectively` | 3 | contrastive |
| `specifically` | 3 | discourse (all three open a methods elaboration) |
| `significantly` | 2 | statistical |
| `substantially` | 2 | degree |
| `predominantly` | 2 | proportion |
| `commonly` | 2 | frequency |
| `typically` | 2 | frequency |
| `fully` | 2 | completeness |
| `similarly` | 2 | one discourse marker, one manner (`scored similarly`) |
| `solely`, `merely`, `just`, `simply` | 1 each | exclusive |
| `partially` | 1 | completeness |
| `independently` | 1 | evidential |
| `favorably` | 1 | evaluative comparison |

Five are negated: `not solely`, `not merely`, `not just`, `not simply` (`body.p8`, `p10`, `p9`,
`p3`) and `not fully` (`body.p14`).

## The meanings

Notation: `P` a predicate, `a` the entity the qualifier is about, `=` propositional equality
`Id(A, y, a)` at the domain `A` of `P` (decision 5 — `lexicon:Entity` for the paper's relations),
`≻` "stronger than" on a scale, `d(·)` a degree.

### Exclusives: `only`, `solely`, `alone`; scalar `merely`, `just`, `simply`

An exclusive has two parts (Horn 1969): what it **presupposes** — `a` has the property — and what it
**asserts** — nothing else does.

- `only a P` / `a solely P` / `a alone P`:
  presupposes `P(a)`; asserts `∀y. P(y) → y = a`.
- **Negation targets the assertion; the presupposition survives.** `a is not solely P` is
  `P(a) ∧ ¬∀y. (P(y) → y = a)`, that is **`P(a) ∧ ∃y. P(y) ∧ ¬(y = a)`**.
  - `p53 activity … is not solely responsible for WRN dependence` (`body.p8`) =
    `responsible(p53 activity, WRN dependence) ∧ ∃y. responsible(y, WRN dependence) ∧ ¬(y = p53
    activity)`: p53 activity is one of the causes, and there is another. That is what `contributes
    to` in the same sentence says.
- The scalar exclusives rank alternatives rather than individuals: `a is merely Q` presupposes `Q(a)`
  and asserts `¬∃R ≻ Q. R(a)` — nothing stronger than `Q` holds of `a`.
  - `DSBs … are not merely a consequence of cell death` (`body.p10`) = `consequence(DSBs, cell death)
    ∧ ∃R ≻ consequence. R(DSBs)`. The paper names the stronger alternative in the same sentence:
    `DSBs precipitate the lethal effects of WRN loss` — a cause.
  - `not just a consequence of CRISPR/Cas9 activity` (`body.p9`) and `not simply a result of dMMR`
    (`body.p3`) have the same form.
- **`alone` restricts to one contributor, without the others.** `one event alone does not [lead to
  cell death]` (`abstract.p0`) and `hypermutability alone cannot account for WRN dependency`
  (`body.p12`) say that `a`, with the alternatives absent, does not suffice. Its alternatives are the
  co-occurring factors the sentence or its context names (the two genetic events; dMMR and the other
  genomic lesions). The chain's opaque `ontology:sole(x)` stands for this today.

**What the kernel lacks:** equality in lexicon meanings. The kernel has propositional equality
(`Exp::Id`, in Prop since D46) and the type-expression encoding has an `Id` constructor, but the ESL
term encoder (`esl/compile.rs`, `encode_type_expr_to_value`) produces only `Lam`, `Sig`, `Pi` and
`App`: no closed-class meaning can say `y = a`. Measured: a SemTerm containing `Id(lexicon:Entity,
y, x)` compiles and is rejected by the felicity gate.

### Completeness and degree: `fully`, `partially`, `substantially`

These say how far along a scale a predicate holds (Kennedy & McNally 2005).

- A **closed-scale** predicate holds to an extent `d ∈ [0, 1]`.
  - `fully P`: `d(P) = 1`. `partially P`: `0 < d(P) < 1`. `not fully P`: `d(P) < 1`.
  - `dMMR alone contributes to but does not fully explain this synthetic lethal interaction`
    (`body.p14`) = `0 < d(explain, dMMR, the interaction) < 1`: the first conjunct is `contributes
    to`, the second `does not fully explain`, and `alone` restricts the extent to dMMR's own.
  - `Ch3+5 transfer … partially rescued viability from shWRN` (`body.p13`) = `0 < d(rescue) < 1`.
- A **relative-standard** degree compares to a contextual threshold, as a gradable adjective does
  today (`gt(deg_X(x), std_X)`).
  - `WRN silencing … substantially increased ɣH2AX and 53BP1 foci` (`body.p9`) = `d(increase) >
    std_substantial`.
  - `substantially weaker changes in phospho-p53 intensity in WRN-depleted MSS models` (`body.p8`) is
    the comparative: the difference exceeds the standard.

**What the grammar lacks:** a degree on verbs. Adjectives carry one (`deg_X`, `measurements:gt`);
`explain`, `rescue` and `increase` do not.

### Contrastive: `selectively`, `preferentially`

These compare the predicate across a target class and a contrast class.

- `P selectively in A` (against `B`): `P(A) ∧ ¬P(B)`.
  - `WRN was selectively essential in MSI models … yet dispensable in MSS models` (`abstract.p0`)
    states both halves; the contrast class is MSS.
- `P preferentially in A` (against `B`): `d(P, A) > d(P, B)`.
  - The methods define it: `Genes that were preferentially dependent in MSI compared to MSS cell
    lines were identified using linear modeling … We estimated the difference in mean dependency
    between MSS and MSI cell lines` (`methods/differential-dependency-analysis`). So
    `preferentially dependent in MSI` = `mean dependency(MSI) < mean dependency(MSS)`, with its test.
- **The contrast class is stated or implied.** `compared to MSS cell lines` states it
  (`body.p2`); `preferentially in MSI cells` (`body.p15`) implies it from the discourse, where MSS is
  the only contrast the paper draws.

### Frequency and proportion: `commonly`, `typically`, `predominantly`

These quantify over cases.

- `MSI is most commonly observed in colorectal, endometrial, gastric, and ovarian cancers`
  (`body.p3`): the frequency of MSI in each of those four lineages exceeds its frequency in any other.
- `More commonly, MSI cancers arise following somatic MMR inactivation` (`body.p0`): more MSI cancers
  arise that way than from Lynch syndrome, the alternative the previous sentence names.
- `typically MLH1 promoter hypermethylation` (`body.p0`), `predominantly dispersed staining`
  (`body.p11`): a majority — the proportion exceeds one half.

The count operator `lexicon:card` (which `fewer … than` uses) expresses these as comparisons of
counts or proportions.

### Statistical and evidential: `significantly`, `independently`

- `Induction of shWRN1 but not shWRN1-C911 significantly impaired tumor growth` (`body.p6`): the
  impairment holds, and its test rejects the null at the study's level. The test is a D52 statistics
  record; `significantly` asserts a property of that record.
- `Projects Achilles CRISPR/Cas9 and DRIVE each independently identified WRN` (`body.p2`): each
  identified it, and neither identification rests on the other's evidence. This is a claim about the
  two findings' warrants, which the justification layer already models.

### Evaluative comparison: `favorably`

- `the MSI/WRN relationship compared favorably to other strong biomarkers` (`body.p2`): on the
  measures the methods name (PPV and sensitivity, `methods/dependency-and-biomarker-analysis`), the
  MSI/WRN relationship is at least as good as the others.

### Distributive pairing: `respectively`

- `A and B … X and Y, respectively` pairs the lists in order: `R(A, X) ∧ R(B, Y)`. `chromosomes 3
  and 5 (Ch3+5), carrying MLH1 and MSH3, respectively` (`body.p13`) is `carry(Ch3, MLH1) ∧ carry(Ch5,
  MSH3)`. 29 uses, the most of any qualifier.

### Discourse markers: `specifically`, `similarly`

`Specifically, we used linear regression models …` and `Similarly, WRN depletion impaired …` relate a
sentence to its neighbour and add nothing to its truth conditions. Transparent is right for these,
and for manner in a protocol (`foci … were scored similarly`).

## Decisions

1. **How a presupposition is carried.** `not solely P` needs the negation to reach the assertion and
   leave `P(a)` standing.
   - *Proof obligations* — the project's existing design for presupposition
     (`docs/notes/d62-subordinator-design-findings.md` §5, confirmed by the expert review in §7). A
     presupposition is a free proof variable. `solely A P a` introduces `h : P(a)` free in the context
     and asserts `∀(y : A). P y → Id(A, y, a)`. Projection is ordinary variable scoping: negation
     (`¬X := X → False`) and modals do not bind `h`, so it projects, and `not solely` comes out as
     `P(a) ∧ ¬∀(y : A). P y → Id(A, y, a)` with no rule specific to negation; a conditional whose
     antecedent supplies it binds it by `→`-introduction, which filters it. The obligation rides the open-parse carrier
     (`docs/notes/d62-d64-open-parse-carrier.md`) as a `ProofObligation` hole, discharged by a
     grounding verdict (`Holds` / `Open` / `Fails`) and failing closed. The same mechanism serves
     factives (`found that`), definites and `again`.
   - *Σ-conjunction*, `Σ(h : P(a)). …` — rejected. It puts the presupposition inside the
     proposition, so a negation over the meaning negates it too; getting `not solely` right then
     requires placing the negation inside by hand, which is the lexicalized form below.
   - *Lexicalized negated forms* — `not solely`, `not merely`, `not just`, `not simply` as their own
     operators, `λP.λa. P(a) ∧ ∃y. P(y) ∧ ¬(y = a)` — rejected. Exact for the paper's five uses;
     wrong wherever `only` meets any other operator, or `not` stands apart from the particle.
   - *Proposed:* proof obligations.
   - *Depends on the `ProofObligation` arm, which is not built.* The carrier was built to take it —
     `HoleInfo` carries a `kind` for that purpose — but `HoleKind`
     (`kernel/src/dcg/parse/felicity.rs`) has only `EntityRef`, and the arm is documented as
     planned. Exclusives would be its second client, after factives.
   - *Inherited gap: plugs.* An attitude or report verb must bind the obligations its complement
     emits (findings §7). An opaque report axiom (`Prop → Entity → Prop`) acts as a hole instead, so
     the obligation projects to the author. `These observations suggest that WRN dependency is not
     simply a result of MMR deficiency.`, a curated unit, puts a scalar exclusive under `suggest`.
     Whether its presupposition should project there is itself open — the suggesting subject is the
     authors' own data — and the plug fix is what makes either answer expressible.
2. **The degree of a verb.**
   - *One operator over the predicate the qualifier modifies:*

     ```
     extent : Π(A : Set). (A → Prop) → A → Degree
     ```

     A degree qualifier attaches to the verb phrase — `dMMR [does not fully [explain this
     interaction]]` — and a verb phrase is a unary predicate whatever the verb's arity, so arity is
     not a parameter, and `A` comes from the verb phrase as in decision 5.
   - *A degree function minted per verb sense*, as `deg_X` is per adjective — rejected: it multiplies
     the importers' output by every verb sense.
   - *Proposed:* the single operator.
   - An earlier draft of this decision wrote `extent : (Entity → Entity → Prop) → Entity → Entity →
     float`, fixing the type at `Entity` and the arity at two; decision 5 applies to both.
   - Adjectives are unary predicates too, and the argument against per-sense minting applies to them:
     the WordNet importer mints `deg_{loc} : Entity -> core:float` per adjective sense
     (`crates/eigenius-wordnet/src/convert.rs`). The polymorphic `extent` covers both; retiring
     `deg_X` is the direction, not part of this decision.
   - *Scales are declared, and no source records them.* `fully` and `partially` presuppose a closed
     scale, so they apply only to predicates declared with one — a restriction in the type, or `fully
     carry` type-checks. WordNet, UMLS and SPECIALIST do not record a predicate's scale; the paper's
     three verbs (`explain`, `rescue`, `increase`) would be declared by hand.
   - *What `Degree` is — open.* Relative standards (`substantially`, the comparatives) only order
     degrees, which `measurements:gt` over `core:float` already does. A closed scale's endpoint —
     `fully` as `d = 1` — needs every closed scale normalised to `[0, 1]`. That normalisation is a
     choice to state, not to assume.
3. **The contrast class.** Taken from `compared to B` where the sentence states it; otherwise a hole
   the discourse resolver fills (D64), as for demonstratives. *Proposed:* both, the hole only when
   unstated.
   - *The hole is typed at the target's type.* The contrast class is an alternative to the target —
     to `MSI cell lines`, so a kind of cell line — the principle decision 5 applies to exclusives.
     Demonstrative holes are already typed by their restrictor (`these findings` resolves only to
     findings).
   - *Adjective and adverb.* The contrastive occurs as an adjective as well: `the top preferential
     dependency in MSI cell lines compared to MSS cell lines` (the CNL page). The table above counts
     the adverbs only. Written polymorphically, the adjective is the same operator restricting a
     noun.
   - *The stated path needs PP attachment first.* No construction handles `compared to`, and the one
     stated instance in the corpus — the sentence above — had its expected-reading pin removed on
     2026-07-25: the parser attaches the MSI context and the MSS contrast to the identifying event
     rather than to the dependency, and the correct reading is not derivable. It is the seam
     eigenius#264 records for the reading ranker.
   - *How `P` is lifted to classes is unstated.* `P(A) ∧ ¬P(B)` applies a predicate of individuals to
     classes without saying how — every member, generically, or on average — and the paper's methods
     define `preferentially` as a difference in mean dependency. A check of either half has to
     compute whichever it is.
4. **Where `significantly` and `independently` live.** In the claim, since the paper asserts them and
   the claims corpus measures what the proposition says; or at the justification layer only, beside
   the claim. *Proposed:* in the claim, **defined over what each is about**, so that each is checkable
   rather than a bare predicate as opaque as the `ontology:sole` this document replaces.
   - `significantly(e)`: `e`'s D52 record rejects its null at the record's alpha. The record already
     carries what this needs: `ontologies/statistics/statistics.esl` records the alpha, its test uses
     only `p < alpha`, and a one-sided test requires a witness.
   - `independently(f₁, f₂)`: over the two findings — chain resources, in the claim's domain — each
     finding's certificate has a support set citing none of the other's evidence. The justification
     layer computes this: `support` returns a certificate's alternative support sets, which
     `cited_iris` and `survives_without` read. The claim stays about findings, with the warrant
     structure as its definition, rather than quantifying over justification terms.
   - *Rejected:* the justification layer only — the paper's assertion would be missing from the
     proposition.
5. **What `Id` is taken at.** *Decided (2026-09-29).* `Id(A, x, y)` is the kernel's identity type at
   any type `A`; nothing about it is specific to `lexicon:Entity`. An exclusive is polymorphic in `A`,
   and `A` is the domain of the predicate it modifies — the exclusive never chooses a type:

   ```
   solely : Π(A : Set). (A → Prop) → A → Prop
   solely A P a  =  P a  ∧  ∀(y : A). P y → Id(A, y, a)
   ```

   This is the determiners' shape: `lexicon:forall_sem` is `fun (A : Set) => fun (V : A -> Prop) =>
   forall (x : A) => V(x)`, and every determiner in `closed-class.esl` is polymorphic in `T : Set` the
   same way.
   - **The alternatives range over `P`'s domain, not over `a`'s noun type.** They are whatever else
     `P` could hold of. In `p53 activity is not solely responsible for WRN dependence`, restricting `y`
     to activities excludes dMMR, a deficiency — the other contributor this document names — and
     leaves the paper's claim without its witness.
   - For the paper's verbs `A` is `lexicon:Entity`, because relations are typed over the entity top
     and specific types reach argument slots by coercion. That is `P`'s domain, not a property of
     equality; a predicate typed over something narrower gets a narrower `A`.
   - `a` reaches `A` by subtyping coercion, which is the identity on terms, so `Id` at `A` agrees with
     `Id` at `a`'s own type. The overgeneration *Types and the Structure of Meaning* §3.3.2 reports
     for equality under coercion comes from non-injective coercions into dot-types, which this
     lexicon does not have.
   - *Rejected:* equality fixed at `lexicon:Entity` whatever `P` is — `Id` is a type former at every
     type. Common nouns as setoids, each carrying its own identity criterion (Chatzikyriakidis & Luo
     2018, 2020) — `Id` is the equality. Distinctness is written `¬Id(A, y, a)` directly, with no
     intermediate definition standing in for it.
   - *Consequence, not addressed here:* `¬Id` between named individuals has no rule in the kernel.
     `DecEq` reduces to `Refl` on equal ground values and to a neutral otherwise, so it confirms
     equality and never proves distinctness. An exclusive claim can therefore be represented and
     Declared, as the paper does, but not checked. The sound route to checking is discrimination — a
     property one has and the other lacks, turned into `¬Id` by `J`. A unique-name assumption would be
     unsound: WordNet and UMLS name the same concepts, and alignment resolves that only for the
     surfaces it merges, at parse time, so an unmerged alias of `a` would pass as another witness.

## Slices

1. **Equality in ESL**: a surface form that encodes to the kernel's `Id`, so a meaning can say `y = a`.
2. **Exclusives** — `only`, `solely`, `alone`, and the scalar `merely`, `just`, `simply` — with
   decision 1's presupposition mechanism, replacing the opaque `ontology:sole`. Written in the
   determiners' shape (decision 5): polymorphic in `A`, instantiated at the modified predicate's
   domain, with distinctness `¬Id(A, y, a)`.
   - `only`, `solely` quantify over all of `A`.
   - `alone` quantifies over the co-occurring alternatives the sentence or its context names — still
     at type `A`; a discourse hole (D64) where they are not stated.
   - The scalar exclusives quantify over predicates ranked by `≻`, not over individuals, and use no
     `Id`.
   - Acceptance: `p53 activity … is not solely responsible for WRN dependence` admits a witness whose
     noun type differs from `a`'s. A version that takes `A` from `a` instead of from `P` fails it.
   - Depends on the carrier's `ProofObligation` arm (decision 1), which is not built. The `suggest`
     sentence in decision 1 is its plug test.
3. **Completeness and degree** — `fully`, `partially`, `substantially` — with decision 2's degree,
   and closed scales declared by hand for `explain`, `rescue` and `increase`.
4. **Contrastive** — `selectively`, `preferentially` and the adjective `preferential` — with decision
   3's contrast class. The unstated path needs only D64's hole; the stated path needs `compared to`
   attached to the right constituent, which the parser does not do today.
5. **Frequency and proportion** — `commonly`, `typically`, `predominantly`, `most commonly`.
6. **Statistical and evidential** — `significantly`, `independently` — per decision 4.
7. **`respectively`**, a coordination construction.

Each slice re-runs the WRN claims corpus; the 14 qualifier rows are its measure.

## Out of scope

Manner adverbs with no load on the claim (`carefully`, `rapidly` in a protocol). Modal hedges
(`suggest`, `argue`, `may`) — report verbs and modals, handled.
