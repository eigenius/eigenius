//! ESL compiler: AST → Eigon-JSON resources.
//!
//! Walks the AST and produces a Vec<Resource> that can be
//! serialized to Eigon-JSON or loaded directly into the kernel.
//! Namespace aliases are resolved to full IRIs.

use crate::esl::ast;
use crate::esl::error::EslError;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::collections::BTreeMap;

/// Compile an ESL AST to Eigon-JSON resources.
pub fn compile_file(file: &ast::File) -> Result<Vec<Resource>, Vec<EslError>> {
    let mut compiler = Compiler::new();

    // Register namespace aliases.
    for ns in &file.namespaces {
        compiler.namespaces.insert(ns.alias.clone(), ns.uri.clone());
    }

    // First pass: collect every declared inductive constructor so that
    // bare-name references in expression position resolve to the
    // canonical ctor IRI (Phase 11b step 9). Conflicts within a file
    // are caught here rather than at use time.
    if let Err(e) = compiler.collect_ctor_table(file) {
        return Err(vec![e]);
    }

    let mut errors = Vec::new();
    let mut resources = Vec::new();

    for decl in &file.declarations {
        match compiler.compile_declaration(decl) {
            Ok(mut rs) => resources.append(&mut rs),
            Err(e) => errors.push(e),
        }
    }

    if errors.is_empty() {
        Ok(resources)
    } else {
        Err(errors)
    }
}

struct Compiler {
    namespaces: BTreeMap<String, String>,
    /// Per-file constructor table: short ctor name → full ctor IRI.
    /// Built in `collect_ctor_table` before any declaration is compiled,
    /// so expression compilation can resolve bare ctor references.
    ctors: BTreeMap<String, String>,
}

impl Compiler {
    fn new() -> Self {
        Self {
            namespaces: BTreeMap::new(),
            ctors: BTreeMap::new(),
        }
    }

