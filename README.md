# cargo-rrea

Cargo release pipeline with Arkari LLVM obfuscation, metadata stripping, and UPX-packed Rust shared libraries.
Executables remain unpacked.

## Build contract

- `cargo rrea` and direct `cargo-rrea` invocation select the same pipeline.
- `init` changes only the workspace root manifest's release `debug` and `strip` values.
  It preserves unrelated settings, comments, and the panic strategy.
  Repeated initialization does not rewrite unchanged content.
- Builds select the cached Arkari compiler and matching sysroot without changing the active Rust installation.
  A compiler failure stops the build without ordinary-rustc fallback.
- Selected executable packages' own libraries and first-level normal Rust dependencies also produce Rust `dylib` artifacts.
  Compiler arguments link consumers against these artifacts.
  Build scripts and procedural macros remain host-side.
- The pipeline strips nonessential metadata, packs shared libraries and the compiler's dynamic standard library, and leaves executables unpacked.
  Linux executables locate adjacent libraries through `$ORIGIN`.
- Publication follows stripping, metadata audits, UPX integrity checks, and decompression audits in a staging directory.
  Failed builds do not replace existing distribution directories.

## Source ownership

- [CLI](src/main.rs): Cargo invocation, initialization, and build-selection arguments.
- [Project operations](src/project.rs): workspace discovery, format-preserving manifest edits, locks, and child-process errors.
- [Tool acquisition](src/tools.rs): release selection, SHA-256 verification, bounded archive extraction, and user-local caching.
- [Dynamic linkage](src/dynamic.rs): dependency selection and compiler argument adaptation.
- [Build pipeline](src/build.rs): obfuscation, artifact collection, stripping, packing, audits, and publication.
- [Binary inspection](src/audit.rs): ELF/PE metadata, imports, exports, hardening headers, and residual Rust string hints.
- [Integration tests](tests/cli.rs): initialization, failure boundaries, runtime parity, relocation, and required packed dependencies.

Each distribution includes a `build.json` report and Cargo/compiler logs.
The report records tool provenance, hashes, packing status, audit results, and known limits.

## Compatibility and limits

- Native x86_64 Linux GNU execution is verified with the owned workspace fixture.
  Windows MSVC paths exist but do not yet have native runtime verification.
  Cross-compilation and other hosts are rejected.
- UPX is pinned to 5.1.1.
  UPX 5.2.1 failed the tested amd64 Linux shared-library packing with `undefined symbol 'my_memcpy'`.
- Rust `dylib` linkage preserves Rust-to-Rust interfaces without source adapters.
  It does not create a stable C ABI or move executable source into a library.
  Generics and inlining can leave dependency code in executables.
  Incompatible direct dependency crate types fail instead of silently using static linkage.
- Precompiled standard-library code and native dependencies do not receive the target LLVM passes.
  Required system libraries remain platform prerequisites.
  The bundle does not discover or copy arbitrary native shared dependencies.
- Required dynamic exports and unwind data remain.
  Rust runtime strings, type names, and recognizable code can remain.
  Metadata removal does not guarantee removal of every Rust fingerprint.
- UPX packing is reversible, not encryption.
  Integrity checks do not establish resistance to reverse engineering or application behavior beyond the exercised fixtures.
- Application builds execute their own build scripts and procedural macros.
  The pipeline is not a sandbox for untrusted projects.

## Development verification

Run from the repository root:

```sh
cargo fmt --check
cargo test
cargo test --test cli real_native_pipeline -- --ignored --nocapture
cargo build --release
cargo clippy --all-targets
cargo deny check licenses
```

The ignored native test acquires verified tools when the cache is empty.
It compares both fixture executables against an ordinary build, including panic and unwind behavior.
It also checks relocated loading, failure without the packed dependency, and preservation of published output after compilation failure.

The maximum Clippy preset currently reports a backlog, including CLI output, Result-returning test assertions, and function-length findings.
The pipeline's runtime checks do not imply a clean Clippy result.
