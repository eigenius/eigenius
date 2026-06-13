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

//! P5: the WRN xenograft recompute (C-VIVO `InVivoDependence`), wrapping
//! the authors' own lme4 random-slope mixed-model LRT
//! (`data/WRN_manuscript/src/in_vivo_KM12_analysis.R`) through the R
//! runtime. The KM12 shWRN1 xenograft tumor volumes (Fig 2d, Dox± , 5+5
//! mice × up to 8 days) are dispatched as a chain-resident table; the R
//! script fits `lmer(Volume ~ Day + (0+Day|Mouse))` vs `+ Day:Dox` and
//! returns the LRT p as an Eigon `DerivedResource`.
//!
//! Testability: the data marshalling runs everywhere (the script decodes
//! the four columns before `library(lme4)`), so this test exercises the
//! real-data decode even without lme4. The lme4 fit itself needs lme4 in
//! the worker's R — present in the pinned image (D55 P3) but NOT
//! installable in this sandbox (no cmake → nloptr/lme4 won't compile), so
//! when lme4 is absent the test asserts the marshalling reached the model
//! and skips the LRT assertion. With lme4 present it asserts the
//! interaction LRT is significant — WRN depletion (Dox) slows shWRN1
//! tumor growth = `InVivoDependence`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_r::RLanguageRuntime;
use eigenius_runtime_substrate::error::RunError;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::service::LocalServiceSpawner;

const VOL: &str = "urn:eigenius:pub:wrn:vivo_volume";
const DAY: &str = "urn:eigenius:pub:wrn:vivo_day";
const MOUSE: &str = "urn:eigenius:pub:wrn:vivo_mouse";
const DOX: &str = "urn:eigenius:pub:wrn:vivo_dox";
const P_VALUE: &str = "urn:eigenius:measurements:lrt_p_value";

// KM12 shWRN1 xenograft tumor volumes (mm³), Fig 2d (MOESM3), long form:
// Dox(+) = 5 mice × 8 days, Dox(−) = 5 mice × up to 8 days (some censored).
// Tier-1-pinnable from wrn_sourcedata_Fig2_MOESM3.xlsx sheet "Fig 2d".
#[rustfmt::skip]
const VOLUME: [f64; 73] = [
    128.6305,284.9532,500.6995,731.3401,930.1379,1049.7137,1126.9359,1342.8826,
    58.9235,89.3875,202.9264,316.8343,361.9952,421.7119,536.0007,625.9556,
    52.0601,93.7585,256.1576,282.0301,325.1588,320.5281,327.2902,417.8337,
    81.2132,179.314,197.5635,248.1334,248.4152,216.3888,226.261,273.0389,
    110.507,191.8106,260.5912,307.5247,322.5753,364.6765,371.3432,416.6321,
    97.6182,202.1637,488.2011,755.9266,894.1325,1138.3534,857.7838,
    67.0325,173.2418,400.6983,685.5026,938.5215,
    45.2362,152.453,360.0215,530.5973,815.5537,1209.5189,1494.3099,
    94.9881,269.1331,392.4027,629.8772,901.4483,1176.4432,1603.7343,
    61.8507,80.2415,136.6937,146.7078,191.9082,242.374,261.0864,
];
#[rustfmt::skip]
const DAYS: [f64; 73] = [
    0.,3.,6.,9.,12.,15.,18.,21., 0.,3.,6.,9.,12.,15.,18.,21.,
    0.,3.,6.,9.,12.,15.,18.,21., 0.,3.,6.,9.,12.,15.,18.,21.,
    0.,3.,6.,9.,12.,15.,18.,21.,
    0.,3.,6.,9.,12.,15.,18., 0.,3.,6.,9.,12.,
    0.,3.,6.,9.,12.,15.,18., 0.,3.,6.,9.,12.,15.,18.,
    0.,3.,6.,9.,12.,15.,18.,
];
const MICE: [&str; 73] = [
    "Y1", "Y1", "Y1", "Y1", "Y1", "Y1", "Y1", "Y1", "Y2", "Y2", "Y2", "Y2", "Y2", "Y2", "Y2", "Y2",
    "Y3", "Y3", "Y3", "Y3", "Y3", "Y3", "Y3", "Y3", "Y4", "Y4", "Y4", "Y4", "Y4", "Y4", "Y4", "Y4",
    "Y5", "Y5", "Y5", "Y5", "Y5", "Y5", "Y5", "Y5", "N1", "N1", "N1", "N1", "N1", "N1", "N1", "N2",
    "N2", "N2", "N2", "N2", "N3", "N3", "N3", "N3", "N3", "N3", "N3", "N4", "N4", "N4", "N4", "N4",
    "N4", "N4", "N5", "N5", "N5", "N5", "N5", "N5", "N5",
];

