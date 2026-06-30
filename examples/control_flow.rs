//! Compile and run functions with branches and a loop.
//!
//! `abs` lowers a two-way branch; `sum_to` lowers a loop whose header carries the
//! running state as block parameters — the SSA stand-in for a phi node. Both compile to
//! straight native control flow.
//!
//! ```bash
//! cargo run --example control_flow
//! ```

use ir_lang::{BinOp, Builder, Type, UnOp};
use jit_lang::Jit;

fn main() {
    let jit = Jit::new().expect("the host is supported");

    // fn abs(x: int) -> int { if x < 0 { -x } else { x } }
    let abs = {
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

        jit.compile(&b.finish()).expect("abs is well-formed")
    };

    // SAFETY: the signature is `fn(int) -> int`, and `abs` outlives every call.
    let abs_fn: extern "C" fn(i64) -> i64 = unsafe { abs.entry() };
    for x in [-7, 0, 7] {
        println!("abs({x}) = {}", abs_fn(x));
    }

    // fn sum_to(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }
    let sum_to = {
        let mut b = Builder::new("sum_to", &[Type::Int], Type::Int);
        let n0 = b.block_params(b.entry())[0];
        let header = b.create_block(&[Type::Int, Type::Int]); // (n, acc)
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

        jit.compile(&b.finish()).expect("sum_to is well-formed")
    };

    // SAFETY: the signature is `fn(int) -> int`, and `sum_to` outlives every call.
    let sum_to_fn: extern "C" fn(i64) -> i64 = unsafe { sum_to.entry() };
    for n in [5, 10, 100] {
        println!("sum_to({n}) = {}", sum_to_fn(n));
    }
    assert_eq!(sum_to_fn(100), 5050);
}
