//! Compile a function at run time and call it.
//!
//! Build `fn add(a: int, b: int) -> int { a + b }` with the IR builder, compile it to
//! native machine code, and call the result. This is the crate in miniature.
//!
//! ```bash
//! cargo run --example jit_and_call
//! ```

use ir_lang::{BinOp, Builder, Type};
use jit_lang::compile;

fn main() {
    // fn add(a: int, b: int) -> int { a + b }
    let mut b = Builder::new("add", &[Type::Int, Type::Int], Type::Int);
    let a = b.block_params(b.entry())[0];
    let c = b.block_params(b.entry())[1];
    let sum = b.bin(BinOp::Add, a, c);
    b.ret(Some(sum));

    let f = compile(&b.finish()).expect("add is well-formed");
    println!(
        "compiled {}({} params) -> {:?}, {} bytes of code",
        f.name(),
        f.params().len(),
        f.ret(),
        f.code_len(),
    );

    // SAFETY: the signature is `fn(int, int) -> int`, and `f` outlives every call below.
    let add: extern "C" fn(i64, i64) -> i64 = unsafe { f.entry() };
    for (x, y) in [(19, 23), (1, 1), (-5, 5)] {
        println!("add({x}, {y}) = {}", add(x, y));
    }
    assert_eq!(add(19, 23), 42);
}
