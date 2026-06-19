//! FFI entry point for using sokonanoda as a Lean `--external-checker-lib`.
//!
//! Mirrors the two versioned callback tables in the host's
//! `src/kernel/external_checker.h`. The checker keeps a persistent
//! `ExportFile` (its own environment): each `add_decl` imports the new
//! declaration once into that arena, lazily imports any constants it references
//! from the real Lean environment (`find_const`), and runs sokonanoda's
//! `check_declar`. Inductive/quotient/mutual declarations are delegated to the
//! builtin kernel (which generates their recursors); on success of a checked
//! declaration the trusted environment update is done once via
//! `builtin_add_unchecked` (no double-check).
//!
//! All entry points are wrapped in `catch_unwind`: a sokonanoda rejection is a
//! Rust panic, which becomes `Except.error (Kernel.Exception.other ..)`.

use crate::decode::{self, Decoded};
use crate::importer::Importer;
use crate::lean_sys::{self, LeanObj};
use crate::pretty_printer::PpOptions;
use crate::term::Name;
use crate::util::{new_fx_hash_map, new_fx_hash_set, new_fx_index_map, Config, ExportFile, LeanDag};
use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

/// Must match `LEAN_EXTERNAL_CHECKER_ABI_VERSION` in the host header.
const ABI_VERSION: u32 = 1;

type LeanFn1 = unsafe extern "C" fn(*mut LeanObj) -> *mut LeanObj;
type VoidFn1 = unsafe extern "C" fn(*mut LeanObj);
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
type AddUncheckedFn = unsafe extern "C" fn(*mut LeanObj, *mut LeanObj) -> *mut LeanObj;

/// Host -> checker callback table (mirrors `lean_external_checker_host`,
/// field order must match exactly).
#[repr(C)]
pub struct Host {
    pub abi_version: u32,
    pub tick: Option<TickFn>,
    pub mk_ok: Option<LeanFn1>,
    pub mk_error: Option<LeanFn1>,
    pub mk_kernel_exception: Option<MkExcFn>,
    pub find_const: Option<FindConstFn>,
    pub builtin_add_decl: Option<AddDeclFn>,
    pub inc: Option<VoidFn1>,
    pub dec: Option<VoidFn1>,
    pub builtin_add_unchecked: Option<AddUncheckedFn>,
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

static HOST: AtomicPtr<Host> = AtomicPtr::new(ptr::null_mut());

#[inline]
fn host() -> &'static Host {
    unsafe { &*HOST.load(Ordering::Acquire) }
}

fn debug_on() -> bool {
    std::env::var_os("SOKONANODA_DEBUG").is_some()
}

/// The name object of a Check-kind `Declaration` (decl -> val -> ConstantVal -> name).
unsafe fn ls_decl_name(decl: *const LeanObj) -> *const LeanObj {
    let val = lean_sys::ctor_get(decl, 0);
    let cv = lean_sys::ctor_get(val, 0);
    lean_sys::ctor_get(cv, 0)
}

/// Reconstruct a Lean `Name` object's dotted string (for debug output).
unsafe fn name_to_string(n: *const LeanObj) -> String {
    use crate::term::NameKind::*;
    match Name(n).kind() {
        Anon => String::new(),
        Str(pre, s) => {
            let p = name_to_string(pre.0);
            if p.is_empty() {
                s.to_string()
            } else {
                format!("{p}.{s}")
            }
        }
        Num(pre, i) => {
            let p = name_to_string(pre.0);
            let i = i.as_usize().unwrap_or(0);
            if p.is_empty() {
                i.to_string()
            } else {
                format!("{p}.{i}")
            }
        }
    }
}

/// The persistent external checker state: sokonanoda's own growing environment.
///
/// One `Checker` PER WORKER THREAD (`thread_local!` below). Lean checks
/// declarations concurrently on a thread pool; giving each thread its own env
/// removes all shared mutable state, so checks run fully in parallel with no
/// lock. Cross-thread dependencies are handled by lazy import: a declaration
/// checked on thread T pulls any constant it references from the real-env
/// snapshot passed to its `add_decl` (`find_const`), regardless of which thread
/// added it. The only cost is that a shared constant may be imported once per
/// worker thread instead of once globally — bounded by the core count.
struct Checker {
    ef: ExportFile<'static>,
    nat_ext: bool,
    strg_ext: bool,
}

