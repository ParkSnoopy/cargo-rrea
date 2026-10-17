//! Build a native archive whose public functions are only called downstream.

use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("native");
    let configuration = if env::var("DEBUG").expect("DEBUG") == "true" {
        "Debug"
    } else {
        "Release"
    };
    assert!(
        Command::new("cmake")
            .args(["-G", "Ninja", "-S"])
            .arg(root)
            .arg("-B")
            .arg(&output)
            .arg(format!("-DCMAKE_BUILD_TYPE={configuration}"))
            .status()
            .expect("configure native fixture")
            .success()
    );
    assert!(
        Command::new("cmake")
            .arg("--build")
            .arg(&output)
            .args(["--target", "fixture_native"])
            .status()
            .expect("build native fixture")
            .success()
    );
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=fixture_native");
    println!("cargo:rerun-if-changed=native.c");
    println!("cargo:rerun-if-changed=CMakeLists.txt");
}
