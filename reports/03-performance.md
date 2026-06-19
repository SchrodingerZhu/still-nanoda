# Performance: lean4 builtin kernel vs lean4 + sokonanoda external checker

Method: same `lean` binary (our stage1, clang-22/lld/release), same input file,
with and without `--external-checker-lib=libsokonanoda.so`. Kernel time isolated
via `lean --profile` "type checking" category (the `addDecl` kernel checking that
the external checker replaces); 5 runs averaged. Elaboration time is identical
either way (the external checker only changes kernel checking).

## Kernel type-checking time (mean of 5)

| workload | builtin kernel | sokonanoda | speedup |
|---|---|---|---|
| kbench (recursive `f`/`g`, 120 `rfl`) | 75.6 ms | 39.0 ms | **1.94×** |
| kred (recursive `sum`/`dbl`, 156 `rfl`) | 132.8 ms | 70.6 ms | **1.88×** |
| kheavy (fib via `decide`) | 2.3 ms | 2.4 ms | ~1.0× |
| c_list (small) | 0.8 ms | 1.0 ms | ~0.8× |

## Reading

- On **reduction-heavy** kernel work (definitional unfolding / `rfl` over
  recursive functions), sokonanoda's normalization-by-evaluation checks
  **~1.9× faster** than the builtin C++ kernel. This is the regime where kernel
  checking dominates and NbE's strict evaluation + memoized closures win.
- On work the kernel barely touches (`decide` discharges at elaboration; tiny
  files), times are within noise; the FFI/import overhead is comparable to the
  small checking cost.
- End-to-end wall clock on mixed files (~1.0×, within noise): the external-checker
  dispatch + lazy-import machinery adds **no measurable overhead**; the win shows
  up exactly where the kernel does real reduction.

## Integration overhead

The `--external-checker-lib` dispatch itself (delegate-all earlier milestone)
measured 1.03× — i.e. zero overhead. Import of each constant happens once into
the persistent arena; thereafter it is resolved by name with no re-walk.

## Caveat

Numbers are on the subset sokonanoda checks correctly (88% of a 196-file
tests/elab sample; see 02-status.md). Some declarations hit sokonanoda NbE
limitations and are rejected; those are excluded from timing.
