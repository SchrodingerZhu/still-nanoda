//! Typed, read-only views over Lean's in-memory kernel objects (`lean_object`).
//!
//! A [`Term`] wraps a `lean_object*` that is a kernel `Expr`; [`Term::kind`]
//! decodes it into a [`TermKind`] carrying child views, mirroring the field
//! layout in `src/kernel/expr.h`. Companion wrappers cover the other kernel
//! object families reachable from an expression: [`Level`], [`Levels`] (a
//! `List Level`), [`Name`], and [`Nat`].
//!
//! These are the input side of importing a declaration: the importer walks a
//! `Term` and reconstructs the corresponding node in sokonanoda's arena. Nothing
//! here allocates or reference-counts; views borrow the host's live DAG.

use crate::lean_sys::{self, LeanObj};

// Object pointers are 8 bytes; the scalar data word follows the object fields,
// and trailing `u8` flags (binder info / let-nondep) follow that word.
const PTR: usize = 8;
const DATA_WORD: usize = 8;

/// Binder annotation (`BinderInfo` in Lean: the brackets used to write a binder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinderInfo {
    Default,
    Implicit,
    StrictImplicit,
    InstImplicit,
}

impl BinderInfo {
    fn from_u8(b: u8) -> Self {
        match b {
            0 => BinderInfo::Default,
            1 => BinderInfo::Implicit,
            2 => BinderInfo::StrictImplicit,
            _ => BinderInfo::InstImplicit,
        }
    }
}

// ---------------------------------------------------------------------------
// Name
// ---------------------------------------------------------------------------

/// A view over a kernel `Name` object.
#[derive(Debug, Clone, Copy)]
pub struct Name(pub *const LeanObj);

/// Decoded `Name` constructor (`Name.anonymous | .str pre s | .num pre i`).
#[derive(Debug, Clone, Copy)]
pub enum NameKind<'a> {
    Anon,
    Str(Name, &'a str),
    Num(Name, Nat),
}

impl Name {
    /// # Safety
    /// `self.0` must be a live kernel `Name` object (or the anonymous scalar).
    pub unsafe fn kind<'a>(self) -> NameKind<'a> {
        if lean_sys::is_scalar(self.0) {
            return NameKind::Anon;
        }
        match lean_sys::ptr_tag(self.0) {
            1 => NameKind::Str(Name(lean_sys::ctor_get(self.0, 0)), lean_sys::string_str(lean_sys::ctor_get(self.0, 1))),
            _ => NameKind::Num(Name(lean_sys::ctor_get(self.0, 0)), Nat(lean_sys::ctor_get(self.0, 1))),
        }
    }
}

// ---------------------------------------------------------------------------
// Nat
// ---------------------------------------------------------------------------

/// A view over a kernel `Nat` (a tagged scalar, or an mpz bignum object).
#[derive(Debug, Clone, Copy)]
pub struct Nat(pub *const LeanObj);

impl Nat {
    /// The value when it fits in a `usize` (always, for bvar/proj indices).
    /// `None` for an mpz bignum (decoding those is left to the importer).
    pub fn as_usize(self) -> Option<usize> {
        lean_sys::nat_as_usize(self.0)
    }
}

// ---------------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------------

/// A view over a kernel `Level` object.
#[derive(Debug, Clone, Copy)]
pub struct Level(pub *const LeanObj);

/// Decoded `Level` constructor.
#[derive(Debug, Clone, Copy)]
pub enum LevelKind {
    Zero,
    Succ(Level),
    Max(Level, Level),
    IMax(Level, Level),
    Param(Name),
    MVar(Name),
}

impl Level {
    /// # Safety
    /// `self.0` must be a live kernel `Level` object (or the zero scalar).
    pub unsafe fn kind(self) -> LevelKind {
        if lean_sys::is_scalar(self.0) {
            return LevelKind::Zero;
        }
        match lean_sys::ptr_tag(self.0) {
            1 => LevelKind::Succ(Level(lean_sys::ctor_get(self.0, 0))),
            2 => LevelKind::Max(Level(lean_sys::ctor_get(self.0, 0)), Level(lean_sys::ctor_get(self.0, 1))),
            3 => LevelKind::IMax(Level(lean_sys::ctor_get(self.0, 0)), Level(lean_sys::ctor_get(self.0, 1))),
            4 => LevelKind::Param(Name(lean_sys::ctor_get(self.0, 0))),
            _ => LevelKind::MVar(Name(lean_sys::ctor_get(self.0, 0))),
        }
    }
}

