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
    Codata(CodataDecl),
    Data(DataDecl),
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

/// `codata ex:Stream { head : ex:Elem; tail : ex:Stream }`
#[derive(Debug)]
pub struct CodataDecl {
    pub name: QualifiedName,
    pub observations: Vec<ObservationDecl>,
    pub pos: Position,
}

/// `data ex:List(A : Set) { nil, cons(A, List(A)) }` —
/// Phase 11b step 7 inductive type declaration (D19 §10).
///
/// v1 surface syntax (Haskell-style):
/// - Constructors are named, optionally with positional argument types
///   in parentheses.
/// - Argument types are parameterised name references (`Nat`, `List(A)`).
/// - The constructor's result type is implicitly `Self(params)` —
///   not written out explicitly. This sidesteps the need for a general
///   type-expression parser at this stage.
#[derive(Debug)]
pub struct DataDecl {
    pub name: QualifiedName,
    /// Type parameters: `(A : Set, B : Set, ...)`. Empty for
    /// non-parametric inductives.
    pub params: Vec<DataParam>,
    pub ctors: Vec<CtorDecl>,
    pub pos: Position,
}

/// A type parameter on a `data` declaration: `A : Set`.
///
/// For Phase 11b v1 the kind is always `Set`; the field is kept in
/// the AST so that future kinds (e.g. `Type(n)`) can be added without
/// a syntax break.
#[derive(Debug)]
pub struct DataParam {
    pub name: String,
    pub kind: QualifiedName,
    pub pos: Position,
}

/// A single constructor declaration: `nil` or `cons(A, List(A))`.
#[derive(Debug)]
pub struct CtorDecl {
    pub name: String,
    /// Positional argument types. Empty for nullary constructors.
    pub args: Vec<CtorArgType>,
    pub pos: Position,
}

/// A constructor argument type — a parameterised name reference.
///
/// Examples:
/// - `Nat` — `name=Nat`, `params=[]`
/// - `A` (a type parameter) — `name=A`, `params=[]`
/// - `List(A)` — `name=List`, `params=[CtorArgType { name=A, params=[] }]`
#[derive(Debug, Clone)]
pub struct CtorArgType {
    pub name: QualifiedName,
    pub params: Vec<CtorArgType>,
    pub pos: Position,
}

/// A single observation declaration inside a codata block:
/// `head : ex:Elem`
#[derive(Debug)]
pub struct ObservationDecl {
    pub name: String,
    pub typ: QualifiedName,
    pub pos: Position,
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
    /// `f(arg₁, arg₂, …)` with optional trailing config block
    /// `{ key = val; … }` for IO component dispatch.
    ///
    /// `args` carries every positional argument the parser saw —
    /// the compiler is responsible for arity validation based on
    /// what `function` resolves to:
    ///
    /// - Inductive constructors take any non-zero arity (Phase 11b).
    /// - IO components take exactly 1 positional arg, with the
    ///   optional trailing block (or a 2-positional sugar form,
    ///   `f(a, b)` ≡ `f(a) { … b … }`) supplying configuration.
    ///
    /// Mismatches surface as compile-time errors rather than parser
    /// errors so the diagnostic can mention the function's role.
    Apply {
        function: QualifiedName,
        args: Vec<Expr>,
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
    /// `corecord { obs = e1; ... }`
    ///
    /// Values are ordered — the order must match the declared
    /// observations of the target codata type.
    CoRecord { fields: Vec<CoField>, pos: Position },

    /// `match expr [returning T] { ctor -> body; ctor(x, y) -> body; ... }`
    /// (Phase 11b step 11–12, D19 §10).
    ///
    /// Pattern-matches a value of an inductive type. Each arm names
    /// a constructor and (optionally) binds variables for its
    /// arguments.
    ///
    /// `result_type` is the optional type annotation for every arm
    /// body. When present (Phase 11b step 11), the kernel-side
    /// expression builder desugars to `Exp::InductiveRec` with
    /// motive `λ_. result_type`. When absent (Phase 11b step 12),
    /// it produces `Exp::Match`, leaving motive synthesis to the
    /// type checker — which builds `λ_. expected_type` from the
    /// checking-mode context. Inference-mode use of an unannotated
    /// match is a type error with a clear diagnostic.
    Match {
        scrutinee: Box<Expr>,
        result_type: Option<QualifiedName>,
        arms: Vec<MatchArm>,
        pos: Position,
    },
}

/// One arm of a `match` expression.
///
/// Bindings are positional and must match the constructor's arity.
/// Use `_` for arguments that are not referenced in the body.
#[derive(Debug)]
pub struct MatchArm {
    pub ctor_name: String,
    pub bindings: Vec<String>,
    pub body: Expr,
    pub pos: Position,
}

/// A single copattern definition in a corecord: `obs = body`.
#[derive(Debug)]
pub struct CoField {
    pub name: String,
    pub body: Expr,
}

/// A literal value in expression position.
#[derive(Debug)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}
