//! Criterion benchmarks for jit-lang.
//!
//! Two things are worth measuring: how long it takes to compile a function (the cost
//! paid once, per function), and how cheap it is to call the result (the cost paid on
//! every invocation). The compile benchmarks cover a few function shapes and sizes; the
//! call benchmark confirms an invocation is a plain indirect call into native code.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ir_lang::{BinOp, Builder, Function, Type, UnOp};
use jit_lang::Jit;

/// A single-block function of `n` `add` steps over one parameter.
fn straight_line(n: usize) -> Function {
    let mut b = Builder::new("bench", &[Type::Int], Type::Int);
    let mut acc = b.block_params(b.entry())[0];
    for i in 0..n {
        let c = b.iconst(i as i64);
        acc = b.bin(BinOp::Add, acc, c);
    }
    b.ret(Some(acc));
    b.finish()
}

/// `fn max(a: int, b: int) -> int { if a < b { b } else { a } }` — a two-way branch
/// joining through a block parameter.
fn diamond() -> Function {
    let mut b = Builder::new("max", &[Type::Int, Type::Int], Type::Int);
    let a = b.block_params(b.entry())[0];
    let c = b.block_params(b.entry())[1];
    let join = b.create_block(&[Type::Int]);
    let then_blk = b.create_block(&[]);
    let else_blk = b.create_block(&[]);
    let cond = b.bin(BinOp::Lt, a, c);
    b.branch(cond, then_blk, &[], else_blk, &[]);
    b.switch_to(then_blk);
    b.jump(join, &[c]);
    b.switch_to(else_blk);
    b.jump(join, &[a]);
    b.switch_to(join);
    let r = b.block_params(join)[0];
    b.ret(Some(r));
    b.finish()
}

/// `fn sum_to(n: int) -> int { ... }` — a loop with a two-value back-edge.
fn loop_fn() -> Function {
    let mut b = Builder::new("sum_to", &[Type::Int], Type::Int);
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
    b.finish()
}

fn bench_compile(c: &mut Criterion) {
    let jit = Jit::new().expect("the host is supported");
    let mut group = c.benchmark_group("compile");

    let small = straight_line(16);
    group.bench_function("straight_line_16", |b| {
        b.iter(|| jit.compile(black_box(&small)).expect("compiles"))
    });

    let large = straight_line(256);
    group.bench_function("straight_line_256", |b| {
        b.iter(|| jit.compile(black_box(&large)).expect("compiles"))
    });

    let max = diamond();
    group.bench_function("diamond", |b| {
        b.iter(|| jit.compile(black_box(&max)).expect("compiles"))
    });

    let sum_to = loop_fn();
    group.bench_function("loop", |b| {
        b.iter(|| jit.compile(black_box(&sum_to)).expect("compiles"))
    });

    group.finish();
}

fn bench_call(c: &mut Criterion) {
    let jit = Jit::new().expect("the host is supported");

    // fn neg_abs(x: int) -> int { if x < 0 { x } else { -x } } — a branch per call.
    let mut b = Builder::new("neg_abs", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let join = b.create_block(&[Type::Int]);
    let lt = b.create_block(&[]);
    let ge = b.create_block(&[]);
    let zero = b.iconst(0);
    let is_neg = b.bin(BinOp::Lt, x, zero);
    b.branch(is_neg, lt, &[], ge, &[]);
    b.switch_to(lt);
    b.jump(join, &[x]);
    b.switch_to(ge);
    let neg = b.un(UnOp::Neg, x);
    b.jump(join, &[neg]);
    b.switch_to(join);
    let r = b.block_params(join)[0];
    b.ret(Some(r));
    let compiled = jit.compile(&b.finish()).expect("neg_abs is well-formed");

    // SAFETY: the signature is `fn(int) -> int` and `compiled` outlives the benchmark.
    let neg_abs: extern "C" fn(i64) -> i64 = unsafe { compiled.entry() };

    c.bench_function("call_compiled", |b| {
        b.iter(|| black_box(neg_abs(black_box(-7))))
    });
}

criterion_group!(benches, bench_compile, bench_call);
criterion_main!(benches);
