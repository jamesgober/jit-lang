//! The compiled, runnable function: machine code in executable memory and the
//! metadata needed to call it.

use std::mem;

use ir_lang::Type;
use pager_lang::Region;

/// A compiled function: native machine code living in executable memory, ready to run.
///
/// A [`Jit`](crate::Jit) produces one of these by lowering an
/// [`ir_lang::Function`](ir_lang::Function) to host machine code and placing it in a
/// page-aligned, read-execute [`Region`]. `Compiled` owns that region, so the code
/// stays mapped and runnable for exactly as long as the value is alive and is freed
/// when it is dropped — calling into a function whose `Compiled` has been dropped is a
/// use-after-free.
///
/// The signature accessors — [`name`](Self::name), [`params`](Self::params), and
/// [`ret`](Self::ret) — report the function as it was compiled, so a caller can pick
/// the right function-pointer type before calling. Running the code is inherently
/// unsafe: the machine has no way to check that the type you transmute the entry point
/// to matches what was compiled, so that final step is your responsibility through
/// [`entry`](Self::entry). See its documentation for the contract.
///
/// `Compiled` is [`Send`] and [`Sync`]: the region it owns is, and the rest of its
/// fields are plain data. Moving it between threads moves the mapping with it.
///
/// # Examples
///
/// Compile a function and read back what it is:
///
/// ```
/// use jit_lang::compile;
/// use ir_lang::{Builder, BinOp, Type};
///
/// // fn double(x: int) -> int { x + x }
/// let mut b = Builder::new("double", &[Type::Int], Type::Int);
/// let x = b.block_params(b.entry())[0];
/// let sum = b.bin(BinOp::Add, x, x);
/// b.ret(Some(sum));
///
/// let f = compile(&b.finish()).expect("double is well-formed");
/// assert_eq!(f.name(), "double");
/// assert_eq!(f.params(), &[Type::Int]);
/// assert_eq!(f.ret(), Type::Int);
/// assert!(f.code_len() > 0);
/// ```
pub struct Compiled {
    /// The executable memory holding the code. Owned so the mapping outlives every
    /// call and is freed on drop.
    region: Region,
    /// The compiled function's name, carried for inspection.
    name: String,
    /// The parameter types, in order, as the function was compiled.
    params: Vec<Type>,
    /// The return type as the function was compiled.
    ret: Type,
    /// The length of the emitted machine code in bytes (`<=` the region length, which
    /// is rounded up to whole pages).
    code_len: usize,
}

impl Compiled {
    /// Builds the handle from its already-prepared parts. Crate-internal: a `Compiled`
    /// only ever comes from a successful [`Jit::compile`](crate::Jit::compile).
    pub(crate) fn new(
        region: Region,
        name: String,
        params: Vec<Type>,
        ret: Type,
        code_len: usize,
    ) -> Self {
        Self {
            region,
            name,
            params,
            ret,
            code_len,
        }
    }

    /// The compiled function's name, as it was given to the IR builder.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut b = Builder::new("noop", &[], Type::Unit);
    /// b.ret(None);
    /// assert_eq!(compile(&b.finish()).unwrap().name(), "noop");
    /// ```
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The parameter types, in declaration order. These tell you the argument types of
    /// the function-pointer you transmute to with [`entry`](Self::entry).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut b = Builder::new("f", &[Type::Int, Type::Float], Type::Int);
    /// let x = b.block_params(b.entry())[0];
    /// b.ret(Some(x));
    /// assert_eq!(compile(&b.finish()).unwrap().params(), &[Type::Int, Type::Float]);
    /// ```
    #[must_use]
    pub fn params(&self) -> &[Type] {
        &self.params
    }

    /// The return type. A [`Unit`](ir_lang::Type::Unit) return means the function
    /// yields no value, so the matching function-pointer type has no return.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut b = Builder::new("f", &[], Type::Unit);
    /// b.ret(None);
    /// assert_eq!(compile(&b.finish()).unwrap().ret(), Type::Unit);
    /// ```
    #[must_use]
    pub fn ret(&self) -> Type {
        self.ret
    }

    /// The length of the emitted machine code, in bytes.
    ///
    /// This is the size of the function body, which is at most the length of the
    /// region holding it — the region is rounded up to whole pages, so it is usually
    /// larger. Useful for inspecting or logging how much code a function compiled to.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut b = Builder::new("f", &[Type::Int], Type::Int);
    /// let x = b.block_params(b.entry())[0];
    /// b.ret(Some(x));
    /// // An identity function still compiles to at least one instruction.
    /// assert!(compile(&b.finish()).unwrap().code_len() > 0);
    /// ```
    #[must_use]
    pub fn code_len(&self) -> usize {
        self.code_len
    }

