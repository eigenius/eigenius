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

//! EigenTT syntax terms.
//!
//! Ported from `Core/Abs.hs` in the EigenTT reference implementation,
//! extended with Eigon ground types.

use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::sync::{Arc, OnceLock};

pub type Name = String;

/// Expressions — the syntax of EigenTT.
#[derive(Debug, Clone, PartialEq)]
pub enum Exp {
    /// Lambda: λ p. e
    Lam(Patt, Box<Exp>),
    /// Universe at a specific level: Sort(n).
    /// `Sort(0) = Prop`, `Sort(1) = Set` (the universe of small types),
    /// `Sort(n+1)` corresponds to the former `Type(n)` for `n >= 1`.
    /// Typing rule: `Sort(n) : Sort(n+1)`. See D46 §3.
    Sort(usize),
    /// Dependent function type: Π p : A. B
    Pi(Patt, Box<Exp>, Box<Exp>),
    /// Dependent pair type: Σ p : A. B
    Sig(Patt, Box<Exp>, Box<Exp>),
    /// Unit type: 1
    One,
    /// Unit value: ()
    Unit,
    /// Pair value: (e₁, e₂)
    Pair(Box<Exp>, Box<Exp>),
    /// Constructor: $c e
    Con(Name, Box<Exp>),
    /// Sum type: Sum(c₁ A₁ | c₂ A₂ | ...)
    Data(Vec<Summand>),
    /// Case function: fun(c₁ → e₁ | c₂ → e₂ | ...)
    Case(Vec<Branch>),
    /// First projection: e.1
    Fst(Box<Exp>),
    /// Second projection: e.2
    Snd(Box<Exp>),
    /// Application: e₁ e₂
    App(Box<Exp>, Box<Exp>),
    /// Type annotation: `(e : T)` — the bidirectional mode switch that lets a
    /// *checkable* term (e.g. a Curry-style `Lam`, which has no synthesizable
    /// type) appear in *inference* position. `check_infer(Ann(e, T))` checks `e`
    /// against `T` and returns `T`; `eval` is runtime-erased (`eval(Ann(e,_)) =
    /// eval(e)`), so NbE normal forms never contain `Ann`. See D46 (bidirectional
    /// typing) / D63 §8.2 (the determiner λ-semantics need to commit-check).
    Ann(Box<Exp>, Box<Exp>),
    /// Variable: x
    Var(Name),
    /// Declaration followed by expression: let/letrec d; e
    Dec(Decl, Box<Exp>),

    // --- Eigenius extensions ---
    /// Identity type: Id(A, x, y) — propositional equality
    Id(Box<Exp>, Box<Exp>, Box<Exp>),
    /// Reflexivity proof: refl(a) : Id(A, a, a)
    Refl(Box<Exp>),
    /// J eliminator: J(A, C, d, x, y, p) where p : Id(A, x, y)
    IdJ(Box<[Exp; 6]>),

    /// Native constraint check: NativeDecide(constraint, value) reduces to
    /// Refl if the constraint is satisfied, or a neutral if not.
    /// Used for min_value, max_value, pattern, format, etc.
    NativeDecide(Constraint, Box<Exp>),

    /// Decidable equality: DecEq(A, x, y) reduces to Refl if x = y,
    /// or a neutral term if undecidable. Works on ground types (String,
    /// Integer, Float, Boolean, IRI).
    DecEq(Box<Exp>, Box<Exp>, Box<Exp>),

