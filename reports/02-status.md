# Status: live external checker

## Working
`lean --external-checker-lib=libsokonanoda.so <file>` runs sokonanoda as the
kernel for axiom/def/thm/opaque declarations, delegating inductive/quot/mutual to
the builtin kernel, lazily importing referenced constants from the real env.

## Parity (tests/elab corpus, plain-accepted files)
- First 200, parallel elab: 173/196 = 88%.
- **Deterministic (`-D Elab.async=false`): 181/195 = 93%.**
- After Proj/constructor-closure fix: see 04-final.md.

## Two important findings
1. **Nondeterminism under parallel elaboration.** Lean's async elaboration emits
   different (but equivalent) proof terms across runs; sokonanoda's def-eq is
   incomplete for some forms, so a few files were flaky. `-D Elab.async=false`
   makes elaboration (and hence the checker result) deterministic.
2. **Missing inductive/constructor closure (FIXED).** `Expr.proj S idx e` needs
   the structure `S` and its constructor in the env; an inductive needs its
   constructors for `can_be_struct`/`is_ctor_app`. These are not reachable via
   the ConstantInfo's expressions, so they were never imported -> "bad proj" and
   several "def_eq failed". Now collected/imported explicitly. Fixed 6+ files.

## Remaining failures (genuine sokonanoda NbE limitations)
- `def_eq failed` (13581, reduceBEqSimproc, unusedVarDoMatch, ...): sokonanoda's
  def-eq differs from the builtin on some forms. Not import bugs (constructors
  now present). Would require improving sokonanoda's conversion checker.
- timeouts (bv_llvm, 6043): sokonanoda slower on bit-vector/heavy reduction.
- 10577: known lazy_delta level-param edge case.

## Implementation notes
- Persistent `ExportFile<'static>` under a `Mutex` (Lean checks in parallel).
- `add_decl`: import transitive const closure (find_const) BEFORE the decl;
  re-add the decl last so `EnvLimit::ByName` sees all deps (needed for recursion).
- mpz bignums decoded from GMP `__mpz_struct` limbs.
- Rejection = Rust panic caught by `catch_unwind` -> `Kernel.Exception.other`.
