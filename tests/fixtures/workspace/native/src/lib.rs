//! Bindings-only native dependency, including shared mutable C state.

#![no_std]

unsafe extern "C" {
    /// Advance the shared counter; callers must serialize access.
    pub fn fixture_native_next(input: u32) -> u32;
}
