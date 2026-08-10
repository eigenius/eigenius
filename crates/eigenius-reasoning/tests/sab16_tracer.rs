// TEMPORARY tracer test for SAB 16 (D51 gap-5/gap-6 shakedown).
//
// Builds core → reflection → reasoning → bench-core → harness → mol →
// SAB-16 chain, then runs the ValidateJustification handler on the
// hand-authored `ImplementsRequiredFilter(solution)` ReasoningSentence and
// asserts Holds. The chain justifies the program's construction via five
// Declared methodological conformances composed through an acceptance rule
// (the corrected, program-as-object-code model). Move into the harness
// crate when D51 gap 7 lands.

use std::collections::BTreeSet;
use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;

fn esl_against(
    source: &str,
    parent: &Arc<eigenius_kernel::layer::Layer>,
    name: &str,
) -> Arc<eigenius_kernel::layer::Layer> {
    let resources = esl::compile_against_layer(source, parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile:\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent.clone()));
    for r in resources {
        b.add_resource(r).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn sab16_compound_filter_validates_to_holds() {
    // core
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let mut core_builder = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(core_json).unwrap() {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    // reflection (+ eigentt + institution)
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for src in [
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(src).unwrap() {
            reflection_builder.add_resource(r).unwrap();
        }
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    // reasoning (compiled standalone, as drug_screening.rs does)
    let reasoning_src = include_str!("../../../ontologies/reasoning/reasoning.esl");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in esl::compile(reasoning_src).expect("reasoning.esl compiles") {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    // bench-core → harness (bench:TaskOutput) → mol → SAB-16 tracer chain
    let bench_core = esl_against(
        include_str!("../../../experiments/benchmark/base-ontologies/bench-core.esl"),
        &reasoning,
        "bench-core",
    );
    let harness = esl_against(
        include_str!("../../../experiments/benchmark/harness-ontology.esl"),
        &bench_core,
        "harness",
    );
    let mol = esl_against(
        include_str!("../../../experiments/benchmark/base-ontologies/mol.esl"),
        &harness,
        "mol",
    );
    let chain = esl_against(
        include_str!(
            "../../../experiments/benchmark/tasks/sab/16-compound-filter/tracer-chain.esl"
        ),
        &mol,
        "sab16-chain",
    );

    // Force the witness index (admits IsDeclaredAs on the acceptance rule
    // + the five methodological conformances).

    let ctx = ExecutionContext::new(
        chain,
        "sab16-chain",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );

    let sentence_iri =
        Iri::parse("urn:eigenius:bench:sab16:concl_solution_implements").expect("sentence IRI");
    let sentence_arc = ctx
        .resolve(&sentence_iri)
        .unwrap_or_else(|| panic!("sentence `{sentence_iri}` should be on the chain"));
    let sentence = (*sentence_arc).clone();

    let inst = ReasoningInstitution::new();
    let outcome = do_validate_justification(&inst, &sentence, &ctx)
        .expect("validate handler returns an outcome");

    let ctor = outcome
        .output
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .expect("verdict carries ctor_name")
        .to_string();
    let diagnostic = outcome
        .output
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);

    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds; got {ctor}, diagnostic: {diagnostic:?}"
    );

    // ── Coverage check: the decision ↔ code-block overlay ───────────
    // Every methodological decision is realised by some CodeBlock (no
    // unimplemented method), and every block's label has a matching region
    // marker in the program payload (no dangling overlay).
    let solution = ctx
        .resolve(&Iri::parse("urn:eigenius:bench:sab16:solution").unwrap())
        .expect("solution TaskOutput on chain");
    let payload = solution
        .get(&Iri::parse("urn:eigenius:benchmark:payload").unwrap())
        .and_then(Value::as_str)
        .expect("solution carries a payload")
        .to_string();

    let codeblock_class = Iri::parse("urn:eigenius:benchmark:CodeBlock").unwrap();
    let label_iri = Iri::parse("urn:eigenius:benchmark:block_label").unwrap();
    let realizes_iri = Iri::parse("urn:eigenius:benchmark:realizes").unwrap();

    let mut realized: BTreeSet<String> = BTreeSet::new();
    let mut block_count = 0;
    for (_iri, r) in ctx.head().iter_all_resources() {
        if !r.is_instance_of(&codeblock_class) {
            continue;
        }
        block_count += 1;
        let label = r
            .get(&label_iri)
            .and_then(Value::as_str)
            .expect("CodeBlock carries a block_label");
        assert!(
            payload.contains(&format!("# region: {label}")),
            "block label `{label}` has no matching `# region:` marker in the program"
        );
        match r.get(&realizes_iri) {
            Some(Value::Array(items)) => {
                for it in items {
                    if let Some(s) = it.as_str() {
                        realized.insert(s.to_string());
                    }
                }
            }
            other => panic!("CodeBlock `{label}` realizes is not an array: {other:?}"),
        }
    }
    assert_eq!(block_count, 3, "expected 3 code blocks");

    let decisions: BTreeSet<String> = [
        "conf_featurization",
        "conf_alert_catalogs",
        "conf_reduction",
        "conf_threshold",
        "conf_composition",
    ]
    .iter()
    .map(|d| format!("urn:eigenius:bench:sab16:{d}"))
    .collect();

    assert_eq!(
        realized, decisions,
        "code blocks must realise exactly the five methodological decisions \
         (every decision implemented, none unwarranted)"
    );
}
