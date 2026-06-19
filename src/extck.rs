//! FFI entry point for using sokonanoda as a Lean `--external-checker-lib`.
//!
//! Mirrors the two versioned callback tables in the host's
//! `src/kernel/external_checker.h`: the host passes us a table of helpers
//! (`tick`, `Kernel.Exception` builders, `find_const`, `builtin_add_decl`) and
//! we fill a table of our own (`add_decl`, optional primitives).
//!
//! Current stage: `add_decl` delegates every declaration to the host's builtin
//! kernel via `builtin_add_decl`, which makes `lean --external-checker-lib` an
//! exact passthrough of the builtin kernel. Real checking is then moved into
//! sokonanoda one declaration kind at a time. All entry points are wrapped in
//! `catch_unwind` so a Rust panic becomes a `Kernel.Exception.other` rather than
//! unwinding across the FFI boundary.

use crate::lean_sys::LeanObj;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

/// Must match `LEAN_EXTERNAL_CHECKER_ABI_VERSION` in the host header.
const ABI_VERSION: u32 = 1;

type LeanFn1 = unsafe extern "C" fn(*mut LeanObj) -> *mut LeanObj;
type TickFn = unsafe extern "C" fn(u64, *mut i32) -> i32;
type MkExcFn = unsafe extern "C" fn(
    u32,
    *mut LeanObj,
    *mut LeanObj,
    *mut LeanObj,
    *mut LeanObj,
    *mut LeanObj,
    *mut LeanObj,
    *mut LeanObj,
    *mut LeanObj,
) -> *mut LeanObj;
type FindConstFn = unsafe extern "C" fn(*mut LeanObj, *mut LeanObj) -> *mut LeanObj;
type AddDeclFn = unsafe extern "C" fn(*mut LeanObj, usize, *mut LeanObj, *mut LeanObj) -> *mut LeanObj;

/// Host -> checker callback table (mirrors `lean_external_checker_host`).
#[repr(C)]
pub struct Host {
    pub abi_version: u32,
    pub tick: Option<TickFn>,
    pub mk_ok: Option<LeanFn1>,
    pub mk_error: Option<LeanFn1>,
    pub mk_kernel_exception: Option<MkExcFn>,
    pub find_const: Option<FindConstFn>,
    pub builtin_add_decl: Option<AddDeclFn>,
}

type CheckerAddDecl =
    unsafe extern "C" fn(*mut c_void, *mut LeanObj, usize, *mut LeanObj, *mut LeanObj) -> *mut LeanObj;
type CheckerPrim2 = unsafe extern "C" fn(*mut c_void, *mut LeanObj, *mut LeanObj, *mut LeanObj) -> *mut LeanObj;
type CheckerDefEq =
    unsafe extern "C" fn(*mut c_void, *mut LeanObj, *mut LeanObj, *mut LeanObj, *mut LeanObj) -> *mut LeanObj;

/// Checker -> host callback table (mirrors `lean_external_checker_callbacks`).
#[repr(C)]
pub struct Callbacks {
    pub abi_version: u32,
    pub self_: *mut c_void,
    pub add_decl: Option<CheckerAddDecl>,
    pub whnf: Option<CheckerPrim2>,
    pub check: Option<CheckerPrim2>,
    pub is_def_eq: Option<CheckerDefEq>,
    pub release: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// The host table, stored at registration and read (immutably) by callbacks.
/// Set once before any `add_decl`, so a plain atomic pointer suffices.
static HOST: AtomicPtr<Host> = AtomicPtr::new(ptr::null_mut());

#[inline]
fn host() -> &'static Host {
    // Safe: set once in `populate` before the host issues any callback.
    unsafe { &*HOST.load(Ordering::Acquire) }
}

/// Build `Except.error (Kernel.Exception.other msg)` for a panic/other failure.
unsafe fn mk_other_error(h: &Host, msg: &str) -> *mut LeanObj {
    let s = crate::lean_sys::mk_string(msg);
    let exc = (h.mk_kernel_exception.unwrap())(
        12,
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
        s,
    );
    (h.mk_error.unwrap())(exc)
}

/// `add_decl` callback. Owns `env`; borrows `decl`/`opt_cancel_tk`. Returns an
/// owned `Except Kernel.Exception Environment`.
unsafe extern "C" fn add_decl(
    _self: *mut c_void,
    env: *mut LeanObj,
    max_heartbeat: usize,
    decl: *mut LeanObj,
    opt_cancel_tk: *mut LeanObj,
) -> *mut LeanObj {
    let h = host();
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        // Delegate every declaration to the builtin kernel for now. This makes
        // `lean --external-checker-lib` an exact passthrough; checking moves into
        // sokonanoda per declaration kind in later stages.
        (h.builtin_add_decl.unwrap())(env, max_heartbeat, decl, opt_cancel_tk)
    }));
    match result {
        Ok(obj) => obj,
        Err(_) => mk_other_error(h, "sokonanoda external checker panicked"),
    }
}

/// The one symbol the host resolves via `dlsym`. Receives the host table and
/// fills `out`. Returns 0 to accept, nonzero to reject (e.g. version mismatch).
///
/// # Safety
/// `host`/`out` must point to valid `Host`/`Callbacks` from the host.
#[no_mangle]
pub unsafe extern "C" fn lean_external_check_populate_callbacks(host: *const Host, out: *mut Callbacks) -> u32 {
    if host.is_null() || (*host).abi_version != ABI_VERSION {
        return 1;
    }
    HOST.store(host as *mut Host, Ordering::Release);
    let out = &mut *out;
    out.abi_version = ABI_VERSION;
    out.self_ = ptr::null_mut();
    out.add_decl = Some(add_decl);
    out.whnf = None;
    out.check = None;
    out.is_def_eq = None;
    out.release = None;
    0
}
