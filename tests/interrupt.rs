//! Stopping compiled loops with an [`Interrupt`] flag.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and assert on concrete results"
)]

use std::thread;
use std::time::Duration;

use ir_lang::{BinOp, Builder, Function, Type};
use jit_lang::{Interrupt, Jit};

/// `fn spin() -> ret { loop {} }`, an infinite loop with the given return type.
fn spin(ret: Type) -> Function {
    let mut b = Builder::new("spin", &[], ret);
    let top = b.create_block(&[]);
    b.jump(top, &[]);
    b.switch_to(top);
    b.jump(top, &[]);
    b.finish()
}

/// `fn sum_to(n: int) -> int`, a loop that ends.
fn sum_to() -> Function {
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

/// Two nested loops; the inner one never ends.
fn nested_spin() -> Function {
    let mut b = Builder::new("nested", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let outer = b.create_block(&[Type::Int]);
    let inner = b.create_block(&[Type::Int]);
    b.jump(outer, &[x]);
    b.switch_to(outer);
    let o = b.block_params(outer)[0];
    b.jump(inner, &[o]);
    b.switch_to(inner);
    let i = b.block_params(inner)[0];
    let one = b.iconst(1);
    let i2 = b.bin(BinOp::Add, i, one);
    let big = b.iconst(i64::MAX);
    let wrap = b.bin(BinOp::Eq, i2, big);
    b.branch(wrap, outer, &[i2], inner, &[i2]);
    b.finish()
}

/// Sets `interrupt` from another thread after a short delay.
fn trip_later(interrupt: &Interrupt) -> thread::JoinHandle<()> {
    let watchdog = interrupt.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        watchdog.interrupt();
    })
}

#[test]
fn test_watchdog_stops_an_infinite_loop() {
    let interrupt = Interrupt::new();
    let f = Jit::new()
        .unwrap()
        .compile_interruptible(&spin(Type::Int), &interrupt)
        .unwrap();
    let timer = trip_later(&interrupt);
    // SAFETY: the signature is `fn() -> int`, and `f` outlives the call.
    let run: extern "C" fn() -> i64 = unsafe { f.entry() };
    assert_eq!(run(), 0);
    assert!(interrupt.is_interrupted());
    timer.join().unwrap();
}

#[test]
fn test_watchdog_stops_an_inner_loop() {
    let interrupt = Interrupt::new();
    let f = Jit::new()
        .unwrap()
        .compile_interruptible(&nested_spin(), &interrupt)
        .unwrap();
    let timer = trip_later(&interrupt);
    // SAFETY: the signature is `fn(int) -> int`, and `f` outlives the call.
    let run: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(run(5), 0);
    timer.join().unwrap();
}

#[test]
fn test_set_flag_returns_the_zero_value_of_each_type() {
    let jit = Jit::new().unwrap();
    let interrupt = Interrupt::new();
    interrupt.interrupt();

    let f = jit
        .compile_interruptible(&spin(Type::Float), &interrupt)
        .unwrap();
    // SAFETY: the signature is `fn() -> float`, and `f` outlives the call.
    let run: extern "C" fn() -> f64 = unsafe { f.entry() };
    assert_eq!(run().to_bits(), 0.0_f64.to_bits());

    let f = jit
        .compile_interruptible(&spin(Type::Bool), &interrupt)
        .unwrap();
    // SAFETY: the signature is `fn() -> bool`, and `f` outlives the call.
    let run: extern "C" fn() -> bool = unsafe { f.entry() };
    assert!(!run());

    let f = jit
        .compile_interruptible(&spin(Type::Unit), &interrupt)
        .unwrap();
    // SAFETY: the signature is `fn()`, and `f` outlives the call.
    let run: extern "C" fn() = unsafe { f.entry() };
    run();
}

#[test]
fn test_clear_flag_runs_loops_to_completion() {
    let interrupt = Interrupt::new();
    let f = Jit::new()
        .unwrap()
        .compile_interruptible(&sum_to(), &interrupt)
        .unwrap();
    // SAFETY: the signature is `fn(int) -> int`, and `f` outlives every call.
    let run: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(run(100), 5050);

    interrupt.interrupt();
    assert_eq!(run(100), 0);

    interrupt.clear();
    assert_eq!(run(10), 55);
}

#[test]
fn test_code_without_a_loop_ignores_the_flag() {
    let mut b = Builder::new("inc", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let one = b.iconst(1);
    let sum = b.bin(BinOp::Add, x, one);
    b.ret(Some(sum));

    let interrupt = Interrupt::new();
    interrupt.interrupt();
    let f = Jit::new()
        .unwrap()
        .compile_interruptible(&b.finish(), &interrupt)
        .unwrap();
    // SAFETY: the signature is `fn(int) -> int`, and `f` outlives the call.
    let run: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(run(41), 42);
}

#[test]
fn test_compiled_keeps_the_flag_alive() {
    let interrupt = Interrupt::new();
    let f = Jit::new()
        .unwrap()
        .compile_interruptible(&sum_to(), &interrupt)
        .unwrap();
    drop(interrupt);
    // SAFETY: the signature is `fn(int) -> int`, and `f` outlives the call.
    let run: extern "C" fn(i64) -> i64 = unsafe { f.entry() };
    assert_eq!(run(4), 10);
}

#[test]
fn test_one_flag_stops_calls_on_several_threads() {
    let interrupt = Interrupt::new();
    let f = std::sync::Arc::new(
        Jit::new()
            .unwrap()
            .compile_interruptible(&spin(Type::Int), &interrupt)
            .unwrap(),
    );
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let f = std::sync::Arc::clone(&f);
            thread::spawn(move || {
                // SAFETY: the signature is `fn() -> int`, and the `Arc` keeps `f` alive.
                let run: extern "C" fn() -> i64 = unsafe { f.entry() };
                run()
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(20));
    interrupt.interrupt();
    for w in workers {
        assert_eq!(w.join().unwrap(), 0);
    }
}
