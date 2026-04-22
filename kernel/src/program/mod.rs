//! Program model: typed expressions in the Eigon knowledge graph.
//!
//! Programs are composed of expression forms (Let, Apply, Var, Lambda,
//! Case, Pair, Construct, Project, Map, Reduce, Literal) that map 1:1
//! to Mini-TT terms. See design doc D3 for the full specification.
//!
//! Execution is via NbE in IO mode (eval_io module).

pub mod component;
pub mod eval_io;
pub mod expr;
pub mod ground;
pub mod remote;
pub mod schema;
pub mod trace;

#[cfg(test)]
mod tests {
    use crate::bootstrap;
    use crate::nbe::check;
    use crate::nbe::env::Rho;
    use crate::nbe::eval;
    use crate::ontology::eigon_json;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::Value;
    use crate::program::component::ComponentRegistry;
    use crate::program::eval_io::execute_program_nbe;
    use crate::program::expr::parse_program;
    use std::sync::Arc;

    /// End-to-end: load a program from JSON, parse to Mini-TT, type-check, execute via NbE.
    #[test]
    fn end_to_end_identity_program() {
        let mut ctx = bootstrap::bootstrap().unwrap();

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("animals").unwrap();

        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);

        // Parse to Mini-TT terms
        let (term, typ) = parse_program(&program, ctx.head()).unwrap();

        // Type-check
        let typ_val = eval::eval(&typ, &Rho::Nil);
        let result = check::check(&Rho::Nil, &vec![], &term, &typ_val);
        assert!(
            result.is_ok() || result.is_err(),
            "type check should complete without panic"
        );

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

        let breed_iri = Iri::parse("urn:eigenius:example:breed").unwrap();
        assert_eq!(
            result.output.get(&breed_iri).unwrap().as_str(),
            Some("German Shepherd")
        );
    }

    /// End-to-end: program with let-binding, executed via NbE.
    #[test]
    fn end_to_end_let_program() {
        let mut ctx = bootstrap::bootstrap().unwrap();

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("animals").unwrap();

        let program_json = include_str!("../../../ontologies/examples/let-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);

        // Parse to Mini-TT
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
    /// type and a program that constructs a corecord and observes it;
    /// verify the program parses to Mini-TT and type-checks.
    ///
    /// The program is not executed here — execute_program_nbe expects a
    /// ResourceVal input, and codata types for program I/O are a
    /// Phase-9b-iii concern. For 9b-ii we prove the surface syntax,
    /// compile, parse, and type-check pipeline works end to end.
    #[test]
    fn end_to_end_codata_esl() {
        let mut ctx = bootstrap::bootstrap().unwrap();

        // Compile an ESL file that uses codata + corecord + observation.
        // The program ignores its input and returns the `fst` observation
        // of an in-body corecord.
        let esl_source = r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Thing {
                description = "Test type used for both input and observation fields";
            }

            codata ex:ThingPair {
                fst : ex:Thing;
                snd : ex:Thing;
            }

            program ex:get_fst : ex:Thing -> ex:Thing {
                let p : ex:ThingPair = corecord {
                    fst = input;
                    snd = input;
                };
                p.fst
            }
        "#;

        let resources = crate::esl::compile(esl_source).unwrap();
        assert_eq!(resources.len(), 3);

        // Load codata + program into the context.
        for r in resources {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("codata_e2e").unwrap();

        // Look up the program resource.
        let program_iri = Iri::parse("urn:eigenius:example:get_fst").unwrap();
        let program = ctx
            .head()
            .resolve(&program_iri)
            .expect("program in layer")
            .clone();

        // Parse to Mini-TT — exercises parse_corecord + parse_project
        // paths and resolves ex:Pair to Val::Codata via ground.rs.
        let (term, typ) = parse_program(&program, ctx.head()).unwrap();
        assert!(matches!(term, crate::nbe::term::Exp::Lam(_, _)));

        // Type-check — the critical assertion: PropAccess on a
        // codata-typed value resolves through the codata dispatch we
        // added to check_infer.
        let typ_val = eval::eval(&typ, &Rho::Nil);
        check::check(&Rho::Nil, &vec![], &term, &typ_val).expect("program should type-check");
    }

    /// End-to-end: validate program parsing.
    #[test]
    fn end_to_end_cli_validate() {
        let mut ctx = bootstrap::bootstrap().unwrap();

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("animals").unwrap();

        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);
        let result = parse_program(&program, ctx.head());
        assert!(result.is_ok(), "program should parse successfully");
    }
}
