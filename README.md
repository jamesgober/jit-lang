<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>jit-lang</b>
    <br>
    <sub><sup>JIT COMPILER</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/jit-lang"><img alt="Crates.io" src="https://img.shields.io/crates/v/jit-lang"></a>
    <a href="https://crates.io/crates/jit-lang"><img alt="Downloads" src="https://img.shields.io/crates/d/jit-lang?color=%230099ff"></a>
    <a href="https://docs.rs/jit-lang"><img alt="docs.rs" src="https://img.shields.io/docsrs/jit-lang"></a>
    <a href="https://github.com/jamesgober/jit-lang/actions"><img alt="CI" src="https://github.com/jamesgober/jit-lang/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.94%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        jit-lang is the CODE-tier crate: it lowers the <a href="https://docs.rs/ir-lang"><code>ir-lang</code></a> intermediate representation to native machine code via <a href="https://cranelift.dev">Cranelift</a>, places that code in executable memory, and hands back a callable function — so a front-end can run what it compiled, immediately. Part of the -lang language-construction family; see _strategy/LANG_COLLECTION.md for the master plan.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.94+</strong> (Rust 2024 edition). The native code generator sets this floor; the rest of the family targets 1.85.
    </p>
    <blockquote>
        <strong>Status: stable.</strong> The public API is frozen as of <code>1.0.0</code> and follows Semantic Versioning, with no breaking changes before <code>2.0</code>. See <a href="./docs/API.md#semver-promise"><code>docs/API.md</code></a> for the SemVer promise and <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a>.
    </blockquote>
</div>

<hr>
<br>

## Overview

A compiler's middle end produces a function in [SSA form](https://en.wikipedia.org/wiki/Static_single-assignment_form). The usual back end writes an object file; a JIT instead generates machine code in memory and jumps into it. jit-lang is that last step for the family: it takes an [`ir_lang::Function`](https://docs.rs/ir-lang), compiles it for the machine it is running on, and returns a handle you can call.

