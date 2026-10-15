# Architecture

## Scope and build contract

`cargo-rrea` is a Cargo subcommand for native release builds with Arkari LLVM passes, metadata stripping, and UPX-packed Rust shared libraries.
`cargo rrea init` initializes the workspace manifest.
`cargo rrea build --release` and direct `cargo-rrea build --release` invocation select the same pipeline.
Build flags belong to the `build` subcommand, not the root command or `init`.
`build` requires explicit `--release` (or `-r`); development and custom profiles are not supported.
Executables remain unpacked.

`init` changes only the workspace root manifest's release `debug` and `strip` values.
It preserves unrelated settings, comments, and the panic strategy.
Repeated initialization does not rewrite unchanged content; invalid TOML fails without modifying the manifest.

Application builds select the acquired Arkari compiler and matching sysroot without changing the active Rust installation.
A compiler failure stops the build without ordinary-rustc fallback.
Cargo still uses its active toolchain to compile and test `cargo-rrea` itself; bootstrap prepares Arkari for the application pipeline rather than replacing Cargo's compiler mid-build.

## Cargo bootstrap and tool ownership

The [Cargo build script](../build.rs) runs for the first `cargo test`, `cargo build`, or `cargo run` that needs to compile this package.
It automatically acquires the native obfuscating toolchain under the Cargo target directory:

```text
target/
  downloads/<host>/
    compiler.json
    compiler-<release>/stage1/
    tools.lock
```

A custom Cargo target directory uses the same `downloads/<host>/` layout.
Development, test, and release profiles share this toolchain cache.
No manual compiler download is required.

The shared [toolchain module](../src/toolchain.rs) resolves the upstream release through the GitHub API, validates the expected host asset and HTTPS URL, and checks the archive size and SHA-256 before bounded ZIP extraction.
Archive traversal, links, and special files are rejected.
Installation publishes the completed compiler directory before atomically publishing its release record.
A file lock serializes concurrent acquisition, and a cached release record avoids re-querying the moving latest release.
Cached records and the expected compiler/sysroot layout are checked; invalid or incomplete installations fail rather than selecting another compiler.
The downloaded compiler must run and report the matching host before bootstrap succeeds.
The matching `stage1/lib/rustlib/<host>/bin/llvm-objcopy` must also exist and run.
The application pipeline uses this bundled executable for both preflight and stripping; no system `llvm-objcopy` or PATH fallback is used.

The build script watches the release record, so removing the downloads directory causes a subsequent Cargo invocation to prepare it again.
Set `CARGO_NET_OFFLINE=true` to forbid bootstrap downloads; an empty cache then fails.
Cargo's command-line `--offline` alone governs Cargo's own network activity, not this custom downloader.

At runtime, the CLI searches its executable's ancestors for the prepared `downloads/<host>/` cache.
This reuses bootstrap output for both normal binaries and Cargo test artifacts without embedding an absolute source-checkout path.
A relocated or installed CLI without a nearby prepared cache acquires the compiler automatically in the per-user `cargo-rrea/<host>/` cache.
Once a cache is selected, acquisition errors do not trigger a fallback to another cache or ordinary Rust compiler.
The `build` subcommand's `--offline` and `--frozen` options forbid runtime compiler downloads.

UPX is built from the pinned [submodule](../thirdparty/upx), which tracks `devel`, or from the same sources bundled in the published crate.
The [source pin](../thirdparty/upx-source.json) records the upstream revision and a SHA-256 covering every source and recursive vendor file, excluding Git metadata.
The digest is SHA-256 over sorted UTF-8 relative paths with `/` separators, each followed by a tab, the file's lowercase SHA-256, and LF.
Both checkout and archive builds verify this digest; links, special files, missing files, extra files, and changed content fail verification.
Checkout builds additionally require the pinned Git revision and clean recursive submodules.
Crate archives deliberately do not require `.git` files or Git commands.
Normal builds never fetch moving branch updates or fall back to a released UPX binary.
CMake and Ninja build Debug for development/test profiles and Release for release profiles, honoring Cargo's job limit.
Native build output stays under Cargo's `OUT_DIR`, outside the submodule.
The native build directory is keyed by its source path because Cargo can reuse `OUT_DIR` between checkout and extracted-crate builds; their CMake caches must remain separate.

The UPX executable is embedded in `cargo-rrea` and materialized atomically into a content-addressed per-user cache with executable permissions.
Existing cached bytes must match the embedded executable's SHA-256.
Runtime UPX acquisition needs neither downloads nor the source checkout.
The Linux UPX executable links the system C++ runtime, including `libstdc++.so.6`.

## Application build and publication

Selected executable packages' own libraries and first-level normal Rust dependencies also produce Rust `dylib` artifacts.
Compiler arguments link consumers against these artifacts.
Shared-library inclusion follows the current Cargo artifact messages, including metadata sidecars that identify hashed compiler outputs when Cargo reports a selected workspace library through a hard-linked root output.
Build scripts and procedural macros remain host-side and do not receive the target LLVM passes.

