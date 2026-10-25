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

//! **Gap-boundary localization** — for a sentence that got no parse, say WHERE the derivation stopped
//! and WHICH two categories would not combine there.
//!
//! The two existing instruments answer neighbouring questions. `EIGENIUS_DUMP_CELL` prints one cell,
//! which presupposes you already know which cell to look at. `EIGENIUS_TRACE_FOREST` prints the
//! hyperedge DAG, which is the right tool once a constituent exists to be explained. Neither answers
//! the question a grammar gap actually poses: *this sentence produced nothing — where did it stop?*
//! Without that, diagnosis falls back to behavioural A/B (minimal pairs, ablations), which measures
//! the OUTPUT of the whole pipeline and therefore cannot separate a missing category from a feature
//! clash from a normal-form guard. Five successive hypotheses about the XIAP corpus's blockers were
//! formed that way and four were refuted; the one that held was found by instrumentation.
//!
//! ## What it computes
//!
//! **Minimal tiling.** A cell `[i..j]` is *derivable* iff the forest holds a node there. Tile the
//! whole token span with as few derivable cells as possible (a DP over cut positions). A sentence
//! that parses tiles in 1; a gap tiles in `k > 1`, and the `k - 1` seams are where the grammar ran
//! out. This is the measured analogue of "the adjacent maximal spans that don't combine", and unlike
//! re-parsing substrings it reads the REAL chart — same seed, same multiword preference, same cap.
//!
//! **Forced cuts.** A cut position `p` is *forced* iff no minimal tiling straddles it: every minimal
//! tiling breaks there. Computed as `cross[p] > best`, where `cross[p]` is the cheapest tiling whose
//! blocks include one spanning across `p`. A forced cut is a property of the grammar's coverage of
//! this sentence, not of which tiling the DP happened to pick — which is what makes it reportable as
//! *the* gap rather than as one arbitrary decomposition among many.
//!
//! **Nearest miss.** At each seam, cross every left-block node against every right-block node and
//! rank the refusals ([`Verdict`]). Each verdict is *derived* — it names the predicate that was
//! evaluated and failed ([`unify_cat`], [`feat_meets`], [`type_subsumes`], a [`harmonic_licenses`]
//! mode test, a normal-form provenance guard) — rather than inferred from the shape of the sentence.
//! The strongest verdict found at a seam is the one to act on.
//!
//! ## Caveats the output states
//!
//! - **Every rule is tested, but only `apply` is explained.** The cross product runs [`apply`] (the
//!   universal combinators, the nominal-modification rules, the other-grammar rules, the group rules)
//!   and — through the caller's `bin_rule_combines` — the *token-keyed* binary rules, which are
//!   span-keyed and unreachable from a pure function over the forest. A token-keyed rule that
//!   succeeds yields `PRUNED` like any other; when none succeeds, the structural verdict describes
//!   the universal rules. Reporting instead that a seam merely *coincides* with a binary-rule SITE
//!   would say nothing: `trig_appose_group` emits a site at every split of every cell, so that note
//!   fired at all three seams of the first sentences tested and meant nothing at any of them.
//! - **Reps, not readings.** As with the tracer, a packed node carries one representative item. A
//!   verdict is exact for everything the node packs (the packing invariant guarantees the decision is
//!   a function of the signature), but the sem shown is the rep's.
//! - **Fires per cap rung.** The widen ladder re-parses; a gapping sentence therefore reports once per
//!   rung, and the header carries the cap so the rungs can be told apart. A rung whose report repeats
//!   the previous one prints a single line instead, so the rungs that actually CHANGED the diagnosis
//!   — a seam that closes at a wider cap is a `PRUNED` finding — are the ones that stand out.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::term::Exp;

use super::super::category::{feat_meets, is_ctor, slash_parts, type_subsumes, unify_cat};
use super::super::item::Item;
use super::super::preprocess::{join_surfaces, Token};
use super::super::pretty::pretty_term;
use super::super::rules::combinators::{apply, harmonic_licenses};
use super::super::rules::RightContext;
use super::forest::{Forest, NodeId};

