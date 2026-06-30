//! Error-path tests: a compile that should fail does, with the right reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests assert on specific error variants; a wrong one should fail loudly"
)]

use ir_lang::{Builder, Type};
use jit_lang::{JitError, compile};
use std::error::Error;

#[test]
fn test_missing_terminator_is_rejected_as_invalid_ir() {
    // The entry block never receives a terminator.
    let func = Builder::new("f", &[], Type::Unit).finish();
    match compile(&func) {
        Err(JitError::InvalidIr(reason)) => assert!(reason.to_string().contains("terminator")),
        other => panic!("expected InvalidIr, got {other:?}"),
    }
}

#[test]
fn test_return_type_mismatch_is_rejected_as_invalid_ir() {
    // Declares an int return but returns nothing.
    let mut b = Builder::new("bad", &[], Type::Int);
    b.ret(None);
    assert!(matches!(compile(&b.finish()), Err(JitError::InvalidIr(_))));
}

#[test]
fn test_unit_parameter_is_unsupported() {
    // A unit-typed parameter has no value at the machine level.
    let mut b = Builder::new("f", &[Type::Unit], Type::Unit);
    b.ret(None);
    match compile(&b.finish()) {
        Err(JitError::Unsupported(msg)) => assert!(msg.contains("unit")),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn test_invalid_ir_error_exposes_its_source() {
    let func = Builder::new("f", &[], Type::Int).finish(); // missing terminator
    let err = compile(&func).unwrap_err();
    assert!(err.source().is_some());
    assert!(err.to_string().contains("not well-formed"));
}
