//! Translating a validated [`ir_lang::Function`] into Cranelift IR.
//!
//! The two representations line up closely: both are SSA control-flow graphs whose
//! values cross block boundaries as block parameters rather than through phi nodes, so
//! the translation is mostly a relabeling. Each IR [`Value`] is mapped to the Cranelift
//! value its defining instruction produces; each [`Block`] to a Cranelift block; each
//! [`Inst`] to one Cranelift instruction; and each [`Terminator`] to a jump, a
//! conditional branch, or a return.
//!
//! Blocks are emitted in reverse postorder over the reachable graph. That order visits
//! a block only after the blocks that dominate it, so when an instruction references a
//! value defined in another block, the Cranelift value for that definition has already
//! been created. Unreachable blocks are dropped: they cannot run, and a validated
//! function never lets reachable code depend on a value defined in one.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, BlockArg, Function as ClifFunction, InstBuilder, Signature, UserFuncName, types,
};
use cranelift_codegen::ir::{Block as ClifBlock, Value as ClifValue};
use cranelift_codegen::isa::TargetIsa;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use ir_lang::{BinOp, Block, Function, Inst, Terminator, Type, UnOp, Value};

use crate::error::JitError;

/// Translates `func` into a Cranelift function for `isa` to compile.
///
/// The function must already be valid — [`Jit::compile`](crate::Jit::compile) calls
/// [`Function::validate`](ir_lang::Function::validate) first — so this routine relies
/// on the SSA invariants that check guarantees and reports only what validation cannot
/// catch: a [`Unit`](Type::Unit)-typed parameter, which has no machine representation.
pub(crate) fn translate(func: &Function, isa: &dyn TargetIsa) -> Result<ClifFunction, JitError> {
    let order = reachable_rpo(func);
    let signature = signature(func, isa)?;

    let mut clif = ClifFunction::with_name_signature(UserFuncName::user(0, 0), signature);
    let mut context = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut clif, &mut context);

    // One Cranelift block per reachable IR block. The entry is `order[0]`, so its
    // Cranelift block is created first and becomes the function's entry in layout.
    let mut blocks: Vec<Option<ClifBlock>> = vec![None; func.block_count()];
    for &ir_block in &order {
        blocks[ir_block.index()] = Some(builder.create_block());
    }

    // The entry's parameters are the function's parameters, in ABI form.
    let entry = clif_block(&blocks, func.entry())?;
    builder.append_block_params_for_function_params(entry);

    // Every other reachable block gets one parameter per IR block parameter.
    for &ir_block in &order {
        if ir_block == func.entry() {
            continue;
        }
        let block = clif_block(&blocks, ir_block)?;
        for &param in func.block_params(ir_block) {
            let ty = abi_type(value_type(func, param)?)?;
            builder.append_block_param(block, ty);
        }
    }

    // Map every reachable block parameter to the Cranelift value standing in for it.
    let mut values: Vec<Option<ClifValue>> = vec![None; func.value_count()];
    for &ir_block in &order {
        let block = clif_block(&blocks, ir_block)?;
        let params = builder.block_params(block).to_vec();
        for (&ir_value, &clif_value) in func.block_params(ir_block).iter().zip(params.iter()) {
            values[ir_value.index()] = Some(clif_value);
        }
    }

    // Fill each block: its instructions in program order, then its terminator.
    for &ir_block in &order {
        let block = clif_block(&blocks, ir_block)?;
        builder.switch_to_block(block);
        for &ir_value in func.insts(ir_block) {
            if let Some(inst) = func.inst(ir_value) {
                let produced = emit_inst(&mut builder, func, inst, &values)?;
                values[ir_value.index()] = Some(produced);
            }
        }
        let term = func
            .terminator(ir_block)
            .ok_or_else(|| internal("a reachable block has no terminator"))?;
        emit_terminator(&mut builder, term, &blocks, &values)?;
    }

    builder.seal_all_blocks();
    builder.finalize();
    Ok(clif)
}