    /// Non-dependent function type: A → B (sugar for Π _ : A. B)
    Arrow(Box<Exp>, Box<Exp>),
    /// Non-dependent pair type: A × B (sugar for Σ _ : A. B)
    Times(Box<Exp>, Box<Exp>),
    /// Eigon class ground type: resolved from layer chain
    EigonClass(Iri),
    /// Reference to a chain-resident `eigentt:Axiom` resource. An axiom
    /// is an opaque typed constant — the IRI carries no body, only the
    /// type registered in the chain's [`crate::program::axiom_env::AxiomEnv`].
    /// `check_infer` looks the IRI up in the layer's cached
    /// `axiom_env()` to recover the registered type; `eval` /
    /// `readback` are identity (axioms have no reduction rules), and
    /// the D47 codec round-trips it as `ConstRef(iri)` exactly like
    /// `EigonClass`. Parallels D46 §10 + the encoding-probe in
    /// `crates/eigenius-statistics/tests/axiom_encoding_probe.rs`.
    EigonAxiom(Iri),
    /// Eigon primitive type
    EigonPrimitive(PrimitiveType),
    /// A concrete Eigon resource value
    EigonResource(Box<Resource>),
    /// Literal string value at the expression level (D49 / eigenius#71).
    /// Type: `Exp::EigonPrimitive(PrimitiveType::String)`. Distinct from
    /// `Exp::Template`, which carries embedded property references; a
    /// `LitString` is a closed string literal with no interpolation.
    /// Authored to support D39 §4.1's `Asserts(iri)` and any other
    /// value-parameter inductive that takes string arguments at the
    /// type level. Round-trips through the D47 codec as the `LitString`
    /// ctor of `eigentt:TypeExpr` (eigenius#71).
    LitString(String),
    /// Literal integer value at the expression level (eigenius#71).
    /// Type: `Exp::EigonPrimitive(PrimitiveType::Integer)`. Same shape
    /// as `LitString` — a closed literal that round-trips through D47
    /// as `LitInt`. Sized at i64 to match `core:integer`'s 53-bit
    /// safe-integer range with headroom.
    LitInt(i64),
    /// Literal floating-point value at the expression level
    /// (eigenius#71). Type: `Exp::EigonPrimitive(PrimitiveType::Float)`.
    /// Round-trips through D47 as `LitFloat`.
    LitFloat(f64),
    /// Property access on a resource: e.property
    PropAccess(Box<Exp>, Iri),
    /// Template literal with extracted property references.
    /// Template("..{{iri1}}..{{iri2}}..", [(iri1, type1), (iri2, type2)])
    Template(String, Vec<(Iri, Box<Exp>)>),
    /// Construct a typed resource: Construct(class_iri, [(prop_iri, expr), ...])
    Construct(Iri, Vec<(Iri, Box<Exp>)>),

    // --- Codata (D11, Phase 9b-i) ---
    /// Codata type declaration: codata { obs₁ : T₁; obs₂ : T₂; ... }
    ///
    /// Dual of `Data`: defines a type by its observations rather than
    /// its constructors. The canonical example is
    /// `codata Stream A { head : A; tail : Stream A }`.
    Codata(Vec<Observation>),
    /// Codata value (copattern definition): corecord { obs₁ = e₁; obs₂ = e₂; ... }
    ///
    /// A corecord binds each observation to a body expression. The body
    /// is evaluated lazily, once per observation, in the corecord's
    /// captured environment. Productivity (each observation terminates)
    /// should be checked by a guardedness pass before running untrusted
    /// code; the evaluator itself does not enforce it.
    CoRecord(Vec<CoField>),
    /// Observation on a codata value: e.obs
    ///
    /// Picks the named field from a `CoRecord` and evaluates its body,
    /// or produces a blocked neutral if `e` is not yet a concrete
    /// corecord.
    Observe(Box<Exp>, Name),

    // --- Map/Reduce (Phase 11a) ---
    /// Map: apply a function to each element of a list.
    /// `Map(f, collection)` — type: `(A → B) → List A → List B`.
    /// Termination: structural over a finite list.
    Map(Box<Exp>, Box<Exp>),
    /// Reduce: fold a function over a list with an initial accumulator.
    /// `Reduce(f, initial, collection)` — type: `(B → A → B) → B → List A → B`.
    /// Termination: structural over a finite list.
    Reduce(Box<Exp>, Box<Exp>, Box<Exp>),

