//! Functions with frames far larger than one page.
//!
//! A loop that carries thousands of integers around its back edge keeps them all live
//! at once, so the register allocator spills them and the frame grows to tens of
//! kilobytes. The prologue must touch the stack one page at a time so the operating
//! system's guard page is hit in order. Without that, the first store into the frame
//! lands past the guard page and the process dies with an access violation (Windows)
//! or can step into an adjacent mapping (Linux threads).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and assert on concrete results"
)]

use ir_lang::{BinOp, Builder, Function, Type};
use jit_lang::compile;

/// `fn wide(x: int, k: int) -> int`: `n` accumulators, each starting at `x * (7i + 3)`
/// and gaining `x` on each of `k` iterations, summed at the end.
fn wide(n: usize) -> Function {
    let mut b = Builder::new("wide", &[Type::Int, Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let k0 = b.block_params(b.entry())[1];
    let header = b.create_block(&vec![Type::Int; n + 1]);
    let body = b.create_block(&[]);
    let exit = b.create_block(&[]);

    let mut init = vec![k0];
    for i in 0..n {
        let c = b.iconst(i as i64 * 7 + 3);
        init.push(b.bin(BinOp::Mul, x, c));
    }
    b.jump(header, &init);

    b.switch_to(header);
    let hp = b.block_params(header).to_vec();
    let zero = b.iconst(0);
    let more = b.bin(BinOp::Gt, hp[0], zero);
    b.branch(more, body, &[], exit, &[]);

    b.switch_to(body);
    let one = b.iconst(1);
    let mut next = vec![b.bin(BinOp::Sub, hp[0], one)];
    for &v in &hp[1..] {
        next.push(b.bin(BinOp::Add, v, x));
    }
    b.jump(header, &next);

    b.switch_to(exit);
    let mut acc = hp[1];
    for &p in &hp[2..] {
        acc = b.bin(BinOp::Add, acc, p);
    }
    b.ret(Some(acc));
    b.finish()
}

fn expected(n: usize, x: i64, k: i64) -> i64 {
    (0..n).fold(0i64, |a, i| {
        a.wrapping_add(
            x.wrapping_mul(i as i64 * 7 + 3)
                .wrapping_add(x.wrapping_mul(k)),
        )
    })
}

/// Compiles `wide(n)` and calls it on a fresh thread with a 1 MiB stack, so the frame
/// starts near the committed top of a stack the OS grows one guard page at a time.
fn run_wide(n: usize) {
    let f = compile(&wide(n)).expect("wide is well-formed");
    let got = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            // SAFETY: the signature is `fn(int, int) -> int`, and `f` is moved into
            // this closure, so it outlives the call.
            let wide: extern "C" fn(i64, i64) -> i64 = unsafe { f.entry() };
            wide(1, 3)
        })
        .expect("spawn")
        .join()
        .expect("the compiled call returns");
    assert_eq!(got, expected(n, 1, 3));
}

#[test]
fn test_small_frame_runs() {
    run_wide(100);
}

#[test]
fn test_frame_of_many_pages_runs() {
    // About 40 KB of spill slots: ten pages below the stack pointer. Smaller frames
    // can land inside stack the OS has already committed and pass by luck.
    run_wide(5_000);
}
