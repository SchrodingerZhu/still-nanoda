//! Import Lean's in-memory `lean_object` expression DAG directly into
//! sokonanoda's persistent arena, replacing the old JSON export parser.
//!
//! Performance model: every node is interned into the **persistent**
//! `ExportFile` dag ([`DagMarker::ExportFile`]), so once a constant's type/value
//! is imported (when its own declaration is added) it is never re-imported — a
//! later declaration that mentions it only builds a `Const` leaf, and the
//! checker resolves the body through the environment by name. Within a single
//! import, a pointer-keyed memo dedups shared subterms (Lean exprs are
//! hash-consed, so a shared child is the same `lean_object*`).
//!
//! These interners mirror the `mk_*` constructors on `TcCtx` (same `hash64!`
//! formulas, same `num_loose_bvars`/`has_fvars` accounting) but target a
//! `&mut LeanDag` directly, which `TcCtx` cannot do (it borrows the export file
//! immutably).

use crate::expr::{
    BinderStyle, Expr, APP_HASH, CONST_HASH, LAMBDA_HASH, LET_HASH, NAT_LIT_HASH, PI_HASH, PROJ_HASH, SORT_HASH,
    STRING_LIT_HASH, VAR_HASH,
};
use crate::hash64;
use crate::lean_sys::LeanObj;
use crate::level::{Level, IMAX_HASH, MAX_HASH, PARAM_HASH, SUCC_HASH};
use crate::name::{Name, NUM_HASH, STR_HASH};
use crate::term::{self, BinderInfo, Lit, NameKind, TermKind};
use crate::util::{
    new_fx_hash_map, CowStr, DagMarker, ExprPtr, FxHashMap, LeanDag, LevelPtr, LevelsPtr, NamePtr, Ptr, StringPtr,
};
use num_bigint::BigUint;
use std::sync::Arc;

/// An error encountered while importing a `lean_object` DAG (a malformed or
/// unsupported node). Surfaced as a kernel error rather than a panic.
#[derive(Debug, Clone)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "external checker import error: {}", self.0)
    }
}
impl std::error::Error for ImportError {}

type Result<T> = std::result::Result<T, ImportError>;

/// Walks `lean_object` views ([`term`]) into a persistent [`LeanDag`].
///
/// One importer is used per declaration: the memos dedup shared subterms within
/// that declaration's DAG. They are keyed on the source `lean_object*` address,
/// valid for the duration of the import call.
pub struct Importer<'a, 'd> {
    dag: &'d mut LeanDag<'a>,
    nat_extension: bool,
    string_extension: bool,
    expr_memo: FxHashMap<*const LeanObj, ExprPtr<'a>>,
    level_memo: FxHashMap<*const LeanObj, LevelPtr<'a>>,
    name_memo: FxHashMap<*const LeanObj, NamePtr<'a>>,
}

impl<'a, 'd> Importer<'a, 'd> {
    pub fn new(dag: &'d mut LeanDag<'a>, nat_extension: bool, string_extension: bool) -> Self {
        Self {
            dag,
            nat_extension,
            string_extension,
            expr_memo: new_fx_hash_map(),
            level_memo: new_fx_hash_map(),
            name_memo: new_fx_hash_map(),
        }
    }

    // ----- low-level persistent interners (always ExportFile-marked) -----

