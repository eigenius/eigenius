# Philosophical and methodological precursors

Works that situate Eigenius within a longer arc of research on Mathematical Knowledge Management, verified-at-scale formalization, Suppes-style structuralism, formal ontologies in science, the reproducibility movement, and adjacent contemporary projects.

_Generated from `docs/references/eigenius_precursors.bib` by `scripts/bib-to-md.py`. Do not edit by hand._

Total entries: **15**.

---

### `brown2008pharos`

Brown, Jr., Allen L. (2008). "Enforcing the Scientific Method". In *Provenance and Annotation of Data and Processes*, ed. Freire, Juliana, Koop, David, and Moreau, Luc, pp. 2–2, Springer Berlin Heidelberg.

> Presentation of Pharos, the Microsoft Health Solutions Group platform for life-sciences researchers, framed around audited inference + audited workflow as the substrate for enforcing the scientific method. Direct ancestor of Eigenius: same animating goal (machine-readable audit trail over both the conduct and results of scientific investigation), distinct stakeholder set (researchers, regulators, funders, tenure committees, research managements). Pharos shipped a port of Isabelle/HOL called MetaL and a typed graph store on SQL Server with a SPARQL processor called DataNet. Eigenius generalises and substantially extends that substrate: Lean 4 instead of Isabelle/HOL for the proof assistant; a kernel rooted in a Martin-Löf type theory (Pharos had none); explicit institutions as a structural concept (Pharos had none); a layer-graph model with lattice structure and merge operations (Pharos had a flat graph); a notebook interface; and epistemic tracing as core infrastructure substrate, not metadata.

### `carette-farmer2009`

Carette, Jacques and Farmer, William M. (2009). "A Review of Mathematical Knowledge Management". In *Intelligent Computer Mathematics, MKM 2009*, Lecture Notes in Computer Science 5625, pp. 233–246, Springer.

> Survey of the first decade of MKM, framing the open problems (large-scale formalization, library interoperability, knowledge retrieval) that Eigenius addresses for science more broadly.

### `farmer2004mkm`

Farmer, William M. (2004). "MKM: A New Interdisciplinary Field of Research". In *Mathematical Knowledge Management, Third International Conference, MKM 2004*, Lecture Notes in Computer Science 3119, Springer.

> Position paper introducing MKM as a coherent research field concerned with the representation, exchange, and reuse of machine-processable mathematical knowledge.

### `gene-ontology2000`

Ashburner, Michael, Ball, Catherine A., Blake, Judith A., Botstein, David, Butler, Heather, Cherry, J. Michael, Davis, Allan P., Dolinski, Kara, Dwight, Selina S., Eppig, Janan T., Harris, Midori A., Hill, David P., Issel-Tarver, Laurie, Kasarskis, Andrew, Lewis, Suzanna, Matese, John C., Richardson, Joel E., Ringwald, Martin, Rubin, Gerald M., and Sherlock, Gavin (2000). "Gene Ontology: tool for the unification of biology". *Nature Genetics*, 25(1), pp. 25–29.