/// Builds the Cranelift signature from the IR function's parameter and return types,
/// using the host's C calling convention so the result is callable through
/// `extern "C"`.
fn signature(func: &Function, isa: &dyn TargetIsa) -> Result<Signature, JitError> {
    let mut sig = Signature::new(isa.default_call_conv());
    for &ty in func.params() {
        sig.params.push(AbiParam::new(abi_type(ty)?));
    }
    if func.ret() != Type::Unit {
        sig.returns.push(AbiParam::new(abi_type(func.ret())?));
    }
    Ok(sig)
}

/// The machine type an IR type lowers to. `int` is a 64-bit integer, `float` an
/// `f64`, and `bool` a byte holding `0` or `1`. `unit` has no machine representation;
/// it is meaningful only as a function's return type (handled in [`signature`]), so a
/// `unit`-typed value position is rejected here.
fn abi_type(ty: Type) -> Result<types::Type, JitError> {
    match ty {
        Type::Int => Ok(types::I64),
        Type::Float => Ok(types::F64),
        Type::Bool => Ok(types::I8),
        Type::Unit => Err(JitError::Unsupported(
            "a parameter or block parameter has type unit, which has no machine representation",
        )),
    }
}

/// Emits one IR instruction and returns the Cranelift value it defines.
fn emit_inst(
    builder: &mut FunctionBuilder,
    func: &Function,
    inst: &Inst,
    values: &[Option<ClifValue>],
) -> Result<ClifValue, JitError> {
    match inst {
        Inst::Iconst(value) => Ok(builder.ins().iconst(types::I64, *value)),
        Inst::Fconst(value) => Ok(builder.ins().f64const(*value)),
        Inst::Bconst(value) => Ok(builder.ins().iconst(types::I8, i64::from(*value))),
        Inst::Bin(op, lhs, rhs) => {
            let operand_ty = value_type(func, *lhs)?;
            let l = clif_value(values, *lhs)?;
            let r = clif_value(values, *rhs)?;
            Ok(emit_bin(builder, *op, l, r, operand_ty))
        }
        Inst::Un(op, operand) => {
            let operand_ty = value_type(func, *operand)?;
            let x = clif_value(values, *operand)?;
            Ok(emit_un(builder, *op, x, operand_ty))
        }
    }
}

/// Emits a binary operation, choosing the floating-point or integer instruction by the
/// operand type. Comparisons produce a `0`/`1` byte; integer ordering is signed.
fn emit_bin(
    builder: &mut FunctionBuilder,
    op: BinOp,
    l: ClifValue,
    r: ClifValue,
    operand_ty: Type,
) -> ClifValue {
    let is_float = operand_ty == Type::Float;
    match op {
        BinOp::Add if is_float => builder.ins().fadd(l, r),
        BinOp::Add => builder.ins().iadd(l, r),
        BinOp::Sub if is_float => builder.ins().fsub(l, r),
        BinOp::Sub => builder.ins().isub(l, r),
        BinOp::Mul if is_float => builder.ins().fmul(l, r),
        BinOp::Mul => builder.ins().imul(l, r),
        BinOp::Div if is_float => builder.ins().fdiv(l, r),
        BinOp::Div => builder.ins().sdiv(l, r),
        BinOp::Eq if is_float => builder.ins().fcmp(FloatCC::Equal, l, r),
        BinOp::Eq => builder.ins().icmp(IntCC::Equal, l, r),
        BinOp::Ne if is_float => builder.ins().fcmp(FloatCC::NotEqual, l, r),
        BinOp::Ne => builder.ins().icmp(IntCC::NotEqual, l, r),
        BinOp::Lt if is_float => builder.ins().fcmp(FloatCC::LessThan, l, r),
        BinOp::Lt => builder.ins().icmp(IntCC::SignedLessThan, l, r),
        BinOp::Le if is_float => builder.ins().fcmp(FloatCC::LessThanOrEqual, l, r),
        BinOp::Le => builder.ins().icmp(IntCC::SignedLessThanOrEqual, l, r),
        BinOp::Gt if is_float => builder.ins().fcmp(FloatCC::GreaterThan, l, r),
        BinOp::Gt => builder.ins().icmp(IntCC::SignedGreaterThan, l, r),
        BinOp::Ge if is_float => builder.ins().fcmp(FloatCC::GreaterThanOrEqual, l, r),
        BinOp::Ge => builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, l, r),
        BinOp::And => builder.ins().band(l, r),
        BinOp::Or => builder.ins().bor(l, r),
    }
}

