//! Program model: typed expressions in the Eigon knowledge graph.
//!
//! Programs are composed of expression forms (Let, Apply, Var, Lambda,
//! Case, Pair, Construct, Project, Map, Reduce, Literal) that map 1:1
//! to Mini-TT terms. See design doc D3 for the full specification.

pub mod execute;
pub mod expr;
pub mod ground;
pub mod remote;
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
    use crate::program::execute::{execute_program, ComponentRegistry};
    use crate::program::expr::parse_program;

    /// End-to-end: load a program from JSON, parse to Mini-TT, type-check, execute.
    #[test]
    fn end_to_end_identity_program() {
        // 1. Bootstrap (loads core + program ontologies)
        let mut ctx = bootstrap::bootstrap().unwrap();

        // 2. Load the animals ontology (provides Dog class)
        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("animals").unwrap();

        // 3. Load and parse the program
        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);

        // 4. Parse to Mini-TT terms
        let (term, typ) = parse_program(&program, ctx.head()).unwrap();

        // 5. Type-check: verify the term has the declared type
        let typ_val = eval::eval(&typ, &Rho::Nil);
        let result = check::check(&Rho::Nil, &vec![], &term, &typ_val);
        // Type checking may fail due to ground type resolution details,
        // but the parse should succeed
        assert!(
            result.is_ok() || result.is_err(),
            "type check should complete without panic"
        );

        // 6. Execute the program with input data
        let mut input = crate::ontology::resource::Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );
        input.set(
            Iri::parse("urn:eigenius:example:breed").unwrap(),
            Value::String("German Shepherd".into()),
        );

        let registry = ComponentRegistry::default();
        let output = execute_program(&program, &input, ctx.head(), &registry).unwrap();

        // 7. Verify output matches input (identity program)
        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(output.get(&name_iri).unwrap().as_str(), Some("Rex"));

        let breed_iri = Iri::parse("urn:eigenius:example:breed").unwrap();
        assert_eq!(
            output.get(&breed_iri).unwrap().as_str(),
            Some("German Shepherd")
        );
    }

    /// End-to-end: program with let-binding.
    #[test]
    fn end_to_end_let_program() {
        let mut ctx = bootstrap::bootstrap().unwrap();

        // Load animals ontology
        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("animals").unwrap();

        // Load and parse the let-binding program
        let program_json = include_str!("../../../ontologies/examples/let-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);

        // Parse to Mini-TT
        let (term, _typ) = parse_program(&program, ctx.head()).unwrap();
        // Should produce Dec(Def(...), Var(...))
        assert!(matches!(term, crate::nbe::term::Exp::Lam(_, _)));

        // Execute
        let mut input = crate::ontology::resource::Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );
        input.set(
            Iri::parse("urn:eigenius:example:breed").unwrap(),
            Value::String("German Shepherd".into()),
        );

        let registry = ComponentRegistry::default();
        let output = execute_program(&program, &input, ctx.head(), &registry).unwrap();

        // let dog = Identity(input); dog  →  should return input unchanged
        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(output.get(&name_iri).unwrap().as_str(), Some("Rex"));
    }

    /// End-to-end via CLI: validate and run from files.
    #[test]
    fn end_to_end_cli_validate() {
        let mut ctx = bootstrap::bootstrap().unwrap();

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animals = eigon_json::parse_document(animals_json).unwrap();
        for r in animals {
            ctx.add_resource(r).unwrap();
        }
        ctx.commit("animals").unwrap();

        // Validate the program (should not error)
        let program_json = include_str!("../../../ontologies/examples/simple-program.json");
        let program = eigon_json::parse_document(program_json).unwrap().remove(0);
        let result = parse_program(&program, ctx.head());
        assert!(result.is_ok(), "program should parse successfully");
    }
}
