//! Minimal hand-written bindings to the Lean runtime object ABI (`lean_object`).
//!
//! These mirror the `static inline` accessors in `lean/lean.h` and
//! `src/kernel/expr.h` from the Lean source tree. Those accessors are not
//! exported symbols (they are `static inline`), so we cannot link to them; we
//! reimplement the layout here. Everything is read-only: the external checker
//! borrows the host's live DAG and never mutates or reference-counts it.
//!
//! Object header (`lean_object`, little-endian):
//! ```text
//!   offset 0: i32 m_rc
//!   offset 4: u16 m_cs_sz
//!   offset 6: u8  m_other   (number of object fields for constructors)
//!   offset 7: u8  m_tag     (constructor tag)
//!   offset 8: lean_object* m_objs[]   (constructor fields)
//! ```
//! Constructor scalar fields (uint8/uint64/usize) live after the object
//! pointers; offsets passed to the `ctor_get_*` helpers are relative to the
//! start of `m_objs` (i.e. object base + 8), matching `lean.h`.

use std::os::raw::{c_char, c_void};

/// An opaque Lean heap object. Pointers may also be *tagged scalars* (see
/// [`is_scalar`]); always check before dereferencing.
pub type LeanObj = c_void;

extern "C" {
    /// Lean runtime: allocate a `String` from a NUL-terminated UTF-8 buffer.
    /// Resolved against the host (liblean) via `RTLD_GLOBAL`.
    fn lean_mk_string(s: *const c_char) -> *mut LeanObj;
    /// Lean runtime: `Name.str p s` (consumes both owned arguments).
    fn lean_name_mk_string(p: *mut LeanObj, s: *mut LeanObj) -> *mut LeanObj;
}

/// Build a Lean `String` object from a Rust `&str` (owned by the caller).
///
/// # Safety
/// Calls into the Lean runtime; only valid once the host is loaded.
pub unsafe fn mk_string(s: &str) -> *mut LeanObj {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    lean_mk_string(c.as_ptr())
}

/// Build a dotted Lean `Name` (e.g. `["String","ofList"]` -> `String.ofList`),
/// owned by the caller. `Name.anonymous` is the nullary ctor, boxed as scalar 0.
///
/// # Safety
/// Calls into the Lean runtime; only valid once the host is loaded.
pub unsafe fn mk_name(parts: &[&str]) -> *mut LeanObj {
    let mut n = 1usize as *mut LeanObj; // lean_box(0) = Name.anonymous
    for p in parts {
        n = lean_name_mk_string(n, mk_string(p));
    }
    n
}

/// `Name.anonymous` (the nullary constructor, a boxed scalar).
#[inline]
pub fn name_anon() -> *mut LeanObj {
    1usize as *mut LeanObj
}

/// `Name.str prefix str` (consumes `pfx`; `s` is a Rust string interned fresh).
///
/// # Safety
/// Calls into the Lean runtime.
pub unsafe fn name_mk_str(pfx: *mut LeanObj, s: &str) -> *mut LeanObj {
    lean_name_mk_string(pfx, mk_string(s))
}

/// `Name.num prefix n` (consumes `pfx`).
///
/// # Safety
/// Calls into the Lean runtime.
pub unsafe fn name_mk_num(pfx: *mut LeanObj, n: u64) -> *mut LeanObj {
    lean_name_mk_numeral(pfx, mk_nat(n))
}