    /// A raw pointer to the first byte of the compiled code — the function's entry
    /// point.
    ///
    /// The pointer is valid to obtain and to compare; it is into read-execute memory,
    /// so reading or running through it is `unsafe`. For the common case of calling
    /// the function, prefer [`entry`](Self::entry), which produces a typed
    /// function-pointer. The pointer is valid only while this `Compiled` is alive.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut b = Builder::new("f", &[], Type::Unit);
    /// b.ret(None);
    /// let f = compile(&b.finish()).unwrap();
    /// assert!(!f.as_ptr().is_null());
    /// ```
    #[must_use]
    pub fn as_ptr(&self) -> *const u8 {
        self.region.as_ptr()
    }

    /// Reinterprets the entry point as a function pointer of type `F` so the compiled
    /// code can be called.
    ///
    /// This is how you run a compiled function: pick an `extern "C"` function-pointer
    /// type whose signature matches what was compiled, get it from `entry`, and call
    /// it. The mapping from IR types to the C ABI is `int` to a 64-bit integer,
    /// `float` to `f64`, `bool` to a byte that is `0` or `1`, and a `unit` return to no
    /// return value. The compiled code uses the host C calling convention, which is
    /// what `extern "C"` denotes, so the two line up.
    ///
    /// # Safety
    ///
    /// The caller guarantees all of the following:
    ///
    /// - `F` is a function-pointer type (so it is pointer-sized, the size of the entry
    ///   address). Passing a non-function or differently-sized type is undefined
    ///   behavior.
    /// - `F`'s signature matches the compiled function: one `extern "C"` argument per
    ///   entry in [`params`](Self::params) with the ABI type above, and a return that
    ///   matches [`ret`](Self::ret). A mismatch is undefined behavior.
    /// - Every call through the returned pointer happens while `self` is alive. Once
    ///   `self` is dropped the code is unmapped and the pointer dangles.
    ///
    /// # Examples
    ///
    /// Call a compiled `fn double(x: int) -> int`:
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, BinOp, Type};
    ///
    /// let mut b = Builder::new("double", &[Type::Int], Type::Int);
    /// let x = b.block_params(b.entry())[0];
    /// let sum = b.bin(BinOp::Add, x, x);
    /// b.ret(Some(sum));
    /// let f = compile(&b.finish()).unwrap();
    ///
    /// // SAFETY: the signature matches `fn double(x: int) -> int`, and `f` outlives the call.
    /// let double: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    /// assert_eq!(double(21), 42);
    /// assert_eq!(double(-5), -10);
    /// ```
    ///
    /// Call one that returns a `bool`:
    ///
    /// ```
    /// use jit_lang::compile;
    /// use ir_lang::{Builder, BinOp, Type};
    ///
    /// // fn lt(a: int, b: int) -> bool { a < b }
    /// let mut b = Builder::new("lt", &[Type::Int, Type::Int], Type::Bool);
    /// let a = b.block_params(b.entry())[0];
    /// let c = b.block_params(b.entry())[1];
    /// let lt = b.bin(BinOp::Lt, a, c);
    /// b.ret(Some(lt));
    /// let f = compile(&b.finish()).unwrap();
    ///
    /// // SAFETY: a `bool` return is a 0/1 byte, which `extern "C" fn(..) -> bool` reads correctly.
    /// let lt: extern "C" fn(i64, i64) -> bool = unsafe { f.entry() };
    /// assert!(lt(1, 2));
    /// assert!(!lt(2, 1));
    /// ```
    #[must_use]
    pub unsafe fn entry<F: Copy>(&self) -> F {
        debug_assert_eq!(
            mem::size_of::<F>(),
            mem::size_of::<*const u8>(),
            "entry::<F>() requires F to be a pointer-sized function pointer",
        );
        let ptr = self.as_ptr();
        // SAFETY: `ptr` is the entry address of the compiled function. The caller's
        // `# Safety` contract guarantees `F` is a function-pointer type — hence
        // pointer-sized — whose signature matches the compiled code, so reinterpreting
        // the address as `F` produces a valid, callable pointer. `transmute_copy` reads
        // `size_of::<F>()` bytes from `&ptr`; with `F` pointer-sized that is exactly the
        // pointer, checked in debug by the assertion above.
        unsafe { mem::transmute_copy::<*const u8, F>(&ptr) }
    }
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled")
            .field("name", &self.name)
            .field("params", &self.params)
            .field("ret", &self.ret)
            .field("code_len", &self.code_len)
            .field("entry", &self.as_ptr())
            .finish()
    }
}
