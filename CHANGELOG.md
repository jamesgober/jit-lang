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

[Unreleased]: https://github.com/jamesgober/jit-lang/compare/v1.0.0...HEAD
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
