//! Compile and run floating-point functions.
//!
//! Shows the `float` and `bool` ABI: a `float` is an `f64`, and a `bool` result is a
//! byte that an `extern "C" fn(..) -> bool` reads correctly.
//!
//! ```bash
//! cargo run --example floats
//! ```

use ir_lang::{BinOp, Builder, Type};
use jit_lang::Jit;

fn main() {
    let jit = Jit::new().expect("the host is supported");

    // fn hypot_sq(a: float, b: float) -> float { a * a + b * b }
    let hypot_sq = {
        let mut bld = Builder::new("hypot_sq", &[Type::Float, Type::Float], Type::Float);
        let a = bld.block_params(bld.entry())[0];
        let b = bld.block_params(bld.entry())[1];
        let aa = bld.bin(BinOp::Mul, a, a);
        let bb = bld.bin(BinOp::Mul, b, b);
        let sum = bld.bin(BinOp::Add, aa, bb);
        bld.ret(Some(sum));
        jit.compile(&bld.finish()).expect("hypot_sq is well-formed")
    };

    // SAFETY: the signature is `fn(float, float) -> float`, and the handle outlives the call.
    let hypot_sq_fn: extern "C" fn(f64, f64) -> f64 = unsafe { hypot_sq.entry() };
    println!("hypot_sq(3.0, 4.0) = {}", hypot_sq_fn(3.0, 4.0)); // 25.0

    // fn is_close(a: float, b: float) -> bool { a == b }
    let is_close = {
        let mut bld = Builder::new("is_close", &[Type::Float, Type::Float], Type::Bool);
        let a = bld.block_params(bld.entry())[0];
        let b = bld.block_params(bld.entry())[1];
        let eq = bld.bin(BinOp::Eq, a, b);
        bld.ret(Some(eq));
        jit.compile(&bld.finish()).expect("is_close is well-formed")
    };

    // SAFETY: a `bool` return is a 0/1 byte; the handle outlives every call.
    let is_close_fn: extern "C" fn(f64, f64) -> bool = unsafe { is_close.entry() };
    println!("is_close(1.5, 1.5) = {}", is_close_fn(1.5, 1.5));
    println!("is_close(1.5, 2.5) = {}", is_close_fn(1.5, 2.5));

    assert_eq!(hypot_sq_fn(3.0, 4.0), 25.0);
    assert!(is_close_fn(1.5, 1.5));
    assert!(!is_close_fn(1.5, 2.5));
}