extern "C" {
    fn lean_name_mk_numeral(p: *mut LeanObj, n: *mut LeanObj) -> *mut LeanObj;
    fn lean_cstr_to_nat(s: *const c_char) -> *mut LeanObj;
    // Host-exported wrappers over the inline `lean_alloc_ctor`/`lean_ctor_set`.
    fn lean_extern_alloc_ctor(tag: u32, num_objs: u32) -> *mut LeanObj;
    fn lean_extern_ctor_set(o: *mut LeanObj, i: u32, v: *mut LeanObj);
    fn lean_expr_mk_bvar(idx: *mut LeanObj) -> *mut LeanObj;
    fn lean_expr_mk_fvar(n: *mut LeanObj) -> *mut LeanObj;
    fn lean_expr_mk_sort(l: *mut LeanObj) -> *mut LeanObj;
    fn lean_expr_mk_const(n: *mut LeanObj, ls: *mut LeanObj) -> *mut LeanObj;
    fn lean_expr_mk_app(f: *mut LeanObj, a: *mut LeanObj) -> *mut LeanObj;
    fn lean_expr_mk_lambda(n: *mut LeanObj, t: *mut LeanObj, e: *mut LeanObj, bi: u8) -> *mut LeanObj;
    fn lean_expr_mk_forall(n: *mut LeanObj, t: *mut LeanObj, e: *mut LeanObj, bi: u8) -> *mut LeanObj;
    fn lean_expr_mk_let(n: *mut LeanObj, t: *mut LeanObj, v: *mut LeanObj, b: *mut LeanObj, nondep: u8) -> *mut LeanObj;
    fn lean_expr_mk_lit(l: *mut LeanObj) -> *mut LeanObj;
    fn lean_expr_mk_proj(s: *mut LeanObj, idx: *mut LeanObj, e: *mut LeanObj) -> *mut LeanObj;
    fn lean_level_mk_zero(dummy: *mut LeanObj) -> *mut LeanObj;
    fn lean_level_mk_succ(l: *mut LeanObj) -> *mut LeanObj;
    fn lean_level_mk_max(a: *mut LeanObj, b: *mut LeanObj) -> *mut LeanObj;
    fn lean_level_mk_imax(a: *mut LeanObj, b: *mut LeanObj) -> *mut LeanObj;
    fn lean_level_mk_param(n: *mut LeanObj) -> *mut LeanObj;
}

/// Build a Lean `Nat` from a `u64` (boxed scalar when small enough).
#[inline]
pub fn mk_nat(n: u64) -> *mut LeanObj {
    // Lean represents a `Nat` that fits in `isize` as the boxed scalar `(n<<1)|1`.
    if n <= (usize::MAX >> 1) as u64 {
        (((n as usize) << 1) | 1) as *mut LeanObj
    } else {
        unsafe { mk_nat_str(&n.to_string()) }
    }
}

/// Build a Lean `Nat` from its decimal string (handles arbitrary precision).
///
/// # Safety
/// Calls into the Lean runtime.
pub unsafe fn mk_nat_str(decimal: &str) -> *mut LeanObj {
    let c = std::ffi::CString::new(decimal).unwrap_or_default();
    lean_cstr_to_nat(c.as_ptr())
}

// Thin owned-builder wrappers (each consumes its object arguments, returns owned).
#[inline] pub unsafe fn expr_bvar(idx: usize) -> *mut LeanObj { lean_expr_mk_bvar(mk_nat(idx as u64)) }
#[inline] pub unsafe fn expr_fvar(name: *mut LeanObj) -> *mut LeanObj {
    // `FVarId` is a one-field structure wrapping a `Name`.
    let id = lean_extern_alloc_ctor(0, 1);
    lean_extern_ctor_set(id, 0, name);
    lean_expr_mk_fvar(id)
}
#[inline] pub unsafe fn expr_sort(l: *mut LeanObj) -> *mut LeanObj { lean_expr_mk_sort(l) }
#[inline] pub unsafe fn expr_const(n: *mut LeanObj, ls: *mut LeanObj) -> *mut LeanObj { lean_expr_mk_const(n, ls) }
#[inline] pub unsafe fn expr_app(f: *mut LeanObj, a: *mut LeanObj) -> *mut LeanObj { lean_expr_mk_app(f, a) }
#[inline] pub unsafe fn expr_lam(n: *mut LeanObj, t: *mut LeanObj, b: *mut LeanObj, bi: u8) -> *mut LeanObj { lean_expr_mk_lambda(n, t, b, bi) }
#[inline] pub unsafe fn expr_forall(n: *mut LeanObj, t: *mut LeanObj, b: *mut LeanObj, bi: u8) -> *mut LeanObj { lean_expr_mk_forall(n, t, b, bi) }
#[inline] pub unsafe fn expr_let(n: *mut LeanObj, t: *mut LeanObj, v: *mut LeanObj, b: *mut LeanObj, nondep: bool) -> *mut LeanObj { lean_expr_mk_let(n, t, v, b, nondep as u8) }
#[inline] pub unsafe fn expr_proj(ty: *mut LeanObj, idx: usize, s: *mut LeanObj) -> *mut LeanObj { lean_expr_mk_proj(ty, mk_nat(idx as u64), s) }
#[inline] pub unsafe fn expr_lit_nat(decimal: &str) -> *mut LeanObj {
    let lit = lean_extern_alloc_ctor(0, 1); // Literal.natVal
    lean_extern_ctor_set(lit, 0, mk_nat_str(decimal));
    lean_expr_mk_lit(lit)
}
#[inline] pub unsafe fn expr_lit_str(s: &str) -> *mut LeanObj {
    let lit = lean_extern_alloc_ctor(1, 1); // Literal.strVal
    lean_extern_ctor_set(lit, 0, mk_string(s));
    lean_expr_mk_lit(lit)
}
#[inline] pub unsafe fn level_zero() -> *mut LeanObj { lean_level_mk_zero(1usize as *mut LeanObj) }
#[inline] pub unsafe fn level_succ(l: *mut LeanObj) -> *mut LeanObj { lean_level_mk_succ(l) }
#[inline] pub unsafe fn level_max(a: *mut LeanObj, b: *mut LeanObj) -> *mut LeanObj { lean_level_mk_max(a, b) }
#[inline] pub unsafe fn level_imax(a: *mut LeanObj, b: *mut LeanObj) -> *mut LeanObj { lean_level_mk_imax(a, b) }
#[inline] pub unsafe fn level_param(n: *mut LeanObj) -> *mut LeanObj { lean_level_mk_param(n) }
/// `List.cons head tail` for a `List Level` (`List.nil` is `name_anon`-style scalar 0).
#[inline] pub unsafe fn list_cons(head: *mut LeanObj, tail: *mut LeanObj) -> *mut LeanObj {
    let c = lean_extern_alloc_ctor(1, 2);
    lean_extern_ctor_set(c, 0, head);
    lean_extern_ctor_set(c, 1, tail);
    c
}
#[inline] pub fn list_nil() -> *mut LeanObj { 1usize as *mut LeanObj }

