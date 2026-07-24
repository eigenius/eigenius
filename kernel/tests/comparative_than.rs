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

//! D63 §8.12 — the comparative `than` construction over a RELATIONAL adjective, end to end,
//! snapshot-free. The comparative machinery (`than`/`cat_pp_than`, `less_deg`/`less_deg_bare`,
//! the `cat_pp_arg` argument markers, `cat_measure`) is COMMITTED chain data from the
//! bootstrapped `ontologies/lexicon/closed-class.esl`; the fixture adds only the content word —
//! a relational adjective `dependent` shaped EXACTLY as the WordNet importer emits a
//! gloss-governed adjective (`cat_measure / cat_pp_arg(prep_on)`, a 2-place
//! `deg_dependent_rel : Entity → Entity → float`) — plus the entities. No DB, no LLM, no reseed.
//!
//! This is the snapshot-free guard for Fix A's relational comparative (the WRN-page unit
//! "The lines from rare lineages were less dependent on WRN" is the ELIDED case below), and the
//! executable spec for the `than`-standard cases: NP (subject standard, works) and PP (relatum
//! standard, the known relational-comparative gap — `#[ignore]`d until that slice lands).

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

/// Bootstrap (lexicon schema + the committed closed-class comparative layer), then layer a fixture
/// that seeds one relational adjective `dependent` and the entities WRN / MSI / the lines / their
/// counterparts. The importer emits a gloss-governed adjective as `cat_measure / cat_pp_arg(prep_on)`
/// with a 2-place degree; this fixture mirrors that emission exactly (no importer, no snapshot).
const FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
namespace core      = "urn:eigenius:core";

// Relational degree: deg_dependent_rel(relatum, subject) : float. "on WRN" fills the relatum.
axiom lexicon:deg_dependent_rel : lexicon:Entity -> lexicon:Entity -> core:float

// `dependent` — gloss-governed relational adjective. Consumes its `on`-PP (the relatum, via
// cat_pp_arg(prep_on)) and yields a cat_measure `λx. deg_dependent_rel(relatum, x)` : Entity → float.
resource lexicon:dependent_rel_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (r : lexicon:Entity) => fun (x : lexicon:Entity) => lexicon:deg_dependent_rel(r, x)
          : lexicon:Entity -> lexicon:Entity -> core:float )
    );
}
resource lexicon:dependent_rel : lexicon:LexicalEntry {
    core:description = "relational adjective: X depends on Y (importer cat_measure/cat_pp_arg(prep_on)).";
    lexicon:form     = "dependent";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:cat_measure, lexicon:cat_pp_arg(lexicon:prep_on)) );
    lexicon:sem      = lexicon:dependent_rel_sem;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> core:float );
    lexicon:sense    = "wn:dependent.a.01";
    lexicon:grade    = epistemic:declared;
}

// Entities + their proper-noun / bare-plural NP entries.
axiom lexicon:wrn : lexicon:Entity
resource lexicon:wrn_sem : lexicon:SemTerm { lexicon:term = type_expr( lexicon:wrn ); }
resource lexicon:wrn_np : lexicon:LexicalEntry {
    lexicon:form = "WRN"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:num_any) );
    lexicon:sem = lexicon:wrn_sem; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "wrn"; lexicon:grade = epistemic:declared;
}
axiom lexicon:msi : lexicon:Entity
resource lexicon:msi_sem : lexicon:SemTerm { lexicon:term = type_expr( lexicon:msi ); }
resource lexicon:msi_np : lexicon:LexicalEntry {
    lexicon:form = "MSI"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:num_any) );
    lexicon:sem = lexicon:msi_sem; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "msi"; lexicon:grade = epistemic:declared;
}
axiom lexicon:the_lines : lexicon:Entity
resource lexicon:the_lines_sem : lexicon:SemTerm { lexicon:term = type_expr( lexicon:the_lines ); }
resource lexicon:lines_np : lexicon:LexicalEntry {
    lexicon:form = "lines"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:pl) );
    lexicon:sem = lexicon:the_lines_sem; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "lines"; lexicon:grade = epistemic:declared;
}
axiom lexicon:counterparts : lexicon:Entity
resource lexicon:counterparts_sem : lexicon:SemTerm { lexicon:term = type_expr( lexicon:counterparts ); }
resource lexicon:counterparts_np : lexicon:LexicalEntry {
    lexicon:form = "counterparts"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:num_any) );
    lexicon:sem = lexicon:counterparts_sem; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "counterparts"; lexicon:grade = epistemic:declared;
}
"#;

