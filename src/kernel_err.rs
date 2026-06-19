//! Structured kernel errors carried out of the type checker via
//! `std::panic::panic_any`, so the integration layer can rebuild the exact
//! `Lean.Kernel.Exception` instead of a generic `other` message.
//!
//! The panic payload must be `'static + Send`, so arena-bound `NamePtr`/`ExprPtr`
//! cannot cross the unwind: names are carried as their rendered dotted `String`
//! (see `TcCtx::name_to_string`). Only the "name-only" variants live here for now
//! — the ones whose `Kernel.Exception` message renders no expressions
//! (see src/Lean/Message.lean). Expr-carrying variants need expr export (Phase 2).

/// An owned, `Send` `lean_object *` exported at the panic site (e.g. a type
/// expression). The pointer references a Lean-runtime heap object, independent of
/// sokonanoda's arena, so it survives the unwind. The catch site takes ownership.
pub struct SendObj(pub *mut core::ffi::c_void);
// SAFETY: the referenced Lean object is not aliased across threads; it is built
// on the checking thread and consumed by that thread's catch handler.
unsafe impl Send for SendObj {}

impl std::fmt::Debug for SendObj {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SendObj({:p})", self.0)
    }
}

/// A kernel rejection with enough structure to reconstruct the matching
/// `Lean.Kernel.Exception` constructor host-side. Variant order mirrors the
/// `Kernel.Exception` codes used by `lean_extern_mk_kernel_exception`.
#[derive(Debug)]
pub enum KernelErr {
    /// code 0: `unknown constant '{name}'`
    UnknownConstant { name: String },
    /// code 1: constant `{name}` has already been declared
    AlreadyDeclared { name: String },
    /// code 3: declaration `{name}` has metavariables
    DeclHasMVars { name: String },
    /// code 7: `let-declaration type mismatch '{name}'`
    LetTypeMismatch { name: String },
    /// code 2: declaration type mismatch — the declaration's value has
    /// `given_type` but its declared type differs. The expected type comes from
    /// the `decl` object host-side; `given_type` is exported here.
    DeclTypeMismatch { given_type: SendObj },
    /// The host `tick` requested an abort: `reason` is a nullary `Kernel.Exception`
    /// tag (13 = deterministicTimeout, 16 = interrupted).
    Aborted { reason: i32 },

    /// code 4: declaration `{name}` has free variables in `e`.
    DeclHasFVars { name: String, e: SendObj },
    /// code 9: `application type mismatch {app}` (argument has `arg_type`, function
    /// has `fn_type`).
    AppTypeMismatch { app: SendObj, fn_type: SendObj, arg_type: SendObj },
    /// code 11: `type of theorem '{name}' is not a proposition{ty}`.
    ThmTypeIsNotProp { name: String, ty: SendObj },
}
