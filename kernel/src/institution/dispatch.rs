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

//! AutoOnLoad institution dispatch (D14 §9.1).
//!
//! For each newly committed resource whose class has at least one
//! `QueryClass` with `dispatch_role` including `AutoOnLoad`, run the
//! query and gate the Load on the resulting `Verdict`:
//! - `Holds` and `Undecidable` accept.
//! - `Fails` produces a typed `ValidationError`.
//!
//! This module also serves the post-translation validation invariant
//! (D14 §9.3 step 5): after [`Exp::InstitutionInvoke`] produces a
//! target-class resource, the same single-resource dispatch runs to
//! verify the target institution accepts what its `reify` constructed.
//!
//! Component-implemented QueryClasses (where `query_handler` resolves
//! to a kernel-registered Component rather than an institution-runtime
//! procedure) are not yet wired here — the kernel surfaces a
//! `NotImplemented` error for them. M8 lands the Component path
//! alongside the legacy retirement.

use crate::context::ExecutionContext;
use crate::institution::marshal::embed_typed_resource_refs_recursively;
use crate::institution::registry::{DispatchRole, InstitutionIndex};
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::validation::{ValidationError, ValidationRule};

/// One AutoOnLoad dispatch that produced a well-formed Verdict.
/// Carries everything the commit pipeline needs to build the chain-
/// side `RuntimeInvocation` + `Verdict` resources per
/// [D31 §6.3](../../docs/design/d31-external-institution-lifecycle.md#63-verdict-commit-semantics).
///
/// Handler crashes / malformed Verdicts do NOT produce one of these —
/// they go into `AutoOnLoadOutcome::errors` instead, because there is
/// no Verdict to commit as the audit anchor.
#[derive(Debug, Clone)]
pub struct AutoOnLoadDispatch {
    /// The gated resource IRI (the resource whose Load triggered this
    /// dispatch). Lifted onto the Verdict's `verdict_subject` and the
    /// RuntimeInvocation's `inputs` list. `None` when the dispatch
    /// fired against an embedded resource (no `@id`) — the
    /// post-translation validation path in `nbe::eval` exercises this:
    /// it gates a freshly-reified resource that hasn't been assigned
    /// an IRI yet. Such dispatches still apply the
    /// Holds/Fails/Undecidable rule but skip the chain-side
    /// provenance commit.
    pub subject_iri: Option<Iri>,
    /// The `QueryClass` resource IRI that fired. Lifted onto the
    /// Verdict's `verdict_query_class`.
    pub query_class_iri: Iri,
    /// IRI of the `RuntimeMethodSignature` the handler dispatched
    /// against (= `QueryClass.query_handler`). Lifted onto the
    /// RuntimeInvocation's `script` property.
    pub signature_iri: Iri,
    /// The verdict the institution returned.
    pub verdict: VerdictReading,
    /// The institution's output `Resource` — the institution-level
    /// Verdict. Carries the `ctor_name` the kernel reads via
    /// [`parse_verdict`] plus any institution-set properties the
    /// kernel preserves onto the chain-committed Verdict via
    /// [`build_verdict_resource`]'s merge step. This resource is the
    /// pass/fail gate; per-derivation propositions live separately on
    /// [`derivations`].
    pub output: Resource,
    /// Side-effect resources the institution emitted as artefacts of
    /// validation — committed alongside the Verdict when it Holds,
    /// dropped when it Fails. Each derivation is marked
    /// `reflection:InstitutionEmittedDerivation` and carries a
    /// `canonical_proposition` the chain attests; the witness emitter
    /// walks these directly to admit `IsDerivedAs(derivation_iri, P)`.
    /// Empty for institutions whose only job is the pass/fail gate.
    pub derivations: Vec<Resource>,
    /// Substrate-captured partial `RuntimeInvocation` (D26 §5.5).
    /// `None` for in-process institutions whose dispatch
    /// happens entirely inside the kernel host process — the kernel
    /// records its own program-trace provenance for those.
    pub partial_invocation: Option<Resource>,
    /// The `RuntimeEnvironment` IRI the institution declared via
    /// `requires_environment` (D31 §5). Only populated for
    /// external-runtime institutions; lifted onto the
    /// RuntimeInvocation's `environment` property.
    pub environment_iri: Option<Iri>,
}

/// Aggregate outcome of an AutoOnLoad sweep. Errors arise only for
/// dispatches that couldn't produce a Verdict at all (missing
/// institution, handler crashed, malformed output). Every dispatch
/// that *did* produce a Verdict — including Fails — lands in
/// `dispatches` so the commit pipeline can build the audit-anchor
/// chain resources before applying the Holds/Fails/Undecidable rule.
#[derive(Debug, Clone, Default)]
pub struct AutoOnLoadOutcome {
    /// Handler-side failures with no chain-side provenance.
    pub errors: Vec<ValidationError>,
    /// Per-dispatch provenance bundles to be lifted into chain
    /// resources by the caller.
    pub dispatches: Vec<AutoOnLoadDispatch>,
}

