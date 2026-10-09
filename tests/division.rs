//! Integer division never traps (ISSUES H10, fixed in 1.0.1).
//!
//! Cranelift's `sdiv` faults in hardware on a zero divisor and on `i64::MIN / -1`, which
//! in 1.0.0 killed the host process. These tests run exactly those inputs through
//! natively compiled code, in this test process, and compare every result with the
//! reference interpreter in `support` (the bytecode tier's operator semantics, with
//! `x / 0 == 0` as jit-lang documents). A regression would not fail an assertion: it
//! would abort the whole test binary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and assert on concrete results"
)]

mod support;

use ir_lang::{BinOp, Builder, Function, Type, UnOp, Value};
use jit_lang::{Compiled, compile};
use proptest::prelude::*;
use support::{Value as Ref, eval, int_div};

/// The values a divide is most likely to get wrong.
const EDGES: [i64; 9] = [
    0,
    1,
    -1,
    2,
    -2,
    i64::MIN,
    i64::MIN + 1,
    i64::MAX,
    i64::MAX - 1,
];

/// `fn div(a: int, b: int) -> int { a / b }`.
fn div_fn() -> Function {
    let mut b = Builder::new("div", &[Type::Int, Type::Int], Type::Int);
    let a = b.block_params(b.entry())[0];
    let c = b.block_params(b.entry())[1];
    let q = b.bin(BinOp::Div, a, c);
    b.ret(Some(q));
    b.finish()
}

/// `fn rem(a: int, b: int) -> int { a - b * (a / b) }`: ir-lang 1.0 has no remainder
/// op, so a front-end spells truncated remainder this way. It divides, so it exercises
/// the same guard.
fn rem_fn() -> Function {
    let mut b = Builder::new("rem", &[Type::Int, Type::Int], Type::Int);
    let a = b.block_params(b.entry())[0];
    let c = b.block_params(b.entry())[1];
    let q = b.bin(BinOp::Div, a, c);
    let p = b.bin(BinOp::Mul, c, q);
    let r = b.bin(BinOp::Sub, a, p);
    b.ret(Some(r));
    b.finish()
}

/// Calls a compiled `fn(int, int) -> int`.
fn call2(compiled: &Compiled, a: i64, b: i64) -> i64 {
    // SAFETY: every caller compiles a `fn(int, int) -> int`, and `compiled` outlives the call.
    let f: extern "C" fn(i64, i64) -> i64 = unsafe { compiled.entry() };
    f(a, b)
}

/// The reference result for a two-int-argument function.
fn reference2(func: &Function, a: i64, b: i64) -> i64 {
    eval(func, &[Ref::Int(a), Ref::Int(b)], 1_000).int()
}

#[test]
fn test_divide_by_zero_returns_zero_without_trapping() {
    let func = div_fn();
    let compiled = compile(&func).expect("div is well-formed");
    for a in EDGES.into_iter().chain([42, -42, 7]) {
        assert_eq!(call2(&compiled, a, 0), 0, "{a} / 0");
        assert_eq!(call2(&compiled, a, 0), reference2(&func, a, 0));
    }
}

#[test]
fn test_min_divided_by_negative_one_wraps_to_min() {
    let func = div_fn();
    let compiled = compile(&func).expect("div is well-formed");
    assert_eq!(call2(&compiled, i64::MIN, -1), i64::MIN);
    assert_eq!(call2(&compiled, i64::MIN, -1), i64::MIN.wrapping_div(-1));
    assert_eq!(
        call2(&compiled, i64::MIN, -1),
        reference2(&func, i64::MIN, -1)
    );
    // The other divide-by-minus-one cases are plain negation.
    assert_eq!(call2(&compiled, i64::MAX, -1), -i64::MAX);
    assert_eq!(call2(&compiled, i64::MIN + 1, -1), i64::MAX);
    assert_eq!(call2(&compiled, 0, -1), 0);
    assert_eq!(call2(&compiled, 5, -1), -5);
}

#[test]
fn test_min_remainder_negative_one_is_zero() {
    let func = rem_fn();
    let compiled = compile(&func).expect("rem is well-formed");
    assert_eq!(call2(&compiled, i64::MIN, -1), 0);
    assert_eq!(call2(&compiled, i64::MIN, -1), i64::MIN.wrapping_rem(-1));
    // Remainder by zero divides by zero first: `a - 0 * 0 == a`, and no trap.
    assert_eq!(call2(&compiled, i64::MIN, 0), i64::MIN);
    assert_eq!(call2(&compiled, 17, 0), 17);
    // Ordinary remainders keep the dividend's sign, as Rust's `%` does.
    assert_eq!(call2(&compiled, 17, 5), 2);
    assert_eq!(call2(&compiled, -17, 5), -2);
    assert_eq!(call2(&compiled, 17, -5), 2);
}

