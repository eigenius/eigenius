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
    /// D37 §3.3 — `merge_comorphism <iri> for <class> { … }`.
    /// Lowers to a `MergeComorphism` resource carrying
    /// `merge_target_class` + `merge_transformation`, plus (for the
    /// inline-body form) a synthesised standalone Lambda resource at a
    /// content-hash IRI.
    MergeComorphism(MergeComorphismDecl),
}

/// `class ex:Dog : ex:Animal { ... }` or
/// `class ex:HybridCell : ex:Cell, ex:Visualisable { ... }` for the
/// multi-parent header form (eigenius#29). Both this list and any
/// in-body `subclass_of A, B;` items contribute to the emitted
/// `core:subclass_of` array.
#[derive(Debug)]
pub struct ClassDecl {
    pub name: QualifiedName,
    pub parents: Vec<QualifiedName>,
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

/// `resource ex:rex : ex:Dog { ... }` or `resource ex:rex : ex:Dog, ex:Pet { ... }`.
/// At least one class is required; a comma-separated list expresses
/// multi-class instances (eigenius#29).
#[derive(Debug)]
pub struct ResourceDecl {
    pub name: QualifiedName,
    pub classes: Vec<QualifiedName>,
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
    /// Inductive constructor application: `Foo(arg1, arg2, ...)`.
    /// Lands in property positions whose `data_type` is
    /// `core:inductive` — D32 inductive-value literals on the
    /// surface. Nullary ctors require parentheses (`Foo()`) so
    /// the parser can disambiguate against bare resource
    /// references at parse time without needing the target
    /// property's type. The `ctor` name is unqualified — ctor
    /// names live in a per-inductive scope and the compiler
    /// resolves the ctor against the target property's
    /// declared `class_types` inductive at commit time.
    CtorApp {
        ctor: String,
        args: Vec<Value>,
        pos: Position,
    },
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

/// `codata ex:Stream { head : ex:Elem; tail : ex:Stream }` —
/// optionally parameterised `codata ex:Stream(i : Size, A : Set) { … }`.
#[derive(Debug)]
pub struct CodataDecl {
    pub name: QualifiedName,
    /// Type parameters: `(A : Set, i : Size, ...)`. Empty for
    /// non-parametric codata — the legacy shape.
    pub params: Vec<DataParam>,
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
    /// Positional or named constructor arguments.
    pub args: Vec<CtorArg>,
    pub pos: Position,
}

/// A single constructor argument.
///
/// Positional (the legacy form) carries just a type; the binder is
/// implicitly anonymous. Named (Phase 11b step 15h) introduces a
/// bound variable that subsequent args can reference, and can carry
/// an optional upper bound for sized termination tracking.
#[derive(Debug, Clone)]
pub enum CtorArg {
    /// `cons(A, List(A))` — positional, anonymous binder.
    Positional(CtorArgType),
    /// `succ(j : core:Size, ex:Nat(j))` — named binder with kind.
    /// The optional `bound` encodes a `< upper` clause; when the
    /// kind is `core:Size` and `bound` is present, this compiles to
    /// `Exp::SizedPi { upper, body }` and introduces a TSO
    /// hypothesis in the constructor's telescope.
    Named {
        name: String,
        kind: QualifiedName,
        bound: Option<QualifiedName>,
        pos: Position,
    },
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
/// `head : ex:Elem` or, for sized codata,
/// `tail : {j < i} -> ex:Stream(j, A)`.
#[derive(Debug)]
pub struct ObservationDecl {
    pub name: String,
    pub typ: TypeExpr,
    pub pos: Position,
}

/// A type expression (Phase 11b step 15h.3).
///
/// Covers the shapes needed for sized codata observations:
/// - Parameterised type refs (`ex:Stream(j, A)`)
/// - Size literals (`Inf`) and built-in sort (`Size`)
/// - Function types with optional bounded size binder
///   (`{j < i} -> ex:Stream(j, A)` or `A -> B`)
///
/// Purposely restricted — this isn't a full type-expression grammar,
/// just enough for the codata observation-type surface. Data ctor
/// arg types still use the simpler `CtorArgType` shape.
#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// Type reference with zero or more type args: `Name` or
    /// `Name(arg, arg, ...)`. Bare identifiers (no namespace) are
    /// resolved as: declared param / `Inf` / `Size` first, otherwise
    /// namespace lookup.
    Ref {
        name: QualifiedName,
        args: Vec<TypeExpr>,
        pos: Position,
    },
    /// Non-dependent arrow `A -> B` — used when the user writes a
    /// plain function type without a named binder.
    Arrow {
        domain: Box<TypeExpr>,
        codomain: Box<TypeExpr>,
        pos: Position,
    },
    /// Size-binder arrow `{j < bound} -> body` (sized) or
    /// `{j : Kind} -> body` (unbounded) or
    /// `{j : Kind < bound} -> body`.
    /// Compiles to `Exp::SizedPi` when bound is present and kind is
    /// `Size`; to plain `Exp::Pi` otherwise.
    BinderArrow {
        name: String,
        kind: QualifiedName,
        bound: Option<QualifiedName>,
        body: Box<TypeExpr>,
        pos: Position,
    },
    /// D37 §3.5 — value-typed Pi binder:
    /// `pi x_1 : T_1, ..., x_N : T_N => U`. Compiles to N nested
    /// single-parameter `Exp::Pi` nodes; the rightmost `U` is the
    /// return type. Distinct from `BinderArrow` (size-binder
    /// specific) and `Arrow` (anonymous, non-dependent) — `Pi` is
    /// the general value-typed binder needed for standalone Lambda
    /// resources' `program:type` slot and for `merge_comorphism`
    /// transformation signatures.
    Pi {
        params: Vec<TypedParam>,
        codomain: Box<TypeExpr>,
        pos: Position,
    },
}

/// A typed binder: `name : type`. Used by `TypeExpr::Pi` and by the
/// new typed `lambda` literal (D37 §3.1). The type can be any
/// `TypeExpr`, including nested `Pi` / `Ref` / `Arrow` forms.
#[derive(Debug, Clone)]
pub struct TypedParam {
    pub name: String,
    pub typ: TypeExpr,
    pub pos: Position,
}

/// D37 §3.3 — `merge_comorphism <iri> for <class> { <body> }`.
/// Compiles to a `MergeComorphism` resource (with `merge_target_class`
/// + `merge_transformation`) plus, for the inline-body form, a
/// synthesised standalone Lambda resource at a content-hash IRI.
#[derive(Debug)]
pub struct MergeComorphismDecl {
    /// The comorphism's own IRI (the `<iri>` after `merge_comorphism`).
    pub name: QualifiedName,
    /// The class A the comorphism is declared for. Compiled into
    /// `urn:eigenius:core:merge_target_class`.
    pub target_class: QualifiedName,
    pub body: MergeComorphismBody,
    pub pos: Position,
}

/// Either an inline lambda body or a reference to a separately-declared
/// Lambda resource. The two forms compile to the same `MergeComorphism`
/// resource shape; the inline form additionally emits the synthesised
/// transformation.
#[derive(Debug)]
pub enum MergeComorphismBody {
    /// `(a, b, opt) => <expression>` — the parameter types are
    /// inferred from the surrounding `for <class>` clause as
    /// `(class, class, Option<class>)`.
    Inline {
        params: Vec<String>,
        body: Expr,
        pos: Position,
    },
    /// `transformation = <iri>;` — references a previously-declared
    /// Lambda resource. Its Pi-type must match `(A, A, Option<A>) -> A`.
    Reference {
        transformation: QualifiedName,
        pos: Position,
    },
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
    /// `\x -> e` (untyped, embedded in `program` bodies) or
    /// `lambda x : T => body` (typed, D37 §3.1). The `param_type` slot
    /// is `None` for the untyped form (the type is inferred from the
    /// surrounding Pi during program elaboration) and `Some(T)` for
    /// the typed form. Multi-parameter `lambda x_1 : T_1, …, x_N : T_N
    /// => body` parses to N nested single-parameter `Lambda` nodes.
    Lambda {
        param: String,
        param_type: Option<TypeExpr>,
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