/// Emits a unary operation. Negation follows the operand type; logical not flips the
/// low bit of a `0`/`1` byte.
fn emit_un(builder: &mut FunctionBuilder, op: UnOp, x: ClifValue, operand_ty: Type) -> ClifValue {
    match op {
        UnOp::Neg if operand_ty == Type::Float => builder.ins().fneg(x),
        UnOp::Neg => builder.ins().ineg(x),
        UnOp::Not => builder.ins().bxor_imm(x, 1),
    }
}

/// Emits a block's terminator: a return, an unconditional jump, or a conditional
/// branch. A branch tests the condition byte and takes the `then` block when it is
/// nonzero. Each edge carries one argument per target block parameter.
fn emit_terminator(
    builder: &mut FunctionBuilder,
    term: &Terminator,
    blocks: &[Option<ClifBlock>],
    values: &[Option<ClifValue>],
) -> Result<(), JitError> {
    match term {
        Terminator::Return(None) => {
            builder.ins().return_(&[]);
        }
        Terminator::Return(Some(value)) => {
            let v = clif_value(values, *value)?;
            builder.ins().return_(&[v]);
        }
        Terminator::Jump(target, args) => {
            let block = clif_block(blocks, *target)?;
            let block_args = block_args(values, args)?;
            builder.ins().jump(block, &block_args);
        }
        Terminator::Branch {
            cond,
            then_block,
            then_args,
            else_block,
            else_args,
        } => {
            let condition = clif_value(values, *cond)?;
            let then_b = clif_block(blocks, *then_block)?;
            let else_b = clif_block(blocks, *else_block)?;
            let then_a = block_args(values, then_args)?;
            let else_a = block_args(values, else_args)?;
            builder
                .ins()
                .brif(condition, then_b, &then_a, else_b, &else_a);
        }
    }
    Ok(())
}

/// Resolves the IR arguments on a control-flow edge to the Cranelift block arguments
/// the jump or branch carries.
fn block_args(values: &[Option<ClifValue>], args: &[Value]) -> Result<Vec<BlockArg>, JitError> {
    args.iter()
        .map(|&arg| clif_value(values, arg).map(BlockArg::from))
        .collect()
}

/// The Cranelift value standing in for an IR value, or an internal error if it has not
/// been emitted yet — which the reverse-postorder walk and SSA dominance rule out.
fn clif_value(values: &[Option<ClifValue>], value: Value) -> Result<ClifValue, JitError> {
    values
        .get(value.index())
        .copied()
        .flatten()
        .ok_or_else(|| internal("a value was referenced before it was defined"))
}

/// The Cranelift block standing in for a reachable IR block, or an internal error if
/// the target is unreachable — which a reachable terminator never names.
fn clif_block(blocks: &[Option<ClifBlock>], block: Block) -> Result<ClifBlock, JitError> {
    blocks
        .get(block.index())
        .copied()
        .flatten()
        .ok_or_else(|| internal("a reachable terminator named an unreachable block"))
}

/// An IR value's type, mapping an out-of-range handle to an internal error. Validation
/// rules this out for any value the function references.
fn value_type(func: &Function, value: Value) -> Result<Type, JitError> {
    func.value_type(value)
        .ok_or_else(|| internal("a referenced value has no recorded type"))
}

