//! Transitive no_std dependency shared by the fixture executables.

#![no_std]

// Keep observable read-only data in the tightly linked, packed shared library.
static LOOKUP: [u32; 4096] = [0x1928_3746; 4096];

/// Calculate a deterministic value with runtime control flow.
#[inline(never)]
pub fn calculate(input: u32) -> u32 {
    let mut result = input;
    for index in 0..11 {
        result = if result & 1 == 0 {
            result.rotate_left(index).wrapping_add(
                core::hint::black_box(&LOOKUP)[input as usize % LOOKUP.len()],
            )
        } else {
            result.rotate_right(index).wrapping_mul(31)
        };
    }
    result
}