    fn intern_name(&mut self, n: Name<'a>) -> NamePtr<'a> {
        Ptr::from(DagMarker::ExportFile, self.dag.names.insert_full(n).0)
    }
    fn intern_level(&mut self, l: Level<'a>) -> LevelPtr<'a> {
        Ptr::from(DagMarker::ExportFile, self.dag.levels.insert_full(l).0)
    }
    fn intern_expr(&mut self, e: Expr<'a>) -> ExprPtr<'a> {
        Ptr::from(DagMarker::ExportFile, self.dag.exprs.insert_full(e).0)
    }
    fn intern_string(&mut self, s: &str) -> StringPtr<'a> {
        Ptr::from(DagMarker::ExportFile, self.dag.strings.insert_full(CowStr::Owned(s.to_string())).0)
    }
    fn intern_levels(&mut self, ls: Arc<[LevelPtr<'a>]>) -> LevelsPtr<'a> {
        Ptr::from(DagMarker::ExportFile, self.dag.uparams.insert_full(ls).0)
    }

    /// Read back an already-interned (persistent) expression, for bottom-up
    /// `num_loose_bvars`/`has_fvars` accounting.
    fn expr_at(&self, p: ExprPtr<'a>) -> Expr<'a> {
        *self.dag.exprs.get_index(p.idx()).unwrap()
    }

    // ----- node builders (mirror TcCtx::mk_*, ExportFile-marked) -----

    fn mk_app(&mut self, fun: ExprPtr<'a>, arg: ExprPtr<'a>) -> ExprPtr<'a> {
        let hash = hash64!(APP_HASH, fun, arg);
        let num_loose_bvars = self.expr_at(fun).num_loose_bvars().max(self.expr_at(arg).num_loose_bvars());
        let has_fvars = self.expr_at(fun).has_fvars() || self.expr_at(arg).has_fvars();
        self.intern_expr(Expr::App { fun, arg, num_loose_bvars, has_fvars, hash })
    }

    fn mk_binding(&mut self, is_pi: bool, name: NamePtr<'a>, style: BinderStyle, ty: ExprPtr<'a>, body: ExprPtr<'a>) -> ExprPtr<'a> {
        let num_loose_bvars = self.expr_at(ty).num_loose_bvars().max(self.expr_at(body).num_loose_bvars().saturating_sub(1));
        let has_fvars = self.expr_at(ty).has_fvars() || self.expr_at(body).has_fvars();
        let e = if is_pi {
            let hash = hash64!(PI_HASH, name, style, ty, body);
            Expr::Pi { binder_name: name, binder_style: style, binder_type: ty, body, num_loose_bvars, has_fvars, hash }
        } else {
            let hash = hash64!(LAMBDA_HASH, name, style, ty, body);
            Expr::Lambda { binder_name: name, binder_style: style, binder_type: ty, body, num_loose_bvars, has_fvars, hash }
        };
        self.intern_expr(e)
    }

    fn mk_let(&mut self, name: NamePtr<'a>, ty: ExprPtr<'a>, val: ExprPtr<'a>, body: ExprPtr<'a>, nondep: bool) -> ExprPtr<'a> {
        let hash = hash64!(LET_HASH, name, ty, val, body, nondep);
        let num_loose_bvars = self
            .expr_at(ty)
            .num_loose_bvars()
            .max(self.expr_at(val).num_loose_bvars().max(self.expr_at(body).num_loose_bvars().saturating_sub(1)));
        let has_fvars = self.expr_at(ty).has_fvars() || self.expr_at(val).has_fvars() || self.expr_at(body).has_fvars();
        self.intern_expr(Expr::Let { binder_name: name, binder_type: ty, val, body, num_loose_bvars, has_fvars, hash, nondep })
    }

    fn mk_proj(&mut self, ty_name: NamePtr<'a>, idx: usize, structure: ExprPtr<'a>) -> ExprPtr<'a> {
        let hash = hash64!(PROJ_HASH, ty_name, idx, structure);
        let num_loose_bvars = self.expr_at(structure).num_loose_bvars();
        let has_fvars = self.expr_at(structure).has_fvars();
        self.intern_expr(Expr::Proj { ty_name, idx, structure, num_loose_bvars, has_fvars, hash })
    }

    // ----- recursive walks with memoization -----

    /// Import a kernel `Name`.
    ///
    /// # Safety
    /// `n.0` must be a live kernel `Name` object.
    pub unsafe fn import_name(&mut self, n: term::Name) -> Result<NamePtr<'a>> {
        if let Some(p) = self.name_memo.get(&n.0) {
            return Ok(*p);
        }
        let out = match n.kind() {
            NameKind::Anon => self.dag.anonymous(),
            NameKind::Str(pre, s) => {
                let pre = self.import_name(pre)?;
                let sfx = self.intern_string(s);
                let hash = hash64!(STR_HASH, pre, sfx);
                self.intern_name(Name::Str(pre, sfx, hash))
            }
            NameKind::Num(pre, i) => {
                let pre = self.import_name(pre)?;
                let i = i.as_usize().ok_or_else(|| ImportError("bignum name component".into()))? as u64;
                let hash = hash64!(NUM_HASH, pre, i);
                self.intern_name(Name::Num(pre, i, hash))
            }
        };
        self.name_memo.insert(n.0, out);
        Ok(out)
    }

    /// Import a kernel `Level`.
    ///
    /// # Safety
    /// `l.0` must be a live kernel `Level` object.
    pub unsafe fn import_level(&mut self, l: term::Level) -> Result<LevelPtr<'a>> {
        if let Some(p) = self.level_memo.get(&l.0) {
            return Ok(*p);
        }
        use term::LevelKind::*;
        let out = match l.kind() {
            Zero => self.dag.zero(),
            Succ(x) => {
                let x = self.import_level(x)?;
                let hash = hash64!(SUCC_HASH, x);
                self.intern_level(Level::Succ(x, hash))
            }
            Max(x, y) => {
                let (x, y) = (self.import_level(x)?, self.import_level(y)?);
                let hash = hash64!(MAX_HASH, x, y);
                self.intern_level(Level::Max(x, y, hash))
            }
            IMax(x, y) => {
                let (x, y) = (self.import_level(x)?, self.import_level(y)?);
                let hash = hash64!(IMAX_HASH, x, y);
                self.intern_level(Level::IMax(x, y, hash))
            }
            Param(n) => {
                let n = self.import_name(n)?;
                let hash = hash64!(PARAM_HASH, n);
                self.intern_level(Level::Param(n, hash))
            }
            MVar(_) => return Err(ImportError("universe metavariable in declaration".into())),
        };
        self.level_memo.insert(l.0, out);
        Ok(out)
    }

    unsafe fn import_levels(&mut self, ls: term::Levels) -> Result<LevelsPtr<'a>> {
        let raw = ls.collect();
        let mut out = Vec::with_capacity(raw.len());
        for l in raw {
            out.push(self.import_level(l)?);
        }
        Ok(self.intern_levels(Arc::from(out)))
    }

