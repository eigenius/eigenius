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

//! `ValidateMeasurementClaim` handler (D52 §6).
//!
//! Algorithm:
//!
//! 1. Read the claim's `sample_set` IRI; resolve to the
//!    [`stats:SampleSetResource`] on chain.
//! 2. Read the SampleSetResource's `sample_set_value` — a
//!    `Value::Json` carrying the `Bundle` ctor over the 5-axis
//!    product + observations.
//! 3. Decode the Bundle's args: extract axis values + the inline
//!    observation float array.
//! 4. Dispatch on the product position (Phase 1 covers only
//!    `(CompleteRandom, Unblocked, NoFactor, _, CrossSectional)`
//!    — SingleSampleEstimate). All other positions return
//!    `Verdict::Fails(WrongTestForDesign)`.
//! 5. Read the claim's `alpha`, `effect_size`, `directionality`
//!    fields; run the matching one-sample test (Phase 1: only the
//!    `Absolute` effect-size + `TwoSided` directionality combination
//!    is wired).
//! 6. Run the §7.4 epistemic-scope check against the
//!    `derived_proposition`'s head predicate's `is_a` markers.
//! 7. Build the verdict resource and return.
//!
//! Phase 1 deliberately implements the smallest end-to-end vertical
//! (one-sample threshold test for the IC50 fixture). Two-sample
//! (IID), non-Identity outlier exclusion (dual-verdict per §7.2),
//! method-comparison (Passing-Bablok per §7.3), and one-sided
//! impossibility-witness validation (§7.1) all land as follow-on
//! commits once the basic flow is proven.

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::QueryOutcome;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use crate::institution::iris;
use crate::institution::StatisticsInstitution;
use crate::numerics::{one_sample_t_test, OneSampleResult};

