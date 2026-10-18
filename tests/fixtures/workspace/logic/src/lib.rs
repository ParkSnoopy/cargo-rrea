//! Branches, integer and floating-point constants, and Rust string fixtures.

#![no_std]

extern crate alloc;

use alloc::{format, string::String};

// Zero-filled padding stays read-only rather than becoming encrypted writable data.
static OFFSETS: [f64; 2048] = [0.0; 2048];

#[derive(Debug)]
enum Value {
    Even(u32),
    Odd(u32),
}

/// Calculate a deterministic value with runtime control flow.
pub fn calculate(input: u32) -> u32 {
    fixture_shared::calculate(input)
}

/// Return observable text whose formatting and embedded NUL must survive.
pub fn describe(input: u32) -> String {
    let value = if input & 1 == 0 { Value::Even(input) } else { Value::Odd(input) };
    let observed = match value { Value::Even(value) | Value::Odd(value) => value };
    let float = f64::from(core::hint::black_box(input)) * 1.375
        + core::hint::black_box(&OFFSETS)[input as usize % OFFSETS.len()];
    format!("{value:?}:{observed}:{}:{float:.4}:Rust string\0with NUL:文字", calculate(input))
}