// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The SYN-26-002 chain over a base: the bootstrap chain with WordNet and HPO stubbed, or a
//! snapshot's head.

#![allow(dead_code)]

use std::sync::Arc;

use eigenius_kernel::commit::{BackendPersister, LayerPersister};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::validation::Validator;

/// The experiment's files in load order (`experiments/pmi-registry/`, `01`–`08`). The first three
/// are the folder's links to `ontologies/`.
const FILES: [(&str, &str); 8] = [
    (
        "variant",
        include_str!("../../../../experiments/pmi-registry/01-variant.esl"),
    ),
    (
        "clinical",
        include_str!("../../../../experiments/pmi-registry/02-clinical.esl"),
    ),
    (
        "clinvar",
        include_str!("../../../../experiments/pmi-registry/03-clinvar.esl"),
    ),
    (
        "registry",
        include_str!("../../../../experiments/pmi-registry/04-registry.esl"),
    ),
    (
        "syngene",
        include_str!("../../../../experiments/pmi-registry/05-syngene.esl"),
    ),
    (
        "syn-26-002",
        include_str!("../../../../experiments/pmi-registry/06-syn-26-002.esl"),
    ),
    (
        "syn-26-002-conclusions",
        include_str!("../../../../experiments/pmi-registry/07-syn-26-002-conclusions.esl"),
    ),
    (
        "refused",
        include_str!("../../../../experiments/pmi-registry/08-refused.esl"),
    ),
];

pub fn compile_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile(src, &parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile: {}",
            errs.iter()
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

pub fn all_errors(layer: &Arc<Layer>) -> Vec<String> {
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect()
}

pub fn errors_for(layer: &Arc<Layer>, iri: &str) -> Vec<String> {
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == iri))
        .map(|e| format!("{e:?}"))
        .collect()
}

/// Where the experiment's layers are built.
pub trait Base {
    fn head(&self) -> Arc<Layer>;

    /// Compile `src` over `parent`. A persistent base commits it to `branch`, starting the
    /// branch at `parent` when it is not `main`.
    fn build(&self, name: &str, src: &str, parent: &Arc<Layer>, branch: &str) -> Arc<Layer>;
}

/// The bootstrap chain, with the one WordNet verb and the HP classes the registry layer names
/// stubbed.
pub struct BootstrapBase;

impl Base for BootstrapBase {
    fn head(&self) -> Arc<Layer> {
        let head = Arc::clone(
            eigenius_kernel::bootstrap::bootstrap()
                .expect("bootstrap")
                .head(),
        );
        compile_layer("stubs", &stubs(), head)
    }

    fn build(&self, name: &str, src: &str, parent: &Arc<Layer>, _branch: &str) -> Arc<Layer> {
        compile_layer(name, src, Arc::clone(parent))
    }
}

/// A snapshot's head, with WordNet and HPO loaded. Layers are built on the persistent store (an
/// in-memory layer over a DB base OOMs at index build) and committed.
pub struct SnapshotBase {
    pub head: Arc<Layer>,
    pub backend: Arc<dyn PersistentBackend>,
}

impl Base for SnapshotBase {
    fn head(&self) -> Arc<Layer> {
        Arc::clone(&self.head)
    }

    fn build(&self, name: &str, src: &str, parent: &Arc<Layer>, branch: &str) -> Arc<Layer> {
        let resources = esl::compile(src, parent)
            .unwrap_or_else(|e| panic!("{name} compiles against the chain: {e:?}"));
        let mut b = LayerBuilder::new(name, Some(Arc::clone(parent)));
        for r in resources {
            b.add_resource(r)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
        let layer = Arc::new(b.build(LayerStorage::with_persistent(Arc::clone(&self.backend))));
        if branch != "main" {
            self.backend
                .put_branch(branch, parent.id())
                .unwrap_or_else(|e| panic!("start {branch} at {name}'s parent: {e:?}"));
        }
        let info = BackendPersister::new(Some(Arc::clone(&self.backend)))
            .persist(branch, &layer)
            .unwrap_or_else(|e| panic!("persist {name}: {e:?}"));
        assert!(info.branch_advanced, "{name} advances {branch}");
        layer
    }
}

/// The WordNet verb and the HP classes `04-registry.esl` names.
fn stubs() -> String {
    let registry = FILES[3].1;
    let mut ids: Vec<&str> = registry
        .split("hp:'")
        .skip(1)
        .filter_map(|rest| rest.split('\'').next())
        .collect();
    ids.sort();
    ids.dedup();
    let mut s = String::from(
        "namespace lexicon = \"urn:eigenius:lexicon\";\n\
         namespace wn = \"urn:eigenius:wn\";\n\
         namespace hp = \"urn:obo:HP\";\n\
         axiom wn:v00065370_t : lexicon:Entity -> lexicon:Entity -> Prop\n",
    );
    for id in ids {
        s.push_str(&format!(
            "class hp:'{id}' {{ description = \"HP:{id} (stub)\"; }}\n"
        ));
    }
    s
}

/// The experiment's layers over a base.
pub struct Chain {
    pub layers: Vec<(&'static str, Arc<Layer>)>,
    pub refused: Arc<Layer>,
}

impl Chain {
    /// Every layer meant to commit.
    pub fn committed(&self) -> impl Iterator<Item = &(&'static str, Arc<Layer>)> {
        self.layers.iter()
    }

