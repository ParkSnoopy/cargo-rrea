//! Generate a value to exercise host build-script separation.

fn main() {
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(output.join("generated.rs"), "const GENERATED: u32 = 17;\n")
        .expect("write generated source");
}