thread_local! {
    static CHECKER: RefCell<Checker> = RefCell::new(Checker::new());
}

fn make_config() -> Config {
    Config {
        export_file_path: None,
        use_stdin: false,
        permitted_axioms: None,
        unpermitted_axiom_hard_error: false,
        num_threads: 0,
        nat_extension: true,
        string_extension: true,
        pp_declars: None,
        unknown_pp_declar_hard_error: false,
        pp_options: PpOptions::default(),
        pp_output_path: None,
        pp_to_stdout: false,
        print_success_message: false,
        print_axioms: false,
        unsafe_permit_all_axioms: true,
    }
}

impl Checker {
    fn new() -> Checker {
        let config = make_config();
        let (nat_ext, strg_ext) = (config.nat_extension, config.string_extension);
        let dag = LeanDag::new(&config);
        let name_cache = dag.mk_name_cache();
        let ef = ExportFile {
            dag,
            declars: new_fx_index_map(),
            notations: new_fx_hash_map(),
            name_cache,
            config,
            mutual_block_sizes: new_fx_hash_map(),
        };
        Checker { ef, nat_ext, strg_ext }
    }

    /// Import the transitive constant closure of `roots` from the real env.
    unsafe fn ensure_closure(&mut self, h: &Host, env: *mut LeanObj, roots: &[*const LeanObj]) {
        let mut worklist = Vec::new();
        decode::collect_consts(roots, &mut worklist);
        let mut seen = new_fx_hash_set();
        while let Some(name_obj) = worklist.pop() {
            if !seen.insert(name_obj) {
                continue;
            }
            let np = {
                let mut imp = Importer::new(&mut self.ef.dag, self.nat_ext, self.strg_ext);
                match imp.import_name(Name(name_obj)) {
                    Ok(n) => n,
                    Err(_) => continue,
                }
            };
            if self.ef.declars.contains_key(&np) {
                continue;
            }
            let ci_opt = (h.find_const.unwrap())(env, name_obj as *mut LeanObj);
            if lean_sys::is_scalar(ci_opt) {
                if debug_on() {
                    eprintln!("[sokonanoda] find_const MISS {}", name_to_string(name_obj));
                }
                continue; // `none`: not a constant (e.g. the declaration being added)
            }
            let ci = lean_sys::ctor_get(ci_opt, 0);
            let ci_roots = decode::ci_expr_roots(ci);
            decode::collect_consts(&ci_roots, &mut worklist);
            // Inductive<->constructor<->recursor relations not reachable via exprs.
            worklist.extend(decode::ci_extra_deps(ci));
            let decoded = {
                let mut imp = Importer::new(&mut self.ef.dag, self.nat_ext, self.strg_ext);
                decode::decode_constant_info(&mut imp, ci)
            };
            match decoded {
                Ok(d) => {
                    if debug_on() {
                        let kind = match &d {
                            crate::env::Declar::Axiom { .. } => "axiom",
                            crate::env::Declar::Definition { .. } => "def",
                            crate::env::Declar::Theorem { .. } => "thm",
                            crate::env::Declar::Opaque { .. } => "opaque",
                            crate::env::Declar::Quot { .. } => "quot",
                            crate::env::Declar::Inductive(..) => "ind",
                            crate::env::Declar::Constructor(..) => "ctor",
                            crate::env::Declar::Recursor(..) => "rec",
                        };
                        eprintln!("[sokonanoda] import {} as {}", name_to_string(name_obj), kind);
                    }
                    self.ef.declars.insert(np, d);
                }
                Err(e) => {
                    if debug_on() {
                        eprintln!("[sokonanoda] decode FAIL {}: {}", name_to_string(name_obj), e);
                    }
                }
            }
            (h.dec.unwrap())(ci_opt);
        }
    }

