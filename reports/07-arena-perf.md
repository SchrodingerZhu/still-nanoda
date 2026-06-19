# Performance: cedar & mathlib (lean4 vs lean4 + external checker)

Setup: the lean-kernel-arena `kernel` replay tool (Lean `Environment.replay` over a
lean4export ndjson) built against our lean (4.33.0-pre, the `external-checker-lib`
branch). "lean4" = builtin kernel; "lean4 + external" = same tool with
`LEAN_EXTERNAL_CHECKER_OVERRIDE=libsokonanoda.so`. Exports: cedar 729 MB / 120 026
decls, mathlib 5.3 GB. Both modes accept identically.

## IMPORTANT: lean's replay is single-threaded

`Environment.replay` adds constants sequentially (`addDeclCore` one at a time), so
the replay comparison below is **single-threaded** for BOTH modes — lean's replay
has no `-j`. sokonanoda's per-thread parallelism (the thread-local checkers) only
engages when lean drives `addDecl` concurrently (parallel *elaboration* of source,
`Elab.async`), which the sequential replay never does. For the genuine 4-thread
number we therefore also list sokonanoda's **native** parallel checker (the arena
`still-nanoda` checker, `num_threads=4`), which reads the same ndjson directly.

## Numbers

### Single-threaded replay (apples-to-apples, our integration)

| workload | decls | lean4 (builtin) | lean4 + external | ratio |
|---|---|---|---|---|
| cedar   | 120 026 | 65.9 s wall (1.2 GB) | 70.0 s wall (1.2 GB) | external 1.06× **slower** |
| mathlib | 670 627 | 1202 s wall (9.6 GB) | 1079 s wall (9.6 GB) | external **1.11× faster** |

Both modes accept every declaration on both corpora — full accept/reject parity
on ~790 k real declarations.

### 4 threads — sokonanoda native parallel checker (arena `still-nanoda`)

| workload | lean4 builtin (1 thread, replay) | sokonanoda native (4 threads) | speedup |
|---|---|---|---|
| cedar   | 65.9 s wall | 14.8 s wall (42.7 s cpu, ~2.9× parallel) | ~4.5× |
| mathlib | 1202 s wall | 178 s wall (566 s cpu, ~3.2× parallel) | ~6.7× |

## Reading the result

- **Single-threaded, end-to-end, the external checker is ≈ parity** — corpus
  dependent: **1.11× faster on mathlib**, 1.06× slower on cedar. The earlier "~2×"
  was *kernel reduction time in isolation* (`--profile` "type checking"). Net
  end-to-end, the NbE reduction win competes with a per-decl cost the builtin does
  not pay: importing each declaration's `lean_object` DAG into sokonanoda's arena
  (and committing it back via `builtin_add_unchecked`). Proof-heavy mathlib has
  enough reduction for the win to dominate; definition-heavy cedar does not.
- **The external checker's real lever is parallelism.** sokonanoda checks each
  declaration on its own thread with no shared lock; its native 4-thread run does
  cedar in 14.8 s and mathlib in 178 s — ~4.5× and (pending) faster than the
  single-threaded builtin replay. Realising this *inside lean* requires lean to
  add declarations concurrently (parallel elaboration), which the replay harness
  does not exercise.
- **Memory** is essentially identical between the two single-threaded modes
  (1.2 GB cedar, 9.6 GB mathlib) — the external checker's arena is offset by it
  not growing a second full kernel env. The native 4-thread run trades memory for
  speed (5.6 GB on mathlib).
- **Correctness at scale**: identical accept on 120 026 + 670 627 = ~790 k real
  declarations is the strongest parity evidence to date.

## Performance investigation (profiling + ablation)

Profiled the external replay (`perf`) and ablated the recently-added/suspect
paths. Conclusion: **no sokonanoda hotspot; the recent changes do not regress
performance.**

- **`perf` (init/cedar-like, definition+inductive-heavy)**: time is dominated by
  the ndjson **parse** (`Export.parseStream`, `Json.Parser`, `mi_malloc`/`mi_free`
  ~54%, `get_line`) and lean's **env management** (`replace_rec_fn` in
  `add(check:=false)`), plus the builtin kernel checking the **delegated
  inductives**. `libsokonanoda.so` barely appears — its checking is a small
  fraction. All of this is identical between the two modes, so the end-to-end
  "1.06× slower / 1.11× faster" is a *small* checker difference diluted by a large
  common parse cost.
- **heartbeat (tick) overhead** — ablated to a no-op on a reduction-heavy test
  (app-lam): 2.89 s vs 2.94 s, i.e. within noise. The const-init thread-local +
  4096-gated FFI is effectively free.
- **transient-decl re-import** — keeping every checked declaration instead
  (no re-import) was *slower* on init (34.7 s vs 33.3 s): the transient design
  keeps the working set small, so it is a net win, not a cost.
- **`mk_name_cache` recompute per add_decl** — computing it once vs every
  declaration: no measurable difference on init (within noise).

The genuinely sokonanoda-specific cost is DAG **import** (decode `lean_object` →
arena), inherent to any checker with its own representation; it is not dominant
and not safely reducible (e.g. type-only theorem import trades a soundness-subtle
proof-irrelevance assumption for a parse-diluted, likely-small gain — not worth
it).

## Caveats
- The native 4-thread figures are from the arena's `still-nanoda` checker (a
  recent sokonanoda rev, same NbE core), not this exact `binding` build, because
  our build is now a cdylib for the lean integration and no longer ships a
  standalone ndjson binary.
- A 4-thread number for the *integration* would need a parallel replay or a
  buildable parallel source workload; lean's `Environment.replay` is sequential.
