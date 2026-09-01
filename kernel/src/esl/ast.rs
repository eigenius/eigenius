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
use serde::{Deserialize, Serialize};

/// A complete ESL file.
#[derive(Debug)]
pub struct File {
    pub namespaces: Vec<NamespaceDecl>,
    /// Level variables bound by `universe` declarations (eigenius#188), in source order.
    ///
    /// File-scoped like `namespace`, and for the same reason: a level variable is a name that has
    /// to be introduced before it is meaningful. Without this, `Sort u` auto-binds on first use —
    /// Lean's `autoBound` behaviour — and a typo (`Sort v` for `Sort u`) silently becomes a second,
    /// unrelated parameter instead of an error.
    pub universes: Vec<UniverseDecl>,
    pub declarations: Vec<Declaration>,
}

/// `universe u v;` — binds one or more level variables for the rest of the file.
///
/// Follows Lean's `universe ident ident*` (space-separated), with ESL's statement terminator.
#[derive(Debug)]
pub struct UniverseDecl {
    pub names: Vec<String>,
    pub pos: Position,
}

/// A namespace alias: `namespace core = "urn:eigenius:core";`
#[derive(Debug)]
pub struct NamespaceDecl {
    pub alias: String,
    pub uri: String,
    pub pos: Position,
}

/// A qualified name: `ex:Dog` or bare `Dog`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    Data(DataDecl),
    /// D37 §3.3 — `merge_comorphism <iri> for <class> { … }`.
    /// Lowers to a `MergeComorphism` resource carrying
    /// `merge_target_class` + `merge_transformation`, plus (for the
    /// inline-body form) a synthesised standalone Lambda resource at a
    /// content-hash IRI.
    MergeComorphism(MergeComorphismDecl),
    /// D43 §3.1 — `text_index <iri> { target_property = ...; text_analyzer = "..."; }`.
    /// Lowers to a `core:TextIndex` Resource (M2+ compiler work). The
    /// declared body fields populate the Resource's properties; the
    /// class is implicit in the keyword.
    TextIndex(TextIndexDecl),
    /// D43 §3.1 — `vector_index <iri> { target_property = ...; vec_model = ...; vec_dim = ...; ... }`.
    /// Lowers to a `core:VectorIndex` Resource (M2+ compiler work). The
    /// declared body fields populate the Resource's properties; the
    /// class is implicit in the keyword.
    VectorIndex(VectorIndexDecl),
    /// eigenius#72 — `axiom Name : <type-expr>` declares a named
    /// chain-resident axiom whose statement is the supplied type
    /// expression. Lowers to a `core:Axiom` Resource (D46 §10) whose
    /// `axiom_statement` is the type expression encoded via the D47
    /// codec. Optional `note: "..."` populates `core:axiom_justification`.
    Axiom(AxiomDecl),
    /// `def ex:F(p : T, ...) : R = <type-expr>` — a transparent definition (D66). Lowers to an
    /// `eigentt:Definition` carrying `definition_type` and a lambda-chain `definition_body`.
    Def(DefDecl),
    /// D52 §12 — file-level `macro` declaration. Defines a smart
    /// constructor `macro ns:Name(p1 : T1, ...) : RetT => body`
    /// where `body` is a `Value` expression that can reference the
    /// parameter names. Compile-time AST substitution only: each
    /// call site substitutes the actual-argument `Value`s into the
    /// body positionally and recursively compiles the result. No
    /// runtime closure / lambda value is created, no kernel NbE
    /// evaluation. The name "Macro" is deliberate — this is *not*
    /// a function in the runtime-callable sense, and the `Function`
    /// AST name is reserved for a possible future addition with
    /// real evaluation semantics.
    ///
    /// Lets D52 author `stats:IID(replicates, BiologicalReplication)`
    /// as a brief surface form that desugars to a fully-positional
    /// `stats:SampleSet.Set(...)` ctor call.
    Macro(MacroDecl),
}