impl AutoOnLoadOutcome {
    /// Flatten the outcome into a single error list for callers that
    /// don't need the commit-pipeline shape (post-translation
    /// validation in `nbe::eval`, FIBER queries that just need
    /// "did the gate accept?"). Handler errors pass through; `Fails`
    /// dispatches become `InstitutionValidation` errors with the
    /// QueryClass IRI in the message.
    pub fn flatten_to_errors(self) -> Vec<ValidationError> {
        let mut errors = self.errors;
        for d in self.dispatches {
            if matches!(d.verdict, VerdictReading::Fails) {
                errors.push(ValidationError {
                    resource_id: d.subject_iri.clone(),
                    property: None,
                    rule: ValidationRule::InstitutionValidation,
                    message: format!(
                        "AutoOnLoad QueryClass `{}` returned Fails",
                        d.query_class_iri
                    ),
                });
            }
        }
        errors
    }
}

/// Run AutoOnLoad QueryClasses for every class on `resource`, against
/// the given index + runtime.
///
/// Returns an [`AutoOnLoadOutcome`] aggregating per-dispatch
/// provenance (Holds, Fails, and Undecidable verdicts all produce a
/// dispatch entry) alongside handler-side errors that prevented a
/// Verdict from being produced at all (missing institution, handler
/// crashed, malformed output). The caller decides how to translate
/// these into chain-side `RuntimeInvocation` + `Verdict` commits and
/// `Load`-failure ValidationErrors per
/// [D31 §6.3](../../docs/design/d31-external-institution-lifecycle.md#63-verdict-commit-semantics).
///
/// Used both by the Load-path layer dispatch (one resource at a time
/// from the new layer) and by [`Exp::InstitutionInvoke`] post-
/// translation validation (a single resource produced by reify).
pub fn dispatch_auto_on_load_for_resource(
    resource: &Resource,
    index: &InstitutionIndex,
    runtime: &InstitutionRuntime,
    ctx: &ExecutionContext,
) -> AutoOnLoadOutcome {
    let mut outcome = AutoOnLoadOutcome::default();
    let res_id = resource.id().cloned();

    for class_iri_str in resource.is_a() {
        let class_iri = match Iri::parse(class_iri_str.as_str()) {
            Ok(i) => i,
            Err(_) => continue,
        };
        for query_class_iri in index.auto_on_load_for(&class_iri) {
            let Some(query_class) = index.query_class(query_class_iri) else {
                continue;
            };
            // Sanity: AutoOnLoad QueryClasses must declare Verdict as
            // their result_class — D14 §4.4. If a malformed
            // declaration slipped past structural validation, surface
            // it here rather than silently mis-dispatching.
            if !query_class
                .dispatch_roles
                .contains(&DispatchRole::AutoOnLoad)
            {
                continue;
            }

            // M7 supports only institution-runtime handlers. Component-
            // implemented QueryClasses surface a typed error so the
            // caller (Load handler / post-translation invariant) sees
            // the gap clearly.
            let Some(institution) = runtime.get(&query_class.institution_ref) else {
                outcome.errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InstitutionValidation,
                    message: format!(
                        "AutoOnLoad QueryClass `{query_class_iri}` declares institution `{}` not registered in runtime",
                        query_class.institution_ref
                    ),
                });
                continue;
            };

            // The institution declaration may carry a
            // `requires_environment` IRI for external-runtime
            // institutions; the commit pipeline uses it to populate
            // the RuntimeInvocation's `environment` property.
            let environment_iri = index
                .institution(&query_class.institution_ref)
                .and_then(|e| e.requires_environment.clone());

            // Marshal IRI-shaped resource references in the input
            // resource into embedded resources before dispatch. The
            // worker's mirror decoders expect resource-typed fields
            // to carry an embedded map, not a chain-bound IRI string;
            // a chain-author writing
            //   resource :s : SomeClass {
            //       :referenced_field = :other_resource;
            //   }
            // expects the kernel to inline `:other_resource` before
            // serialising — same dereference pass FIBER param values
            // get (D2 v2 §6.12 / Phase 19d.2 follow-on), now extended
            // to AutoOnLoad-gated subjects so the recursion walks
            // nested resource refs (e.g. an OdeSolution → OdeProblem
            // → Vector<RhsComponent> chain).
            let marshaled = match embed_typed_resource_refs_recursively(
                resource.clone(),
                ctx.head(),
            ) {
                Ok(r) => r,
                Err(e) => {
                    outcome.errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: None,
                        rule: ValidationRule::InstitutionValidation,
                        message: format!(
                            "AutoOnLoad QueryClass `{query_class_iri}`: failed to marshal resource references: {e:?}"
                        ),
                    });
                    continue;
                }
            };

            match institution.query(&query_class.query_handler, &marshaled, ctx) {
                Ok(out) => {
                    let verdict = parse_verdict(&out.output);
                    if let VerdictReading::Malformed(reason) = &verdict {
                        outcome.errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: None,
                            rule: ValidationRule::InstitutionValidation,
                            message: format!(
                                "AutoOnLoad QueryClass `{query_class_iri}` returned a non-Verdict result: {reason}"
                            ),
                        });
                        continue;
                    }
                    outcome.dispatches.push(AutoOnLoadDispatch {
                        subject_iri: res_id.clone(),
                        query_class_iri: query_class_iri.clone(),
                        signature_iri: query_class.query_handler.clone(),
                        verdict,
                        output: out.output,
                        derivations: out.derivations,
                        partial_invocation: out.partial_invocation,
                        environment_iri,
                    });
                }
                Err(e) => outcome.errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InstitutionValidation,
                    message: format!(
                        "AutoOnLoad QueryClass `{query_class_iri}` handler `{}` failed: {e}",
                        query_class.query_handler
                    ),
                }),
            }
        }
    }
    outcome
}