    /// Walk every `data` declaration in the file and register its
    /// constructors in the ctor table. Each ctor's IRI is derived from
    /// the parent inductive's IRI plus its local name (`urn:…:Nat:succ`).
    /// Duplicate ctor names within a file are an error.
    fn collect_ctor_table(&mut self, file: &ast::File) -> Result<(), EslError> {
        for decl in &file.declarations {
            if let ast::Declaration::Data(d) = decl {
                let parent_iri = self.resolve(&d.name)?;
                for ctor in &d.ctors {
                    let ctor_iri = format!("{parent_iri}:{}", ctor.name);
                    if let Some(existing) = self.ctors.insert(ctor.name.clone(), ctor_iri) {
                        return Err(EslError::compiler(
                            Some(ctor.pos.clone()),
                            format!(
                                "constructor `{}` declared in `{}` collides with an earlier \
                                 declaration whose IRI is `{existing}` — rename one of them \
                                 (qualified ctor references in source are a future addition)",
                                ctor.name, parent_iri
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve a qualified name to a full IRI string.
    fn resolve(&self, qn: &ast::QualifiedName) -> Result<String, EslError> {
        match &qn.namespace {
            Some(ns) => match self.namespaces.get(ns) {
                Some(uri) => Ok(format!("{uri}:{}", qn.name)),
                None => Err(EslError::compiler(
                    Some(qn.pos.clone()),
                    format!("unknown namespace alias: '{ns}'"),
                )),
            },
            None => Err(EslError::compiler(
                Some(qn.pos.clone()),
                format!(
                    "bare name '{}' has no namespace — use a qualified name like ns:{}",
                    qn.name, qn.name
                ),
            )),
        }
    }

    /// Resolve a qualified name to an Iri.
    fn resolve_iri(&self, qn: &ast::QualifiedName) -> Result<Iri, EslError> {
        let s = self.resolve(qn)?;
        Iri::parse(&s).map_err(|e| {
            EslError::compiler(Some(qn.pos.clone()), format!("invalid IRI '{s}': {e}"))
        })
    }

    fn compile_declaration(&self, decl: &ast::Declaration) -> Result<Vec<Resource>, EslError> {
        match decl {
            ast::Declaration::Class(c) => self.compile_class(c),
            ast::Declaration::Property(p) => self.compile_property(p),
            ast::Declaration::Resource(r) => self.compile_resource(r),
            ast::Declaration::Program(p) => self.compile_program(p),
            ast::Declaration::Codata(c) => self.compile_codata(c),
            ast::Declaration::Data(d) => self.compile_data(d),
        }
    }

    // --- Codata ---

    fn compile_codata(&self, decl: &ast::CodataDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:core:CodataType".to_string(),
            )]),
        );
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(decl.name.name.clone()),
        );

        let mut observations = Vec::new();
        for obs in &decl.observations {
            let type_iri = self.resolve(&obs.typ)?;
            let mut obs_r = Resource::new_embedded();
            set_is_a(&mut obs_r, "urn:eigenius:core:Observation");
            obs_r.set(
                iri("urn:eigenius:core:observation_name"),
                Value::String(obs.name.clone()),
            );
            obs_r.set(
                iri("urn:eigenius:core:observation_type"),
                Value::String(type_iri),
            );
            observations.push(Value::Embedded(Box::new(obs_r)));
        }
        r.set(
            iri("urn:eigenius:core:observations"),
            Value::Array(observations),
        );

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    // --- Data (Phase 11b step 8, D19 §10) ---

    /// Compile a `data` declaration to an `InductiveType` resource.
    ///
    /// The resource shape is documented in
    /// [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json):
    /// embedded `InductiveParam` resources for type parameters and
    /// embedded `InductiveCtor` resources for constructors, each with
    /// embedded `InductiveArgType` resources for arg types.
    ///
    /// Argument-type names that match a declared parameter are
    /// recorded as bare names; everything else is resolved through
    /// the namespace table to a class IRI. Phase 11b step 8b will
    /// decode this back into an `Arc<InductiveDecl>` for use by the
    /// kernel.
    fn compile_data(&self, decl: &ast::DataDecl) -> Result<Vec<Resource>, EslError> {
        use crate::ontology::well_known as wk;

        let id = self.resolve_iri(&decl.name)?;
        let mut r = Resource::new(id);
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::INDUCTIVE_TYPE.to_string())]),
        );
        r.set(iri(wk::SHORT_NAME), Value::String(decl.name.name.clone()));

        let param_names: std::collections::HashSet<&str> =
            decl.params.iter().map(|p| p.name.as_str()).collect();

        let params: Result<Vec<Value>, EslError> = decl
            .params
            .iter()
            .map(|p| {
                let mut pr = Resource::new_embedded();
                set_is_a(&mut pr, wk::INDUCTIVE_PARAM);
                pr.set(iri(wk::PARAM_NAME), Value::String(p.name.clone()));
                let kind = self.resolve(&p.kind)?;
                pr.set(iri(wk::PARAM_KIND), Value::String(kind));
                Ok(Value::Embedded(Box::new(pr)))
            })
            .collect();
        r.set(iri(wk::TYPE_PARAMS), Value::Array(params?));

        let parent_iri_str = self.resolve(&decl.name)?;
        let ctors: Result<Vec<Value>, EslError> = decl
            .ctors
            .iter()
            .map(|c| {
                let ctor_iri_str = format!("{parent_iri_str}:{}", c.name);
                let ctor_iri = Iri::parse(&ctor_iri_str).map_err(|e| {
                    EslError::compiler(
                        Some(c.pos.clone()),
                        format!("invalid ctor IRI `{ctor_iri_str}`: {e}"),
                    )
                })?;
                let mut cr = Resource::new(ctor_iri);
                set_is_a(&mut cr, wk::INDUCTIVE_CTOR);
                cr.set(iri(wk::CTOR_NAME), Value::String(c.name.clone()));
                let arg_types: Result<Vec<Value>, EslError> = c
                    .args
                    .iter()
                    .map(|a| self.compile_ctor_arg_type(a, &param_names))
                    .collect();
                cr.set(iri(wk::ARG_TYPES), Value::Array(arg_types?));
                Ok(Value::Embedded(Box::new(cr)))
            })
            .collect();
        r.set(iri(wk::CTORS), Value::Array(ctors?));

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    /// Compile a constructor argument type to an embedded
    /// `InductiveArgType` resource.
    ///
    /// Bare references that match a declared parameter name are kept
    /// as the bare string (so the decoder can recognise them as
    /// parameter substitutions). Everything else must namespace-resolve
    /// to a class IRI.
    fn compile_ctor_arg_type(
        &self,
        arg: &ast::CtorArgType,
        params: &std::collections::HashSet<&str>,
    ) -> Result<Value, EslError> {
        use crate::ontology::well_known as wk;
        let mut ar = Resource::new_embedded();
        set_is_a(&mut ar, wk::INDUCTIVE_ARG_TYPE);

        let type_name = if arg.name.namespace.is_none() && params.contains(arg.name.name.as_str()) {
            arg.name.name.clone()
        } else {
            self.resolve(&arg.name)?
        };
        ar.set(iri(wk::TYPE_NAME), Value::String(type_name));

        let type_args: Result<Vec<Value>, EslError> = arg
            .params
            .iter()
            .map(|p| self.compile_ctor_arg_type(p, params))
            .collect();
        ar.set(iri(wk::TYPE_ARGS), Value::Array(type_args?));

        Ok(Value::Embedded(Box::new(ar)))
    }

    // --- Class ---

    fn compile_class(&self, class: &ast::ClassDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&class.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".to_string())]),
        );

        // short_name from the local part of the qualified name
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(class.name.name.clone()),
        );

        // subclass_of
        if let Some(parent) = &class.parent {
            let parent_iri = self.resolve(parent)?;
            r.set(
                iri("urn:eigenius:core:subclass_of"),
                Value::Array(vec![Value::String(parent_iri)]),
            );
        }

        for item in &class.body {
            match item {
                ast::ClassItem::Description(s) => {
                    r.set(
                        iri("urn:eigenius:core:description"),
                        Value::String(s.clone()),
                    );
                }
                ast::ClassItem::Requires(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:requires"), Value::Array(iris?));
                }
                ast::ClassItem::Recommends(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:recommends"), Value::Array(iris?));
                }
            }
        }

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    // --- Property ---

