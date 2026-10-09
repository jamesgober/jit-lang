<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>jit-lang</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

### Added

### Changed

### Fixed

### Security

---

## [1.0.1] - 2026-10-08

A correctness patch: compiled integer division no longer kills the host process, the Windows ARM64 instruction-cache path now links, and `bool` crosses the ABI zero-extended. No public API change; a `cargo update` picks it up.

### Fixed

- **Integer division trapped in hardware and killed the process (ISSUES H10).** `BinOp::Div` on `int` lowered to a bare Cranelift `sdiv`, which traps on a zero divisor and on `i64::MIN / -1`: Cranelift's explicit zero check (`ud2`) raises SIGILL / `STATUS_ILLEGAL_INSTRUCTION`, and x86-64 `idiv` overflow raises SIGFPE / `STATUS_INTEGER_OVERFLOW` (on AArch64, whose `sdiv` does not fault, Cranelift emits explicit trap checks for both). Any caller passing those inputs crashed, and so did the IR it came from, although `ir-lang` validates it. Division is now total and never faults: `x / 0 == 0`, and `i64::MIN / -1 == i64::MIN` (two's-complement wrap, exactly `i64::wrapping_div`). Every other divisor is the same truncating division as before. The guard is branch-free (one range compare, a `select` of a safe divisor, and a `select` of `l * r` for the two special divisors), costs well under a nanosecond per call on x86-64, and folds away entirely for a constant divisor.
  - **Why these values.** The `1.x` calling convention (`extern "C" fn(..) -> T`) has no channel for a runtime error, so neither case can be an error here. `MIN / -1 == MIN` matches the bytecode tier (codegen-lang's reference interpreter uses `wrapping_div`) and the wrap jit-lang already applies to `add`/`sub`/`mul`/`neg`. Division by zero is defined by neither ir-lang 1.0 nor the bytecode tier; `0` is the result AArch64's `sdiv` gives, and is now documented. The LexerSketch `OPS` defaults (`div_zero = error`, `overflow = error`) need an error return, which arrives with the native backend that replaces Cranelift (Phase 5), not in a `1.x` patch.
  - Audited the other trapping Cranelift instructions (`srem`, `udiv`, `urem`, `fcvt_to_sint`, `fcvt_to_uint`, `trap*`, `uadd_overflow_trap`): ir-lang 1.0 has no op that lowers to any of them, and a unit test now fails if the translator ever emits a trapping instruction other than the guarded `sdiv`.
  - `docs/API.md` said "Integer division by zero traps in hardware … the compiled code does not guard it"; it now documents the defined results under [Integer arithmetic](./docs/API.md#integer-arithmetic).
- **Windows ARM64 referenced a symbol MSVC does not provide (ISSUES M55).** The AArch64 instruction-cache flush called the compiler-runtime `__clear_cache` on every non-macOS target, but MSVC's runtime has no such symbol, so a Windows ARM64 build could not link. Windows now calls `FlushInstructionCache(GetCurrentProcess(), ..)` from kernel32 through raw FFI (no new dependency), the same way pager-lang reaches `VirtualAlloc`. Verified with `cargo check --target aarch64-pc-windows-msvc` (stable and 1.94); not executed on Windows ARM64 hardware.
- **`bool` parameters and returns were not declared zero-extended (ISSUES M56).** They were plain `I8` ABI slots, so the upper bits of a returned `bool` register were unspecified, while Rust's `bool` and C's `_Bool` are `zeroext`. They are now declared `uext`, so a `bool` return is a clean `0`/`1` in the full register and incoming `bool` arguments are treated as the zero-extended bytes callers pass.
- **`Compiled::entry::<F>` checked `size_of::<F>()` only in debug builds (ISSUES M56).** The pointer-size check is now a compile-time assertion (`const { assert!(..) }`), evaluated when `entry::<F>` is instantiated, in every profile. Calling `entry` with a type that is not pointer-sized was already undefined behavior under the documented contract; it now fails to compile instead.

### Added

- `tests/division.rs`: divide-by-zero, `MIN / -1`, `MIN rem -1` (spelled `a - b * (a / b)`, since ir-lang 1.0 has no remainder op), every pair of nine edge values, constant and fully constant divisors, division inside a loop, float division by zero, and three property tests (single divide, composed remainder, random add/sub/mul/div/neg chains biased toward edge values). Every case runs natively in the test process and is compared with a reference interpreter (`tests/support`) replicated from codegen-lang's bytecode-tier oracle. Run against the 1.0.0 lowering, the test binary dies with `STATUS_INTEGER_OVERFLOW` / `STATUS_ILLEGAL_INSTRUCTION`.
- Unit tests: the translated signature marks `bool` slots `uext`; no trapping instruction except the guarded `sdiv` is emitted for a function using every op. A workflow test passes `bool` arguments and reads a `bool` return. A `compile_fail` doctest shows `entry::<[u64; 2]>()` rejected.
- Benchmark `call_guarded_div` (a runtime-divisor division per call).

---

## [1.0.0] - 2026-06-30

API freeze. The public surface is stable and frozen until `2.0`; the crate follows Semantic Versioning from this release. No breaking change to the `0.2.0` surface.

### Added

- Instruction-cache coherence for freshly compiled code. `place` now synchronizes the instruction cache over the emitted bytes before returning — a no-op on x86-64, where the instruction and data caches are unified, and an instruction-cache flush on ARM64 (`sys_icache_invalidate` on macOS, the compiler runtime's `__clear_cache` elsewhere) — so a compiled function runs correctly on aarch64, not only x86-64.
- Stability documentation: the crate root, `README`, and `docs/API.md` state the frozen `1.0` surface and the [SemVer promise](./docs/API.md#semver-promise).

### Changed

- The public API — [`Jit`], [`Compiled`], [`JitError`], and [`compile`] — is frozen as of `1.0.0` and follows Semantic Versioning, with no breaking changes before `2.0`.

---

## [0.2.0] - 2026-06-30

The core release: jit-lang now compiles an `ir-lang` function to native machine code and runs it. This is the hard part of the roadmap, not deferred.

### Added

- The JIT engine. [`Jit`] holds a code generator for the host; [`Jit::new`] builds it once and [`Jit::compile`] lowers an [`ir_lang::Function`] to native machine code, placing the result in an executable, guard-flanked memory region. The free function [`compile`] does both in one call for the one-off case.
- [`Compiled`], the runnable result: `name`, `params`, `ret`, and `code_len` report the function as it was compiled; `as_ptr` exposes the entry point; and the `unsafe` `entry::<F>()` reinterprets it as an `extern "C"` function pointer to call.
- [`JitError`] with `InvalidIr`, `Unsupported`, `Codegen`, and `Memory` variants, `Display` and `std::error::Error` implementations, and `From` conversions for `?`. The enum is `#[non_exhaustive]`.
- Dependencies wired as first used: `ir-lang` (the input IR), `pager-lang` (executable memory), and `cranelift-codegen` / `cranelift-frontend` / `cranelift-native` (native code generation).
- Test suite: workflow tests that compile and run `double`, `abs`, `max`, a countdown loop, float arithmetic, boolean logic, integer division, and a unit function; property tests comparing compiled output to an independent evaluation; error-path tests; and per-module unit tests.
- Three runnable examples (`jit_and_call`, `control_flow`, `floats`) and a Criterion benchmark suite for compile time and call cost.
- `docs/API.md` documenting the full public surface with worked examples.

### Changed

- MSRV raised to 1.94 (from the family's 1.85), the floor required by `cranelift-codegen` 0.133. Recorded in `Cargo.toml`, `clippy.toml`, the CI matrix, and the README badge.

### Fixed

- `Cargo.toml` `keywords` and `categories` were unquoted TOML arrays that failed to parse.
- `clippy.toml` declared MSRV `1.87`, disagreeing with the manifest's `1.85`; both now read `1.94`.
- `deny.toml` carried a stray `rate-net` reference left from the scaffold template.

---

## [0.1.0] - 2026-06-18

Initial scaffold and repository bootstrap. No domain logic yet &mdash; this release establishes the structure, tooling, and quality gates the implementation will be built on.

### Added

- `Cargo.toml` with crate metadata, Rust 2024 edition, MSRV 1.85.
- Dual `Apache-2.0 OR MIT` license files.
- `README.md`, `CHANGELOG.md`, and a documentation skeleton.
- `REPS.md` compliance baseline.
- `.github/workflows/ci.yml` CI matrix; `deny.toml`, `clippy.toml`, `rustfmt.toml`.
- `dev/DIRECTIVES.md` and `dev/ROADMAP.md` (committed engineering standards + plan).

[Unreleased]: https://github.com/jamesgober/jit-lang/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/jamesgober/jit-lang/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/jamesgober/jit-lang/compare/v0.2.0...v1.0.0
[0.2.0]: https://github.com/jamesgober/jit-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/jit-lang/releases/tag/v0.1.0

[`Jit`]: ./docs/API.md#jit
[`Jit::new`]: ./docs/API.md#jitnew
[`Jit::compile`]: ./docs/API.md#jitcompile
[`compile`]: ./docs/API.md#compile
[`Compiled`]: ./docs/API.md#compiled
[`JitError`]: ./docs/API.md#jiterror
[`ir_lang::Function`]: https://docs.rs/ir-lang
