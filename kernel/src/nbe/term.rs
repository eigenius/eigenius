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
    /// Universe at a level: `Sort(l)`.
    ///
    /// `Sort(Zero) = Prop`, `Sort(Succ(Zero)) = Set`, and `Sort(Succ^{k+1}(Zero))` is the
    /// surface's `Type k`. Typing rule: `Sort(l) : Sort(Succ(l))`. See D46 §3.
    ///
    /// Carried a `usize` until eigenius#188. A [`Level`](crate::nbe::level::Level) may also be a
    /// `Max`, an `IMax` or a `Param`, which is what lets one declaration serve every rung instead
    /// of one declaration per rung — each of which was a bootstrap edit and a reseed. Sites that
    /// only ever see concrete levels read them back with `Level::as_nat`.
    Sort(crate::nbe::level::Level),
    /// Dependent function type: Π p : A. B
    Pi(Patt, Box<Exp>, Box<Exp>),
    /// Dependent pair type: Σ p : A. B
    Sig(Patt, Box<Exp>, Box<Exp>),
    /// D76 Phase B1 — a reference to a chain-resident declaration, by name.
    ///
    /// **The form nanoda uses.** `ind_consts.push(mk_const(ind.name, uparams))`
    /// (`references/nanoda_lib/src/inductive.rs:506`): a constructor's type names
    /// its own inductive with an ordinary `Const`, the same form as any other
    /// reference. There is no hollowed-out declaration because none is needed.
    ///
    /// **What it replaces.** Eigenius wrote a *stub* — an `Arc<InductiveDecl>`
    /// with the parts it could not supply left empty — because
    /// `Exp::InductiveType`'s slot holds a declaration, so a self-reference had
    /// to *be* one. Three sites then disagreed about which parts "could not
    /// supply" means (D76 §8 Phase B), because the concept was never defined:
    /// D19 does not mention it and D48 records it only as a preserved artifact.
    ///
    /// **`levels` is empty until #188's residual.** The wire form already carries
    /// the IRI — `encode_type_json` emits `ConstRef(iri)` for an
    /// `InductiveType` — so a level-free `Const` round-trips through the
    /// existing codec unchanged. Levels are what makes this a chain-format
    /// change, and they are Phase E2.
    Const(Iri, Vec<crate::nbe::level::Level>),
    /// Record type: a **named**, canonically-ordered dependent telescope (D78 §1).
    ///
    /// Each entry is `(field IRI, binder, field type)`, and a later field's type
    /// may mention earlier binders — the same dependency `Sig` expresses, but
    /// keyed by IRI rather than by position.
    ///
    /// **Canonical order is an invariant**: the topological order induced by the
    /// dependency relation, ties broken by IRI. Because `eq_nf` compares by
    /// readback and syntactic equality, canonical order is what makes two
    /// spellings of the same field set compare equal without a bespoke
    /// conversion arm. Build with [`Exp::record`], which establishes it.
    ///
    /// Distinct from `Sig`, which survives for *anonymous* pairs (`Exp::Times`).
    /// Records subsume the class use of `Sig`, not the pair use — D78 §5.1.
    Record(Vec<(Iri, Patt, Exp)>),
    /// Refinement: a record type together with the **set** of class constraints
    /// it satisfies (D78 §3). `Construct` returns one of these (D75 §8 Q7, 7b).
    ///
    /// A set, not a nest. `is_a` is a list, so a record may satisfy several
    /// classes; `Refine(R, {C, D})` is the direct image of that, has one
    /// representation where nesting would have two, and degenerates to `R` at
    /// the empty set — which is the "0 or more constraints" of D75 §6.3 with no
    /// special case.
    ///
    /// The constraint set is carried as **IRIs**, not as resolved field sets.
    /// Nominal identity (D75 §8 Q2) requires it: `Refine(R, {Alpha})` and
    /// `Refine(R, {Beta})` must differ even when `Alpha` and `Beta` have the same
    /// fields — which, measured, is the case for 749 of 894 shipped classes.
    Refine(Box<Exp>, std::collections::BTreeSet<Iri>),
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
    /// Literal boolean value at the expression level (eigenius#142).
    /// Type: `Exp::EigonPrimitive(PrimitiveType::Boolean)`. Same shape
    /// as `LitString` / `LitInt` / `LitFloat` — a closed literal that
    /// round-trips through D47 as the `LitBool` ctor of
    /// `eigentt:TypeExpr`. Added so `program:Literal` booleans decode
    /// to their value rather than to `EigonPrimitive(Boolean)`.
    LitBool(bool),
    /// Property access on a resource: e.property
    PropAccess(Box<Exp>, Iri),
    /// Template literal with extracted property references.
    /// Template("..{{iri1}}..{{iri2}}..", [(iri1, type1), (iri2, type2)])
    Template(String, Vec<(Iri, Box<Exp>)>),
    /// Construct a typed resource: Construct(class_iri, [(prop_iri, expr), ...])
    Construct(Iri, Vec<(Iri, Box<Exp>)>),

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
    //
    // D76 Phase B — `Inductive(decl)` and `InductiveType(decl, args)` are gone.
    // A reference to an inductive is `Const(iri, levels)`, applied through `App`
    // like any other reference (`Exp::const_applied` / `Exp::as_const_spine`).
    //
    // They were two spellings of one thing: `Inductive(d)` evaluated to the same
    // `Val::InductiveType` as `InductiveType(d, [])`, and a negative occurrence
    // written in the first form once evaded positivity checking
    // (`positivity::rejects_disguised_inductive_negative_occurrence`). Neither had
    // a slot for a level argument, which is what blocked #188's residual.
    /// Constructor application: `c(a₁, …, aₘ)` on the **named** inductive.
    ///
    /// D76 Phase B: the inductive's IRI, not its declaration. A constructor's
    /// identity is `(inductive IRI, constructor name)`, which is what the D47 wire
    /// has always carried — `CtorApp(D, c)` plus an `App` spine. Holding the
    /// declaration here meant the codec had to *decode* the target inductive, every
    /// constructor type of it, to build a reference; and for a self-reference it
    /// had nothing to decode yet, which is what the stub was for.
    ///
    /// nanoda goes further and gives each constructor its own `Const`, but its
    /// constructors are environment entries with names. Here they are not
    /// chain-resident — `InductiveCtorDecl { name, typ }` lives *inside* the
    /// inductive's resource and has no IRI — so minting constructor IRIs is a
    /// chain-format change and belongs with E2.
    InductiveCtor(Iri, Name, Vec<Exp>),
    /// Recursor application: eliminate a value of the inductive with
    /// motive and one minor per constructor.
    InductiveRec {
        /// The inductive's IRI (D76 Phase B), resolved through `Γ_env`.
        iri: Iri,
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
    /// `EvalCtx::pure()` used at type-check time), the expression
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
/// Equality is defined by `iri` alone — not structural. (The comment
/// here said `name` until eigenius#199; the impl below has compared
/// `iri` since gh #75 made the IRI the identifier and demoted `name`
/// to a diagnostic label.) This matches the identity-based dispatch the
/// kernel uses everywhere (iota reduction, type checker arm,
/// cross-inductive references). Semantically two inductive
/// declarations with the same IRI are the same inductive (we don't
/// support overloading). The practical payoff: a "stub"
/// `Arc<InductiveDecl>` carrying just the IRI can stand in for the
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
    /// **Universe parameters** — the level variables this declaration binds
    /// (eigenius#188, D76 Phase E2). nanoda's `uparams`
    /// (`references/nanoda_lib/src/env.rs:38`).
    ///
    /// **Ordered, and a duplicate is a bug**, because a reference instantiates by
    /// position: `Const(iri, levels)` substitutes `levels[i]` for `uparams[i]`.
    /// nanoda asserts the same at declaration admission (`no_dupes_all_params`,
    /// `tc.rs:167`).
    ///
    /// Empty for every monomorphic declaration, which is all ten shipped ones —
    /// so `subst` over an empty list is the identity and the common path is
    /// untouched.
    pub uparams: Vec<Name>,
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

impl InductiveDecl {
    /// Instantiate this declaration's universe parameters with `levels`
    /// (eigenius#188, D76 Phase E2) — nanoda's `subst_declar_info_levels`.
    ///
    /// **By position**, against `uparams`. Returns the declaration unchanged when
    /// it binds nothing, which is every shipped one, so the common path allocates
    /// nothing.
    ///
    /// **Arity is the caller's to check.** A wrong-length level list is a type
    /// error at the reference site, where the diagnostic can name the reference;
    /// silently padding or truncating here would turn `List.{}` into `List.{0}`
    /// and lose exactly the distinction Phase B built the slot for.
    pub fn instantiate_levels(&self, levels: &[crate::nbe::level::Level]) -> InductiveDecl {
        if self.uparams.is_empty() {
            return self.clone();
        }
        let ks = &self.uparams;
        InductiveDecl {
            // The parameters are CONSUMED by instantiation: the result is a
            // monomorphic declaration, and leaving them would let it be
            // instantiated twice.
            uparams: Vec::new(),
            iri: self.iri.clone(),
            name: self.name.clone(),
            params: self
                .params
                .iter()
                .map(|(p, k)| (p.clone(), k.subst_levels(ks, levels)))
                .collect(),
            indices: self
                .indices
                .iter()
                .map(|(p, k)| (p.clone(), k.subst_levels(ks, levels)))
                .collect(),
            sort: self.sort.subst_levels(ks, levels),
            ctors: self
                .ctors
                .iter()
                .map(|c| InductiveCtorDecl {
                    name: c.name.clone(),
                    typ: c.typ.subst_levels(ks, levels),
                })
                .collect(),
        }
    }
}

impl PartialEq for InductiveDecl {
    fn eq(&self, other: &Self) -> bool {
        self.iri == other.iri
    }
}

// `InductiveDecl::is_direct_recursive_ref` lived here until eigenius#92. Its doc claimed the head
// check "suffices for both recursor-type derivation and iota reduction" BECAUSE higher-order
// occurrences were rejected at positivity-check time — and once positivity started admitting them
// that premise was gone, leaving a second definition of "recursive occurrence" for the two halves
// of the eliminator to drift apart on. Replaced by `nbe::positivity::recursive_arg_shape`, which
// all three sites now consult.

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

/// Why a record could not be built in canonical order (D78 §1).
///
/// A cycle is a malformed *class declaration* — the dependency edges come from
/// `class_types` references and `when_property` conditions, both ontology data —
/// so the primary gate is a validation rule on the commit path. This type is the
/// kernel's defence in depth: [`Exp::record`] returns an error rather than
/// panicking, so a hand-built record cannot smuggle one past.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// Fields whose types depend on each other, directly or transitively.
    DependencyCycle(Vec<Iri>),
    /// The same field IRI appears twice. Union semantics has no reading for this.
    DuplicateField(Iri),
    /// Two fields bind the same name, so a later type mentioning it is ambiguous.
    DuplicateBinder {
        name: String,
        first: Iri,
        second: Iri,
    },
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DependencyCycle(iris) => {
                let names: Vec<&str> = iris.iter().map(|i| i.as_str()).collect();
                write!(
                    f,
                    "record field dependency cycle among: {}",
                    names.join(", ")
                )
            }
            Self::DuplicateField(iri) => write!(f, "duplicate record field `{iri}`"),
            Self::DuplicateBinder {
                name,
                first,
                second,
            } => write!(
                f,
                "fields `{first}` and `{second}` both bind `{name}`; a later field's type \
                 mentioning it would be ambiguous"
            ),
        }
    }
}