    /// Check or delegate one declaration. `env` is owned; `decl`/`cancel`
    /// borrowed. Returns owned `Except Kernel.Exception Environment`.
    unsafe fn run_add_decl(
        &mut self,
        h: &Host,
        env: *mut LeanObj,
        max_heartbeat: usize,
        decl: *mut LeanObj,
        cancel: *mut LeanObj,
    ) -> *mut LeanObj {
        let decoded = {
            let mut imp = Importer::new(&mut self.ef.dag, self.nat_ext, self.strg_ext);
            decode::decode_declaration(&mut imp, decl)
        };
        let declar = match decoded {
            Err(e) => {
                (h.dec.unwrap())(env);
                return mk_other_error(h, &e.to_string());
            }
            Ok(Decoded::Delegate) => {
                return (h.builtin_add_decl.unwrap())(env, max_heartbeat, decl, cancel);
            }
            Ok(Decoded::Check(d)) => d,
        };

        let np = declar.info().name;
        // Import dependencies FIRST so they precede the new declaration in the
        // index map: `check_declar` uses `EnvLimit::ByName(np)`, whose cutoff is
        // `np`'s index, so only constants inserted before `np` are visible.
        let roots = decode::decl_expr_roots(decl);
        if debug_on() {
            let mut deps = Vec::new();
            decode::collect_consts(&roots, &mut deps);
            let names: Vec<String> = deps.iter().map(|&n| name_to_string(n)).collect();
            eprintln!("[sokonanoda] CHECK {} deps={:?}", name_to_string(ls_decl_name(decl)), names);
        }
        // Remove any prior entry for `np` (e.g. a forward placeholder) so that
        // after importing deps and re-inserting, `np` is the LAST entry and its
        // `ByName` cutoff sees every dependency.
        self.ef.declars.shift_remove(&np);
        self.ensure_closure(h, env, &roots);
        self.ef.declars.insert(np, declar);
        self.ef.name_cache = self.ef.dag.mk_name_cache();

        let accepted = {
            let ef = &self.ef;
            let dr = ef.declars.get(&np).unwrap();
            panic::catch_unwind(AssertUnwindSafe(|| ef.check_declar(dr)))
        };
        // The checked declaration is TRANSIENT: it was inserted only for its own
        // `ByName` check. Remove it so the persistent env holds only constants
        // lazily imported from the real environment (always their final form).
        // Under Lean's parallel elaboration a declaration is added in stages
        // (signature then body) possibly on different worker threads; keeping our
        // own checked copy could pin an intermediate form. A later reference
        // re-imports the committed, final version via `find_const`.
        self.ef.declars.shift_remove(&np);
        match accepted {
            Ok(()) => (h.builtin_add_unchecked.unwrap())(env, decl),
            Err(payload) => {
                (h.dec.unwrap())(env);
                mk_other_error(h, &panic_msg(&payload))
            }
        }
    }
}

fn panic_msg(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        format!("(sokonanoda) {s}")
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("(sokonanoda) {s}")
    } else {
        "(sokonanoda) declaration rejected".to_string()
    }
}

unsafe fn mk_other_error(h: &Host, msg: &str) -> *mut LeanObj {
    let s = lean_sys::mk_string(msg);
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

unsafe extern "C" fn add_decl(
    self_: *mut c_void,
    env: *mut LeanObj,
    max_heartbeat: usize,
    decl: *mut LeanObj,
    cancel: *mut LeanObj,
) -> *mut LeanObj {
    let _ = self_; // checker state is thread-local, not carried in `self`
    let h = host();
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        CHECKER.with(|c| c.borrow_mut().run_add_decl(h, env, max_heartbeat, decl, cancel))
    }));
    match result {
        Ok(obj) => obj,
        Err(_) => {
            // Panic outside the (already-caught) check: env was not consumed.
            (h.dec.unwrap())(env);
            mk_other_error(h, "sokonanoda external checker panicked")
        }
    }
}

/// The one symbol the host resolves via `dlsym`.
///
/// # Safety
/// `host`/`out` must point to valid `Host`/`Callbacks` from the host.
#[no_mangle]
pub unsafe extern "C" fn lean_external_check_populate_callbacks(host: *const Host, out: *mut Callbacks) -> u32 {
    if host.is_null() || (*host).abi_version != ABI_VERSION {
        return 1;
    }
    HOST.store(host as *mut Host, Ordering::Release);
    // sokonanoda signals a rejected declaration by panicking; we catch those, so
    // silence the default panic printer unless debugging.
    if !debug_on() {
        panic::set_hook(Box::new(|_| {}));
    }
    let out = &mut *out;
    out.abi_version = ABI_VERSION;
    out.self_ = ptr::null_mut(); // checker state is thread-local
    out.add_decl = Some(add_decl);
    out.whnf = None;
    out.check = None;
    out.is_def_eq = None;
    out.release = None;
    0
}