    /// Import a kernel `Expr`.
    ///
    /// # Safety
    /// `t.0` must be a live kernel `Expr` object.
    pub unsafe fn import_expr(&mut self, t: term::Term) -> Result<ExprPtr<'a>> {
        if let Some(p) = self.expr_memo.get(&t.0) {
            return Ok(*p);
        }
        let out = match t.kind() {
            TermKind::BVar(i) => {
                let idx = i.as_usize().ok_or_else(|| ImportError("bignum bvar index".into()))?;
                let dbj_idx = u16::try_from(idx).map_err(|_| ImportError("bvar index exceeds u16".into()))?;
                let hash = hash64!(VAR_HASH, dbj_idx);
                self.intern_expr(Expr::Var { dbj_idx, hash })
            }
            TermKind::Sort(l) => {
                let level = self.import_level(l)?;
                let hash = hash64!(SORT_HASH, level);
                self.intern_expr(Expr::Sort { level, hash })
            }
            TermKind::Const(n, ls) => {
                let name = self.import_name(n)?;
                let levels = self.import_levels(ls)?;
                let hash = hash64!(CONST_HASH, name, levels);
                self.intern_expr(Expr::Const { name, levels, hash })
            }
            TermKind::App(f, a) => {
                let (f, a) = (self.import_expr(f)?, self.import_expr(a)?);
                self.mk_app(f, a)
            }
            TermKind::Lambda(n, bi, ty, body) => {
                let name = self.import_name(n)?;
                let ty = self.import_expr(ty)?;
                let body = self.import_expr(body)?;
                self.mk_binding(false, name, binder_style(bi), ty, body)
            }
            TermKind::Pi(n, bi, ty, body) => {
                let name = self.import_name(n)?;
                let ty = self.import_expr(ty)?;
                let body = self.import_expr(body)?;
                self.mk_binding(true, name, binder_style(bi), ty, body)
            }
            TermKind::Let(n, ty, val, body, nondep) => {
                let name = self.import_name(n)?;
                let ty = self.import_expr(ty)?;
                let val = self.import_expr(val)?;
                let body = self.import_expr(body)?;
                self.mk_let(name, ty, val, body, nondep)
            }
            TermKind::Lit(Lit::Nat(nat)) => {
                if !self.nat_extension {
                    return Err(ImportError("Nat literal but nat_extension disabled".into()));
                }
                let n = nat.as_usize().ok_or_else(|| ImportError("bignum Nat literal (mpz decode TODO)".into()))?;
                let num_ptr = self.intern_bignum(BigUint::from(n))?;
                let hash = hash64!(NAT_LIT_HASH, num_ptr);
                self.intern_expr(Expr::NatLit { ptr: num_ptr, hash })
            }
            TermKind::Lit(Lit::Str(s)) => {
                if !self.string_extension {
                    return Err(ImportError("String literal but string_extension disabled".into()));
                }
                let ptr = self.intern_string(s);
                let hash = hash64!(STRING_LIT_HASH, ptr);
                self.intern_expr(Expr::StringLit { ptr, hash })
            }
            // `kind()` already unwrapped MData to its inner expression.
            TermKind::MData(inner) => self.import_expr(inner)?,
            TermKind::Proj(n, i, structure) => {
                let ty_name = self.import_name(n)?;
                let idx = i.as_usize().ok_or_else(|| ImportError("bignum proj index".into()))?;
                let structure = self.import_expr(structure)?;
                self.mk_proj(ty_name, idx, structure)
            }
            TermKind::FVar(_) | TermKind::MVar(_) => {
                return Err(ImportError("free/meta variable in closed declaration".into()))
            }
        };
        self.expr_memo.insert(t.0, out);
        Ok(out)
    }

    fn intern_bignum(&mut self, n: BigUint) -> Result<crate::util::BigUintPtr<'a>> {
        let set = self.dag.bignums.as_mut().ok_or_else(|| ImportError("bignum store disabled".into()))?;
        Ok(Ptr::from(DagMarker::ExportFile, set.insert_full(n).0))
    }
}

fn binder_style(bi: BinderInfo) -> BinderStyle {
    match bi {
        BinderInfo::Default => BinderStyle::Default,
        BinderInfo::Implicit => BinderStyle::Implicit,
        BinderInfo::StrictImplicit => BinderStyle::StrictImplicit,
        BinderInfo::InstImplicit => BinderStyle::InstanceImplicit,
    }
}
