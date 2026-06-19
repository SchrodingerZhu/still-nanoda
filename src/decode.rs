//! Decode Lean kernel `Declaration` and `ConstantInfo` `lean_object`s into
//! sokonanoda `Declar`s, importing their embedded expressions/levels/names into
//! the persistent arena via [`Importer`].
//!
//! Slot layout is documented in `reports/01-layouts.md` (from
//! `lean4/src/kernel/declaration.h`). All `*Val` structs begin with a nested
//! `ConstantVal` at slot 0; `Nat`-typed fields are object slots holding a
//! (usually small) `Nat`; trailing `Bool`/`UInt32` fields are scalars after the
//! object slots.

use crate::env::{ConstructorData, Declar, DeclarInfo, InductiveData, RecRule, RecursorData, ReducibilityHint};
use crate::importer::{ImportError, Importer};
use crate::lean_sys::{self as ls, LeanObj};
use crate::term::{Name, Term};
use crate::util::NamePtr;
use std::sync::Arc;

type Result<T> = std::result::Result<T, ImportError>;

const PTR: usize = 8;

/// The outcome of decoding a `Declaration` handed to `add_decl`: either a
/// declaration sokonanoda checks itself, or one to delegate to the builtin
/// kernel (quotient/mutual/inductive — the builtin generates their recursors).
pub enum Decoded<'a> {
    Check(Declar<'a>),
    Delegate,
}

unsafe fn list_objs(mut o: *const LeanObj) -> Vec<*const LeanObj> {
    let mut v = Vec::new();
    while !ls::is_scalar(o) {
        v.push(ls::ctor_get(o, 0));
        o = ls::ctor_get(o, 1);
    }
    v
}

unsafe fn nat_u16(o: *const LeanObj, slot: usize) -> Result<u16> {
    let n = ls::nat_as_usize(ls::ctor_get(o, slot))
        .ok_or_else(|| ImportError("bignum where small Nat field expected".into()))?;
    u16::try_from(n).map_err(|_| ImportError("Nat field exceeds u16".into()))
}

unsafe fn import_names<'a>(imp: &mut Importer<'a, '_>, list: *const LeanObj) -> Result<Arc<[NamePtr<'a>]>> {
    let objs = list_objs(list);
    let mut v = Vec::with_capacity(objs.len());
    for o in objs {
        v.push(imp.import_name(Name(o))?);
    }
    Ok(Arc::from(v))
}

/// Decode a `ConstantVal` (slot0=name, slot1=levelParams, slot2=type).
unsafe fn decode_cv<'a>(imp: &mut Importer<'a, '_>, cv: *const LeanObj) -> Result<DeclarInfo<'a>> {
    let name = imp.import_name(Name(ls::ctor_get(cv, 0)))?;
    let uparams = imp.import_uparams(ls::ctor_get(cv, 1))?;
    let ty = imp.import_expr(Term(ls::ctor_get(cv, 2)))?;
    Ok(DeclarInfo { name, uparams, ty })
}

/// Decode `ReducibilityHints`: `Opaque`/`Abbreviation` are nullary scalars
/// (tags 0/1); `Regular height` is a heap ctor (tag 2) with a `UInt32` scalar.
unsafe fn decode_hint(o: *const LeanObj) -> ReducibilityHint {
    if ls::is_scalar(o) {
        match ls::unbox(o) {
            0 => ReducibilityHint::Opaque,
            _ => ReducibilityHint::Abbrev,
        }
    } else {
        let h = ls::ctor_get_uint32(o, 0);
        ReducibilityHint::Regular(h.min(u16::MAX as u32) as u16)
    }
}

unsafe fn decode_axiom_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    Ok(Declar::Axiom { info: decode_cv(imp, ls::ctor_get(v, 0))? })
}

unsafe fn decode_def_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    let info = decode_cv(imp, ls::ctor_get(v, 0))?;
    let val = imp.import_expr(Term(ls::ctor_get(v, 1)))?;
    let hint = decode_hint(ls::ctor_get(v, 2));
    Ok(Declar::Definition { info, val, hint })
}

