# cargo-rrea

Build Rust release applications with Arkari obfuscation, metadata stripping, and UPX packing.

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
cargo rrea build --release
```

`build` temporarily disables debug information and strips symbols without editing `Cargo.toml`.  
`build` requires `--release` (or `-r`); development profiles are not supported.

Select a package and executable, or build with an already cached toolchain offline:

```sh
cargo rrea build --release --package my-app --bin my-app
cargo rrea build --release --offline --bins
cargo rrea build --help
```

Executable paths are printed on stdout.
The distribution contains only selected executables and required Rust shared libraries in `target/release/dist/`, beside the selected `Cargo.toml`.
This destination does not follow a custom Cargo target directory.
For a virtual workspace manifest, the destination is beneath the workspace root.
Distribute the complete `dist` directory.
Reports and logs stay in Cargo's target directory under `rrea/dist-<id>/`, outside the distribution.
The pipeline packs executables and shared libraries.
Their UPX header magic is `RREA`, so normal UPX decompression fails.

## Limits

- Cross-compilation is not supported.
- Required native/system libraries are not automatically bundled.
- Rust strings and recognizable code can remain; UPX packing is reversible, not encryption.
- Replacing the header magic is reversible and does not prevent manual unpacking.
- Only build projects you trust: their build scripts and procedural macros execute during compilation.

[Architecture and source development](docs/architecture.md)