/// D52 §12 — file-level smart-constructor `macro` declaration.
///
/// Surface form: `macro ns:Name(p1 : T1, p2 : T2, ...) : RetT => body;`
///
/// Compile-time AST substitution only: each call site `ns:Name(arg1,
/// arg2, ...)` substitutes positional argument `Value`s into the
/// body and recursively compiles the result. The macro is *not*
/// lowered to a chain resource — it lives only in the compiler's
/// per-file `macros` table and disappears at compile time. Parameter
/// types and return type are stored for diagnostics but the macro
/// expansion does not type-check at the macro-decl site; type errors
/// surface at the expanded call site against the body's substituted
/// shape.
///
/// Two restrictions in v1:
/// - The body must be a `Value` (resource-property value AST), not
///   a `Term` or `Expr` (program body). Smart constructors
///   produce ctor values; that's their use case.
/// - No recursion. The compile-time expansion has no termination
///   guarantee for recursive calls and the use case (named-design
///   smart constructors) doesn't need it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroDecl {
    pub name: QualifiedName,
    pub params: Vec<MacroParam>,
    pub return_type: Term,
    pub body: Value,
    pub pos: Position,
}

/// A single parameter in a [`MacroDecl`]'s parameter list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroParam {
    pub name: String,
    pub typ: Term,
    pub pos: Position,
}

/// eigenius#72 — `axiom Name : <type-expr> [desc: "..."] [note: "..."]` declaration.
///
/// `desc:` populates `core:description` — the axiom's natural-language gloss (a WordNet
/// verb/adjective sense's definition), which the concept-`core:description` text index makes
/// searchable (D63 lexicon-augmentation §6a index c). `note:` populates
/// `core:axiom_justification` (the warrant for admitting the axiom unchecked). Both optional,
/// either order.
#[derive(Debug)]
pub struct AxiomDecl {
    pub name: QualifiedName,
    pub statement: Term,
    pub description: Option<String>,
    pub justification: Option<String>,
    pub pos: Position,
}