/// Whether any token-keyed binary rule combines a seam's pair, given both operand spans and the two
/// representative items. The *caller* supplies this: those rules are keyed to reserved words in the
/// token stream, which a pure function over the forest does not have.
type BinRuleCombines<'a> = &'a dyn Fn((usize, usize), (usize, usize), &Item, &Item) -> bool;

/// How close a left/right node pair came to combining, worst to best. The ordering is the point: the
/// seam's report leads with the MAXIMUM verdict over the cross product, because that is the smallest
/// change to the grammar that would bridge the seam.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Verdict {
    /// Neither operand is a functor whose slash faces the other, and no dependent-application
    /// constructor (`cat_forall` / `cat_unit_forall`) is in play. Nothing about these two categories
    /// could ever combine — the gap is that one of them is the wrong category outright.
    NoFunctor,
    /// A functor faces the right way, but its argument slot and the operand are built from DIFFERENT
    /// constructors (`cat_pp` offered to a `cat_np` slot). A categorial gap: the construction has no
    /// entry, not a feature that failed to meet.
    WrongArgument,
    /// The slot and the operand share a constructor and differ only in a FEATURE position, named in
    /// the report. The grammar has the construction; this instance's mood / number / finiteness /
    /// preposition does not meet. Usually the cheapest real fix — a `feat_meets` subsumption or a
    /// lexical entry carrying the other feature.
    FeatureClash(&'static str),
    /// Both operands are same-direction functors whose slot unifies, so composition is structurally
    /// available, but a slash MODE refuses it (`harmonic_licenses`). A governed complement cannot be
    /// composed away from its head.
    ModeBlocked,
    /// The argument slot unifies and no mode objects — the combination is refused by an Eisner
    /// normal-form provenance guard. The report names both operands' provenance so the guard can be
    /// identified against `ProvGuard`.
    NormalForm,
    /// A rule SUCCEEDS on the representatives — [`apply`], or one of the token-keyed binary rules.
    /// The cell is empty anyway, so the result was dropped by the sense cap or the cell beam: a
    /// pruning artifact, not a grammar gap. The one verdict that says "do not change the grammar".
    Pruned(&'static str),
}

impl Verdict {
    fn label(self) -> String {
        match self {
            Verdict::NoFunctor => "NO-FUNCTOR".into(),
            Verdict::WrongArgument => "WRONG-ARG".into(),
            Verdict::FeatureClash(pos) => format!("FEATURE({pos})"),
            Verdict::ModeBlocked => "MODE".into(),
            Verdict::NormalForm => "NORMAL-FORM".into(),
            Verdict::Pruned(rule) => format!("PRUNED({rule})"),
        }
    }
}

/// `usize::MAX` stands for "no tiling reaches here"; saturating arithmetic keeps it absorbing.
const UNREACHABLE: usize = usize::MAX;

/// Which cells are derivable: `occ[i][j]` iff the forest holds a node spanning tokens `i..=j`.
///
/// The tiling analysis below takes THIS rather than the forest, because none of it is about
/// categories — it is combinatorics over which spans exist. Separating them is what lets the DP and
/// the forced-cut rule be tested on hand-written occupancy matrices instead of on a chart that would
/// have to be parsed into existence first.
type Occupancy = Vec<Vec<bool>>;

fn occupancy(forest: &Forest, n: usize) -> Occupancy {
    (0..n)
        .map(|i| (0..n).map(|j| !forest.cells[i][j].is_empty()).collect())
        .collect()
}

/// Minimum number of derivable cells needed to tile tokens `[0, p)`, for every `p` in `0..=n`.
/// `fwd[0] = 0`; `fwd[p] = min over derivable [i..p-1] of fwd[i] + 1`.
fn forward_tiling(occ: &Occupancy, n: usize) -> Vec<usize> {
    let mut fwd = vec![UNREACHABLE; n + 1];
    fwd[0] = 0;
    for p in 1..=n {
        for i in 0..p {
            if fwd[i] == UNREACHABLE || !occ[i][p - 1] {
                continue;
            }
            fwd[p] = fwd[p].min(fwd[i] + 1);
        }
    }
    fwd
}

/// Minimum number of derivable cells needed to tile tokens `[p, n)`, for every `p` in `0..=n`.
/// `bwd[n] = 0`; `bwd[p] = min over derivable [p..j] of bwd[j + 1] + 1`.
fn backward_tiling(occ: &Occupancy, n: usize) -> Vec<usize> {
    let mut bwd = vec![UNREACHABLE; n + 1];
    bwd[n] = 0;
    for p in (0..n).rev() {
        for j in p..n {
            if bwd[j + 1] == UNREACHABLE || !occ[p][j] {
                continue;
            }
            bwd[p] = bwd[p].min(bwd[j + 1] + 1);
        }
    }
    bwd
}

/// One minimal tiling, as the blocks `[i..j]` it uses, reconstructed from `fwd`/`bwd`: at each
/// position take the block that keeps the total at `best`, preferring the LONGEST such block so the
/// report shows maximal constituents rather than a shredded equivalent.
fn minimal_tiling(occ: &Occupancy, n: usize, fwd: &[usize], bwd: &[usize]) -> Vec<(usize, usize)> {
    let best = fwd[n];
    let mut blocks = Vec::new();
    let mut p = 0;
    while p < n {
        let mut chosen = None;
        for j in (p..n).rev() {
            if !occ[p][j] || bwd[j + 1] == UNREACHABLE {
                continue;
            }
            if fwd[p].saturating_add(1).saturating_add(bwd[j + 1]) == best {
                chosen = Some(j);
                break;
            }
        }
        // `fwd[p] + bwd[p] == best` holds for every position a minimal tiling reaches, so some block
        // always qualifies; the guard keeps a corrupt table from looping instead of reporting.
        let Some(j) = chosen else { break };
        blocks.push((p, j));
        p = j + 1;
    }
    blocks
}

/// The cut positions every minimal tiling breaks at. `p` (between tokens `p-1` and `p`) is forced iff
/// no minimal tiling has a block straddling it: a tiling either cuts at `p` or contains a block that
/// crosses it, so `cross[p] > best` makes the cut unavoidable.
///
/// This is the instrument's sharpest output. A seam read off ONE tiling is an artifact of which
/// decomposition the DP happened to reconstruct; a forced cut is a property of the grammar's coverage
/// of this sentence, true of every minimal tiling there is.
fn forced_cuts(occ: &Occupancy, n: usize, fwd: &[usize], bwd: &[usize]) -> Vec<usize> {
    let best = fwd[n];
    let mut forced = Vec::new();
    for p in 1..n {
        let mut cross = UNREACHABLE;
        for (i, &fwd_i) in fwd.iter().enumerate().take(p) {
            if fwd_i == UNREACHABLE {
                continue;
            }
            for j in p..n {
                if !occ[i][j] || bwd[j + 1] == UNREACHABLE {
                    continue;
                }
                cross = cross.min(fwd_i.saturating_add(1).saturating_add(bwd[j + 1]));
            }
        }
        if cross > best {
            forced.push(p);
        }
    }
    forced
}

/// How ONE argument position of a category constructor is matched, mirroring `category::unify_into`.
/// The positions do NOT all use the same test, and using one uniform predicate for all of them is
/// how a diagnostic starts naming the wrong cause: `cat_s`'s mood matches by EQUALITY (a mood is
/// semantic, so there is no wildcard to meet), while its finiteness goes through the feature-meet.
enum PosTest {
    /// Exact equality — `cat_s`'s mood, a measure phrase's reading.
    Eq,
    /// The feature-meet lattice (`feat_meets`) — number, finiteness, connective, preposition.
    Feat,
    /// The subclass lattice (`type_subsumes`), var-aware — a `cat_np` / `cat_n` / `cat_group` type
    /// index. A slot variable binds to anything, so it never fails.
    Type,
    /// A measure phrase's unit: a slot variable binds, a literal matches only itself. Units have no
    /// subtyping.
    Unit,
}

impl PosTest {
    fn holds(&self, slot: &Exp, arg: &Exp, layer: &Arc<Layer>) -> bool {
        match self {
            PosTest::Eq => slot == arg,
            PosTest::Feat => slot == arg || feat_meets(slot, arg),
            PosTest::Type => matches!(slot, Exp::Var(_)) || type_subsumes(slot, arg, layer),
            PosTest::Unit => matches!(slot, Exp::Var(_)) || slot == arg,
        }
    }
}

/// The argument positions of each category constructor `unify_into` decomposes, with the test each
/// one runs. Kept in the same order as `unify_into`'s arms so the two can be read against each other.
fn ctor_positions(ctor: &str) -> Option<&'static [(&'static str, PosTest)]> {
    Some(match ctor {
        "cat_np" => &[("type", PosTest::Type), ("number", PosTest::Feat)],
        "cat_n" => &[("type", PosTest::Type), ("number", PosTest::Feat)],
        "cat_group" => &[
            ("type", PosTest::Type),
            ("conn", PosTest::Feat),
            ("number", PosTest::Feat),
        ],
        "cat_s" => &[("mood", PosTest::Eq), ("fin", PosTest::Feat)],
        "cat_pp_arg" => &[("prep", PosTest::Feat)],
        "cat_mp" | "cat_mpc" | "cat_mpq" => &[("unit", PosTest::Unit), ("reading", PosTest::Eq)],
        _ => return None,
    })
}

