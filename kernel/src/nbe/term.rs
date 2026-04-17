//! Mini-TT syntax terms.
//!
//! Ported from `Core/Abs.hs` in the Mini-TT reference implementation,
//! extended with Eigon ground types.

use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;

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

    /// List type as a recursive Sum: Sum(nil 1 | cons (A × List A))
    /// For now, represented as a ground type marker.
    pub fn list(element_type: Exp) -> Exp {
        // Simplified: lists are treated as a primitive construct
        // rather than encoding via recursive types
        Exp::Data(vec![
            Summand {
                name: "nil".to_string(),
                typ: Exp::One,
            },
            Summand {
                name: "cons".to_string(),
                typ: Exp::times(element_type, Exp::Var("__list_tail".to_string())),
            },
        ])
    }
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
}
