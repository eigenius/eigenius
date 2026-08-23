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

//! Universe levels for EigenTT sorts (eigenius#188, design note
//! [`docs/notes/p2-n3-universe-polymorphism.md`](../../../docs/notes/p2-n3-universe-polymorphism.md)).
//!
//! A level is `Zero`, `Succ`, `Max`, `IMax`, or a `Param` — the five constructors Lean uses, and
//! the same five the chain already carries for the Lean mirror as `urn:eigenius:lean:LeanLevel`
//! (`ontologies/lean/lean-expressions.eigon.json`). Mirroring that shape is deliberate: with an
//! isomorphic algebra, translating an EigenTT sort into a Lean one (eigenius#159 / D74) is a fold
//! rather than a special case that only covers the monomorphic fragment.
//!
//! Ported from `references/nanoda_lib/src/level.rs` @ `6ae1f0c` — `simplify` (`:55`),
//! `combining` (`:43`), `subst_level` (`:108`), `leq_core` (`:176`), `leq` (`:229`),
//! `leq_imax_by_cases` (`:160`). The `IMax` cases in `leq_core` are the part not worth
//! re-deriving: `imax l r` is `0` when `r` is `0` and `max l r` otherwise, so an `IMax` whose
//! right side is a parameter cannot be decided without splitting on whether that parameter is
//! zero — which is what `leq_imax_by_cases` does, and why this is not simply integer comparison.
//!
//! ## Why `IMax` at all
//!
//! `Pi (a : A) (b : B)` lives at `imax (level A) (level B)`: impredicative when `B : Prop` (the
//! whole Pi is a `Prop` regardless of `A`), predicative otherwise. With concrete levels that is a
//! two-case branch, which is what `check_infer`'s `infer_dependent_sort` does today. With a level
//! *variable* in `B`, the answer is not known until the variable is instantiated, and `IMax` is
//! the term that defers it.
//!
//! ## What is NOT here
//!
//! Hash-consing. nanoda interns levels in an arena and compares pointers; this module owns its
//! `Box`es and compares structurally. Levels in practice are tiny — `Succ(Succ(Zero))` is the
//! deepest thing in the tree today — so the arena would buy nothing but complexity. `leq` is
//! nonetheless memo-free and exponential in nested `IMax`-over-`Max`; if that ever shows up in a
//! profile, cache on `(l, r, diff)`.

use std::fmt;

/// A universe level.
///
/// `Sort(Zero)` is `Prop`, `Sort(Succ(Zero))` is `Set`, and `Sort(Succ^{k+1}(Zero))` is the
/// surface's `Type k`. [`Level::of_nat`] and [`Level::as_nat`] convert to and from that numeral
/// form, which is what every monomorphic site uses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// `Prop`.
    Zero,
    /// One universe above its argument.
    Succ(Box<Level>),
    /// The larger of two levels.
    Max(Box<Level>, Box<Level>),
    /// `imax l r` — `Zero` when `r` is `Zero`, `max l r` otherwise. The impredicative-`Prop` rule
    /// for `Pi`, held open until `r` is known.
    IMax(Box<Level>, Box<Level>),
    /// A level variable, bound by a declaration's `uparams`.
    Param(String),
}

impl Level {
    /// The `n`-th numeral level: `of_nat(0) == Zero`, `of_nat(1) == Succ(Zero)`, …
    ///
    /// This is the bridge every monomorphic site uses, and the reason the change from
    /// `Exp::Sort(usize)` is mechanical rather than a rewrite.
    pub fn of_nat(n: usize) -> Level {
        (0..n).fold(Level::Zero, |acc, _| Level::Succ(Box::new(acc)))
    }

    /// The numeral value of a closed `Succ`-chain over `Zero`, or `None` if the level mentions a
    /// `Param`, `Max` or `IMax`.
    ///
    /// Callers that only ever see concrete levels use this to keep reading as arithmetic. A
    /// `None` means the level is genuinely polymorphic and the caller must go through [`Level::leq`].
    pub fn as_nat(&self) -> Option<usize> {
        let mut n = 0usize;
        let mut cur = self;
        loop {
            match cur {
                Level::Zero => return Some(n),
                Level::Succ(inner) => {
                    n = n.checked_add(1)?;
                    cur = inner;
                }
                _ => return None,
            }
        }
    }

    /// `Prop`.
    pub fn zero() -> Level {
        Level::Zero
    }

    /// One above `self`.
    pub fn succ(self) -> Level {
        Level::Succ(Box::new(self))
    }

    /// Every `Param` name occurring in the level, in first-occurrence order.
    ///
    /// Used to generalise a declaration's free levels into its `uparams`.
    pub fn params(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_params(&mut out);
        out
    }