/// `def ex:F(m : Set, g : Set) : Prop = <body>` — a chain-resident TRANSPARENT definition (D66).
///
/// Distinct from [`AxiomDecl`] in exactly the way that matters: an axiom's IRI evaluates to a rigid
/// neutral and never unfolds, whereas a definition's body is substituted by decode at every use, so
/// `F(a, b)` and the body-with-arguments are the same term and hash identically.
///
/// The parameters give both halves: the declared type is `Pi(m : Set). Pi(g : Set). Prop`, and the
/// body is the lambda chain `Lam(m, Lam(g, ...))`. Nothing is stored twice — arity and parameter
/// types are read back off the type.
#[derive(Debug)]
pub struct DefDecl {
    pub name: QualifiedName,
    /// `(m : Set, g : Set)`. Reuses [`TypedParam`] — the production `forall` already uses — so a
    /// parameter's type can be any type expression (`P : T -> Prop`), not just a class or a sort
    /// as [`DataParam`] allows.
    pub params: Vec<TypedParam>,
    /// The result type, after the `:`.
    pub result: Term,
    /// The right-hand side, after the `=`.
    pub body: Term,
    pub description: Option<String>,
    pub pos: Position,
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
    /// `expected_type <term>;` — the type this property's values must check
    /// against. The validator forms `Ann(value, <term>)`, so the kernel's
    /// existing annotation rule does the checking.
    ExpectedType(Term),
    /// `is_a_type;` — the values must themselves be types (`check_type`).
    /// Separate from `ExpectedType` because the sorts vary within a slot and
    /// `ExpectedType` holds one term.
    IsAType,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceField {
    pub property: QualifiedName,
    pub value: Value,
}

/// A value in structural position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Ref(QualifiedName),
    Array(Vec<Value>),
    Block(Vec<ResourceField>),
    /// `json({ "k": 0.5 })` — an opaque JSON value for a `core:json`-typed property
    /// (eigenius#222). A general chain feature: `core:json` is a declared `core:DataType` with a
    /// dozen declared properties, all of them written by institution runtimes (Julia solver
    /// outputs, trajectories, witness blobs) rather than by hand — and until this existed there
    /// was no ESL spelling for one at all.
    ///
    /// Distinct from [`Value::Block`], which makes an embedded RESOURCE. The wire forms are
    /// distinguished the way `eigon_json::parse_json_value` distinguishes them: a resource has
    /// IRI-shaped keys, opaque JSON does not.
    Json(serde_json::Value),
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
    /// `type_expr(<type-expr>)` — inline D47-encoded EigenTT type
    /// expression as a resource-field value. Lowered via
    /// `lower_type_expr_to_exp` + `encode_type` at compile time;
    /// the resulting `Value::Json` lands directly on the property,
    /// matching what a programmatic `encode_type` caller produces.
    /// Surface counterpart of `formula(...)` for D32 §3.7
    /// inductive values — same purpose (write the expression
    /// readably instead of the tagged-dict tree the codec emits),
    /// different codec.
    Term {
        typ: Term,
        pos: Position,
    },
    /// D52 §12 — call to a [`MacroDecl`] smart constructor declared
    /// elsewhere in the same file (or, with the cross-file extension,
    /// in a parent layer). The compiler resolves `name` to a
    /// registered macro and expands the call site by substituting
    /// `args` (positionally) into the macro body's `Value`, then
    /// recursively compiling the result.
    ///
    /// The shape that distinguishes this from `CtorApp` is the
    /// presence of a namespace qualifier on the name: bare
    /// `Foo(args)` parses as `CtorApp { ctor: "Foo" }`, while
    /// `ns:Foo(args)` parses as `MacroCall { name: ns:Foo }`.
    /// Constructors live in per-inductive scopes (unqualified);
    /// macros live in file-level qualified namespaces.
    MacroCall {
        name: QualifiedName,
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
    /// `description = "…";` in the body — `core:description` on the emitted resource. The field
    /// `ClassDecl`, `AxiomDecl` and `DefDecl` already carry; `data` did not until eigenius#221,
    /// which kept every inductive out of `core:description_text_index`.
    pub description: Option<String>,
    /// Type parameters: `(A : Set, B : Set, ...)`. Empty for
    /// non-parametric inductives.
    pub params: Vec<DataParam>,
    /// Index telescope: written after `:` between params and the result
    /// sort. Example: `data Vec(A : Set) : Nat -> Set { ... }` has one
    /// index `_ : Nat`. Empty for non-indexed declarations (the default,
    /// matching the pre-D48 / pre-eigenius#72-Layer-2 surface).
    ///
    /// Indices use [`DataIndex`] rather than [`DataParam`] because
    /// index kinds can be Sort literals (e.g., D39 §5's
    /// `justification:Certificate : justification:Term → Prop → Type` has `Prop` as
    /// its second index kind). Type params have no such use case in
    /// v1 — they're always Set-kinded today.
    pub indices: Vec<DataIndex>,
    /// Result sort declared after the index telescope's arrow chain.
    /// `None` defaults to `Set` (`Sort(1)`).
    pub result_sort: Option<SortKind>,
    /// Additional `is_a` class memberships for the emitted
    /// inductive-type resource, beyond the implicit `InductiveType`
    /// (D52 §12 #8 / §7.4 enabler). Surface syntax:
    /// `data X : T, Marker1, Marker2 { ctors }`. Used by the
    /// statistics institution to mark predicates with scope classes
    /// (`stats:PopulationLevel` / `stats:MeasurementLevel`) without
    /// the companion-resource workaround that collides with
    /// `stamp_declared`. Empty for the standard non-marked case.
    pub extra_classes: Vec<QualifiedName>,
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
    /// The parameter's kind — a qualified-name class OR a sort literal
    /// (`Prop` / `Set` / `Type N`). Sort-typed parameters are needed for
    /// Lean-style parametrized inductives such as `And (P : Prop, Q : Prop)`.
    pub kind: IndexKind,
    pub pos: Position,
}

/// One entry in an indexed-data declaration's index telescope
/// (eigenius#72 Layer 2). Differs from [`DataParam`] in that the kind
/// can be a Sort literal (`Prop` / `Set` / `Type N`) as well as a
/// qualified-name reference — D39 §5's
/// `justification:Certificate : justification:Term → Prop → Type` has `Prop` as its
/// second index kind, which `DataParam`'s `QualifiedName`-only kind
/// field can't express.
#[derive(Debug)]
pub struct DataIndex {
    pub name: String,
    pub kind: IndexKind,
    pub pos: Position,
}

/// The kind of an index in an indexed-data declaration. See
/// [`DataIndex`].
#[derive(Debug, Clone)]
pub enum IndexKind {
    /// A qualified-name reference — either a bare parameter name
    /// (resolved against the enclosing `data` declaration's
    /// `params`) or a class IRI (resolved through the namespace
    /// registry). The Phase-4 implementation of
    /// `parse_data_index_telescope` accepted only this shape.
    Named(QualifiedName),
    /// A Sort literal — `Prop` / `Set` / `Type N`. Emitted by the
    /// parser when an intermediate index-telescope segment is itself
    /// a sort. Encoded in JSON as the literal string (`"Prop"`,
    /// `"Set"`, `"Type:N"`) that the kernel's `decode_param_kind_str`
    /// recognises.
    Sort(SortKind),
}

/// A single constructor declaration.
///
/// Two surface forms (eigenius#72 Layer 2):
///
/// - `Positional` — the legacy form: `nil` (nullary) or
///   `cons(A, List(A))` (with positional / named arg list).
///   The constructor's conclusion is implicitly `Self(params)`; this
///   form cannot express conclusion indices and is therefore only
///   usable for non-indexed declarations.
/// - `Typed` — the indexed-aware form: `cons : forall (n : Nat) => A
///   -> Vec(A, n) -> Vec(A, succ(n))`. The full Π-telescope including
///   the conclusion (with explicit indices) is supplied as a single
///   `Term`. Required when the declaration has indices.
#[derive(Debug)]
pub enum CtorDecl {
    Positional {
        name: String,
        /// Positional or named constructor arguments.
        args: Vec<CtorArg>,
        pos: Position,
    },
    Typed {
        name: String,
        /// Full Π-telescope of the constructor type, ending in an
        /// application of the parent inductive to its params and indices.
        typ: Term,
        pos: Position,
    },
}

impl CtorDecl {
    pub fn name(&self) -> &str {
        match self {
            CtorDecl::Positional { name, .. } | CtorDecl::Typed { name, .. } => name,
        }
    }