/// Name the position at which two same-constructor categories diverge, by re-running that position's
/// own test. This is what makes the label a derived fact rather than a guess about the shape of the
/// sentence. `None` when the two are built from different constructors (a categorial gap, not a
/// feature one) or when every position holds (the refusal is elsewhere).
fn feature_position(slot: &Exp, arg: &Exp, layer: &Arc<Layer>) -> Option<&'static str> {
    let Exp::InductiveCtor(_, ctor, s) = slot else {
        return None;
    };
    let a = is_ctor(arg, ctor)?;
    let positions = ctor_positions(ctor)?;
    if s.len() != a.len() || s.len() != positions.len() {
        return None;
    }
    positions
        .iter()
        .enumerate()
        .find(|(k, (_, test))| !test.holds(&s[*k], &a[*k], layer))
        .map(|(_, (name, _))| *name)
}

/// Diagnose ONE ordered pair of adjacent nodes. Each branch evaluates the predicate the parser itself
/// would have evaluated, so the verdict names a cause rather than classifying a surface shape.
fn diagnose(
    left: &Item,
    right: &Item,
    layer: &Arc<Layer>,
    rctx: RightContext,
    bin_combines: &dyn Fn(&Item, &Item) -> bool,
) -> Verdict {
    // The cell is empty, so a success here means the result existed and was pruned away. Both rule
    // families are tried: a token-keyed rule that combines the pair is as much a pruning artifact as
    // a universal one, and treating only `apply` as evidence would call it a grammar gap.
    if apply(left, right, layer, rctx).is_some() {
        return Verdict::Pruned("apply");
    }
    if bin_combines(left, right) {
        return Verdict::Pruned("token-keyed");
    }
    let (lc, rc) = (&left.category.cat, &right.category.cat);
    let mut best = Verdict::NoFunctor;

    // Application, both directions. Application is keyed to the lattice root `⋆`, so no mode can
    // refuse it: a functor facing the right way either unifies its slot or does not.
    for (functor, operand, dir) in [(lc, rc, "fwd"), (rc, lc, "bwd")] {
        let Some((_mode, _res, slot)) = slash_parts(functor, dir) else {
            continue;
        };
        if unify_cat(slot, operand, layer).is_some() {
            // The slot accepts the operand and application has no mode test left to fail, so what
            // refused the pair is a normal-form provenance guard.
            best = best.max(Verdict::NormalForm);
        } else if let Some(pos) = feature_position(slot, operand, layer) {
            best = best.max(Verdict::FeatureClash(pos));
        } else {
            best = best.max(Verdict::WrongArgument);
        }
    }

    // Composition. `fwd` ONLY: `comb_rules()` registers `forward_comp` and no backward counterpart,
    // so testing `bwd` here would report a mode or normal-form refusal for a rule the parser never
    // tries — a verdict about an absent rule rather than about this sentence.
    if let (Some((lm, _l_res, l_arg)), Some((rm, r_res, _r_arg))) =
        (slash_parts(lc, "fwd"), slash_parts(rc, "fwd"))
    {
        if unify_cat(l_arg, r_res, layer).is_some() {
            if !harmonic_licenses(lm) || !harmonic_licenses(rm) {
                best = best.max(Verdict::ModeBlocked);
            } else {
                best = best.max(Verdict::NormalForm);
            }
        }
    }

    // Dependent application (the determiner, `DepApply`) destructures a `cat_forall` against a
    // `cat_n` and gates on `feat_meets(det_num, noun_num)` — so a determiner meeting a noun of the
    // wrong number is a FEATURE clash with a name, not an unanalysed non-combination.
    if let (Some([det_num, _body]), Some([_t, noun_num])) =
        (is_ctor(lc, "cat_forall"), is_ctor(rc, "cat_n"))
    {
        best = best.max(if feat_meets(det_num, noun_num) {
            Verdict::NormalForm
        } else {
            Verdict::FeatureClash("det-number")
        });
    }

    // The remaining dependent-application functors destructure a `cat_forall` / `cat_unit_forall`
    // instead of a slash, so a pair involving one is never a NO-FUNCTOR even when neither side
    // carries a slash.
    for ctor in ["cat_forall", "cat_unit_forall"] {
        if is_ctor(lc, ctor).is_some() || is_ctor(rc, ctor).is_some() {
            best = best.max(Verdict::WrongArgument);
        }
    }
    best
}

