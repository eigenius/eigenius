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

//! Program model: typed expressions in the Eigon knowledge graph.
//!
//! Programs are composed of expression forms (Let, Apply, Var, Lambda,
//! Case, Pair, Construct, Project, Map, Reduce, Literal) that map 1:1
//! to EigenTT terms. See design doc D3 for the full specification.
//!
//! Execution is via NbE in IO mode (eval_io module).

pub mod axiom_env;
pub mod check_hooks;
pub mod component;
pub mod eigentt_type_mirror;
pub mod embedder;
pub mod embedding_cache;
pub mod eval_io;
pub mod expr;
pub mod ground;
pub mod remote;
pub mod schema;
pub mod trace;

#[cfg(test)]
mod tests {
    use crate::bootstrap;
    use crate::context::ExecutionContext;
    use crate::lattice::commit_layer_default;
    use crate::layer::{Layer, LayerStorage};
    use crate::nbe::check::{self, CheckCtx};
    use crate::nbe::env::Rho;
    use crate::nbe::eval;
    use crate::ontology::eigon_json;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::Value;
    use crate::program::component::ComponentRegistry;
    use crate::program::eval_io::execute_program_nbe;
    use crate::program::expr::parse_program;
    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::PersistentBackend;
    use std::sync::Arc;

    /// Bootstrap with a memory-backed persistent backend so the test
    /// can route layer commits through [`commit_layer_default`] — the
    /// D41 supported single-layer-commit surface. Returns the context
    /// alongside the backend so commit callers can hand it to
    /// `commit_layer_default`.
    fn bootstrap_with_memory_backend(
    ) -> Result<(ExecutionContext, Arc<MemoryPersistentBackend>), Box<dyn std::error::Error>> {
        let backend = Arc::new(MemoryPersistentBackend::new());
        let storage =
            LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
        let ctx = bootstrap::bootstrap_with_storage(storage)?;
        Ok((ctx, backend))
    }

    /// Commit the working layer through `commit_layer_default` and
    /// advance `ctx.head` to the new layer. D41 §11.
    fn commit_and_advance(
        ctx: &mut ExecutionContext,
        backend: &MemoryPersistentBackend,
        name: &str,
    ) -> Arc<Layer> {
        let working = ctx.take_working(name).expect("take_working");
        let layer = commit_layer_default(working, ctx.storage().clone(), backend)
            .expect("commit_layer_default");
        ctx.advance_head(Arc::clone(&layer), name)
            .expect("advance_head");
        layer
    }

    /// End-to-end: load a program from JSON, parse to EigenTT, execute via NbE.
    #[test]
    fn end_to_end_identity_program() -> Result<(), Box<dyn std::error::Error>> {
        let (mut ctx, backend) = bootstrap_with_memory_backend()?;

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        commit_and_advance(&mut ctx, &backend, "animals");

        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);

        // Parse to EigenTT terms. No type-check follows — see
        // `identity_program_does_not_type_check` for what
        // `check::check` returns on this program.
        let (term, _typ) = parse_program(&program, ctx.head()).unwrap();
        assert!(matches!(term, crate::nbe::term::Exp::Lam(_, _)));

        // Execute via NbE
        let mut input = crate::ontology::resource::Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );
        input.set(
            Iri::parse("urn:eigenius:example:breed").unwrap(),
            Value::String("German Shepherd".into()),
        );

        let layer = Arc::clone(ctx.head());
        let registry = Arc::new(ComponentRegistry::default());
        let result = execute_program_nbe(&program, &input, layer, registry, None)?;

        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(result.output.get(&name_iri).unwrap().as_str(), Some("Rex"));

        let breed_iri = Iri::parse("urn:eigenius:example:breed").unwrap();
        assert_eq!(
            result.output.get(&breed_iri).unwrap().as_str(),
            Some("German Shepherd")
        );
        Ok(())
    }

    /// The checker cannot type a program that calls a component, and
    /// this pins what it does instead.
    ///
    /// `parse_apply` encodes `program:function: <component IRI>` as
    /// `Exp::Var(<iri>)`, and `check_infer`'s `Var` arm resolves names
    /// in `Gamma` only — there is no path from a component IRI to the
    /// `component:input_type` / `component:output_type` slots the
    /// ontology declares. So the identity example, which runs
    /// correctly (`end_to_end_identity_program`), fails to check.
    ///
    /// This is why `ValidateProgram` reports `checks_performed`
    /// without `"type_check"` (issue #143). If this test starts
    /// failing because the checker learned to type components, the
    /// handler in `server/programs.rs` must be wired to
    /// `check::check` and must add `"type_check"` to the list.
    #[test]
    fn identity_program_does_not_type_check() -> Result<(), Box<dyn std::error::Error>> {
        let (mut ctx, backend) = bootstrap_with_memory_backend()?;

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        for r in eigon_json::parse_document(animals_json).unwrap() {
            ctx.add_resource(r).unwrap();
        }
        commit_and_advance(&mut ctx, &backend, "animals");

        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);
        let (term, typ) = parse_program(&program, ctx.head()).unwrap();

        let typ_val = eval::eval(&typ, &Rho::Nil)?;
        let mut check_ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(ctx.head()));
        let err = check::check(&mut check_ctx, &term, &typ_val)
            .expect_err("a component reference has no type in Gamma");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("unbound variable in type context")
                && msg.contains("urn:eigenius:program:components:Identity"),
            "expected the component IRI to be the unbound name, got {msg}"
        );
        Ok(())
    }

    /// End-to-end: program with let-binding, executed via NbE.
    #[test]
    fn end_to_end_let_program() {
        let (mut ctx, backend) = bootstrap_with_memory_backend().unwrap();

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        commit_and_advance(&mut ctx, &backend, "animals");

        let program_json = include_str!("../../../ontologies/examples/let-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);

        // Parse to EigenTT
        let (term, _typ) = parse_program(&program, ctx.head()).unwrap();
        assert!(matches!(term, crate::nbe::term::Exp::Lam(_, _)));

        // Execute via NbE
        let mut input = crate::ontology::resource::Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );
        input.set(
            Iri::parse("urn:eigenius:example:breed").unwrap(),
            Value::String("German Shepherd".into()),
        );

        let layer = Arc::clone(ctx.head());
        let registry = Arc::new(ComponentRegistry::default());
        let result = execute_program_nbe(&program, &input, layer, registry, None).unwrap();

        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(result.output.get(&name_iri).unwrap().as_str(), Some("Rex"));
    }

    /// End-to-end codata: compile an ESL file that declares a codata
    /// End-to-end: validate program parsing.
    #[test]
    fn end_to_end_cli_validate() {
        let (mut ctx, backend) = bootstrap_with_memory_backend().unwrap();

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        commit_and_advance(&mut ctx, &backend, "animals");

        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);
        let result = parse_program(&program, ctx.head());
        assert!(result.is_ok(), "program should parse successfully");
    }
}