/// Run AutoOnLoad dispatch for every resource in `layer.resources()`.
/// Used by the Load path on commit (D14 §9.1). Walks only the
/// current layer's *own* resources — parent-chain resources have
/// already been validated when their layer landed.
pub fn dispatch_auto_on_load_for_layer(
    layer: &Layer,
    index: &InstitutionIndex,
    runtime: &InstitutionRuntime,
    ctx: &ExecutionContext,
) -> AutoOnLoadOutcome {
    let mut outcome = AutoOnLoadOutcome::default();
    for (_iri, resource) in layer.iter_resources() {
        let part = dispatch_auto_on_load_for_resource(&resource, index, runtime, ctx);
        outcome.errors.extend(part.errors);
        outcome.dispatches.extend(part.dispatches);
    }
    outcome
}

/// Build a chain-committable `Verdict` resource per [D31 §6.3].
/// The IRI scheme `urn:eigenius:invocation:<inv-id>:verdict` ties the
/// Verdict deterministically to its produced-by `RuntimeInvocation`
/// — the suffix is enough discriminator because one AutoOnLoad
/// firing produces exactly one Verdict.
///
/// `runtime_invocation_iri` is `Some` when the dispatch was external
/// (a chain-committed RuntimeInvocation accompanies the Verdict);
/// `None` for in-process dispatches whose provenance is
/// program-trace-only. In the `None` case the IRI scheme falls back
/// to `urn:eigenius:verdict:<query-class-short>:<subject-short>` so
/// every Verdict still has a stable @id without inventing a fake
/// invocation ID.
///
/// Returns `None` when the dispatch fired against an embedded subject
/// — there's no `@id` to record on the Verdict's `verdict_subject`,
/// and the post-translation validation path that produces such
/// dispatches doesn't need a chain-side audit anchor.
pub fn build_verdict_resource(
    dispatch: &AutoOnLoadDispatch,
    runtime_invocation_iri: Option<&Iri>,
    diagnostic: Option<&str>,
    dispatched_to: Option<&str>,
) -> Option<Resource> {
    use crate::ontology::well_known as wk;

    let subject_iri = dispatch.subject_iri.as_ref()?;
    let verdict_iri = derive_verdict_iri_for(runtime_invocation_iri, subject_iri);
    let mut r = Resource::new(verdict_iri);
    r.set(
        Iri::parse(wk::IS_A).expect("static IRI"),
        Value::Array(vec![
            Value::String(wk::VERDICT.to_string()),
            Value::String(wk::DERIVED_RESOURCE.to_string()),
        ]),
    );
    r.set(
        Iri::parse(wk::CTOR_NAME).expect("static IRI"),
        Value::String(dispatch.verdict.ctor_name().to_string()),
    );
    r.set(
        Iri::parse(VERDICT_SUBJECT_PROP).expect("static IRI"),
        Value::ResourceRef(subject_iri.clone()),
    );
    r.set(
        Iri::parse(VERDICT_QUERY_CLASS_PROP).expect("static IRI"),
        Value::ResourceRef(dispatch.query_class_iri.clone()),
    );
    if let Some(inv) = runtime_invocation_iri {
        r.set(
            Iri::parse(RUNTIME_INVOCATION_PROP).expect("static IRI"),
            Value::ResourceRef(inv.clone()),
        );
    }
    if let Some(d) = dispatched_to {
        r.set(
            Iri::parse("urn:eigenius:runtime:dispatched_to").expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    if let Some(diag) = diagnostic {
        r.set(
            Iri::parse(VERDICT_DIAGNOSTIC_PROP).expect("static IRI"),
            Value::String(diag.to_string()),
        );
    }
    // Merge institution-output properties onto the chain-committed
    // Verdict. The kernel sets is_a, ctor_name, verdict_subject,
    // verdict_query_class, runtime_invocation, dispatched_to,
    // diagnostic; any other property the institution returned on its
    // output Resource (e.g. statistics-institution's
    // canonical_proposition, computed_statistic, computed_p_value) is
    // copied through so the Verdict carries the full audit-anchor
    // shape the institution computed.
    let protected = protected_verdict_properties();
    for (prop_iri, value) in dispatch.output.properties() {
        if protected.contains(prop_iri.as_str()) {
            continue;
        }
        // Skip if already set (kernel-set properties take precedence).
        if r.has(prop_iri) {
            continue;
        }
        r.set(prop_iri.clone(), value.clone());
    }
    Some(r)
}

/// IRIs the kernel's `build_verdict_resource` sets itself — institution-
/// output properties at the same IRI are NOT merged through, because the
/// kernel's values are the source of truth. Other institution outputs
/// (computed_statistic, computed_p_value, the institution's own
/// diagnostic, etc.) flow through.
///
/// `VERDICT_DIAGNOSTIC_PROP` is intentionally NOT in this set:
/// although the kernel exposes a `diagnostic` parameter on
/// [`build_verdict_resource`], every kernel call site today passes
/// `None`. Protecting the IRI would silently swallow institution-set
/// diagnostics (every institution puts its own diagnostic on the
/// returned Verdict's `institution:diagnostic` property), making
/// AutoOnLoad Fails verdicts on the chain unreadable. Once the
/// kernel grows its own diagnostic-set callers we can reintroduce
/// the guard with a "kernel preempts" merge semantic instead of an
/// "everything-or-nothing" filter.
fn protected_verdict_properties() -> std::collections::HashSet<&'static str> {
    use crate::ontology::well_known as wk;
    [
        wk::IS_A,
        wk::CTOR_NAME,
        VERDICT_SUBJECT_PROP,
        VERDICT_QUERY_CLASS_PROP,
        RUNTIME_INVOCATION_PROP,
        "urn:eigenius:runtime:dispatched_to",
    ]
    .into_iter()
    .collect()
}

/// Compute the chain-committable Verdict IRI per D31 §6.3 + the
/// D52 verdict-as-DerivedResource shape:
///
/// - If a `RuntimeInvocation` accompanies the Verdict (external /
///   substrate-hosted institutions), the Verdict IRI is
///   `{invocation_iri}:verdict` — preserves the existing scheme so
///   D28 / Julia institutions keep their per-invocation Verdict IRIs.
/// - Otherwise (in-process institutions where the verdict is
///   a deterministic function of the subject), the Verdict IRI is
///   `{subject_iri}:verdict` — a deterministic, 1:1 derivation that
///   lets downstream `DerivedEvidence` consumers cite the Verdict
///   directly without a UUID-indirection lookup. Re-runs against the
///   same claim produce the same Verdict IRI; the chain's append-only
///   discipline collapses idempotent re-emission to a no-op.
pub fn derive_verdict_iri_for(runtime_invocation_iri: Option<&Iri>, subject_iri: &Iri) -> Iri {
    match runtime_invocation_iri {
        Some(inv) => Iri::parse(&format!("{}:verdict", inv.as_str())).expect("derived IRI"),
        None => fallback_verdict_iri(subject_iri),
    }
}

/// Build the full chain-committable `RuntimeInvocation` resource by
/// folding the substrate's partial provenance with the IRIs the
/// kernel knows: `script` ← signature_iri, `environment` ← env_iri,
/// `inputs` ← gated subject IRI (single-element list per AutoOnLoad
/// firing), `output` ← Verdict IRI.
pub fn build_runtime_invocation_resource(
    dispatch: &AutoOnLoadDispatch,
    invocation_iri: &Iri,
    verdict_iri: &Iri,
) -> Option<Resource> {
    use crate::ontology::well_known as wk;

    let partial = dispatch.partial_invocation.as_ref()?;
    let subject_iri = dispatch.subject_iri.as_ref()?;
    let mut r = Resource::new(invocation_iri.clone());
    r.set(
        Iri::parse(wk::IS_A).expect("static IRI"),
        Value::Array(vec![
            Value::String("urn:eigenius:runtime:RuntimeInvocation".to_string()),
            Value::String(wk::DERIVED_RESOURCE.to_string()),
        ]),
    );
    // Carry forward every property the substrate captured (language,
    // image_digest, started_at, completed_at, numerical_metadata,
    // dispatched_to). Skip `is_a` because we already set our own.
    let is_a_iri = Iri::parse(wk::IS_A).expect("static IRI");
    for (prop_iri, val) in partial.properties() {
        if prop_iri == &is_a_iri {
            continue;
        }
        r.set(prop_iri.clone(), val.clone());
    }
    // Now stamp the IRIs only the kernel knows.
    r.set(
        Iri::parse("urn:eigenius:runtime:script").expect("static IRI"),
        Value::ResourceRef(dispatch.signature_iri.clone()),
    );
    if let Some(env) = &dispatch.environment_iri {
        r.set(
            Iri::parse("urn:eigenius:runtime:environment").expect("static IRI"),
            Value::ResourceRef(env.clone()),
        );
    }
    r.set(
        Iri::parse("urn:eigenius:runtime:inputs").expect("static IRI"),
        Value::Array(vec![Value::ResourceRef(subject_iri.clone())]),
    );
    r.set(
        Iri::parse("urn:eigenius:runtime:output").expect("static IRI"),
        Value::ResourceRef(verdict_iri.clone()),
    );
    Some(r)
}

/// Allocate an IRI for a fresh `RuntimeInvocation`. Uses a v4 UUID
/// so concurrent dispatches don't collide; the `urn:eigenius:invocation:`
/// prefix matches D31 §6.3's Verdict-IRI derivation rule.
pub fn allocate_invocation_iri() -> Iri {
    Iri::parse(&format!("urn:eigenius:invocation:{}", uuid::Uuid::new_v4()))
        .expect("uuid-derived IRI parses")
}

/// Stamp the kernel-set linkage properties on each institution-emitted
/// derivation resource: add
/// `reflection:InstitutionEmittedDerivation` + `reflection:DerivedResource`
/// to the `is_a` list, set `reflection:from_subject` to the gated
/// subject IRI, and set `reflection:runtime_invocation` to the producing
/// RuntimeInvocation IRI (when one was allocated for this dispatch).
///
/// The institution sets the derivation's `@id` (typically a suffix off
/// the gated subject, e.g. `{analysis_iri}:result:{effect_name}`) and
/// the domain-specific properties (canonical_proposition, numerics,
/// per-effect ctor). The kernel adds only the linkage + marker class.
///
/// Returns `None` for derivations the kernel can't link (no
/// `@id` on the derivation, or no `subject_iri` on the dispatch — both
/// indicate an embedded-resource path that doesn't get a chain commit).
pub fn finalize_emitted_derivation(
    dispatch: &AutoOnLoadDispatch,
    runtime_invocation_iri: Option<&Iri>,
    mut derivation: Resource,
) -> Option<Resource> {
    use crate::ontology::well_known as wk;

    derivation.id()?;
    let subject_iri = dispatch.subject_iri.as_ref()?;

    let is_a_iri = Iri::parse(wk::IS_A).expect("static IRI");
    let mut classes: Vec<Value> = match derivation.get(&is_a_iri) {
        Some(Value::Array(arr)) => arr.clone(),
        Some(other) => vec![other.clone()],
        None => Vec::new(),
    };
    let has_class = |classes: &[Value], iri: &str| {
        classes.iter().any(|v| match v {
            Value::String(s) => s == iri,
            Value::ResourceRef(i) => i.as_str() == iri,
            _ => false,
        })
    };
    if !has_class(&classes, wk::DERIVED_RESOURCE) {
        classes.push(Value::String(wk::DERIVED_RESOURCE.to_string()));
    }
    if !has_class(&classes, wk::INSTITUTION_EMITTED_DERIVATION) {
        classes.push(Value::String(
            wk::INSTITUTION_EMITTED_DERIVATION.to_string(),
        ));
    }
    derivation.set(is_a_iri, Value::Array(classes));

    derivation.set(
        Iri::parse(wk::FROM_SUBJECT).expect("static IRI"),
        Value::ResourceRef(subject_iri.clone()),
    );
    if let Some(inv) = runtime_invocation_iri {
        derivation.set(
            Iri::parse(wk::RUNTIME_INVOCATION).expect("static IRI"),
            Value::ResourceRef(inv.clone()),
        );
    }
    Some(derivation)
}

/// Property IRI for `Verdict.verdict_subject` (D31 §6.3).
const VERDICT_SUBJECT_PROP: &str = "urn:eigenius:institution:verdict_subject";
/// Property IRI for `Verdict.verdict_query_class`.
const VERDICT_QUERY_CLASS_PROP: &str = "urn:eigenius:institution:verdict_query_class";
/// Property IRI for `Verdict.runtime_invocation`.
const RUNTIME_INVOCATION_PROP: &str = "urn:eigenius:institution:runtime_invocation";
/// Property IRI for `Verdict.diagnostic`.
const VERDICT_DIAGNOSTIC_PROP: &str = "urn:eigenius:institution:diagnostic";

/// Deterministic Verdict IRI when there's no companion RuntimeInvocation
/// to derive from (in-process dispatches). Uses the gated
/// subject's IRI plus a `:verdict` suffix — the verdict is a
/// deterministic function of the subject (no UUID indirection
/// required for institutions whose verdicts are reproducible from
/// the subject alone, like D52's statistics institution under its
/// decidable-recomputation contract). Re-running an AutoOnLoad-gated
/// commit against the same subject produces the same verdict IRI;
/// the chain's append-only discipline collapses re-emission to a
/// no-op when the verdict's content is also idempotent (which it
/// must be for any decidable institution).
fn fallback_verdict_iri(subject: &Iri) -> Iri {
    Iri::parse(&format!("{}:verdict", subject.as_str())).expect("derived verdict IRI parses")
}

/// Result of reading a Verdict off a result resource. Mirrors the
/// `parse_verdict` helper in `nbe::eval` but produces a typed
/// outcome rather than `DecResult` so the AutoOnLoad caller can
/// distinguish a malformed shape from an ordinary verdict.
#[derive(Debug, Clone)]
pub enum VerdictReading {
    Holds,
    Fails,
    Undecidable,
    Malformed(String),
}

impl VerdictReading {
    /// Inductive ctor name (`"Holds"`, `"Fails"`, `"Undecidable"`)
    /// matching the kernel's `parse_verdict` and the Julia mirror's
    /// codec convention. Panics for `Malformed` — that variant
    /// should never reach the commit pipeline.
    pub fn ctor_name(&self) -> &'static str {
        match self {
            VerdictReading::Holds => "Holds",
            VerdictReading::Fails => "Fails",
            VerdictReading::Undecidable => "Undecidable",
            VerdictReading::Malformed(_) => {
                unreachable!("malformed verdict should never reach the commit pipeline")
            }
        }
    }

    /// `true` for verdicts that pass the AutoOnLoad gate (`Holds`,
    /// `Undecidable`); `false` only for `Fails`.
    pub fn admits(&self) -> bool {
        matches!(self, VerdictReading::Holds | VerdictReading::Undecidable)
    }
}