/// Reports a broken translator invariant. These cannot arise from a validated function
/// the translator accepts; surfacing them as an error keeps the path total rather than
/// panicking, and the message names the invariant for a bug report.
fn internal(what: &str) -> JitError {
    JitError::Codegen(format!("internal translator invariant violated: {what}"))
}

/// The reachable blocks of `func` in reverse postorder, entry first. A block appears
/// only after every block that dominates it, so a definition is always seen before its
/// uses. Unreachable blocks are omitted.
fn reachable_rpo(func: &Function) -> Vec<Block> {
    let n = func.block_count();
    let entry = func.entry();
    let mut order = Vec::new();
    if entry.index() >= n {
        return order;
    }

    let mut visited = vec![false; n];
    // Each frame is (block, its successors, index of the next successor to visit). An
    // explicit stack keeps a deep graph from overflowing the call stack.
    let mut stack: Vec<(Block, Vec<Block>, usize)> = Vec::new();
    visited[entry.index()] = true;
    stack.push((entry, succs_of(func, entry), 0));

    while !stack.is_empty() {
        let top = stack.len() - 1;
        let (block, next) = (stack[top].0, stack[top].2);
        if next < stack[top].1.len() {
            let succ = stack[top].1[next];
            stack[top].2 += 1;
            if succ.index() < n && !visited[succ.index()] {
                visited[succ.index()] = true;
                let succs = succs_of(func, succ);
                stack.push((succ, succs, 0));
            }
        } else {
            order.push(block);
            let _ = stack.pop();
        }
    }

    order.reverse();
    order
}

/// The successor blocks of `block`, read from its terminator.
fn succs_of(func: &Function, block: Block) -> Vec<Block> {
    let mut out = Vec::new();
    if let Some(term) = func.terminator(block) {
        term.each_successor(|s| out.push(s));
    }
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests build known-valid functions and assert on specific outcomes"
)]
mod tests {
    use super::translate;
    use crate::JitError;
    use cranelift_codegen::settings::{self, Configurable};
    use cranelift_codegen::verifier::verify_function;
    use ir_lang::{BinOp, Builder, Type};

    fn host_isa() -> cranelift_codegen::isa::OwnedTargetIsa {
        let mut fb = settings::builder();
        fb.set("opt_level", "speed").unwrap();
        fb.set("is_pic", "false").unwrap();
        let flags = settings::Flags::new(fb);
        cranelift_native::builder()
            .expect("host is a supported target")
            .finish(flags)
            .expect("flags are valid")
    }

    #[test]
    fn test_straight_line_translation_verifies() {
        // fn double(x: int) -> int { x + x }
        let mut b = Builder::new("double", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let sum = b.bin(BinOp::Add, x, x);
        b.ret(Some(sum));
        let isa = host_isa();
        let clif = translate(&b.finish(), &*isa).expect("translates");
        assert_eq!(clif.signature.params.len(), 1);
        assert_eq!(clif.signature.returns.len(), 1);
        verify_function(&clif, &*isa).expect("the translated function is well-formed CLIF");
    }

    #[test]
    fn test_diamond_with_block_parameter_verifies() {
        // fn max(a: int, b: int) -> int { if a < b { b } else { a } }
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
        let isa = host_isa();
        let clif = translate(&b.finish(), &*isa).expect("translates");
        verify_function(&clif, &*isa).expect("the translated function is well-formed CLIF");
    }

    #[test]
    fn test_unit_parameter_is_unsupported() {
        // A unit-typed parameter has no machine representation.
        let mut b = Builder::new("f", &[Type::Unit], Type::Unit);
        b.ret(None);
        let isa = host_isa();
        assert!(matches!(
            translate(&b.finish(), &*isa),
            Err(JitError::Unsupported(_))
        ));
    }
}