    fn collect_params(&self, out: &mut Vec<String>) {
        match self {
            Level::Zero => {}
            Level::Succ(a) => a.collect_params(out),
            Level::Max(a, b) | Level::IMax(a, b) => {
                a.collect_params(out);
                b.collect_params(out);
            }
            Level::Param(n) => {
                if !out.iter().any(|e| e == n) {
                    out.push(n.clone());
                }
            }
        }
    }

    /// Whether the level is closed — no `Param` anywhere.
    pub fn is_closed(&self) -> bool {
        self.params().is_empty()
    }

    /// `self[ks |-> vs]` — simultaneous substitution of level parameters.
    ///
    /// Port of nanoda's `subst_level` (`level.rs:108`). A `Param` not named in `ks` is left alone.
    pub fn subst(&self, ks: &[String], vs: &[Level]) -> Level {
        match self {
            Level::Zero => Level::Zero,
            Level::Succ(a) => Level::Succ(Box::new(a.subst(ks, vs))),
            Level::Max(a, b) => Level::Max(Box::new(a.subst(ks, vs)), Box::new(b.subst(ks, vs))),
            Level::IMax(a, b) => Level::IMax(Box::new(a.subst(ks, vs)), Box::new(b.subst(ks, vs))),
            Level::Param(n) => ks
                .iter()
                .position(|k| k == n)
                .and_then(|i| vs.get(i).cloned())
                .unwrap_or_else(|| self.clone()),
        }
    }

    /// Normalise. Port of nanoda's `simplify` (`level.rs:55`).
    ///
    /// The `IMax` arm carries the two identities that are easy to get wrong: `imax 0 r == r`
    /// (because `imax 0 0 == 0 == r` and `imax 0 r == max 0 r == r` for `r > 0`), and
    /// `imax 1 r == r` (`r == 0` gives `0`, `r > 0` gives `max 1 r == r`). Both are nanoda's
    /// `is_zero(l) || is_one(l)` test on the LEFT argument, which reads backwards until worked
    /// through.
    pub fn simplify(&self) -> Level {
        match self {
            Level::Zero | Level::Param(_) => self.clone(),
            Level::Succ(a) => Level::Succ(Box::new(a.simplify())),
            Level::Max(a, b) => combining(&a.simplify(), &b.simplify()),
            Level::IMax(a, b) => {
                let l = a.simplify();
                let r = b.simplify();
                // `imax 0 r == r` and `imax 1 r == r`.
                if matches!(l.as_nat(), Some(0) | Some(1)) {
                    return r;
                }
                match &r {
                    // `imax l 0 == 0`.
                    Level::Zero => r,
                    // `r` is known non-zero, so `imax l r == max l r`.
                    Level::Succ(_) => combining(&l, &r),
                    _ => Level::IMax(Box::new(l), Box::new(r)),
                }
            }
        }
    }

    /// `self <= other`, deciding the parameter cases by splitting. Port of nanoda's `leq`
    /// (`level.rs:229`): simplify both sides, then compare with an offset of zero.
    ///
    /// This replaces the integer `m <= n` cumulativity test that `conv.rs` used while `Exp::Sort`
    /// carried a `usize`. On closed levels it agrees with it exactly — see
    /// `agrees_with_integer_comparison_on_numerals`.
    pub fn leq(&self, other: &Level) -> bool {
        leq_core(&self.simplify(), &other.simplify(), 0)
    }

    /// Antisymmetric equality — `l <= r` and `r <= l`. Two levels may be equal without being
    /// structurally identical (`max 0 l` and `l`, say).
    pub fn eq_antisymm(&self, other: &Level) -> bool {
        self.leq(other) && other.leq(self)
    }
}

/// `max` with the normalising identities. Port of nanoda's `combining` (`level.rs:43`).
fn combining(l: &Level, r: &Level) -> Level {
    match (l, r) {
        (Level::Zero, _) => r.clone(),
        (_, Level::Zero) => l.clone(),
        (Level::Succ(a), Level::Succ(b)) => Level::Succ(Box::new(combining(a, b))),
        _ => Level::Max(Box::new(l.clone()), Box::new(r.clone())),
    }
}

fn is_param(l: &Level) -> bool {
    matches!(l, Level::Param(_))
}

fn is_any_max(l: &Level) -> bool {
    matches!(l, Level::Max(..) | Level::IMax(..))
}

/// `lhs <= rhs` holds regardless of whether `param` is zero. Port of nanoda's
/// `leq_imax_by_cases` (`level.rs:160`).
///
/// This is the case-split that makes `IMax` decidable: substitute `param := 0` and
/// `param := succ(param)` into both sides and require the inequality on both branches.
fn leq_imax_by_cases(param: &str, lhs: &Level, rhs: &Level, diff: isize) -> bool {
    let ks = [param.to_string()];
    let zero = [Level::Zero];
    let succ_p = [Level::Succ(Box::new(Level::Param(param.to_string())))];

    let l0 = lhs.subst(&ks, &zero).simplify();
    let r0 = rhs.subst(&ks, &zero).simplify();
    let ls = lhs.subst(&ks, &succ_p).simplify();
    let rs = rhs.subst(&ks, &succ_p).simplify();

    leq_core(&l0, &r0, diff) && leq_core(&ls, &rs, diff)
}

