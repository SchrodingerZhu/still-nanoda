//! Structured kernel errors carried out of the type checker via
//! `std::panic::panic_any`, so the integration layer can rebuild the exact
//! `Lean.Kernel.Exception` instead of a generic `other` message.
//!
//! The panic payload must be `'static + Send`, so arena-bound `NamePtr`/`ExprPtr`
//! cannot cross the unwind: names are carried as their rendered dotted `String`
//! (see `TcCtx::name_to_string`). Only the "name-only" variants live here for now
//! — the ones whose `Kernel.Exception` message renders no expressions
//! (see src/Lean/Message.lean). Expr-carrying variants need expr export (Phase 2).

/// A kernel rejection with enough structure to reconstruct the matching
/// `Lean.Kernel.Exception` constructor host-side. Variant order mirrors the
/// `Kernel.Exception` codes used by `lean_extern_mk_kernel_exception`.
#[derive(Debug, Clone)]
pub enum KernelErr {
    /// code 0: `unknown constant '{name}'`
    UnknownConstant { name: String },
    /// code 1: constant `{name}` has already been declared
    AlreadyDeclared { name: String },
    /// code 3: declaration `{name}` has metavariables
    DeclHasMVars { name: String },
    /// code 7: `let-declaration type mismatch '{name}'`
    LetTypeMismatch { name: String },
}