    // --- Inductive types (Phase 11b, D19) ---
    /// Introduce an inductive type declaration.
    /// Evaluating this form produces the type former; the declaration is
    /// shared with constructor and recursor occurrences via `Arc`.
    Inductive(Arc<InductiveDecl>),
    /// Inductive type applied to parameter expressions: `I(p₁, …, pₙ)`.
    InductiveType(Arc<InductiveDecl>, Vec<Exp>),
    /// Constructor application: `c(a₁, …, aₘ)` on the named inductive.
    InductiveCtor(Arc<InductiveDecl>, Name, Vec<Exp>),
    /// Recursor application: eliminate a value of the inductive with
    /// motive and one minor per constructor.
    InductiveRec {
        decl: Arc<InductiveDecl>,
        motive: Box<Exp>,
        minors: Vec<Exp>,
        major: Box<Exp>,
    },

    /// Pattern-match elimination with *motive inferred from context*
    /// (Phase 11b step 12, D19 §10). Each arm binds the constructor's
    /// arguments and evaluates a body. Unlike `InductiveRec`, no
    /// explicit motive is carried — the type checker synthesises
    /// `λ_. expected_type` from the checking-mode expected type.
    ///
    /// In inference mode this form has no known result type and is
    /// rejected with a diagnostic pointing to either `returning T`
    /// annotation or a checking-mode context.
    ///
    /// Evaluation is uniform with `InductiveRec`: on a constructor
    /// scrutinee we dispatch to the matching arm's body (instantiated
    /// with the constructor's arguments as bindings and the recursor's
    /// IHs for recursive args); on a neutral scrutinee we produce a
    /// blocked `Neut::NtMatch`.
    Match {
        scrutinee: Box<Exp>,
        arms: Vec<MatchArm>,
    },

    // --- Sized types (Phase 11b step 14, D19 §8) ---
    /// `SizeSort` — the sort of size expressions. Inhabited by
    /// `SizeInf` and applications of `SizeSucc`. Itself a type
    /// (`SizeSort : Type(1)`).
    ///
    /// Sizes are used as termination/productivity indices on
    /// inductive and coinductive types: `List(A, i)` denotes a
    /// list-at-size-i, where `i` strictly decreases on each
    /// recursive call (inductives) or strictly increases on each
    /// observation (codata). This step lands the primitives only;
    /// constraint generation against inductives is Phase 11b step 15.
    SizeSort,
    /// `SizeSucc(s)` — successor of a size: the next size strictly
    /// larger than `s`. The smallest enclosing size for a value
    /// produced by one constructor application.
    SizeSucc(Box<Exp>),
    /// `SizeInf` — the unbounded ("infinity") size. Used when no
    /// size discipline is enforced; sized inductive/coinductive
    /// definitions degenerate to the unsized form when their size
    /// argument is `SizeInf`.
    SizeInf,

    /// Applied codata type expression: `C(p₁, …, pₙ)` where `C` is
    /// declared by an `Arc<CodataDecl>` and the `Vec<Exp>` supplies
    /// the type arguments (including size arguments). Parallels
    /// `Exp::InductiveType`.
    ///
    /// Observation types inside the referenced decl may contain
    /// further `Exp::CodataType` values — in particular,
    /// self-references of the form `Exp::CodataType(self_ref_stub,
    /// new_args)` where `self_ref_stub` is an `Arc<CodataDecl>`
    /// whose only load-bearing field is its name (PartialEq on
    /// CodataDecl compares by name, so the stub unifies with the
    /// full declaration at evaluation time).
    CodataType(Arc<CodataDecl>, Vec<Exp>),

    /// Cross-institution translation via a declared comorphism (D14 §9.3).
    ///
    /// `comorphism_iri` identifies a `Comorphism` resource indexed by
    /// the [`InstitutionIndex`]; `source` is the expression producing
    /// the source-institution resource to translate. Evaluation runs
    /// the four-step pipeline — extract → transformation Component →
    /// reify — and the produced target-class resource is committed to
    /// the chain (D14 §9.3 step 4) before being wrapped as
    /// `Val::ResourceVal` for downstream evaluation.
    ///
    /// `target_iri` carries an optional explicit IRI override for the
    /// produced resource. `None` (the ESL default) instructs the kernel
    /// to assign a deterministic content-hash IRI of the form
    /// `urn:eigenius:comorphism-output:<comorphism-tail>:<hex>`. `Some`
    /// (set by EigenQL's `INTO` clause) commits the produced resource
    /// at the caller-named IRI.
    ///
    /// Without a institution index/runtime attached (bare
    /// `EvalCtx::Pure` used at type-check time), the expression
    /// reduces to a passthrough neutral so the conversion checker can
    /// compare two `InstitutionInvoke`s structurally. Runtime callers
    /// attach the index/runtime via an effectful `EvalCtx` (the IO or check-time institution engine).
    ///
    /// [`InstitutionIndex`]: crate::institution::registry::InstitutionIndex
    InstitutionInvoke {
        comorphism_iri: Iri,
        source: Box<Exp>,
        target_iri: Option<Iri>,
    },

