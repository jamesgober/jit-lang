//! End-to-end JIT workflow tests.
//!
//! Each test walks the whole path a front-end takes: build a function with the IR
//! builder, compile it with [`jit_lang`] to native machine code, transmute the entry
//! point to a matching `extern "C"` function pointer, and call it — checking that the
//! code computes what the source program means. Unlike a reference interpreter, the
//! answers here come from instructions the CPU actually executed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and assert on concrete results"
)]

use ir_lang::{BinOp, Builder, Type, UnOp};
use jit_lang::{Jit, compile};

#[test]
fn test_double_compiles_and_runs() {
    // fn double(x: int) -> int { x + x }
    let mut b = Builder::new("double", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let sum = b.bin(BinOp::Add, x, x);
    b.ret(Some(sum));
    let f = compile(&b.finish()).expect("double is well-formed");

    assert_eq!(f.name(), "double");
    assert_eq!(f.params(), &[Type::Int]);
    assert_eq!(f.ret(), Type::Int);

    // SAFETY: the signature is `fn(int) -> int` and `f` outlives every call.
    let double: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(double(21), 42);
    assert_eq!(double(-5), -10);
    assert_eq!(double(0), 0);
}

#[test]
fn test_abs_with_a_branch_runs_both_arms() {
    // fn abs(x: int) -> int { if x < 0 { -x } else { x } }
    let mut b = Builder::new("abs", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let join = b.create_block(&[Type::Int]);
    let neg_blk = b.create_block(&[]);
    let pos_blk = b.create_block(&[]);

    let zero = b.iconst(0);
    let is_neg = b.bin(BinOp::Lt, x, zero);
    b.branch(is_neg, neg_blk, &[], pos_blk, &[]);

    b.switch_to(neg_blk);
    let negated = b.un(UnOp::Neg, x);
    b.jump(join, &[negated]);

    b.switch_to(pos_blk);
    b.jump(join, &[x]);

    b.switch_to(join);
    let result = b.block_params(join)[0];
    b.ret(Some(result));

    let f = compile(&b.finish()).expect("abs is well-formed");
    // SAFETY: the signature is `fn(int) -> int` and `f` outlives every call.
    let abs: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(abs(-7), 7);
    assert_eq!(abs(7), 7);
    assert_eq!(abs(0), 0);
}

#[test]
fn test_max_diamond_passes_the_winner_through_a_block_parameter() {
    // fn max(a: int, b: int) -> int { if a < b { b } else { a } }
    let mut f = Builder::new("max", &[Type::Int, Type::Int], Type::Int);
    let a = f.block_params(f.entry())[0];
    let c = f.block_params(f.entry())[1];
    let join = f.create_block(&[Type::Int]);
    let then_blk = f.create_block(&[]);
    let else_blk = f.create_block(&[]);

    let cond = f.bin(BinOp::Lt, a, c);
    f.branch(cond, then_blk, &[], else_blk, &[]);
    f.switch_to(then_blk);
    f.jump(join, &[c]);
    f.switch_to(else_blk);
    f.jump(join, &[a]);
    f.switch_to(join);
    let r = f.block_params(join)[0];
    f.ret(Some(r));

    let compiled = compile(&f.finish()).expect("max is well-formed");
    // SAFETY: the signature is `fn(int, int) -> int` and `compiled` outlives every call.
    let max: extern "C" fn(i64, i64) -> i64 = unsafe { compiled.entry() };
    assert_eq!(max(3, 9), 9);
    assert_eq!(max(9, 3), 9);
    assert_eq!(max(4, 4), 4);
    assert_eq!(max(-2, -8), -2);
}

#[test]
fn test_countdown_loop_runs_to_completion() {
    // fn sum_to_zero(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }
    let mut b = Builder::new("sum_to_zero", &[Type::Int], Type::Int);
    let n0 = b.block_params(b.entry())[0];
    let header = b.create_block(&[Type::Int, Type::Int]);
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

    let f = compile(&b.finish()).expect("sum_to_zero is well-formed");
    // SAFETY: the signature is `fn(int) -> int` and `f` outlives every call.
    let sum_to_zero: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(sum_to_zero(5), 15); // 5 + 4 + 3 + 2 + 1
    assert_eq!(sum_to_zero(1), 1);
    assert_eq!(sum_to_zero(0), 0);
    assert_eq!(sum_to_zero(-3), 0);
    assert_eq!(sum_to_zero(100), 5050);
}

#[test]
fn test_float_arithmetic_runs() {
    // fn scale_sq(r: float) -> float { 3.0 * r * r }
    let mut b = Builder::new("scale_sq", &[Type::Float], Type::Float);
    let r = b.block_params(b.entry())[0];
    let three = b.fconst(3.0);
    let pr = b.bin(BinOp::Mul, three, r);
    let scaled = b.bin(BinOp::Mul, pr, r);
    b.ret(Some(scaled));

    let f = compile(&b.finish()).expect("scale_sq is well-formed");
    // SAFETY: the signature is `fn(float) -> float` and `f` outlives every call.
    let scale_sq: extern "C" fn(f64) -> f64 = unsafe { f.entry() };
    assert!((scale_sq(2.0) - 12.0).abs() < 1e-9);
    assert!((scale_sq(0.0) - 0.0).abs() < 1e-9);
    assert!((scale_sq(10.0) - 300.0).abs() < 1e-9);
}

#[test]
fn test_float_comparison_returns_bool() {
    // fn not_less(a: float, b: float) -> bool { !(a < b) }
    let mut f = Builder::new("not_less", &[Type::Float, Type::Float], Type::Bool);
    let a = f.block_params(f.entry())[0];
    let c = f.block_params(f.entry())[1];
    let less = f.bin(BinOp::Lt, a, c);
    let not_less = f.un(UnOp::Not, less);
    f.ret(Some(not_less));

    let compiled = compile(&f.finish()).expect("not_less is well-formed");
    // SAFETY: a `bool` return is a 0/1 byte; `compiled` outlives every call.
    let not_less: extern "C" fn(f64, f64) -> bool = unsafe { compiled.entry() };
    assert!(!not_less(1.0, 2.0));
    assert!(not_less(2.0, 1.0));
    assert!(not_less(1.0, 1.0));
}

#[test]
fn test_boolean_logic_runs() {
    // fn between(x: int, lo: int, hi: int) -> bool { lo <= x && x <= hi }
    let mut b = Builder::new("between", &[Type::Int, Type::Int, Type::Int], Type::Bool);
    let x = b.block_params(b.entry())[0];
    let lo = b.block_params(b.entry())[1];
    let hi = b.block_params(b.entry())[2];
    let above_lo = b.bin(BinOp::Le, lo, x);
    let below_hi = b.bin(BinOp::Le, x, hi);
    let inside = b.bin(BinOp::And, above_lo, below_hi);
    b.ret(Some(inside));

    let f = compile(&b.finish()).expect("between is well-formed");
    // SAFETY: the signature is `fn(int, int, int) -> bool` and `f` outlives every call.
    let between: extern "C" fn(i64, i64, i64) -> bool = unsafe { f.entry() };
    assert!(between(5, 1, 10));
    assert!(between(1, 1, 10));
    assert!(between(10, 1, 10));
    assert!(!between(0, 1, 10));
    assert!(!between(11, 1, 10));
}

#[test]
fn test_integer_division_runs() {
    // fn half(x: int) -> int { x / 2 }
    let mut b = Builder::new("half", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let two = b.iconst(2);
    let q = b.bin(BinOp::Div, x, two);
    b.ret(Some(q));

    let f = compile(&b.finish()).expect("half is well-formed");
    // SAFETY: the signature is `fn(int) -> int` and `f` outlives every call.
    let half: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(half(42), 21);
    assert_eq!(half(7), 3); // truncates toward zero
    assert_eq!(half(-8), -4);
}

#[test]
fn test_unit_function_returns_nothing() {
    // fn noop() -> unit {}
    let mut b = Builder::new("noop", &[], Type::Unit);
    b.ret(None);
    let f = compile(&b.finish()).expect("noop is well-formed");
    assert_eq!(f.ret(), Type::Unit);

    // SAFETY: the signature is `fn()` with no return; `f` outlives the call.
    let noop: extern "C" fn() = unsafe { f.entry() };
    noop(); // returns cleanly
}

#[test]
fn test_one_engine_compiles_many_functions() {
    let jit = Jit::new().expect("the host is supported");
    for k in 0..8_i64 {
        // fn add_k(x: int) -> int { x + k }
        let mut b = Builder::new("add_k", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let c = b.iconst(k);
        let sum = b.bin(BinOp::Add, x, c);
        b.ret(Some(sum));
        let compiled = jit.compile(&b.finish()).expect("add_k is well-formed");
        // SAFETY: the signature is `fn(int) -> int` and `compiled` outlives the call.
        let add_k: extern "C" fn(i64) -> i64 = unsafe { compiled.entry() };
        assert_eq!(add_k(100), 100 + k);
    }
}
