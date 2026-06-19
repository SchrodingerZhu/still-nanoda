# Plan & findings: recovering kernel error messages

Goal: make sokonanoda's rejections produce the SAME `Kernel.Exception` message
the builtin kernel would, instead of the generic `(kernel) (sokonanoda) def_eq
failed`. (Concretely this is what test 10577 pins via `#guard_msgs`.)

## Findings

1. **sokonanoda has no structured errors.** Every kernel error is a `panic!`
   with a string (tc.rs has ~30 `panic!` sites: `def_eq failed`, `funExpected`-
   like `ensure_pi`, `declaration not found`, etc.). There is no error enum.

2. **The integration maps every panic to `Kernel.Exception.other`** (code 12),
   shown as `(kernel) (sokonanoda) <panic text>`. That `(sokonanoda)` marker and
   the generic text are why messages differ from the builtin.

3. **The host ABI already supports all 17 variants.** `lean_extern_mk_kernel_
   exception(code, env, lctx, name, decl, e0, e1, e2, msg)` builds the exact
   `Kernel.Exception` ctor. So the missing piece is purely on the checker side:
   produce the right code + payloads.

4. **Mechanism (per your suggestion): `std::panic::panic_any(KernelErr)`** then
   `downcast_ref::<KernelErr>()` at the existing `catch_unwind` in extck.rs. The
   payload must be `'static + Send`, so it carries OWNED data (e.g. names as
   `String`, since the arena-bound `NamePtr`/`ExprPtr` can't cross the unwind).

5. **What each message actually needs** (src/Lean/Message.lean:867). Two classes:
   - **Name-only (NO exprs in the message):**
     - `unknownConstant env name`            → `unknown constant '{n}'`
     - `alreadyDeclared env name`            → `… already been declared '{n}'`
     - `declHasMVars env name _`             → name only (expr ignored)
     - `letTypeMismatch env lctx n _ _`      → `let-declaration type mismatch '{n}'`  ← 10577
   - **Expr-carrying (message renders exprs via `indentExpr`):**
     - `funExpected/typeExpected/invalidProj env lctx e`
     - `exprTypeMismatch env lctx e _`        (1 expr)
     - `appTypeMismatch env lctx app fn arg`  (3 exprs)
     - `declTypeMismatch env decl given` / `declHasFVars … e` / `thmTypeIsNotProp … type`

6. **Empty `LocalContext` is available**: `lean_mk_empty_local_ctx(box(0))`. The
   name-only messages call `mkCtx env lctx …` but print no exprs, so an empty
   lctx is sufficient for them.

7. **Expr export does NOT exist.** sokonanoda is NbE; its exprs are arena
   `ExprPtr`/`Value`, not `lean_object`. The expr-carrying variants need an
   `ExprPtr → lean_object` builder (all `Expr`/`Level`/`Name`/literal ctors via
   FFI) — and, to pp those exprs under their binders, the **local context**
   (fvar names/types) the error occurred under, which sokonanoda tracks as de
   Bruijn levels, not a Lean `LocalContext`. Both are real work.

## Feasibility verdict
- **Name-only variants: feasible now, cleanly, no expr export.** Fixes 10577 and
  upgrades the common `unknown constant` / `already declared` cases to exact
  messages. ~self-contained.
- **Expr-carrying variants: feasible but a real project** (expr export + lctx
  reconstruction). High effort; needed for type-mismatch messages to be exact.

## Proposed phasing

**Phase 1 (small, recommended first):** structured `KernelErr` for the name-only
variants.
- Add `enum KernelErr { UnknownConstant{name:String}, LetTypeMismatch{name:String},
  DeclHasMVars{name:String}, AlreadyDeclared{name:String} }`.
- `panic_any(KernelErr::…)` at the matching tc.rs sites (infer_const not-found;
  infer_let's value/type def-eq; etc.).
- extck.rs: downcast → `mk_kernel_exception(code, env, /*lctx*/null, mk_name(name))`.
- Host: when `lctx==null`, fill `lean_mk_empty_local_ctx`. For name-only codes,
  pass null for the ignored expr fields (verify no traversal touches them; else
  pass a trivial `Sort 0`).
- Drop the `(sokonanoda)` marker for these so text matches the builtin exactly.
- Outcome: 10577 passes; parity 100% on the sampled set.

**Phase 2 (larger, optional):** expr export + the expr-carrying variants.
- Implement `ExprPtr → lean_object` (quote from `Value` or read the `ExprPtr`
  arena node; build via `lean_expr_mk_*`, `lean_level_mk_*`, name/lit ctors).
- Decide lctx handling: simplest is an empty lctx (fvars print as opaque
  `_fvar.N`), which makes the STRUCTURE/wording match but not the bound-variable
  names; full fidelity needs reconstructing the `LocalContext`.
- Map `appTypeMismatch`/`exprTypeMismatch`/`funExpected`/… with exported exprs.

## Open questions for discussion
1. Is Phase 1 alone enough for now (exact messages where no exprs are shown,
   generic elsewhere), or do you want Phase 2 (expr export) too?
2. For name-only variants, OK to pass null for the message-ignored expr fields,
   or should I always synthesize a placeholder `Sort 0` for safety?
3. Drop the `(sokonanoda)` marker entirely (for exact text match), or keep it on
   the `other` fallback only (so genuine sokonanoda-specific failures stay
   identifiable)?
4. Phase 2 lctx: accept opaque fvar names (structure matches, names don't), or
   invest in full `LocalContext` reconstruction?