fn parser() -> Parser {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources = esl::compile_against_layer(FIXTURE, ctx.head()).expect("fixture compiles");
    let mut b = LayerBuilder::new("cmp-than", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add fixture resource");
    }
    let layer: Arc<Layer> = Arc::new(b.build(LayerStorage::in_memory()));
    Parser::build(layer)
}

/// ELIDED `than` (the WRN-page form, Fix A): "lines were less dependent on WRN" — no explicit
/// standard, so the comparison target is anaphoric (`lexicon:anaphor`) and the whole clause is an
/// OPEN parse: a Π-abstraction `λh. gt(deg_dependent_rel(wrn, h), deg_dependent_rel(wrn, the_lines))`
/// (the standard is the abstracted parameter the D64 resolver fills from discourse). WRN is the
/// RELATUM of both measured dependences.
#[test]
fn elided_than_is_an_open_relational_comparative() {
    let (closed, open) = parser().parse_open("lines were less dependent on WRN", &Identity);
    assert!(
        open.iter().any(|o| {
            o.holes.len() == 1 && {
                let s = pretty_term(o.item.sem());
                s.starts_with('λ')
                    && s.matches("deg_dependent_rel").count() == 2
                    && s.contains("wrn")
            }
        }),
        "elided comparative must be OPEN with one abstracted standard over the relational degree; \
         closed={:?} open={:?}",
        closed
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>(),
        open.iter()
            .map(|o| (o.holes.len(), pretty_term(o.item.sem())))
            .collect::<Vec<_>>(),
    );
}

/// EXPLICIT `than [NP]` — the SUBJECT standard: "lines were less dependent on WRN than counterparts"
/// means the counterparts' dependence-on-WRN exceeds the lines' (both measured against WRN). CLOSED
/// (no hole): `gt(deg_dependent_rel(wrn, counterparts), deg_dependent_rel(wrn, the_lines))`. This is
/// the case the committed complement `than_marker` + `less_deg` already build.
#[test]
fn explicit_than_np_is_a_closed_subject_standard_comparative() {
    let (closed, _open) = parser().parse_open(
        "lines were less dependent on WRN than counterparts",
        &Identity,
    );
    assert!(
        closed.iter().any(|p| {
            let s = pretty_term(p.sem());
            !s.starts_with('λ')
                && s.matches("deg_dependent_rel").count() == 2
                && s.contains("counterparts")
                && s.contains("wrn")
        }),
        "than-NP must give a closed subject-standard comparison over WRN; got {:?}",
        closed
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>(),
    );
}

/// KNOWN GAP — EXPLICIT `than [PP]`, the RELATUM standard: "lines were less dependent on WRN than on
/// MSI" means dependence-on-WRN vs dependence-on-MSI for the SAME subject:
/// `gt(deg_dependent_rel(wrn, the_lines), deg_dependent_rel(msi, the_lines))`. This needs a
/// two-measure (relational) comparative — the single-μ degree sem `gt(μ(x), μ(std))` cannot express
/// a relatum-varying comparison, and `than_marker` consumes an NP not a PP. It is its own slice, and
/// does NOT occur on the WRN page. Flip this on when the relational-comparative slice lands.
#[test]
#[ignore = "known gap: than-PP relatum-standard needs the relational (two-measure) comparative slice"]
fn explicit_than_pp_is_a_relatum_standard_comparative() {
    let (closed, _open) =
        parser().parse_open("lines were less dependent on WRN than on MSI", &Identity);
    assert!(
        closed.iter().any(|p| {
            let s = pretty_term(p.sem());
            s.contains("msi") && s.contains("wrn") && s.matches("deg_dependent_rel").count() == 2
        }),
        "than-PP must compare dependence on WRN vs on MSI for the same subject; got {:?}",
        closed
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>(),
    );
}
