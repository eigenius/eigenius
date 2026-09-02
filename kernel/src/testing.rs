//! Chains that in-crate tests compile ESL against.
//!
//! Compiling ESL needs a layer under it: almost every declaration lowers to values of
//! `eigentt:Term` and `core:Level` — a `param_kind`, a `type_name`, a `result_sort` — and in
//! the D85 §6.1 form each of those values names its constructor's arguments as properties.
//! Those names belong to the inductive's own declaration, so a compile with nothing beneath it
//! cannot write them, and `esl::compile` says so rather than inventing positional ones.

use std::sync::{Arc, OnceLock};

use crate::layer::{Layer, LayerBuilder, LayerStorage};

/// `core` + `eigentt-type-fragment`, built once per test binary.
///
/// The shortest chain carrying `core:Level` and `eigentt:Term`, which is all the ESL surface
/// needs; the rest of `BOOTSTRAP_CHAIN` adds only weight. Both are JSON, so building this
/// costs no ESL compile of its own.
pub(crate) fn term_chain() -> &'static Arc<Layer> {
    static CHAIN: OnceLock<Arc<Layer>> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let mut parent: Option<Arc<Layer>> = None;
        for (name, src) in [
            (
                "core",
                include_str!("../../ontologies/core/core-ontology.json"),
            ),
            (
                "eigentt-type-fragment",
                include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            ),
        ] {
            let mut b = LayerBuilder::new(name, parent);
            for r in
                crate::ontology::eigon_json::parse_document(src).expect("bootstrap JSON parses")
            {
                b.add_resource(r).expect("bootstrap resource");
            }
            parent = Some(Arc::new(b.build(LayerStorage::in_memory())));
        }
        parent.expect("two layers")
    })
}
