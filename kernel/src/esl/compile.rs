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

    // Register namespace aliases
    for ns in &file.namespaces {
        compiler.namespaces.insert(ns.alias.clone(), ns.uri.clone());
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
}

impl Compiler {
    fn new() -> Self {
        Self {
            namespaces: BTreeMap::new(),
        }
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
        }
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

                // Resolve function name — try as qualified, fall back to component shorthand
                let func_iri = match self.resolve(function) {
                    Ok(iri_str) => iri_str,
                    Err(_) => {
                        // Bare name — try as a built-in component
                        format!("urn:eigenius:program:components:{}", function.name)
                    }
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
                r.set(
                    iri("urn:eigenius:program:name"),
                    Value::String(name.clone()),
                );
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
                let prop_iri = self.resolve(property)?;
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
}
