//! A small reference interpreter for `ir-lang` functions, shared by the integration tests.
//!
//! It is the oracle the native results are checked against, replicated from
//! codegen-lang's test support (`tests/support/{mod,ir}.rs`), the bytecode tier's
//! reference interpreter: values live in an environment indexed by IR value, block
//! arguments bind as one parallel assignment, and the operators are the bytecode
//! interpreter's — wrapping integer `add`/`sub`/`mul`/`neg` and `i64::wrapping_div`
//! for `div` (so `i64::MIN / -1 == i64::MIN`).
//!
//! One case is extended, because the bytecode reference leaves it undefined (its
//! `wrapping_div` panics): integer division by zero is `0`, the result jit-lang 1.0.1
//! documents. Everything else is the bytecode tier's semantics verbatim.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::unreachable,
    reason = "test-support code: each test crate uses a subset, and a bad value should fail loudly"
)]

use ir_lang::{BinOp, Function, Inst, Terminator, UnOp, Value as IrValue};

/// A runtime value flowing through the evaluator.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    /// A signed integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// The absence of a value.
    Unit,
}

impl Value {
    /// The integer inside, or a panic naming what was found instead.
    #[must_use]
    pub fn int(self) -> i64 {
        match self {
            Value::Int(v) => v,
            other => panic!("expected an int, found {other:?}"),
        }
    }

    fn boolean(self) -> bool {
        match self {
            Value::Bool(v) => v,
            other => panic!("expected a bool, found {other:?}"),
        }
    }
}

/// The integer division every tier is checked against: the bytecode reference's
/// `wrapping_div`, extended with `x / 0 == 0`.
#[must_use]
pub fn int_div(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a.wrapping_div(b) }
}

/// Evaluates `func` on `args` and returns what it returns. `fuel` bounds the number of
/// blocks entered so a non-terminating loop fails the test instead of hanging it.
#[must_use]
pub fn eval(func: &Function, args: &[Value], fuel: usize) -> Value {
    let mut env: Vec<Option<Value>> = vec![None; func.value_count()];
    let entry = func.entry();
    let params = func.block_params(entry);
    assert_eq!(params.len(), args.len(), "argument count mismatch");
    for (&param, &arg) in params.iter().zip(args) {
        env[param.index()] = Some(arg);
    }

    let mut block = entry;
    let mut remaining = fuel;
    loop {
        remaining = remaining
            .checked_sub(1)
            .unwrap_or_else(|| panic!("evaluation ran out of fuel ({fuel} blocks)"));

        for &value in func.insts(block) {
            let inst = func.inst(value).expect("a listed instruction resolves");
            let result = match *inst {
                Inst::Iconst(v) => Value::Int(v),
                Inst::Fconst(v) => Value::Float(v),
                Inst::Bconst(v) => Value::Bool(v),
                Inst::Bin(op, lhs, rhs) => binary(op, read(&env, lhs), read(&env, rhs)),
                Inst::Un(op, src) => unary(op, read(&env, src)),
            };
            env[value.index()] = Some(result);
        }

        match func
            .terminator(block)
            .expect("a validated block is terminated")
        {
            Terminator::Return(value) => return value.map_or(Value::Unit, |v| read(&env, v)),
            Terminator::Jump(target, args) => {
                enter(func, &mut env, *target, args);
                block = *target;
            }
            Terminator::Branch {
                cond,
                then_block,
                then_args,
                else_block,
                else_args,
            } => {
                let (target, args) = if read(&env, *cond).boolean() {
                    (*then_block, then_args)
                } else {
                    (*else_block, else_args)
                };
                enter(func, &mut env, target, args);
                block = target;
            }
        }
    }
}

/// Binds `args` to `target`'s parameters as one parallel assignment.
fn enter(func: &Function, env: &mut [Option<Value>], target: ir_lang::Block, args: &[IrValue]) {
    let incoming: Vec<Value> = args.iter().map(|&a| read(env, a)).collect();
    for (&param, value) in func.block_params(target).iter().zip(incoming) {
        env[param.index()] = Some(value);
    }
}

fn read(env: &[Option<Value>], value: IrValue) -> Value {
    env[value.index()].unwrap_or_else(|| panic!("{value} read before it was defined"))
}

fn binary(op: BinOp, lhs: Value, rhs: Value) -> Value {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => arithmetic(op, lhs, rhs),
        BinOp::Eq => Value::Bool(lhs == rhs),
        BinOp::Ne => Value::Bool(lhs != rhs),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => ordering(op, lhs, rhs),
        BinOp::And => Value::Bool(lhs.boolean() && rhs.boolean()),
        BinOp::Or => Value::Bool(lhs.boolean() || rhs.boolean()),
    }
}

fn arithmetic(op: BinOp, lhs: Value, rhs: Value) -> Value {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => Value::Int(match op {
            BinOp::Add => a.wrapping_add(b),
            BinOp::Sub => a.wrapping_sub(b),
            BinOp::Mul => a.wrapping_mul(b),
            BinOp::Div => int_div(a, b),
            _ => unreachable!(),
        }),
        (Value::Float(a), Value::Float(b)) => Value::Float(match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
            _ => unreachable!(),
        }),
        other => panic!("arithmetic on mismatched operands: {other:?}"),
    }
}

fn ordering(op: BinOp, lhs: Value, rhs: Value) -> Value {
    let result = match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => compare(op, a, b),
        (Value::Float(a), Value::Float(b)) => compare(op, a, b),
        other => panic!("comparison on mismatched operands: {other:?}"),
    };
    Value::Bool(result)
}

fn compare<T: PartialOrd>(op: BinOp, a: T, b: T) -> bool {
    match op {
        BinOp::Lt => a < b,
        BinOp::Le => a <= b,
        BinOp::Gt => a > b,
        BinOp::Ge => a >= b,
        _ => unreachable!(),
    }
}

fn unary(op: UnOp, operand: Value) -> Value {
    match op {
        UnOp::Neg => match operand {
            Value::Int(v) => Value::Int(v.wrapping_neg()),
            Value::Float(v) => Value::Float(-v),
            other => panic!("negation of a non-numeric value: {other:?}"),
        },
        UnOp::Not => Value::Bool(!operand.boolean()),
    }
}