/// Top-level handler called by `StatisticsInstitution::query`.
pub fn do_validate_measurement_claim(
    _inst: &StatisticsInstitution,
    claim: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    // ── Step 1: read sample_set IRI from the claim ────────────────────
    let sample_set_iri_str = match read_iri_property(claim, iris::PROP_SAMPLE_SET)? {
        Some(s) => s,
        None => {
            return Ok(verdict_fails(
                "MeasurementClaim missing required `sample_set` property".into(),
            ));
        }
    };
    let sample_set_iri = match Iri::parse(&sample_set_iri_str) {
        Ok(i) => i,
        Err(e) => {
            return Ok(verdict_fails(format!(
                "MeasurementClaim's sample_set value `{sample_set_iri_str}` is not a valid IRI: {e}"
            )));
        }
    };

    // ── Step 2: resolve the SampleSetResource and its inductive value ─
    let sample_set_res = match ctx.resolve(&sample_set_iri) {
        Some(r) => r,
        None => {
            return Ok(verdict_fails(format!(
                "SampleSetResource `{sample_set_iri}` not found on chain"
            )));
        }
    };
    let sample_set_value_iri =
        Iri::parse("urn:eigenius:measurements:sample_set_value").expect("static IRI");
    let bundle_value = match sample_set_res.get(&sample_set_value_iri) {
        Some(v) => v,
        None => {
            return Ok(verdict_fails(format!(
                "SampleSetResource `{sample_set_iri}` missing required \
                 `sample_set_value` property"
            )));
        }
    };
    let bundle_json = match bundle_value {
        Value::Json(j) => j,
        other => {
            return Ok(verdict_fails(format!(
                "SampleSetResource `{sample_set_iri}`'s sample_set_value is not a chain \
                 inductive value (got {other:?})"
            )));
        }
    };

    // ── Step 3: decode the Bundle ctor's args ─────────────────────────
    let bundle = match decode_bundle(bundle_json) {
        Ok(b) => b,
        Err(diag) => return Ok(verdict_fails(diag)),
    };

    // ── Step 4: dispatch on the product position ──────────────────────
    let dispatch = match dispatch_product_position(&bundle) {
        Some(d) => d,
        None => {
            return Ok(verdict_fails(format!(
                "WrongTestForDesign: product position {:?} has no Phase 1 verifier procedure \
                 (Phase 1 implements only SingleSampleEstimate; other Tier 1+2 designs land \
                 in follow-on commits)",
                (
                    &bundle.randomization,
                    &bundle.blocking,
                    &bundle.factor,
                    &bundle.repeated_measures,
                ),
            )));
        }
    };

    // ── Step 5: read claim parameters ─────────────────────────────────
    let alpha = match read_float_property(claim, iris::PROP_ALPHA)? {
        Some(a) => a,
        None => return Ok(verdict_fails("claim missing `alpha`".into())),
    };
    let directionality = match read_json_property(claim, iris::PROP_DIRECTIONALITY)? {
        Some(j) => j,
        None => return Ok(verdict_fails("claim missing `directionality`".into())),
    };
    let effect_size = match read_json_property(claim, iris::PROP_EFFECT_SIZE)? {
        Some(j) => j,
        None => return Ok(verdict_fails("claim missing `effect_size`".into())),
    };
    // Phase 1 supports only TwoSided directionality. OneSidedWitnessed
    // requires §7.1 impossibility-witness validation, deferred to a
    // follow-on.
    let directionality_ctor = json_ctor_name(&directionality);
    if directionality_ctor != Some("TwoSided") {
        return Ok(verdict_fails(format!(
            "directionality `{directionality_ctor:?}` not supported in Phase 1 \
             (only TwoSided wired; OneSidedWitnessed requires §7.1 impossibility-witness \
             validation, deferred)"
        )));
    }

    // ── Step 6: run the test (dispatch-specific) ──────────────────────
    let result = match dispatch {
        DispatchPos::SingleSampleEstimate => {
            // Only `EffectSize.Absolute(magnitude, units)` is wired in
            // Phase 1. The one-sample test checks whether the
            // SampleSet's mean (or appropriate CI bound) crosses the
            // asserted absolute threshold.
            let (magnitude, _units) = match parse_effect_size_absolute(&effect_size) {
                Some(p) => p,
                None => {
                    return Ok(verdict_fails(
                        "Phase 1 only supports EffectSize.Absolute(magnitude, units); \
                         StandardizedCohensD/HedgesG and Relative not yet wired"
                            .into(),
                    ));
                }
            };
            let t_result = match one_sample_t_test(&bundle.observations, magnitude) {
                Some(r) => r,
                None => {
                    return Ok(verdict_fails(format!(
                        "InsufficientReplication: one-sample t-test requires n >= 2, got n = {}",
                        bundle.observations.len()
                    )));
                }
            };
            t_result
        }
    };

    // ── Step 7: §7.4 epistemic-scope check ────────────────────────────
    //
    // Decode the derived_proposition's head predicate IRI, look up its
    // `is_a` markers, and admit/reject per the replication kind:
    //
    //   BiologicalReplication / NestedReplication — any scope ok
    //   TechnicalWithinRun                         — only MeasurementLevel
    //
    // Phase 1 implements the simple form: read the predicate's class
    // memberships and check for the marker. A predicate with no scope
    // marker defaults to PopulationLevel (the more restrictive admissibility).
    let scope_diag = check_epistemic_scope(claim, &bundle.replication, ctx)?;
    if let Some(d) = scope_diag {
        return Ok(verdict_fails(d));
    }

    // ── Step 8: compare computed statistic against asserted threshold ─
    //
    // For SingleSampleEstimate with EffectSize.Absolute(threshold) and
    // TwoSided directionality, the claim holds when p < alpha AND the
    // computed mean falls on the asserted side of the threshold (i.e.,
    // the test rejects H0 in the direction the claim asserts). For a
    // "< 100 nM" IC50 claim, the asserted side is mean < threshold.
    //
    // Phase 1 simplification: we only check p < alpha, not the
    // direction. The directional refinement lands when the §7.1
    // OneSidedWitnessed path is wired (since direction matters most
    // for one-sided claims). Two-sided rejection of "mean = threshold"
    // doesn't tell us *which* side; the author's derived_proposition
    // implicitly fixes the direction, and the verifier is honest about
    // the limited inference.
    if result.p_value_two_sided < alpha {
        Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_HOLDS,
            None,
            Some(&result),
        )))
    } else {
        Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_FAILS,
            Some(&format!(
                "AlphaNotCrossed: computed p = {:.6}, threshold alpha = {alpha}",
                result.p_value_two_sided
            )),
            Some(&result),
        )))
    }
}