#[test]
fn test_every_edge_pair_matches_the_reference() {
    let div = div_fn();
    let rem = rem_fn();
    let compiled_div = compile(&div).expect("div is well-formed");
    let compiled_rem = compile(&rem).expect("rem is well-formed");
    for a in EDGES {
        for b in EDGES {
            assert_eq!(
                call2(&compiled_div, a, b),
                reference2(&div, a, b),
                "{a} / {b}"
            );
            assert_eq!(
                call2(&compiled_rem, a, b),
                reference2(&rem, a, b),
                "{a} rem {b}"
            );
        }
    }
}

#[test]
fn test_normal_division_truncates_toward_zero() {
    let compiled = compile(&div_fn()).expect("div is well-formed");
    assert_eq!(call2(&compiled, 7, 2), 3);
    assert_eq!(call2(&compiled, -7, 2), -3);
    assert_eq!(call2(&compiled, 7, -2), -3);
    assert_eq!(call2(&compiled, -7, -2), 3);
    assert_eq!(call2(&compiled, i64::MAX, 2), i64::MAX / 2);
    assert_eq!(call2(&compiled, i64::MIN, 2), i64::MIN / 2);
    assert_eq!(call2(&compiled, i64::MIN, i64::MIN), 1);
    assert_eq!(call2(&compiled, 3, i64::MAX), 0);
}

/// A constant divisor lets Cranelift fold the guard at compile time. The folded code
/// must give the same answers, and folding `sdiv` by a constant `0` or `-1` must not
/// reintroduce the fault.
#[test]
fn test_constant_divisors_fold_to_the_same_results() {
    for divisor in EDGES {
        let mut b = Builder::new("div_by_k", &[Type::Int], Type::Int);
        let a = b.block_params(b.entry())[0];
        let k = b.iconst(divisor);
        let q = b.bin(BinOp::Div, a, k);
        b.ret(Some(q));
        let func = b.finish();
        let compiled = compile(&func).expect("div_by_k is well-formed");
        // SAFETY: the signature is `fn(int) -> int` and `compiled` outlives the call.
        let f: extern "C" fn(i64) -> i64 = unsafe { compiled.entry() };
        for a in EDGES {
            let expected = eval(&func, &[Ref::Int(a)], 10).int();
            assert_eq!(f(a), expected, "{a} / const {divisor}");
            assert_eq!(f(a), int_div(a, divisor));
        }
    }
}

/// Both operands constant: the whole division is evaluated by Cranelift's constant
/// folder, including the two cases that would fault at run time.
#[test]
fn test_fully_constant_division_folds_without_faulting() {
    for (a, divisor) in [(i64::MIN, -1), (5, 0), (i64::MIN, 0), (-9, 4)] {
        let mut b = Builder::new("const_div", &[], Type::Int);
        let x = b.iconst(a);
        let k = b.iconst(divisor);
        let q = b.bin(BinOp::Div, x, k);
        b.ret(Some(q));
        let compiled = compile(&b.finish()).expect("const_div is well-formed");
        // SAFETY: the signature is `fn() -> int` and `compiled` outlives the call.
        let f: extern "C" fn() -> i64 = unsafe { compiled.entry() };
        assert_eq!(f(), int_div(a, divisor), "{a} / {divisor}");
    }
}

/// `fn digits(n: int, base: int) -> int`: count how many times `n` divides by `base`
/// before reaching zero. The loop divides by a runtime divisor on every iteration,
/// including `base == 0` (stops at once: `n / 0 == 0`) and `base == -1`.
#[test]
fn test_division_in_a_loop_matches_the_reference() {
    let mut b = Builder::new("digits", &[Type::Int, Type::Int], Type::Int);
    let n0 = b.block_params(b.entry())[0];
    let base = b.block_params(b.entry())[1];
    let header = b.create_block(&[Type::Int, Type::Int]); // (n, count)
    let body = b.create_block(&[]);
    let exit = b.create_block(&[]);
    let zero = b.iconst(0);
    b.jump(header, &[n0, zero]);

    b.switch_to(header);
    let n = b.block_params(header)[0];
    let count = b.block_params(header)[1];
    let z = b.iconst(0);
    let more = b.bin(BinOp::Ne, n, z);
    // Cap the iterations so `base == 1` or `-1` still ends: stop after 70 rounds.
    let cap = b.iconst(70);
    let under = b.bin(BinOp::Lt, count, cap);
    let go = b.bin(BinOp::And, more, under);
    b.branch(go, body, &[], exit, &[]);

    b.switch_to(body);
    let n2 = b.bin(BinOp::Div, n, base);
    let one = b.iconst(1);
    let count2 = b.bin(BinOp::Add, count, one);
    b.jump(header, &[n2, count2]);

    b.switch_to(exit);
    b.ret(Some(count));
    let func = b.finish();
    let compiled = compile(&func).expect("digits is well-formed");

    assert_eq!(call2(&compiled, 12_345, 10), 5);
    assert_eq!(call2(&compiled, 12_345, 0), 1);
    assert_eq!(call2(&compiled, i64::MIN, -1), 70);
    for n in EDGES.into_iter().chain([12_345, -999]) {
        for base in EDGES.into_iter().chain([10, 16]) {
            assert_eq!(
                call2(&compiled, n, base),
                eval(&func, &[Ref::Int(n), Ref::Int(base)], 10_000).int(),
                "digits({n}, {base})"
            );
        }
    }
}