impl std::error::Error for RecordError {}

impl Exp {
    /// `Sort` at the numeral level `n` — `sort(0)` is `Prop`, `sort(1)` is `Set`.
    ///
    /// The ergonomic constructor for the monomorphic case, which is 942 of the 944 sort uses in
    /// the tree. A polymorphic sort is built with `Exp::Sort(Level::Param(..))` or one of the
    /// other [`Level`](crate::nbe::level::Level) forms directly.
    pub fn sort(n: usize) -> Exp {
        Exp::Sort(crate::nbe::level::Level::of_nat(n))
    }

    /// A reference to a declaration, applied to arguments —
    /// `App(App(Const(iri), a₁), a₂)`.
    ///
    /// The de-inlined form of what `Exp::const_applied(decl.iri.clone(), Vec::new(), args)` fused into a
    /// single node (D76 §8 Phase B). It is also what the D47 wire has always
    /// carried: the encoder emits `ConstRef` plus an `App` spine, so this
    /// constructor produces the shape the codec already round-trips.
    pub fn const_applied(iri: Iri, levels: Vec<crate::nbe::level::Level>, args: Vec<Exp>) -> Exp {
        args.into_iter().fold(Exp::Const(iri, levels), |head, arg| {
            Exp::App(Box::new(head), Box::new(arg))
        })
    }