fn parse_verdict(result: &Resource) -> VerdictReading {
    use crate::ontology::well_known as wk;

    if let Some(ctor) = result
        .get(&Iri::parse(wk::CTOR_NAME).expect("well-known IRI"))
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        return match ctor.as_str() {
            "Holds" => VerdictReading::Holds,
            "Fails" => VerdictReading::Fails,
            "Undecidable" => VerdictReading::Undecidable,
            other => VerdictReading::Malformed(format!("unknown ctor_name `{other}`")),
        };
    }
    for class_iri in result.is_a() {
        match class_iri.as_str() {
            "urn:eigenius:institution:verdicts:holds" => return VerdictReading::Holds,
            "urn:eigenius:institution:verdicts:fails" => return VerdictReading::Fails,
            "urn:eigenius:institution:verdicts:undecidable" => return VerdictReading::Undecidable,
            _ => {}
        }
    }
    VerdictReading::Malformed(format!(
        "result resource is_a={:?} carries no Verdict marker",
        result.is_a()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionMode;
    use crate::institution::error::InstitutionError;
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, QueryOutcome};
    use crate::layer::LayerBuilder;
    use crate::nbe::val::Val;
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Institution that returns a configurable Verdict-shaped result
    /// regardless of input. Used to drive the AutoOnLoad dispatch
    /// through every Verdict branch.
    struct VerdictStub {
        iri: Iri,
        verdict_class: &'static str,
    }

    impl Institution for VerdictStub {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!()
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!()
        }
        fn query(
            &self,
            _procedure_iri: &Iri,
            _input: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<QueryOutcome, InstitutionError> {
            let mut r = Resource::new_embedded();
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::String(self.verdict_class.into())]),
            );
            Ok(QueryOutcome::from_output(r))
        }
    }

    /// Build a chain with a single AutoOnLoad QueryClass on
    /// `urn:eigenius:test:auto:Subject`, plus a runtime registering
    /// the stub institution with the given verdict.
    fn build_dispatch_setup(
        verdict_class: &'static str,
    ) -> (
        Arc<InstitutionIndex>,
        Arc<InstitutionRuntime>,
        ExecutionContext,
    ) {
        let mut b = LayerBuilder::new("test", None);

        let inst_iri = "urn:eigenius:test:auto:inst";
        let qc_iri = "urn:eigenius:test:auto:check";
        let subject = "urn:eigenius:test:auto:Subject";

        let mut qc = Resource::new(iri(qc_iri));
        qc.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(iri(wk::QUERY_CLASS), Value::String(subject.into()));
        qc.set(iri(wk::RESULT_CLASS), Value::String(wk::VERDICT.into()));
        qc.set(
            iri(wk::DISPATCH_ROLE),
            Value::Array(vec![Value::String(wk::DISPATCH_AUTO_ON_LOAD.into())]),
        );
        qc.set(
            iri(wk::QUERY_HANDLER),
            Value::String("urn:eigenius:test:auto:proc:check".into()),
        );
        qc.set(
            iri("urn:eigenius:institution:institution_ref"),
            Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        let storage = crate::layer::LayerStorage::in_memory();
        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let idx = Arc::new(idx);

        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(VerdictStub {
                iri: iri(inst_iri),
                verdict_class,
            }))
            .unwrap();
        let runtime = Arc::new(runtime);

        let exec_ctx = ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly, storage);
        (idx, runtime, exec_ctx)
    }

    fn make_subject() -> Resource {
        let mut r = Resource::new(iri("urn:eigenius:test:auto:r1"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:auto:Subject".into())]),
        );
        r
    }

    #[test]
    fn auto_on_load_holds_produces_no_error() {
        let (idx, runtime, ctx) = build_dispatch_setup("urn:eigenius:institution:verdicts:holds");
        let outcome = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert!(
            outcome.errors.is_empty(),
            "Holds should produce no errors; got {:?}",
            outcome.errors
        );
        assert_eq!(outcome.dispatches.len(), 1);
        assert!(matches!(
            outcome.dispatches[0].verdict,
            VerdictReading::Holds
        ));
        assert!(outcome.dispatches[0].partial_invocation.is_none());
    }

    #[test]
    fn auto_on_load_undecidable_produces_no_error() {
        let (idx, runtime, ctx) =
            build_dispatch_setup("urn:eigenius:institution:verdicts:undecidable");
        let outcome = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.dispatches.len(), 1);
        assert!(matches!(
            outcome.dispatches[0].verdict,
            VerdictReading::Undecidable
        ));
    }

    #[test]
    fn auto_on_load_fails_lands_in_dispatches_not_errors() {
        let (idx, runtime, ctx) = build_dispatch_setup("urn:eigenius:institution:verdicts:fails");
        let outcome = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        // Fails verdicts are well-formed dispatches — they go into
        // `dispatches` so the commit pipeline can produce a Verdict
        // resource for the audit trail. The caller (the commit
        // pipeline's `autoonload_dispatch` phase) is responsible for
        // translating a Fails dispatch into a ValidationError
        // pointing at the Verdict IRI.
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.dispatches.len(), 1);
        assert!(matches!(
            outcome.dispatches[0].verdict,
            VerdictReading::Fails
        ));
    }

    /// AutoOnLoad's recursive resource-ref dereference (Phase 19h.2).
    /// The institution's `query` is given a subject that has a
    /// `core:resource`-typed property pointing to an IRI string —
    /// the kernel must inline the referenced resource as an
    /// `Value::Embedded` *before* the institution's handler runs,
    /// because external-runtime mirror decoders can't follow chain
    /// refs themselves.
    #[test]
    fn auto_on_load_inlines_iri_resource_refs_in_subject() {
        use std::sync::Mutex;

        // Capture the input the institution receives so we can assert
        // its shape post-marshal. Mutex (rather than Rc<RefCell>) so
        // the stub satisfies the `Send + Sync` bounds the institution
        // trait carries.
        struct CaptureStub {
            iri: Iri,
            captured: Arc<Mutex<Option<Resource>>>,
        }
        impl Institution for CaptureStub {
            fn institution_iri(&self) -> &Iri {
                &self.iri
            }
            fn extract_typed(
                &self,
                _: &Iri,
                _: &Resource,
                _: &ExecutionContext,
            ) -> Result<Val, InstitutionError> {
                unreachable!()
            }
            fn reify(
                &self,
                _: &Iri,
                _: &Val,
                _: &ExecutionContext,
            ) -> Result<Resource, InstitutionError> {
                unreachable!()
            }
            fn query(
                &self,
                _: &Iri,
                input: &Resource,
                _: &ExecutionContext,
            ) -> Result<QueryOutcome, InstitutionError> {
                *self.captured.lock().unwrap() = Some(input.clone());
                let mut r = Resource::new_embedded();
                r.set(
                    iri(wk::IS_A),
                    Value::Array(vec![Value::String(wk::VERDICT.into())]),
                );
                r.set(iri(wk::CTOR_NAME), Value::String(wk::VERDICT_HOLDS.into()));
                Ok(QueryOutcome::from_output(r))
            }
        }

        let mut b = LayerBuilder::new("test", None);

        let inst_iri = "urn:eigenius:test:deref:inst";
        let qc_iri = "urn:eigenius:test:deref:check";
        let subject_class = "urn:eigenius:test:deref:Subject";
        let inner_class = "urn:eigenius:test:deref:Inner";
        let nested_prop = "urn:eigenius:test:deref:nested";

        // Property declaration for the resource-typed reference. The
        // marshaler reads `data_type` to decide which properties to
        // dereference; without this declaration on chain the value
        // would pass through unchanged.
        let mut prop_decl = Resource::new(iri(nested_prop));
        prop_decl.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::PROPERTY.into())]),
        );
        prop_decl.set(iri(wk::DATA_TYPE_PROP), Value::String(wk::RESOURCE.into()));
        prop_decl.set(
            iri("urn:eigenius:core:class_types"),
            Value::Array(vec![Value::String(inner_class.into())]),
        );
        b.add_resource(prop_decl).unwrap();

        // Inner referenced resource.
        let mut inner = Resource::new(iri("urn:eigenius:test:deref:inner_x"));
        inner.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(inner_class.into())]),
        );
        inner.set(
            iri("urn:eigenius:test:deref:label"),
            Value::String("the-inner-payload".into()),
        );
        b.add_resource(inner).unwrap();

        // AutoOnLoad QueryClass on the subject class.
        let mut qc = Resource::new(iri(qc_iri));
        qc.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(iri(wk::QUERY_CLASS), Value::String(subject_class.into()));
        qc.set(iri(wk::RESULT_CLASS), Value::String(wk::VERDICT.into()));
        qc.set(
            iri(wk::DISPATCH_ROLE),
            Value::Array(vec![Value::String(wk::DISPATCH_AUTO_ON_LOAD.into())]),
        );
        qc.set(
            iri(wk::QUERY_HANDLER),
            Value::String("urn:eigenius:test:deref:proc:check".into()),
        );
        qc.set(
            iri("urn:eigenius:institution:institution_ref"),
            Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        let storage = crate::layer::LayerStorage::in_memory();
        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");

        let captured = Arc::new(Mutex::new(None));
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(CaptureStub {
                iri: iri(inst_iri),
                captured: Arc::clone(&captured),
            }))
            .unwrap();
        let exec_ctx = ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly, storage);

        // Subject carries `nested = "urn:eigenius:test:deref:inner_x"` as
        // an IRI string — the on-the-wire shape an ESL `nested = inner_x`
        // declaration produces.
        let mut subject = Resource::new(iri("urn:eigenius:test:deref:s1"));
        subject.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(subject_class.into())]),
        );
        subject.set(
            iri(nested_prop),
            Value::String("urn:eigenius:test:deref:inner_x".into()),
        );

        let outcome = dispatch_auto_on_load_for_resource(&subject, &idx, &runtime, &exec_ctx);
        assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
        assert_eq!(outcome.dispatches.len(), 1);

        // The captured input must carry an embedded inner resource —
        // the kernel dereferenced the IRI before dispatch.
        let captured = captured.lock().unwrap();
        let captured = captured.as_ref().expect("query was called");
        let nested_value = captured
            .get(&iri(nested_prop))
            .expect("nested property present on captured input");
        let Value::Embedded(inner_r) = nested_value else {
            panic!(
                "expected nested ref to be dereferenced into Value::Embedded; got {nested_value:?}"
            );
        };
        let label = inner_r
            .get(&iri("urn:eigenius:test:deref:label"))
            .expect("inner resource has its label");
        assert!(matches!(label, Value::String(s) if s == "the-inner-payload"));
    }

    #[test]
    fn auto_on_load_skips_resources_without_matching_class() {
        let (idx, runtime, ctx) = build_dispatch_setup("urn:eigenius:institution:verdicts:fails");
        // Resource of an unrelated class — no QueryClass binds to it.
        let mut r = Resource::new(iri("urn:eigenius:test:auto:r_unrelated"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Other".into())]),
        );
        let outcome = dispatch_auto_on_load_for_resource(&r, &idx, &runtime, &ctx);
        assert!(outcome.errors.is_empty(), "non-matching class skipped");
        assert!(outcome.dispatches.is_empty());
    }

    #[test]
    fn auto_on_load_for_layer_walks_all_resources() {
        let (idx, runtime, ctx) = build_dispatch_setup("urn:eigenius:institution:verdicts:fails");
        let mut b = LayerBuilder::new("test_data", None);
        b.add_resource(make_subject()).unwrap();
        let mut r2 = Resource::new(iri("urn:eigenius:test:auto:r2"));
        r2.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:auto:Subject".into())]),
        );
        b.add_resource(r2).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let outcome = dispatch_auto_on_load_for_layer(&layer, &idx, &runtime, &ctx);
        assert!(outcome.errors.is_empty());
        assert_eq!(
            outcome.dispatches.len(),
            2,
            "expected one Fails dispatch per Subject resource; got {:?}",
            outcome.dispatches
        );
    }

    #[test]
    fn malformed_verdict_surfaces_error() {
        // Stub returns a resource with no Verdict shape at all.
        struct BrokenStub {
            iri: Iri,
        }
        impl Institution for BrokenStub {
            fn institution_iri(&self) -> &Iri {
                &self.iri
            }
            fn extract_typed(
                &self,
                _: &Iri,
                _: &Resource,
                _: &ExecutionContext,
            ) -> Result<Val, InstitutionError> {
                unreachable!()
            }
            fn reify(
                &self,
                _: &Iri,
                _: &Val,
                _: &ExecutionContext,
            ) -> Result<Resource, InstitutionError> {
                unreachable!()
            }
            fn query(
                &self,
                _: &Iri,
                _: &Resource,
                _: &ExecutionContext,
            ) -> Result<QueryOutcome, InstitutionError> {
                Ok(QueryOutcome::from_output(Resource::new_embedded()))
            }
        }

        // Same chain shape as build_dispatch_setup but with a different
        // institution registered.
        let mut b = LayerBuilder::new("test", None);
        let mut qc = Resource::new(iri("urn:eigenius:test:auto:check"));
        qc.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            iri(wk::QUERY_CLASS),
            Value::String("urn:eigenius:test:auto:Subject".into()),
        );
        qc.set(iri(wk::RESULT_CLASS), Value::String(wk::VERDICT.into()));
        qc.set(
            iri(wk::DISPATCH_ROLE),
            Value::Array(vec![Value::String(wk::DISPATCH_AUTO_ON_LOAD.into())]),
        );
        qc.set(
            iri(wk::QUERY_HANDLER),
            Value::String("urn:eigenius:test:auto:proc:check".into()),
        );
        qc.set(
            iri("urn:eigenius:institution:institution_ref"),
            Value::String("urn:eigenius:test:auto:inst".into()),
        );
        b.add_resource(qc).unwrap();
        let storage = crate::layer::LayerStorage::in_memory();
        let layer = Arc::new(b.build(storage.clone()));
        let (idx, _) = InstitutionIndex::from_layer(&layer);
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(BrokenStub {
                iri: iri("urn:eigenius:test:auto:inst"),
            }))
            .unwrap();
        let ctx = ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly, storage);

        let outcome = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert_eq!(outcome.errors.len(), 1);
        assert!(
            outcome.errors[0].message.contains("non-Verdict"),
            "unexpected message: {}",
            outcome.errors[0].message
        );
        assert!(
            outcome.dispatches.is_empty(),
            "malformed Verdict yields no dispatch entry"
        );
    }
}
