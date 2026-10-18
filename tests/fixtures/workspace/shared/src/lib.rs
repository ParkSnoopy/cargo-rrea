//! Transitive no_std dependency shared by the fixture executables.

#![no_std]

// Zero-filled padding stays read-only rather than becoming encrypted writable data.
static LOOKUP: [u32; 4096] = [0; 4096];

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