    /// Peel an application spine down to a [`Exp::Const`] head, returning the
    /// head's IRI and levels and the arguments **in application order**.
    ///
    /// `None` when the head is anything else. The reversal matters: a spine peels
    /// outermost-first, so the arguments come off backwards relative to how they
    /// were applied — a caller that pairs them against a parameter telescope
    /// without reversing pairs each argument with the wrong binder.
    pub fn as_const_spine(&self) -> Option<(&Iri, &[crate::nbe::level::Level], Vec<&Exp>)> {
        let mut args: Vec<&Exp> = Vec::new();
        let mut head = self;
        while let Exp::App(f, x) = head {
            args.push(x.as_ref());
            head = f.as_ref();
        }
        let Exp::Const(iri, levels) = head else {
            return None;
        };
        args.reverse();
        Some((iri, levels.as_slice(), args))
    }

    /// Substitute level arguments for level parameters throughout this expression
    /// (eigenius#188, D76 Phase E2).
    ///
    /// **Positional**, matching `Level::subst`: `vs[i]` replaces `Param(ks[i])`.
    /// A `Param` not in `ks` is left alone, which is what makes this safe to run
    /// over a term that mentions an outer declaration's parameters.
    ///
    /// Only two variants carry a level — `Sort` and `Const` — and everything else
    /// either has none or is a container. **Written without a catch-all arm on
    /// purpose:** a new level-carrying variant must then fail to compile here
    /// rather than silently keeping an uninstantiated `Param`, which is the failure
    /// mode this whole phase exists to make impossible.
    pub fn subst_levels(&self, ks: &[Name], vs: &[crate::nbe::level::Level]) -> Exp {
        if ks.is_empty() {
            return self.clone();
        }
        let go = |e: &Exp| e.subst_levels(ks, vs);
        let bx = |e: &Exp| Box::new(e.subst_levels(ks, vs));
        match self {
            Exp::Sort(l) => Exp::Sort(l.subst(ks, vs)),
            Exp::Const(iri, levels) => Exp::Const(
                iri.clone(),
                levels.iter().map(|l| l.subst(ks, vs)).collect(),
            ),

            Exp::Lam(p, b) => Exp::Lam(p.clone(), bx(b)),
            Exp::Pi(p, a, b) => Exp::Pi(p.clone(), bx(a), bx(b)),
            Exp::Sig(p, a, b) => Exp::Sig(p.clone(), bx(a), bx(b)),
            Exp::Record(fs) => Exp::Record(
                fs.iter()
                    .map(|(i, p, t)| (i.clone(), p.clone(), go(t)))
                    .collect(),
            ),
            Exp::Refine(c, s) => Exp::Refine(bx(c), s.clone()),
            Exp::Pair(a, b) => Exp::Pair(bx(a), bx(b)),
            Exp::Con(n, b) => Exp::Con(n.clone(), bx(b)),
            Exp::Data(ss) => Exp::Data(
                ss.iter()
                    .map(|s| Summand {
                        name: s.name.clone(),
                        typ: go(&s.typ),
                    })
                    .collect(),
            ),
            Exp::Case(bs) => Exp::Case(
                bs.iter()
                    .map(|b| Branch {
                        name: b.name.clone(),
                        body: go(&b.body),
                    })
                    .collect(),
            ),
            Exp::Fst(a) => Exp::Fst(bx(a)),
            Exp::Snd(a) => Exp::Snd(bx(a)),
            Exp::App(f, a) => Exp::App(bx(f), bx(a)),
            Exp::Ann(a, t) => Exp::Ann(bx(a), bx(t)),
            Exp::Dec(d, b) => Exp::Dec(
                match d {
                    Decl::Def(p, t, v) => Decl::Def(p.clone(), bx(t), bx(v)),
                    Decl::Drec(p, t, v) => Decl::Drec(p.clone(), bx(t), bx(v)),
                },
                bx(b),
            ),
            Exp::Id(a, x, y) => Exp::Id(bx(a), bx(x), bx(y)),
            Exp::Refl(a) => Exp::Refl(bx(a)),
            Exp::IdJ(six) => Exp::IdJ(Box::new([
                go(&six[0]),
                go(&six[1]),
                go(&six[2]),
                go(&six[3]),
                go(&six[4]),
                go(&six[5]),
            ])),
            Exp::NativeDecide(c, b) => Exp::NativeDecide(c.clone(), bx(b)),
            Exp::DecEq(a, b, c) => Exp::DecEq(bx(a), bx(b), bx(c)),
            Exp::Arrow(a, b) => Exp::Arrow(bx(a), bx(b)),
            Exp::Times(a, b) => Exp::Times(bx(a), bx(b)),
            Exp::PropAccess(a, i) => Exp::PropAccess(bx(a), i.clone()),
            Exp::Template(t, fs) => Exp::Template(
                t.clone(),
                fs.iter().map(|(i, e)| (i.clone(), bx(e))).collect(),
            ),
            Exp::Construct(c, fs) => Exp::Construct(
                c.clone(),
                fs.iter().map(|(i, e)| (i.clone(), bx(e))).collect(),
            ),
            Exp::Map(f, c) => Exp::Map(bx(f), bx(c)),
            Exp::Reduce(f, i, c) => Exp::Reduce(bx(f), bx(i), bx(c)),
            Exp::InductiveCtor(iri, n, args) => {
                Exp::InductiveCtor(iri.clone(), n.clone(), args.iter().map(go).collect())
            }
            Exp::InductiveRec {
                iri,
                motive,
                minors,
                major,
            } => Exp::InductiveRec {
                iri: iri.clone(),
                motive: bx(motive),
                minors: minors.iter().map(go).collect(),
                major: bx(major),
            },
            Exp::Match { scrutinee, arms } => Exp::Match {
                scrutinee: bx(scrutinee),
                arms: arms
                    .iter()
                    .map(|a| MatchArm {
                        ctor_name: a.ctor_name.clone(),
                        bindings: a.bindings.clone(),
                        body: go(&a.body),
                    })
                    .collect(),
            },
            Exp::InstitutionInvoke {
                comorphism_iri,
                source,
                target_iri,
            } => Exp::InstitutionInvoke {
                comorphism_iri: comorphism_iri.clone(),
                source: bx(source),
                target_iri: target_iri.clone(),
            },

            // Level-free leaves.
            Exp::One
            | Exp::Unit
            | Exp::Var(_)
            | Exp::EigonClass(_)
            | Exp::EigonAxiom(_)
            | Exp::EigonPrimitive(_)
            | Exp::EigonResource(_)
            | Exp::LitString(_)
            | Exp::LitInt(_)
            | Exp::LitFloat(_)
            | Exp::LitBool(_) => self.clone(),
        }
    }