/// A node's one-line identity in a seam report.
fn node_brief(forest: &Forest, id: NodeId) -> String {
    let node = &forest.nodes[id];
    format!("{} [{:?}]", pretty_term(node.rep.cat()), node.rep.prov())
}

/// The token text of span `[i..j]`, space-joined.
fn span_text(tokens: &[Token], i: usize, j: usize) -> String {
    tokens
        .get(i..=j.min(tokens.len().saturating_sub(1)))
        .map(join_surfaces)
        .unwrap_or_default()
}

/// How many distinct categories a seam report prints per side before it elides.
const SIDE_CAT_CAP: usize = 8;

/// Report one seam: the two blocks, their categories, and the nearest miss over the cross product.
fn report_seam(
    forest: &Forest,
    tokens: &[Token],
    layer: &Arc<Layer>,
    (li, lj): (usize, usize),
    (ri, rj): (usize, usize),
    bin_rule_combines: BinRuleCombines,
    out: &mut String,
) {
    let lefts: Vec<NodeId> = forest.cells[li][lj].values().copied().collect();
    let rights: Vec<NodeId> = forest.cells[ri][rj].values().copied().collect();
    out.push_str(&format!(
        "  SEAM [{li}..{lj}] «{}»  ⊕  [{ri}..{rj}] «{}»   ({}×{} nodes)\n",
        span_text(tokens, li, lj),
        span_text(tokens, ri, rj),
        lefts.len(),
        rights.len(),
    ));
    // A minimal-tiling block always holds nodes; a diagnostic still says so rather than indexing blind.
    let (Some(&_l0), Some(&r0)) = (lefts.first(), rights.first()) else {
        out.push_str("    (empty block — no nodes to diagnose)\n");
        return;
    };
    // The right operand's right context is the combined cell's, as both chart drivers compute it.
    let rctx = forest.nodes[r0].rctx;
    let mut best: Option<(Verdict, NodeId, NodeId)> = None;
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    for &l in &lefts {
        for &r in &rights {
            let (lrep, rrep) = (&forest.nodes[l].rep, &forest.nodes[r].rep);
            let v = diagnose(lrep, rrep, layer, rctx, &|a, b| {
                bin_rule_combines((li, lj), (ri, rj), a, b)
            });
            *tally.entry(v.label()).or_default() += 1;
            if best.is_none_or(|(b, _, _)| v > b) {
                best = Some((v, l, r));
            }
        }
    }

    for (side, ids) in [("left ", &lefts), ("right", &rights)] {
        let shown: Vec<String> = ids
            .iter()
            .take(SIDE_CAT_CAP)
            .map(|&id| node_brief(forest, id))
            .collect();
        let elided = ids.len().saturating_sub(SIDE_CAT_CAP);
        out.push_str(&format!("    {side} cats: {}", shown.join(" | ")));
        if elided > 0 {
            out.push_str(&format!(" … (+{elided})"));
        }
        out.push('\n');
    }

    if let Some((v, l, r)) = best {
        out.push_str(&format!(
            "    NEAREST MISS {}\n      left  #{l} {}\n      right #{r} {}\n",
            v.label(),
            node_brief(forest, l),
            node_brief(forest, r),
        ));
    }
    let counts: Vec<String> = tally.iter().map(|(k, n)| format!("{k}×{n}")).collect();
    out.push_str(&format!("    all pairs: {}\n", counts.join("  ")));
}

