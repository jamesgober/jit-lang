//! Instruction-cache coherence for freshly written code.
//!
//! Writing machine code into memory and then executing it is only correct once the
//! processor's instruction fetch sees the bytes the write left in the data cache. On
//! x86 and x86-64 the instruction and data caches are unified, so that happens for
//! free. On architectures with split caches — AArch64 — the data cache must be cleaned
//! and the instruction cache invalidated over the new code first, or the fetcher may
//! read stale bytes and run garbage.
//!
//! [`synchronize`] does this at the point the code is made executable. It is a no-op
//! where the caches are already coherent, and dispatches to the platform's correct
//! primitive where they are not. Every `(architecture, operating system)` combination
//! is covered by exactly one branch below.

/// Makes `len` bytes of freshly written machine code at `code` safe to execute by
/// synchronizing the instruction cache over that range.
///
/// Called after the code has been written and the region made readable and executable.
/// On a platform with coherent caches this does nothing; on one with split caches it
/// issues the clean-and-invalidate the architecture requires.
pub(crate) fn synchronize(code: *const u8, len: usize) {
    sync(code, len);
}

/// x86 and x86-64 keep the instruction and data caches coherent, so code written into
/// a region is visible to the fetcher with no further action.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn sync(_code: *const u8, _len: usize) {}

/// Apple platforms expose the correct user-space primitive for this. The raw
/// cache-maintenance instructions (`ic ivau` and friends) fault from user space on
/// Apple silicon, so `sys_icache_invalidate` is the supported path.
#[cfg(all(not(any(target_arch = "x86", target_arch = "x86_64")), target_os = "macos"))]
fn sync(code: *const u8, len: usize) {
    unsafe extern "C" {
        /// libkern: flush the instruction cache over `[start, start + len)`.
        fn sys_icache_invalidate(start: *const core::ffi::c_void, len: usize);
    }
    // SAFETY: `code`/`len` describe the just-written, now-readable code region. The
    // call only reads that range to synchronize caches over it and returns nothing to
    // uphold; passing a valid pointer and its length is the whole contract.
    unsafe { sys_icache_invalidate(code.cast(), len) };
}

/// Everywhere else with split caches (Linux and Windows on ARM64, and other non-x86
/// targets) the compiler runtime's `__clear_cache` issues the clean, invalidate, and
/// barrier sequence, which the kernel emulates where user-space cache ops are trapped.
#[cfg(all(not(any(target_arch = "x86", target_arch = "x86_64")), not(target_os = "macos")))]
fn sync(code: *const u8, len: usize) {
    unsafe extern "C" {
        /// Compiler runtime: synchronize caches over the half-open range `[start, end)`.
        fn __clear_cache(start: *mut core::ffi::c_char, end: *mut core::ffi::c_char);
    }
    let start = code as *mut core::ffi::c_char;
    let end = start.wrapping_add(len);
    // SAFETY: `[code, code + len)` is the just-written code region, readable and
    // executable. `__clear_cache` only performs cache maintenance over that range; the
    // pointers bound a single allocation and `end` is one-past-the-end, as required.
    unsafe { __clear_cache(start, end) };
}
