//! Rust consumer that must share the executable's single native archive.

#![no_std]

/// Advance the native counter; callers must serialize access.
#[inline(never)]
pub unsafe fn next(input: u32) -> u32 {
    // SAFETY: the caller guarantees single-threaded access to the native counter.
    unsafe { fixture_native::fixture_native_next(input) }
}