    pub fn pos(&self) -> &Position {
        match self {
            CtorDecl::Positional { pos, .. } | CtorDecl::Typed { pos, .. } => pos,
        }
    }

    /// Test-side convenience: return the positional args list. Panics
    /// on `Typed` — callers that aren't sure should match the variant
    /// explicitly.
    #[cfg(test)]
    pub fn args(&self) -> &[CtorArg] {
        match self {
            CtorDecl::Positional { args, .. } => args,
            CtorDecl::Typed { .. } => panic!("CtorDecl::args() called on Typed variant"),
        }
    }
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
    /// `succ(base : ex:Nat)` — a NAMED argument. The name is the readable label for the slot and
    /// lands in `core:arg_name`, a `recommends` on `core:InductiveArgType` that the Julia mirror
    /// generator reads for its field names (D32 §3.2).
    ///
    /// The spelling was `{name : kind}` — brace-delimited — because this variant began as the
    /// SIZED bounded binder (`{j : Size < i}`), where the braces marked a size argument. Sized
    /// types are gone (eigenius#218) and the braces went with them: they were never needed to
    /// disambiguate, since `ns:name` lexes as one atomic `QualName` token and the standalone
    /// `Colon` is reserved for the binder colon (eigenius#221).
    Named {
        name: String,
        typ: CtorArgType,
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
    pub typ: Term,
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
/// This IS the full term grammar — it is what `type_expr( … )` and an
/// `axiom` statement parse into, and it carries Pi/`forall`, Sigma/`exists`,
/// lambdas, applications and literals. It began as the restricted
/// codata-observation surface and grew; the comment saying so outlived the
/// truth, exactly as the name `TypeExpr` outlived the class it mirrored.
/// Data ctor arg types still use the simpler `CtorArgType` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Term {
    /// Type reference with zero or more type args: `Name` or
    /// `Name(arg, arg, ...)`. Bare identifiers (no namespace) are
    /// resolved as: declared param / `Inf` / `Size` first, otherwise
    /// namespace lookup.
    Ref {
        name: QualifiedName,
        args: Vec<Term>,
        pos: Position,
    },
    /// Non-dependent arrow `A -> B` — used when the user writes a
    /// plain function type without a named binder.
    Arrow {
        domain: Box<Term>,
        codomain: Box<Term>,
        pos: Position,
    },
    /// Dependent binder arrow `{j : Kind} -> body`. Compiles to `Exp::Pi`.
    ///
    /// It also carried a `bound`, for `{j : Kind < b}` and its shorthand `{j < b}`, which
    /// compiled to `Exp::SizedPi`. Both went with sized types (eigenius#218) — and the bound
    /// was already unwritable before that: it emitted a `core:binder_bound` property that
    /// `core-ontology.json` never declared, so Rule 22 rejected any resource carrying one.
    BinderArrow {
        name: String,
        kind: QualifiedName,
        body: Box<Term>,
        pos: Position,
    },
    /// Dependent PAIR binder: `exists x_1 : T_1, ..., x_N : T_N . B`.
    ///
    /// Compiles to N nested `Exp::Sig`, exactly as `Pi`/`forall` compiles to nested
    /// `Exp::Pi`. Named `exists` by analogy with `forall` (which is itself an alias for
    /// `pi`) — a Sigma in Prop position IS the existential, and the first projection
    /// `eigentt:fst` is what recovers the witness.
    ///
    /// The DCG parser produces these constantly — every definite description is
    /// `the(Sig x : C. P(x)).1` — so without this variant no parsed proposition could be
    /// written in ESL at all.
    /// The unit VALUE `()` — `Exp::Unit`, the sole inhabitant of `One`.
    ///
    /// Hand-written certificate terms normally omit it (the kernel synthesises the slot in
    /// e.g. `declared(bridge, P)`), so this exists mainly so a printer can emit back what the
    /// kernel encoded and have it reparse — see `esl::print`.
    Unit {
        pos: Position,
    },
    Sigma {
        params: Vec<TypedParam>,
        body: Box<Term>,
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
    ///
    /// eigenius#72: `forall` is an alias for `pi` produced by the
    /// `Forall` keyword. Both parse into this variant.
    Pi {
        params: Vec<TypedParam>,
        codomain: Box<Term>,
        pos: Position,
    },
    /// eigenius#72 — sort literal in type position. `Prop` is
    /// `Sort(0)`, `Set` is `Sort(1)`, `Type N` is `Sort(N+1)`.
    /// Used in `axiom` statements, indexed `data` declarations, and
    /// motives.
    Sort {
        kind: SortKind,
        pos: Position,
    },
    /// String / integer / float literal in type position. Lowers to
    /// `Exp::LitString` / `LitInt` / `LitFloat` — Phase-2 term-level
    /// constructors the kernel admits as arguments to value-indexed
    /// inductives (e.g. `Asserts(iri : core:string) : Prop` consumes
    /// a `LitString` at its iri index slot). Surface required by
    /// `type_expr(...)` so authors can write `Asserts("urn:foo")`
    /// directly instead of binding the IRI through a separate `Var`.
    LitString {
        value: String,
        pos: Position,
    },
    LitInt {
        value: i64,
        pos: Position,
    },
    LitFloat {
        value: f64,
        pos: Position,
    },
    /// eigenius#142 — boolean literal in type position. Lowers to
    /// `Exp::LitBool`. Needed so a D47 term containing the `LitBool`
    /// ctor prints to ESL source that reparses.
    LitBool {
        value: bool,
        pos: Position,
    },
    /// eigenius#72 Layer 3 — type-level lambda introduced by `fun`:
    /// `fun (i : T) => body`. Used as a motive for `match … returning
    /// <motive>` over indexed inductives. Compiles to nested
    /// single-parameter `Exp::Lam` chains, mirroring how `Pi` compiles
    /// to nested `Exp::Pi` chains.
    Lambda {
        params: Vec<TypedParam>,
        body: Box<Term>,
        pos: Position,
    },
    /// Compile-time aliases in type-expression position:
    /// `alias name1 = expr1, name2 = expr2, ... in body`. The
    /// bindings are textual substitutions resolved at lowering time
    /// — they shadow no chain-resident identifier and produce no D47
    /// encoding of their own. The body's free references to each
    /// binding name (as bare `Term::Ref`) inline the bound
    /// expression. Earlier bindings are in scope inside later
    /// bindings (sequential lexical scoping). Pure ESL surface
    /// sugar; the kernel's NbE never sees these.
    ///
    /// Distinct from kernel-level `let` (`Decl::Def` in NbE; surface
    /// `let x : T = e; body` in the program-body parser). Reserving
    /// the `alias` keyword keeps the two semantics distinguishable
    /// even when a future type-position `let` with real δ-binding
    /// lands.
    Alias {
        bindings: Vec<AliasBinding>,
        body: Box<Term>,
        pos: Position,
    },
    /// Type annotation `(e : T)` — the bidirectional mode switch. Compiles to
    /// `Exp::Ann`, letting a checkable term (e.g. a `fun` lambda, which has no
    /// synthesizable type) appear where a type is inferred — e.g. a determiner's
    /// λ-semantics in `lexicon:sem` (D63 §8.2).
    Ann {
        expr: Box<Term>,
        typ: Box<Term>,
        pos: Position,
    },
}

/// Single binding in an `alias name = expr, ... in body` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasBinding {
    pub name: String,
    pub value: Term,
    pub pos: Position,
}

/// Sort literals recognised in type expressions (eigenius#72).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SortKind {
    /// `Prop` — `Sort 0`.
    Prop,
    /// `Set` — `Sort 1`.
    ///
    /// This is Lean's `Type` (= `Type 0`). The names diverge and the levels do not; Lean's `Set`
    /// is a library type for sets, so a reader arriving from Lean will misread this one. Kept
    /// because renaming touches 230 uses for no semantic gain — see N3 §3.
    Set,
    /// `Type <level>` — `Sort (level + 1)`, the same numbering Lean uses.
    Type(LevelExpr),
    /// `Sort <level>` — the general form (eigenius#188).
    Sort(LevelExpr),
}

impl std::fmt::Display for SortKind {
    /// The surface spelling, for diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortKind::Prop => write!(f, "Prop"),
            SortKind::Set => write!(f, "Set"),
            SortKind::Type(l) => write!(f, "Type {l}"),
            SortKind::Sort(l) => write!(f, "Sort {l}"),
        }
    }
}

