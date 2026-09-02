//! Chains that in-crate tests compile ESL against.
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
pub(crate) fn term_chain() -> &'static Arc<Layer> {
    static CHAIN: OnceLock<Arc<Layer>> = OnceLock::new();
    CHAIN.get_or_init(|| {
        Arc::clone(
            crate::bootstrap::bootstrap()
                .expect("the bootstrap chain builds")
                .head(),
        )
    })
}

/// The D47 codec's constructor argument names, read from [`term_chain`] once per test binary.
///
/// Encoding a term names its constructor's arguments (D85 §6.1), and those names come from
/// `eigentt:Term` and `core:Level`'s declarations — so an encode needs a chain just as a
/// compile does.
pub(crate) fn codec_names() -> &'static crate::program::eigentt_type_mirror::CodecNames {
    static NAMES: OnceLock<crate::program::eigentt_type_mirror::CodecNames> = OnceLock::new();
    NAMES.get_or_init(|| crate::program::eigentt_type_mirror::CodecNames::from_layer(term_chain()))
}