The surface is three types and a function. [`Jit`](./docs/API.md#jit) is the engine; [`Jit::compile`](./docs/API.md#jitcompile) turns a function into a [`Compiled`](./docs/API.md#compiled); [`Compiled::entry`](./docs/API.md#compiledentry) reinterprets the compiled code as a function pointer you can call. The free function [`compile`](./docs/API.md#compile) does all of that in one step for the one-off case. Failures are reported through [`JitError`](./docs/API.md#jiterror).

Under the hood, a validated function is translated to [Cranelift](https://cranelift.dev) IR — an almost one-to-one mapping, since both are SSA control-flow graphs whose values cross blocks as block parameters rather than through phi nodes — Cranelift generates optimized native code, and the bytes are copied into a guard-flanked, read-execute region from [`pager-lang`](https://docs.rs/pager-lang).

<br>
<hr>
<br>

## Installation

```toml
[dependencies]
jit-lang = "1"
ir-lang = "1"
```

Or from the terminal:

```bash
cargo add jit-lang ir-lang
```

You build the IR with `ir-lang`'s `Builder` and hand the result to jit-lang, so both crates appear in your dependencies.

<br>

## Quick Start

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// Build `fn add(a: int, b: int) -> int { a + b }` with the IR builder.
let mut b = Builder::new("add", &[Type::Int, Type::Int], Type::Int);
let a = b.block_params(b.entry())[0];
let c = b.block_params(b.entry())[1];
let sum = b.bin(BinOp::Add, a, c);
b.ret(Some(sum));

// Compile it to native machine code.
let f = compile(&b.finish()).expect("add is well-formed");
assert_eq!(f.name(), "add");

// Reinterpret the entry point as a function pointer and call it.
// SAFETY: the signature is `fn(int, int) -> int`, and `f` outlives the call.
let add: extern "C" fn(i64, i64) -> i64 = unsafe { f.entry() };
assert_eq!(add(19, 23), 42);
```

<br>
<hr>
<br>

## How it works

A compile runs three stages:

1. **Validate.** The function is checked with [`Function::validate`](https://docs.rs/ir-lang), so only well-formed SSA is ever lowered. A malformed function is rejected with [`JitError::InvalidIr`](./docs/API.md#jiterror), not miscompiled.
2. **Translate and generate.** The IR is translated to Cranelift IR and Cranelift generates optimized machine code for the host, with its native instruction-set features enabled.
3. **Place.** The emitted bytes are copied into a guard-flanked memory region, which is then flipped from writable to read-execute — the write-xor-execute discipline. The functions compiled here are leaf functions with no outgoing calls, so the code is self-contained and needs no runtime relocation.

### Types and calling convention

The IR's four machine types map onto the host C ABI:

| IR type | ABI | In Rust |
|---|---|---|
| `int` | 64-bit integer | `i64` |
| `float` | IEEE-754 double | `f64` |
| `bool` | byte holding `0` or `1` | `bool` / `u8` |
| `unit` (return) | no return value | `()` |

A `unit`-typed **parameter** has no machine representation and is refused with [`JitError::Unsupported`](./docs/API.md#jiterror). The compiled function uses the host's C calling convention, which is exactly what Rust's `extern "C"` denotes, so the two agree when you call it.

### A branch and a loop

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn sum_to(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }
let mut b = Builder::new("sum_to", &[Type::Int], Type::Int);
let n0 = b.block_params(b.entry())[0];
let header = b.create_block(&[Type::Int, Type::Int]); // (n, acc)
let body = b.create_block(&[]);
let exit = b.create_block(&[]);

let zero = b.iconst(0);
b.jump(header, &[n0, zero]);

b.switch_to(header);
let n = b.block_params(header)[0];
let acc = b.block_params(header)[1];
let z = b.iconst(0);
let more = b.bin(BinOp::Gt, n, z);
b.branch(more, body, &[], exit, &[]);

b.switch_to(body);
let acc2 = b.bin(BinOp::Add, acc, n);
let one = b.iconst(1);
let n2 = b.bin(BinOp::Sub, n, one);
b.jump(header, &[n2, acc2]);

b.switch_to(exit);
b.ret(Some(acc));

let f = compile(&b.finish()).expect("sum_to is well-formed");
// SAFETY: the signature is `fn(int) -> int` and `f` outlives every call.
let sum_to: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
assert_eq!(sum_to(100), 5050);
```

<br>
<hr>
<br>

## API Overview

For a complete reference with examples, see [`docs/API.md`](./docs/API.md).

- [`Jit`](./docs/API.md#jit) — the engine; holds a code generator for the host and compiles many functions.
- [`Jit::new`](./docs/API.md#jitnew) / [`Jit::compile`](./docs/API.md#jitcompile) — build the engine, then compile a function to runnable code.
- [`compile`](./docs/API.md#compile) — the one-call shortcut: build an engine and compile a single function.
- [`Compiled`](./docs/API.md#compiled) — the result: native code in executable memory, with [`name`](./docs/API.md#compiled), [`params`](./docs/API.md#compiled), [`ret`](./docs/API.md#compiled), [`code_len`](./docs/API.md#compiled), [`as_ptr`](./docs/API.md#compiled), and the `unsafe` [`entry`](./docs/API.md#compiledentry).
- [`JitError`](./docs/API.md#jiterror) — the reason a compile failed: invalid IR, an unsupported feature or host, a code-generation failure, or executable memory being unavailable.

### Safety

Generating code and jumping into it cannot be checked by the compiler. The two obligations — the function-pointer type must match what was compiled, and the [`Compiled`](./docs/API.md#compiled) that owns the code must outlive every call — are concentrated in the single `unsafe` method [`Compiled::entry`](./docs/API.md#compiledentry). Building the IR, compiling it, and inspecting the result are all safe.

<br>
<hr>
<br>

## Performance

Performance is a hard constraint, not an afterthought (see [`REPS.md`](./REPS.md)). The translation to Cranelift IR is a single linear pass over the function; the heavy lifting — instruction selection, register allocation, optimization — is Cranelift's, run at its `speed` optimization level with the host's native CPU features enabled. A compiled function is plain native code: calling it is an indirect call, with no interpreter loop or dispatch in between.

Latest local Criterion means (`cargo bench --bench bench`, Linux x86_64 / WSL2, Rust stable, release build). `compile` covers validation, translation, code generation, and placing the code in executable memory:

| Work | Shape | Time |
|---|---|---:|
| `compile` | two-way branch (`max`) | ~27 µs |
| `compile` | loop with a back-edge (`sum_to`) | ~34 µs |
| `compile` | straight-line, 16 `add` steps | ~178 µs |
| `compile` | straight-line, 256 `add` steps | ~4.1 ms |
| call | invoke a compiled function | ~1.4 ns |

A call into compiled code is a plain indirect call — about a nanosecond, the cost of the jump and return. Compile time is Cranelift's, and grows with function size; the large straight-line case is register allocation over a single big block. Numbers vary by CPU and environment; run the benches on your hardware for trends.

<br>
<hr>
<br>

## Configuration

jit-lang has no feature flags: the JIT is the crate, and it links the standard library and reaches the operating system for executable memory, so it is not `no_std`.

Its dependencies are the engine it is built on:

| Dependency | Role |
|---|---|
| [`ir-lang`](https://docs.rs/ir-lang) | the IR a function is built in — the input to every compile |
| [`cranelift-codegen`](https://docs.rs/cranelift-codegen) / [`cranelift-frontend`](https://docs.rs/cranelift-frontend) / [`cranelift-native`](https://docs.rs/cranelift-native) | native code generation for the host |
| [`pager-lang`](https://docs.rs/pager-lang) | executable, page-aligned memory with write-xor-execute control |

<br>

## Examples

Three runnable examples live in [`examples/`](./examples):

```bash
# Build `add`, compile it, and call it.
cargo run --example jit_and_call

# Compile and run a branch (abs) and a loop (sum_to).
cargo run --example control_flow

# Compile and run floating-point and boolean-returning functions.
cargo run --example floats
```

<br>

## Testing

```bash
# Unit, integration (workflow, properties, error paths), and doc tests
cargo test

# Property tests only
cargo test --test properties

# Lints and formatting
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings

# Benchmarks (Criterion)
cargo bench --bench bench
```

The integration suite in [`tests/workflow.rs`](./tests/workflow.rs) compiles representative functions — `double`, `abs`, `max`, a countdown loop, float arithmetic, boolean logic, integer division, and a unit function — to native code and **runs** them, checking the results the CPU produced. The property tests in [`tests/properties.rs`](./tests/properties.rs) generate random functions, run the compiled code, and compare each result to an independent evaluation in Rust.

<br>

## Cross-Platform Support

Code generation targets the host through Cranelift, and executable memory is managed by pager-lang, so the supported targets are their intersection: **Linux**, **macOS**, and **Windows** on **x86-64** and **ARM64**. Freshly written code is made coherent before it runs — a no-op on x86-64, where the instruction and data caches are unified, and an instruction-cache synchronization on ARM64, where they are not. The CI matrix builds and tests on Linux, macOS, and Windows (macOS on ARM64, so the cache path is exercised) on stable and the 1.94 MSRV.

<br>

## Contributing

Engineering standards for this crate are the [Rust Efficiency &amp; Performance Standards](./REPS.md); the current scope and plan are in [`dev/ROADMAP.md`](./dev/ROADMAP.md). Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b> &mdash; <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober <me@jamesgober.com>.</strong></sup>
</div>