thread_local! {
    /// The previous rung's report BODY, for [`gap_report`]'s rung dedup. A diagnostic-only cell: it
    /// is read and written only on the `EIGENIUS_GAP_BOUNDARY` path, so no production parse touches
    /// it, and threading a dedup argument down through `parse_scoped_open` to serve a diagnostic
    /// would put it in a signature every caller pays for.
    static LAST_BODY: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Drop `#<digits>` node references, so the dedup key compares what a seam SAYS rather than which
/// arena slots this rung happened to allocate.
fn strip_node_ids(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '#' {
            out.push(c);
            continue;
        }
        while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
            chars.next();
        }
        out.push('#');
    }
    out
}

/// Forget the previous rung's body, so the next report prints in full. Called when a new SENTENCE
/// starts; without it a sentence whose report happens to match the previous sentence's last rung
/// would be elided as a repeat.
pub(crate) fn reset_rung_dedup() {
    LAST_BODY.with(|b| b.borrow_mut().clear());
}

/// Render the gap report for `forest`, prefixed by the caller's `header` (cap / token context).
///
/// `top` is the top-span finite-clause / question node set the parser already computed. A sentence
/// whose whole span IS derivable but whose `top` is empty failed at the ROOT category, not at a seam —
/// a distinct diagnosis the report makes explicitly, since the two are indistinguishable from the fact
/// of getting no parse. `bin_rule_combines(left_span, right_span, l, r)` asks the caller — the only
/// holder of the token stream — whether any token-keyed binary rule actually combines that pair;
/// see the module caveats.
pub(crate) fn gap_report(
    forest: &Forest,
    tokens: &[Token],
    top: &[NodeId],
    layer: &Arc<Layer>,
    bin_rule_combines: BinRuleCombines,
    header: &str,
) -> String {
    let body = gap_report_body(forest, tokens, top, layer, bin_rule_combines);
    // The widen ladder re-parses, so a gapping sentence reports once per cap rung. The rungs agree
    // unless widening actually changed the chart — and when they do agree, printing the same seam
    // four times buries the ones that differ. Print the body only when it CHANGES; a rung whose
    // diagnosis moved (a seam that closes at a wider cap is a `PRUNED` finding, not a grammar gap)
    // still prints in full. The key strips node IDS, which are indices into THIS rung's arena and
    // shift whenever the forest grows — keying on them defeats the dedup entirely, as the first run
    // showed.
    let key = strip_node_ids(&body);
    let repeat = LAST_BODY.with(|last| {
        let mut last = last.borrow_mut();
        if *last == key {
            true
        } else {
            *last = key;
            false
        }
    });
    if repeat {
        return format!("{header}  (unchanged from previous cap rung)\n");
    }
    format!("{header}\n{body}")
}

