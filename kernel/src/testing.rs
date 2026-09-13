//! Chains and fixture builders that tests compile ESL against.
//!
//! Public because the shape a fixture must produce is the chain's, not the test's: a term is
//! a resource whose `is_a` names its constructor's class and whose arguments are that class's
//! properties (D85 §6.1). Every crate whose tests author one needs the same builder, and a
//! second copy of it would be a second answer to what a value looks like.
//!
//! Compiling ESL needs a layer under it: almost every declaration lowers to values of
//! `eigentt:Term` and `core:Level` — a `param_kind`, a `type_name`, a `result_sort` — and in
//! the D85 §6.1 form each of those values names its constructor's arguments as properties.
//! Those names belong to the inductive's own declaration, so a compile with nothing beneath it
//! cannot write them, and `esl::compile` says so rather than inventing positional ones.

use std::sync::{Arc, OnceLock};

use crate::layer::Layer;

/// The bootstrap chain, built once per test binary.
///
/// It was `core` + `eigentt-type-fragment` — the shortest chain carrying `eigentt:Term` and
/// `core:Level` — until D85 §5 step 4. Constructor VALUES now name their class, so a test that
/// spells `OpRef(...)` needs `formulas:FormulaTerm` declared, not just the term language; the
/// minimal chain refused those with "`OpRef` is not a constructor". The full chain is what any
/// real compile runs against anyway.
pub fn term_chain() -> &'static Arc<Layer> {
    &bootstrap_parts().0
}

/// A fresh `ExecutionContext` over the bootstrap chain, whose LAYERS are built once per
/// test binary.
///
/// `bootstrap()` parses, validates and builds every ontology layer in `BOOTSTRAP_CHAIN`,
/// and it was being called once per TEST. In the slowest test binary that is 140 calls
/// across 157 tests, and it dominated the whole workspace suite: 796 seconds in CI for
/// that one file, with the kernel's own unit tests a further 571, against 7 seconds for
/// the 123 binaries that finish in under a second. Caching the layers took one file from
/// 66.6 to 7.5 seconds run serially.
///
/// **Each caller still gets its own `ExecutionContext`**, and that is the part that makes
/// sharing safe rather than lucky. A context owns a `working: LayerBuilder` that tests
/// add resources to, so one shared context would let tests write into each other's
/// working layer. What is shared is the finished chain and the storage behind it — layers
/// are content-addressed, so two tests building the same layer agree and two building
/// different ones do not collide.
///
/// Use this wherever a test called `bootstrap()` for a chain to build on. A test that
/// needs its OWN storage — one that persists, or that asserts on what a backend received —
/// should keep calling `bootstrap_with_storage` directly.
pub fn bootstrap_context() -> crate::context::ExecutionContext {
    let (head, storage) = bootstrap_parts();
    crate::context::ExecutionContext::new(
        Arc::clone(head),
        "working",
        crate::context::ExecutionMode::ReadWrite,
        storage.clone(),
    )
}

/// The bootstrap chain's head and its storage, built once per test binary.
///
/// Separate from [`term_chain`] only in also handing back the storage: a test that builds
/// a layer on this head must persist it to the same storage the head lives on, or the
/// layer is bound to one store and written to another.
fn bootstrap_parts() -> &'static (Arc<Layer>, crate::layer::LayerStorage) {
    static PARTS: OnceLock<(Arc<Layer>, crate::layer::LayerStorage)> = OnceLock::new();
    PARTS.get_or_init(|| {
        let ctx = crate::bootstrap::bootstrap().expect("the bootstrap chain builds");
        (Arc::clone(ctx.head()), ctx.storage().clone())
    })
}

/// The D47 codec's constructor argument names, read from [`term_chain`] once per test binary.
///
/// Encoding a term names its constructor's arguments (D85 §6.1), and those names come from
/// `eigentt:Term` and `core:Level`'s declarations — so an encode needs a chain just as a
/// compile does.
pub fn codec_names() -> &'static crate::program::eigentt_type_mirror::CodecNames {
    static NAMES: OnceLock<crate::program::eigentt_type_mirror::CodecNames> = OnceLock::new();
    NAMES.get_or_init(|| crate::program::eigentt_type_mirror::CodecNames::from_layer(term_chain()))
}

/// Materialise a tagged literal as the value resources it denotes — a FIXTURE builder.
///
/// Tests describe terms as `{"ctor": …, "args": […]}` because that reads well in a literal.
/// The values themselves are resources (D85 §6.1), so the literal is built out through the
/// declaration by [`CodecNames::value_of_tagged`]: a fixture cannot name a constructor, or an
/// arity, the chain does not have. Panicking on that is the point — it is a test fixture.
pub fn term_value(tagged: &serde_json::Value) -> crate::ontology::resource::Value {
    use crate::ontology::well_known as wk;
    codec_names()
        .value_of_tagged(&[wk::EIGENTT_TERM, wk::LEVEL], tagged)
        .unwrap_or_else(|e| panic!("fixture literal is not a value: {e}"))
}