const HEADER_SIZE: usize = 8;
const PTR_SIZE: usize = std::mem::size_of::<usize>();

/// Tagged scalars have their low bit set (`lean_is_scalar`).
#[inline]
pub fn is_scalar(o: *const LeanObj) -> bool {
    (o as usize) & 1 == 1
}

/// Decode a tagged scalar (`lean_unbox`).
#[inline]
pub fn unbox(o: *const LeanObj) -> usize {
    (o as usize) >> 1
}

/// The constructor tag (`lean_ptr_tag` / `cnstr_tag`). Only valid for non-scalar
/// objects.
///
/// # Safety
/// `o` must be a non-scalar pointer to a live `lean_object`.
#[inline]
pub unsafe fn ptr_tag(o: *const LeanObj) -> u8 {
    *(o as *const u8).add(7)
}

/// The number of object fields of a constructor (`lean_ctor_num_objs`,
/// stored in `m_other`).
///
/// # Safety
/// `o` must be a non-scalar constructor object.
#[inline]
pub unsafe fn ctor_num_objs(o: *const LeanObj) -> usize {
    *(o as *const u8).add(6) as usize
}

/// Object field `i` (`lean_ctor_get` / `cnstr_get_ref`). The result is borrowed:
/// it shares the lifetime of `o` and must not be reference-counted.
///
/// # Safety
/// `o` must be a constructor with at least `i + 1` object fields.
#[inline]
pub unsafe fn ctor_get(o: *const LeanObj, i: usize) -> *const LeanObj {
    let objs = (o as *const u8).add(HEADER_SIZE) as *const *const LeanObj;
    *objs.add(i)
}

/// Read a `u8` scalar field at byte `offset` past the object array
/// (`lean_ctor_get_uint8`).
///
/// # Safety
/// `o` must be a constructor whose scalar area contains a `u8` at `offset`.
#[inline]
pub unsafe fn ctor_get_uint8(o: *const LeanObj, offset: usize) -> u8 {
    *(o as *const u8).add(HEADER_SIZE + offset)
}

/// Read a `u32` scalar field at byte `offset` past the object array
/// (`lean_ctor_get_uint32`).
///
/// # Safety
/// As [`ctor_get_uint8`], for a `u32`.
#[inline]
pub unsafe fn ctor_get_uint32(o: *const LeanObj, offset: usize) -> u32 {
    let p = (o as *const u8).add(HEADER_SIZE + offset);
    (p as *const u32).read_unaligned()
}

