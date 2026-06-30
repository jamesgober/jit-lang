//! The error type a compile can fail with, and the conversions that build it.

use std::error::Error;
use std::fmt;

use ir_lang::ValidationError;
use pager_lang::PagerError;

/// The reason a function could not be compiled and made executable.
///
/// A compile runs three stages, and each contributes a variant: the input is
/// validated as well-formed SSA, translated and lowered to machine code, then placed
/// in executable memory. [`InvalidIr`](JitError::InvalidIr) reports the first stage
/// rejecting malformed input; [`Unsupported`](JitError::Unsupported) reports a
/// function that is valid but uses something this back-end does not lower, or a host
/// the code generator cannot target; [`Codegen`](JitError::Codegen) reports the code
/// generator itself failing; and [`Memory`](JitError::Memory) reports executable
/// memory being unavailable.
///
/// The enum is `#[non_exhaustive]`: a future release may add a variant without it
/// being a breaking change, so a `match` on it must keep a wildcard arm. It implements
/// [`Display`](fmt::Display) and [`std::error::Error`], and converts from the two
/// wrapped error types with [`From`], so `?` propagates them.
///
/// # Examples
///
/// ```
/// use jit_lang::{compile, JitError};
/// use ir_lang::{Builder, Type};
///
/// // A function whose entry block never gets a terminator is not well-formed.
/// let func = Builder::new("f", &[], Type::Unit).finish();
/// match compile(&func) {
///     Err(JitError::InvalidIr(reason)) => {
///         assert!(reason.to_string().contains("terminator"));
///     }
///     other => panic!("expected InvalidIr, got {other:?}"),
/// }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum JitError {
    /// The function did not pass [`Function::validate`](ir_lang::Function::validate);
    /// the wrapped [`ValidationError`] names the offending block or value. The code
    /// generator is never handed input the IR already forbids.
    InvalidIr(ValidationError),

    /// The function is well-formed but uses something this back-end cannot lower, or
    /// the host architecture has no code generator. The message says which — for
    /// example a parameter typed [`Unit`](ir_lang::Type::Unit), which has no value at
    /// the machine level, or an unsupported target CPU.
    Unsupported(&'static str),

    /// The native code generator rejected the translated function. This does not
    /// happen for a validated function the translator accepts; it is the surfaced
    /// form of an internal code-generation failure, carried as the generator's own
    /// message rather than swallowed.
    Codegen(String),

    /// Executable memory for the compiled code could not be obtained or protected;
    /// the wrapped [`PagerError`] says why (the mapping failed, or the
    /// write-then-execute protection change was refused).
    Memory(PagerError),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JitError::InvalidIr(err) => write!(f, "the function is not well-formed: {err}"),
            JitError::Unsupported(what) => write!(f, "unsupported: {what}"),
            JitError::Codegen(msg) => write!(f, "code generation failed: {msg}"),
            JitError::Memory(err) => write!(f, "executable memory unavailable: {err}"),
        }
    }
}

impl Error for JitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            JitError::InvalidIr(err) => Some(err),
            JitError::Memory(err) => Some(err),
            JitError::Unsupported(_) | JitError::Codegen(_) => None,
        }
    }
}

impl From<ValidationError> for JitError {
    fn from(err: ValidationError) -> Self {
        JitError::InvalidIr(err)
    }
}

impl From<PagerError> for JitError {
    fn from(err: PagerError) -> Self {
        JitError::Memory(err)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests assert on specific variants; a wrong one should fail loudly"
)]
mod tests {
    use super::JitError;
    use ir_lang::{Builder, Type};
    use std::error::Error;

    #[test]
    fn test_invalid_ir_reports_the_validation_reason_as_source() {
        let func = Builder::new("f", &[], Type::Unit).finish(); // no terminator
        let err: JitError = func.validate().unwrap_err().into();
        assert!(matches!(err, JitError::InvalidIr(_)));
        assert!(err.source().is_some());
        assert!(err.to_string().contains("not well-formed"));
    }

    #[test]
    fn test_unsupported_carries_its_message_and_has_no_source() {
        let err = JitError::Unsupported("a parameter has type unit");
        assert!(err.to_string().contains("unit"));
        assert!(err.source().is_none());
    }

    #[test]
    fn test_codegen_message_is_preserved() {
        let err = JitError::Codegen("verifier error".to_string());
        assert!(err.to_string().contains("verifier error"));
        assert!(err.source().is_none());
    }
}
