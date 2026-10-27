# cargo-rrea

Build Rust release applications with Arkari obfuscation, metadata stripping, and UPX packing.  

## Requirements

| Requirement | Details |
| --- | --- |
| Platform | Native x86_64 Linux GNU or Windows MSVC |
| Tools | Rust/Cargo, Git, and curl on PATH |
| Native build | CMake, Ninja, and a C/C++ toolchain with C++17 support |
| Linux runtime | `libstdc++.so.6` |

Linux application builds are runtime-tested.  
Windows UPX execution is CI-tested, but the complete Windows build pipeline remains unverified.  

## Install

Install from [crates.io](https://crates.io/crates/cargo-rrea):  

```sh
cargo install cargo-rrea --locked
```

Installation builds UPX from bundled sources.  
The obfuscating compiler is downloaded on demand, so an empty toolchain cache requires internet access.  

## Use

Run from the target Rust project:  

```sh
cargo rrea build --release
```

Release builds enable every implemented obfuscation feature, including library filename and ELF Rust symbol mangling.  

| Argument | Effect |
| --- | --- |
| `--release`, `-r` | Required. Development profiles are not supported. |
| `--no-<feature>` | Disable one feature, such as `--no-fla` or `--no-upx`. See the [feature contract](docs/architecture.md#obfuscation-policy). |
| `--no-mangle symbol` | Keep Rust dynamic symbol names. |
| `--no-mangle library` | Keep library filenames. |
| `--no-mangle` | Disable both mangling policies. |
| `--keep-symbol NAME` | Preserve an externally consumed or runtime-looked-up Rust name under the [closed-bundle contract](docs/rust-fingerprints.md#closed-bundle-symbol-map). |
| `--no-strip` | Retain the project's debug and strip settings. |

Builds use `panic = "unwind"` to match the shared runtime, even when the project requests `abort`.  
Profile overrides apply to the build process rather than `Cargo.toml`.  

**Windows:** disable symbol mangling with `--no-mangle symbol` or bare `--no-mangle`.  

Package selection, offline builds, and the full argument reference:  

```sh
cargo rrea build --release --package my-app --bin my-app
cargo rrea build --release --offline --bins
cargo rrea build --help
```

## Output

| Output | Location or contents |
| --- | --- |
| Executable paths | stdout |
| Executables | `target/release/` beside the selected `Cargo.toml` |
| Linux shared libraries | `target/release/libs/` |
| Build reports and logs | `rrea/dist-<id>/` under Cargo's target directory |

Distribute the executables together with the complete `libs` directory on Linux; Windows DLLs remain beside the executables.  
A virtual workspace manifest places these outputs beneath the workspace root.  
Custom Cargo target directories affect builds and reports, not this destination.  

Native-linking crates and their Rust consumers retain rlib linkage.  
Their Rust code still receives the selected obfuscation passes.  

## Packing

UPX is built from the pinned [ParkSnoopy/upx](https://github.com/ParkSnoopy/upx) fork.  

- Linux and Windows x86_64 executables and shared libraries use `--opaque` by default.  
- `--no-opaque` selects normal compression for all bundled artifacts.  
- `--no-upx` disables packing for both executables and libraries.  

Recovery requires the matching patched packer because stock UPX does not support the modified format.  

## Limits

- Cross-compilation is not supported.  
- Required native/system libraries are not automatically bundled.  
- Rust strings and recognizable code can remain. UPX packing is reversible, not encryption.  
- Changed packer identifiers do not prevent structural recognition or manual unpacking.  
- The fork's Linux startup guard can terminate a packed program when an analysis-tool process name is visible, even if that tool is unrelated.  
- Modified UPX requires GPL-compatible programs under the [upstream license](thirdparty/upx/LICENSE).  
- Only build projects you trust: their build scripts and procedural macros execute during compilation.  

## Documentation

- [Architecture and source development](docs/architecture.md)  
- [Rust fingerprint policy and symbol renaming](docs/rust-fingerprints.md)  
- [UPX fork maintenance](docs/upx-maintenance.md)  
- [Native evaluation](docs/evaluation.md) and [recorded measurements](docs/evaluation.json)  
- [Reverse-engineering comparison](docs/comparison.md)  
- [Evaluation and protection follow-ups](docs/todo.md)  
