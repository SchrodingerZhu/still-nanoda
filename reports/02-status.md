# Status: live external checker

## Working
`lean --external-checker-lib=libsokonanoda.so <file>` runs sokonanoda as the
kernel for axiom/def/thm/opaque declarations, delegating inductive/quot/mutual to
the builtin kernel, lazily importing referenced constants from the real env.

- Correct accept/reject on hand battery (typeclasses, structures, lists, props,
  pattern-match, recursion via `Nat.brecOn`, nat-literal arithmetic).
- Real corpus (tests/elab, first 200, 196 plain-accepted): **173 ext-accept,
  23 ext-reject = 88% parity**. Rejects bad proofs correctly.

## Key implementation points
- Persistent `ExportFile<'static>` in the checker (`Mutex` for Lean's parallel
  checking). Each constant imported once; resolved by name thereafter.
- `add_decl`: import deps closure (via `find_const`) BEFORE the decl, re-add the
  decl last so `EnvLimit::ByName` cutoff sees all deps. Recursive defs need this.
- mpz bignums decoded from the GMP `__mpz_struct` limbs.
- Rejection = Rust panic caught by `catch_unwind` -> `Kernel.Exception.other`.

## Failure categories (23/196) — to investigate
- `eval.rs:680 spine_type: bad proj` (refl.lean) — projection reduction.
- `find_const MISS *.eq_def` (4673.lean) — equation lemmas not found via find_const.
- `def_eq failed` / `let-declaration type mismatch` (10577.lean) — def-eq edge
  (10577 is the known lazy_delta level-param case).
- mutual/structural (structuralMutual, TermSeq) — no clear error yet.
- Several grind_* (heavy) — may be timeouts or unsupported.

Mix of (a) sokonanoda NbE limitations (experimental checker) and (b) possible
import bugs. 88% parity is the current baseline.
