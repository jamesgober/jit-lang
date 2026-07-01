//! The [`Jit`] engine: it owns a code generator for the host and turns IR functions
//! into runnable [`Compiled`] code.

use std::fmt;

use cranelift_codegen::Context;
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use ir_lang::Function;
use pager_lang::{Protection, Region};

use crate::compiled::Compiled;
use crate::error::JitError;
use crate::{icache, translate};

/// A just-in-time compiler for the host machine.
///
/// A `Jit` holds a code generator configured for the CPU it is running on, built once
/// in [`new`](Self::new). Each call to [`compile`](Self::compile) validates an
/// [`ir_lang::Function`], lowers it to native machine code, and places that code in
/// executable memory, returning a [`Compiled`] that can be called. The engine carries
/// no per-function state, so one `Jit` compiles many functions, and it is [`Send`] and
/// [`Sync`], so it can be shared across threads.
///
/// For a one-off compile where reuse does not matter, the free function
/// [`compile`](crate::compile) builds a `Jit` and compiles in a single call.
///
/// # Examples
///
/// Build the engine once, compile two functions, and run them:
///
/// ```
/// use jit_lang::Jit;
/// use ir_lang::{Builder, BinOp, Type};
///
/// let jit = Jit::new().expect("the host is supported");
///
/// // fn inc(x: int) -> int { x + 1 }
/// let mut b = Builder::new("inc", &[Type::Int], Type::Int);
/// let x = b.block_params(b.entry())[0];
/// let one = b.iconst(1);
/// let sum = b.bin(BinOp::Add, x, one);
/// b.ret(Some(sum));
/// let inc = jit.compile(&b.finish()).expect("inc is well-formed");
///
/// // fn neg(x: int) -> int { 0 - x }
/// let mut b = Builder::new("neg", &[Type::Int], Type::Int);
/// let x = b.block_params(b.entry())[0];
/// let zero = b.iconst(0);
/// let r = b.bin(BinOp::Sub, zero, x);
/// b.ret(Some(r));
/// let neg = jit.compile(&b.finish()).expect("neg is well-formed");
///
/// // SAFETY: both signatures are `fn(int) -> int`, and each handle outlives its call.
/// let inc_fn: extern "C" fn(i64) -> i64 = unsafe { inc.entry() };
/// let neg_fn: extern "C" fn(i64) -> i64 = unsafe { neg.entry() };
/// assert_eq!(inc_fn(41), 42);
/// assert_eq!(neg_fn(42), -42);
/// ```
pub struct Jit {
    /// The host code generator. Reference-counted internally, so cloning the engine's
    /// target is cheap; it is built once and reused for every compile.
    isa: OwnedTargetIsa,
}

impl Jit {
    /// Builds an engine targeting the host CPU, with its native instruction-set
    /// features enabled and the optimizing code path selected.
    ///
    /// # Errors
    ///
    /// - [`JitError::Unsupported`] if the host architecture has no Cranelift back-end.
    /// - [`JitError::Codegen`] if the code generator rejects the configuration — not
    ///   expected on a supported host.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::Jit;
    ///
    /// let jit = Jit::new().expect("the host is supported");
    /// let _ = jit; // ready to compile
    /// ```
    pub fn new() -> Result<Self, JitError> {
        let mut flags = settings::builder();
        // Optimize for speed; we run the code we generate, so its quality is the point.
        set_flag(&mut flags, "opt_level", "speed")?;
        // The code runs at the fixed address of its own region, so position-independent
        // addressing buys nothing and only adds indirection.
        set_flag(&mut flags, "is_pic", "false")?;
        let flags = settings::Flags::new(flags);

        let isa = cranelift_native::builder()
            .map_err(JitError::Unsupported)?
            .finish(flags)
            .map_err(|err| JitError::Codegen(err.to_string()))?;
        Ok(Self { isa })
    }

    /// Compiles a function to native machine code and makes it runnable.
    ///
    /// The function is validated, translated to Cranelift IR, compiled for the host,
    /// and copied into a guard-flanked, read-execute memory region that the returned
    /// [`Compiled`] owns. The emitted code is self-contained — a leaf function with no
    /// outgoing calls — so it needs no runtime relocation; a function that somehow
    /// required one is refused rather than run wrong.
    ///
    /// # Errors
    ///
    /// - [`JitError::InvalidIr`] if the function is not well-formed SSA.
    /// - [`JitError::Unsupported`] if it uses something this back-end cannot lower, such
    ///   as a [`Unit`](ir_lang::Type::Unit)-typed parameter.
    /// - [`JitError::Codegen`] if the code generator fails on otherwise valid input.
    /// - [`JitError::Memory`] if executable memory cannot be obtained or protected.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::Jit;
    /// use ir_lang::{Builder, BinOp, Type};
    ///
    /// // fn mul(a: int, b: int) -> int { a * b }
    /// let mut b = Builder::new("mul", &[Type::Int, Type::Int], Type::Int);
    /// let a = b.block_params(b.entry())[0];
    /// let c = b.block_params(b.entry())[1];
    /// let p = b.bin(BinOp::Mul, a, c);
    /// b.ret(Some(p));
    ///
    /// let jit = Jit::new().unwrap();
    /// let mul = jit.compile(&b.finish()).expect("mul is well-formed");
    ///
    /// // SAFETY: the signature is `fn(int, int) -> int`, and `mul` outlives the call.
    /// let mul_fn: extern "C" fn(i64, i64) -> i64 = unsafe { mul.entry() };
    /// assert_eq!(mul_fn(6, 7), 42);
    /// ```
    pub fn compile(&self, func: &Function) -> Result<Compiled, JitError> {
        func.validate()?;
        let clif = translate::translate(func, &*self.isa)?;

        let mut context = Context::for_function(clif);
        let code = context
            .compile(&*self.isa, &mut Default::default())
            .map_err(|err| JitError::Codegen(err.inner.to_string()))?;

        if !code.buffer.relocs().is_empty() {
            return Err(JitError::Unsupported(
                "the compiled code requires runtime relocations",
            ));
        }

        let bytes = code.code_buffer();
        let region = place(bytes)?;
        Ok(Compiled::new(
            region,
            func.name().to_string(),
            func.params().to_vec(),
            func.ret(),
            bytes.len(),
        ))
    }
}

