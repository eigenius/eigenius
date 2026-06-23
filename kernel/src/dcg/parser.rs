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

//! The composition parser: parse items, forward/backward application, and a CKY
//! chart over categorial categories. The categorial type drives composition; the
//! kernel confirms the assembled term is well-typed (the felicity oracle).

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::term::Exp;

use super::category::{cat_subsumes, is_ctor};

/// A parse item: a category (`lexicon:Cat` term) and its assembled EigenTT sem.
#[derive(Clone)]
pub struct Item {
    pub cat: Exp,
    pub sem: Exp,
}

/// One combinatory step: forward (`A/B · B → A`) or backward (`B · A\B → A`),
/// assembling the sem by application in lockstep. The argument category need not
/// equal the slot — it must *subsume into* it ([`cat_subsumes`]), so an
/// `NP[Gene]` fills an `NP[Entity]` slot. A non-match returns `None` — the
/// parse-time felicity filter, on the category alone.
pub fn apply(left: &Item, right: &Item, layer: &Arc<Layer>) -> Option<Item> {
    if let Some(args) = is_ctor(&left.cat, "fwd") {
        if args.len() == 2 && cat_subsumes(&args[1], &right.cat, layer) {
            return Some(Item {
                cat: args[0].clone(),
                sem: Exp::App(Box::new(left.sem.clone()), Box::new(right.sem.clone())),
            });
        }
    }
    if let Some(args) = is_ctor(&right.cat, "bwd") {
        if args.len() == 2 && cat_subsumes(&args[1], &left.cat, layer) {
            return Some(Item {
                cat: args[0].clone(),
                sem: Exp::App(Box::new(right.sem.clone()), Box::new(left.sem.clone())),
            });
        }
    }
    None
}

/// CKY: `chart[i][j]` holds every item spanning tokens `i..=j`; returns the
/// items spanning the whole input.
pub fn cky_parse(tokens: &[Item], layer: &Arc<Layer>) -> Vec<Item> {
    let n = tokens.len();
    if n == 0 {
        return Vec::new();
    }
    let mut chart: Vec<Vec<Vec<Item>>> = vec![vec![Vec::new(); n]; n];
    for (i, t) in tokens.iter().enumerate() {
        chart[i][i].push(t.clone());
    }
    for len in 2..=n {
        for i in 0..=(n - len) {
            let j = i + len - 1;
            let mut produced = Vec::new();
            for k in i..j {
                let lefts = chart[i][k].clone();
                let rights = chart[k + 1][j].clone();
                for l in &lefts {
                    for r in &rights {
                        if let Some(item) = apply(l, r, layer) {
                            produced.push(item);
                        }
                    }
                }
            }
            chart[i][j] = produced;
        }
    }
    chart[0][n - 1].clone()
}