impl std::fmt::Display for LevelExpr {
    /// The surface spelling, so `result_sort` strings and diagnostics round-trip.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LevelExpr::Num(n) => write!(f, "{n}"),
            LevelExpr::Var(v) => write!(f, "{v}"),
            LevelExpr::Add(l, n) => write!(f, "{l} + {n}"),
            LevelExpr::Max(l, r) => write!(f, "max {l} {r}"),
            LevelExpr::IMax(l, r) => write!(f, "imax {l} {r}"),
        }
    }
}

/// A universe level as written in ESL (eigenius#188).
///
/// Surface syntax follows Lean 4 — see
/// <https://lean-lang.org/doc/reference/latest/The-Type-System/Universes/>. Lowered to
/// [`crate::nbe::level::Level`] by the compiler; `Add` becomes iterated `Succ`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LevelExpr {
    /// A numeral: `0`, `1`, …
    Num(usize),
    /// A level variable, bound by a `universe` declaration.
    Var(String),
    /// `l + n`.
    Add(Box<LevelExpr>, usize),
    /// `max l r`.
    Max(Box<LevelExpr>, Box<LevelExpr>),
    /// `imax l r` — `0` when `r` is `0`, `max l r` otherwise. The impredicative-Pi rule, held
    /// open while `r` is a variable.
    IMax(Box<LevelExpr>, Box<LevelExpr>),
}