/// Port of nanoda's `leq_core` (`level.rs:176`).
///
/// `diff` counts how many `Succ`s have been peeled from the right minus the left — the more
/// positive, the more room the right side has. Both sides must already be simplified.
fn leq_core(l: &Level, r: &Level, diff: isize) -> bool {
    match (l, r) {
        (Level::Zero, _) if diff >= 0 => true,
        (_, Level::Zero) if diff < 0 => false,
        (Level::Param(a), Level::Param(x)) => a == x && diff >= 0,
        (Level::Param(_), Level::Zero) => false,
        (Level::Zero, Level::Param(_)) => diff >= 0,
        (Level::Succ(s), _) => leq_core(s, r, diff - 1),
        (_, Level::Succ(s)) => leq_core(l, s, diff + 1),
        (Level::Max(a, b), _) => leq_core(a, r, diff) && leq_core(b, r, diff),
        (Level::Param(_) | Level::Zero, Level::Max(x, y)) => {
            leq_core(l, x, diff) || leq_core(l, y, diff)
        }
        (Level::IMax(a, b), Level::IMax(x, y)) if a == x && b == y && diff >= 0 => true,
        (Level::IMax(_, b), _) if is_param(b) => {
            let Level::Param(p) = b.as_ref() else {
                unreachable!("guarded by is_param")
            };
            leq_imax_by_cases(p, l, r, diff)
        }
        (_, Level::IMax(_, y)) if is_param(y) => {
            let Level::Param(p) = y.as_ref() else {
                unreachable!("guarded by is_param")
            };
            leq_imax_by_cases(p, l, r, diff)
        }
        (Level::IMax(a, b), _) if is_any_max(b) => match b.as_ref() {
            Level::IMax(x, y) => {
                let new_lhs = Level::IMax(a.clone(), y.clone());
                let new_rhs = Level::IMax(x.clone(), y.clone());
                leq_core(&Level::Max(Box::new(new_lhs), Box::new(new_rhs)), r, diff)
            }
            Level::Max(x, y) => {
                let new_lhs = Level::IMax(a.clone(), x.clone());
                let new_rhs = Level::IMax(a.clone(), y.clone());
                let m = Level::Max(Box::new(new_lhs), Box::new(new_rhs)).simplify();
                leq_core(&m, r, diff)
            }
            _ => unreachable!("guarded by is_any_max"),
        },
        (_, Level::IMax(x, y)) if is_any_max(y) => match y.as_ref() {
            Level::IMax(j, k) => {
                let new_lhs = Level::IMax(x.clone(), k.clone());
                let new_rhs = Level::IMax(j.clone(), k.clone());
                leq_core(l, &Level::Max(Box::new(new_lhs), Box::new(new_rhs)), diff)
            }
            Level::Max(j, k) => {
                let new_lhs = Level::IMax(x.clone(), j.clone());
                let new_rhs = Level::IMax(x.clone(), k.clone());
                let m = Level::Max(Box::new(new_lhs), Box::new(new_rhs)).simplify();
                leq_core(l, &m, diff)
            }
            _ => unreachable!("guarded by is_any_max"),
        },
        // nanoda `panic!()`s here. Every shape is covered by the arms above once both sides are
        // simplified; returning `false` rather than panicking keeps an unexpected shape from
        // taking down the commit gate, and the arms are exhaustive by construction.
        _ => false,
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(n) = self.as_nat() {
            return write!(f, "{n}");
        }
        match self {
            Level::Zero => write!(f, "0"),
            Level::Succ(a) => write!(f, "({a}+1)"),
            Level::Max(a, b) => write!(f, "max({a}, {b})"),
            Level::IMax(a, b) => write!(f, "imax({a}, {b})"),
            Level::Param(n) => write!(f, "{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: &str) -> Level {
        Level::Param(n.to_string())
    }

    #[test]
    fn numerals_round_trip() {
        for n in 0..6 {
            assert_eq!(Level::of_nat(n).as_nat(), Some(n));
        }
        assert_eq!(p("u").as_nat(), None);
        assert_eq!(
            Level::Max(Box::new(Level::Zero), Box::new(p("u"))).as_nat(),
            None
        );
    }

    /// **The compatibility property the whole migration rests on.** Every site that read
    /// `Exp::Sort(usize)` and compared with `<=` is replaced by [`Level::leq`]; on closed numeral
    /// levels the two must agree exactly, or the change is not behaviour-preserving for the 942
    /// monomorphic sort uses in the tree.
    #[test]
    fn agrees_with_integer_comparison_on_numerals() {
        for m in 0..6usize {
            for n in 0..6usize {
                assert_eq!(
                    Level::of_nat(m).leq(&Level::of_nat(n)),
                    m <= n,
                    "leq({m}, {n}) must match integer <="
                );
            }
        }
    }

    #[test]
    fn a_param_is_below_its_own_successor_and_not_above() {
        assert!(p("u").leq(&p("u").succ()));
        assert!(!p("u").succ().leq(&p("u")));
        assert!(p("u").leq(&p("u")));
    }

    #[test]
    fn distinct_params_are_incomparable() {
        assert!(!p("u").leq(&p("v")));
        assert!(!p("v").leq(&p("u")));
    }

    #[test]
    fn zero_is_bottom_but_a_param_is_not_above_zero_structurally() {
        assert!(Level::Zero.leq(&p("u")));
        // A parameter may itself be zero, so it is not strictly above zero.
        assert!(!p("u").succ().leq(&Level::of_nat(1)));
        assert!(!p("u").leq(&Level::Zero));
    }

    /// `max` normalises away `Zero` on either side, and pushes through matching `Succ`s.
    #[test]
    fn combining_normalises() {
        assert_eq!(combining(&Level::Zero, &p("u")), p("u"));
        assert_eq!(combining(&p("u"), &Level::Zero), p("u"));
        assert_eq!(
            combining(&Level::of_nat(2), &Level::of_nat(3)),
            Level::of_nat(3)
        );
    }

    /// The two `IMax` identities from `simplify`'s doc, which read backwards on first encounter.
    #[test]
    fn imax_with_zero_or_one_on_the_left_collapses() {
        let r = p("v");
        assert_eq!(
            Level::IMax(Box::new(Level::Zero), Box::new(r.clone())).simplify(),
            r
        );
        assert_eq!(
            Level::IMax(Box::new(Level::of_nat(1)), Box::new(r.clone())).simplify(),
            r
        );
    }

    /// `imax l 0 == 0` — the impredicative case: a `Pi` into `Prop` is a `Prop` whatever its
    /// domain. This is the rule `check_infer`'s `infer_dependent_sort` implements by hand for
    /// concrete levels, and the reason `IMax` has to exist once levels can be variables.
    #[test]
    fn imax_into_prop_is_prop() {
        let l = Level::IMax(Box::new(p("u")), Box::new(Level::Zero));
        assert_eq!(l.simplify(), Level::Zero);
    }

    /// `imax u v` with `v` a parameter cannot be decided without splitting on whether `v` is
    /// zero. Both branches hold here: at `v := 0` both sides are `0`; at `v := v+1` both are
    /// `max u (v+1)`.
    #[test]
    fn imax_over_a_param_is_decided_by_cases() {
        let a = Level::IMax(Box::new(p("u")), Box::new(p("v")));
        assert!(a.leq(&a));
        // `imax u v <= max u v` — true on both branches.
        let m = Level::Max(Box::new(p("u")), Box::new(p("v")));
        assert!(a.leq(&m));
    }

    /// `max u v` is NOT below `imax u v`: at `v := 0` the left is `u` and the right is `0`.
    #[test]
    fn max_is_not_below_imax() {
        let a = Level::IMax(Box::new(p("u")), Box::new(p("v")));
        let m = Level::Max(Box::new(p("u")), Box::new(p("v")));
        assert!(!m.leq(&a));
    }

    #[test]
    fn subst_replaces_only_named_params() {
        let l = Level::Max(Box::new(p("u")), Box::new(p("v")));
        let out = l.subst(&["u".to_string()], &[Level::of_nat(2)]);
        assert_eq!(
            out,
            Level::Max(Box::new(Level::of_nat(2)), Box::new(p("v")))
        );
    }

    #[test]
    fn params_are_collected_in_first_occurrence_order_without_duplicates() {
        let l = Level::Max(
            Box::new(Level::IMax(Box::new(p("v")), Box::new(p("u")))),
            Box::new(p("v")),
        );
        assert_eq!(l.params(), vec!["v".to_string(), "u".to_string()]);
        assert!(!l.is_closed());
        assert!(Level::of_nat(3).is_closed());
    }

    #[test]
    fn eq_antisymm_sees_through_normalisation() {
        let l = Level::Max(Box::new(Level::Zero), Box::new(p("u")));
        assert!(l.eq_antisymm(&p("u")));
    }

    #[test]
    fn display_prefers_the_numeral_form() {
        assert_eq!(Level::of_nat(2).to_string(), "2");
        assert_eq!(p("u").to_string(), "u");
        assert_eq!(
            Level::Max(Box::new(p("u")), Box::new(Level::of_nat(1))).to_string(),
            "max(u, 1)"
        );
    }
}
