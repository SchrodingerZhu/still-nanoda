# Plan: fix sokonanoda's whnf/def-eq gap (final stage)

## Target (high leverage)
The dominant residual failure is the **Nat literal reduction not firing in
context**: `Nat.beq (Nat.land 1 (Nat.shiftRight 2 0)) 1 ≡ Bool.false` is left
stuck by `whnf` while checking certain matcher / `sparseCasesOn` helpers, even
though:
- the identical term reduces fine **in isolation**, and
- every individual op (land/shiftRight/beq/...) reduces fine, and
- the `name_cache` is populated at the failing decl.

This single gap accounts for 13581, unusedVarDoMatch, reduceBEqSimproc,
grind_cutsat_div_1 (4 of 5 sampled def-eq failures). The 5th (grind_clean_den)
is a separate field/ring conversion case.

## Investigation approach
1. Read sokonanoda's reduction core: `whnf` / `whnf_core` (tc.rs ~664), the
   nat-literal shortcut `try_reduce_nat` (tc.rs ~327) and its `NatBinOp`
   dispatch, and how `def_eq_core` drives whnf.
2. Determine the condition under which `try_reduce_nat` does NOT fire on
   `Nat.beq (Nat.land …) 1`. Leading hypotheses:
   - (H1) `try_reduce_nat` requires its operands to already be `NatLit`, and the
     enclosing `whnf` does not recursively force operands like
     `OfNat.ofNat α n (instOfNatNat n)` to `NatLit` in this position.
   - (H2) The matcher-helper check runs whnf in a reduced mode
     (`whnf_no_unfolding` / cheap-proj) that bypasses the nat extension.
   - (H3) `def_eq_core` compares structurally and recurses into args with a whnf
     variant that doesn't apply the nat extension to nested redexes.
3. Pinpoint with a targeted trace (instrument `try_reduce_nat` entry / the whnf
   variant used by the matcher path).

## Fix shape (to be confirmed by the above)
Make the nat-literal reduction robust to un-forced operands: before bailing,
`whnf` each operand of a recognized `Nat.*` op (forcing `OfNat.ofNat`/`instOfNat`
to `NatLit`) and retry; or ensure the matcher/`sparseCasesOn` path uses the full
`whnf` (which includes the nat extension) rather than a reduced variant.

## Validation
- Re-run 13581 / unusedVarDoMatch / reduceBEqSimproc / grind_cutsat_div_1.
- Re-run the tests/elab parity sweep (async OFF and ON); expect parity up from
  97.7% / 92%.
- Guard against regressions on the passing set + the perf benchmarks.

## Findings (ROOT CAUSE)
def-eq is NbE, not the `whnf`/`try_reduce_nat` path. `def_eq_core` -> `eval` ->
`unify`, and `unify` forces values **shallowly** via `force_thunk`. For a nat-op
head that path calls `do_nat_red_shallow` = `do_nat_red_at(.., deep=false)`,
which extracts each operand's bignum via `value_to_bignum_at(arg, deep=false)`.

In `value_to_bignum_at` (src/eval.rs), when the operand is a `Value::Unfold`
(a not-yet-reduced nat-op application such as `Nat.land 1 (Nat.shiftRight 2 0)`):
```
Value::Unfold { head_value, .. } => {
    if let Some(NatLit ..) = head_value.get() { ... }
    if !deep { return None; }          // <-- shallow GIVES UP on a nested nat op
    return self.bignum_via_force(cur)..;
}
```
So `Nat.beq (Nat.land …) 1` can't get `Nat.land …`'s value and stays stuck;
`conv_nat`/`unify` then can't equate it to `Bool.false`. In isolation the
elaborator pre-reduces the operands, so the kernel never exercises this.

`H1` confirmed (operands not forced); `H2`/`H3` were wrong (it's the NbE path,
and `force_all`/`do_nat_red` already pass `deep=true` — only the shallow force
path is buggy).

## Fix
A nat operation can only be computed once its operands are concrete bignums, so
operand extraction must force them — regardless of the outer shallow/deep mode.
Change `do_nat_red_at` to extract operand bignums with `deep=true` always
(`value_to_bignum_at(arg, true)`). This makes the shallow force path reduce
nested nat ops too; `force_all`/`do_nat_red` are unaffected (already deep). Bounded
work (operands are nat expressions reducing to bignums) and results are cached.