/// The report proper, without the header or the rung dedup. See [`gap_report`].
fn gap_report_body(
    forest: &Forest,
    tokens: &[Token],
    top: &[NodeId],
    layer: &Arc<Layer>,
    bin_rule_combines: BinRuleCombines,
) -> String {
    let n = tokens.len();
    let mut out = String::new();
    if n == 0 {
        out.push_str("  (no tokens)\n");
        return out;
    }

    // Tokens no derivable cell covers at all — a lexical gap, which no amount of category analysis
    // further up will explain. Reported first because it subsumes every seam it causes.
    let occ = occupancy(forest, n);
    let uncovered: Vec<usize> = (0..n)
        .filter(|&t| !(0..=t).any(|i| (t..n).any(|j| occ[i][j])))
        .collect();
    if !uncovered.is_empty() {
        let words: Vec<String> = uncovered
            .iter()
            .map(|&t| format!("{t}:«{}»", tokens[t].surface()))
            .collect();
        out.push_str(&format!(
            "  LEXICAL: {} token(s) in no derivable cell: {}\n",
            uncovered.len(),
            words.join(" "),
        ));
    }

    let fwd = forward_tiling(&occ, n);
    let bwd = backward_tiling(&occ, n);
    if fwd[n] == UNREACHABLE {
        out.push_str(
            "  NO TILING: the sentence cannot be covered by derivable cells at all \
             (see LEXICAL above, or a multiword cell with no single-token fallback)\n",
        );
        return out;
    }

    if fwd[n] == 1 {
        // The whole span derives; the failure is the root category, not a seam.
        out.push_str(&format!(
            "  ROOT: the full span [0..{}] derives ({} node(s)) but none is an accepted top \
             (finite clause / question); top={}\n",
            n - 1,
            forest.cells[0][n - 1].len(),
            top.len(),
        ));
        for &id in forest.cells[0][n - 1].values().take(SIDE_CAT_CAP) {
            out.push_str(&format!("    #{id} {}\n", node_brief(forest, id)));
        }
        return out;
    }

    let blocks = minimal_tiling(&occ, n, &fwd, &bwd);
    let forced = forced_cuts(&occ, n, &fwd, &bwd);
    out.push_str(&format!(
        "  TILING: {} blocks minimum — {}\n",
        fwd[n],
        blocks
            .iter()
            .map(|&(i, j)| format!("[{i}..{j}]«{}»", span_text(tokens, i, j)))
            .collect::<Vec<_>>()
            .join(" | "),
    ));
    out.push_str(&format!(
        "  FORCED CUTS: {}\n",
        if forced.is_empty() {
            "none (every cut is tiling-dependent; the blocks below are one minimal choice)".into()
        } else {
            forced
                .iter()
                .map(|&p| format!("{p} (before «{}»)", tokens[p].surface()))
                .collect::<Vec<_>>()
                .join(", ")
        },
    ));

    for w in blocks.windows(2) {
        let (l, r) = (w[0], w[1]);
        let cut = r.0;
        let marker = if forced.contains(&cut) {
            " [FORCED]"
        } else {
            ""
        };
        out.push_str(&format!("  ── cut {cut}{marker} ──\n"));
        report_seam(forest, tokens, layer, l, r, bin_rule_combines, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an occupancy matrix from `spans`, the derivable cells `[i..j]`.
    fn occ_of(n: usize, spans: &[(usize, usize)]) -> Occupancy {
        let mut occ = vec![vec![false; n]; n];
        for &(i, j) in spans {
            occ[i][j] = true;
        }
        occ
    }

    fn analyse(n: usize, spans: &[(usize, usize)]) -> (usize, Vec<(usize, usize)>, Vec<usize>) {
        let occ = occ_of(n, spans);
        let fwd = forward_tiling(&occ, n);
        let bwd = backward_tiling(&occ, n);
        (
            fwd[n],
            minimal_tiling(&occ, n, &fwd, &bwd),
            forced_cuts(&occ, n, &fwd, &bwd),
        )
    }

    #[test]
    fn a_full_span_tiles_in_one_block_with_no_cuts() {
        let (best, blocks, forced) = analyse(4, &[(0, 0), (1, 1), (2, 2), (3, 3), (0, 3)]);
        assert_eq!(best, 1);
        assert_eq!(blocks, vec![(0, 3)]);
        assert!(forced.is_empty(), "a sentence that derives has no cut");
    }

    #[test]
    fn the_minimal_tiling_prefers_the_longest_block() {
        // Every single token derives, and so does [0..2]. The minimum is 2 blocks; the reconstruction
        // must report [0..2] | [3..3] rather than any 2-block tiling built from shorter pieces — the
        // report is only useful if its blocks are the maximal constituents the chart actually built.
        let (best, blocks, _) = analyse(4, &[(0, 0), (1, 1), (2, 2), (3, 3), (0, 2)]);
        assert_eq!(best, 2);
        assert_eq!(blocks, vec![(0, 2), (3, 3)]);
    }

    #[test]
    fn a_cut_no_minimal_tiling_can_straddle_is_forced() {
        // `[0..1]` and `[2..3]` derive; nothing spans position 2. Minimum 2, and every minimal tiling
        // breaks at 2 — the one place the grammar could not bridge.
        let (best, blocks, forced) = analyse(4, &[(0, 0), (1, 1), (2, 2), (3, 3), (0, 1), (2, 3)]);
        assert_eq!(best, 2);
        assert_eq!(blocks, vec![(0, 1), (2, 3)]);
        assert_eq!(forced, vec![2]);
    }

    #[test]
    fn a_cut_some_minimal_tiling_straddles_is_not_forced() {
        // Two different 2-block tilings exist: [0..0]|[1..3] and [0..1]|[2..3]. Neither cut is in
        // BOTH, so neither is forced — reporting either as "the" gap would be reporting the DP's
        // arbitrary choice. This is the case the forced-cut rule exists to exclude.
        let (best, _, forced) =
            analyse(4, &[(0, 0), (1, 1), (2, 2), (3, 3), (0, 1), (1, 3), (2, 3)]);
        assert_eq!(best, 2);
        assert!(
            forced.is_empty(),
            "cut 1 is straddled by [1..3] and cut 2 by [0..1]-free tilings; got {forced:?}"
        );
    }

    #[test]
    fn several_forced_cuts_are_all_reported() {
        // Three isolated blocks: both seams are unavoidable.
        let (best, blocks, forced) = analyse(
            6,
            &[
                (0, 1),
                (2, 3),
                (4, 5),
                (0, 0),
                (1, 1),
                (2, 2),
                (3, 3),
                (4, 4),
                (5, 5),
            ],
        );
        assert_eq!(best, 3);
        assert_eq!(blocks, vec![(0, 1), (2, 3), (4, 5)]);
        assert_eq!(forced, vec![2, 4]);
    }

    #[test]
    fn a_token_with_no_cell_leaves_the_sentence_untileable() {
        // Token 2 is in no derivable cell at all. No tiling exists, which the report states as a
        // LEXICAL gap rather than inventing a seam around it.
        let occ = occ_of(4, &[(0, 0), (1, 1), (3, 3), (0, 1)]);
        let fwd = forward_tiling(&occ, 4);
        assert_eq!(fwd[4], UNREACHABLE);
    }
}