/// The authors' lme4 model (in_vivo_KM12_analysis.R), wrapped: random-slope
/// mixed model, Day:Dox interaction LRT. Returns the LRT p as a
/// DerivedResource.
const SCRIPT: &str = r#"
in0   <- eigenius_inputs[[1]]
vol   <- .Call("r_eigon_f64_array", in0, "urn:eigenius:pub:wrn:vivo_volume")
day   <- .Call("r_eigon_f64_array", in0, "urn:eigenius:pub:wrn:vivo_day")
mouse <- .Call("r_eigon_str_array", in0, "urn:eigenius:pub:wrn:vivo_mouse")
dox   <- .Call("r_eigon_str_array", in0, "urn:eigenius:pub:wrn:vivo_dox")
df <- data.frame(Volume = vol, Day = day, Mouse = factor(mouse), Dox = factor(dox))
stopifnot(nrow(df) == 73L, nlevels(df$Dox) == 2L)
library(lme4)
m1 <- lmer(Volume ~ Day + (0 + Day | Mouse), df, REML = FALSE)
m2 <- lmer(Volume ~ Day + Day:Dox + (0 + Day | Mouse), df, REML = FALSE)
a <- anova(m1, m2)
pval <- a[["Pr(>Chisq)"]][2]
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:vivo_lme4:result")
.Call("r_eigon_add_class", b, "urn:eigenius:reflection:DerivedResource")
.Call("r_eigon_set_f64", b, "urn:eigenius:measurements:lrt_p_value", pval)
.Call("r_eigon_finish", b)
"#;

fn cdylib_path() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let profile = exe.parent().and_then(|d| d.parent()).expect("profile dir");
    profile.join("libeigenius_r_worker.so")
}
fn driver_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eigenius-r-worker/r/EigeniusRWorker.R")
}
fn rscript_available() -> bool {
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn xenograft_lme4_recompute() {
    if !rscript_available() {
        eprintln!("skipping xenograft_lme4_recompute: Rscript unavailable");
        return;
    }
    let cdylib = cdylib_path();
    assert!(cdylib.exists(), "cdylib not built");

    let depot = tempfile::tempdir().expect("depot");
    let spawner = Arc::new(LocalServiceSpawner::new(depot.path().to_path_buf()));
    let runtime = RLanguageRuntime::new(spawner, driver_path(), cdylib, depot.path().to_path_buf());

    let mut table = Resource::new(Iri::parse("urn:eigenius:pub:wrn:vivo_xenograft_table").unwrap());
    let f64s = |xs: &[f64]| Value::Array(xs.iter().map(|v| Value::Float(*v)).collect());
    let strs =
        |xs: &[&str]| Value::Array(xs.iter().map(|s| Value::String(s.to_string())).collect());
    table.set(Iri::parse(VOL).unwrap(), f64s(&VOLUME));
    table.set(Iri::parse(DAY).unwrap(), f64s(&DAYS));
    table.set(Iri::parse(MOUSE).unwrap(), strs(&MICE));
    table.set(
        Iri::parse(DOX).unwrap(),
        strs(&MICE.map(|m| if m.starts_with('Y') { "Y" } else { "N" })),
    );

    let mut script = Resource::new(Iri::parse("urn:eigenius:pub:wrn:vivo_lme4_script").unwrap());
    script.set(
        Iri::parse("urn:eigenius:runtime:source").unwrap(),
        Value::String(SCRIPT.to_string()),
    );
    let env = Resource::new(Iri::parse("urn:eigenius:test:renv").unwrap());

    match runtime.run_script(&env, &script, &[table]) {
        Ok(outcome) => {
            // lme4 present → the LRT ran. WRN depletion (Dox) slows shWRN1
            // tumor growth: the Day:Dox interaction is significant.
            let p = match outcome.output.get(&Iri::parse(P_VALUE).unwrap()) {
                Some(Value::Float(f)) => *f,
                other => panic!("lrt_p_value not a Float: {other:?}"),
            };
            assert!(
                p < 0.05,
                "xenograft Day:Dox LRT not significant (p={p}) — expected InVivoDependence"
            );
        }
        Err(RunError::RuntimeError(msg))
            if msg.contains("lme4") || msg.contains("there is no package") =>
        {
            // lme4 not in the worker's R (this sandbox). The marshalling
            // still ran: the script decoded all 4 columns + built the
            // 73-row data.frame and passed the stopifnot(nrow==73) BEFORE
            // library(lme4) failed — so the real-data decode is exercised.
            eprintln!("skipping LRT assertion (lme4 unavailable in worker R): {msg}");
        }
        Err(e) => panic!("xenograft recompute failed unexpectedly: {e}"),
    }
}
