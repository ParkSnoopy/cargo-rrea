# cargo-rrea

Build Rust release applications with Arkari obfuscation, metadata stripping, and packed shared libraries.

## Requirements

- Rust/Cargo and curl on PATH.
- Native x86_64 Linux GNU or Windows MSVC; Linux is runtime-tested, Windows is not yet runtime-verified.
- Installation requires CMake, Ninja, and a C/C++ toolchain with C++17 support.
- On Linux, the system C++ runtime must provide `libstdc++.so.6`.

## Install

Install from [crates.io](https://crates.io/crates/cargo-rrea):

```sh
cargo install cargo-rrea --locked
```

UPX is built from bundled sources during installation; no Git checkout or submodule initialization is needed.
The obfuscating toolchain is downloaded automatically when needed; no manual download is required.
An empty toolchain cache requires internet access.

## Use

Run inside the Rust project you want to build:

```sh
cargo rrea init
cargo rrea build --release
```

`init` sets release debug information to zero and enables symbol stripping in the workspace manifest.
It preserves unrelated settings and comments.
`build` requires `--release` (or `-r`); development profiles are not supported.

Select a package and executable, or build with an already cached toolchain offline:

```sh
cargo rrea build --release --package my-app --bin my-app
cargo rrea build --release --offline --bins
cargo rrea build --help
```

Executable paths are printed on stdout.
Find the complete distribution, report, and logs under your project's `target/rrea/dist-<id>/` directory, or the equivalent custom Cargo target directory.
Distribute the entire application directory, including its shared libraries—not just the executable.
Executables remain unpacked; their Rust shared libraries are packed.

## Limits

- Cross-compilation is not supported.
- Required native/system libraries are not automatically bundled.
- Rust strings and recognizable code can remain; UPX packing is reversible, not encryption.
- Only build projects you trust: their build scripts and procedural macros execute during compilation.

[Architecture and source development](docs/architecture.md)