impl Term {
    /// Position of the type-expression's root for error reporting.
    pub fn pos(&self) -> &Position {
        match self {
            Term::Ref { pos, .. }
            | Term::Arrow { pos, .. }
            | Term::BinderArrow { pos, .. }
            | Term::Pi { pos, .. }
            | Term::Sigma { pos, .. }
            | Term::Unit { pos, .. }
            | Term::Sort { pos, .. }
            | Term::Lambda { pos, .. }
            | Term::Alias { pos, .. }
            | Term::Ann { pos, .. }
            | Term::LitString { pos, .. }
            | Term::LitInt { pos, .. }
            | Term::LitFloat { pos, .. }
            | Term::LitBool { pos, .. } => pos,
        }
    }
}

/// A typed binder: `name : type`. Used by `Term::Pi` and by the
/// new typed `lambda` literal (D37 §3.1). The type can be any
/// `Term`, including nested `Pi` / `Ref` / `Arrow` forms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedParam {
    pub name: String,
    pub typ: Term,
    pub pos: Position,
}

/// D37 §3.3 — `merge_comorphism <iri> for <class> { <body> }`.
/// Compiles to a `MergeComorphism` resource (with the
/// `merge_target_class` and `merge_transformation` slots populated)
/// plus, for the inline-body form, a synthesised standalone Lambda
/// resource at a content-hash IRI.
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