The pipeline strips nonessential metadata, packs shared libraries and the compiler's dynamic standard library, and leaves executables unpacked.
Linux executables locate adjacent libraries through `$ORIGIN`.
Required imports, exports, relocations, and unwind data remain intact.

Publication follows stripping, metadata audits, UPX integrity checks, and decompression audits in a staging directory.
Failed builds do not replace existing distribution directories.
Distributions are published beneath the selected project's Cargo target directory as `rrea/dist-<id>/`.
The CLI prints executable paths on stdout and diagnostics on stderr.

Each distribution includes a `build.json` report and Cargo/compiler logs.
The report records compiler release provenance, the UPX source commit and executable hash, packing status, artifact hashes, audit results, and known limits.
Report-relative paths resolve against the published distribution directory.

## Source ownership

| File | Responsibility |
| --- | --- |
| [src/main.rs](../src/main.rs) | Cargo invocation, initialization, and build-selection arguments |
| [src/project.rs](../src/project.rs) | Workspace discovery and format-preserving manifest edits |
| [src/support.rs](../src/support.rs) | Shared child-process checks and file locks |
| [build.rs](../build.rs) | Compiler bootstrap, pinned UPX source validation, native compilation, and embedding |
| [thirdparty/upx-source.json](../thirdparty/upx-source.json) | UPX revision and recursive source-content digest for checkout and crate builds |
| [src/toolchain.rs](../src/toolchain.rs) | Compiler release selection, verified acquisition, and bounded ZIP extraction shared by bootstrap and runtime |
| [src/tools.rs](../src/tools.rs) | Runtime cache selection and embedded UPX materialization |
| [src/dynamic.rs](../src/dynamic.rs) | Dependency selection and compiler argument adaptation |
| [src/build.rs](../src/build.rs) | Obfuscation, artifact collection, stripping, packing, audits, and publication |
| [src/audit.rs](../src/audit.rs) | ELF/PE metadata, imports, exports, hardening headers, and residual Rust string hints |
| [tests/cli.rs](../tests/cli.rs) | Bootstrap, initialization, failure boundaries, runtime parity, relocation, and required packed dependencies |

## Compatibility and limits

- Native x86_64 Linux GNU execution is covered by the owned workspace fixture.
  Windows MSVC paths exist but do not yet have native runtime verification.
  Cross-compilation and other hosts are rejected.
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

## Source development

Git checkouts require recursive submodules initialized; crate archives already contain the verified UPX sources and vendors.
Building `cargo-rrea` requires Rust/Cargo, curl, CMake, Ninja, and a C/C++ toolchain with C++17 support; checkout builds also require Git.

Initialize the submodules from the repository root:

```sh
git submodule update --init --recursive
```

To update UPX deliberately, advance its branch and initialize the vendor revisions pinned by that commit:

```sh
git submodule update --remote thirdparty/upx
git -C thirdparty/upx submodule update --init --recursive
```

Review and record the changed gitlink with the consuming code, and update the source pin's revision and digest together.
Read the new revision with `git -C thirdparty/upx rev-parse HEAD` and compute the canonical source digest from the repository root:

```sh
set -o pipefail
git -c core.quotePath=false -C thirdparty/upx ls-files --recurse-submodules | LC_ALL=C sort |
while IFS= read -r path; do
    checksum=$(sha256sum "thirdparty/upx/$path") || exit 1
    printf '%s\t%s\n' "$path" "${checksum%% *}"
done | sha256sum
```

UPX's [LICENSE](../thirdparty/upx/LICENSE) and [COPYING](../thirdparty/upx/COPYING) remain in the upstream checkout.

Run from the repository root:

```sh
cargo fmt --check
cargo test
cargo test --test cli real_native_pipeline -- --ignored --nocapture
cargo build --release
cargo run -- --help
cargo clippy --all-targets
cargo deny check licenses
cargo package --locked
```

The regular tests verify the bootstrapped compiler and matching sysroot, initialization, archive extraction boundaries, detached offline cache misses, unsupported targets, and UPX cache integrity.
The ignored native test exercises the actual `cargo rrea init` and `cargo rrea build --release` workflow for both default workspace selection and an explicitly selected executable package.
It uses the bootstrap compiler offline with an initially empty user cache.
It compares both fixture executables against an ordinary build, including panic and unwind behavior.
It also checks relocated loading, failure without the packed dependency, and preservation of published output after compilation failure.

The maximum Clippy preset applies to the CLI, Cargo build script, shared modules, and tests.
Run the uncapped all-target gate without changing lint levels or thresholds.
Packaging verification must build the extracted crate with UPX Git metadata absent; `--no-verify` does not exercise this boundary.
Use `--allow-dirty` only when intentionally checking uncommitted packaging changes locally.