unsafe fn decode_thm_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    let info = decode_cv(imp, ls::ctor_get(v, 0))?;
    let val = imp.import_expr(Term(ls::ctor_get(v, 1)))?;
    Ok(Declar::Theorem { info, val })
}

unsafe fn decode_opaque_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    let info = decode_cv(imp, ls::ctor_get(v, 0))?;
    let val = imp.import_expr(Term(ls::ctor_get(v, 1)))?;
    Ok(Declar::Opaque { info, val })
}

unsafe fn decode_quot_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    Ok(Declar::Quot { info: decode_cv(imp, ls::ctor_get(v, 0))? })
}

unsafe fn decode_inductive_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    let info = decode_cv(imp, ls::ctor_get(v, 0))?;
    let num_params = nat_u16(v, 1)?;
    let num_indices = nat_u16(v, 2)?;
    let all_ind_names = import_names(imp, ls::ctor_get(v, 3))?;
    let all_ctor_names = import_names(imp, ls::ctor_get(v, 4))?;
    let num_nested = ls::nat_as_usize(ls::ctor_get(v, 5)).unwrap_or(1);
    // 6 object slots, then isRec/isUnsafe/isReflexive bool scalars.
    let is_recursive = ls::ctor_get_uint8(v, 6 * PTR) != 0;
    Ok(Declar::Inductive(InductiveData {
        info,
        is_recursive,
        is_nested: num_nested != 0,
        num_params,
        num_indices,
        all_ind_names,
        all_ctor_names,
    }))
}

unsafe fn decode_ctor_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    let info = decode_cv(imp, ls::ctor_get(v, 0))?;
    let inductive_name = imp.import_name(Name(ls::ctor_get(v, 1)))?;
    let ctor_idx = nat_u16(v, 2)?;
    let num_params = nat_u16(v, 3)?;
    let num_fields = nat_u16(v, 4)?;
    Ok(Declar::Constructor(ConstructorData { info, inductive_name, ctor_idx, num_params, num_fields }))
}

unsafe fn decode_rec_val<'a>(imp: &mut Importer<'a, '_>, v: *const LeanObj) -> Result<Declar<'a>> {
    let info = decode_cv(imp, ls::ctor_get(v, 0))?;
    let all_inductives = import_names(imp, ls::ctor_get(v, 1))?;
    let num_params = nat_u16(v, 2)?;
    let num_indices = nat_u16(v, 3)?;
    let num_motives = nat_u16(v, 4)?;
    let num_minors = nat_u16(v, 5)?;
    let mut rec_rules = Vec::new();
    for r in list_objs(ls::ctor_get(v, 6)) {
        let ctor_name = imp.import_name(Name(ls::ctor_get(r, 0)))?;
        let nfields = nat_u16(r, 1)?;
        let rhs = imp.import_expr(Term(ls::ctor_get(r, 2)))?;
        rec_rules.push(RecRule { ctor_name, ctor_telescope_size_wo_params: nfields, val: rhs });
    }
    // 7 object slots, then k/isUnsafe bool scalars.
    let is_k = ls::ctor_get_uint8(v, 7 * PTR) != 0;
    Ok(Declar::Recursor(RecursorData {
        info,
        all_inductives,
        num_params,
        num_indices,
        num_motives,
        num_minors,
        rec_rules: Arc::from(rec_rules),
        is_k,
    }))
}

/// Decode a `ConstantInfo` (for lazy import). slot0 = the `*Val`.
///
/// # Safety
/// `ci` must be a live `ConstantInfo` object.
pub unsafe fn decode_constant_info<'a>(imp: &mut Importer<'a, '_>, ci: *const LeanObj) -> Result<Declar<'a>> {
    let v = ls::ctor_get(ci, 0);
    match ls::ptr_tag(ci) {
        0 => decode_axiom_val(imp, v),
        1 => decode_def_val(imp, v),
        2 => decode_thm_val(imp, v),
        3 => decode_opaque_val(imp, v),
        4 => decode_quot_val(imp, v),
        5 => decode_inductive_val(imp, v),
        6 => decode_ctor_val(imp, v),
        7 => decode_rec_val(imp, v),
        t => Err(ImportError(format!("unknown ConstantInfo tag {t}"))),
    }
}

