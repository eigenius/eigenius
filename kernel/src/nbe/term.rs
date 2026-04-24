//! Mini-TT syntax terms.
//!
//! Ported from `Core/Abs.hs` in the Mini-TT reference implementation,
//! extended with Eigon ground types.

use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::sync::{Arc, OnceLock};

pub type Name = String;

/// Expressions — the syntax of Mini-TT.
#[derive(Debug, Clone, PartialEq)]
pub enum Exp {
    /// Lambda: λ p. e
    Lam(Patt, Box<Exp>),
    /// Universe: U (the type of types). Level 0 by default.
    Set,
    /// Universe at a specific level: Type(n). Type(0) = Set.
    /// Type(0) : Type(1) : Type(2)
    Type(usize),
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
    /// Eigon primitive type
    EigonPrimitive(PrimitiveType),
    /// A concrete Eigon resource value
    EigonResource(Box<Resource>),
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
    pub name: Name,
    /// Parameter telescope shared by every constructor: `(x₁ : A₁) … (xₙ : Aₙ)`.
    pub params: Vec<(Patt, Exp)>,
    /// Universe of the type former — typically `Exp::Set` or `Exp::Type(n)`.
    pub sort: Exp,
    pub ctors: Vec<InductiveCtorDecl>,
}

impl PartialEq for InductiveDecl {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
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
    let self_ref = Arc::new(InductiveDecl {
        name: "List".to_string(),
        params: Vec::new(),
        sort: Exp::Set,
        ctors: Vec::new(),
    });
    let list_a_typ = Exp::InductiveType(self_ref, vec![Exp::Var("A".to_string())]);
    Arc::new(InductiveDecl {
        name: "List".to_string(),
        params: vec![(Patt::Var("A".to_string()), Exp::Set)],
        sort: Exp::Set,
        ctors: vec![
            // nil : Π A:Set. List A
            InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Set),
                    Box::new(list_a_typ.clone()),
                ),
            },
            // cons : Π A:Set. A → List A → List A
            InductiveCtorDecl {
                name: "cons".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Set),
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
        let t = Exp::arrow(Exp::One, Exp::Set);
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
        let t = Exp::list(Exp::Set);
        match t {
            Exp::InductiveType(decl, params) => {
                assert_eq!(decl.name, "List");
                assert_eq!(decl.ctors.len(), 2);
                assert_eq!(decl.ctors[0].name, "nil");
                assert_eq!(decl.ctors[1].name, "cons");
                assert_eq!(params.len(), 1);
                assert!(matches!(params[0], Exp::Set));
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
}
