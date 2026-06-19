# Explain & plan: remaining errors + parallelism

## 1. What errors remain (97.7% parity; 18/783 fail)

All residual failures fall into two buckets.

### A. `def_eq failed` (14 files) — sokonanoda conversion-checker incompleteness
Instrumenting `assert_def_eq` (gated by `SOKONANODA_DEBUG`) shows e.g. 13581 and
unusedVarDoMatch both fail on the **same** goal while checking a matcher helper
`_private._..._sparseCasesOn_1`:

```
u = Eq Bool (Nat.beq (Nat.land 1 (Nat.shiftRight 2 0)) 1) Bool.false
v = Eq Bool Bool.false Bool.false
whnf u = (unchanged)         <-- sokonanoda does NOT reduce the Nat.beq term
```

`Nat.shiftRight 2 0 = 2`, `Nat.land 1 2 = 0`, `Nat.beq 0 1 = false`, so `u ≡ v`.
The builtin kernel reduces it; sokonanoda leaves it stuck.

Key diagnostic facts:
- The **identical term in isolation reduces fine** (`example : Nat.beq (Nat.land 1
  (Nat.shiftRight 2 0)) 1 = false := rfl` passes under the external checker).
- Each individual op (land/lor/xor/shiftLeft/shiftRight/beq/ble/mul) reduces fine.
- At the failing decl the `name_cache` IS populated (`land=true shr=true
  beq=true`), so it is **not** a missing-name-cache problem.

=> It is a **context-sensitive whnf bug inside sokonanoda**: the Nat-literal
reduction does not fire while checking certain matcher/`sparseCasesOn` helpers,
even though it fires for the same term standalone. Likely the operands are
reachable only behind extra delta/proj unfolding that sokonanoda's `whnf` skips
in that position (e.g. it tries the nat-extension shortcut before/without fully
forcing `OfNat.ofNat (instOfNatNat _)` to a `NatLit` in this code path).

This is sokonanoda-internal (an experimental NbE checker), not an integration
bug. Fixing it = improving sokonanoda's `whnf`/`try_reduce_nat` to force operands
in all positions. Repro file: any of 13581, unusedVarDoMatch, reduceBEqSimproc.

### B. Timeouts (3 files: bv_llvm, 6043, heavy grind)
sokonanoda is slower than the builtin on some bit-vector / very heavy reduction
inputs and exceeds a 30-40s wall limit. Not a correctness issue; a performance
tail. Would shrink with the whnf/caching improvements above and real parallelism
(below).

## 2. The `-D Elab.async=false` situation

`Elab.async=false` is currently used for **reproducible measurement**, not
because the checker needs serial elaboration. Two distinct things are entangled:

### (i) Flakiness (why async=false helps)
With async ON, Lean's parallel elaboration emits **different but equivalent**
proof terms across runs. Because of the def-eq incompleteness in (A), sokonanoda
rejects *some* of those equivalent forms, so a handful of files flip pass/fail
run-to-run. `async=false` makes elaboration deterministic, so results are
reproducible. **The principled fix is (A)** — a complete def-eq is robust to
whichever equivalent term elaboration produces; then async can stay ON.

### (ii) Serialization (the real parallelism cost)
Independently, the checker wraps its state in a single `Mutex<Checker>`, so even
with async ON every kernel check is **serialized**. We get correctness but throw
away Lean's parallel checking. This is the thing to fix for performance.

## 3. Plan to keep parallelism

Goal: drop the global `Mutex`, let Lean's worker threads check concurrently,
while keeping "import each constant once (per worker)".

### Recommended: per-thread checker contexts (matches Lean's async branching)
Lean already checks each declaration against an **immutable environment
snapshot** on its own task/thread. Mirror that:

- Replace the single `Mutex<Checker>` with **thread-local `Checker`s** (one
  `ExportFile` per worker thread; `thread_local!` keyed by the OS thread, or use
  the plan's `thread_enter`/`thread_exit` host hooks to get a per-thread `ctx`).
- No shared mutable state => no lock => fully concurrent checks. sokonanoda
  already allocates a fresh per-check `Bump`, so only the persistent `ExportFile`
  needs to become per-thread.
- Cross-thread dependencies are already handled: a decl checked on thread T
  lazily imports any constant it references from the **real env snapshot** passed
  to its `add_decl` (`find_const`), which contains its dependencies regardless of
  which thread added them.

Trade-off: a shared constant (e.g. `Nat`) is imported once *per thread* instead
of once globally — bounded by core count, cheap relative to checking, and exactly
the cost Lean's own per-branch model pays.

### Alternative: `RwLock<ExportFile>` with phase split
Keep one shared env; take a write lock only for the import+insert phase, a read
lock for `check_declar`. Cheaper memory, but the ByName cutoff/index shifts under
concurrent inserts make this fiddly and lower-throughput than per-thread.

### Steps
1. Make `Checker` per-thread (`thread_local!`), drop `HOST`-global `Mutex`.
2. Ensure `find_const`/`builtin_add_*`/`inc`/`dec` are callable from any thread
   (they are — they touch only the passed objects + the real env).
3. Re-run the corpus with async ON; expect determinism only after fix (A), but
   throughput should scale with cores immediately.
4. Re-measure: end-to-end wall clock with async ON should now beat the serialized
   number on multi-decl files.

## TL;DR
- Remaining failures = (A) a context-sensitive nat-literal `whnf` gap in
  sokonanoda + (B) a few timeouts. Both are checker-internal, not integration.
- `async=false` is a measurement aid for the flakiness that (A) causes; fix (A)
  and async can stay on.
- For real parallelism, replace the global `Mutex` with **per-thread `Checker`
  contexts**; cross-thread deps are already covered by per-thread lazy import.