/// Collect the `Const` names referenced anywhere in the given expression roots
/// (used to lazily import a declaration's transitive constant dependencies).
///
/// # Safety
/// Each root must be a live kernel `Expr`.
pub unsafe fn collect_consts(roots: &[*const LeanObj], out: &mut Vec<*const LeanObj>) {
    use crate::term::TermKind::*;
    let mut visited = crate::util::new_fx_hash_set();
    let mut stack: Vec<*const LeanObj> = roots.to_vec();
    while let Some(e) = stack.pop() {
        if !visited.insert(e) {
            continue;
        }
        match Term(e).kind() {
            Const(name, _) => out.push(name.0),
            App(f, a) => {
                stack.push(f.0);
                stack.push(a.0);
            }
            Lambda(_, _, ty, b) | Pi(_, _, ty, b) => {
                stack.push(ty.0);
                stack.push(b.0);
            }
            Let(_, ty, v, b, _) => {
                stack.push(ty.0);
                stack.push(v.0);
                stack.push(b.0);
            }
            Proj(_, _, s) => stack.push(s.0),
            MData(inner) => stack.push(inner.0),
            Sort(_) | BVar(_) | Lit(_) | FVar(_) | MVar(_) => {}
        }
    }
}

/// The `Expr` roots of a `Declaration` (Check case) whose constants must be
/// imported before checking: its type, and value if it has one.
///
/// # Safety
/// `decl` must be a live `Declaration` (tag 0..=3).
pub unsafe fn decl_expr_roots(decl: *const LeanObj) -> Vec<*const LeanObj> {
    let v = ls::ctor_get(decl, 0);
    let cv = ls::ctor_get(v, 0);
    let mut roots = vec![ls::ctor_get(cv, 2)];
    if matches!(ls::ptr_tag(decl), 1 | 2 | 3) {
        roots.push(ls::ctor_get(v, 1));
    }
    roots
}

/// The `Expr` roots of a `ConstantInfo` (type, value, recursor rule rhss).
///
/// # Safety
/// `ci` must be a live `ConstantInfo`.
pub unsafe fn ci_expr_roots(ci: *const LeanObj) -> Vec<*const LeanObj> {
    let v = ls::ctor_get(ci, 0);
    let cv = ls::ctor_get(v, 0);
    let mut roots = vec![ls::ctor_get(cv, 2)];
    match ls::ptr_tag(ci) {
        1 | 2 | 3 => roots.push(ls::ctor_get(v, 1)),
        7 => {
            for r in list_objs(ls::ctor_get(v, 6)) {
                roots.push(ls::ctor_get(r, 2));
            }
        }
        _ => {}
    }
    roots
}

/// Decode a `Declaration` (for `add_decl`). slot0 = the `*Val` for the
/// non-mutual/non-inductive kinds. Quot/Mutual/Inductive are delegated.
///
/// # Safety
/// `decl` must be a live `Declaration` object.
pub unsafe fn decode_declaration<'a>(imp: &mut Importer<'a, '_>, decl: *const LeanObj) -> Result<Decoded<'a>> {
    match ls::ptr_tag(decl) {
        0 => Ok(Decoded::Check(decode_axiom_val(imp, ls::ctor_get(decl, 0))?)),
        1 => Ok(Decoded::Check(decode_def_val(imp, ls::ctor_get(decl, 0))?)),
        2 => Ok(Decoded::Check(decode_thm_val(imp, ls::ctor_get(decl, 0))?)),
        3 => Ok(Decoded::Check(decode_opaque_val(imp, ls::ctor_get(decl, 0))?)),
        // 4=Quot, 5=MutualDefinition, 6=Inductive -> builtin kernel.
        _ => Ok(Decoded::Delegate),
    }
}
