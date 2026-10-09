# jit-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../../_strategy/LANG_COLLECTION.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt.

## v0.2.0 - Core (THE HARD PART, NOT DEFERRED) (DONE)
Lowers IR to machine code in executable memory via Cranelift and runs it immediately.
Dependencies (wires ir, pager, Cranelift) are wired here, when first used.
Exit criteria:
- [x] Every public item has rustdoc + a runnable example.
- [x] Core invariants property-tested (API authored at this stage in docs/API.md).

Decisions recorded here per the anti-deferral rule:
- Cranelift 0.133 is the code generator (cranelift-codegen + cranelift-frontend +
  cranelift-native); pager-lang hosts the emitted bytes in a read-execute region. No
  cranelift-jit, so memory management stays with pager-lang as planned.
- MSRV raised to 1.94 — the floor cranelift-codegen 0.133 requires. This is the only
  crate in the family above the 1.85 baseline; the dependency imposes it.
- Scope held to leaf functions over int/float/bool with a unit return. A unit-typed
  parameter is rejected (no machine representation), not silently lowered.

## v1.0.0 - API freeze (DONE)
Public surface stable and frozen until 2.0.
- [x] docs/API.md marked stable; SemVer promise recorded.
- [x] Full test + benchmark suite green on all three platforms (CI matrix covers Linux,
      macOS on ARM64, and Windows; x86-64 verified locally in WSL2, both ARM64 targets
      cross-checked to compile).

Frozen 1.0 surface: `Jit` / `Jit::new` / `Jit::compile`, the free `compile`, `Compiled`
(`name` / `params` / `ret` / `code_len` / `as_ptr` / `unsafe entry::<F>()`), and
`JitError` (`#[non_exhaustive]`). Hardening at the freeze: ARM64 instruction-cache
coherence in `place()`, so freshly compiled code runs correctly on aarch64 (x86-64 was
already coherent). Additive 1.x room: function calls, wider machine types, and more
back-ends or optimization levels — each a minor release when it lands.

## v1.0.1 - Patch: non-trapping division, Windows ARM64 i-cache, bool ABI (DONE, 2026-10-08)
Fixes ISSUES **H10**, **M55**, and **M56** from `_lexersketch/ISSUES.md`. No public API
change.

Delivered:
- **H10 (process-killing traps).** Integer `div` lowers to a branch-free guarded
  sequence instead of a bare `sdiv`: the divisor is swapped for `1` when it is `0` or
  `-1`, so the hardware divide cannot fault, and the result for those divisors is
  `l * r` (wrapping), giving `x / 0 == 0` and `i64::MIN / -1 == i64::MIN`. Folds away for
  constant divisors. Audit: no other trapping Cranelift instruction (`srem`, `udiv`,
  `urem`, `fcvt_to_*int`, `trap*`) is emitted, because ir-lang 1.0 has no op that
  lowers to one; a unit test enforces it.
- **M55.** Windows ARM64 flushes the instruction cache with kernel32
  `FlushInstructionCache` (raw FFI, no new dependency) instead of `__clear_cache`,
  which MSVC lacks. Cross-checked with `cargo check --target aarch64-pc-windows-msvc`
  on stable and 1.94; not executed on Windows ARM64 hardware.
- **M56.** `bool` parameters and returns are declared `uext`; `Compiled::entry::<F>`'s
  pointer-size check is a compile-time `const { assert!(..) }` in every profile.

Semantics decision (recorded per the anti-deferral rule): the frozen `1.x` calling
convention (`extern "C" fn(..) -> T`) has no runtime-error channel, so the `OPS` defaults
(`div_zero = error`, `overflow = error`, `_lexersketch/specs/OPS.md`) cannot be honored in
`1.x`. jit-lang 1.x matches the bytecode tier instead (wrapping integer arithmetic,
`MIN / -1 == MIN` as `wrapping_div`) and defines `x / 0 == 0`, which neither ir-lang 1.0
nor the bytecode tier defines (ISSUES M44). Erroring semantics move to jit-lang 2.0.

Dependency wiring: unchanged (`ir-lang = "1"`, `pager-lang = "1"`, Cranelift 0.133).
The Cranelift dependency itself (ISSUES M57) is not a patch-level change: it is removed in
**Phase 5** of the LexerSketch roadmap (step 5.6, jit-lang 2.0: our native backend into
pager-lang, a code arena instead of one guarded region per function, and an error return
for OPS-conformant division and overflow).