    /// Build an [`Exp::Record`] in **canonical order** (D78 §1), or report a
    /// dependency cycle.
    ///
    /// Canonical order is the topological order induced by the dependency
    /// relation — field `b` follows field `a` when `b`'s type mentions `a`'s
    /// binder — with ties broken by field IRI. Two properties make it the right
    /// invariant:
    ///
    /// - **Deterministic.** The same field set always yields the same telescope,
    ///   so `eq_nf`'s readback-and-compare decides record equality with no
    ///   bespoke conversion arm. This is what D78 §3.7 means by turning the
    ///   `BTreeMap`-ordering accident into a stated invariant.
    /// - **Dependency-respecting.** A field never precedes one its type mentions.
    ///   Sorting by IRI alone would not guarantee this, and rejecting the records
    ///   where it fails would be a wedge — the dependency is legitimate.
    ///
    /// Cycle detection is free: a cycle is exactly a topological sort that cannot
    /// place every field.
    pub fn record(fields: Vec<(Iri, Patt, Exp)>) -> Result<Exp, RecordError> {
        // Binder name → the index that binds it. A field's type depending on an
        // earlier binder is what induces an edge.
        let mut binder_of: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for (i, (_, patt, _)) in fields.iter().enumerate() {
            if let Patt::Var(name) = patt {
                if let Some(prev) = binder_of.insert(name.clone(), i) {
                    return Err(RecordError::DuplicateBinder {
                        name: name.clone(),
                        first: fields[prev].0.clone(),
                        second: fields[i].0.clone(),
                    });
                }
            }
        }
        let mut seen_iris: std::collections::BTreeSet<&Iri> = std::collections::BTreeSet::new();
        for (iri, _, _) in &fields {
            if !seen_iris.insert(iri) {
                return Err(RecordError::DuplicateField(iri.clone()));
            }
        }

        // deps[i] = the set of field indices field i's type depends on.
        let deps: Vec<std::collections::BTreeSet<usize>> = fields
            .iter()
            .map(|(_, _, ty)| {
                crate::nbe::subst::free_vars(ty)
                    .iter()
                    .filter_map(|v| binder_of.get(v).copied())
                    .collect()
            })
            .collect();

        // Kahn's algorithm, choosing the IRI-least ready field at each step so
        // the result is canonical and not merely valid.
        let mut placed: Vec<bool> = vec![false; fields.len()];
        let mut order: Vec<usize> = Vec::with_capacity(fields.len());
        while order.len() < fields.len() {
            let next = (0..fields.len())
                .filter(|&i| !placed[i] && deps[i].iter().all(|d| placed[*d]))
                .min_by(|&a, &b| fields[a].0.as_str().cmp(fields[b].0.as_str()));
            match next {
                Some(i) => {
                    placed[i] = true;
                    order.push(i);
                }
                None => {
                    let stuck: Vec<Iri> = (0..fields.len())
                        .filter(|&i| !placed[i])
                        .map(|i| fields[i].0.clone())
                        .collect();
                    return Err(RecordError::DependencyCycle(stuck));
                }
            }
        }

        let mut out = Vec::with_capacity(fields.len());
        for i in order {
            out.push(fields[i].clone());
        }
        Ok(Exp::Record(out))
    }

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
        Exp::const_applied(list_decl().iri.clone(), Vec::new(), vec![element_type])
    }
}

