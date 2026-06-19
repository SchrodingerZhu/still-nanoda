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
}

/// Build a Lean `String` object from a Rust `&str` (owned by the caller).
///
/// # Safety
/// Calls into the Lean runtime; only valid once the host is loaded.
pub unsafe fn mk_string(s: &str) -> *mut LeanObj {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    lean_mk_string(c.as_ptr())
}

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
/// Returns the small value when it fits, else `None` (bignum — TODO: decode
/// mpz; bvar/proj indices are always small in practice).
#[inline]
pub fn nat_as_usize(o: *const LeanObj) -> Option<usize> {
    if is_scalar(o) {
        Some(unbox(o))
    } else {
        None
    }
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
