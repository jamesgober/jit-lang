<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>jit-lang</b><br>
    <sub><sup>API REFERENCE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>API</span>
        <span>&nbsp;│&nbsp;</span>
        <a href="../dev/ROADMAP.md" title="Roadmap"><b>ROADMAP</b></a>
    </sup>
</div>
<br>

> **Status: stable (1.0).** The surface below is the `1.0` contract: it follows [Semantic Versioning](#semver-promise) and will not change in a breaking way before `2.0`. See [`dev/ROADMAP.md`](../dev/ROADMAP.md).

jit-lang lowers the [`ir-lang`](https://docs.rs/ir-lang) intermediate representation to native machine code, places it in executable memory, and returns a callable handle. It is the run-it-now end of the language-construction pipeline.

<br>

## Table of Contents

- [Installation](#installation)
- [How it works](#how-it-works)
  - [Type and ABI mapping](#type-and-abi-mapping)
- [Public API](#public-api)
  - [`Jit`](#jit)
  - [`Jit::new`](#jitnew)
  - [`Jit::compile`](#jitcompile)
  - [`compile`](#compile)
  - [`Compiled`](#compiled)
  - [`Compiled::entry`](#compiledentry)
  - [`JitError`](#jiterror)
- [Feature flags](#feature-flags)
- [SemVer promise](#semver-promise)

<br>
<hr>
<br>

## Installation

```toml
[dependencies]
jit-lang = "1"
ir-lang = "1"
```

```bash
cargo add jit-lang ir-lang
```

You build the function with `ir-lang`'s `Builder` and hand the result to jit-lang, so both crates are listed. The crate links the standard library and reaches the operating system for executable memory; it is not `no_std`.

<br>
<hr>
<br>

## How it works

A compile is three stages:

1. **Validate.** The function is checked with [`Function::validate`](https://docs.rs/ir-lang). Only well-formed SSA is lowered; a malformed function is rejected with [`JitError::InvalidIr`](#jiterror).
2. **Translate and generate.** The IR is translated to [Cranelift](https://cranelift.dev) IR — close to a relabeling, since both are SSA control-flow graphs whose values cross blocks as block parameters — and Cranelift generates optimized machine code for the host.
3. **Place.** The emitted bytes are copied into a guard-flanked region from [`pager-lang`](https://docs.rs/pager-lang), the region is flipped from writable to read-execute, and the instruction cache is synchronized over the new code so it is safe to run — nothing to do on x86-64, an instruction-cache flush on ARM64. The functions compiled here are leaf functions with no outgoing calls, so the code is self-contained and needs no runtime relocation; one that somehow did is refused rather than run wrong.

### Type and ABI mapping

The IR's machine types map onto the host C ABI. The compiled function uses the host C calling convention, which is what Rust's `extern "C"` denotes, so a matching `extern "C"` function pointer calls it correctly.

| IR type | Machine ABI | Rust function-pointer type |
|---|---|---|
| `int` | 64-bit integer | `i64` |
| `float` | IEEE-754 double | `f64` |
| `bool` | a byte that is `0` or `1` | `bool` or `u8` |
| `unit` (as a return type) | no return value | `()` |

A `unit`-typed **parameter** has no machine representation and is refused with [`JitError::Unsupported`](#jiterror). Integer division by zero traps in hardware, as it does on the CPU directly; the compiled code does not guard it.

<br>
<hr>
<br>

## Public API

### `Jit`

```rust
pub struct Jit { /* private fields */ }
```

A just-in-time compiler for the host machine. It holds a code generator configured for the CPU it is running on — built once — and compiles many functions; it carries no per-function state. `Jit` is [`Send`] and [`Sync`], so one engine can be shared across threads.

Build it with [`Jit::new`](#jitnew) and compile with [`Jit::compile`](#jitcompile). For a single compile where reuse does not matter, the free [`compile`](#compile) function does both in one call.

**Examples**

Build the engine once and compile two functions:

```rust
use jit_lang::Jit;
use ir_lang::{Builder, BinOp, Type};

let jit = Jit::new().expect("the host is supported");

let mut b = Builder::new("inc", &[Type::Int], Type::Int);
let x = b.block_params(b.entry())[0];
let one = b.iconst(1);
let sum = b.bin(BinOp::Add, x, one);
b.ret(Some(sum));
let inc = jit.compile(&b.finish()).expect("inc is well-formed");

assert_eq!(inc.name(), "inc");
```

<br>

### `Jit::new`

```rust
pub fn new() -> Result<Jit, JitError>
```

Builds an engine targeting the host CPU, with its native instruction-set features enabled and the optimizing code path selected.

**Returns**

- `Ok(Jit)` — an engine ready to compile.
- `Err(JitError::Unsupported(_))` — the host architecture has no Cranelift back-end.
- `Err(JitError::Codegen(_))` — the code generator rejected the configuration; not expected on a supported host.

**Examples**

```rust
use jit_lang::Jit;

let jit = Jit::new().expect("the host is supported");
let _ = jit; // ready to compile
```

<br>

### `Jit::compile`

```rust
pub fn compile(&self, func: &ir_lang::Function) -> Result<Compiled, JitError>
```

Compiles a function to native machine code and makes it runnable. The function is validated, translated to Cranelift IR, generated for the host, and copied into a guard-flanked, read-execute region that the returned [`Compiled`](#compiled) owns.

**Parameters**

- `func` — the function to compile, in SSA form, as produced by `ir_lang::Builder`. It is validated before anything else.

**Returns**

- `Ok(Compiled)` — the runnable [`Compiled`](#compiled).
- `Err(JitError::InvalidIr(_))` — `func` did not pass `Function::validate`.
- `Err(JitError::Unsupported(_))` — `func` is valid but uses something the back-end cannot lower, such as a `unit`-typed parameter.
- `Err(JitError::Codegen(_))` — the code generator failed on otherwise valid input.
- `Err(JitError::Memory(_))` — executable memory could not be obtained or protected.

**Examples**

Compile and run a two-argument function:

```rust
use jit_lang::Jit;
use ir_lang::{Builder, BinOp, Type};

// fn mul(a: int, b: int) -> int { a * b }
let mut b = Builder::new("mul", &[Type::Int, Type::Int], Type::Int);
let a = b.block_params(b.entry())[0];
let c = b.block_params(b.entry())[1];
let p = b.bin(BinOp::Mul, a, c);
b.ret(Some(p));

let jit = Jit::new().unwrap();
let mul = jit.compile(&b.finish()).expect("mul is well-formed");

// SAFETY: the signature is `fn(int, int) -> int`, and `mul` outlives the call.
let mul_fn: extern "C" fn(i64, i64) -> i64 = unsafe { mul.entry() };
assert_eq!(mul_fn(6, 7), 42);
```

Reuse one engine across many functions:

```rust
use jit_lang::Jit;
use ir_lang::{Builder, BinOp, Type};

let jit = Jit::new().unwrap();
for k in 0..4_i64 {
    let mut b = Builder::new("add_k", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let c = b.iconst(k);
    let sum = b.bin(BinOp::Add, x, c);
    b.ret(Some(sum));
    let f = jit.compile(&b.finish()).unwrap();
    // SAFETY: the signature is `fn(int) -> int`, and `f` outlives the call.
    let add_k: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(add_k(100), 100 + k);
}
```

<br>

### `compile`

```rust
pub fn compile(func: &ir_lang::Function) -> Result<Compiled, JitError>
```

Compiles a function with a fresh host engine, for the one-off case. It builds a [`Jit`](#jit), compiles, and returns the runnable [`Compiled`](#compiled). When compiling several functions, build one `Jit` with [`Jit::new`](#jitnew) and reuse it — that creates the host code generator once instead of per call.

**Parameters**

- `func` — the function to compile, as for [`Jit::compile`](#jitcompile).

**Returns**

The same as [`Jit::new`](#jitnew) followed by [`Jit::compile`](#jitcompile).

**Examples**

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn square(x: int) -> int { x * x }
let mut b = Builder::new("square", &[Type::Int], Type::Int);
let x = b.block_params(b.entry())[0];
let sq = b.bin(BinOp::Mul, x, x);
b.ret(Some(sq));

let square = compile(&b.finish()).expect("square is well-formed");
// SAFETY: the signature is `fn(int) -> int`, and `square` outlives the call.
let square_fn: extern "C" fn(i64) -> i64 = unsafe { square.entry() };
assert_eq!(square_fn(7), 49);
```

Handle a rejection:

```rust
use jit_lang::{compile, JitError};
use ir_lang::{Builder, Type};

// Declares an int return but returns nothing.
let mut b = Builder::new("bad", &[], Type::Int);
b.ret(None);
assert!(matches!(compile(&b.finish()), Err(JitError::InvalidIr(_))));
```

<br>

### `Compiled`

```rust
pub struct Compiled { /* private fields */ }
```

A compiled function: native machine code in executable memory, ready to run. `Compiled` owns the memory region, so the code stays mapped and runnable for exactly as long as the value is alive, and is freed on drop — calling a function whose `Compiled` has been dropped is a use-after-free. `Compiled` is [`Send`] and [`Sync`].

The signature accessors report the function as it was compiled, so a caller can pick the right function-pointer type before calling.

**Methods**

| Method | Returns | Description |
|---|---|---|
| `name()` | `&str` | The compiled function's name. |
| `params()` | `&[ir_lang::Type]` | The parameter types, in declaration order — the argument types of the matching function pointer. |
| `ret()` | `ir_lang::Type` | The return type. A `unit` return means the function yields no value. |
| `code_len()` | `usize` | The length of the emitted machine code in bytes (at most the region length, which is rounded up to whole pages). |
| `as_ptr()` | `*const u8` | A raw pointer to the entry point. Valid to obtain; running through it is `unsafe`. |
| `entry::<F>()` | `F` | **`unsafe`.** Reinterprets the entry point as a function pointer of type `F`. See [below](#compiledentry). |

**Examples**

Compile a function and read back what it is:

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn double(x: int) -> int { x + x }
let mut b = Builder::new("double", &[Type::Int], Type::Int);
let x = b.block_params(b.entry())[0];
let sum = b.bin(BinOp::Add, x, x);
b.ret(Some(sum));

let f = compile(&b.finish()).expect("double is well-formed");
assert_eq!(f.name(), "double");
assert_eq!(f.params(), &[Type::Int]);
assert_eq!(f.ret(), Type::Int);
assert!(f.code_len() > 0);
assert!(!f.as_ptr().is_null());
```

<br>

### `Compiled::entry`

```rust
pub unsafe fn entry<F: Copy>(&self) -> F
```

Reinterprets the entry point as a function pointer of type `F` so the compiled code can be called. This is how you run a compiled function: pick an `extern "C"` function-pointer type whose signature matches what was compiled, get it from `entry`, and call it. See [Type and ABI mapping](#type-and-abi-mapping) for how IR types map to the C ABI.

**Type parameter**

- `F` — the function-pointer type to read the entry address as, for example `extern "C" fn(i64) -> i64`.

**Safety**

The caller guarantees all of:

- `F` is a function-pointer type, hence pointer-sized. A non-function or differently-sized type is undefined behavior. (In debug builds a size mismatch is caught by an assertion.)
- `F`'s signature matches the compiled function: one `extern "C"` argument per [`params`](#compiled) entry with the ABI type from the mapping table, and a return that matches [`ret`](#compiled). A mismatch is undefined behavior.
- Every call through the returned pointer happens while `self` is alive. Once `self` is dropped the code is unmapped and the pointer dangles.

**Examples**

Call a function that returns an `int`:

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

let mut b = Builder::new("double", &[Type::Int], Type::Int);
let x = b.block_params(b.entry())[0];
let sum = b.bin(BinOp::Add, x, x);
b.ret(Some(sum));
let f = compile(&b.finish()).unwrap();

// SAFETY: the signature matches `fn double(x: int) -> int`, and `f` outlives the call.
let double: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
assert_eq!(double(21), 42);
assert_eq!(double(-5), -10);
```

Call one that returns a `bool`:

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn lt(a: int, b: int) -> bool { a < b }
let mut b = Builder::new("lt", &[Type::Int, Type::Int], Type::Bool);
let a = b.block_params(b.entry())[0];
let c = b.block_params(b.entry())[1];
let lt = b.bin(BinOp::Lt, a, c);
b.ret(Some(lt));
let f = compile(&b.finish()).unwrap();

// SAFETY: a `bool` return is a 0/1 byte; `f` outlives every call.
let lt: extern "C" fn(i64, i64) -> bool = unsafe { f.entry() };
assert!(lt(1, 2));
assert!(!lt(2, 1));
```

Call one over `float`s:

```rust
use jit_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn add(a: float, b: float) -> float { a + b }
let mut b = Builder::new("add", &[Type::Float, Type::Float], Type::Float);
let a = b.block_params(b.entry())[0];
let c = b.block_params(b.entry())[1];
let sum = b.bin(BinOp::Add, a, c);
b.ret(Some(sum));
let f = compile(&b.finish()).unwrap();

// SAFETY: the signature is `fn(float, float) -> float`, and `f` outlives the call.
let add: extern "C" fn(f64, f64) -> f64 = unsafe { f.entry() };
assert_eq!(add(1.5, 2.5), 4.0);
```

<br>

### `JitError`

```rust
#[non_exhaustive]
pub enum JitError {
    InvalidIr(ir_lang::ValidationError),
    Unsupported(&'static str),
    Codegen(String),
    Memory(pager_lang::PagerError),
}
```

The reason a function could not be compiled and made executable. Each stage of a compile contributes a variant. `JitError` implements `Display` and `std::error::Error` (with `source` set to the wrapped error where there is one), and converts from `ir_lang::ValidationError` and `pager_lang::PagerError` with `From`, so `?` propagates them. The enum is `#[non_exhaustive]`, so a `match` on it must keep a wildcard arm.

**Variants**

| Variant | Meaning | What to do |
|---|---|---|
| `InvalidIr(ValidationError)` | The function did not pass `Function::validate`. | Fix the IR; the wrapped error names the offending block or value. |
| `Unsupported(&'static str)` | A valid function uses something the back-end cannot lower (e.g. a `unit` parameter), or the host has no code generator. | Read the message; adjust the function or run on a supported target. |
| `Codegen(String)` | The native code generator failed on input the translator accepted. | Not expected; the message carries the generator's own report. |
| `Memory(PagerError)` | Executable memory could not be obtained or its protection changed. | The wrapped error says whether the mapping or the protection change failed. |

**Examples**

Match the reason a compile failed:

```rust
use jit_lang::{compile, JitError};
use ir_lang::{Builder, Type};

// A function whose entry block never gets a terminator is not well-formed.
let func = Builder::new("f", &[], Type::Unit).finish();
match compile(&func) {
    Err(JitError::InvalidIr(reason)) => assert!(reason.to_string().contains("terminator")),
    other => panic!("expected InvalidIr, got {other:?}"),
}
```

Recognize an unsupported feature:

```rust
use jit_lang::{compile, JitError};
use ir_lang::{Builder, Type};

// A unit-typed parameter has no value at the machine level.
let mut b = Builder::new("f", &[Type::Unit], Type::Unit);
b.ret(None);
match compile(&b.finish()) {
    Err(JitError::Unsupported(msg)) => assert!(msg.contains("unit")),
    other => panic!("expected Unsupported, got {other:?}"),
}
```

Use it as a `std::error::Error`:

```rust
use jit_lang::compile;
use ir_lang::{Builder, Type};
use std::error::Error;

let func = Builder::new("f", &[], Type::Int).finish(); // missing return value
let err = compile(&func).unwrap_err();
assert!(err.source().is_some());
```

<br>
<hr>
<br>

## Feature flags

jit-lang has no feature flags. The JIT is the crate: it always links the standard library and always depends on its code generator and executable-memory layers. There is no `no_std` build, because generating and running code needs an operating system.

<br>

## SemVer promise

As of `1.0.0` the public surface above is frozen. The crate follows [Semantic Versioning](https://semver.org):

- No documented item is removed or changed in a breaking way within `1.x`; breaking changes wait for `2.0`.
- New functionality is additive and arrives in minor releases. [`JitError`](#jiterror) is `#[non_exhaustive]`, so a new failure variant is a minor change, not a breaking one; a `match` on it must keep a wildcard arm.
- The MSRV is Rust `1.94`, the floor the code generator imposes; raising it is a minor change, never a patch.
- Behaviour is part of the contract: a function that compiles today keeps compiling, and a compiled function computes the same result on the same inputs. The set of supported host architectures may grow, never shrink.

This file is updated in lockstep with every release so it always matches the code.

<br>
<hr>

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>