impl fmt::Debug for Jit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Jit")
            .field("triple", &self.isa.triple())
            .finish()
    }
}

/// Compiles a function with a fresh host engine, for the one-off case.
///
/// This is the shortcut for compiling a single function where holding onto a
/// [`Jit`] does not pay off. It builds an engine, compiles, and returns the runnable
/// [`Compiled`]. When compiling several functions, build one [`Jit`] with
/// [`Jit::new`] and reuse it — that creates the host code generator once instead of
/// per call.
///
/// # Errors
///
/// The same as [`Jit::new`] followed by [`Jit::compile`]: the host being unsupported,
/// the function being invalid or using an unsupported feature, a code-generation
/// failure, or executable memory being unavailable.
///
/// # Examples
///
/// ```
/// use jit_lang::compile;
/// use ir_lang::{Builder, BinOp, Type};
///
/// // fn square(x: int) -> int { x * x }
/// let mut b = Builder::new("square", &[Type::Int], Type::Int);
/// let x = b.block_params(b.entry())[0];
/// let sq = b.bin(BinOp::Mul, x, x);
/// b.ret(Some(sq));
///
/// let square = compile(&b.finish()).expect("square is well-formed");
///
/// // SAFETY: the signature is `fn(int) -> int`, and `square` outlives the call.
/// let square_fn: extern "C" fn(i64) -> i64 = unsafe { square.entry() };
/// assert_eq!(square_fn(7), 49);
/// ```
pub fn compile(func: &Function) -> Result<Compiled, JitError> {
    Jit::new()?.compile(func)
}

/// Sets one code-generator flag, mapping a rejected setting to a [`JitError::Codegen`].
fn set_flag(flags: &mut settings::Builder, name: &str, value: &str) -> Result<(), JitError> {
    flags
        .set(name, value)
        .map_err(|err| JitError::Codegen(err.to_string()))
}

/// Copies machine code into a fresh guard-flanked region, flips it to read-execute, and
/// synchronizes the instruction cache so the code is safe to run on any supported host.
fn place(code: &[u8]) -> Result<Region, JitError> {
    let mut region = Region::with_guard_pages(code.len())?;
    region.write(0, code)?;
    region.protect(Protection::ReadExecute)?;
    icache::synchronize(region.as_ptr(), code.len());
    Ok(region)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and call the compiled result directly"
)]
mod tests {
    use super::{Jit, compile};
    use crate::JitError;
    use ir_lang::{BinOp, Builder, Type};

    #[test]
    fn test_compile_reports_signature_and_code_size() {
        let mut b = Builder::new("double", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let sum = b.bin(BinOp::Add, x, x);
        b.ret(Some(sum));
        let compiled = compile(&b.finish()).expect("double is well-formed");

        assert_eq!(compiled.name(), "double");
        assert_eq!(compiled.params(), &[Type::Int]);
        assert_eq!(compiled.ret(), Type::Int);
        assert!(compiled.code_len() > 0);
    }

    #[test]
    fn test_compiled_double_runs() {
        let mut b = Builder::new("double", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let sum = b.bin(BinOp::Add, x, x);
        b.ret(Some(sum));
        let compiled = compile(&b.finish()).unwrap();

        // SAFETY: the signature is `fn(int) -> int` and `compiled` outlives the call.
        let double: extern "C" fn(i64) -> i64 = unsafe { compiled.entry() };
        assert_eq!(double(21), 42);
        assert_eq!(double(-1), -2);
    }

    #[test]
    fn test_engine_is_reusable_across_functions() {
        let jit = Jit::new().unwrap();
        for (name, k) in [("a", 1_i64), ("b", 2), ("c", 3)] {
            let mut b = Builder::new(name, &[Type::Int], Type::Int);
            let x = b.block_params(b.entry())[0];
            let c = b.iconst(k);
            let sum = b.bin(BinOp::Add, x, c);
            b.ret(Some(sum));
            let compiled = jit.compile(&b.finish()).unwrap();
            assert_eq!(compiled.name(), name);
            // SAFETY: the signature is `fn(int) -> int` and `compiled` outlives the call.
            let f: extern "C" fn(i64) -> i64 = unsafe { compiled.entry() };
            assert_eq!(f(40), 40 + k);
        }
    }

    #[test]
    fn test_invalid_function_is_rejected() {
        // Declares an int return but returns nothing.
        let mut b = Builder::new("bad", &[], Type::Int);
        b.ret(None);
        assert!(matches!(compile(&b.finish()), Err(JitError::InvalidIr(_))));
    }
}
