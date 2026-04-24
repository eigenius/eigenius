//! Constraint-based solver for size inequalities (Phase 11b step 15).
//!
//! Direct port of MiniAgda's [`Warshall.hs`](../../../references/miniagda/src/Warshall.hs),
//! stripped to the subset D19 §8 requires. Solves systems of size
//! inequalities such as `x + n ≤ y` (where `x`, `y` are size variables
//! and `n : i32` is a constant offset) by constructing a weighted
//! graph, taking its transitive closure via the Floyd–Warshall
//! algorithm in the min-plus semiring, and extracting least-value
//! assignments for flexible (meta) variables.
//!
//! This module is pure — no dependencies on eval/check/term. It
//! operates on abstract node identifiers so constraint emitters
//! elsewhere in the kernel (Phase 11b step 16 onward) can plug in by
//! maintaining their own ID namespace.
//!
//! ## MiniAgda correspondence
//!
//! | MiniAgda (Haskell) | This module |
//! |---|---|
//! | `Weight = Finite Int \| Infinite` | [`Weight`] |
//! | `Rigid = RConst Weight \| RVar RigidId` | [`Rigid`] |
//! | `Node rigid = Rigid rigid \| Flex FlexId` | [`Node`] |
//! | `Constrnt Weight Rigid Scope` | [`Constraint`] |
//! | `SizeExpr = SizeVar Int Int \| SizeConst Weight` | [`SizeExpr`] |
//! | `Solution = Map Int [SizeExpr]` | [`Solution`] |
//! | `solve :: Constraints -> Maybe Solution` | [`solve`] |
//!
//! Intentionally **not** ported for now:
//! - Scope tracking for flexible variables (MiniAgda has a `scope`
//!   predicate that constrains which rigids a flex may unify with;
//!   its comment admits "SEEMS WRONG TO IGNORE THINGS NOT IN SCOPE"
//!   and the production path doesn't consult it — `inScope` is
//!   hard-coded to `True`).
//! - `Max`/`Plus` size arithmetic — D19 only needs `Succ` and `∞`.
//! - Debug trace output (MiniAgda's `traceSolve`).

use std::collections::BTreeMap;

/// Edge weight in the constraint graph. `Finite(k)` means a size
/// difference of `k` (may be negative to encode strict decrease);
/// `Infinite` is the semiring zero (absence of an edge).
///
/// `PartialOrd` / `Ord` follow the natural order: any finite value
/// is strictly less than `Infinite`, and finite values compare
/// normally. This is the same ordering MiniAgda's `Ord Weight`
/// instance uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Weight {
    Finite(i32),
    Infinite,
}

impl Weight {
    /// `inc(w, n)` = `w + n`, with `Infinite + n = Infinite`.
    pub fn inc(self, n: i32) -> Weight {
        match self {
            Weight::Infinite => Weight::Infinite,
            Weight::Finite(k) => Weight::Finite(k + n),
        }
    }

    /// Semiring `otimes` = min-plus multiplication (i.e. integer
    /// addition, with `Infinite` absorbing).
    fn otimes(self, other: Weight) -> Weight {
        match (self, other) {
            (Weight::Infinite, _) | (_, Weight::Infinite) => Weight::Infinite,
            (Weight::Finite(a), Weight::Finite(b)) => Weight::Finite(a + b),
        }
    }

    /// Semiring `oplus` = min (under our ordering where `Infinite` is
    /// the greatest element).
    fn oplus(self, other: Weight) -> Weight {
        if self.le(other) {
            self
        } else {
            other
        }
    }

    fn le(self, other: Weight) -> bool {
        match (self, other) {
            (_, Weight::Infinite) => true,
            (Weight::Infinite, Weight::Finite(_)) => false,
            (Weight::Finite(a), Weight::Finite(b)) => a <= b,
        }
    }
}

/// A rigid (non-meta) node: either a rigid size variable or a
/// concrete size constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rigid {
    /// A compile-time-fixed size, typically `Finite(0)` (base case)
    /// or `Infinite` (∞).
    Const(Weight),
    /// A rigid (universally-quantified) size variable, identified by
    /// its caller-assigned ID.
    Var(RigidId),
}