/// Float division is IEEE and never traps; dividing by zero gives an infinity or NaN.
#[test]
fn test_float_division_by_zero_is_ieee() {
    let mut b = Builder::new("fdiv", &[Type::Float, Type::Float], Type::Float);
    let a = b.block_params(b.entry())[0];
    let c = b.block_params(b.entry())[1];
    let q = b.bin(BinOp::Div, a, c);
    b.ret(Some(q));
    let compiled = compile(&b.finish()).expect("fdiv is well-formed");
    // SAFETY: the signature is `fn(float, float) -> float` and `compiled` outlives the call.
    let f: extern "C" fn(f64, f64) -> f64 = unsafe { compiled.entry() };
    assert_eq!(f(1.0, 0.0), f64::INFINITY);
    assert_eq!(f(-1.0, 0.0), f64::NEG_INFINITY);
    assert!(f(0.0, 0.0).is_nan());
}

/// Either an edge value or an arbitrary one, so the special divisors come up often.
fn edgy() -> impl Strategy<Value = i64> {
    prop_oneof![
        prop::sample::select(EDGES.to_vec()),
        any::<i64>(),
        -8i64..=8,
    ]
}

/// A random straight-line integer program over `add`/`sub`/`mul`/`div`/`neg`.
#[derive(Debug, Clone)]
struct Program {
    consts: Vec<i64>,
    /// `(op, a, b)`: `op` picks add, sub, mul, div (twice as likely), or neg; `a`/`b`
    /// index the value pool modulo its length.
    steps: Vec<(u8, usize, usize)>,
    inputs: [i64; 2],
}

fn program() -> impl Strategy<Value = Program> {
    (
        prop::collection::vec(edgy(), 0..=4),
        prop::collection::vec((0u8..6, any::<usize>(), any::<usize>()), 1..=16),
        [edgy(), edgy()],
    )
        .prop_map(|(consts, steps, inputs)| Program {
            consts,
            steps,
            inputs,
        })
}

fn build(prog: &Program) -> Function {
    let mut b = Builder::new("prog", &[Type::Int, Type::Int], Type::Int);
    let mut pool: Vec<Value> = b.block_params(b.entry()).to_vec();
    for &k in &prog.consts {
        pool.push(b.iconst(k));
    }
    for &(op, i, j) in &prog.steps {
        let len = pool.len();
        let (x, y) = (pool[i % len], pool[j % len]);
        let v = match op {
            0 => b.bin(BinOp::Add, x, y),
            1 => b.bin(BinOp::Sub, x, y),
            2 => b.bin(BinOp::Mul, x, y),
            3 | 4 => b.bin(BinOp::Div, x, y),
            _ => b.un(UnOp::Neg, x),
        };
        pool.push(v);
    }
    let last = pool[pool.len() - 1];
    b.ret(Some(last));
    b.finish()
}

proptest! {
    /// A single division, natively, equals the reference on any operands.
    #[test]
    fn div_matches_reference(a in edgy(), c in edgy()) {
        let func = div_fn();
        let compiled = compile(&func).expect("div is well-formed");
        prop_assert_eq!(call2(&compiled, a, c), reference2(&func, a, c));
    }

    /// Composed remainder, natively, equals the reference on any operands.
    #[test]
    fn rem_matches_reference(a in edgy(), c in edgy()) {
        let func = rem_fn();
        let compiled = compile(&func).expect("rem is well-formed");
        prop_assert_eq!(call2(&compiled, a, c), reference2(&func, a, c));
    }

    /// Random arithmetic chains — where intermediate results hit zero, `-1`, and
    /// `MIN` far more often than uniform inputs would — run natively without a fault
    /// and agree with the reference interpreter.
    #[test]
    fn arithmetic_programs_match_reference(prog in program()) {
        let func = build(&prog);
        let compiled = compile(&func).expect("a generated program is well-formed");
        let [a, c] = prog.inputs;
        prop_assert_eq!(call2(&compiled, a, c), reference2(&func, a, c));
    }
}
