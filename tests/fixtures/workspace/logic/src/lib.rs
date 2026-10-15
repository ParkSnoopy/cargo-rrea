//! Branches, integer and floating-point constants, and Rust string fixtures.

#[derive(Debug)]
enum Value {
    Even(u32),
    Odd(u32),
}

/// Calculate a deterministic value with runtime control flow.
pub fn calculate(input: u32) -> u32 {
    let mut result = input;
    for index in 0..11 {
        result = if result & 1 == 0 {
            result.rotate_left(index).wrapping_add(0x1928_3746)
        } else {
            result.rotate_right(index).wrapping_mul(31)
        };
    }
    result
}

/// Return observable text whose formatting and embedded NUL must survive.
pub fn describe(input: u32) -> String {
    let value = if input & 1 == 0 { Value::Even(input) } else { Value::Odd(input) };
    let observed = match value { Value::Even(value) | Value::Odd(value) => value };
    let float = f64::from(std::hint::black_box(input)) * 1.375 + 0.0625;
    format!("{value:?}:{observed}:{}:{float:.4}:Rust string\0with NUL:文字", calculate(input))
}