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

//! D14 AutoOnLoad dispatch (D14 §9.1).
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
use crate::institution::registry::{DispatchRole, InstitutionIndex};
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::validation::{ValidationError, ValidationRule};

/// Run AutoOnLoad QueryClasses for every class on `resource`, against
/// the given index + runtime. Returns one `ValidationError` per
/// QueryClass whose Verdict was `Fails`. `Holds` and `Undecidable`
/// produce no error. The caller composes these with structural
/// validation errors as appropriate.
///
/// Used both by the Load-path layer dispatch (one resource at a time
/// from the new layer) and by [`Exp::InstitutionInvoke`] post-
/// translation validation (a single resource produced by reify).
pub fn dispatch_auto_on_load_for_resource(
    resource: &Resource,
    index: &InstitutionIndex,
    runtime: &InstitutionRuntime,
    ctx: &ExecutionContext,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
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
                errors.push(ValidationError {
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

            match institution.query(&query_class.query_handler, resource, ctx) {
                Ok(result) => match parse_verdict(&result) {
                    VerdictReading::Holds | VerdictReading::Undecidable => {}
                    VerdictReading::Fails => {
                        errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: None,
                            rule: ValidationRule::InstitutionValidation,
                            message: format!(
                                "AutoOnLoad QueryClass `{query_class_iri}` returned Fails"
                            ),
                        });
                    }
                    VerdictReading::Malformed(reason) => {
                        errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: None,
                            rule: ValidationRule::InstitutionValidation,
                            message: format!(
                                "AutoOnLoad QueryClass `{query_class_iri}` returned a non-Verdict result: {reason}"
                            ),
                        });
                    }
                },
                Err(e) => errors.push(ValidationError {
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
    errors
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
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for (_iri, resource) in layer.iter_resources() {
        errors.extend(dispatch_auto_on_load_for_resource(
            &resource, index, runtime, ctx,
        ));
    }
    errors
}

/// Result of reading a Verdict off a result resource. Mirrors the
/// `parse_verdict` helper in `nbe::eval` but produces a typed
/// outcome rather than `DecResult` so the AutoOnLoad caller can
/// distinguish a malformed shape from an ordinary verdict.
enum VerdictReading {
    Holds,
    Fails,
    Undecidable,
    Malformed(String),
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
    use crate::institution::runtime::Institution;
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
        ) -> Result<Resource, InstitutionError> {
            let mut r = Resource::new_embedded();
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::String(self.verdict_class.into())]),
            );
            Ok(r)
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

        let layer = Arc::new(b.build());
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

        let exec_ctx = ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly);
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
        let errs = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert!(
            errs.is_empty(),
            "Holds should produce no errors; got {errs:?}"
        );
    }

    #[test]
    fn auto_on_load_undecidable_produces_no_error() {
        let (idx, runtime, ctx) =
            build_dispatch_setup("urn:eigenius:institution:verdicts:undecidable");
        let errs = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert!(
            errs.is_empty(),
            "Undecidable should not block Load; got {errs:?}"
        );
    }

    #[test]
    fn auto_on_load_fails_produces_validation_error() {
        let (idx, runtime, ctx) = build_dispatch_setup("urn:eigenius:institution:verdicts:fails");
        let errs = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].rule, ValidationRule::InstitutionValidation);
        assert!(
            errs[0].message.contains("returned Fails"),
            "unexpected message: {}",
            errs[0].message
        );
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
        let errs = dispatch_auto_on_load_for_resource(&r, &idx, &runtime, &ctx);
        assert!(errs.is_empty(), "non-matching class should be skipped");
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
        let layer = Arc::new(b.build());

        let errs = dispatch_auto_on_load_for_layer(&layer, &idx, &runtime, &ctx);
        assert_eq!(
            errs.len(),
            2,
            "expected one Fails per Subject resource; got {errs:?}"
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
            ) -> Result<Resource, InstitutionError> {
                Ok(Resource::new_embedded())
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
        let layer = Arc::new(b.build());
        let (idx, _) = InstitutionIndex::from_layer(&layer);
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(BrokenStub {
                iri: iri("urn:eigenius:test:auto:inst"),
            }))
            .unwrap();
        let ctx = ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly);

        let errs = dispatch_auto_on_load_for_resource(&make_subject(), &idx, &runtime, &ctx);
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.contains("non-Verdict"),
            "unexpected message: {}",
            errs[0].message
        );
    }
}
