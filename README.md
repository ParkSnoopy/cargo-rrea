# cargo-rrea

Build Rust release applications with Arkari obfuscation, metadata stripping, and UPX packing.

## Requirements

- Rust/Cargo, Git, and curl on PATH.
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

`build --release` enables all implemented obfuscation features by default, including library and ELF Rust symbol mangling.  
Individual `--no-<feature>` switches disable only that feature, such as `--no-fla`, `--no-mangle-symbols`, or `--no-upx`.  
The complete [feature contract](docs/architecture.md#obfuscation-policy) defines pass selection, stripping, path remapping, and packing.  
Stripping temporarily disables debug information and removes metadata without editing `Cargo.toml`; `--no-strip` retains the project's debug/strip settings.  
`build` requires `--release` (or `-r`); development profiles are not supported.  
Release builds automatically use `panic = "unwind"` to match the shared runtime, even when the project requests `abort`.  
This changes panic behavior without editing `Cargo.toml`; no environment-variable prefix is needed.  
The default follows the [closed-bundle symbol contract](docs/rust-fingerprints.md#closed-bundle-symbol-map).  
Externally consumed or runtime-looked-up Rust names require `--keep-symbol NAME`, unless symbol mangling is disabled.  
Windows currently requires `--no-mangle-symbols`; unsupported default features fail before building rather than silently weakening protection.  

Select a package and executable, or build with an already cached toolchain offline:

```sh
cargo rrea build --release --package my-app --bin my-app
cargo rrea build --release --offline --bins
cargo rrea build --help
```

Executable paths are printed on stdout.
The distribution contains only selected executables and required Rust shared libraries in `target/release/dist/`, beside the selected `Cargo.toml`.
Native-linking crates and their Rust consumers retain rlib linkage for correct native symbol resolution; their Rust code still receives the selected obfuscation passes.
This destination does not follow a custom Cargo target directory.
For a virtual workspace manifest, the destination is beneath the workspace root.
Distribute the complete `dist` directory.
Reports and logs stay in Cargo's target directory under `rrea/dist-<id>/`, outside the distribution.
The pipeline packs executables and shared libraries unless `--no-upx` is selected.  
The bundled UPX patch removes known identification banners, PE section labels, and named runtime mappings, and changes packer/stub tags together.
The matching patched packer can decompress the output; stock UPX does not support its modified format.

## Limits

- Cross-compilation is not supported.
- Required native/system libraries are not automatically bundled.
- Rust strings and recognizable code can remain; UPX packing is reversible, not encryption.
- Removing known packer identifiers does not prevent structural recognition or manual unpacking.
- Modified UPX requires GPL-compatible programs under the [upstream license](thirdparty/upx/LICENSE).
- Only build projects you trust: their build scripts and procedural macros execute during compilation.

[Architecture and source development](docs/architecture.md)

[Reverse-engineering comparison](docs/comparison.md)

[Measured evaluation and remaining work](docs/evaluation.md)  
[UPX overlay maintenance](docs/upx-maintenance.md)  
[Rust fingerprint policy and symbol renaming](docs/rust-fingerprints.md)  
