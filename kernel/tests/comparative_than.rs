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
//! snapshot-free. The comparative machinery (`than`/`cat_pp_than`, `less_deg`, the `elided_than`
//! grammar shift, the `cat_pp_arg` argument markers, `cat_measure`) is COMMITTED chain data from the
//! bootstrapped `ontologies/lexicon/closed-class.esl` + kernel grammar; the fixture adds only the
//! content word —
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
namespace lexicon      = "urn:eigenius:lexicon";
namespace epistemic    = "urn:eigenius:reflection:epistemic";
namespace core         = "urn:eigenius:core";
namespace measurements = "urn:eigenius:measurements";

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
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:cat_measure, lexicon:cat_pp_arg(lexicon:prep_on)) );
    lexicon:sem      = lexicon:dependent_rel_sem;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> core:float );
    lexicon:sense    = "wn:dependent.a.01";
    lexicon:grade    = epistemic:declared;
}

// `dependent` — the POSITIVE relational predication the importer emits for a governed adjective
// (C3-positive / Fix A (c)): `(S[adj]\NP)/cat_pp_arg(prep_on)`, sem `λr.λx. gt(deg_rel(r,x), std)`.
// Consumes the governed PP as the ground, then predicates against the ABSOLUTE standard, so the copula
// lifts it like any predicative adjective — "the lines were dependent ON WRN" binds WRN as the relatum.
axiom lexicon:std_dependent : core:float
resource lexicon:dependent_pos_rel_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (r : lexicon:Entity) => fun (x : lexicon:Entity) =>
            measurements:gt(lexicon:deg_dependent_rel(r, x), lexicon:std_dependent)
          : lexicon:Entity -> lexicon:Entity -> Prop )
    );
}
resource lexicon:dependent_pos_rel : lexicon:LexicalEntry {
    core:description = "relational adjective, POSITIVE predication (importer (S[adj]\\NP)/cat_pp_arg(prep_on)).";
    lexicon:form     = "dependent";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_pp_arg(lexicon:prep_on)) );
    lexicon:sem      = lexicon:dependent_pos_rel_sem;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wn:dependent.a.01";
    lexicon:grade    = epistemic:declared;
}

// `dependent` — the PLAIN gradable reading the importer also emits (1-place degree, no governed
// preposition). It feeds the ADJUNCT competitor: "less dependent" [plain] + "on WRN" as a free
// VP-adjunct, competing with the relational reading above. The argument/adjunct fix targets it.
axiom lexicon:deg_dependent : lexicon:Entity -> core:float
resource lexicon:dependent_plain_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (x : lexicon:Entity) => lexicon:deg_dependent(x) : lexicon:Entity -> core:float )
    );
}
resource lexicon:dependent_plain : lexicon:LexicalEntry {
    core:description = "plain gradable adjective: dependent (degree, no governed preposition).";
    lexicon:form     = "dependent";
    lexicon:cat      = type_expr( lexicon:cat_measure );
    lexicon:sem      = lexicon:dependent_plain_sem;
    lexicon:sem_type = type_expr( lexicon:Entity -> core:float );
    lexicon:sense    = "wn:dependent.a.02";
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

/// POSITIVE relational predication (Fix A piece (c), D63 §8.12): "lines were dependent on WRN" — no
/// comparative at all. The governed adjective consumes its `on`-PP as the GROUND and predicates against
/// the absolute standard, binding WRN as the relatum: `gt(deg_dependent_rel(wrn, lines), std_dependent)`.
/// CLOSED (the positive needs no comparison target, unlike the elided comparative). Before (c) this
/// reading did not exist — only the comparative consumed the relational `cat_measure`, so a positive
/// "dependent on WRN" could only strand the PP as a free VP-adjunct.
#[test]
fn positive_relational_predication_binds_the_ground() {
    let (closed, open) = parser().parse_open("lines were dependent on WRN", &Identity);
    assert!(
        closed.iter().any(|p| {
            let s = pretty_term(p.sem());
            s.contains("deg_dependent_rel") && s.contains("std_dependent") && s.contains("wrn")
        }),
        "the positive must bind WRN as the relatum vs the absolute standard; closed={:?} open={:?}",
        closed
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>(),
        open.iter()
            .map(|o| pretty_term(o.item.sem()))
            .collect::<Vec<_>>(),
    );
}

/// The argument/adjunct distinction (D63 §8.13, `suppress_governed_adjunct`). With both `dependent`
/// readings seeded (relational `dependent on` + plain gradable `dependent`), "lines were less dependent
/// on WRN" could parse two ways: the correct relational one (WRN is the argument of `dependent`, 2×
/// `deg_dependent_rel`) and a degenerate ADJUNCT competitor (plain `deg_dependent` + `on WRN` as a free
/// VP-adjunct, a stray `prep_on`). Because the ADJECTIVE head `dependent` governs `prep_on` and it is
/// immediately followed by `on`, the seed-time gate drops `on`'s VP-adjunct entry — so only the
/// relational reading survives and the `prep_on` adjunct is gone. Parse-time, no reseed.
#[test]
fn governed_pp_is_not_also_a_free_adjunct() {
    let (_closed, open) = parser().parse_open("lines were less dependent on WRN", &Identity);
    // The correct relational reading (WRN as argument, two relational degrees) must survive.
    assert!(
        open.iter().any(|o| pretty_term(o.item.sem())
            .matches("deg_dependent_rel")
            .count()
            == 2),
        "the relational reading must survive; got {:?}",
        open.iter()
            .map(|o| pretty_term(o.item.sem()))
            .collect::<Vec<_>>(),
    );
    // The degenerate adjunct reading (a free prep_on over the same span) must be GONE.
    assert!(
        !open
            .iter()
            .any(|o| pretty_term(o.item.sem()).contains("prep_on")),
        "governed 'on WRN' must be the argument of 'dependent', not a free VP-adjunct; got {:?}",
        open.iter()
            .map(|o| pretty_term(o.item.sem()))
            .collect::<Vec<_>>(),
    );
}