impl Rigid {
    fn is_infinite(self) -> bool {
        matches!(self, Rigid::Const(Weight::Infinite))
    }

    /// `is_below(r, w, r')` — checks whether `r + w ≤ r'` when the
    /// relation between `r` and `r'` is finite (i.e. `w` is not
    /// `Infinite`). Used during solvability checking to reject
    /// relationships between distinct rigid variables that the
    /// transitive closure has inferred.
    ///
    /// **On the finite-finite constants case**: the arithmetic here
    /// (`i + n ≤ j`) looks inconsistent with our edge-weight
    /// convention (edge weight `w` from `src` to `dst` encodes
    /// `src ≤ dst + w`, which would suggest `i ≤ j + n` here). This
    /// is a defensive path that MiniAgda's `mkConstraint`
    /// (TCM.hs:1395) never triggers in practice — the only rigid
    /// *constant* the type checker ever places in the graph is
    /// `RConst(Infinite)` (as a `≤ ∞` sink). Preserving the exact
    /// Haskell formula keeps behavioural parity; step 16+ only
    /// needs to avoid generating rigid-constant arcs with finite
    /// weights, which MiniAgda itself avoids.
    fn is_below(self, w: Weight, other: Rigid) -> bool {
        match (self, w, other) {
            (_, Weight::Infinite, _) => true,
            (_, _, Rigid::Const(Weight::Infinite)) => true,
            (
                Rigid::Const(Weight::Finite(i)),
                Weight::Finite(n),
                Rigid::Const(Weight::Finite(j)),
            ) => i + n <= j,
            // Distinct rigid variables are never related unless the
            // edge is Infinite (handled above).
            _ => false,
        }
    }
}

/// Graph node — either rigid (fixed by context) or flexible
/// (to be solved).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Node {
    Rigid(Rigid),
    Flex(FlexId),
}

pub type RigidId = u32;
pub type FlexId = u32;
type NodeId = u32;

/// One constraint in the input system. Either introduces a new
/// flexible variable (with no constraint edge yet — the solver adds
/// a self-loop of weight 0 so it's guaranteed to appear in the
/// matrix) or asserts a size inequality encoded as a weighted edge.
///
/// **Edge-weight convention** (matches MiniAgda Warshall.hs):
/// the edge `src → dst` with weight `k` encodes `src ≤ dst + k`.
/// Equivalently `src + (-k) ≤ dst`. So:
/// - `k = 0`: `src ≤ dst`
/// - `k < 0`: `src + |k| ≤ dst` (strict-decrease style: `k = -1`
///   means `src + 1 ≤ dst`, i.e. `src < dst`)
/// - `k > 0`: `src ≤ dst + k` (loose bound the other way)
#[derive(Debug, Clone, Copy)]
pub enum Constraint {
    NewFlex(FlexId),
    /// `Arc(src, k, dst)` — the edge `src ≤ dst + k` (see type docs).
    Arc(Node, i32, Node),
}

/// Convenience constructor: `arc(a, k, b)` encodes `a ≤ b + k`.
/// Use `k = -n` for `a + n ≤ b` (strict-decrease style).
pub fn arc(a: Node, k: i32, b: Node) -> Constraint {
    Constraint::Arc(a, k, b)
}

/// A solution value for a single flexible variable: either a rigid
/// variable plus an offset (`v + n`), or a fixed size constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeExpr {
    /// `Rigid::Var(id) + offset`.
    Var(RigidId, i32),
    /// A constant size (typically `Infinite`).
    Const(Weight),
}

impl SizeExpr {
    /// The size expression corresponding to `r + n`.
    fn from_rigid(r: Rigid, n: i32) -> SizeExpr {
        match r {
            Rigid::Const(w) => SizeExpr::Const(w.inc(n)),
            Rigid::Var(id) => SizeExpr::Var(id, n),
        }
    }
}

/// Solution map: each flexible variable has a list of candidate
/// size expressions (representing a least upper bound over the
/// candidates, mirroring MiniAgda's `MaxExpr = [SizeExpr]`). A flex
/// with multiple candidates is constrained to be at least the
/// maximum of them all; the solver emits each as the constraint
/// system flows through it.
pub type Solution = BTreeMap<FlexId, Vec<SizeExpr>>;