/// D43 §3.1 — `text_index <iri> { … }`. Sugar over a `core:TextIndex`
/// Resource declaration; the class is implicit in the keyword and
/// the body fields populate the Resource's properties (`target_property`,
/// `text_analyzer`). The compiler is responsible for the lowering to a
/// regular Resource at commit time.
#[derive(Debug)]
pub struct TextIndexDecl {
    pub name: QualifiedName,
    pub body: Vec<ResourceField>,
    pub pos: Position,
}

/// D43 §3.1 — `vector_index <iri> { … }`. Sugar over a `core:VectorIndex`
/// Resource declaration; the class is implicit in the keyword and the
/// body fields populate the Resource's properties (`target_property`,
/// `vec_model`, `vec_dim`, `vec_distance`, `vec_strategy`, `vec_hnsw_m`,
/// `vec_hnsw_ef_construction`, `vec_embedding_policy`). The compiler is
/// responsible for the lowering to a regular Resource at commit time.
#[derive(Debug)]
pub struct VectorIndexDecl {
    pub name: QualifiedName,
    pub body: Vec<ResourceField>,
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
        param_type: Option<Term>,
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

    /// `match expr [returning <motive>] { ctor -> body; ctor(x, y) -> body; ... }`
    /// (Phase 11b step 11–12, D19 §10; extended in eigenius#72 Layer 3).
    ///
    /// Pattern-matches a value of an inductive type. Each arm names
    /// a constructor and (optionally) binds variables for its
    /// arguments.
    ///
    /// `returning` is the optional motive. When present, the
    /// kernel-side expression builder desugars to `Exp::InductiveRec`
    /// with the supplied motive. When absent, it produces `Exp::Match`
    /// and the type checker synthesises the motive from the checking
    /// context — inference-mode use without `returning` is a type
    /// error with a clear diagnostic.
    ///
    /// Two motive shapes are accepted in source:
    /// - A bare `Term::Ref` (qualified name) — desugars to the
    ///   constant motive `λ_. T`. This is the pre-Layer-3 surface and
    ///   stays supported for non-indexed inductives.
    /// - A `Term::Lambda` (`fun (i : T) => body`) — used as the
    ///   motive directly, abstracting over the scrutinee's indices.
    ///   Required when matching on an indexed inductive whose result
    ///   type depends on those indices.
    Match {
        scrutinee: Box<Expr>,
        returning: Option<Term>,
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

/// A literal value in expression position.
#[derive(Debug)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}