    /// Bounded size Π-type: `Π {i < upper}. body` — the function
    /// type of a sized function that takes a size argument strictly
    /// smaller than `upper`.
    ///
    /// The binder `patt` has type `SizeSort` implicitly; the hypothesis
    /// `patt < upper` is registered in the type-checker's rigid
    /// hypothesis tracker (TSO) when `body` is checked. Applying a
    /// value of this type to a size `i` requires proving
    /// `size_lt(i, upper)` — either structurally (`i = SizeSucc(..)`
    /// making ∞-absorption trivial) or via the hypothesis chain.
    ///
    /// `upper` must normalise to a rigid size variable or `SizeInf`
    /// — the TSO can only track hypotheses rooted at rigid nodes.
    /// Composite upper bounds like `{i < ŝ j}` are rejected in v1.
    SizedPi {
        patt: Patt,
        upper: Box<Exp>,
        body: Box<Exp>,
    },
}

/// A single arm of an `Exp::Match`.
///
/// `ctor_name` is the local name of the constructor (matched against
/// `decl.ctors[i].name` during elimination). `bindings` lists the
/// binding patterns for the constructor's positional arguments, in
/// declaration order. Bindings may be `Patt::Var(name)` for named
/// access or `Patt::Unit` for wildcards. The IHs produced by the
/// recursor are currently bound anonymously — accessing them is the
/// job of a future "IH-aware match" extension.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub ctor_name: Name,
    pub bindings: Vec<Patt>,
    pub body: Exp,
}

/// Declarations.
#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    /// Non-recursive: let p : A = e
    Def(Patt, Box<Exp>, Box<Exp>),
    /// Recursive: letrec p : A = e
    Drec(Patt, Box<Exp>, Box<Exp>),
}

/// Patterns for binding.
#[derive(Debug, Clone, PartialEq)]
pub enum Patt {
    /// Pair pattern: (p₁, p₂)
    Pair(Box<Patt>, Box<Patt>),
    /// Wildcard: _
    Unit,
    /// Variable pattern: x
    Var(Name),
}

/// A branch of a Sum type: constructor name with its type.
#[derive(Debug, Clone, PartialEq)]
pub struct Summand {
    pub name: Name,
    pub typ: Exp,
}

/// A branch of a Case expression: constructor name with body.
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub name: Name,
    pub body: Exp,
}

/// A declared observation on a codata type: obs : T.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub name: Name,
    pub typ: Exp,
}

/// A copattern definition in a corecord: obs = e.
#[derive(Debug, Clone, PartialEq)]
pub struct CoField {
    pub name: Name,
    pub body: Exp,
}

/// A native constraint that can be checked at type-check time.
#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    /// Value >= minimum
    MinValue(i64),
    /// Value <= maximum
    MaxValue(i64),
    /// String length >= minimum
    MinLength(i64),
    /// String length <= maximum
    MaxLength(i64),
    /// String matches regex pattern
    Pattern(String),
    /// String matches a format (date, datetime, uuid, etc.)
    Format(String),
    /// Institution-dispatched constraint (D14 §9.2).
    ///
    /// The check-time reducer looks up `iri` as a Decidable QueryClass
    /// in the [`InstitutionIndex`]; if found, evaluates `args` to
    /// values, marshals them as a `decide_args` array onto a synthetic
    /// input resource, and dispatches via `Institution::query`. The
    /// returned `Verdict` resource is parsed into a [`DecResult`]:
    /// `Holds` reduces the surrounding `NativeDecide` to `Refl`,
    /// `Fails` emits a failing neutral, and `Undecidable` (or no
    /// matching QueryClass) stays as a passthrough neutral.
    ///
    /// [`InstitutionIndex`]: crate::institution::registry::InstitutionIndex
    /// [`DecResult`]: crate::institution::DecResult
    Institution { iri: Iri, args: Vec<Exp> },
}