/// Solve a system of size constraints.
///
/// Returns:
/// - `Some(solution)` — every flexible variable has at least one
///   assignment; rigid variables are consistent; the system admits
///   a least solution.
/// - `None` — the constraint set is unsatisfiable (a rigid is
///   strictly below itself, two distinct rigids are forced related,
///   a flex is forced strictly below a rigid, etc.).
pub fn solve(constraints: &[Constraint]) -> Option<Solution> {
    let mut g = Graph::new();
    for c in constraints {
        g.add_constraint(*c);
    }
    let matrix = warshall(&g.edge_matrix());
    if !g.solvable(&matrix) {
        return None;
    }
    let mut solution: Solution = Solution::new();
    // Loop 1: for each rigid row, find flex columns the rigid lower-
    // bounds and record `r + n` (where n ≥ 0) as a candidate.
    for r in &g.rigids {
        let row = *g.node_map.get(&Node::Rigid(*r)).expect("rigid in graph");
        for f in &g.flexes {
            let col = *g.node_map.get(&Node::Flex(*f)).expect("flex in graph");
            if let Weight::Finite(z) = matrix[row as usize][col as usize] {
                // Edge `r + z ≤ f`. Negative z means `r + |z| ≤ f +
                // 0`; for the solution we want the offset bounded
                // below by 0 (can't subtract from a rigid).
                let offset = if z >= 0 { 0 } else { -z };
                extend_solution(&mut solution, *f, SizeExpr::from_rigid(*r, offset));
            }
        }
    }
    // Loop 2: any flex variable not yet constrained gets a rigid
    // upper bound from its matrix row, or defaults to Infinite.
    for f in &g.flexes {
        if solution.contains_key(f) {
            continue;
        }
        let row = *g.node_map.get(&Node::Flex(*f)).expect("flex in graph");
        let mut assigned = false;
        for col in 0..g.next_node {
            if let Some(Node::Rigid(r)) = g.int_map.get(&col) {
                if r.is_infinite() {
                    continue;
                }
                match matrix[row as usize][col as usize] {
                    Weight::Finite(z) if z >= 0 => {
                        extend_solution(&mut solution, *f, SizeExpr::from_rigid(*r, z));
                        assigned = true;
                        break;
                    }
                    Weight::Finite(_) => {
                        // Negative weight on row means `f + |z| ≤ r`,
                        // i.e. rigid bounds flex from above but is
                        // strictly smaller — cannot satisfy.
                        return None;
                    }
                    Weight::Infinite => {}
                }
            }
        }
        if !assigned {
            extend_solution(&mut solution, *f, SizeExpr::Const(Weight::Infinite));
        }
    }
    Some(solution)
}

fn extend_solution(sol: &mut Solution, f: FlexId, e: SizeExpr) {
    sol.entry(f).or_default().push(e);
}

/// Graph under construction from a constraint list.
struct Graph {
    node_map: BTreeMap<Node, NodeId>,
    int_map: BTreeMap<NodeId, Node>,
    next_node: NodeId,
    /// Adjacency storage: `edges[&(src, dst)]` is the weight of the
    /// edge from `src` to `dst`. Absent pairs carry `Infinite`.
    edges: BTreeMap<(NodeId, NodeId), Weight>,
    flexes: Vec<FlexId>,
    rigids: Vec<Rigid>,
}

impl Graph {
    fn new() -> Self {
        Self {
            node_map: BTreeMap::new(),
            int_map: BTreeMap::new(),
            next_node: 0,
            edges: BTreeMap::new(),
            flexes: Vec::new(),
            rigids: Vec::new(),
        }
    }

    fn add_node(&mut self, n: Node) -> NodeId {
        if let Some(&id) = self.node_map.get(&n) {
            return id;
        }
        let id = self.next_node;
        self.node_map.insert(n, id);
        self.int_map.insert(id, n);
        self.next_node += 1;
        match n {
            Node::Rigid(r) => {
                if !self.rigids.contains(&r) {
                    self.rigids.push(r);
                }
            }
            Node::Flex(f) => {
                if !self.flexes.contains(&f) {
                    self.flexes.push(f);
                }
            }
        }
        id
    }