/// Read a `u64` scalar field at byte `offset` past the object array
/// (`lean_ctor_get_uint64`).
///
/// # Safety
/// As [`ctor_get_uint8`], for a `u64`.
#[inline]
pub unsafe fn ctor_get_uint64(o: *const LeanObj, offset: usize) -> u64 {
    let p = (o as *const u8).add(HEADER_SIZE + offset);
    (p as *const u64).read_unaligned()
}

// ---------------------------------------------------------------------------
// Nat
// ---------------------------------------------------------------------------

/// A Lean `Nat` is either a tagged scalar (small) or an mpz bignum object.
/// Returns the small value when it is a scalar, else `None` (bignum).
#[inline]
pub fn nat_as_usize(o: *const LeanObj) -> Option<usize> {
    if is_scalar(o) {
        Some(unbox(o))
    } else {
        None
    }
}

// Lean (built with GMP) stores a big `Nat` as an `mpz_object`:
//   offset 0:  lean_object m_header        (8 bytes)
//   offset 8:  __mpz_struct { i32 _mp_alloc; i32 _mp_size; mp_limb_t* _mp_d }
// so _mp_size is at +12, the limb pointer at +16. Limbs are `mp_limb_t` (u64 on
// LP64), little-endian limb order; `_mp_size` is the signed limb count (a `Nat`
// is non-negative, so it is >= 0).
const MPZ_SIZE_OFFSET: usize = 12;
const MPZ_LIMBS_OFFSET: usize = 16;

/// Decode a Lean `Nat` (scalar or mpz bignum) into a `BigUint`.
///
/// # Safety
/// `o` must be a live Lean `Nat` object.
pub unsafe fn nat_to_biguint(o: *const LeanObj) -> num_bigint::BigUint {
    use num_bigint::BigUint;
    if is_scalar(o) {
        return BigUint::from(unbox(o));
    }
    let base = o as *const u8;
    let size = (base.add(MPZ_SIZE_OFFSET) as *const i32).read_unaligned();
    if size == 0 {
        return BigUint::from(0u32);
    }
    let nlimbs = size.unsigned_abs() as usize;
    let limbs_ptr = (base.add(MPZ_LIMBS_OFFSET) as *const *const u64).read_unaligned();
    let limbs = std::slice::from_raw_parts(limbs_ptr, nlimbs);
    let mut bytes = Vec::with_capacity(nlimbs * 8);
    for &limb in limbs {
        bytes.extend_from_slice(&limb.to_le_bytes());
    }
    BigUint::from_bytes_le(&bytes)
}

// ---------------------------------------------------------------------------
// String  (lean_string_object: header, m_size, m_capacity, m_length, data[])
// ---------------------------------------------------------------------------

#[repr(C)]
struct LeanStringHeader {
    _rc: i32,
    _cs_sz: u16,
    _other: u8,
    _tag: u8,
    m_size: usize,     // byte length including the trailing NUL
    _capacity: usize,
    _length: usize,    // UTF-8 (codepoint) length
}

const STRING_DATA_OFFSET: usize = std::mem::size_of::<LeanStringHeader>();

/// Borrow the UTF-8 bytes of a Lean `String` object (excluding the NUL).
///
/// # Safety
/// `o` must point to a live Lean `String` object, valid for the returned slice's
/// use.
#[inline]
pub unsafe fn string_bytes<'a>(o: *const LeanObj) -> &'a [u8] {
    let hdr = &*(o as *const LeanStringHeader);
    let len = hdr.m_size.saturating_sub(1);
    let data = (o as *const u8).add(STRING_DATA_OFFSET);
    std::slice::from_raw_parts(data, len)
}

/// Borrow a Lean `String` as `&str` (Lean strings are valid UTF-8).
///
/// # Safety
/// As [`string_bytes`].
#[inline]
pub unsafe fn string_str<'a>(o: *const LeanObj) -> &'a str {
    std::str::from_utf8_unchecked(string_bytes(o))
}

// Keep PTR_SIZE referenced so the layout assumption (64-bit pointers) is
// documented and a 32-bit target fails loudly rather than silently.
const _: () = assert!(PTR_SIZE == 8, "lean_object bindings assume 64-bit pointers");
