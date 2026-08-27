# Contemporary related work

Contemporary work in applied formal reasoning for science and engineering: institution theory in physics and systems engineering, higher-order logic for the natural sciences, formal ontologies for engineering and chemistry, Homotopy Type Theory and its directed and dynamic extensions, and the epistemology of formal proof.

_Generated from `docs/references/eigenius_related_work.bib` by `scripts/bib-to-md.py`. Do not edit by hand._

Total entries: **61**.

---

### `awodey-warren2009`

Awodey, Steve and Warren, Michael A. (2009). "Homotopy theoretic models of identity types". *Mathematical Proceedings of the Cambridge Philosophical Society*, 146(1), pp. 45–55.

[DOI: 10.1017/S0305004108001783](https://doi.org/10.1017/S0305004108001783)

> Establishes the homotopy-theoretic interpretation of Martin-Löf identity types — a precursor to the univalent foundations program.

### `babaeigiglou2023llms4ol`

Babaei Giglou, Hamed, D'Souza, Jennifer, and Auer, Sören (2023). "LLMs4OL: Large Language Models for Ontology Learning". In *The Semantic Web — ISWC 2023*.

[arXiv:2307.16648](https://arxiv.org/abs/2307.16648)

> Casts ontology learning as three LLM tasks — term typing, taxonomy (is-a) discovery, and non-taxonomic relation extraction — and evaluates foundational LLMs across them. Finding: foundational LLMs alone are insufficient for high-reasoning ontology construction. The task taxonomy for D50's ontology-learning landscape.

### `bangalore-joshi-1999-supertagging`

Bangalore, Srinivas and Joshi, Aravind K. (1999). "Supertagging: An Approach to Almost Parsing". *Computational Linguistics*, 25(2), pp. 237–265.

> Origin of SUPERTAGGING ("almost parsing"): assign the most-probable rich lexical descriptor per word BEFORE parsing, collapsing the lexical-ambiguity search space. The direct analog of our problem — WordNet sense-polysemy seeding many items per token. ACL J99-2004.

### `bekki-lightblue`

Bekki, Daisuke, et al.. *lightblue: a CCG parser with Dependent Type Semantics (DTS) representations*. \urlhttps://github.com/DaisukeBekki/lightblue.

> CCG parser whose derivation maps homomorphically (Curry-Howard) to DTS preterms — a Martin-Lof dependent type theory over Sigma-types with proof-carrying witnesses; lexical entries are (form, CCG category, DTS preterm) triples. Built-in type-checker (Semantic Felicity Condition) + the Wani DTT theorem prover = the CHECK stage. Covers the engine's stages 2/4/5 natively. BSD-3-Clause (LICENSE file; package.yaml has a stray AllRightsReserved Cabal default). Active (2026). Japanese-strong, English less mature (the key risk). The fourth records-first / Sigma-type semantics (with TTR, MTT, Carpenter) and the only verified one shipping a parser + native check. See also Bekki et al., BriGap 2025, aclanthology.org/2025.brigap-1.1. Verified: deep-research wea30vq2b.

### `billot-lang-1989-shared-forests`

Billot, Sylvie and Lang, Bernard (1989). "The Structure of Shared Forests in Ambiguous Parsing". In *ACL 1989*, pp. 143–151.

[DOI: 10.3115/981623.981641](https://doi.org/10.3115/981623.981641)

> Formal account of the cubic-size shared parse forest — sub-derivation sharing as a grammar in its own right. ACL P89-1018.

### `birkhoff-vonneumann1936`

Birkhoff, Garrett and von Neumann, John (1936). "The logic of quantum mechanics". *Annals of Mathematics*, 37(4), pp. 823–843.

[DOI: 10.2307/1968621](https://doi.org/10.2307/1968621)

> Founding paper of quantum logic; introduces the orthomodular lattice structure of quantum events as an alternative to classical Boolean algebra.

### `borgo-mizoguchi2009`

Borgo, Stefano and Mizoguchi, Riichiro (2009). "A formal ontological perspective on the behaviors and functions of technical artifacts". *AI EDAM (Artificial Intelligence for Engineering Design, Analysis and Manufacturing)*, 23(1), pp. 3–21.

[DOI: 10.1017/S0890060409000079](https://doi.org/10.1017/S0890060409000079)

> Uses DOLCE to give a uniform formal account of artifact behavior vs. function — a long-standing source of miscommunication in engineering design.

### `caraballo-charniak-1998-fom`

Caraballo, Sharon A. and Charniak, Eugene (1998). "New Figures of Merit for Best-First Probabilistic Chart Parsing". *Computational Linguistics*, 24(2), pp. 275–298.

> Figure-of-merit (inside x outside estimate) agenda prioritization for best-first chart parsing — the inexact heuristic-ordering family. ACL J98-2004.

### `carpenter1997`

Carpenter, Bob (1997). *Type-Logical Semantics*. Language, Speech, and Communication, MIT Press.

> Categorial grammar built on the Lambek calculus + typed-lambda semantics. The compositional mechanism Eigenius's composition front-end needs spelled out: a "composition slot" is a type, and a derivation is a term under the Curry-Howard correspondence — syntactic combination and semantic combination are the same operation over the type substrate. The classic worked reference for NL -> typed-term translation by categorial composition; the Lambek-categories-as-types view is the precursor the dependent categorial grammars of Chatzikyriakidis & Luo (Ch. 7.3) lift into MTT. Primary-read.

### `caufield2024spires`

Caufield, J. Harry, Hegde, Harshad, Emonet, Vincent, Harris, Nomi L., Joachimiak, Marcin P., Matentzoglu, Nicolas, Kim, HyeongSik, Moxon, Sierra, Reese, Justin T., Haendel, Melissa A., Robinson, Peter N., and Mungall, Christopher J. (2024). "SPIRES: a method for populating knowledge bases using zero-shot learning to extract the contents of texts". *Bioinformatics*, 40(3), pp. btae104.

[DOI: 10.1093/bioinformatics/btae104](https://doi.org/10.1093/bioinformatics/btae104)

> Structured Prompt Interrogation and Recursive Extraction of Semantics: a LinkML schema constrains the LLM extraction, which recurses over the text and grounds every extracted entity to an ontology ID. The schema-constrained, ontology-grounded extraction precedent for D50; same shape as the "tool produces typed output grounded to the ontology" boundary the pilot uses.

### `charniak-2000-maxent-parser`

Charniak, Eugene (2000). "A Maximum-Entropy-Inspired Parser". In *NAACL 2000*, pp. 132–139.

> Canonical source for beam / overparsing-style (inexact) pruning in lexicalized statistical parsing. ACL A00-2018.

### `charniak-etal-2006-coarse-to-fine`

Charniak, Eugene, Johnson, Mark, Elsner, Micha, et al. (2006). "Multilevel Coarse-to-Fine PCFG Parsing". In *HLT-NAACL 2006*, pp. 168–175.

> COARSE-TO-FINE: parse with a coarse grammar to prune the chart for the finer pass. Cf. a categorial-skeleton-first then type-refine pass. ACL N06-1022.

### `chatzikyriakidis-luo-2020`

Chatzikyriakidis, Stergios and Luo, Zhaohui (2020). *Formal Semantics in Modern Type Theories*. ISTE Ltd / John Wiley & Sons.

[DOI: 10.1002/9781119489252](https://doi.org/10.1002/9781119489252)

> MTT-semantics: natural-language meaning in Modern Type Theories (dependent types; coercive subtyping; an impredicative universe Prop — cf. Eigenius's Prop universe, D46). Argues MTT-semantics is BOTH model-theoretic AND proof-theoretic — a dual nature not available before MTTs, i.e. a framework-level resolution of the model-vs-proof tension. Includes Coq verification of NL semantics (the proof-assistant precedent for the engine's check; cf. D28/D30) and dependent event types. The comprehensive reference for the Luo / D18 anchor. Main chapters paywalled; front matter + appendices (the formal system LF-Delta, impredicative Prop, Coq codes) are open access. Also: the impredicative MTTs are UTT (Luo 1994) / pCIC; lineage from Ranta's application of MTT to semantics (1994; Grammatical Framework); CNs-as-types and CNs-as-setoids (types with a criterion of identity); and Ch.7.3 develops dependent categorial grammars (DCGs) — dependent Lambek categories AS MTT semantic types, unifying categorial composition with the type substrate. Characterized from the abstract + preface + TOC + free appendices (main case-study chapters paywalled).

### `chen2025reform`

Chen, Guoxin, Wang, Jing, Xu, Tianyi, Zhang, Kai, Liu, Pengfei, and Dong, Bin (2025). *ReForm: Reflective Autoformalization with Prospective Bounded Sequence Optimization*.

[arXiv:2510.24592](https://arxiv.org/abs/2510.24592)

> ICLR 2026. Reflective autoformalization with an LLM-judge semantic check whose ceiling is reported at 85.8%. CORRECTION on the headline error rates: the 16.4% (miniF2F) and 38.5% (ProofNet) semantic-error figures are error rates in the HUMAN-WRITTEN benchmark statements ReForm audited, not the error rate of ReForm's own autoformalizer output. Further evidence that "type-checks" is not "faithful" — the gap D30 is built to close.

### `chiang-2007-hierarchical`

Chiang, David (2007). "Hierarchical Phrase-Based Translation". *Computational Linguistics*, 33(2), pp. 201–228.

[DOI: 10.1162/coli.2007.33.2.201](https://doi.org/10.1162/coli.2007.33.2.201)

> Origin of CUBE PRUNING: merged (recombined) items retain the multiset of antecedent back-pointers; k-best lists of the antecedents form a sorted cube whose cheap corner is enumerated best-first and the rest pruned. The extraction algorithm for our packed forest. ACL J07-2003.

### `clark-curran-2004-supertagging`

Clark, Stephen and Curran, James R. (2004). "The Importance of Supertagging for Wide-Coverage CCG Parsing". In *COLING 2004*, pp. 282–288.

> The precise source of ADAPTIVE (beta-level) supertagging: start with a tight per-word category beam, widen only when the parse fails. The "start tight, widen on failure" policy we adopt for D65 sense_rank-driven seed pruning. ACL C04-1041.

### `clark-curran-2007-ccg`

Clark, Stephen and Curran, James R. (2007). "Wide-Coverage Efficient Statistical Parsing with CCG and Log-Linear Models". *Computational Linguistics*, 33(4), pp. 493–552.

[DOI: 10.1162/coli.2007.33.4.493](https://doi.org/10.1162/coli.2007.33.4.493)

> The canonical wide-coverage CCG parser; supertagger as a parser front-end filter + the full adaptive-supertagging loop. ACL J07-4004.

### `cooper2023perception`

Cooper, Robin (2023). *From Perception to Communication: A Theory of Types for Action and Meaning*. Oxford Studies in Semantics and Pragmatics, Oxford University Press.

[DOI: 10.1093/oso/9780192871312.001.0001](https://doi.org/10.1093/oso/9780192871312.001.0001)

> TTR (Type Theory with Records). Records = labelled fields of objects; record types = labelled fields of types; a record is a witness for a record type iff the fields match; a proposition is a type, true iff witnessed. Intensional, types-not-possible-worlds (Ch. 6); first-class types + reflection. Records-first — the same record-type / Sigma-type structure as an Eigenius Class (requires/recommends as fields; Resource = record = witness): the closest external substrate match. Open access (CC BY-NC-ND 4.0). Primary-read (intro + TOC); local copy sha256 4ac02f9d1e555ee22f1a3490afd2d822215d39633cd45e8693b240e0b233f3f9.

### `darrigol2008-modular`

Darrigol, Olivier (2008). "The modular structure of physical theories". *Synthese*, 162(2), pp. 195–223.

[DOI: 10.1007/s11229-007-9181-x](https://doi.org/10.1007/s11229-007-9181-x)

> Argues that mature physical theories decompose into modules with stable interfaces — the philosophical analogue of the colimit-style composition that institutions formalize.

### `dorre-1997-underspecified-packing`

Dörre, Jochen (1997). "Efficient Construction of Underspecified Semantics under Massive Ambiguity". In *35th ACL / 8th EACL (ACL 1997)*, pp. 386–393.

[arXiv:cmp-lg/9706028](https://arxiv.org/abs/cmp-lg/9706028)

> Builds ONE packed underspecified semantics (packed UDRS) off the parse forest instead of enumerating readings; an OR-node's semantics is the disjunction of its children under one shared variable. Establishes that UNDERSPECIFIED holes (scope/referent) are COMPATIBLE with packing — grounds our deferred $quant$/$anaphor$ holes riding the packed stream. ACL P97-1050.

### `fca-process-ontologies`

Tutcher, Jonathan (2014). "A Formal Concept Analysis-based method for developing process ontologies, applied to chemical engineering". In *Proceedings of an industrial-ontology workshop*.

> Uses FCA to derive class hierarchies for chemical engineering processes and integrates them with ISO 15926 / SUMO. Exact venue per ResearchGate publication 271505148.

### `gao2024herald`

Gao, Guoxiong, Wang, Yutong, Jiang, Jiedong, Gao, Qi, Qin, Zihan, Xu, Tianyi, and Dong, Bin (2024). *Herald: A Natural Language Annotated Lean 4 Dataset*.

[arXiv:2410.10878](https://arxiv.org/abs/2410.10878)

> ICLR 2025. A natural-language <-> Lean 4 dataset built with back-translation, using an LLM-judge as the faithfulness check on the autoformalized statements. NOTE: the LLM-judge is the reported faithfulness mechanism; the much lower human-audited pass rate is a finding of Ospanov et al. (miniF2F-Lean Revisited), not a Herald claim — see ospanov2025minif2f.

### `garbacz-engineering-functions`

Garbacz, Paweł, et al.. "Towards a formal ontology of engineering functions, behaviours, and capabilities". *Semantic Web Journal*.

[Link](https://www.semantic-web-journal.net/system/files/swj3188.pdf)

> Submission swj3188; surveys formalizations of engineering function/behaviour/capability and proposes a unifying ontology.

### `genetic-code-hott`

Anonymous (2024). "Exploring the genetic code: A Homotopy Type Theory approach to uncovering embedded information". *Preprint*.

> Analyzes codon relationships and nucleotide permutations in a 3D lattice model under HoTT; surfaces structure suggestive of layered molecular language. ResearchGate publication 380075492; authorship to be verified.

### `glymour2013-theoretical-equivalence`

Glymour, Clark (2013). "Theoretical equivalence and the semantic view of theories". *Philosophy of Science*, 80(2), pp. 286–297.

[DOI: 10.1086/670261](https://doi.org/10.1086/670261)

> Response to Halvorson 2012. The user's reading list links this title to an institution-theoretic treatment with quantum cosmology examples; the title matches Glymour's paper exactly, but Glymour's paper itself does not use institutions — it defends the semantic view via De Bouvere's criteria. The institution-theoretic angle on this debate is in Halvorson & Tsementzis, "Categories of Scientific Theories" (2017).

### `halvorson2012`

Halvorson, Hans (2012). "What scientific theories could not be". *Philosophy of Science*, 79(2), pp. 183–206.

[DOI: 10.1086/664745](https://doi.org/10.1086/664745)

> Critique of the strict semantic view of scientific theories; starts the debate that motivates institution- and category-theoretic accounts.

### `harper-1994-logical-form-forest`

Harper, Mary P. (1994). "Storing Logical Form in a Shared-Packed Forest". *Computational Linguistics*, 20(4).

> THE direct model for our design ("Method 3"): store a DEFERRED PROCEDURE CALL at each packed forest node and build the logical form ON DEMAND, so the LF-bearing forest has the SAME node count as the bare syntactic forest (9,658 B vs 1,477,644 B eager at 132 parses). Also states the sharing PITFALL: a packed child's semantics may not be affected by the parent's LF construction, else copy-and-modify (= our cat_group carve-out). ACL J94-4006.

### `herre2010gfo`

Herre, Heinrich (2010). "General Formal Ontology (GFO): A foundational ontology for conceptual modelling". In *Theory and Applications of Ontology: Computer Applications*, ed. Poli, Roberto, Healy, Michael, and Kameas, Achilles, pp. 297–345, Springer.

> Specification of GFO, an integrative axiomatic top-level ontology used as a peer to BFO and DOLCE.

### `hopkins-langmead-2009-cube-search`

Hopkins, Mark and Langmead, Greg (2009). "Cube Pruning as Heuristic Search". In *EMNLP 2009*, pp. 62–71.

> The clean formal SEAM our design turns on: an item is keyed for combination by (span, POSTCONDITION = category) while the CARRY (extra scoring info) affects cost ONLY, not combinability. Maps 1:1 onto our felicity-by-category (postcondition) vs. differing semantics (carry) — the licence to erase type-indices from the packing signature. ACL D09-1007.

### `huang-chiang-2005-kbest`

Huang, Liang and Chiang, David (2005). "Better k-best Parsing". In *IWPT 2005 (9th Int. Workshop on Parsing Technologies)*, pp. 53–64.

> k-best over a weighted hypergraph (= packed forest); k-best is bounded PER VERTEX (signature), O(|V|.k), NOT globally — exactly what stops the pile evicting the rare correct reading. Algorithm 3 (LazyKthBest) extracts k-best top-down, materializing sub-derivations only as needed (the lazy-unpack our extractor uses). ACL W05-1506.

### `kent-iff2011`

Kent, Robert E. (2011). *The Information Flow Framework: A descriptive category metatheory*.

[arXiv:1108.4133](https://arxiv.org/abs/1108.4133)

> Structural metatheory for the IEEE P1600.1 Standard Upper Ontology (SUO) project; uses institutions and their morphisms as the upper-metalevel axiomatization for semantic integration of ontologies.

### `khan-afshar-complex-vectors`

Khan-Afshar, Sanaz, Siddique, Umair, Mahmoud, Muhammad Yasir, Hasan, Osman, and Tahar, Sofiène (2014). "Formalization of complex vectors in higher-order logic". In *CICM 2014: International Conference on Intelligent Computer Mathematics*, Lecture Notes in Artificial Intelligence 8543, pp. 123–137, Springer.

> HOL Light formalization of complex vector analysis used to verify foundational laws of electromagnetism.

### `klein-manning-2003-astar`

Klein, Dan and Manning, Christopher D. (2003). "A* Parsing: Fast Exact Viterbi Parse Selection". In *HLT-NAACL 2003*, pp. 119–126.

> EXACT best-first parsing via an ADMISSIBLE outside heuristic — no search errors, <3% of exhaustive edges. The "exactness via a sound filter" principle our typed felicity oracle affords. ACL N03-1016.

### `knapp-institutional-framework-uml2015`

Knapp, Alexander, Mossakowski, Till, and Roggenbach, Markus (2015). "Towards an institutional framework for heterogeneous formal development in UML — A position paper". In *Software, Services, and Systems: Essays Dedicated to Martin Wirsing on the Occasion of His Retirement from the Chair of Programming and Software Engineering*, ed. De Nicola, Rocco and Hennicker, Rolf, Lecture Notes in Computer Science 8950, pp. 215–230, Springer.

[DOI: 10.1007/978-3-319-15545-6_15](https://doi.org/10.1007/978-3-319-15545-6_15)

### `knapp-uml-state-machines2015`

Knapp, Alexander, Mossakowski, Till, Roggenbach, Markus, and Glauer, Martin (2015). "An institution for simple UML state machines". In *FASE 2015: Fundamental Approaches to Software Engineering*, ed. Egyed, Alexander and Schaefer, Ina, Lecture Notes in Computer Science 9033, pp. 3–18, Springer.

[DOI: 10.1007/978-3-662-46675-9_1](https://doi.org/10.1007/978-3-662-46675-9_1)

### `lai2020dependently`

Lai, Zhangsheng, Ng, Aik Beng, Wong, Liang Ze, See, Simon, and Lin, Shaowei (2020). *Dependently Typed Knowledge Graphs*.

[arXiv:2003.03785](https://arxiv.org/abs/2003.03785)

> Reproduces RDF + SPARQL inside the Calculus of Inductive Constructions (Coq): graph triples become typed terms, and a query is reformulated AS A TYPE whose inhabitants are the answers, each carrying a proof witness ("queries as types, answers as proof-carrying witnesses"). The direct external precedent for Eigenius's "queries-as-types" stance (D30) and for the certificate/witness shape of D39's JustifiedBy (answers that carry their own justification). Explicitly a proof-of-concept, not a production system. Primary-read.

### `luo2012cnt`

Luo, Zhaohui (2012). "Common Nouns as Types". In *Logical Aspects of Computational Linguistics (LACL 2012)*, ed. Béchet, Denis and Dikovsky, Alexandre, Lecture Notes in Computer Science 7351, pp. 173–185, Springer.

[DOI: 10.1007/978-3-642-31262-5_12](https://doi.org/10.1007/978-3-642-31262-5_12)

> CNs-as-types: a common noun denotes a TYPE, not a predicate over entities — so a Class IS a type, the exact move Eigenius makes (EigonClass : Set; "cell line" -> the type CellLine, D62 §8.6). This is what makes cat_np(T) type-indexing well-founded. Subsumption between CNs is coercive subtyping (Gene <= Entity), so a general predicate typed at a supertype applies to specific-typed arguments without re-typing: the primary warrant for reflecting core:subclass_of as the EigonClass subtype rule (the inclusion-coercion fragment). Author copy (open): cs.rhul.ac.uk/home/zhaohui/LACL12.pdf. DOI verified via dblp + Springer (LACL 2012 = LNCS 7351, pp. 173–185).

### `luo2012coercive`

Luo, Zhaohui (2012). "Formal Semantics in Modern Type Theories with Coercive Subtyping". *Linguistics and Philosophy*, 35(6), pp. 491–513.

[DOI: 10.1007/s10988-013-9126-4](https://doi.org/10.1007/s10988-013-9126-4)

> The focused primary for coercive subtyping as the MTT-semantics device for selectional restrictions, copredication, and multiple categorization: an argument slot at type A admits a value of type B via a coherent coercion B -> A; the inclusion case B <= A is subsumption. This is the mechanism behind "depends on : Entity -> Entity -> Prop" admitting Gene / CellLine arguments because Gene, CellLine <= Entity — i.e. behind making the kernel's EigonClass subtype check consult core:subclass_of (D62 §8.6). Companion to the comprehensive Chatzikyriakidis & Luo (2020). Author copy (open): cs.rhul.ac.uk/ zhaohui/LP12final3.pdf. DOI verified via Springer + PhilPapers (LUOFSI); Ling.&Phil. 35(6):491–513, 2012.

### `martinot2024ontological-purity`

Martinot, Robin (2024). "Ontological purity for formal proofs". *The Review of Symbolic Logic*, 17(2), pp. 395–434.

[DOI: 10.1017/S1755020323000333](https://doi.org/10.1017/S1755020323000333)

> Develops a graded notion of ontological purity for formal proofs, including a notion of "secondary purity" for proofs that use surrogate content via formal interpretations. Directly relevant to evaluating cross-layer reasoning in Eigenius.

### `masolo2003dolce`

Masolo, Claudio, Borgo, Stefano, Gangemi, Aldo, Guarino, Nicola, and Oltramari, Alessandro (2003). "WonderWeb Deliverable D18: Ontology Library". Laboratory For Applied Ontology (ISTC-CNR).

[Link](http://wonderweb.man.ac.uk/deliverables/D18.shtml)

> Foundational specification of DOLCE (Descriptive Ontology for Linguistic and Cognitive Engineering).

### `mclean-horspool-1996-earley`

McLean, Philippe and Horspool, R. Nigel (1996). "A Faster Earley Parser". In *Compiler Construction (CC)*, Lecture Notes in Computer Science.

> LRE(k): a hybrid of Earley parsing and LR(k). Precomputed LR(k) item-sets make Earley 10-15x faster with <half the storage, while still parsing arbitrary (incl. ambiguous) CFGs. The chart-parsing backbone for the D62 engine's composition / parsing front-end: ambiguity-tolerant (parse forest), incremental (cf. Cooper's TTR "chart type"), and friendly to attaching semantic actions (build the typed term during the parse). Lifted to the type-logical / DCG composition via "parsing as deduction" (Pereira & Warren). Venue/pages (CC'96, LNCS 1060?) to verify — not shown in the dropped PDF.

### `mihindukulasooriya2023text2kgbench`

Mihindukulasooriya, Nandana, Tiwari, Sanju, Enguix, Carlos F., and Lata, Kusum (2023). *Text2KGBench: A Benchmark for Ontology-Driven Knowledge Graph Generation from Text*.

[arXiv:2308.02357](https://arxiv.org/abs/2308.02357)

> Benchmark for generating ontology-conformant knowledge graphs from text: scores both ontology conformance (do the produced triples respect the target ontology's classes/relations) and subject / relation / object hallucination (do they introduce entities or relations absent from the source and ontology). The external precedent for D50's typed-KG-generation metrics.

### `model-management-syseng-kg`

Anonymous (2025). *Model management to support systems engineering workflows using ontology-based knowledge graphs*.

[arXiv:2512.09596](https://arxiv.org/abs/2512.09596)

> Uses the Ontology Modelling Language (OML) to relate systems engineering processes to artifacts via a knowledge graph; supports versioning and reasoning.

### `oepen-carroll-2000-ambiguity-packing`

Oepen, Stephan and Carroll, John (2000). "Ambiguity Packing in Constraint-based Parsing — Practical Results". In *NAACL 2000*, pp. 162–169.

> LOCAL AMBIGUITY PACKING in a (unification/HPSG) chart: subsumption- and equivalence-based packing of feature structures — the closest prior art to packing + exact type-filtering in a typed chart. ACL A00-2022.

### `ontology-based-plm-polimi`

Polimi authors (2025). "Ontology-based product lifecycle management: Insights from a proof-of-concept implementation". *IEEE Access*.

> Demonstrates ontology-driven PLM in an Engineer-to-Order setting; combines linked-data infrastructure with description-logic reasoning. Authorship per Polimi repository entry; exact attribution should be verified.

### `ontology-of-physics-for-biology`

Cook, Daniel L., Bookstein, Fred L., and Gennari, John H. (2011). "Physical properties of biological entities: An introduction to the Ontology of Physics for Biology". *PLOS ONE*, 6(12), pp. e28708.

[DOI: 10.1371/journal.pone.0028708](https://doi.org/10.1371/journal.pone.0028708)

> Specification of OPB, which extends BFO with energy-bearing biophysical entities and quantitative dependencies.

### `ospanov2025minif2f`

Ospanov, Azim, Farnia, Farzan, and Yousefzadeh, Roozbeh (2025). *miniF2F-Lean Revisited: Reviewing Limitations and Charting a Path Forward*.

[arXiv:2511.03108](https://arxiv.org/abs/2511.03108)

> NeurIPS 2025. Audits autoformalization-faithfulness claims. CORRECTION it supplies to the Herald numbers: Herald's statement formalization, reported 97% faithful by its LLM-judge, drops to 66% under human evaluation, with an end-to-end correct rate of 34.8%. These figures are this paper's audit OF Herald, not Herald's own claims — evidence that an LLM-judge faithfulness check is not a substitute for the kernel-checked faithfulness D30 requires.

### `poernomo2025dhott`

Poernomo, Iman (2025). *DHoTT: A Temporal Extension of Homotopy Type Theory for Semantic Drift*.

[arXiv:2506.09671](https://arxiv.org/abs/2506.09671)

> Indexes types by a context-time parameter so they may deform, rupture, and reassemble — supports formal reasoning about semantic evolution and discontinuity.

### `psrl-nist`

Patil, Lalit, Dutta, Debasish, and Sriram, Ram D. (2005). "Ontology Formalization of Product Semantics for Product Lifecycle Management". National Institute of Standards and Technology, NISTIR 7274.

[Link](https://nvlpubs.nist.gov/nistpubs/Legacy/IR/nistir7274.pdf)

> Defines the Product Semantic Representation Language (PSRL), a description-logic-based formalism for application-independent product semantics in PLM.

### `qiu2025state-formalization`

Qiu, others (2025). "Research on a general state formalization method from the perspective of logic". *Mathematics (MDPI)*, 13(20), pp. 3324.

[DOI: 10.3390/math13203324](https://doi.org/10.3390/math13203324)

> Proposes a unified axiomatization of states as interpretations of formulas across first-order, higher-order, and infinitary logics. Full author list per Crossref starts with Qiu; complete attribution should be filled in from the MDPI page before any citation use.

### `ranta2026symbolic-informalization`

Ranta, Aarne (2026). *Symbolic Informalization: Fluent, Productive, Multilingual*.

[arXiv:2606.16893](https://arxiv.org/abs/2606.16893)

> The Informath project: symbolic (grammar-based) informalization of formal mathematics to natural language via Grammatical Framework, with Dedukti as an interlingua hub over Agda/Lean/Rocq. Two-level grammar (abstract semantics / concrete surface); type-checking disambiguates parses; the NLG side over-generates and RANKS variants by structural penalties (tree size/depth) rather than hard-cutting; reversible (parse <-> generate). Relevant to Eigenius as (i) a reliable symbolic back-translation for faithfulness checks vs. inflated LLM back-translation, (ii) a model for soft structural ranking over hard normal-form cuts, and (iii) type-checked lexical symbol tables as a category-consistency gate.

### `rashid-hasan-systems-biology`

Rashid, Adnan, Hasan, Osman, Siddique, Umair, and Tahar, Sofiène (2017). "Formal reasoning about systems biology using theorem proving". *PLOS ONE*, 12(7), pp. e0180179.

[DOI: 10.1371/journal.pone.0180179](https://doi.org/10.1371/journal.pone.0180179)

> Formalizes Zsyntax in HOL4 / HOL Light for the verification of molecular pathways and biological networks.

### `rashid-laplace-fourier`

Rashid, Adnan and Hasan, Osman. "Formalization of Laplace and Fourier transforms in higher-order logic for the analysis of synthetic biology genetic circuits". *Theoretical Computer Science (or related)*.

> Frequency-domain analysis of synthetic genetic circuits carried out in HOL Light; exact venue should be confirmed. Source: cited by the survey at PMC5495343.

### `siddique2015thesis`

Siddique, Umair (2015). *Formal Analysis of Fractional Order Systems in Higher-Order Logic*. PhD thesis, Concordia University.

> Develops a HOL Light library for fractional-order systems and applies it to engineering analysis.

### `smith2012classifying-processes`

Smith, Barry (2012). "Classifying processes: An essay in applied ontology". *Ratio (new series)*, 25(4), pp. 463–488.

[DOI: 10.1111/j.1467-9329.2012.00557.x](https://doi.org/10.1111/j.1467-9329.2012.00557.x)

> Articulates the BFO treatment of occurrents (processes) and their classification.

### `sowa-signs-processes`

Sowa, John F. (2010). *Signs, Processes, and Language Games: Foundations for Ontology*. Online manuscript, jfsowa.com.

[Link](http://www.jfsowa.com/pubs/signproc.htm)

> Sowa's foundational essay arguing that ontology must account for signs, processes, and language games — a framing closely related to Eigenius's typed-layer view of scientific knowledge.

### `tomita-1987-glr`

Tomita, Masaru (1987). "An Efficient Augmented-Context-Free Parsing Algorithm". vol. 13, pp. 31–46.

> Generalized LR (GLR) + the SHARED PACKED PARSE FOREST (SPPF): share common sub-derivations so an exponential ambiguity set is stored compactly. The "cheaper items via sharing" lever (vs. our current Box-owned, unshared term trees). ACL J87-1004.

### `voevodsky2014univalent`

Voevodsky, Vladimir (2014). *The Origins and Motivations of Univalent Foundations*. The Institute Letter, Institute for Advanced Study.

[Link](https://www.ias.edu/ideas/2014/voevodsky-origins)

> Voevodsky's own account of the genesis of univalent foundations and the univalence axiom.

### `weaver-bicubical-dtt`

Weaver, Matthew Z. (2024). *Bicubical Directed Type Theory*. PhD thesis, Princeton University.

[Link](https://dataspace.princeton.edu/handle/88435/dsp017s75dg778)

> Generalizes groupoid-style HoTT to a directed setting with morphisms — a foundation for type-theoretic reasoning over directed processes.

### `xu-auli-clark-2015-rnn-supertag`

Xu, Wenduan, Auli, Michael, and Clark, Stephen (2015). "CCG Supertagging with a Recurrent Neural Network". In *ACL-IJCNLP 2015 (Short Papers)*, pp. 250–255.

> Neural supertagger conditioning on full-sentence context → tighter, more accurate per-word category beams. Grounds "sense_rank as a learned supertag prior" for our adaptive seed pruning. ACL P15-2041.

### `zhou2023marie-bert`

Zhou, Xiaochi, Zhang, Shaocong, Agarwal, Mehal, Akroyd, Jethro, Mosbach, Sebastian, and Kraft, Markus (2023). "Marie and BERT—A knowledge graph embedding based question answering system for chemistry". *ACS Omega*, 8(36), pp. 33039–33057.

[DOI: 10.1021/acsomega.3c05114](https://doi.org/10.1021/acsomega.3c05114)

> Combines hybrid knowledge graph embeddings with BERT-based entity linking to answer multihop chemistry questions over OntoSpecies / OntoMOPs.
