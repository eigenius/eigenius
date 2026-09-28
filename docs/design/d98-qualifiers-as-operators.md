# D98 — Qualifiers as logical operators

**Status: proposed** (2026-09-28). Supersedes the inert/measurement split of
`docs/notes/d62-adverb-semantics-decision.md` for the qualifiers below. Decisions 1–4 are open.

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
(`Id(Entity, y, a)`), `≻` "stronger than" on a scale, `d(·)` a degree.

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

## Decisions (open)

1. **How a presupposition is carried.** `not solely P` needs the negation to reach the assertion and
   leave `P(a)` standing.
   - *Two dimensions:* a meaning is an at-issue proposition plus presupposed content; negation and
     the other operators act on the at-issue part, and the presupposition projects to the claim.
     This is the general mechanism — the same projection serves factives (`found that`), definites
     and `again` — and it is the presupposition arm D64's open-parse carrier left deferred.
   - *Lexicalized negated forms:* `not solely`, `not merely`, `not just`, `not simply` as their own
     operators, `λP.λa. P(a) ∧ ∃y. P(y) ∧ ¬(y = a)`. Exact for the paper's five uses; wrong wherever
     `only` meets any other operator, or `not` stands apart from the particle.
   - *Proposed:* two dimensions. The lexicalized forms get the paper's five sentences right and
     every other placement wrong.
2. **The degree of a verb.** A single operator `extent : (Entity → Entity → Prop) → Entity → Entity
   → float` over any relation, with the scale's bounds a property of the relation; or a degree
   function minted per verb sense, as `deg_X` is per adjective. *Proposed:* the single operator; a
   per-sense function would multiply the importers' output by every verb sense.
3. **The contrast class.** Taken from `compared to B` where stated; otherwise a hole the discourse
   resolver fills (D64), as for demonstratives. *Proposed:* both, the hole only when unstated.
4. **Where `significantly` and `independently` live.** As predicates in the claim over a statistics
   record or over the two warrants; or at the justification layer only, beside the claim. *Proposed:*
   in the claim, pointing at the record — the paper asserts them.

## Slices

1. **Equality in ESL**: a surface form that encodes to the kernel's `Id`, so a meaning can say `y = a`.
2. **Exclusives** — `only`, `solely`, `alone`, and the scalar `merely`, `just`, `simply` — with
   decision 1's presupposition mechanism, replacing the opaque `ontology:sole`.
3. **Completeness and degree** — `fully`, `partially`, `substantially` — with decision 2's degree.
4. **Contrastive** — `selectively`, `preferentially` — with decision 3's contrast class.
5. **Frequency and proportion** — `commonly`, `typically`, `predominantly`, `most commonly`.
6. **Statistical and evidential** — `significantly`, `independently` — per decision 4.
7. **`respectively`**, a coordination construction.

Each slice re-runs the WRN claims corpus; the 14 qualifier rows are its measure.

## Out of scope

Manner adverbs with no load on the claim (`carefully`, `rapidly` in a protocol). Modal hedges
(`suggest`, `argue`, `may`) — report verbs and modals, handled.