/// Eigon primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    String,
    Integer,
    Float,
    Boolean,
    Json,
}

/// Declaration of an inductive type (Phase 11b, D19).
///
/// Carries the declaration inline in the AST; shared by value via `Arc`
/// so type / constructor / recursor occurrences of the same inductive
/// do not duplicate the telescope. Later phases may migrate this into
/// a top-level environment (nanoda_lib style); for now the inline
/// representation keeps the change local to the NbE evaluator.
///
/// Equality is defined by `name` alone — not structural. This matches
/// the name-based dispatch the kernel uses everywhere (iota reduction,
/// type checker arm, cross-inductive references). Semantically two
/// inductive declarations with the same name are the same inductive
/// (we don't support overloading). The practical payoff: a "stub"
/// `Arc<InductiveDecl>` carrying just a name can stand in for the
/// full declaration at use sites where the full ctor list isn't yet
/// available (self-references during ctor-type construction, cross-
/// inductive argument-type references) without breaking type-checker
/// equality. This was originally worked around with clever shared-Arc
/// tricks; the name-based `PartialEq` is the proper structural fix.
#[derive(Debug, Clone)]
pub struct InductiveDecl {
    /// Stable chain-resident identifier (gh #75). Same discipline as
    /// `core:Class`: the IRI uniquely identifies the inductive across
    /// every construction path (resolver, ESL stubs, test fixtures);
    /// the [`name`](Self::name) field below is a human-readable label.
    /// The D47 codec encoder writes this into `ConstRef` / `CtorApp`
    /// slots — using `name` there would produce decoder-incompatible
    /// short-name shapes for chain-resolved decls.
    pub iri: Iri,
    /// Human-readable short name. Used in diagnostic strings only.
    /// Same convenience role as `core:short_name` on `core:Class` —
    /// readable when unambiguous, but never the identifier.
    pub name: Name,
    /// Parameter telescope shared by every constructor: `(x₁ : A₁) … (xₙ : Aₙ)`.
    pub params: Vec<(Patt, Exp)>,
    /// Index telescope — varies per constructor (D48). Empty for non-
    /// indexed declarations (the default; matches D19's pre-D48 shape).
    /// Index expressions in constructor return types are checked against
    /// these telescope types, after substituting the parameter prefix.
    pub indices: Vec<(Patt, Exp)>,
    /// Universe of the type former — `Exp::Sort(n)`.
    pub sort: Exp,
    pub ctors: Vec<InductiveCtorDecl>,
}

impl PartialEq for InductiveDecl {
    fn eq(&self, other: &Self) -> bool {
        self.iri == other.iri
    }
}

impl InductiveDecl {
    /// Whether `typ` is a direct application of this inductive
    /// (`Exp::InductiveType(self, _)`) — the only shape of recursive
    /// constructor argument the Phase 11b/D19 iota reduction can
    /// eliminate. Higher-order or nested occurrences are rejected at
    /// positivity-check time, so this simple head check suffices for
    /// both recursor-type derivation and iota reduction.
    pub fn is_direct_recursive_ref(&self, typ: &Exp) -> bool {
        matches!(typ, Exp::InductiveType(d, _) if d.iri == self.iri)
    }
}

