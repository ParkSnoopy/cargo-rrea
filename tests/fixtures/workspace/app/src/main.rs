//! Functional fixture for obfuscation and packing.

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

fn main() {
    let argument = std::env::args().nth(1).unwrap_or_else(|| "7".to_owned());
    match argument.as_str() {
        "--panic" => panic!("fixture panic marker"),
        "--catch" => {
            let result = std::panic::catch_unwind(|| panic!("caught fixture panic"));
            println!("caught={}", result.is_err());
        }
        _ => {
            let input: u32 = argument.parse().expect("integer input");
            println!("{}", fixture_logic::describe(std::hint::black_box(input)));
            println!("{}:{GENERATED}", fixture_literal::message!());
        }
    }
}