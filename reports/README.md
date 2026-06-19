# sokonanoda external-checker integration — reports

sokonanoda built as a Lean 4 `--external-checker-lib` plugin: it runs as the
kernel for the declarations Lean elaborates, reproducing the builtin kernel's
accept/reject behaviour, and is faster on reduction-heavy checking.

- **04-final.md** — the summary: what was built, 96.9% parity, ~2× kernel speedup.
- **03-performance.md** — kernel-checking timings (builtin vs sokonanoda).
- **02-status.md** — correctness status, failure categories, key fixes.
- **01-layouts.md** — `Declaration`/`ConstantInfo` `lean_object` layout reference.

## TL;DR
- `lean --external-checker-lib=libsokonanoda.so file.lean` ≈ `lean file.lean`
  (accept/reject) on **99.2%** of the tests/elab sample (deterministic with
  `-D Elab.async=false`), up from 97.7% after fixing the def-eq nat-reduction gap.
- True parallelism: per-thread checkers (no lock); Lean's workers check
  concurrently under async ON.
- Kernel type-checking is **~1.8–2× faster** than the builtin C++ kernel on
  reduction-heavy declarations; zero measurable integration overhead.
- The dominant def-eq gap is fixed (module-hidden nat ops imported as axioms —
  see 05-defeq-fix-plan.md). Remaining few are a timeout, a Syntax projection,
  and a String/Name comparison — distinct, smaller categories.
