//! Property tests: compiled code agrees with an independent reference evaluation.
//!
//! Each test generates a random function, compiles it, runs the native code, and
//! compares the result to the same computation done directly in Rust. The integer
//! property uses wrapping arithmetic on both sides, since that is what the generated
//! `iadd`/`isub`/`imul` do; the float property compares the single IEEE-754 operation
//! exactly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and assert on concrete results"
)]

use ir_lang::{BinOp, Builder, Type, Value};
use jit_lang::compile;
use proptest::prelude::*;

/// A randomly generated straight-line integer function and the inputs to run it with.
#[derive(Debug, Clone)]
struct IntProgram {
    /// Number of `int` parameters, 1 to 3.
    nparams: usize,
    /// Integer constants seeded into the value pool.
    consts: Vec<i64>,
    /// A sequence of `(op, a, b)` steps; `a` and `b` index the value pool modulo its
    /// current length, and `op` selects add (0), sub (1), or mul (2).
    ops: Vec<(u8, usize, usize)>,
    /// One argument per parameter for the call.
    inputs: Vec<i64>,
}

fn int_program() -> impl Strategy<Value = IntProgram> {
    (1usize..=3).prop_flat_map(|nparams| {
        (
            Just(nparams),
            prop::collection::vec(any::<i64>(), 0..=4),
            prop::collection::vec((0u8..3, any::<usize>(), any::<usize>()), 1..=12),
            prop::collection::vec(any::<i64>(), nparams),
        )
            .prop_map(|(nparams, consts, ops, inputs)| IntProgram {
                nparams,
                consts,
                ops,
                inputs,
            })
    })
}

/// Picks the IR binary op and the matching wrapping reference operation for a step.
fn op_of(tag: u8) -> (BinOp, fn(i64, i64) -> i64) {
    match tag % 3 {
        0 => (BinOp::Add, i64::wrapping_add),
        1 => (BinOp::Sub, i64::wrapping_sub),
        _ => (BinOp::Mul, i64::wrapping_mul),
    }
}

/// Evaluates the program directly with wrapping arithmetic — the oracle.
fn reference(prog: &IntProgram) -> i64 {
    let mut pool: Vec<i64> = prog.inputs.clone();
    pool.extend_from_slice(&prog.consts);
    for &(tag, a, b) in &prog.ops {
        let (_, f) = op_of(tag);
        let len = pool.len();
        let value = f(pool[a % len], pool[b % len]);
        pool.push(value);
    }
    *pool.last().expect("the pool always holds at least one value")
}

/// Builds the program as IR, compiles it, and runs the native code with its inputs.
fn jit(prog: &IntProgram) -> i64 {
    let mut b = Builder::new("prop", &vec![Type::Int; prog.nparams], Type::Int);
    let mut pool: Vec<Value> = b.block_params(b.entry()).to_vec();
    for &k in &prog.consts {
        pool.push(b.iconst(k));
    }
    for &(tag, a, b_idx) in &prog.ops {
        let (op, _) = op_of(tag);
        let len = pool.len();
        let value = b.bin(op, pool[a % len], pool[b_idx % len]);
        pool.push(value);
    }
    let result = *pool.last().expect("the pool always holds at least one value");
    b.ret(Some(result));

    let compiled = compile(&b.finish()).expect("a generated function is well-formed");
    let inputs = &prog.inputs;
    match prog.nparams {
        1 => {
            // SAFETY: the signature is `fn(int) -> int` and `compiled` outlives the call.
            let f: extern "C" fn(i64) -> i64 = unsafe { compiled.entry() };
            f(inputs[0])
        }
        2 => {
            // SAFETY: the signature is `fn(int, int) -> int` and `compiled` outlives the call.
            let f: extern "C" fn(i64, i64) -> i64 = unsafe { compiled.entry() };
            f(inputs[0], inputs[1])
        }
        _ => {
            // SAFETY: the signature is `fn(int, int, int) -> int` and `compiled` outlives the call.
            let f: extern "C" fn(i64, i64, i64) -> i64 = unsafe { compiled.entry() };
            f(inputs[0], inputs[1], inputs[2])
        }
    }
}

proptest! {
    /// The native code computes exactly what evaluating the same steps in Rust does.
    #[test]
    fn int_jit_matches_reference(prog in int_program()) {
        prop_assert_eq!(jit(&prog), reference(&prog));
    }

    /// A single float operation, compiled and run, equals the same IEEE-754 operation
    /// in Rust. Inputs are finite so the result is a definite value to compare against.
    #[test]
    fn float_binop_matches_reference(
        a in any::<f64>().prop_filter("finite", |x| x.is_finite()),
        c in any::<f64>().prop_filter("finite", |x| x.is_finite()),
        tag in 0u8..3,
    ) {
        let (op, expected): (BinOp, f64) = match tag {
            0 => (BinOp::Add, a + c),
            1 => (BinOp::Sub, a - c),
            _ => (BinOp::Mul, a * c),
        };
        let mut b = Builder::new("fop", &[Type::Float, Type::Float], Type::Float);
        let x = b.block_params(b.entry())[0];
        let y = b.block_params(b.entry())[1];
        let r = b.bin(op, x, y);
        b.ret(Some(r));
        let compiled = compile(&b.finish()).expect("fop is well-formed");
        // SAFETY: the signature is `fn(float, float) -> float` and `compiled` outlives the call.
        let f: extern "C" fn(f64, f64) -> f64 = unsafe { compiled.entry() };
        let got = f(a, c);
        prop_assert!(got == expected || (got.is_nan() && expected.is_nan()));
    }
}
