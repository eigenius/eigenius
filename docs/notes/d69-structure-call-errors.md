# Structure-call errors on the WRN page — analyses draws of 2026-09-30

Analysed 2026-10-01 (eigenius#264; summary in `d69-reading-presentation.md` §7q). The assessments of
pins and ledger rows below are proposals for the maintainer, not rulings.

Inputs: the six live draws `experiments/parsing/selections/2026-09-30-analyses-{jev-latest,claude-sonnet-4-6}-{1,2,3}.json` (built at 52cdff7); the decision dump of the same forest, written by the harness under `EIGENIUS_DUMP_DECISIONS` over a replay of `2026-09-30-analyses-jev-latest-1` (not committed; re-create it with `scripts/measure-parse-rate.sh --replay experiments/parsing/ranks/2026-09-29-d95-slice8.json --selections experiments/parsing/selections/2026-09-30-analyses-jev-latest-1.json` and `EIGENIUS_DUMP_DECISIONS=<file>`); the pins `expected-readings.tsv`; the ledger `reading-adjudications.tsv`.

Status: since 85f40b9 the sense call asks one question per word, with no `none` option (§7q). The
abstentions discussed under P8 and recommendation 7 came from the whole-reading sense call these
draws used.

## Method

- 27 of the 41 units get a structure call; 81 structure decisions per model. The other 14 units have one analysis.
- In all 158 recorded `STRUCTURE n` decisions the chosen reading lies in dump group n−1, and each draw's candidate list equals the dump's (by `sem`). `analysis.rs`, `verbalize.rs` and `structure_groups` are unchanged between 52cdff7 and HEAD.
- jev abstained 4 times after a structure call. An abstained record has an empty rationale: in `DecisionReadingRanker::select` the sense call's `?` returns `None`, which drops the structure account already built. The structure choice is recovered from the draw logs: the sense call lists the chosen group's readings plus `none`, and group sizes are distinct. «Some cancers…» jev1–3: indices 0–7 → option 4 (8 readings). «The use of immune checkpoint blockade…» jev1: indices 0–143 → option 3 (144 readings). Marked `*`.
- The pin is offered in all 27 structure calls. No call was truncated (largest: 6 options; cap 12). The pin is in the forest for all 41 units.
- The counts reconcile with the harness: structure-correct jev 31/38, 32/39, 30/38; sonnet 32/41 in each draw.

## Summary

Non-pin structure-call choices: **jev 25 of 81, sonnet 24 of 81**, on 10 units.

| cause | jev | sonnet | units |
|---|---|---|---|
| (a) presentation | 9 | 12 | U4, U6, U7, U9 |
| (b) pin or ledger questionable | 15 | 12 | U1, U2, U3, U5, U10 (jev) |
| (c) model error under a clear presentation | 0 | 0 | — |
| (d) upstream, as the primary cause | 0 | 0 | contributes in U2, U4, U7, U9 |
| undetermined | 1 | 0 | U8 (structure account not recorded) |
| **total** | **25** | **24** | |

| unit | jev | sonnet | pin opt | chosen | cause |
|---|---|---|---|---|---|
| U1 «Scientists can exploit synthetic lethality for cancer therapeutics.» | 3 | 3 | 2 | 1 (all) | (b) open question; pin was the sole reading when pinned |
| U2 «Many cancers exhibit an impairment of a DNA repair pathway.» | 3 | 3 | 1 | jev 4, son 3 | (b) stale pin: only the C0000702 «DNA, A-Form» readings match it; (d) lexicon |
| U3 «MSI results from deficient DNA mismatch repair.» | 3 | 3 | 2 | 3 (all) | (b) ledger ground false; bracket display decides (a) |
| U4 «Depletion of WRN induced double-stranded DNA breaks.» | 3 | 3 | 1 | 4 (all) | (a) one-term option invisible; degree adjective hidden (d) |
| U5 «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.» | 3 | 3 | 1 | 2 (all) | (b) pin narrower than ledger (`departs` rows accept option 2) |
| U6 «Nucleotide repeat regions are microsatellites.» | 2 | 3 | 2 | 1 (5/6) | (a) identical analyses, abstract predication labels |
| U7 «Some cancers do not respond to immune checkpoint blockade.» | 3* | 3 | 3 | 4 (all) | (a) one-term option unbracketed; (d) every decomposition malformed |
| U8 «The use of immune checkpoint blockade can be limited by toxicity.» | 1* | 0 | 2 | jev1 3 | undetermined (U7 pattern) |
| U9 «The four other RecQ DNA helicases were not preferentially essential in MSI cell lines.» | 1 | 3 | 2 | 1 (4/6) | (a) `[The four]` with no function line; (d) the_count has no link |
| U10 «WRN dependency may require specific lineages or a stronger mutation phenotype.» | 3 | 0 | 2 | jev 1, son 2 | (b) options logically equivalent; no function lines |

Outside the structure call, 4 sense-call decisions changed the skeleton through gene vs protein: sonnet ×3 on U10 (structure call chose the pin's option), jev3 on «WRN encodes a RecQ DNA helicase.» (no structure call). The harness counts them as structure misses.

31 of the 49 non-pin choices (U2, U3, U4, U5, U7, U8) are choices between a lexicalised multi-word concept and its decomposition.

Both models chose the same non-pin option on U1, U3, U4, U5, U6, U7, U9 (jev 1 of 3 on U9, 2 of 3 on U6). They differ on U2 (jev 4, sonnet 3; both ledger-faithful) and U10 (jev option 1; sonnet the pin's option).

Option blocks below are verbatim from the dump. `PIN` marks the option holding the pinned skeleton; `<-` lists the draws that chose it.

## Per unit


### U1 «Scientists can exploit synthetic lethality for cancer therapeutics.»

```text
[1]     Scientists can [exploit synthetic lethality] for cancer therapeutics.   <- jev1 jev2 jev3 son1 son2 son3
        - «for therapeutics» is an adverbial of «exploit»: it says how, where, when or why
[2] PIN Scientists can exploit [synthetic lethality for cancer therapeutics].
        - «for therapeutics» postmodifies «synthetic lethality»: it says which or what kind of synthetic lethality
```

jev probabilities: jev1: 1=0.98, 2=0.02, none=0.00 · jev2: 1=0.98, 2=0.02, none=0.00 · jev3: 1=0.96, 2=0.04, none=0.00

sonnet reasons:
> son1: In context, the sentence means that scientists can use synthetic lethality as a strategy *in order to develop/achieve* cancer therapeutics — i.e., "for cancer therapeutics" expresses the purpose of the exploiting action, functioning as an adverbial modifier of "exploit." There is no implication that synthetic lethality is being partitioned into a subtype specifically "for cancer therapeutics" (as opposed to some other kind), which is what option 2 would require.
> son2: In context, the sentence means that scientists can use (exploit) synthetic lethality as a strategy *for the purpose of* developing cancer therapeutics — "for cancer therapeutics" expresses the purpose/goal of the exploiting action, functioning as an adverbial modifier of "exploit." It does not restrict which kind of synthetic lethality is meant (option 2), since synthetic lethality has already been defined generically in the preceding sentences.
> son3: In context, "synthetic lethality" is a general concept (already defined as an interaction between two genetic events), not a subcategory called "synthetic lethality for cancer therapeutics." The phrase "for cancer therapeutics" expresses the purpose of the exploiting action — i.e., scientists exploit synthetic lethality *in order to achieve* cancer therapeutics — making it an adverbial modifier of "exploit" rather than a postmodifier restricting which synthetic lethality is meant.

**Pin offered:** yes (option 2). **Cause: (b)** — the open maintainer question. **Models agree** (6/6).

- The pin was recorded 2026-07-28 as «SOLE reading and correct» (expected-readings.tsv). The verb attachment did not exist then, so the noun attachment was never adjudicated against it.
- «exploit X for Y» states the purpose of the exploiting. The 2026-09-30 library ruling moved the instrument PPs of «screened»/«analysed» to the verb; a purpose PP on «exploit» is the same kind of verb adjunct.
- jev puts 0.96–0.98 on the adverbial. Sonnet's three reasons: «synthetic lethality» was defined generically earlier, so a postmodifier saying «which or what kind of synthetic lethality» has nothing to restrict.
- The postmodifier template («it says which or what kind of X») presents every noun attachment as restrictive; for a named concept that reading is implausible. If the maintainer keeps the noun attachment, this template biases against it.
- Encoding (does not affect the choice): the adverbial reading is `And(v01164273_t(kind_of(C4280020), kind_of(n10560637)), prep_for(kind_of(n10560637), …therapeutic…))` — the verb-adjunct encoding predicates the purpose of the *scientists*. A re-pin to the verb would pin «scientists are for cancer therapeutics».


### U2 «Many cancers exhibit an impairment of a DNA repair pathway.»

```text
[1] PIN Many cancers exhibit an impairment of [[a DNA] repair] pathway.
        - «a DNA» is a noun modifier of «repair»
        - «repair» is a noun modifier of «pathway»
        - «of pathway» postmodifies «impairment»: it says which or what kind of impairment
[2]     Many cancers exhibit an impairment of a [DNA repair pathway].
        - «of DNA repair pathway» postmodifies «impairment»: it says which or what kind of impairment
[3]     Many cancers exhibit an impairment of a [[DNA repair] pathway].   <- son1 son2 son3
        - «DNA repair» is a noun modifier of «pathway»
        - «of pathway» postmodifies «impairment»: it says which or what kind of impairment
[4]     Many cancers exhibit an impairment of a [[DNA repair] pathway].   <- jev1 jev2 jev3
        - «DNA» is a noun modifier of «repair»
        - «repair» is a noun modifier of «pathway»
        - «of pathway» postmodifies «impairment»: it says which or what kind of impairment
```

jev probabilities: jev1: 4=0.65, 3=0.29, 2=0.04, 1=0.02, none=0.00 · jev2: 4=0.61, 3=0.32, 2=0.05, 1=0.02, none=0.00 · jev3: 4=0.63, 3=0.31, 2=0.04, 1=0.02, none=0.00

sonnet reasons:
> son1: In context, "DNA repair" is a fixed compound noun (a biological process) that together modifies "pathway," giving "DNA repair pathway" — a pathway belonging to the DNA repair process. Option 3 correctly brackets [[DNA repair] pathway] and correctly describes "DNA repair" as a noun modifier of "pathway," while option 4 wrongly decomposes it as "DNA" modifying "repair" and "repair" modifying "pathway" (treating them as two separate, chained modifiers rather than a compound). Option 2 is also consistent with the meaning but fails to show the internal [[DNA repair] pathway] bracketing structure explicitly.
> son2: The document discusses "DNA repair pathway" as a compound noun where "DNA repair" (a well-established biological process) functions as a unit that modifies "pathway," giving "a DNA-repair pathway." The impairment is of that pathway, so "of [DNA repair pathway]" postmodifies "impairment." Option 3 correctly captures this bracketing and correctly describes "DNA repair" as a noun modifier of "pathway" (treating "DNA repair" as a compound), without incorrectly splitting "DNA" and "repair" into separate modifier layers as option 4 does.
> son3: The document consistently treats "DNA repair" as a compound noun (a biological process), and "DNA repair pathway" as a pathway of that process — so "DNA repair" modifies "pathway" as a unit. Option 3 correctly brackets [[DNA repair] pathway] and correctly identifies "DNA repair" as a noun modifier of "pathway," without the error in option 4 of decomposing "DNA repair" into "DNA" modifying "repair" separately (which would misrepresent the compound). Option 3 also correctly notes that "of pathway" postmodifies "impairment."

**Pin offered:** yes (option 1), and it is the analysis the ledger rejects. **Cause: (b)** stale pin; **(d)** lexicon. **Models disagree** (jev 4, sonnet 3); both choices are ledger-faithful.

- All 4 readings with the pinned skeleton take tokens 6–7 «a DNA» as C0000702 «DNA, A-Form» (gloss: «… + compound-with «repair» [n00266806] + compound-with «DNA, A-Form» [C0000702]»). The article is absorbed into the term, the NP comes out bare, and that yields the pin's `kind_of(compound chain)`.
- The pin note describes `compound_kind(pathway, compound_kind(repair, DNA))` — DNA, not its A-form. Option 4 is that chain under the existential «a»: `ΠG#5:ΣG#5:§. compound_kind(G#5, ΣG#6:§. compound_kind(G#6, §))`.
- Ledger: row 132 (BEST, option 2, C1511689 «DNA Repair Pathway») says it «avoids the rival readings that … land on C0000702 «DNA, A-Form»». Row 210 (option-4 shape) and row 219 (option-3 shape) are `correct, departs`. Row 196 (sonnet's reading) is wrong for the sense n14561618 only and calls the option-3 structure «a decomposition of the BEST row's C1511689, not a ground».
- Option 1's function line «a DNA» is a noun modifier of «repair» exposes the defect; neither model chose it (jev ≤0.02). Sonnet prefers 3 over 2 because option 2 «fails to show the internal [[DNA repair] pathway] bracketing» — the P1 preference for visible structure.


### U3 «MSI results from deficient DNA mismatch repair.»

```text
[1]     MSI results from deficient DNA mismatch repair.
        - «from deficient DNA mismatch repair» is an adverbial of «results»: it says how, where, when or why
[2] PIN MSI results from deficient [DNA [mismatch repair]].
        - «DNA» is a noun modifier of «mismatch repair»
        - «deficient» is an adjective describing «mismatch repair»
        - «from mismatch repair» is an adverbial of «results»: it says how, where, when or why
[3]     MSI results from deficient [DNA mismatch repair].   <- jev1 jev2 jev3 son1 son2 son3
        - «deficient» is an adjective describing «DNA mismatch repair»
        - «from DNA mismatch repair» is an adverbial of «results»: it says how, where, when or why
```

jev probabilities: jev1: 3=0.52, 2=0.33, 1=0.14, none=0.01 · jev2: 3=0.47, 2=0.38, 1=0.14, none=0.01 · jev3: 3=0.51, 2=0.37, 1=0.11, none=0.01

sonnet reasons:
> son1: In the document, "DNA mismatch repair" is a fixed compound noun (a specific biological repair pathway), and "deficient" is an adjective modifying that entire compound noun phrase. The prepositional phrase "from DNA mismatch repair" (deficient) functions as an adverbial explaining the cause/origin of MSI. Option 2 incorrectly treats "DNA" as a modifier of just "mismatch repair" and "from mismatch repair" as the adverbial, losing "DNA" from the prepositional phrase, which misrepresents the structure.
> son2: In the document, "DNA mismatch repair" is a fixed compound noun (a specific biological repair pathway), and "deficient" is an adjective modifying the entire compound "DNA mismatch repair" — meaning the repair pathway itself is deficient. Option [3] correctly captures this bracketing and function. Option [2] incorrectly treats "DNA" as a modifier of "mismatch repair" separately, implying "deficient mismatch repair" is the head modified by "DNA," which misrepresents the standard biological terminology where "DNA mismatch repair" is the established compound.
> son3: In the document, "DNA mismatch repair" is a fixed compound noun (a specific biological repair pathway), and "deficient" is an adjective modifying that entire compound noun phrase. The sentence means MSI arises from a deficient [DNA mismatch repair] system, not that "DNA" modifies only "mismatch repair" while "deficient" modifies just "mismatch repair." Option 3 correctly brackets "DNA mismatch repair" as a single unit modified by "deficient," with "from DNA mismatch repair" serving as the adverbial of "results."

**Pin offered:** yes (option 2). **Cause: (b)** — the ledger ground is false; which of options 2 and 3 a model picks is decided by bracket display **(a)**. **Models agree** (6/6).

- `senses_at`: option 3's C1155661 «Mismatch Repair» spans tokens 4–6 «DNA mismatch repair»; option 2's spans 5–6 «mismatch repair» with «DNA» n14830364 compounded on. Option 3 lexicalises the whole term. Nothing is dropped.
- Row 59 rules option 3 `wrong`, ground `structure`: «drops the 'DNA mismatch repair' compound». The pin note says the same («loses the DNA compound»). Both are false. The ledger accepts the parallel lexicalisations: row 153, option 1, «deficient DNA mismatch repair» = C4522088, `correct, departs`; row 132, C1511689 for «DNA repair pathway». Option 2 compounds DNA onto a concept that already is DNA mismatch repair.
- jev: 3 = 0.47–0.52, 2 = 0.33–0.38. Sonnet: «"DNA mismatch repair" is a fixed compound noun … Option 3 correctly brackets "DNA mismatch repair" as a single unit».

**Control — «Defects in DNA mismatch repair promote a hypermutable state.»** (not a miss; shown because it isolates the cause)


```text
[1]     Defects in DNA mismatch repair promote a hypermutable state.
        - «in DNA mismatch repair» postmodifies «Defects»: it says which or what kind of Defects
[2] PIN Defects in DNA [mismatch repair] promote a hypermutable state.   <- jev1 jev2 jev3 son1 son2 son3
        - «DNA» is a noun modifier of «mismatch repair»
        - «in mismatch repair» postmodifies «Defects»: it says which or what kind of Defects
```

jev probabilities: jev1: 2=0.64, 1=0.35, none=0.01 · jev2: 2=0.59, 1=0.40, none=0.01 · jev3: 2=0.64, 1=0.35, none=0.01

sonnet reasons:
> son1: In the document, "DNA mismatch repair" is a compound noun referring to a specific biological process (the MMR pathway), where "DNA" modifies "mismatch repair" as a unit. The sentence is saying that defects *in* [DNA mismatch repair] promote a hypermutable state — i.e., "DNA mismatch repair" is the thing being defected in, not "DNA" being a separate modifier of "Defects." Option 2 correctly brackets "DNA mismatch repair" as the compound and treats "in [DNA mismatch repair]" as the postmodifying prepositional phrase on "Defects."
> son2: In the document, "DNA mismatch repair" is a compound noun referring to a specific biological process (the MMR pathway), where "DNA" modifies "mismatch repair" as a whole — not a separate prepositional phrase "in DNA mismatch repair." The defects are defects *in* [DNA mismatch repair], treating "DNA mismatch repair" as the unified noun phrase being modified by "in." Option 2 correctly captures this by bracketing "DNA mismatch repair" as the object of "in," with "DNA" as a noun modifier of "mismatch repair."
> son3: In the document, "DNA mismatch repair" is a compound noun referring to a specific biological process (the MMR pathway), where "DNA" modifies "mismatch repair" as a whole. The defects are defects *in* that compound process (DNA mismatch repair), so "DNA" attaches to "mismatch repair" rather than to "Defects." Option 2 correctly captures this bracketing: [DNA mismatch repair] is the noun phrase, and "in [DNA mismatch repair]" postmodifies "Defects."

- Here the lexicalised option (1, C1155661 over tokens 2–4) has no bracket: both analyses build span 2–4, so it is common. The compositional pin shows «DNA [mismatch repair]».
- All six draws chose the pin. Sonnet's argument is the one it gives on U3 («"DNA mismatch repair" is a compound noun … as a unit»), and it takes option 2's «DNA [mismatch repair]» for that unit: «Option 2 correctly brackets "DNA mismatch repair" as the compound». Option 1 shows no structure inside the term at all. Same reasoning, opposite structure; in both units the option with a visible bracket inside «… DNA mismatch repair» wins.
- The pin note here («The rejected skeleton makes it atomic and drops DNA») repeats U3's false ground.


### U4 «Depletion of WRN induced double-stranded DNA breaks.»

```text
[1] PIN Depletion of WRN induced double-stranded DNA breaks.
        - «double-stranded DNA breaks» is the object of «induced»
[2]     Depletion of WRN induced double-stranded [DNA breaks].
        - «breaks» is the object of «induced»
        - «DNA» is a noun modifier of «breaks»
        - «double-stranded» is an adjective describing «breaks»
[3]     Depletion of WRN induced [double-stranded DNA] breaks.
        - «breaks» is the object of «induced»
        - «double-stranded DNA» is a noun modifier of «breaks»
[4]     Depletion of WRN induced double-stranded [DNA breaks].   <- jev1 jev2 jev3 son1 son2 son3
        - «DNA breaks» is the object of «induced»
        - «double-stranded» is an adjective describing «DNA breaks»
```

jev probabilities: jev1: 2=0.29, 4=0.29, 3=0.25, 1=0.16, none=0.01 · jev2: 4=0.35, 2=0.24, 3=0.23, 1=0.17, none=0.01 · jev3: 4=0.34, 2=0.29, 3=0.21, 1=0.15, none=0.01

sonnet reasons:
> son1: "DNA breaks" is a fixed compound noun (a recognized biological term for strand breaks in DNA), and "double-stranded" is an adjective that modifies the entire compound "DNA breaks," specifying that the breaks are in double-stranded DNA. This matches the biological meaning in the document, where "double-stranded DNA breaks" refers to a specific type of DNA damage (DSBs). Option 3 misreads "double-stranded DNA" as a modifier of "breaks," treating "double-stranded" as part of the noun modifier rather than an adjective on the head noun phrase; option 2 treats "DNA" alone as the modifier of "breaks" and "double-stranded" as a separate adjective, which fails to capture that "DNA breaks" is the established compound.
> son2: In the biological context, "DNA breaks" is a fixed compound noun referring to breaks in DNA strands, and "double-stranded" is an adjective modifying the entire compound "DNA breaks," specifying that the breaks affect double-stranded DNA (i.e., both strands are broken). This is the standard terminology in molecular biology (double-stranded DNA breaks = DSBs), where "double-stranded" describes the nature of the break, not the DNA alone.
> son3: "DNA breaks" is a fixed compound noun (a recognized biological term for strand breaks in DNA), and "double-stranded" is an adjective modifying the entire compound "DNA breaks," specifying that the breaks are in double-stranded DNA. This matches the standard biological meaning: depletion of WRN causes double-stranded DNA breaks (DSBs), a well-established concept in molecular biology.

**Pin offered:** yes (option 1). **Cause: (a)**, with **(d)** contributing. **Models agree** (6/6).

- Option 1 renders as the bare sentence. Its leaf C1511667 «DNA Breaks, Double-Stranded» spans tokens 4–6; options 2–4 also build a constituent over 4–6, so `analysis::analyses` drops the span as common. Its one function line («double-stranded DNA breaks» is the object of «induced») does not say the three words are one concept.
- jev is near uniform: draw 1 2=0.29, 4=0.29, 3=0.25, 1=0.16. Sonnet's reasons compare options 2, 3 and 4 and never mention option 1.
- What makes option 4 wrong is not shown. «double-stranded» there is the degree term `gt(deg_a02251576(x), std_a02251576)` («more double-stranded than the norm»), rendered ««double-stranded» is an adjective describing «DNA breaks»». Options 2 and 4 carry the degree; option 3 (C0311474 «DNA, Double-Stranded» ⊗ break) does not, and rows 193/194 call it «a decomposition … not a ground». Row 233 rules option 4 wrong on structure. The acceptable set is {1, 3}; both models chose 4.


### U5 «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.»

```text
[1] PIN Depletion of WRN [promoted [apoptosis and [cell cycle arrest]]] in MSI models.
        - «cell cycle arrest» is the object of «promoted»
        - «in models» is an adverbial of «promoted»: it says how, where, when or why
[2]     Depletion of WRN [promoted [apoptosis and [[cell cycle] arrest]]] in MSI models.   <- jev1 jev2 jev3 son1 son2 son3
        - «arrest» is the object of «promoted»
        - «cell cycle» is a noun modifier of «arrest»
        - «in models» is an adverbial of «promoted»: it says how, where, when or why
[3]     Depletion of WRN promoted [apoptosis and [[cell cycle] [arrest in MSI models]]].
        - «arrest» is the object of «promoted»
        - «cell cycle» is a noun modifier of «arrest»
        - «in models» postmodifies «arrest»: it says which or what kind of arrest
[4]     Depletion of WRN promoted [apoptosis and [[cell cycle arrest] in MSI models]].
        - «cell cycle arrest» is the object of «promoted»
        - «in models» postmodifies «cell cycle arrest»: it says which or what kind of cell cycle arrest
```

jev probabilities: jev1: 2=0.56, 1=0.38, 3=0.03, 4=0.03, none=0.00 · jev2: 2=0.60, 1=0.31, 3=0.04, 4=0.04, none=0.01 · jev3: 2=0.60, 1=0.33, 3=0.04, 4=0.03, none=0.00

sonnet reasons:
> son1: The sentence means that WRN depletion promoted two things—apoptosis and cell cycle arrest—in MSI models, where "in MSI models" is an adverbial modifying "promoted" (specifying the context/setting of the promotion), not a postmodifier of "arrest." "Cell cycle" is a compound noun modifier of "arrest," making "cell cycle arrest" the intended noun phrase as the second conjunct. This matches option 2, which correctly brackets [[cell cycle] arrest] as the object and treats "in MSI models" as an adverbial of "promoted."
> son2: The sentence means that WRN depletion promoted two things—apoptosis and cell cycle arrest—in MSI models, where "in MSI models" is an adverbial modifying "promoted" (specifying the context/setting of the promotion), not a postmodifier of "arrest." "Cell cycle" is a compound noun modifier of "arrest," making the object "cell cycle arrest." This matches option 2. Option 4 is a runner-up but treats "in MSI models" as postmodifying "cell cycle arrest" rather than as an adverbial of "promoted," which is less natural given the parallel construction with sentence 13 ("WRN was selectively essential in MSI models") and the consistent pattern throughout the document of using "in MSI models" as a locative adverbial.
> son3: The sentence means that WRN depletion promoted two things—apoptosis and cell cycle arrest—in MSI models, where "in MSI models" is an adverbial modifying "promoted" (specifying the context/setting of the promotion), not a postmodifier of "arrest." "Cell cycle" is a compound noun modifier of "arrest," making the object "cell cycle arrest." This matches option 2. Option 4 is similar but wrongly treats "in MSI models" as postmodifying "cell cycle arrest" rather than as an adverbial of "promoted"; the document's consistent pattern (e.g., sentence 13: "WRN is essential … in a MSI model") treats "in MSI models" as an adverbial of the main verb.

**Pin offered:** yes (option 1). **Cause: (b)** — the pin is narrower than the ledger. **Models agree** (6/6).

- Options 1 and 2 differ only in «cell cycle arrest» = C1155873 vs «cell cycle» ⊗ «arrest». Both attach «in MSI models» to the verb, as the maintainer ruled on 2026-09-30; all six draws chose that attachment.
- The pin note: «The compound variant is the same structure and is recorded `available`». Rows 207 and 237 mark jev's option-2 readings `correct, departs`. Sonnet's option-2 reading (row 241) is wrong for the sense n14017332 only. The 6 structure misses are 3 ledger-correct readings and 3 sense errors.
- Option 1 «[cell cycle arrest]» and option 2 «[[cell cycle] arrest]» differ only in an inner bracket; nothing says option 1 is a named concept. Choosing the more-bracketed option matches P1. jev 2 = 0.56–0.60, 1 = 0.31–0.38.


### U6 «Nucleotide repeat regions are microsatellites.»

```text
[1]     Nucleotide repeat regions are microsatellites.   <- jev1 jev2 son1 son2 son3
        - a statement about the subject's kind as a whole
[2] PIN Nucleotide repeat regions are microsatellites.   <- jev3
        - a generalisation: it says something of every member of the subject's class
```

jev probabilities: jev1: 1=0.54, 2=0.45, none=0.01 · jev2: 1=0.50, 2=0.49, none=0.01 · jev3: 2=0.50, 1=0.49, none=0.01

sonnet reasons:
> son1: In context, the sentence "Nucleotide repeat regions are microsatellites" is providing a definitional/terminological equivalence — it is saying that the category "nucleotide repeat regions" is (i.e., is called / is the same thing as) "microsatellites." This is a statement about the subject kind as a whole (an identity/definition claim), not a generalisation predicated of each individual member (e.g., it is not saying "every nucleotide repeat region has property X"). Option 1 captures this whole-kind definitional reading.
> son2: In context, the sentence is providing a definitional/terminological clarification — "nucleotide repeat regions" as a category are what scientists call "microsatellites." This is a statement about the kind (i.e., what the subject class *is*), not a universal generalisation about each individual member doing or having something. It equates the two terms as co-referential categories, functioning as a nominal definition rather than a predication over every individual member.
> son3: In context, the sentence is defining what nucleotide repeat regions are called — it equates the category "nucleotide repeat regions" with the term "microsatellites" as a whole-class identity statement (a definitional/taxonomic claim), not a universal generalisation about each individual member. The sentence functions as a naming/classification statement: the kind "nucleotide repeat regions" is (the same thing as) the kind "microsatellites."

**Pin offered:** yes (option 2). **Cause: (a)**; **(c)** secondary for sonnet. **Models agree** (5/6).

- The two analyses are character-identical; only the predication line differs. jev is at chance: 0.54/0.45, 0.50/0.49, 0.49/0.50.
- Option 1's term is `is_a(kind_of(NRR), C1519302)`: the kind itself is one Short Tandem Repeat. Option 2's is `subclass_of(NRR, C1519302)`: every nucleotide repeat region is one. Sonnet reads «a statement about the subject's kind as a whole» as a definitional equation of two categories («the kind "nucleotide repeat regions" is (the same thing as) the kind "microsatellites"») — which is the subclass claim — and reads «generalisation» as property ascription («not … "every nucleotide repeat region has property X"»). The labels do not state the consequence of either term.
- The same label is the correct answer on «Synthetic lethality is an interaction between two genetic events.» (mass subject; pin = «kind as a whole»; 6/6 correct there, decided by the PP attachment). Which label is right depends on the subject, so a model cannot learn the labels; a paraphrase in the sentence's words carries the difference.


### U7 «Some cancers do not respond to immune checkpoint blockade.»

```text
[1]     Some cancers do not respond to immune checkpoint blockade.
        - «to immune checkpoint blockade» is an adverbial of «respond»: it says how, where, when or why
[2]     Some cancers do not respond to immune [checkpoint blockade].
        - «checkpoint» is a noun modifier of «blockade»
        - «immune» is an adjective describing «blockade»
        - «to blockade» is an adverbial of «respond»: it says how, where, when or why
[3] PIN Some cancers do not respond to immune checkpoint blockade.
        - «immune checkpoint blockade» is the prepositional object of «respond», which takes it through a preposition
[4]     Some cancers do not respond to immune [checkpoint blockade].   <- jev1* jev2* jev3* son1 son2 son3
        - «blockade» is the prepositional object of «respond», which takes it through a preposition
        - «checkpoint» is a noun modifier of «blockade»
        - «immune» is an adjective describing «blockade»
```

jev probabilities: jev1: (abstained; rationale empty) · jev2: (abstained; rationale empty) · jev3: (abstained; rationale empty)

sonnet reasons:
> son1: In context, "immune checkpoint blockade" is a fixed medical term where "checkpoint" modifies "blockade" and "immune" is an adjective on "blockade" (forming the noun phrase "immune checkpoint blockade"), and the whole phrase is the prepositional object of "respond to" — i.e., "respond" takes its complement through the preposition "to," with "blockade" as the head noun. Option 4 correctly identifies this structure, distinguishing the prepositional object (blockade) from its modifiers, consistent with the prior reading in sentence 31 ("Immune Checkpoint Blockade" as a named entity).
> son2: In context, "immune checkpoint blockade" is a compound noun phrase where "checkpoint" modifies "blockade" and "immune" is an adjective on "blockade" (the whole phrase names a specific therapeutic approach). The verb "respond" takes a prepositional complement via "to," making "blockade" (the head noun of the compound) the prepositional object. This matches option 4, which correctly identifies the internal structure of the noun phrase and the grammatical role of the whole PP. Option 3 is close but labels the entire "immune checkpoint blockade" as the prepositional object without decomposing the noun phrase, and option 1 mislabels the PP as an adverbial rather than a complement.
> son3: In this context, "immune checkpoint blockade" is a fixed medical term where "checkpoint" modifies "blockade" and "immune" is an adjective on "blockade" (not on "checkpoint"), making "blockade" the prepositional object of "respond to." Option 3 treats "immune checkpoint blockade" as a single undifferentiated unit without specifying the internal structure, while option 4 correctly captures both the prepositional-object relationship and the internal noun-phrase structure. The prior readings (e.g., sentence 31: "Immune Checkpoint Blockade") confirm this is a compound noun headed by "blockade."

**Pin offered:** yes (option 3). **Cause: (a)**, with **(d)** contributing. **Models agree** (6/6; jev inferred).

- Options 1 and 3 render identically (no bracket), as do 2 and 4. ICB as C5392067 spans tokens 6–8, which every analysis builds, so it gets no bracket.
- Sonnet rejects option 1 correctly (adverbial vs governed complement), then takes 4 over 3: «Option 3 treats "immune checkpoint blockade" as a single undifferentiated unit without specifying the internal structure». It cites the prior selection of «Immune Checkpoint Blockade» as a named entity and still chooses the decomposition.
- Sonnet contradicts itself across units. On U8 the lexicalised ICB has a bracket and sonnet takes it 3/3, rejecting the decomposition because it «parses "immune" as modifying "blockade" alone rather than the full compound». On U7, son3: «"immune" is an adjective on "blockade" (not on "checkpoint")».
- Upstream: no reading in either ICB forest builds [immune checkpoint] (0 of 27 here, 0 of 171 in U8), and «checkpoint» seeds only C1155874 «Cell Cycle Checkpoints». Every compositional option is malformed; C5392067 is the only faithful one. Options 2 and 4 each merge two skeletons (`gt(deg_a02365900…)` and `a02745862(x)`) under one «adjective describing» line.
- jev took option 4 in all three draws (inferred from the 8-reading sense call), then answered `none` in the sense call (0.74–0.77): it rejects option 4's senses, one call after choosing its structure.


### U8 «The use of immune checkpoint blockade can be limited by toxicity.»

```text
[1]     The [use of] immune checkpoint blockade can be limited by toxicity.
        - «immune checkpoint blockade» is the object of «limited»
        - «use of» is a noun modifier of «immune checkpoint blockade»
[2] PIN The use [of immune checkpoint blockade] can be limited by toxicity.   <- jev2 jev3 son1 son2 son3
        - «use» is the object of «limited»
        - «of immune checkpoint blockade» postmodifies «use»: it says which or what kind of use
[3]     The use [of immune [checkpoint blockade]] can be limited by toxicity.   <- jev1*
        - «use» is the object of «limited»
        - «checkpoint» is a noun modifier of «blockade»
        - «immune» is an adjective describing «blockade»
        - «of blockade» postmodifies «use»: it says which or what kind of use
```

jev probabilities: jev1: (abstained; rationale empty) · jev2: 2=0.53, 3=0.44, 1=0.02, none=0.01 · jev3: 2=0.55, 3=0.41, 1=0.03, none=0.01

sonnet reasons:
> son1: The sentence is saying that "the use" (of immune checkpoint blockade) is what can be limited by toxicity — i.e., "use" is the head noun and subject/object, with "of immune checkpoint blockade" as a postmodifying prepositional phrase specifying which use. Option 3 is structurally similar but misanalyses "immune" as modifying "blockade" alone rather than the full compound noun "immune checkpoint blockade," which is a fixed medical term where "immune checkpoint" together modifies "blockade."
> son2: The sentence is saying that the *use* (of immune checkpoint blockade) can be limited by toxicity — "use" is the head noun being limited, and "of immune checkpoint blockade" is a postmodifying prepositional phrase specifying which use. This is consistent with the prior sentence's reading (sentence 32) where "Cell Cycle Checkpoints Blockade" is treated as a unified concept, and with the document's meaning that toxicity constrains the applicability/use of the therapy. Option 1 wrongly makes "immune checkpoint blockade" the object of "limited," and option 3 incorrectly parses "immune" as modifying "blockade" alone rather than the full compound "immune checkpoint blockade."
> son3: The sentence is saying that the *use* (of immune checkpoint blockade) can be limited by toxicity — "use" is the head noun being limited, and "of immune checkpoint blockade" is a postmodifying prepositional phrase specifying which use. This is consistent with the prior reading of sentence 32, where "Cell Cycle Checkpoints Blockade" is treated as a unified concept, and with the document's meaning that toxicity constrains the applicability/use of the therapy. Option 1 wrongly makes "immune checkpoint blockade" the object of "limited," and option 3 incorrectly parses "immune" as modifying "blockade" alone rather than the full compound "immune checkpoint blockade."

**Pin offered:** yes (option 2). **Cause: undetermined** (U7 pattern). 1 of 6 draws.

- jev1 chose option 3 (inferred from its 144-reading sense call) and abstained with none=0.59. Its structure probabilities were not recorded. jev2/3 put 0.53–0.55 on 2 and 0.41–0.44 on 3 (margin 0.09–0.14); jev1's own probabilities are unknown, so the cause cannot be separated from a model error.
- Here the lexicalised ICB is visible («[of immune checkpoint blockade]»); sonnet chose it 3/3.


### U9 «The four other RecQ DNA helicases were not preferentially essential in MSI cell lines.»

```text
[1]     The [four other RecQ DNA helicases] were not preferentially essential in MSI cell lines.   <- jev3 son1 son2 son3
        - «four» is an adjective describing «DNA helicases»
[2] PIN [The four] other RecQ DNA helicases were not preferentially essential in MSI cell lines.   <- jev1 jev2
        (no function lines)
```

jev probabilities: jev1: 2=0.60, 1=0.39, none=0.01 · jev2: 2=0.61, 1=0.38, none=0.01 · jev3: 1=0.51, 2=0.48, none=0.01

sonnet reasons:
> son1: The sentence means that there are four other RecQ DNA helicases (besides WRN) that were not preferentially essential in MSI cell lines. In option [1], "four other RecQ DNA helicases" is correctly treated as the subject noun phrase, with "four" functioning as an adjective modifying "DNA helicases" — this matches the meaning. Option [2] incorrectly brackets "The four" as a unit separate from the rest of the noun phrase, which does not reflect standard grammatical structure or the intended meaning.
> son2: The sentence means that there are four other RecQ DNA helicases (besides WRN) that were not preferentially essential in MSI cell lines. In option [1], "four other RecQ DNA helicases" is correctly treated as the subject noun phrase, with "four" functioning as an adjective modifying "DNA helicases." Option [2] incorrectly brackets "The four" as a unit separate from the rest of the noun phrase, which does not reflect the grammatical or semantic structure of the sentence.
> son3: The sentence means that the four other RecQ DNA helicases (as a group of four entities) were not preferentially essential in MSI cell lines. In option 1, "four other RecQ DNA helicases" is correctly treated as the full noun phrase subject, with "four" functioning as an adjective modifying "DNA helicases." Option 2 incorrectly brackets "The four" as a standalone constituent separate from the rest of the noun phrase, which does not reflect standard grammatical structure or the intended meaning.

**Pin offered:** yes (option 2). **Cause: (a)**, with **(d)** contributing. **Models agree** in direction (sonnet 3/3, jev 1/3).

- Option 2 shows «[The four] other RecQ DNA helicases …» and no function line. `verbalize::collect_links` emits a `Numeral` link for `has_count` but none for `the_count`, so nothing states that «four» counts the helicases. The pin candidate's links contain no entry for «four».
- Sonnet, 3/3: «Option [2] incorrectly brackets "The four" as a unit separate from the rest of the noun phrase».
- Option 1's ««four» is an adjective describing «DNA helicases»» reads as ordinary grammar and hides its term, `gt(deg_a02186665(x), std_a02186665)` — «more four than the norm». Rows 239 and 244 rule it wrong on structure.
- jev: 0.60/0.39, 0.61/0.38, then 0.48/0.51.


### U10 «WRN dependency may require specific lineages or a stronger mutation phenotype.»

```text
[1]     WRN dependency [may require] specific lineages or a stronger mutation phenotype.   <- jev1 jev2 jev3
        (no function lines)
[2] PIN WRN dependency may [require specific lineages or a stronger mutation phenotype].   <- son1 son2 son3
        (no function lines)
```

jev probabilities: jev1: 1=0.58, 2=0.41, none=0.01 · jev2: 1=0.55, 2=0.44, none=0.01 · jev3: 1=0.54, 2=0.45, none=0.01

sonnet reasons:
> son1: The sentence expresses that WRN dependency may require either specific lineages or a stronger mutation phenotype — the modal "may" scopes over the entire requirement, and the bracketing in option 2 correctly groups "require specific lineages or a stronger mutation phenotype" as the complement of "may," consistent with the prior readings (e.g., sentence 35: "possibly, Microsatellite Instability create Vulnerability") where modality is treated as a wide-scope operator over the full predication. Option 1 incorrectly brackets "may require" as the main predicate unit, isolating the modal from its full scope.
> son2: The sentence expresses that WRN dependency possibly requires "specific lineages or a stronger mutation phenotype" — the modal "may" scopes over the entire requirement, and the object of "require" is the disjunction "specific lineages or a stronger mutation phenotype." This matches the pattern seen in prior readings (e.g., sentence 35: "we hypothesize possibly, X create Y and possibly, Z create Y"), where the modal/epistemic operator is treated as a prefix modifier and the main predicate takes its full argument. Option [2] brackets "require specific lineages or a stronger mutation phenotype" as the unit under "may," correctly capturing that the disjunction is the object of requiring, not an alternative to requiring.
> son3: In context, the sentence asserts that WRN dependency actually does require (specific lineages or a stronger mutation phenotype) — "may" here hedges the entire claim but the core predicate is "require [specific lineages or a stronger mutation phenotype]" as a single bracketed constituent. The prior readings (e.g., sentence 35 uses "possibly" as a modal hedge wrapping the full proposition) confirm that modality is treated as an outer operator, leaving the main verb and its full object as the primary structural unit. Option 2 correctly brackets "require specific lineages or a stronger mutation phenotype" as the governed phrase, consistent with the document's meaning that WRN dependency is contingent on those specific conditions.

**Pin offered:** yes (option 2). **Cause, jev: (b)** — the options are logically equivalent; **(a)** — no function lines. **Sonnet: no structure-call error**; its miss is a sense-call outcome. **Models disagree.**

- Option 1 is `Or(Possible(A), Possible(B))`, option 2 `Possible(Or(A, B))`. ◇ distributes over ∨, so they are equivalent; the pin note says «Logically equivalent for ◇/∨; pinned on the surface bracketing». Both options have the same links and no predication, so the only difference shown is «[may require]» vs «may [require …]». jev puts 0.54–0.58 on option 1.
- Sonnet chose option 2. Its 32 readings span two skeletons: `compound(G, C1337007)` (the gene, an individual) and `compound_kind(G, C0388246)` (the protein, a kind). The sense call took the gene in all three draws, citing the prior selection «sentence 48: "WRN gene encode"». The 2026-09-30 ruling assigns need-claims to the protein; row 136 rules it `wrong`, basis `structure; sense C1337007`.


### Skeleton miss without a structure call — «WRN encodes a RecQ DNA helicase.»

One analysis; no structure call. jev3's sense call took the protein reading (0 = 0.50 vs 1 = 0.45), whose skeleton has `kind_of(C0388246)` where the pin has the bare individual `C1337007`. Ledger row 223: `wrong`, sense C0388246. Cause: **(d)** — the gene/protein sense choice is also a skeleton difference (P7) — plus a near-chance sense error.

### Not structure errors

jev's abstentions on «Each event alone does not lead to cell death.» (jev3) and «We hypothesized that other DNA repair defects would give rise to synthetic-lethal relationships.» (jev1–3) are `none` in the sense call of single-analysis units.

## Patterns

**P1 — a lexicalised multi-word concept cannot be told from its decomposition.** 31 of 49 non-pin choices (U2, U3, U4, U5, U7, U8); U3c is right by the same artefact.
- `analysis::analyses` compares constituent spans. `Derivation::constituents()` returns spans only, so a multi-token leaf and a composed phrase over the same tokens are the same span. When every analysis builds that span, the lexicalised option has no bracket (U4 option 1, U7 option 3, U3c option 1). Otherwise it gets a flat bracket that looks like any composed phrase (U3 option 3, U5 option 1, U2 option 2).
- No line of the prompt says that brackets show only what differs, or that a flat bracket may be one term (`render_prompt` adds only the return instruction; `notes` are empty in all 27 calls).
- Both models prefer the option with more visible internal structure: sonnet on U7 («without specifying the internal structure»), on U2 («fails to show the internal … bracketing»), and the U3/U3c and U7/U8 reversals under the same stated reasoning.

**P2 — pins and ledger disagree on lexicalised vs compositional.** 18 decisions (U2, U3, U5).
- Pins take the concept for C1511667 (U4), C1155873 (U5) and C5392067 (U7, U8), and the decomposition for DNA ⊗ C1155661 (U3, U3c) and the A-DNA chain (U2).
- The ledger's `departs` accepts decompositions (rows 207, 210, 219, 237; «not a ground» in 193, 194, 196) and lexicalisations (132, 153), and rejects C1155661 over «DNA mismatch repair» on a false ground (59).
- The structure diagnostic scores against the pin alone: U2 and U5 count 12 structure misses on readings the ledger accepts or rejects for a sense only; U3 counts 6 on the false ground.

**P3 — a degree term on a non-gradable word is shown as a plain adjective.** «double-stranded» a02251576 (U4), «four» a02186665 (U9), «immune» a02365900 (U7, U8, merged with the plain a02745862 into one option). The degree term is what makes U4 option 4 and U9 option 1 wrong.

**P4 — counting determiners have no function line.** `the_count` produces no link (U9). `has_count` produces «a numeral gives the number of «events»» with the numeral unnamed (`Link.dependent` is empty by design).

**P5 — predication lines name logical forms, not claims.** U6: identical analyses, jev at chance, sonnet maps «definitional identity» to the wrong label.

**P6 — options that differ only non-truth-conditionally, with no function line.** U10 (◇ over ∨).

**P7 — the skeleton carries the gene/protein sense.** C0388246 occurs only under `kind_of`, C1337007 only bare (pin note on «Depletion of WRN induced…»). The structure call groups both into one option — in 11 units the pin's option holds two skeletons (WRN gene/protein in 10; two MLH1 concepts, C0879389/C0252642, in «Somatic MMR inactivation…») — and the sense call then decides the skeleton: U10 sonnet ×3, «WRN encodes» jev3.

**P8 — abstentions lose the structure decision.** 4 jev structure decisions were recovered only from logs; their probabilities are lost.

## Recommendations, by decisions affected

1. **Mark lexicalised multi-word concepts in the analysis (P1).** 13 wrong decisions (U4 6, U7 6, U8 1), and it makes the 18 U2/U3/U5 decisions turn on meaning instead of bracket display.
   - Pass each reading's multi-token leaves to `analysis::Parse` (`Derivation::leaves()` with `span.1 > span.0`; the derivation already records them).
   - In `analyses`, key constituents by (span, is-leaf), so a one-term leaf and a composed phrase over the same tokens differ and both get shown.
   - Render a multi-token leaf as one unit (e.g. `⟨double-stranded DNA breaks⟩`) and add a function line for each one not shared by all analyses: ««double-stranded DNA breaks» is one term, a single named concept».
   - Add a `notes` line to the structure choice: brackets mark only what the analyses group differently; `⟨…⟩` marks one term.
   - Do 2 at the same time: once one-term options are visible, models will pick them on U3/U3c, where the pins are compositional.
2. **One policy for lexicalised vs compositional in pins and ledger (P2).** 18 decisions re-scored (U2 6, U3 6, U5 6).
   - **DECIDED `2026-10-05` (the maintainer):** the pin takes the one-term concept, and its decomposition is `correct, departs` in the ledger. Applied to U3 and U3c (C1155661 over «DNA mismatch repair»); row 59's ground and both pin notes corrected. U3 was re-pinned at the same time to the governed «from» of `result from` (D97 slice 2), with the three `arise from` units on the page; their free-adjunct readings are `wrong`, as for «respond to». U2 was re-pinned to the one term C1511689 on `2026-10-06`; the lexicon gate on «a DNA» (C0000702 «DNA, A-Form» absorbing the article) is still open and needs a reseed.
   - Either a concept and its decomposition count as one structure (the pin file admits alternates, or the structure diagnostic accepts a ledger `departs`-correct structure), or pins name one form and the ledger stops accepting the other.
   - Correct row 59 and the pin notes of U3 and U3c: C1155661 over «DNA mismatch repair» drops nothing.
   - Re-pin U2 to option 4's skeleton (DNA ⊗ repair ⊗ pathway under the existential) or to the BEST row's C1511689. In the lexicon, gate the surface form «a DNA» of C0000702 «DNA, A-Form» (written «A-DNA») so an article cannot seed as part of a term.
3. **Rule U1 (6 decisions).** If the verb: re-pin, and decide whether the verb-adjunct encoding, which predicates the purpose of the subject, is the term to pin.
4. **Paraphrase predications in the sentence's words (P5).** 5 decisions (U6). `verbalize::predication` gets the subject and complement head words: «every nucleotide repeat region is a microsatellite» vs «the kind "nucleotide repeat region" is itself one microsatellite».
5. **Numerals and degree terms (P3, P4).** 4 decisions (U9); it also exposes the degree term that makes U4 option 4 wrong (6 decisions, shared with 1).
   - In `collect_links`, emit `Function::Numeral` for `the_count` with the numeral word as dependent, and fill the dependent for `has_count`: ««four» counts «helicases»: there are exactly four».
   - Split `Function::Adjective` by term: `gt(deg_A(x), std_A)` → ««four» is graded: the helicases are more four than usual»; `A(x)` → ««four» describes «helicases»».
   - Upstream: lexicon-mark classifying adjectives and number words as non-gradable so the degree reading is not built.
6. **Equivalent options (P6).** 3 decisions (U10 jev). Normalise `Or(Possible a, Possible b)` to `Possible(Or(a, b))` before grouping and in the skeleton comparison, so the two analyses become one option; the pin note already calls them equivalent. Without merging, add a scope line («"may" covers the whole choice» / «"may" applies to each alternative»).
7. **Scoring and logging (P7, P8).** 4 sense-call misses re-attributed; 4 structure decisions recorded.
   - Return the structure account with an abstention instead of dropping it at the sense call's `?`.
   - Report structure-call accuracy (the chosen option contains the pin) separately from skeleton accuracy.
   - On a sense-call `none`, re-ask on the structure runner-up before abstaining (U7: jev none = 0.74–0.77 after option 4). Yield unknown: jev's runner-ups on U7 were not recorded.
8. **ICB decomposition (U7, U8).** No [immune checkpoint] constituent exists, and «checkpoint» seeds only C1155874 «Cell Cycle Checkpoints»; every compositional ICB option is malformed. A lexicon entry for «immune checkpoint» would make the decomposition faithful; without one, the two malformed options per unit stay on offer.
