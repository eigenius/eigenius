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

//! Per-rule validation modules. Each file owns one D1 §5.4 rule plus
//! its directly-related helpers, with rule tests living alongside the
//! rule they exercise. The Validator's driver loop and shared helpers
//! remain in `validation/mod.rs`; the rule modules extend `Validator`
//! with `impl Validator { ... }` blocks split across files.

pub(super) mod allows_only;
pub(super) mod class_types;
pub(super) mod conditional;
pub(super) mod domain;
pub(super) mod eigentt_value;
pub(super) mod format;
pub(super) mod inductive;
pub(super) mod is_a;
pub(super) mod length;
pub(super) mod pattern;
pub(super) mod range;
pub(super) mod reference_integrity;
pub(super) mod type_check;