    fn compile_property(&self, prop: &ast::PropertyDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&prop.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:core:Property".to_string(),
            )]),
        );

        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(prop.name.name.clone()),
        );

        let dt = self.resolve(&prop.data_type)?;
        r.set(iri("urn:eigenius:core:data_type"), Value::String(dt));

        for item in &prop.body {
            match item {
                ast::PropertyItem::Description(s) => {
                    r.set(
                        iri("urn:eigenius:core:description"),
                        Value::String(s.clone()),
                    );
                }
                ast::PropertyItem::MinValue(v) => {
                    if *v == (*v as i64) as f64 {
                        r.set(
                            iri("urn:eigenius:core:min_value"),
                            Value::Integer(*v as i64),
                        );
                    } else {
                        r.set(iri("urn:eigenius:core:min_value"), Value::Float(*v));
                    }
                }
                ast::PropertyItem::MaxValue(v) => {
                    if *v == (*v as i64) as f64 {
                        r.set(
                            iri("urn:eigenius:core:max_value"),
                            Value::Integer(*v as i64),
                        );
                    } else {
                        r.set(iri("urn:eigenius:core:max_value"), Value::Float(*v));
                    }
                }
                ast::PropertyItem::MinLength(v) => {
                    r.set(iri("urn:eigenius:core:min_length"), Value::Integer(*v));
                }
                ast::PropertyItem::MaxLength(v) => {
                    r.set(iri("urn:eigenius:core:max_length"), Value::Integer(*v));
                }
                ast::PropertyItem::Pattern(s) => {
                    r.set(iri("urn:eigenius:core:pattern"), Value::String(s.clone()));
                }
                ast::PropertyItem::Format(f) => {
                    let fmt = self.resolve(f)?;
                    r.set(iri("urn:eigenius:core:format"), Value::String(fmt));
                }
                ast::PropertyItem::AllowsOnly(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:allows_only"), Value::Array(iris?));
                }
                ast::PropertyItem::Domain(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:domain"), Value::Array(iris?));
                }
                ast::PropertyItem::ClassTypes(names) => {
                    let iris: Result<Vec<Value>, _> = names
                        .iter()
                        .map(|n| self.resolve(n).map(Value::String))
                        .collect();
                    r.set(iri("urn:eigenius:core:class_types"), Value::Array(iris?));
                }
                ast::PropertyItem::ElementType(t) => {
                    let et = self.resolve(t)?;
                    r.set(iri("urn:eigenius:core:element_type"), Value::String(et));
                }
            }
        }

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    // --- Resource ---

    fn compile_resource(&self, res: &ast::ResourceDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&res.name)?;
        let mut r = Resource::new(id);

        let class_iri = self.resolve(&res.class)?;
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(class_iri)]),
        );

        for field in &res.body {
            let prop_iri = self.resolve_iri(&field.property)?;
            let value = self.compile_value(&field.value)?;
            r.set(prop_iri, value);
        }

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    fn compile_value(&self, value: &ast::Value) -> Result<Value, EslError> {
        match value {
            ast::Value::String(s) => Ok(Value::String(s.clone())),
            ast::Value::Int(n) => Ok(Value::Integer(*n)),
            ast::Value::Float(f) => Ok(Value::Float(*f)),
            ast::Value::Bool(b) => Ok(Value::Boolean(*b)),
            ast::Value::Ref(qn) => {
                let s = self.resolve(qn)?;
                Ok(Value::String(s))
            }
            ast::Value::Array(items) => {
                let compiled: Result<Vec<_>, _> =
                    items.iter().map(|v| self.compile_value(v)).collect();
                Ok(Value::Array(compiled?))
            }
            ast::Value::Block(fields) => {
                let mut embedded = Resource::new_embedded();
                for field in fields {
                    let prop_iri = self.resolve_iri(&field.property)?;
                    let val = self.compile_value(&field.value)?;
                    embedded.set(prop_iri, val);
                }
                Ok(Value::Embedded(Box::new(embedded)))
            }
        }
    }

    // --- Program ---

    fn compile_program(&self, prog: &ast::ProgramDecl) -> Result<Vec<Resource>, EslError> {
        let id = self.resolve_iri(&prog.name)?;
        let mut r = Resource::new(id);

        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String(
                "urn:eigenius:program:Program".to_string(),
            )]),
        );

        let input_type = self.resolve(&prog.input_type)?;
        r.set(
            iri("urn:eigenius:program:input_type"),
            Value::String(input_type),
        );

        let output_type = self.resolve(&prog.output_type)?;
        r.set(
            iri("urn:eigenius:program:output_type"),
            Value::String(output_type),
        );

        for attr in &prog.attributes {
            match attr {
                ast::ProgramAttribute::Description(s) => {
                    r.set(
                        iri("urn:eigenius:core:description"),
                        Value::String(s.clone()),
                    );
                }
            }
        }

        let body = self.compile_expr(&prog.body)?;
        r.set(
            iri("urn:eigenius:program:body"),
            Value::Embedded(Box::new(body)),
        );

        stamp_declared(&mut r);
        Ok(vec![r])
    }

    // --- Expression compilation ---

    fn compile_expr(&self, expr: &ast::Expr) -> Result<Resource, EslError> {
        match expr {
            ast::Expr::Let {
                name,
                typ,
                value,
                body,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Let");
                r.set(
                    iri("urn:eigenius:program:name"),
                    Value::String(name.clone()),
                );
                let type_iri = self.resolve(typ)?;
                r.set(iri("urn:eigenius:program:type"), Value::String(type_iri));
                let value_r = self.compile_expr(value)?;
                r.set(
                    iri("urn:eigenius:program:value"),
                    Value::Embedded(Box::new(value_r)),
                );
                let body_r = self.compile_expr(body)?;
                r.set(
                    iri("urn:eigenius:program:body"),
                    Value::Embedded(Box::new(body_r)),
                );
                Ok(r)
            }

            ast::Expr::Apply {
                function,
                argument,
                component_argument,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Apply");

                // Resolve function name. Order:
                // 1. Bare name matching a declared inductive ctor → ctor IRI
                //    (Phase 11b step 9). Takes precedence over component
                //    shorthand so user ctors can never be accidentally
                //    routed through the component dispatcher.
                // 2. Qualified name → namespace-resolved IRI.
                // 3. Bare name with no ctor match → component shorthand.
                let func_iri = if function.namespace.is_none() {
                    if let Some(ctor_iri) = self.ctors.get(&function.name) {
                        ctor_iri.clone()
                    } else {
                        format!("urn:eigenius:program:components:{}", function.name)
                    }
                } else {
                    self.resolve(function)?
                };
                r.set(
                    iri("urn:eigenius:program:function"),
                    Value::String(func_iri),
                );

                let arg_r = self.compile_expr(argument)?;
                r.set(
                    iri("urn:eigenius:program:argument"),
                    Value::Embedded(Box::new(arg_r)),
                );

                if let Some(comp_arg) = component_argument {
                    let comp_arg_r = self.compile_expr(comp_arg)?;
                    r.set(
                        iri("urn:eigenius:program:component_argument"),
                        Value::Embedded(Box::new(comp_arg_r)),
                    );
                }

                Ok(r)
            }

            ast::Expr::Var { name, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Var");
                // Bare name matching a declared ctor → ctor IRI as the
                // var name (Phase 11b step 9). The expression builder
                // recognises the IRI shape and produces an
                // `Exp::InductiveCtor` with no arguments.
                let resolved = self
                    .ctors
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                r.set(iri("urn:eigenius:program:name"), Value::String(resolved));
                Ok(r)
            }

            ast::Expr::Lambda { param, body, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Lambda");
                r.set(
                    iri("urn:eigenius:program:parameter"),
                    Value::String(param.clone()),
                );
                let body_r = self.compile_expr(body)?;
                r.set(
                    iri("urn:eigenius:program:body"),
                    Value::Embedded(Box::new(body_r)),
                );
                Ok(r)
            }

            ast::Expr::Case {
                scrutinee,
                branches,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Case");
                let scrut_r = self.compile_expr(scrutinee)?;
                r.set(
                    iri("urn:eigenius:program:scrutinee"),
                    Value::Embedded(Box::new(scrut_r)),
                );
                let mut branch_resources = Vec::new();
                for (constructor, body) in branches {
                    let mut br = Resource::new_embedded();
                    set_is_a(&mut br, "urn:eigenius:program:Branch");
                    br.set(
                        iri("urn:eigenius:program:constructor"),
                        Value::String(constructor.clone()),
                    );
                    let body_r = self.compile_expr(body)?;
                    br.set(
                        iri("urn:eigenius:program:body"),
                        Value::Embedded(Box::new(body_r)),
                    );
                    branch_resources.push(Value::Embedded(Box::new(br)));
                }
                r.set(
                    iri("urn:eigenius:program:branches"),
                    Value::Array(branch_resources),
                );
                Ok(r)
            }

            ast::Expr::ConstructExpr { class, fields, .. } => {
                // Anonymous block (empty class name) — used for component arguments.
                // Emit a plain embedded resource with resolved keys and data values.
                // Unlike expression compilation, qualified names here resolve to
                // IRI strings (data references), not variable references.
                if class.name.is_empty() {
                    let mut r = Resource::new_embedded();
                    for (prop, expr) in fields {
                        let prop_iri = self.resolve_iri(prop)?;
                        let val = self.compile_block_value(expr)?;
                        r.set(prop_iri, val);
                    }
                    return Ok(r);
                }

                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Construct");
                let class_iri = self.resolve(class)?;
                r.set(iri("urn:eigenius:program:class"), Value::String(class_iri));
                let mut fields_r = Resource::new_embedded();
                for (prop, expr) in fields {
                    let prop_iri = match self.resolve(prop) {
                        Ok(iri_str) => Iri::parse(&iri_str).map_err(|e| {
                            EslError::compiler(Some(prop.pos.clone()), format!("{e}"))
                        })?,
                        Err(_) => {
                            return Err(EslError::compiler(
                                Some(prop.pos.clone()),
                                format!("field name '{}' needs a namespace qualifier", prop.name),
                            ));
                        }
                    };
                    let expr_r = self.compile_expr(expr)?;
                    fields_r.set(prop_iri, Value::Embedded(Box::new(expr_r)));
                }
                r.set(
                    iri("urn:eigenius:program:fields"),
                    Value::Embedded(Box::new(fields_r)),
                );
                Ok(r)
            }

            ast::Expr::Project { expr, property, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Project");
                let expr_r = self.compile_expr(expr)?;
                r.set(
                    iri("urn:eigenius:program:expression"),
                    Value::Embedded(Box::new(expr_r)),
                );
                // Bare names are treated as codata observation names
                // (D11 §8) and emitted under a synthetic URN so the
                // resulting IRI's `local_name()` returns the bare name.
                // Namespaced names resolve to full IRIs as before.
                let prop_iri = match &property.namespace {
                    Some(_) => self.resolve(property)?,
                    None => format!("urn:eigenius:_obs:{}", property.name),
                };
                r.set(
                    iri("urn:eigenius:program:property"),
                    Value::String(prop_iri),
                );
                Ok(r)
            }

            ast::Expr::MapExpr {
                function,
                collection,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Map");
                let func_r = self.compile_expr(function)?;
                r.set(
                    iri("urn:eigenius:program:function"),
                    Value::Embedded(Box::new(func_r)),
                );
                let coll_r = self.compile_expr(collection)?;
                r.set(
                    iri("urn:eigenius:program:collection"),
                    Value::Embedded(Box::new(coll_r)),
                );
                Ok(r)
            }

            ast::Expr::ReduceExpr {
                function,
                initial,
                collection,
                ..
            } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Reduce");
                let func_r = self.compile_expr(function)?;
                r.set(
                    iri("urn:eigenius:program:function"),
                    Value::Embedded(Box::new(func_r)),
                );
                let init_r = self.compile_expr(initial)?;
                r.set(
                    iri("urn:eigenius:program:initial"),
                    Value::Embedded(Box::new(init_r)),
                );
                let coll_r = self.compile_expr(collection)?;
                r.set(
                    iri("urn:eigenius:program:collection"),
                    Value::Embedded(Box::new(coll_r)),
                );
                Ok(r)
            }

            ast::Expr::Pair { first, second, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Pair");
                let first_r = self.compile_expr(first)?;
                r.set(
                    iri("urn:eigenius:program:first"),
                    Value::Embedded(Box::new(first_r)),
                );
                let second_r = self.compile_expr(second)?;
                r.set(
                    iri("urn:eigenius:program:second"),
                    Value::Embedded(Box::new(second_r)),
                );
                Ok(r)
            }

            ast::Expr::Literal { value, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:Literal");
                let v = match value {
                    ast::LiteralValue::String(s) => Value::String(s.clone()),
                    ast::LiteralValue::Int(n) => Value::Integer(*n),
                    ast::LiteralValue::Float(f) => Value::Float(*f),
                    ast::LiteralValue::Bool(b) => Value::Boolean(*b),
                };
                r.set(iri("urn:eigenius:program:value"), v);
                Ok(r)
            }

            ast::Expr::CoRecord { fields, .. } => {
                let mut r = Resource::new_embedded();
                set_is_a(&mut r, "urn:eigenius:program:CoRecord");
                let mut cofields = Vec::new();
                for f in fields {
                    let body_r = self.compile_expr(&f.body)?;
                    let mut cf = Resource::new_embedded();
                    set_is_a(&mut cf, "urn:eigenius:program:CoField");
                    cf.set(
                        iri("urn:eigenius:program:observation_name"),
                        Value::String(f.name.clone()),
                    );
                    cf.set(
                        iri("urn:eigenius:program:body"),
                        Value::Embedded(Box::new(body_r)),
                    );
                    cofields.push(Value::Embedded(Box::new(cf)));
                }
                r.set(iri("urn:eigenius:program:cofields"), Value::Array(cofields));
                Ok(r)
            }
        }
    }

    /// Compile a block value expression to a resource Value.
    ///
    /// Unlike `compile_expr`, this treats qualified names as IRI string
    /// references (data), not as variable references (code). Used for
    /// component argument blocks where `patent:PatentAnalysis` means
    /// the IRI string, not a program variable.
    fn compile_block_value(&self, expr: &ast::Expr) -> Result<Value, EslError> {
        match expr {
            ast::Expr::Literal { value, .. } => match value {
                ast::LiteralValue::String(s) => Ok(Value::String(s.clone())),
                ast::LiteralValue::Int(n) => Ok(Value::Integer(*n)),
                ast::LiteralValue::Float(f) => Ok(Value::Float(*f)),
                ast::LiteralValue::Bool(b) => Ok(Value::Boolean(*b)),
            },
            ast::Expr::Var { name, pos } => {
                // Resolve qualified name to IRI string
                let qn = ast::QualifiedName {
                    namespace: if name.contains(':') {
                        Some(name.split(':').next().unwrap().to_string())
                    } else {
                        None
                    },
                    name: if name.contains(':') {
                        name.split(':').nth(1).unwrap().to_string()
                    } else {
                        name.clone()
                    },
                    pos: pos.clone(),
                };
                let iri_str = self.resolve(&qn)?;
                Ok(Value::String(iri_str))
            }
            ast::Expr::ConstructExpr { class, fields, .. } if class.name.is_empty() => {
                // Nested block — recurse
                let mut r = Resource::new_embedded();
                for (prop, inner_expr) in fields {
                    let prop_iri = self.resolve_iri(prop)?;
                    let val = self.compile_block_value(inner_expr)?;
                    r.set(prop_iri, val);
                }
                Ok(Value::Embedded(Box::new(r)))
            }
            _ => {
                // Fall back to expression compilation for complex cases
                let expr_r = self.compile_expr(expr)?;
                Ok(extract_literal_value(&expr_r))
            }
        }
    }
}

