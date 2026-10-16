//! A second selected executable.

static INPUTS: [u32; 4096] = [19; 4096];

fn main() {
    println!("secondary:{}", fixture_logic::calculate(std::hint::black_box(&INPUTS)[0]));
}