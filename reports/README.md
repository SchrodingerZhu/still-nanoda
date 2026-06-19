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
  (accept/reject) on **97.7%** of a 291-file tests/elab sample (deterministic
  with `-D Elab.async=false`).
- Kernel type-checking is **~2× faster** than the builtin C++ kernel on
  reduction-heavy declarations; zero measurable integration overhead.
- Remaining gaps are sokonanoda's own NbE conversion-checker limitations
  (matcher/derived-`BEq` def-eq) + a couple of timeouts, not integration bugs.