// --- Helpers ---

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI must be valid")
}

/// Extract the value from a compiled expression resource.
/// If it's a Literal (has urn:eigenius:program:value), return the value directly.
/// If it's an anonymous block (no is_a), return as embedded resource.
/// Otherwise wrap as embedded.
fn extract_literal_value(resource: &Resource) -> Value {
    // Check for literal value
    if let Some(val) = resource.get(&iri("urn:eigenius:program:value")) {
        return val.clone();
    }
    // Return as embedded resource
    Value::Embedded(Box::new(resource.clone()))
}

fn set_is_a(resource: &mut Resource, class_iri: &str) {
    resource.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String(class_iri.to_string())]),
    );
}

/// Append `DeclaredResource` to `is_a` and set `declared_by` on a
/// compiled resource (D6b epistemic stamping, Phase 10b Step 3).
fn stamp_declared(resource: &mut Resource) {
    let is_a_iri = iri("urn:eigenius:core:is_a");
    let mut types = match resource.get(&is_a_iri) {
        Some(Value::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    types.push(Value::String(
        crate::ontology::well_known::DECLARED_RESOURCE.to_string(),
    ));
    resource.set(is_a_iri, Value::Array(types));
    resource.set(
        iri(crate::ontology::well_known::DECLARED_BY),
        Value::String("esl-compiler".to_string()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esl;
    use crate::ontology::eigon_json;

    fn compile_esl(input: &str) -> Vec<Resource> {
        esl::compile(input).unwrap()
    }

    #[test]
    fn compile_class() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Document {
                description = "A text document";
                requires ex:text;
            }
        "#,
        );
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:Document");
        let is_a = r.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:core:Class");
    }

    #[test]
    fn compile_class_with_parent() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Dog : ex:Animal {
                description = "A dog";
                requires ex:breed;
            }
        "#,
        );
        let r = &resources[0];
        let parent = r
            .get(&iri("urn:eigenius:core:subclass_of"))
            .unwrap()
            .as_iri_array();
        assert_eq!(parent[0].as_str(), "urn:eigenius:example:Animal");
    }

    #[test]
    fn compile_property() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            property ex:count : core:integer {
                description = "Number of items";
                min_value = 0;
                max_value = 100;
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:count");
        assert_eq!(
            r.get(&iri("urn:eigenius:core:data_type")).unwrap().as_str(),
            Some("urn:eigenius:core:integer")
        );
        assert_eq!(
            r.get(&iri("urn:eigenius:core:min_value"))
                .unwrap()
                .as_integer(),
            Some(0)
        );
    }

    #[test]
    fn compile_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            resource ex:rex : ex:Dog {
                ex:name = "Rex";
                ex:breed = "German Shepherd";
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:rex");
        assert_eq!(
            r.get(&iri("urn:eigenius:example:name")).unwrap().as_str(),
            Some("Rex")
        );
    }

    #[test]
    fn compile_simple_program() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:identity : ex:Document -> ex:Document {
                input
            }
        "#,
        );
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:identity");
        assert_eq!(
            r.get(&iri("urn:eigenius:program:input_type"))
                .unwrap()
                .as_str(),
            Some("urn:eigenius:example:Document")
        );
    }

    #[test]
    fn compile_program_with_let_and_construct() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:summarize : ex:Document -> ex:Document {
                let summary : core:string = CompleteText(input);
                Construct ex:Document { ex:text = summary }
            }
        "#,
        );
        let r = &resources[0];
        let body = r
            .get(&iri("urn:eigenius:program:body"))
            .unwrap()
            .as_embedded()
            .unwrap();
        // Body should be a Let
        let is_a = body.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:program:Let");
    }

    #[test]
    fn compile_component_shorthand() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:test : ex:A -> ex:B {
                CompleteText(input)
            }
        "#,
        );
        let r = &resources[0];
        let body = r
            .get(&iri("urn:eigenius:program:body"))
            .unwrap()
            .as_embedded()
            .unwrap();
        // Function should be the full component IRI
        let func = body
            .get(&iri("urn:eigenius:program:function"))
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(func, "urn:eigenius:program:components:CompleteText");
    }

    #[test]
    fn compile_codata_declaration() {
        // A codata type with two observations, one referencing itself.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:IntStream {
                head : core:integer;
                tail : ex:IntStream;
            }
        "#,
        );
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:IntStream");
        let is_a = r.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:core:CodataType");
        assert_eq!(
            r.get(&iri("urn:eigenius:core:short_name"))
                .unwrap()
                .as_str(),
            Some("IntStream")
        );

        // Observations array
        let observations = r
            .get(&iri("urn:eigenius:core:observations"))
            .expect("observations property");
        let arr = match observations {
            Value::Array(a) => a,
            _ => panic!("observations must be an array"),
        };
        assert_eq!(arr.len(), 2);

        // First observation: head -> core:integer
        let head = match &arr[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("observation must be embedded"),
        };
        assert_eq!(
            head.get(&iri("urn:eigenius:core:observation_name"))
                .unwrap()
                .as_str(),
            Some("head")
        );
        assert_eq!(
            head.get(&iri("urn:eigenius:core:observation_type"))
                .unwrap()
                .as_str(),
            Some("urn:eigenius:core:integer")
        );

        // Second observation: tail -> ex:IntStream (self-reference)
        let tail = match &arr[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("observation must be embedded"),
        };
        assert_eq!(
            tail.get(&iri("urn:eigenius:core:observation_name"))
                .unwrap()
                .as_str(),
            Some("tail")
        );
        assert_eq!(
            tail.get(&iri("urn:eigenius:core:observation_type"))
                .unwrap()
                .as_str(),
            Some("urn:eigenius:example:IntStream")
        );
    }

    #[test]
    fn compile_corecord_expression() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:mk_pair : ex:Unit -> ex:Pair {
                corecord {
                    fst = 1;
                    snd = 2;
                }
            }
        "#,
        );
        let r = &resources[0];
        let body = r
            .get(&iri("urn:eigenius:program:body"))
            .unwrap()
            .as_embedded()
            .unwrap();
        // Body should be a CoRecord
        assert_eq!(body.is_a()[0].as_str(), "urn:eigenius:program:CoRecord");

        let cofields = body
            .get(&iri("urn:eigenius:program:cofields"))
            .expect("cofields");
        let arr = match cofields {
            Value::Array(a) => a,
            _ => panic!("cofields must be array"),
        };
        assert_eq!(arr.len(), 2);

        let fst = match &arr[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("cofield must be embedded"),
        };
        assert_eq!(fst.is_a()[0].as_str(), "urn:eigenius:program:CoField");
        assert_eq!(
            fst.get(&iri("urn:eigenius:program:observation_name"))
                .unwrap()
                .as_str(),
            Some("fst")
        );
    }

    #[test]
    fn compile_full_file() {
        let input = r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Document {
                description = "A text document";
                requires ex:text;
            }

            property ex:text : core:string {
                description = "The text content";
            }

            resource ex:doc1 : ex:Document {
                ex:text = "Hello world";
            }

            program ex:summarize : ex:Document -> ex:Document {
                let summary : core:string = CompleteText(input);
                Construct ex:Document { ex:text = summary }
            }
        "#;

        let resources = compile_esl(input);
        assert_eq!(resources.len(), 4);

        // Verify all resources serialize to valid Eigon-JSON
        for r in &resources {
            let json = eigon_json::serialize_resource(r);
            assert!(json.is_object(), "resource should serialize to JSON object");
        }
    }

    #[test]
    fn compile_unknown_namespace_error() {
        let result = esl::compile(
            r#"
            class unknown:Foo {
                description = "Bad";
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn round_trip_demo() {
        // Compile the demo ESL and verify it produces the same structure
        // as the hand-written demo/document.json
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace demo = "urn:eigenius:demo";

            class demo:Document {
                description = "A text document for analysis.";
                requires demo:text;
            }

            property demo:text : core:string {
                description = "The text content of a document.";
            }

            resource demo:doc_001 : demo:Document {
                demo:text = "Eigenius is a typed knowledge graph platform.";
            }
        "#,
        );

        assert_eq!(resources.len(), 3);
        // Class
        assert_eq!(
            resources[0].id().unwrap().as_str(),
            "urn:eigenius:demo:Document"
        );
        // Property
        assert_eq!(
            resources[1].id().unwrap().as_str(),
            "urn:eigenius:demo:text"
        );
        // Resource
        assert_eq!(
            resources[2].id().unwrap().as_str(),
            "urn:eigenius:demo:doc_001"
        );
        assert_eq!(
            resources[2]
                .get(&iri("urn:eigenius:demo:text"))
                .unwrap()
                .as_str(),
            Some("Eigenius is a typed knowledge graph platform.")
        );
    }

    // --- DeclaredResource stamping tests (Phase 10b) ---

    fn has_declared_resource(r: &Resource) -> bool {
        r.is_a()
            .iter()
            .any(|i| i.as_str() == crate::ontology::well_known::DECLARED_RESOURCE)
    }

    fn declared_by(r: &Resource) -> Option<String> {
        r.get(&iri(crate::ontology::well_known::DECLARED_BY))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    #[test]
    fn esl_class_stamped_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Foo {
                description = "test";
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL class should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_property_stamped_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            property ex:bar : core:string {
                description = "test";
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL property should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_resource_stamped_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            resource ex:thing : ex:Foo {
                ex:name = "test";
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL resource should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_program_stamped_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            program ex:identity : ex:A -> ex:B {
                input
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL program should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn esl_codata_stamped_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:Stream {
                head : core:integer;
                tail : ex:Stream;
            }
        "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL codata should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    // --- `data` declaration compilation (Phase 11b step 8) ---

    #[test]
    fn compile_data_nat_non_parametric() {
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }
            "#,
        );
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:Nat");
        assert!(r
            .is_a()
            .iter()
            .any(|i| i.as_str() == "urn:eigenius:core:InductiveType"));
        assert_eq!(
            r.get(&iri("urn:eigenius:core:short_name"))
                .and_then(|v| v.as_str()),
            Some("Nat")
        );

        // No params for Nat.
        let params = match r.get(&iri("urn:eigenius:core:type_params")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_params must be an array"),
        };
        assert!(params.is_empty());

        // Two constructors.
        let ctors = match r.get(&iri("urn:eigenius:core:ctors")) {
            Some(Value::Array(a)) => a,
            _ => panic!("ctors must be an array"),
        };
        assert_eq!(ctors.len(), 2);

        // zero
        let zero = match &ctors[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("ctor must be embedded"),
        };
        // Each ctor carries an IRI derived from parent + local name
        // (Phase 11b step 9 — IRI as canonical identity).
        assert_eq!(
            zero.id().map(|i| i.as_str()),
            Some("urn:eigenius:example:Nat:zero")
        );
        assert_eq!(
            zero.get(&iri("urn:eigenius:core:ctor_name"))
                .and_then(|v| v.as_str()),
            Some("zero")
        );
        let zero_args = match zero.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            _ => panic!("arg_types must be an array"),
        };
        assert!(zero_args.is_empty());

        // succ(ex:Nat)
        let succ = match &ctors[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("ctor must be embedded"),
        };
        assert_eq!(
            succ.id().map(|i| i.as_str()),
            Some("urn:eigenius:example:Nat:succ")
        );
        assert_eq!(
            succ.get(&iri("urn:eigenius:core:ctor_name"))
                .and_then(|v| v.as_str()),
            Some("succ")
        );
        let succ_args = match succ.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            _ => panic!("arg_types must be an array"),
        };
        assert_eq!(succ_args.len(), 1);
        let succ_arg = match &succ_args[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("arg type must be embedded"),
        };
        assert_eq!(
            succ_arg
                .get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("urn:eigenius:example:Nat")
        );
    }

    #[test]
    fn compile_data_list_parametric_records_param_references_as_bare_names() {
        // The bare `A` in `cons(A, ex:List(A))` is a reference to the
        // type parameter — compile encodes it as the raw name `"A"`,
        // not a resolved IRI.
        let resources = compile_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:List(A : core:Set) {
                nil,
                cons(A, ex:List(A)),
            }
            "#,
        );
        let r = &resources[0];

        // One param, name=A, kind=core:Set.
        let params = match r.get(&iri("urn:eigenius:core:type_params")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_params must be an array"),
        };
        assert_eq!(params.len(), 1);
        let p = match &params[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("param must be embedded"),
        };
        assert_eq!(
            p.get(&iri("urn:eigenius:core:param_name"))
                .and_then(|v| v.as_str()),
            Some("A")
        );
        assert_eq!(
            p.get(&iri("urn:eigenius:core:param_kind"))
                .and_then(|v| v.as_str()),
            Some("urn:eigenius:core:Set")
        );

        // cons ctor: first arg is bare "A", second is parametric List(A).
        let ctors = match r.get(&iri("urn:eigenius:core:ctors")) {
            Some(Value::Array(a)) => a,
            _ => panic!("ctors must be an array"),
        };
        let cons = match &ctors[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("cons must be embedded"),
        };
        let cons_args = match cons.get(&iri("urn:eigenius:core:arg_types")) {
            Some(Value::Array(a)) => a,
            _ => panic!("arg_types must be an array"),
        };
        assert_eq!(cons_args.len(), 2);

        // arg 0: bare A — type_name is "A", no type_args.
        let arg0 = match &cons_args[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("arg must be embedded"),
        };
        assert_eq!(
            arg0.get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("A")
        );
        let arg0_args = match arg0.get(&iri("urn:eigenius:core:type_args")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_args must be an array"),
        };
        assert!(arg0_args.is_empty());

        // arg 1: ex:List(A) — type_name is IRI, type_args = [bare A].
        let arg1 = match &cons_args[1] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("arg must be embedded"),
        };
        assert_eq!(
            arg1.get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("urn:eigenius:example:List")
        );
        let arg1_args = match arg1.get(&iri("urn:eigenius:core:type_args")) {
            Some(Value::Array(a)) => a,
            _ => panic!("type_args must be an array"),
        };
        assert_eq!(arg1_args.len(), 1);
        let arg1_a = match &arg1_args[0] {
            Value::Embedded(r) => r.as_ref(),
            _ => panic!("type arg must be embedded"),
        };
        assert_eq!(
            arg1_a
                .get(&iri("urn:eigenius:core:type_name"))
                .and_then(|v| v.as_str()),
            Some("A")
        );
    }

    #[test]
    fn compile_data_is_stamped_as_declared_resource() {
        let resources = compile_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Bool {
                tt,
                ff,
            }
            "#,
        );
        let r = &resources[0];
        assert!(
            has_declared_resource(r),
            "ESL data should have DeclaredResource in is_a"
        );
        assert_eq!(declared_by(r), Some("esl-compiler".to_string()));
    }

    #[test]
    fn ctor_name_collision_within_a_file_is_rejected() {
        // Two inductives both declaring `mk` — the per-file ctor table
        // catches the collision at compile time so bare references can
        // be unambiguously resolved later.
        let result = esl::compile(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Foo {
                mk,
            }

            data ex:Bar {
                mk,
            }
            "#,
        );
        let err = result.expect_err("collision must be rejected");
        let msg = err[0].message.clone();
        assert!(
            msg.contains("constructor `mk`") && msg.contains("collides"),
            "unexpected error: {msg}"
        );
    }
}