/// Canonical `List(A)` inductive declaration, lazily built and shared.
///
/// Returns the same `Arc<InductiveDecl>` on every call so that all
/// list types and constructors throughout the kernel reference one
/// declaration. The self-references inside the constructor types are
/// `Exp::Const` naming the IRI (D76 Phase B), which is why no stub
/// declaration is needed and no cyclic `Arc` allocation is either.
pub fn list_decl() -> Arc<InductiveDecl> {
    static LIST_DECL: OnceLock<Arc<InductiveDecl>> = OnceLock::new();
    LIST_DECL.get_or_init(build_list_decl).clone()
}

fn build_list_decl() -> Arc<InductiveDecl> {
    let list_iri = Iri::parse("urn:eigenius:core:List").expect("static List IRI");
    let list_a_typ = Exp::const_applied(
        list_iri.clone(),
        Vec::new(),
        vec![Exp::Var("A".to_string())],
    );
    Arc::new(InductiveDecl {
        uparams: Vec::new(),
        iri: list_iri,
        name: "List".to_string(),
        params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: vec![
            // nil : Π A:Set. List A
            InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
                    Box::new(list_a_typ.clone()),
                ),
            },
            // cons : Π A:Set. A → List A → List A
            InductiveCtorDecl {
                name: "cons".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
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
/// optional ancestor value. Self-references are `Exp::Const`, as in
/// [`list_decl`].
pub fn option_decl() -> Arc<InductiveDecl> {
    static OPTION_DECL: OnceLock<Arc<InductiveDecl>> = OnceLock::new();
    OPTION_DECL.get_or_init(build_option_decl).clone()
}

fn build_option_decl() -> Arc<InductiveDecl> {
    let option_iri = Iri::parse(crate::ontology::well_known::OPTION).expect("static Option IRI");
    let option_a_typ = Exp::const_applied(
        option_iri.clone(),
        Vec::new(),
        vec![Exp::Var("A".to_string())],
    );
    Arc::new(InductiveDecl {
        uparams: Vec::new(),
        iri: option_iri,
        name: "Option".to_string(),
        params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: vec![
            // none : Π A:Set. Option A
            InductiveCtorDecl {
                name: "none".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
                    Box::new(option_a_typ.clone()),
                ),
            },
            // some : Π A:Set. A → Option A
            InductiveCtorDecl {
                name: "some".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
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
        let t = Exp::arrow(Exp::One, Exp::sort(1));
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
        let t = Exp::list(Exp::sort(1));
        // D76 Phase B: the term NAMES the declaration, so this asserts the
        // reference's shape; the declaration itself is `list_decl`, checked below.
        let (iri, levels, params) = t.as_const_spine().expect("a const spine");
        assert_eq!(iri, &list_decl().iri);
        assert!(levels.is_empty());
        assert_eq!(params.len(), 1);
        assert!(matches!(params[0], Exp::Sort(l) if l.is_nat(1)));
        let decl = list_decl();
        assert_eq!(decl.name, "List");
        assert_eq!(decl.ctors.len(), 2);
        assert_eq!(decl.ctors[0].name, "nil");
        assert_eq!(decl.ctors[1].name, "cons");
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

#[cfg(test)]
mod record_canonical_order {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }
    fn v(n: &str) -> Patt {
        Patt::Var(n.to_string())
    }
    /// A field whose type mentions `dep`, if given — the shape that induces a
    /// dependency edge.
    fn field(name: &str, binder: &str, dep: Option<&str>) -> (Iri, Patt, Exp) {
        let ty = match dep {
            Some(d) => Exp::App(
                Box::new(Exp::Var("F".into())),
                Box::new(Exp::Var(d.to_string())),
            ),
            None => Exp::sort(1),
        };
        (iri(name), v(binder), ty)
    }
    fn iris_of(e: &Exp) -> Vec<String> {
        match e {
            Exp::Record(fs) => fs.iter().map(|(i, _, _)| i.as_str().to_string()).collect(),
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn independent_fields_sort_by_iri() {
        let r = Exp::record(vec![
            field("urn:t:c", "c", None),
            field("urn:t:a", "a", None),
            field("urn:t:b", "b", None),
        ])
        .unwrap();
        assert_eq!(iris_of(&r), ["urn:t:a", "urn:t:b", "urn:t:c"]);
    }

    #[test]
    fn field_order_is_independent_of_input_order() {
        // The property that makes `eq_nf`'s readback-and-compare decide record
        // equality: the same field set always yields the same telescope.
        let a = Exp::record(vec![
            field("urn:t:a", "a", None),
            field("urn:t:b", "b", Some("a")),
            field("urn:t:c", "c", None),
        ])
        .unwrap();
        let b = Exp::record(vec![
            field("urn:t:c", "c", None),
            field("urn:t:b", "b", Some("a")),
            field("urn:t:a", "a", None),
        ])
        .unwrap();
        assert_eq!(a, b, "the same field set must produce the same record");
    }

    #[test]
    fn a_dependency_outranks_iri_order() {
        // `urn:t:a` depends on `urn:t:z`, so IRI order alone would put it first
        // and produce an ill-formed telescope. Topological order wins.
        let r = Exp::record(vec![
            field("urn:t:a", "a", Some("z")),
            field("urn:t:z", "z", None),
        ])
        .unwrap();
        assert_eq!(
            iris_of(&r),
            ["urn:t:z", "urn:t:a"],
            "a field must never precede one its type mentions"
        );
    }

    #[test]
    fn ties_among_ready_fields_break_by_iri() {
        // Both `b` and `c` become ready once `a` is placed; the IRI-least goes
        // first, so the order is total rather than merely valid.
        let r = Exp::record(vec![
            field("urn:t:c", "c", Some("a")),
            field("urn:t:b", "b", Some("a")),
            field("urn:t:a", "a", None),
        ])
        .unwrap();
        assert_eq!(iris_of(&r), ["urn:t:a", "urn:t:b", "urn:t:c"]);
    }

    #[test]
    fn a_dependency_cycle_is_rejected() {
        let e = Exp::record(vec![
            field("urn:t:a", "a", Some("b")),
            field("urn:t:b", "b", Some("a")),
        ])
        .unwrap_err();
        match e {
            RecordError::DependencyCycle(iris) => {
                assert_eq!(iris.len(), 2, "both fields are stuck: {iris:?}");
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    #[test]
    fn a_self_dependency_is_a_cycle() {
        let e = Exp::record(vec![field("urn:t:a", "a", Some("a"))]).unwrap_err();
        assert!(matches!(e, RecordError::DependencyCycle(_)), "got {e:?}");
    }

    #[test]
    fn duplicate_fields_and_binders_are_rejected() {
        // Union semantics has no reading for a repeated field.
        let dup_field = Exp::record(vec![
            field("urn:t:a", "a", None),
            field("urn:t:a", "b", None),
        ])
        .unwrap_err();
        assert!(
            matches!(dup_field, RecordError::DuplicateField(_)),
            "got {dup_field:?}"
        );

        // Two fields binding one name makes a later mention ambiguous.
        let dup_binder = Exp::record(vec![
            field("urn:t:a", "x", None),
            field("urn:t:b", "x", None),
        ])
        .unwrap_err();
        assert!(
            matches!(dup_binder, RecordError::DuplicateBinder { .. }),
            "got {dup_binder:?}"
        );
    }

    #[test]
    fn the_empty_record_is_well_formed() {
        // 749 of 894 shipped classes have no `requires` (D78 §1.2), so this is
        // the common case, not an edge case.
        assert_eq!(Exp::record(vec![]).unwrap(), Exp::Record(vec![]));
    }
}