/// A view over a `List Level` (the universe arguments of a `Const`).
#[derive(Debug, Clone, Copy)]
pub struct Levels(pub *const LeanObj);

impl Levels {
    /// Iterate the levels head-to-tail by walking `List.cons` cells.
    ///
    /// # Safety
    /// `self.0` must be a live `List Level` object.
    pub unsafe fn collect(self) -> Vec<Level> {
        let mut out = Vec::new();
        let mut cur = self.0;
        // `List.nil` is the scalar; `List.cons head tail` is tag 1.
        while !lean_sys::is_scalar(cur) {
            out.push(Level(lean_sys::ctor_get(cur, 0)));
            cur = lean_sys::ctor_get(cur, 1);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Expr / Term
// ---------------------------------------------------------------------------

/// A literal payload (`Literal.natVal n | .strVal s`).
#[derive(Debug, Clone, Copy)]
pub enum Lit<'a> {
    Nat(Nat),
    Str(&'a str),
}

/// A view over a kernel `Expr` object.
#[derive(Debug, Clone, Copy)]
pub struct Term(pub *const LeanObj);

/// Decoded `Expr` constructor, carrying child views. Field order matches
/// `src/kernel/expr.h`. `FVar`/`MVar` cannot occur in a closed declaration but
/// are represented for completeness.
#[derive(Debug, Clone, Copy)]
pub enum TermKind<'a> {
    BVar(Nat),
    FVar(Name),
    MVar(Name),
    Sort(Level),
    Const(Name, Levels),
    App(Term, Term),
    Lambda(Name, BinderInfo, Term, Term),
    Pi(Name, BinderInfo, Term, Term),
    Let(Name, Term, Term, Term, bool),
    Lit(Lit<'a>),
    MData(Term),
    Proj(Name, Nat, Term),
}

impl Term {
    /// # Safety
    /// `self.0` must be a live kernel `Expr` object.
    pub unsafe fn kind<'a>(self) -> TermKind<'a> {
        let o = self.0;
        match lean_sys::ptr_tag(o) {
            0 => TermKind::BVar(Nat(lean_sys::ctor_get(o, 0))),
            1 => TermKind::FVar(Name(lean_sys::ctor_get(o, 0))),
            2 => TermKind::MVar(Name(lean_sys::ctor_get(o, 0))),
            3 => TermKind::Sort(Level(lean_sys::ctor_get(o, 0))),
            4 => TermKind::Const(Name(lean_sys::ctor_get(o, 0)), Levels(lean_sys::ctor_get(o, 1))),
            5 => TermKind::App(Term(lean_sys::ctor_get(o, 0)), Term(lean_sys::ctor_get(o, 1))),
            6 => TermKind::Lambda(
                Name(lean_sys::ctor_get(o, 0)),
                BinderInfo::from_u8(lean_sys::ctor_get_uint8(o, 3 * PTR + DATA_WORD)),
                Term(lean_sys::ctor_get(o, 1)),
                Term(lean_sys::ctor_get(o, 2)),
            ),
            7 => TermKind::Pi(
                Name(lean_sys::ctor_get(o, 0)),
                BinderInfo::from_u8(lean_sys::ctor_get_uint8(o, 3 * PTR + DATA_WORD)),
                Term(lean_sys::ctor_get(o, 1)),
                Term(lean_sys::ctor_get(o, 2)),
            ),
            8 => TermKind::Let(
                Name(lean_sys::ctor_get(o, 0)),
                Term(lean_sys::ctor_get(o, 1)),
                Term(lean_sys::ctor_get(o, 2)),
                Term(lean_sys::ctor_get(o, 3)),
                lean_sys::ctor_get_uint8(o, 4 * PTR + DATA_WORD) != 0,
            ),
            9 => {
                let lit = lean_sys::ctor_get(o, 0);
                // Literal.natVal (tag 0) | Literal.strVal (tag 1)
                if lean_sys::ptr_tag(lit) == 0 {
                    TermKind::Lit(Lit::Nat(Nat(lean_sys::ctor_get(lit, 0))))
                } else {
                    TermKind::Lit(Lit::Str(lean_sys::string_str(lean_sys::ctor_get(lit, 0))))
                }
            }
            10 => TermKind::MData(Term(lean_sys::ctor_get(o, 1))),
            _ => TermKind::Proj(
                Name(lean_sys::ctor_get(o, 0)),
                Nat(lean_sys::ctor_get(o, 1)),
                Term(lean_sys::ctor_get(o, 2)),
            ),
        }
    }
}