    fn add_edge(&mut self, a: Node, w: Weight, b: Node) {
        let i = self.add_node(a);
        let j = self.add_node(b);
        let existing = self.edges.get(&(i, j)).copied().unwrap_or(Weight::Infinite);
        self.edges.insert((i, j), w.oplus(existing));
    }

    fn add_constraint(&mut self, c: Constraint) {
        match c {
            Constraint::NewFlex(f) => {
                // Self-loop of weight 0 guarantees the flex appears
                // in the matrix even without constraints touching it.
                self.add_edge(Node::Flex(f), Weight::Finite(0), Node::Flex(f));
            }
            Constraint::Arc(a, k, b) => {
                self.add_edge(a, Weight::Finite(k), b);
            }
        }
    }

    /// Dense adjacency matrix indexed by `NodeId`.
    fn edge_matrix(&self) -> Vec<Vec<Weight>> {
        let n = self.next_node as usize;
        let mut m = vec![vec![Weight::Infinite; n]; n];
        for (&(i, j), &w) in &self.edges {
            m[i as usize][j as usize] = w;
        }
        m
    }

    /// Check whether the transitive closure admits a solution:
    /// 1. No negative self-loop on any rigid (a rigid cannot be
    ///    strictly below itself).
    /// 2. No two distinct rigids are forced into a finite
    ///    relationship unless `r + k ≤ r'` actually holds for their
    ///    concrete values.
    /// 3. No flex is forced strictly below a rigid-var (because flex
    ///    values are bounded below by 0 from rigids).
    fn solvable(&self, m: &[Vec<Weight>]) -> bool {
        // Diagonal on rigids must be ≥ 0.
        for r in &self.rigids {
            let i = self.node_map[&Node::Rigid(*r)] as usize;
            if !Weight::Finite(0).le(m[i][i]) {
                return false;
            }
        }
        // Rigid-rigid relationships must be semantically compatible.
        for r1 in &self.rigids {
            for r2 in &self.rigids {
                if r1 == r2 {
                    continue;
                }
                let i = self.node_map[&Node::Rigid(*r1)] as usize;
                let j = self.node_map[&Node::Rigid(*r2)] as usize;
                if !r1.is_below(m[i][j], *r2) {
                    return false;
                }
            }
        }
        // Flex → RigidVar edges cannot be strictly negative.
        for f in &self.flexes {
            for r in &self.rigids {
                if let Rigid::Var(_) = *r {
                    let i = self.node_map[&Node::Flex(*f)] as usize;
                    let j = self.node_map[&Node::Rigid(*r)] as usize;
                    if !Weight::Finite(0).le(m[i][j]) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Partial-order comparison on size values: `size_le(s1, s2)` holds
/// iff `s1 ≤ s2` under the size ordering (D19 §8.3).
///
/// The order is generated by three rules:
/// 1. `s ≤ ∞` for every size `s` (infinity is top).
/// 2. `s ≤ ŝ(s')` whenever `s ≤ s'` (successor step).
/// 3. `s ≤ s` — reflexivity on anything the previous rules don't
///    cover, including neutral sizes (size variables / metas).
///
/// Structural `SizeSucc` comparisons unfold on both sides:
/// `ŝ(a) ≤ ŝ(b) ⇐ a ≤ b`. ∞-absorption ensures `ŝ(∞)` never appears
/// as an input — `eval.rs` collapses it — so we don't special-case it.
///
/// Neutral reflexivity is checked by readback equality at level 0.
/// This is conservative — two distinct neutrals are never considered
/// related. Actual entailment between size hypotheses (e.g.
/// `i ≤ ŝ(j)` given `i ≤ j`) is the job of the rigid-hypothesis
/// solver ([`crate::nbe::sized_rigid`]), not this relation.
pub fn size_le(s1: &crate::nbe::val::Val, s2: &crate::nbe::val::Val) -> bool {
    use crate::nbe::val::Val;
    // Rule 1: anything ≤ ∞.
    if matches!(s2, Val::SizeInf) {
        return true;
    }
    // ∞ ≤ non-∞ cannot hold (∞ is top; the previous line already
    // handled ∞ ≤ ∞).
    if matches!(s1, Val::SizeInf) {
        return false;
    }
    // Structural: ŝ(a) ≤ ŝ(b) ⇐ a ≤ b.
    if let (Val::SizeSucc(a), Val::SizeSucc(b)) = (s1, s2) {
        return size_le(a, b);
    }
    // Rule 2 (right-step): s ≤ ŝ(b) ⇐ s ≤ b.
    if let Val::SizeSucc(b) = s2 {
        if size_le(s1, b) {
            return true;
        }
    }
    // Rule 3: reflexivity on the leftover (neutrals, etc.) via readback.
    let e1 = crate::nbe::readback::readback_val(0, s1);
    let e2 = crate::nbe::readback::readback_val(0, s2);
    e1 == e2
}

/// Floyd–Warshall transitive closure over the min-plus semiring.
///
/// `m[i][j]` is the min-weight path from `i` to `j`. After running,
/// `m[i][j]` is the minimum weight over all paths from `i` to `j`.
fn warshall(m0: &[Vec<Weight>]) -> Vec<Vec<Weight>> {
    let n = m0.len();
    let mut m: Vec<Vec<Weight>> = m0.to_vec();
    for k in 0..n {
        for i in 0..n {
            for j in 0..n {
                let candidate = m[i][k].otimes(m[k][j]);
                m[i][j] = m[i][j].oplus(candidate);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rv(id: RigidId) -> Node {
        Node::Rigid(Rigid::Var(id))
    }
    fn rc(w: Weight) -> Node {
        Node::Rigid(Rigid::Const(w))
    }
    fn f(id: FlexId) -> Node {
        Node::Flex(id)
    }

    #[test]
    fn empty_system_has_empty_solution() {
        let sol = solve(&[]).expect("empty is trivially satisfiable");
        assert!(sol.is_empty());
    }

    #[test]
    fn lone_flex_defaults_to_infinity() {
        // Only constraint: flex 0 exists. No bounds → solution is ∞.
        let sol = solve(&[Constraint::NewFlex(0)]).expect("solvable");
        let candidates = sol.get(&0).expect("flex has a solution");
        assert_eq!(candidates.len(), 1);
        assert!(matches!(candidates[0], SizeExpr::Const(Weight::Infinite)));
    }

    #[test]
    fn strict_self_loop_on_rigid_is_unsatisfiable() {
        // r + 1 ≤ r → encoded as arc(r, -1, r) (see edge convention
        // doc: weight -1 means `r + 1 ≤ r`, which is impossible).
        let cs = &[arc(rv(0), -1, rv(0))];
        assert!(solve(cs).is_none());
    }

    #[test]
    fn rigid_le_rigid_passes_when_edge_is_zero() {
        // r ≤ r — edge of weight 0. This is a self-reflexive
        // constraint, harmless.
        let cs = &[arc(rv(0), 0, rv(0))];
        assert!(solve(cs).is_some());
    }

    #[test]
    fn two_rigids_cannot_be_related_by_finite_edge() {
        // r0 ≤ r1 — forced finite relationship between distinct
        // rigid variables. Unsatisfiable in MiniAgda's rule.
        let cs = &[arc(rv(0), 0, rv(1))];
        assert!(solve(cs).is_none());
    }

    #[test]
    fn flex_below_rigid_by_strict_amount_is_unsatisfiable() {
        // flex 0 + 1 ≤ rigid var 0 ⇒ flex < rigid (strict); encoded
        // as arc(f, -1, v). MiniAgda rejects this: a flex may equal
        // a rigid but cannot be strictly below one.
        let cs = &[arc(f(0), -1, rv(0))];
        assert!(solve(cs).is_none());
    }

    #[test]
    fn rigid_lower_bounds_flex() {
        // rigid var 0 ≤ flex 0 ⇒ flex = rigid var 0.
        let cs = &[arc(rv(0), 0, f(0))];
        let sol = solve(cs).expect("solvable");
        let candidates = sol.get(&0).expect("flex 0 solved");
        assert!(candidates.iter().any(|e| matches!(e, SizeExpr::Var(0, 0))));
    }

    #[test]
    fn rigid_plus_k_lower_bounds_flex() {
        // r + 3 ≤ f ⇒ encoded as arc(r, -3, f) (weight -3 means
        // `r + 3 ≤ f`). Loop1 picks this up and truncates negative
        // z to offset +|z|, producing the solution `f = r + 3`.
        let cs = &[arc(rv(0), -3, f(0))];
        let sol = solve(cs).expect("solvable");
        let candidates = sol.get(&0).expect("flex 0 solved");
        assert!(candidates.iter().any(|e| matches!(e, SizeExpr::Var(0, 3))));
    }

    #[test]
    fn flex_upper_bounded_by_rigid_resolves_to_rigid_in_loop2() {
        // f ≤ r with no lower bound: loop1 leaves f unsolved,
        // loop2 picks up the upper bound and assigns f = r.
        let cs = &[arc(f(0), 0, rv(0))];
        let sol = solve(cs).expect("solvable");
        let candidates = sol.get(&0).expect("flex 0 solved");
        assert!(candidates
            .iter()
            .any(|e| matches!(e, SizeExpr::Var(0, 0) | SizeExpr::Const(Weight::Infinite))));
    }

    #[test]
    fn multiple_rigid_lower_bounds_accumulate() {
        // Two distinct rigids both ≤ flex → flex has two candidates.
        // Note: two rigids can't be compared to each other, so we
        // need them to stay decoupled — separate non-paths in the
        // graph.
        let cs = &[arc(rv(0), 0, f(0)), arc(rv(1), 0, f(0))];
        // This actually makes both rigids transitively reachable
        // FROM flex too? No — our arcs only add forward edges.
        // The check passes because there's no direct rigid-to-rigid
        // edge.
        let sol = solve(cs).expect("solvable");
        let candidates = sol.get(&0).expect("flex 0 solved");
        // Should have at least two candidates, one per rigid.
        assert!(candidates.len() >= 2);
    }

    #[test]
    fn constant_infinity_edge_is_harmless() {
        // rigid const Infinite ≤ flex — flex is ≤ ∞, harmless.
        let cs = &[arc(rc(Weight::Infinite), 0, f(0))];
        let sol = solve(cs).expect("solvable");
        assert!(sol.contains_key(&0));
    }

    #[test]
    fn warshall_computes_transitive_closure() {
        // 0 --1--> 1 --1--> 2 should give 0 --2--> 2 in the closure.
        let mut m = vec![vec![Weight::Infinite; 3]; 3];
        m[0][1] = Weight::Finite(1);
        m[1][2] = Weight::Finite(1);
        let closed = warshall(&m);
        assert_eq!(closed[0][2], Weight::Finite(2));
    }

    #[test]
    fn transitive_composition_accumulates_weights() {
        // meta_1 + 2 ≤ meta_2 (arc m1 → m2 weight -2)
        // meta_2 + 3 ≤ meta_3 (arc m2 → m3 weight -3)
        // ⇒ transitively meta_1 + 5 ≤ meta_3 (closed weight -5 on m1 → m3).
        // Check by querying the closure indirectly: stick a rigid at
        // each end and observe the solution shape for a flex in the
        // middle.
        //
        // Concretely we drive m1 → m2 → m3 through three flex nodes,
        // with a rigid lower-bounding m1 by 5 — after closure, the
        // rigid should transitively lower-bound m3 by 10.
        let cs = &[
            // rigid v0 + 5 ≤ f0
            arc(rv(0), -5, f(0)),
            // f0 + 3 ≤ f1
            arc(f(0), -3, f(1)),
            // f1 + 2 ≤ f2
            arc(f(1), -2, f(2)),
        ];
        let sol = solve(cs).expect("satisfiable");
        // f2 should be solved as v0 + 10.
        let c = sol.get(&2).expect("f2 solved");
        assert!(
            c.iter().any(|e| matches!(e, SizeExpr::Var(0, 10))),
            "expected f2 = v0 + 10, got {c:?}"
        );
    }

    #[test]
    fn transitive_closure_detects_indirect_negative_cycle() {
        // Two rigid vars coupled through a flex forming a cycle with
        // strict decrease. Direct edges are individually OK but the
        // transitive closure places a negative self-loop on the flex,
        // rejecting the system.
        //
        // v0 + 1 ≤ f0  (arc v0 → f0, weight -1)
        // f0 + 1 ≤ v0  (arc f0 → v0, weight -1)
        //
        // Transitively: f0 + 2 ≤ f0 → negative self-loop of -2 on f0.
        // But wait — the second edge says flex strictly below rigid,
        // which should be rejected outright.
        let cs = &[arc(rv(0), -1, f(0)), arc(f(0), -1, rv(0))];
        assert!(solve(cs).is_none());
    }

    #[test]
    fn rigid_chain_through_flex_is_rejected_as_distinct_rigid_coupling() {
        // rigid v0 ≤ f0, f0 ≤ rigid v1. Transitively the closure
        // places a finite edge v0 → v1 weight 0, coupling two
        // distinct rigid variables — must be rejected.
        let cs = &[arc(rv(0), 0, f(0)), arc(f(0), 0, rv(1))];
        assert!(solve(cs).is_none());
    }

    #[test]
    fn multiple_lower_bounds_produce_multiple_candidates() {
        // Three distinct rigids each lower-bounding flex 0 by
        // different amounts. The solver should record one candidate
        // per rigid in the MaxExpr-style Vec<SizeExpr>.
        let cs = &[
            arc(rv(0), 0, f(0)),  // v0 ≤ f0   ⇒ f0 ≥ v0
            arc(rv(1), -2, f(0)), // v1 + 2 ≤ f0 ⇒ f0 ≥ v1 + 2
            arc(rv(2), -5, f(0)), // v2 + 5 ≤ f0 ⇒ f0 ≥ v2 + 5
        ];
        let sol = solve(cs).expect("satisfiable");
        let c = sol.get(&0).expect("f0 solved");
        assert!(c.iter().any(|e| matches!(e, SizeExpr::Var(0, 0))));
        assert!(c.iter().any(|e| matches!(e, SizeExpr::Var(1, 2))));
        assert!(c.iter().any(|e| matches!(e, SizeExpr::Var(2, 5))));
    }

    #[test]
    fn flex_with_both_lower_and_upper_bounds_resolves() {
        // v0 ≤ f0 (lower bound)
        // f0 ≤ v0 (upper bound by the same rigid; equivalent to f0 = v0)
        let cs = &[arc(rv(0), 0, f(0)), arc(f(0), 0, rv(0))];
        let sol = solve(cs).expect("satisfiable");
        let c = sol.get(&0).expect("f0 solved");
        // Loop1 catches the lower bound ⇒ candidate v0 + 0.
        assert!(c.iter().any(|e| matches!(e, SizeExpr::Var(0, 0))));
    }

    #[test]
    fn miniagda_bad_size_lambda_style_constraint_rejected() {
        // From `references/miniagda/test/fail/BadSizeLambda.err`:
        //   "new j < v0"
        //   "adding size rel. v1 + 1 <= v0"
        //   "cannot add hypothesis v1 + 1 <= v0 because it is not
        //    satisfiable under all possible valuations of the
        //    current hypotheses"
        //
        // In MiniAgda's output this is a rigid-rigid relationship
        // rejected by the hypothesis-tracking side (TSO, not
        // Warshall). Our Warshall port rejects the same shape via
        // the "distinct rigids with finite edge" rule.
        let cs = &[arc(rv(1), -1, rv(0))];
        assert!(solve(cs).is_none());
    }

    #[test]
    fn miniagda_invalid_size_p_style_constraints() {
        // From `references/miniagda/test/fail/InvalidSizeP.err`:
        //   "adding size rel. v3 + 1 <= v0"
        //   "adding size rel. v3 + 1 <= v1"
        //
        // Taken individually either constraint is a distinct-rigid
        // coupling (rejected). Taken both together (which MiniAgda
        // tracks in TSO) the satisfiability verdict is "OK for each
        // hypothesis, but the subsequent subtyping query v0 ≤ v1 is
        // not entailed." Our Warshall port (meta-focused) rejects
        // the moment any distinct-rigid finite edge enters.
        //
        // Documenting the behaviour divergence: Warshall alone is
        // meta-resolution only; rigid-rigid hypothesis tracking is
        // MiniAgda's separate TSO mechanism, which we'll need to
        // port when implementing subtyping-query entailment.
        let cs = &[arc(rv(3), -1, rv(0))];
        assert!(solve(cs).is_none());
        let cs2 = &[arc(rv(3), -1, rv(1))];
        assert!(solve(cs2).is_none());
    }

    // --- size_le partial order tests (Phase 11b step 15d) ---
    //
    // Cross-checks against the size ordering described in D19 §8.3
    // and the per-rule semantics in the doc comment on `size_le`.

    use crate::nbe::val::{Neut, Val};

    fn succ_val(v: Val) -> Val {
        Val::SizeSucc(Box::new(v))
    }

    fn size_neut(id: usize) -> Val {
        Val::Nt(Neut::Gen(id, format!("s{id}")))
    }

    #[test]
    fn size_inf_is_top() {
        // Anything ≤ ∞.
        assert!(size_le(&Val::SizeInf, &Val::SizeInf));
        assert!(size_le(&succ_val(Val::SizeInf), &Val::SizeInf));
        assert!(size_le(&size_neut(0), &Val::SizeInf));
        assert!(size_le(&succ_val(size_neut(0)), &Val::SizeInf));
    }

    #[test]
    fn size_inf_not_below_finite() {
        // ∞ ≤ ŝ(s) must be false (infinity is the top, not succ of
        // anything except itself — and ∞-absorption collapses ŝ(∞)
        // at eval time, so ŝ(...) is never ∞ here).
        assert!(!size_le(&Val::SizeInf, &succ_val(size_neut(0))));
        assert!(!size_le(&Val::SizeInf, &size_neut(0)));
    }

    #[test]
    fn size_succ_is_reflexive_structurally() {
        // ŝ(s) ≤ ŝ(s) by the structural rule on top of reflexivity.
        let s = size_neut(0);
        assert!(size_le(&succ_val(s.clone()), &succ_val(s)));
    }

    #[test]
    fn size_succ_step_rule() {
        // s ≤ ŝ(s) admitted by the right-step rule.
        let s = size_neut(0);
        assert!(size_le(&s, &succ_val(s.clone())));
        // s ≤ ŝ(ŝ(s)) admitted (two applications of right-step).
        assert!(size_le(&s, &succ_val(succ_val(s.clone()))));
    }

    #[test]
    fn size_succ_step_rule_not_reversible() {
        // ŝ(s) ≤ s must not hold — the step only goes one direction.
        let s = size_neut(0);
        assert!(!size_le(&succ_val(s.clone()), &s));
    }

    #[test]
    fn distinct_neutrals_are_incomparable() {
        // No entailment between unrelated size variables here —
        // that's the job of the rigid-hypothesis solver.
        assert!(!size_le(&size_neut(0), &size_neut(1)));
        assert!(!size_le(&size_neut(1), &size_neut(0)));
    }

    #[test]
    fn neutral_reflexive() {
        // Same neutral compares equal.
        let s = size_neut(0);
        assert!(size_le(&s, &s));
    }

    #[test]
    fn succ_vs_succ_recurses() {
        // ŝ(a) ≤ ŝ(b) iff a ≤ b.
        let a = size_neut(0);
        let b = size_neut(1);
        // distinct neutrals: neither holds at the inner level so
        // neither holds outer.
        assert!(!size_le(&succ_val(a.clone()), &succ_val(b.clone())));
        // a ≤ ∞ at inner level ⇒ ŝ(a) ≤ ŝ(∞) — but ∞-absorption means
        // ŝ(∞) is never constructed; skip this case.
        // a ≤ ŝ(a) at inner level ⇒ ŝ(a) ≤ ŝ(ŝ(a)).
        assert!(size_le(&succ_val(a.clone()), &succ_val(succ_val(a))));
    }

    #[test]
    fn weight_ordering_and_arithmetic() {
        // Min with infinity
        assert_eq!(Weight::Finite(3).oplus(Weight::Infinite), Weight::Finite(3));
        // Sum with infinity
        assert_eq!(Weight::Finite(3).otimes(Weight::Infinite), Weight::Infinite);
        // Sum of finites
        assert_eq!(
            Weight::Finite(2).otimes(Weight::Finite(3)),
            Weight::Finite(5)
        );
        // inc
        assert_eq!(Weight::Finite(5).inc(3), Weight::Finite(8));
        assert_eq!(Weight::Infinite.inc(100), Weight::Infinite);
    }
}