/// Coinductive (codata) declaration — the parameterised analogue of
/// the anonymous [`Exp::Codata`] form. Admits type parameters
/// (including `Size` parameters for sized codata) and supports
/// self-references in observation types via
/// [`Exp::CodataType`] with a name-only stub `Arc<CodataDecl>`.
///
/// `PartialEq` is name-based — mirrors `InductiveDecl`. This is what
/// lets an observation type declared as `Stream(A, j)` (encoded as
/// `Exp::CodataType(stub, …)`) unify with the full declaration when
/// the full decl is looked up through any `Arc<CodataDecl>` reference
/// with the same name.
#[derive(Debug, Clone)]
pub struct CodataDecl {
    /// Stable chain-resident identifier (gh #75). See [`InductiveDecl::iri`]
    /// for the discipline; same role here.
    pub iri: Iri,
    /// Human-readable short name. Diagnostics only.
    pub name: Name,
    /// Parameter telescope shared by every observation.
    pub params: Vec<(Patt, Exp)>,
    /// Universe of the type former.
    pub sort: Exp,
    pub observations: Vec<Observation>,
}

impl PartialEq for CodataDecl {
    fn eq(&self, other: &Self) -> bool {
        self.iri == other.iri
    }
}

/// A single constructor within an `InductiveDecl`.
#[derive(Debug, Clone, PartialEq)]
pub struct InductiveCtorDecl {
    pub name: Name,
    /// Full constructor type: a Π-telescope ending in an application
    /// of the parent inductive to its parameters.
    pub typ: Exp,
}

impl Patt {
    /// Check if a name is bound by this pattern.
    pub fn contains(&self, name: &str) -> bool {
        match self {
            Patt::Var(n) => n == name,
            Patt::Pair(p1, p2) => p1.contains(name) || p2.contains(name),
            Patt::Unit => false,
        }
    }
}

// --- Convenience constructors ---

impl Exp {
    /// Non-dependent function type: A → B
    pub fn arrow(a: Exp, b: Exp) -> Exp {
        Exp::Pi(Patt::Unit, Box::new(a), Box::new(b))
    }

    /// Non-dependent pair type: A × B
    pub fn times(a: Exp, b: Exp) -> Exp {
        Exp::Sig(Patt::Unit, Box::new(a), Box::new(b))
    }

    /// Result type: Sum(ok A | err E)
    pub fn result(ok_type: Exp, err_type: Exp) -> Exp {
        Exp::Data(vec![
            Summand {
                name: "ok".to_string(),
                typ: ok_type,
            },
            Summand {
                name: "err".to_string(),
                typ: err_type,
            },
        ])
    }

    /// List type: `List(element_type)` as a real inductive type
    /// (Phase 11b step 6, D19 §9). Backed by the canonical `List`
    /// inductive declaration from [`list_decl`].
    pub fn list(element_type: Exp) -> Exp {
        Exp::InductiveType(list_decl(), vec![element_type])
    }
}

/// Canonical `List(A)` inductive declaration, lazily built and shared.
///
/// Returns the same `Arc<InductiveDecl>` on every call so that all
/// list types and constructors throughout the kernel reference one
/// declaration. The inner self-reference inside the constructor types
/// uses the "stub Arc" pattern (an empty-ctors `Arc<InductiveDecl>`
/// with matching name) — Phase 11b's name-based lookups handle this
/// without needing genuinely cyclic Arc allocation.
pub fn list_decl() -> Arc<InductiveDecl> {
    static LIST_DECL: OnceLock<Arc<InductiveDecl>> = OnceLock::new();
    LIST_DECL.get_or_init(build_list_decl).clone()
}

fn build_list_decl() -> Arc<InductiveDecl> {
    let list_iri = Iri::parse("urn:eigenius:core:List").expect("static List IRI");
    let self_ref = Arc::new(InductiveDecl {
        iri: list_iri.clone(),
        name: "List".to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: Vec::new(),
    });
    let list_a_typ = Exp::InductiveType(self_ref, vec![Exp::Var("A".to_string())]);
    Arc::new(InductiveDecl {
        iri: list_iri,
        name: "List".to_string(),
        params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: vec![
            // nil : Π A:Set. List A
            InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(list_a_typ.clone()),
                ),
            },
            // cons : Π A:Set. A → List A → List A
            InductiveCtorDecl {
                name: "cons".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(Exp::Var("A".to_string())),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(list_a_typ.clone()),
                            Box::new(list_a_typ),
                        )),
                    )),
                ),
            },
        ],
    })
}