[DOI: 10.1038/75556](https://doi.org/10.1038/75556)

> Founding paper of the Gene Ontology. The most successful example of a formal vocabulary adopted across an entire scientific community.

### `hales2017flyspeck`

Hales, Thomas, Adams, Mark, Bauer, Gertrud, Dang, Tat Dat, Harrison, John, Hoang, Le Truong, Kaliszyk, Cezary, Magron, Victor, McLaughlin, Sean, Nguyen, Tat Thang, Nguyen, Quang Truong, Nipkow, Tobias, Obua, Steven, Pleso, Joseph, Rute, Jason, Solovyev, Alexey, Ta, Thi Hoai An, Tran, Nam Trung, Trieu, Thi Diep, Urban, Josef, Vu, Ky, and Zumkeller, Roland (2017). "A Formal Proof of the Kepler Conjecture". *Forum of Mathematics, Pi*, 5.

[DOI: 10.1017/fmp.2017.1](https://doi.org/10.1017/fmp.2017.1)

> Project Flyspeck: a fully machine-checked formal proof of the Kepler conjecture in HOL Light and Isabelle. A landmark example of formal verification of a non-trivial scientific result.

### `hottbook2013`

The Univalent Foundations Program (2013). *Homotopy Type Theory: Univalent Foundations of Mathematics*. Institute for Advanced Study, Princeton.

[Link](https://homotopytypetheory.org/book/)

> Collective program articulating type theory as a foundation for mathematics in which equivalence and structure are first-class. Conceptual reference for treating scientific structure typetheoretically rather than set-theoretically.

### `klein2009sel4`

Klein, Gerwin, Elphinstone, Kevin, Heiser, Gernot, Andronick, June, Cock, David, Derrin, Philip, Elkaduwe, Dhammika, Engelhardt, Kai, Kolanski, Rafal, Norrish, Michael, Sewell, Thomas, Tuch, Harvey, and Winwood, Simon (2009). "seL4: Formal Verification of an OS Kernel". In *Proceedings of the ACM SIGOPS 22nd Symposium on Operating Systems Principles (SOSP '09)*, pp. 207–220, ACM.

[DOI: 10.1145/1629575.1629596](https://doi.org/10.1145/1629575.1629596)

> End-to-end formal verification of the seL4 microkernel — functional correctness of an OS kernel against a high-level specification. Pre-history for verified systems infrastructure.

### `leroy2009compcert`

Leroy, Xavier (2009). "Formal Verification of a Realistic Compiler". *Communications of the ACM*, 52(7), pp. 107–115.

[DOI: 10.1145/1538788.1538814](https://doi.org/10.1145/1538788.1538814)

> The CompCert C compiler: a production-quality optimizing C compiler proven correct in Coq. Demonstrates that machine-checked correctness is feasible for software systems of industrial complexity.

### `mathlib2020`

The mathlib Community (2020). "The Lean Mathematical Library". In *Proceedings of the 9th ACM SIGPLAN International Conference on Certified Programs and Proofs (CPP 2020)*, pp. 367–381, ACM.

[DOI: 10.1145/3372885.3373824](https://doi.org/10.1145/3372885.3373824)

> Description of mathlib, the largest collaboratively-built formalized library of mathematics. Demonstrates that a community-curated, machine-checked library at the scale of undergraduate and research mathematics is achievable.

### `myexperiment`

Goble, Carole A., Bhagat, Jiten, Aleksejevs, Sergejs, Cruickshank, Don, Michaelides, Danius, Newman, David, Borkum, Mark, Bechhofer, Sean, Roos, Marco, Li, Peter, and De Roure, David (2010). "myExperiment: a repository and social network for the sharing of bioinformatics workflows". *Nucleic Acids Research*, 38(Suppl 2), pp. W677–W682.

[DOI: 10.1093/nar/gkq429](https://doi.org/10.1093/nar/gkq429)

> Pioneering platform for sharing executable scientific workflows as first-class artifacts. Direct precedent for the notion of a typed, shared, executable scientific knowledge artifact.

### `obofoundry2007`

Smith, Barry, Ashburner, Michael, Rosse, Cornelius, Bard, Jonathan, Bug, William, Ceusters, Werner, Goldberg, Louis J., Eilbeck, Karen, Ireland, Amelia, Mungall, Christopher J., The OBI Consortium, Leontis, Neocles, Rocca-Serra, Philippe, Ruttenberg, Alan, Sansone, Susanna-Assunta, Scheuermann, Richard H., Shah, Nigam, Whetzel, Patricia L., and Lewis, Suzanna (2007). "The OBO Foundry: coordinated evolution of ontologies to support biomedical data integration". *Nature Biotechnology*, 25(11), pp. 1251–1255.

[DOI: 10.1038/nbt1346](https://doi.org/10.1038/nbt1346)

> Establishes the OBO Foundry's coordination principles for interoperable biomedical ontologies. Demonstrates the social and governance infrastructure needed for shared formal vocabularies to scale across a discipline.

### `sneed1971`

Sneed, Joseph D. (1971). *The Logical Structure of Mathematical Physics*. Reidel, Dordrecht.

> Foundational text of the structuralist (Sneed-Stegmüller) program in the philosophy of science. Establishes the distinction between theory-cores and theory-nets that foreshadows the layer / parent-pointer architecture.

### `stodden2010`

Stodden, Victoria (2010). "The Scientific Method in Practice: Reproducibility in the Computational Sciences". *MIT Sloan Research Paper*.

[DOI: 10.2139/ssrn.1550193](https://doi.org/10.2139/ssrn.1550193)

> Early influential statement of the reproducibility crisis in computational science, and of the standards (code + data + environment) needed to make computational results verifiable.

### `suppes2002`

Suppes, Patrick (2002). *Representation and Invariance of Scientific Structures*. CSLI Publications, Stanford University.

> Mature statement of Suppes's set-theoretic / structuralist view of scientific theories. Direct philosophical precursor of the institution-theoretic treatment of theories Eigenius adopts.

### `underlay`

Knowledge Futures Group (2017–2022). *The Underlay: A Public Protocol for Knowledge*. MIT Knowledge Futures Group / MIT Media Lab project.

[Link](https://docs.underlay.org/)

> Project to build a distributed, content-addressed graph protocol for public knowledge, with provenance as a first-class concern. Closest contemporary parallel in ambition; differs in not committing to a typed theory layer or executable resources.
