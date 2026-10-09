//! Instruction-cache coherence for freshly written code.
//!
//! Writing machine code into memory and then executing it is only correct once the
//! processor's instruction fetch sees the bytes the write left in the data cache. On
//! x86 and x86-64 the instruction and data caches are unified, so that happens for
//! free. On architectures with split caches — AArch64 — the data cache must be cleaned
//! and the instruction cache invalidated over the new code first, or the fetcher may
//! read stale bytes and run garbage.
//!
//! [`synchronize`] does this at the point the code is made executable: a no-op where
//! the caches are already coherent, and the platform's correct primitive where they
//! are not. The supported hosts are x86-64 and AArch64; on any other architecture no
//! `sync` is defined, so the crate fails to compile rather than run code with unproven
//! cache behavior.

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

/// AArch64 has split caches, so the instruction cache must be synchronized over the
/// new code. macOS exposes the correct user-space primitive for this — the raw
/// cache-maintenance instructions (`ic ivau` and friends) fault from user space on
/// Apple silicon — and Windows exposes `FlushInstructionCache`; other systems provide
/// the compiler runtime's `__clear_cache`, which the kernel emulates where those
/// instructions are trapped. (MSVC's runtime has no `__clear_cache`, which is why
/// Windows needs its own arm: the symbol would not link.)
#[cfg(target_arch = "aarch64")]
fn sync(code: *const u8, len: usize) {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            /// libkern: flush the instruction cache over `[start, start + len)`.
            fn sys_icache_invalidate(start: *const core::ffi::c_void, len: usize);
        }
        // SAFETY: `code`/`len` describe the just-written, now-readable code region. The
        // call only reads that range to synchronize caches over it; passing a valid
        // pointer and its length is the whole contract.
        unsafe { sys_icache_invalidate(code.cast(), len) };
    }

    #[cfg(windows)]
    {
        use core::ffi::c_void;

        #[allow(
            non_snake_case,
            reason = "these are foreign Win32 symbols and must keep their exported names"
        )]
        unsafe extern "system" {
            /// kernel32: the pseudo-handle for the calling process (always valid, never
            /// closed).
            fn GetCurrentProcess() -> *mut c_void;
            /// kernel32: flush the instruction cache of `process` over
            /// `[base, base + size)`. Returns nonzero on success.
            fn FlushInstructionCache(process: *mut c_void, base: *const c_void, size: usize)
            -> i32;
        }
        // SAFETY: `GetCurrentProcess` takes no arguments and returns a constant
        // pseudo-handle for this process. `FlushInstructionCache` only performs cache
        // maintenance over `[code, code + len)`, the just-written, now-executable code
        // region this process owns; the handle, base pointer and length are all valid,
        // which is its whole contract. kernel32 is always linked on Windows (as for
        // pager-lang's `VirtualAlloc`).
        // Failure is only possible for an invalid handle or range, neither of which can
        // occur here, so the BOOL result carries no information worth acting on.
        let _ = unsafe { FlushInstructionCache(GetCurrentProcess(), code.cast(), len) };
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        unsafe extern "C" {
            /// Compiler runtime: synchronize caches over the half-open range `[start, end)`.
            fn __clear_cache(start: *mut core::ffi::c_char, end: *mut core::ffi::c_char);
        }
        let start = code as *mut core::ffi::c_char;
        let end = start.wrapping_add(len);
        // SAFETY: `[code, code + len)` is the just-written code region, readable and
        // executable. `__clear_cache` only performs cache maintenance over that range;
        // the pointers bound one allocation and `end` is one-past-the-end, as required.
        unsafe { __clear_cache(start, end) };
    }
}
