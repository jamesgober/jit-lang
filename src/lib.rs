//! # jit_lang
//!
//! Lower IR to machine code in executable memory and run it now.
//!
//! jit-lang takes a function in the [`ir-lang`](ir_lang) intermediate representation,
//! compiles it to native machine code for the machine it is running on, places that
//! code in executable memory, and hands back a callable handle. It is the end of the
//! pipeline a front-end follows: parse, type-check, lower to IR, and then — instead of
//! writing an object file — run the function immediately.
//!
//! The surface is small. [`Jit`] is the engine; [`Jit::compile`] turns an
//! [`ir_lang::Function`] into a [`Compiled`]; and [`Compiled::entry`] reinterprets the
//! compiled code as a function pointer you can call. For a single compile, the free
//! function [`compile`] does the same in one step.
//!
//! ## How it works
//!
//! A compile runs three stages:
//!
//! 1. **Validate.** The function is checked with
//!    [`Function::validate`](ir_lang::Function::validate), so only well-formed SSA is
//!    ever lowered.
//! 2. **Translate and generate.** The IR is translated to [Cranelift](https://cranelift.dev)
//!    IR — an almost one-to-one mapping, since both are SSA control-flow graphs whose
//!    values cross blocks as block parameters — and Cranelift generates optimized
//!    machine code for the host.
//! 3. **Place.** The emitted bytes are copied into a guard-flanked memory region from
//!    [`pager-lang`](pager_lang), which is then flipped from writable to read-execute.
//!    The functions compiled here are leaf functions with no outgoing calls, so the
//!    code is self-contained and needs no runtime relocation.
//!
//! ## Type and calling convention
//!
//! The IR's four machine types map onto the host C ABI: `int` is a 64-bit integer,
//! `float` is an `f64`, `bool` is a byte holding `0` or `1`, and a `unit` return means
//! the function yields no value. A `unit`-typed *parameter* has no machine
//! representation and is refused. The compiled function uses the host's C calling
//! convention, which is what Rust's `extern "C"` denotes, so the two agree when you
//! call it.
//!
//! ## Example
//!
//! Compile `fn sum_to(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }`
//! — a loop with a back-edge carrying two values — and run it:
//!
//! ```
//! use jit_lang::compile;
//! use ir_lang::{Builder, BinOp, Type};
//!
//! let mut b = Builder::new("sum_to", &[Type::Int], Type::Int);
//! let n0 = b.block_params(b.entry())[0];
//! let header = b.create_block(&[Type::Int, Type::Int]); // (n, acc)
//! let body = b.create_block(&[]);
//! let exit = b.create_block(&[]);
//!
//! let zero = b.iconst(0);
//! b.jump(header, &[n0, zero]);
//!
//! b.switch_to(header);
//! let n = b.block_params(header)[0];
//! let acc = b.block_params(header)[1];
//! let z = b.iconst(0);
//! let more = b.bin(BinOp::Gt, n, z);
//! b.branch(more, body, &[], exit, &[]);
//!
//! b.switch_to(body);
//! let acc2 = b.bin(BinOp::Add, acc, n);
//! let one = b.iconst(1);
//! let n2 = b.bin(BinOp::Sub, n, one);
//! b.jump(header, &[n2, acc2]);
//!
//! b.switch_to(exit);
//! b.ret(Some(acc));
//!
//! let f = compile(&b.finish()).expect("sum_to is well-formed");
//!
//! // SAFETY: the signature is `fn(int) -> int` and `f` outlives every call below.
//! let sum_to: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
//! assert_eq!(sum_to(5), 15); // 5 + 4 + 3 + 2 + 1
//! assert_eq!(sum_to(0), 0);
//! ```
//!
//! ## Safety
//!
//! Generating code and jumping into it cannot be checked by the compiler: the type you
//! transmute the entry point to has to match what was compiled, and the [`Compiled`]
//! that owns the code has to outlive every call. Those obligations are concentrated in
//! the single `unsafe` method [`Compiled::entry`], whose contract spells them out.
//! Everything up to that point — building the IR, compiling it, inspecting the result
//! — is safe.
//!
//! ## Platforms
//!
//! Code generation targets the host through Cranelift, and executable memory is managed
//! by pager-lang, so the supported targets are their intersection: Linux, macOS, and
//! Windows on x86-64 and ARM64. Freshly written code is made coherent before it runs —
//! nothing to do on x86-64, where the caches are unified, and an instruction-cache
//! synchronization on ARM64, where they are not. The crate links the standard library
//! and reaches the operating system for memory; it is not `no_std`.
//!
//! ## Stability
//!
//! The public surface is frozen and stable as of `1.0.0`: it follows Semantic
//! Versioning, with no breaking changes before `2.0`. [`JitError`] is
//! `#[non_exhaustive]`, so a new failure variant is an additive, non-breaking change.
//! The full surface and the SemVer promise are catalogued in
//! [`docs/API.md`](https://github.com/jamesgober/jit-lang/blob/main/docs/API.md#semver-promise).

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr
)]

mod compiled;
mod engine;
mod error;
mod icache;
mod translate;

pub use compiled::Compiled;
pub use engine::{Jit, compile};
pub use error::JitError;