    pub fn top(&self) -> Arc<Layer> {
        Arc::clone(&self.layers.last().unwrap().1)
    }
}

/// base → `01` … `07`; `08-refused.esl` on its own branch.
pub fn chain_on(base: &dyn Base) -> Chain {
    let mut layers = Vec::new();
    let mut parent = base.head();
    for (name, src) in FILES {
        if name == "refused" {
            let refused = base.build(name, src, &parent, "refused");
            return Chain { layers, refused };
        }
        let layer = base.build(name, src, &parent, "main");
        layers.push((name, Arc::clone(&layer)));
        parent = layer;
    }
    unreachable!("08-refused.esl is the last file")
}

// ── The checks, over either base ──────────────────────────────────────────────────────────

/// Every layer meant to commit validates.
pub fn check_chain_validates(chain: &Chain) {
    for (name, layer) in chain.committed() {
        let errors = all_errors(layer);
        assert!(errors.is_empty(), "{name}: {errors:#?}");
    }
}

/// `08-refused.esl`'s three conclusions are refused, each for its reason.
pub fn check_refusals(chain: &Chain) {
    for (local, reason) in [
        // Claim 6 names no allele: nothing grounds Responsive(p.Leu44Pro).
        (
            "responsive_pro",
            "no admitted IsDeclaredAs witness for IRI urn:eigenius:uab:syngene:claim_6",
        ),
        // Responsiveness is stated of protein alleles; c.131T>C is a nucleotide allele.
        (
            "responsive_c131",
            "does not inhabit class urn:eigenius:variant:ProteinAllele",
        ),
        // The hypothesis has no trace, so no witness.
        (
            "chaperone_works",
            "no admitted IsDeclaredAs witness for IRI urn:eigenius:uab:syn-26-002:chaperone_hypothesis",
        ),
    ] {
        let errors = errors_for(
            &chain.refused,
            &format!("urn:eigenius:uab:syn-26-002:{local}"),
        );
        assert!(
            errors.iter().any(|e| e.contains(reason)),
            "{local} must be refused with `{reason}`; got {errors:#?}"
        );
    }
}

/// Each `GROUNDS` row commits when cited by its ground, and none when cited as Observed.
pub fn check_grounds(base: &dyn Base, chain: &Chain) {
    for ground in ["declared", "verified", "observed"] {
        let layer = base.build(
            "cite",
            &citations(ground),
            &chain.top(),
            &format!("cite-{ground}"),
        );
        for (n, (claim, _, _, kind)) in GROUNDS.iter().enumerate() {
            let expected = match ground {
                "declared" => true,
                "verified" => *kind == Ground::Verified,
                _ => false,
            };
            let errors = errors_for(&layer, &format!("urn:eigenius:test:cite:{ground}_{n}"));
            assert_eq!(
                errors.is_empty(),
                expected,
                "claim {claim}, cited as {ground}: {errors:#?}"
            );
            if ground == "observed" {
                assert!(
                    errors
                        .iter()
                        .any(|e| e.contains("no admitted IsObservedAs witness")),
                    "claim {claim}, cited as observed: {errors:#?}"
                );
            }
        }
    }
}

/// How a claim is grounded in this package. Nothing is Observed: the tier-3 records are not held
/// (D99 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    Declared,
    /// A checked proof; also Declared, by its author.
    Verified,
}

/// The analyst's seven claims and the four untyped items, each as the resource that grounds it,
/// its proposition (ESL), and its ground. `08-refused.esl` holds what grounds nothing.
pub const GROUNDS: &[(&str, &str, &str, Ground)] = &[
    (
        "1  carries c.131T>C, hemizygous",
        "urn:eigenius:pmi:SYN_26_002_carries_NM_000010_7_131_C",
        "clinical:Carries(pmi:SYN_26_002_proband, allele:NM_000010_7_131_C, clinical:hemizygous)",
        Ground::Declared,
    ),
    (
        "1  c.131T>C translates to p.Leu44Pro",
        "urn:eigenius:pmi:SYN_26_002_translates_NM_000010_7_131_C",
        "variant:TranslatesTo(allele:NM_000010_7_131_C, allele:HGNC_000010_p_44_Pro)",
        Ground::Declared,
    ),
    (
        "2  trial 1: no measurable reduction",
        "urn:eigenius:uab:syn-26-002:trial_1_outcome",
        "clinical:NoMeasurableReduction(syn002:trial_1)",
        Ground::Declared,
    ),
    (
        "2  trial 2: no measurable reduction",
        "urn:eigenius:uab:syn-26-002:trial_2_outcome",
        "clinical:NoMeasurableReduction(syn002:trial_2)",
        Ground::Declared,
    ),
    (
        "3  p.Leu44Pro: <4% of wild-type binding sites",
        "urn:eigenius:uab:syngene:claim_3",
        "syngene:MembraneSitesBelow(allele:HGNC_000010_p_44_Pro, 0.04r)",
        Ground::Declared,
    ),
    (
        "4  responders: 12.6-31.6% reductions",
        "urn:eigenius:uab:syngene:claim_4",
        "syngene:RespondersReducedWithin(syngene:survey_cohort, 0.126r, 0.316r)",
        Ground::Declared,
    ),
    (
        "5  the responsive cut-point",
        "urn:eigenius:uab:syngene:survey_cutpoint",
        "core:Asserts(\"urn:eigenius:uab:syngene:survey_cutpoint\")",
        Ground::Declared,
    ),
    (
        "6  some responsive protein allele is at Leu44",
        "urn:eigenius:uab:syngene:claim_6",
        "forall (R : Prop) => (forall (q : exists q : variant:ProteinAllele => \
         variant:At(q, location:HGNC_000010_p_44)) => syngene:Responsive(eigentt:fst(q)) -> R) -> R",
        Ground::Declared,
    ),
    (
        "7  chaperone rescue in cell models",
        "urn:eigenius:uab:syngene:claim_7_cells",
        "syngene:RescuesTraffickingInCells(syngene:chaperone_agents)",
        Ground::Declared,
    ),
    (
        "8a the survey cohort carried c.130C>T",
        "urn:eigenius:uab:syn-26-002:cohort_c130",
        "syngene:CohortCarried(syngene:survey_cohort, allele:NM_000010_7_130_T)",
        Ground::Declared,
    ),
    (
        "8a c.130C>T is not c.131T>C",
        "urn:eigenius:uab:syn-26-002:c130_not_c131",
        "eigentt:Eq(variant:NucleotideAllele, allele:NM_000010_7_130_T, allele:NM_000010_7_131_C) \
         -> logic:False",
        Ground::Verified,
    ),
    (
        "8b c.131T>C not listed in the release",
        "urn:eigenius:uab:syn-26-002:c131_not_in_clinvar",
        "clinvar:NotListed(syn002:clinvar_release, allele:NM_000010_7_131_C)",
        Ground::Declared,
    ),
    (
        "8b c.130C>T listed pathogenic in the release",
        "urn:eigenius:uab:syn-26-002:c130_in_clinvar",
        "clinvar:ListedAs(syn002:clinvar_release, allele:NM_000010_7_130_T, clinical:pathogenic)",
        Ground::Declared,
    ),
    (
        "8c c.131T>C is protein-mediated",
        "urn:eigenius:uab:syn-26-002:protein_mediated",
        "variant:ProteinMediated(allele:NM_000010_7_131_C)",
        Ground::Declared,
    ),
    (
        "8c the mechanism bridge at 4%",
        "urn:eigenius:uab:syn-26-002:mechanism",
        "forall (i : clinical:Individual) => forall (a : variant:NucleotideAllele) => \
         forall (q : variant:ProteinAllele) => clinical:Carries(i, a, clinical:hemizygous) \
         -> variant:TranslatesTo(a, q) -> variant:ProteinMediated(a) \
         -> syngene:MembraneSitesBelow(q, 0.04r) \
         -> syngene:ReducesOutput(syngene:first_line_agent, i) -> logic:False",
        Ground::Declared,
    ),
];

const CITE_PREAMBLE: &str = r#"namespace core          = "urn:eigenius:core";
namespace logic         = "urn:eigenius:logic";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace variant       = "urn:eigenius:variant";
namespace clinical      = "urn:eigenius:clinical";
namespace clinvar       = "urn:eigenius:clinvar";
namespace pmi           = "urn:eigenius:pmi";
namespace allele        = "urn:eigenius:variant:allele";
namespace location      = "urn:eigenius:variant:location";
namespace syngene       = "urn:eigenius:uab:syngene";
namespace syn002        = "urn:eigenius:uab:syn-26-002";
namespace cite          = "urn:eigenius:test:cite";
"#;

/// A layer of conclusions citing each `GROUNDS` row by `ground` (`declared`, `observed`,
/// `verified`): `cite:<ground>_<row>`.
pub fn citations(ground: &str) -> String {
    let mut s = String::from(CITE_PREAMBLE);
    for (n, (_, iri, prop, _)) in GROUNDS.iter().enumerate() {
        s.push_str(&format!(
            "resource cite:{ground}_{n} : justification:Conclusion {{\n    \
             justification:grounds_judgement = holds(eigentt:logic_kernel,\n        \
             type_expr({ground}(\"{iri}\", {prop}, ())),\n        \
             type_expr(justification:Grounds({prop})));\n}}\n"
        ));
    }
    s
}