/// Canonical `Option(A)` inductive declaration, lazily built and shared.
///
/// Used by the merge-witness type-check (Phase 15b step 3, D20 §6.1):
/// a `MergeComorphism`'s transformation must have signature
/// `(A, A, Option(A)) -> A`, where the third argument carries the
/// optional ancestor value. Same stub-Arc / name-based-equality
/// pattern as [`list_decl`].
pub fn option_decl() -> Arc<InductiveDecl> {
    static OPTION_DECL: OnceLock<Arc<InductiveDecl>> = OnceLock::new();
    OPTION_DECL.get_or_init(build_option_decl).clone()
}

fn build_option_decl() -> Arc<InductiveDecl> {
    let option_iri = Iri::parse(crate::ontology::well_known::OPTION).expect("static Option IRI");
    let self_ref = Arc::new(InductiveDecl {
        iri: option_iri.clone(),
        name: "Option".to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: Vec::new(),
    });
    let option_a_typ = Exp::InductiveType(self_ref, vec![Exp::Var("A".to_string())]);
    Arc::new(InductiveDecl {
        iri: option_iri,
        name: "Option".to_string(),
        params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
        indices: Vec::new(),
        sort: Exp::Sort(1),
        ctors: vec![
            // none : Π A:Set. Option A
            InductiveCtorDecl {
                name: "none".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(option_a_typ.clone()),
                ),
            },
            // some : Π A:Set. A → Option A
            InductiveCtorDecl {
                name: "some".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(Exp::Var("A".to_string())),
                        Box::new(option_a_typ),
                    )),
                ),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_contains() {
        let p = Patt::Var("x".to_string());
        assert!(p.contains("x"));
        assert!(!p.contains("y"));
    }

    #[test]
    fn pattern_pair_contains() {
        let p = Patt::Pair(
            Box::new(Patt::Var("a".to_string())),
            Box::new(Patt::Var("b".to_string())),
        );
        assert!(p.contains("a"));
        assert!(p.contains("b"));
        assert!(!p.contains("c"));
    }

    #[test]
    fn arrow_desugars_to_pi() {
        let t = Exp::arrow(Exp::One, Exp::Sort(1));
        assert!(matches!(t, Exp::Pi(Patt::Unit, _, _)));
    }

    #[test]
    fn result_type() {
        let t = Exp::result(Exp::One, Exp::One);
        if let Exp::Data(summands) = t {
            assert_eq!(summands.len(), 2);
            assert_eq!(summands[0].name, "ok");
            assert_eq!(summands[1].name, "err");
        } else {
            panic!("expected Data");
        }
    }

    #[test]
    fn list_uses_canonical_inductive() {
        // Phase 11b step 6: Exp::list() now produces an inductive
        // type application backed by the canonical List declaration.
        let t = Exp::list(Exp::Sort(1));
        match t {
            Exp::InductiveType(decl, params) => {
                assert_eq!(decl.name, "List");
                assert_eq!(decl.ctors.len(), 2);
                assert_eq!(decl.ctors[0].name, "nil");
                assert_eq!(decl.ctors[1].name, "cons");
                assert_eq!(params.len(), 1);
                assert!(matches!(params[0], Exp::Sort(1)));
            }
            other => panic!("expected InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn list_decl_is_shared_across_calls() {
        // OnceLock caches the canonical Arc — every call returns the
        // same allocation by ptr identity.
        let a = list_decl();
        let b = list_decl();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn option_decl_shape() {
        let d = option_decl();
        assert_eq!(d.name, "Option");
        assert_eq!(d.params.len(), 1);
        assert!(matches!(d.params[0].0, Patt::Var(ref s) if s == "A"));
        assert_eq!(d.ctors.len(), 2);
        assert_eq!(d.ctors[0].name, "none");
        assert_eq!(d.ctors[1].name, "some");
    }

    #[test]
    fn option_decl_is_shared_across_calls() {
        let a = option_decl();
        let b = option_decl();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
