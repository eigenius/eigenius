//! AST types for ESL.
//!
//! The parser produces these types. The compiler walks them
//! to emit Eigon-JSON resources.

use crate::esl::error::Position;

/// A complete ESL file.
#[derive(Debug)]
pub struct File {
    pub namespaces: Vec<NamespaceDecl>,
    pub declarations: Vec<Declaration>,
}

/// A namespace alias: `namespace core = "urn:eigenius:core";`
#[derive(Debug)]
pub struct NamespaceDecl {
    pub alias: String,
    pub uri: String,
    pub pos: Position,
}

/// A qualified name: `ex:Dog` or bare `Dog`.
#[derive(Debug, Clone)]
pub struct QualifiedName {
    pub namespace: Option<String>,
    pub name: String,
    pub pos: Position,
}

/// A top-level declaration.
#[derive(Debug)]
pub enum Declaration {
    Class(ClassDecl),
    Property(PropertyDecl),
    Resource(ResourceDecl),
    Program(ProgramDecl),
}

/// `class ex:Dog : ex:Animal { ... }`
#[derive(Debug)]
pub struct ClassDecl {
    pub name: QualifiedName,
    pub parent: Option<QualifiedName>,
    pub body: Vec<ClassItem>,
    pub pos: Position,
}

/// Items inside a class block.
#[derive(Debug)]
pub enum ClassItem {
    Description(String),
    Requires(Vec<QualifiedName>),
    Recommends(Vec<QualifiedName>),
}

/// `property ex:name : core:string { ... }`
#[derive(Debug)]
pub struct PropertyDecl {
    pub name: QualifiedName,
    pub data_type: QualifiedName,
    pub body: Vec<PropertyItem>,
    pub pos: Position,
}

/// Items inside a property block.
#[derive(Debug)]
pub enum PropertyItem {
    Description(String),
    MinValue(f64),
    MaxValue(f64),
    MinLength(i64),
    MaxLength(i64),
    Pattern(String),
    Format(QualifiedName),
    AllowsOnly(Vec<QualifiedName>),
    Domain(Vec<QualifiedName>),
    ClassTypes(Vec<QualifiedName>),
    ElementType(QualifiedName),
}

/// `resource ex:rex : ex:Dog { ... }`
#[derive(Debug)]
pub struct ResourceDecl {
    pub name: QualifiedName,
    pub class: QualifiedName,
    pub body: Vec<ResourceField>,
    pub pos: Position,
}

/// A field in a resource block: `ex:name = "Rex";`
#[derive(Debug)]
pub struct ResourceField {
    pub property: QualifiedName,
    pub value: Value,
}

/// A value in structural position.
#[derive(Debug)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Ref(QualifiedName),
    Array(Vec<Value>),
    Block(Vec<ResourceField>),
}

/// `program ex:summarize : ex:Document -> ex:Summary { expr }`
#[derive(Debug)]
pub struct ProgramDecl {
    pub name: QualifiedName,
    pub input_type: QualifiedName,
    pub output_type: QualifiedName,
    pub attributes: Vec<ProgramAttribute>,
    pub body: Expr,
    pub pos: Position,
}

/// Attributes before the expression in a program block.
#[derive(Debug)]
pub enum ProgramAttribute {
    Description(String),
}

/// An expression (ML-style, inside program bodies).
#[derive(Debug)]
pub enum Expr {
    /// `let x : T = e; body`
    Let {
        name: String,
        typ: QualifiedName,
        value: Box<Expr>,
        body: Box<Expr>,
        pos: Position,
    },
    /// `f(arg)` or `f(arg, component_arg)`
    Apply {
        function: QualifiedName,
        argument: Box<Expr>,
        component_argument: Option<Box<Expr>>,
        pos: Position,
    },
    /// `x`
    Var { name: String, pos: Position },
    /// `\x -> e`
    Lambda {
        param: String,
        body: Box<Expr>,
        pos: Position,
    },
    /// `case e { A -> e1; B -> e2 }`
    Case {
        scrutinee: Box<Expr>,
        branches: Vec<(String, Expr)>,
        pos: Position,
    },
    /// `Construct T { f1 = e1, f2 = e2 }`
    ConstructExpr {
        class: QualifiedName,
        fields: Vec<(QualifiedName, Expr)>,
        pos: Position,
    },
    /// `e.prop`
    Project {
        expr: Box<Expr>,
        property: QualifiedName,
        pos: Position,
    },
    /// `map(\x -> e, collection)`
    MapExpr {
        function: Box<Expr>,
        collection: Box<Expr>,
        pos: Position,
    },
    /// `reduce(\acc x -> e, init, collection)`
    ReduceExpr {
        function: Box<Expr>,
        initial: Box<Expr>,
        collection: Box<Expr>,
        pos: Position,
    },
    /// `(a, b)`
    Pair {
        first: Box<Expr>,
        second: Box<Expr>,
        pos: Position,
    },
    /// `"hello"`, `42`, `true`
    Literal { value: LiteralValue, pos: Position },
}

/// A literal value in expression position.
#[derive(Debug)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}