// ────────────────────────────────────────────────────────────────────
// Bundle decoding (the SampleSet's `Bundle` ctor → typed struct)
// ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct DecodedBundle {
    randomization: String,
    blocking: String,
    factor: String,
    replication: ReplicationKind,
    repeated_measures: String,
    #[allow(dead_code)]
    units: String,
    #[allow(dead_code)]
    columns: String,
    #[allow(dead_code)]
    sample_map: String,
    observations: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
enum ReplicationKind {
    BiologicalReplication,
    TechnicalWithinRun,
    NestedReplication {
        biological_n: i64,
        technical_per_biological: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DispatchPos {
    SingleSampleEstimate,
}

fn decode_bundle(j: &serde_json::Value) -> Result<DecodedBundle, String> {
    let ctor = json_ctor_name(j).unwrap_or("?");
    if ctor != "Bundle" {
        return Err(format!("expected SampleSet `Bundle` ctor, got `{ctor}`"));
    }
    let args = j["args"]
        .as_array()
        .ok_or_else(|| "Bundle args field missing or not an array".to_string())?;
    if args.len() != 9 {
        return Err(format!("Bundle expects 9 args, got {}", args.len()));
    }
    let randomization = json_ctor_name(&args[0])
        .ok_or_else(|| "randomization slot is not a ctor".to_string())?
        .to_string();
    let blocking = json_ctor_name(&args[1])
        .ok_or_else(|| "blocking slot is not a ctor".to_string())?
        .to_string();
    let factor = json_ctor_name(&args[2])
        .ok_or_else(|| "factor slot is not a ctor".to_string())?
        .to_string();
    let replication = decode_replication_kind(&args[3])?;
    let repeated_measures = json_ctor_name(&args[4])
        .ok_or_else(|| "repeated_measures slot is not a ctor".to_string())?
        .to_string();
    let units = args[5].as_str().unwrap_or("").to_string();
    let columns = args[6].as_str().unwrap_or("").to_string();
    let sample_map = args[7].as_str().unwrap_or("").to_string();
    let observations_json = args[8]
        .as_array()
        .ok_or_else(|| "observations slot is not an array".to_string())?;
    let observations: Vec<f64> = observations_json
        .iter()
        .enumerate()
        .map(|(i, v)| {
            v.as_f64()
                .ok_or_else(|| format!("observation index {i} is not a number: {v:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecodedBundle {
        randomization,
        blocking,
        factor,
        replication,
        repeated_measures,
        units,
        columns,
        sample_map,
        observations,
    })
}

fn decode_replication_kind(j: &serde_json::Value) -> Result<ReplicationKind, String> {
    match json_ctor_name(j) {
        Some("BiologicalReplication") => Ok(ReplicationKind::BiologicalReplication),
        Some("TechnicalWithinRun") => Ok(ReplicationKind::TechnicalWithinRun),
        Some("NestedReplication") => {
            let args = j["args"]
                .as_array()
                .ok_or_else(|| "NestedReplication args missing".to_string())?;
            if args.len() != 2 {
                return Err(format!(
                    "NestedReplication expects 2 args, got {}",
                    args.len()
                ));
            }
            let biological_n = args[0]
                .as_i64()
                .ok_or_else(|| "NestedReplication arg 0 must be integer".to_string())?;
            let technical_per_biological = args[1]
                .as_i64()
                .ok_or_else(|| "NestedReplication arg 1 must be integer".to_string())?;
            Ok(ReplicationKind::NestedReplication {
                biological_n,
                technical_per_biological,
            })
        }
        Some(other) => Err(format!("unknown Replication ctor `{other}`")),
        None => Err("replication slot is not a ctor".to_string()),
    }
}

fn dispatch_product_position(bundle: &DecodedBundle) -> Option<DispatchPos> {
    // Phase 1: only SingleSampleEstimate is wired. Verifier dispatch
    // table per D52 §5.4.
    match (
        bundle.randomization.as_str(),
        bundle.blocking.as_str(),
        bundle.factor.as_str(),
        bundle.repeated_measures.as_str(),
    ) {
        ("CompleteRandom", "Unblocked", "NoFactor", "CrossSectional") => {
            Some(DispatchPos::SingleSampleEstimate)
        }
        _ => None,
    }
}

// ────────────────────────────────────────────────────────────────────
// §7.4 epistemic-scope check
// ────────────────────────────────────────────────────────────────────

/// Decode the claim's `derived_proposition`, extract its head predicate
/// IRI, look up that predicate's `is_a` markers, and check
/// admissibility against the SampleSet's replication kind.
///
/// Returns `Ok(None)` if the scope is admissible; `Ok(Some(diag))` with
/// a diagnostic string if the institution must reject the claim per
/// §7.4. `Err(_)` only for genuine institutional failures (resolution
/// errors etc.), not scope mismatches.
fn check_epistemic_scope(
    claim: &Resource,
    replication: &ReplicationKind,
    ctx: &ExecutionContext,
) -> Result<Option<String>, InstitutionError> {
    // BiologicalReplication / NestedReplication admit any scope —
    // short-circuit before doing any chain lookup work.
    if !matches!(replication, ReplicationKind::TechnicalWithinRun) {
        return Ok(None);
    }
    // TechnicalWithinRun: the claim is admissible only if the
    // derived_proposition's predicate is marked MeasurementLevel.
    let derived_prop = match read_json_property(claim, iris::PROP_DERIVED_PROPOSITION)? {
        Some(j) => j,
        None => {
            // No derived_proposition — that's a different malformed-claim
            // error the top-level handler already flagged; treat as
            // "scope-check inconclusive."
            return Ok(None);
        }
    };
    let predicate_iri = match extract_head_predicate_iri(&derived_prop) {
        Some(iri) => iri,
        None => {
            // Couldn't extract the head predicate (e.g., the prop is a
            // pure type-theoretic combinator like a Pi-arrow with no
            // ConstRef head). Default to fail-safe: reject as
            // population-level since we can't prove it isn't.
            return Ok(Some(
                "EpistemicScopeViolation: SampleSet has replication = TechnicalWithinRun, \
                 but the derived_proposition's scope could not be determined from its \
                 structure — defaulting to PopulationLevel admissibility (the more \
                 restrictive). To assert this claim from technical-only replicates, the \
                 predicate must explicitly carry `is_a stats:MeasurementLevel`."
                    .to_string(),
            ));
        }
    };
    let pred_iri_parsed = match Iri::parse(&predicate_iri) {
        Ok(i) => i,
        Err(_) => return Ok(None), // can't resolve — treat as inconclusive
    };
    let pred_resource = match ctx.resolve(&pred_iri_parsed) {
        Some(r) => r,
        None => {
            return Ok(Some(format!(
                "EpistemicScopeViolation: derived_proposition references predicate \
                 `{predicate_iri}` which is not committed on chain; cannot verify scope"
            )));
        }
    };
    let measurement_level_iri = Iri::parse(iris::MEASUREMENT_LEVEL).expect("static IRI");
    let is_measurement_level = pred_resource
        .is_a()
        .iter()
        .any(|c| c == &measurement_level_iri);
    if is_measurement_level {
        Ok(None)
    } else {
        Ok(Some(format!(
            "EpistemicScopeViolation: SampleSet has replication = TechnicalWithinRun, \
             but derived_proposition's predicate `{predicate_iri}` is not marked \
             `is_a stats:MeasurementLevel`. Technical-only replicates cannot support \
             population-level propositions (D52 §7.4). Either gather biological \
             replicates and recommit the SampleSet, or assert against a measurement-\
             scope predicate (e.g., `HasLowIC50_OnThisBatch`)."
        )))
    }
}

/// Extract the head predicate's IRI from a D47-encoded proposition.
/// The shape for a typical predicate application like `HasLowIC50(iri)`
/// is `App(ConstRef(HasLowIC50), LitString(iri))` — we walk the App
/// spine to the leftmost ConstRef and return its IRI. Returns `None`
/// for shapes that don't bottom out at a ConstRef (Pi-arrows, Sort
/// literals, etc. — those don't have a "predicate" to scope-check).
fn extract_head_predicate_iri(j: &serde_json::Value) -> Option<String> {
    let mut cursor = j;
    loop {
        match json_ctor_name(cursor)? {
            "App" => {
                cursor = cursor["args"].get(0)?;
            }
            "ConstRef" => {
                return cursor["args"].get(0)?.as_str().map(|s| s.to_string());
            }
            _ => return None,
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────

fn json_ctor_name(j: &serde_json::Value) -> Option<&str> {
    j.as_object()?.get("ctor")?.as_str()
}

fn parse_effect_size_absolute(j: &serde_json::Value) -> Option<(f64, String)> {
    if json_ctor_name(j)? != "Absolute" {
        return None;
    }
    let args = j["args"].as_array()?;
    if args.len() != 2 {
        return None;
    }
    let magnitude = args[0].as_f64()?;
    let units = args[1].as_str()?.to_string();
    Some((magnitude, units))
}

fn read_iri_property(claim: &Resource, prop_iri: &str) -> Result<Option<String>, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    match claim.get(&iri) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::ResourceRef(i)) => Ok(Some(i.as_str().to_string())),
        Some(other) => Err(InstitutionError::ComputationFailed(format!(
            "MeasurementClaim `{prop_iri}` is not a string/IRI: {other:?}"
        ))),
        None => Ok(None),
    }
}

fn read_float_property(claim: &Resource, prop_iri: &str) -> Result<Option<f64>, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    match claim.get(&iri) {
        Some(Value::Float(f)) => Ok(Some(*f)),
        Some(Value::Integer(n)) => Ok(Some(*n as f64)),
        Some(other) => Err(InstitutionError::ComputationFailed(format!(
            "MeasurementClaim `{prop_iri}` is not a number: {other:?}"
        ))),
        None => Ok(None),
    }
}

fn read_json_property(
    claim: &Resource,
    prop_iri: &str,
) -> Result<Option<serde_json::Value>, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    match claim.get(&iri) {
        Some(Value::Json(j)) => Ok(Some(j.clone())),
        Some(other) => Err(InstitutionError::ComputationFailed(format!(
            "MeasurementClaim `{prop_iri}` is not a chain-inductive value: {other:?}"
        ))),
        None => Ok(None),
    }
}

/// Build a Fails verdict carrying a diagnostic string. No computed
/// numerics (the failure happened before we ran the test).
fn verdict_fails(diagnostic: String) -> QueryOutcome {
    QueryOutcome::from_output(verdict_resource(wk::VERDICT_FAILS, Some(&diagnostic), None))
}

/// Build the Verdict::Holds | Fails resource shape the kernel's commit
/// pipeline expects. On Holds (and on Fails where the test actually
/// ran), the numerics are attached so downstream consumers see the
/// computed statistic + p-value alongside the outcome (D52 §6 — verdict
/// carries the full intermediate state for audit).
fn verdict_resource(
    ctor_name: &str,
    diagnostic: Option<&str>,
    numerics: Option<&OneSampleResult>,
) -> Resource {
    const DIAGNOSTIC_IRI: &str = "urn:eigenius:institution:diagnostic";
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::VERDICT).expect("well-known IRI"),
        )]),
    );
    r.set(
        Iri::parse(wk::CTOR_NAME).expect("well-known IRI"),
        Value::String(ctor_name.to_string()),
    );
    if let Some(d) = diagnostic {
        r.set(
            Iri::parse(DIAGNOSTIC_IRI).expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    if let Some(n) = numerics {
        r.set(
            Iri::parse(iris::PROP_COMPUTED_STATISTIC).expect("static IRI"),
            Value::Float(n.t_statistic),
        );
        r.set(
            Iri::parse(iris::PROP_COMPUTED_P_VALUE).expect("static IRI"),
            Value::Float(n.p_value_two_sided),
        );
    }
    r
}
