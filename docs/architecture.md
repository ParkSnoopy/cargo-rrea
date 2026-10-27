# Architecture

- [Build contract](#scope-and-build-contract)
- [Obfuscation policy](#obfuscation-policy)
- [Tool ownership](#cargo-bootstrap-and-tool-ownership)
- [Build and publication pipeline](#application-build-and-publication)
- [Source ownership](#source-ownership)
- [Compatibility and limits](#compatibility-and-limits)
- [Source development](#source-development)

## Scope and build contract

`cargo-rrea` is a Cargo subcommand for native release builds with Arkari LLVM passes, metadata stripping, and UPX packing.

`cargo rrea build --release` and direct `cargo-rrea build --release` invocation select the same pipeline.
Build flags belong to the `build` subcommand, not the root command.
`build` requires explicit `--release` (or `-r`); development and custom profiles are not supported.

### Output levels

| Arguments | Suppressed output |
| --- | --- |
| None | None |
| `-q`, `--quiet` | Cargo progress and standalone compiler code-generation-unit names on either stream |
| `-qq`, `-q -q`, `--quiet --quiet` | The above plus rrea status messages, report-path announcements, and runtime-hint notes |

Warnings, errors, executable paths on stdout, and report contents remain unchanged at every level.
Quiet levels leave compiler flags, obfuscation policy, and build-cache identity unchanged.

### Profile overrides

Overrides apply only to the child build process.
Workspace and member manifests remain unchanged on success and failure.

| Setting | Effective value | Opt-out |
| --- | --- | --- |
| Debug information | `debug = 0` | `--no-strip` retains the project setting |
| Stripping | `strip = "symbols"` | `--no-strip` retains the project setting |
| Panic strategy | `CARGO_PROFILE_RELEASE_PANIC=unwind` | Required by the shared runtime |

The panic override replaces both the project's release setting and any inherited value of that environment variable.
It matches the precompiled dynamic standard library's `panic_unwind` runtime, including when the project requests `panic = "abort"`.
Diagnostics and reports expose the effective strategy.

### Compiler selection

Application builds select the acquired Arkari compiler and matching sysroot without changing the active Rust installation.
A compiler failure stops the build without ordinary-rustc fallback.
Cargo uses its active toolchain to compile and test `cargo-rrea` itself.

## Obfuscation policy

The [feature registry](../src/obfuscation.rs) owns the CLI opt-outs and LLVM flag selection.
All implemented features are enabled by default at the compiler's existing pass levels.
Each selection disables the indicated transformation:

| Switch | Disabled transformation |
| --- | --- |
| `--no-indbr` | Arkari indirect branches |
| `--no-icall` | Arkari indirect calls |
| `--no-indgv` | Arkari indirect global-variable references |
| `--no-cse` | Arkari constant-string encryption |
| `--no-fla` | Arkari control-flow flattening |
| `--no-cie` | Arkari constant-integer encryption |
| `--no-cfe` | Arkari constant-floating-point encryption |
| `--no-mangle library` | Library filename and loader-reference mangling |
| `--no-mangle symbol` | Closed-bundle Rust dynamic-symbol renaming |
| `--no-mangle` | Both library and symbol mangling |
| `--no-upx` | Runtime UPX packing and packing-specific checks |
| `--no-opaque` | Opaque stored-payload packing of executables and shared libraries, restoring normal UPX compression |
| `--no-strip` | Forced debug/strip profile overrides and post-link metadata/static-symbol stripping |
| `--no-remap-paths` | Compiler path remapping and absolute-build-path rejection |

Repeated `--no-mangle` selectors and comma-separated values are combined.

### Policy propagation

- The Arkari master gate is emitted only when at least one of its seven passes is selected. Disabling every pass removes injected obfuscation LLVM flags while preserving the compiler, linkage, and remaining transformations.
- The selected feature set is serialized into one compiler policy and contributes to the build-cache key. The wrapper, post-link stages, and reports consume that policy.
- External LLVM arguments remain rejected even when all passes are disabled.
- LTO restrictions, the unwind panic strategy, runtime-metadata comparisons, input integrity, and atomic publication remain mandatory.
- Stripping and path remapping are independent. Their opt-outs can expose metadata or local paths and are recorded in each report.

### Packing modes

| Artifact | Packing policy |
| --- | --- |
| Linux x86_64 executable or `.so` | `--opaque` stored-block mode by default, normal compression with `--no-opaque` |
| Windows x86_64 executable or `.dll` | `--opaque` stored-image mode by default, normal compression with `--no-opaque` |
| Any artifact with `--no-upx` | No UPX subprocess runs |

The selected packing mode applies uniformly to every artifact in the runtime bundle.
Opaque mode is reversible scrambling, not encryption, and can increase output size.
The bundled helper and its installation prerequisites remain required when runtime packing is disabled.

### Symbol selection

ELF symbol renaming is default-on under the closed-bundle contract below.
`--keep-symbol NAME` retains a specific externally required Rust name and conflicts with any `--no-mangle` selection that disables symbol mangling.

Unsupported selected features fail before metadata discovery, downloads, or compilation.
Because PE symbol renaming is not implemented, Windows builds must disable it with `--no-mangle symbol` or bare `--no-mangle`.

## Cargo bootstrap and tool ownership

### Compiler cache

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

The shared [toolchain module](../src/toolchain.rs) owns acquisition:

1. Resolve the pinned compiler release, including Rust byte-string support, through the GitHub API.
2. Verify the expected host asset, HTTPS URL, archive size, and SHA-256.
3. Extract the ZIP within bounds, rejecting traversal, links, and special files.
4. Check the compiler/sysroot layout. Require a runnable compiler reporting the matching host and a runnable `stage1/lib/rustlib/<host>/bin/llvm-objcopy`.
5. Publish the completed compiler directory, then atomically publish its release record.

A file lock serializes acquisition.
A matching cached release avoids network access; an older release is replaced only after the new one is verified.
Offline mismatches leave the cache record unchanged, and invalid installations fail instead of selecting another compiler.

Preflight and stripping both use the bundled objcopy rather than a system executable.

The build script watches the release record, so removing the downloads directory causes a subsequent Cargo invocation to prepare it again.

### Runtime lookup and offline behavior

At runtime, the CLI searches its executable's ancestors for the prepared `downloads/<host>/` cache.
Normal binaries and Cargo test artifacts share this executable-relative lookup.
A relocated or installed CLI without a nearby prepared cache acquires the compiler automatically in the per-user `cargo-rrea/<host>/` cache.

Once a cache is selected, acquisition errors do not trigger a fallback to another cache or ordinary Rust compiler.

| Control | Network scope |
| --- | --- |
| `CARGO_NET_OFFLINE=true` | Forbids bootstrap downloads; an empty cache fails |
| Cargo's `--offline` | Governs Cargo's network activity, not the custom downloader |
| `cargo rrea build --offline` or `--frozen` | Forbids runtime compiler downloads |

### UPX source integrity

UPX is built from the pinned [ParkSnoopy/upx](https://github.com/ParkSnoopy/upx) [submodule](../thirdparty/upx), which tracks `devel`, or from the same sources bundled in the published crate.
The [source pin](../thirdparty/upx-source.json) records the fork revision and a SHA-256 covering every source, generated stub, and recursive vendor file, excluding Git metadata.

The digest is SHA-256 over sorted UTF-8 relative paths with `/` separators, each followed by a tab, the file's lowercase SHA-256, and LF.

- Checkout and archive builds reject links, special files, missing or extra files, and changed content.
- Checkout builds also require the pinned Git revision and clean recursive submodules.
- Source-integrity failures abort before compiler acquisition.
- The watched source pin is read on every build-script run. Normal builds use that revision rather than fetching branch updates or selecting a released UPX binary.

### Native build and embedding

The fork owns packer changes and matching generated headers/maps together.
After source validation, CMake and Ninja build Debug for development/test profiles and Release for release profiles, honoring Cargo's job limit.
Output stays under Cargo's `OUT_DIR`, keyed by the source path and fork revision.

Linux native compilation remaps UPX source and build paths, including paths in assertions and debug information, so the embedded executable does not leak the checkout location into protected outputs.

The UPX executable is embedded in `cargo-rrea` and materialized atomically into a content-addressed per-user cache with executable permissions.
Existing cached bytes must match the embedded executable's SHA-256.
The Linux UPX executable links the system C++ runtime, including `libstdc++.so.6`.

## Application build and publication

### Rust linkage

The complete normal Rust dependency graph participates in linkage planning, including selected executable packages' own libraries.
Eligible Rust libraries also produce `dylib` artifacts; compiler arguments link consumers against them, matching either Cargo's `.rlib` or `.rmeta` dependency paths.

### Native archive ownership

Crates with native-library arguments (`-l`) retain their `rlib` linkage, as do all Rust consumers of a retained crate, transitively through the final executable.
This keeps native archive ownership at the final link: a forced Rust dylib need not export native functions declared only through FFI, and separate shared consumers must not embed duplicate native state.

- Native-linked dependencies must use `lib` or `rlib` crate types. An explicit `dylib` requirement fails.
- Unrelated Rust dependencies and the standard library remain dynamic. Every target Rust crate still receives the selected LLVM passes and path remapping.
- The library log records linkage after successful compilation. Its latest record for each rlib path determines subsequent consumers and publication; only dynamic records become runtime-bundle candidates.
- Native build scripts own C/C++ path remapping. Rust remap flags leave native debug data and string literals untouched, and the output path audit rejects leaks.

### Compiler scheduling and runtime

- Suppress early target JSON artifact notifications so Cargo waits for linkable output, not merely metadata.
- Retain embedded Rust metadata for compiler linkage even when Cargo requests separate metadata. Distribution stripping removes it afterward.
- Resolve the dynamic standard library for generated shared libraries, including `no_std` dependencies that need the application's allocator and panic runtime.
- Follow current Cargo artifact messages and metadata sidecars when selecting hashed library outputs, including hard-linked workspace root outputs.
- Keep build scripts and procedural macros host-side, outside the target LLVM passes.

### Optimization constraints

Application builds override the release profile's cross-crate LTO setting with `lto = false` without editing the project's manifest.
Cargo must emit native objects for dependencies that the wrapper also builds as shared libraries, rather than bitcode-only linker-plugin inputs.
This retains Cargo's default local ThinLTO where applicable and the selected obfuscation passes.

- Reject explicit LTO or linker-plugin LTO flags that conflict with shared-library linkage.
- Reject panic codegen flags other than `-Cpanic=unwind` before invoking the target compiler.
- Remove conflicting `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` entries before building.

### Bundle discovery and library names

Runtime inclusion follows the ELF/PE import-library graph from the selected executables.
Only reachable generated Rust libraries and the compiler's dynamic standard library enter the distribution, with the selected stripping and packing stages applied.
Linux executables locate libraries through `$ORIGIN/libs`; bundled libraries locate their siblings through `$ORIGIN`. These paths are relative to each binary, not the working directory. Required imports, exports, relocations, and unwind data remain intact.
Windows executables embed a side-by-side dependency on the [private `libs` assembly](https://learn.microsoft.com/en-us/windows/win32/sbscs/assembly-searching-sequence). Its `libs/libs.manifest` lists the final bundled DLL filenames, including mangled names, so the Windows loader resolves direct and transitive imports relative to the executable before application startup. System DLL imports retain the normal system lookup. Manifest generation remains active when mangling or packing is disabled.

Library-name mangling is enabled by default and can be disabled with `--no-mangle library` or bare `--no-mangle`.

1. Reserve dynamic-string space for longer hashed names using redundant `/.` components in the Linux `$ORIGIN` search path. Size the reserve for the dependency graph and alignment slack.
2. Reduce the padded path back to `$ORIGIN/libs` for executables or `$ORIGIN` for shared libraries before rebuilding loader metadata, preserving unrelated search paths.
3. Keep code, data, and unwind addresses unchanged and loader metadata below the UPX stub, including in tightly packed ELF layouts.

### Symbol rewriting

Closed-bundle ELF Rust symbol renaming is enabled unless `--no-mangle symbol` or bare `--no-mangle` is selected.
The build contract requires all externally consumed or runtime-looked-up Rust names to be listed with `--keep-symbol NAME` when renaming is enabled.
Windows rejects symbol renaming before building, including when selected by default.
The [symbol policy](rust-fingerprints.md) defines eligibility, explicit exclusions, deterministic names, and remaining platform obligations.

The complete reachable bundle is inspected before any transformation is selected.
One shared map updates definitions and consumers through the existing ELF builder, including GNU/SysV hashes, GNU version arrays, and symbol-index relocations.
The runtime audit checks addresses, sizes, section ownership, binding/type/visibility, versions, and relocation identities against the expected map.

When stripping is enabled, unreferenced, non-loaded `rust_metadata_*` exports in `.rustc` are removed first in either symbol policy, preventing dangling section references.
Unexpected exported metadata or a bundled reference to compiler metadata fails instead of being stripped.
With `--no-strip`, compiler metadata and its exports remain together.

### Packing and artifact audits

The pinned fork supplies coordinated non-text format tags, native stubs, and lossless PE64 recovery.
The fork's Linux runtime guard checks visible process names at packed executable startup and shared-library initialization and can terminate the process even when a matching analysis tool is unrelated.

Native PE64 packing preserves the original import/export layout in the stored or compressed source image while retaining the packed image's runtime tables.
Recovery restores those original source bytes.

Artifact audits require:

- One checksum-valid patched pack header in the pinned format's search region, valid ELF loader metadata, and rejection of original PE section labels.
- Rejection of old or corrupt headers, including when application payloads contain `UPX!` or `RREA`.
- Offset-bearing whole-file matches in `packed_audit.literal_hints`. Application strings, overlays, and compressed bytes are not classified as packer headers from text alone.
- Retained Rust runtime hints recorded as notes and in the unpacked audit.
- Direct testing and decompression by the matching embedded packer, with recovery matching the pre-packing hash.

### Transactional publication

Transform and audit every artifact in staging before publication.
Runtime-metadata preservation checks remain active when stripping or packing is disabled.

- Publish selected executables to `target/release/`, relative to the selected manifest. Bundled `.so` and `.dll` files go in `target/release/libs/`; Windows also requires `libs/libs.manifest`. Virtual workspaces use the workspace root.
- Lock that manifest-owned destination independently of the custom Cargo target directory, which affects only internal builds and reports.
- Track owned executable names and the `libs` directory in `target/rrea/outputs.json`. Replace only these entries, remove previously selected executables and stale bundled libraries, and preserve unrelated Cargo outputs. Refuse unrelated directories or symlinks at output paths, including an unowned `libs` directory.
- Preserve previous outputs on compilation, audit, or packing failure. Back up replaced entries and restore them after publication failure when filesystem operations permit recovery. Serialize publishers; multi-entry replacement is not atomic to concurrent readers.
- Print executable paths on stdout and diagnostics on stderr.

### Reports

Reports and Cargo/compiler logs stay under Cargo's target directory as `rrea/dist-<id>/`, outside the runtime distribution.
The report records compiler release provenance, the UPX fork repository and source commit, executable hash, packing status, artifact hashes, audit results, and known limits.

| Field or value | Meaning |
| --- | --- |
| Packed audit's opaque bit | Read from the validated header and required to match the selected bundle policy |
| `symbol_mangling` | Exact rename map and retention reasons, or null when disabled |
| `removed_metadata_symbols` | Compiler-only export removals, independent of renaming |
| `disabled_obfuscation` | Explicit feature opt-outs |
| `obfuscation` | Actual injected LLVM flags |
| `panic_strategy` | Effective release strategy, `unwind` |
| Packing disabled | `packed = false`, null `packed_audit`, identical unpacked/output sizes and hashes |
| Artifact paths | Absolute distribution paths |
| Log paths | Relative to the report directory |

## Source ownership

| File | Responsibility |
| --- | --- |
| [src/main.rs](../src/main.rs) | Cargo invocation and build-selection arguments |
| [src/obfuscation.rs](../src/obfuscation.rs) | Shared default-on feature registry, CLI opt-outs, and LLVM flag selection |
| [src/project.rs](../src/project.rs) | Cargo command selection and shared process/lock access |
| [src/support.rs](../src/support.rs) | Shared child-process checks and file locks |
| [build.rs](../build.rs) | Compiler bootstrap, pinned UPX source validation, native compilation, and embedding |
| [thirdparty/upx-source.json](../thirdparty/upx-source.json) | UPX revision and recursive source-content digest for checkout and crate builds |
| [thirdparty/upx](../thirdparty/upx) | Fork-owned packer source, generated runtime stubs, and vendor dependencies |
| [src/toolchain.rs](../src/toolchain.rs) | Compiler release selection, verified acquisition, and bounded ZIP extraction shared by bootstrap and runtime |
| [src/tools.rs](../src/tools.rs) | Runtime cache selection and embedded UPX materialization |
| [src/dynamic.rs](../src/dynamic.rs) | Dependency selection and compiler argument adaptation |
| [src/build.rs](../src/build.rs) | Obfuscation, artifact collection, stripping, packing, audits, and publication |
| [src/audit.rs](../src/audit.rs) | ELF/PE metadata, imports, exports, hardening headers, and residual Rust string hints |
| [src/symbols.rs](../src/symbols.rs) | ELF symbol/relocation inspection, closed-bundle eligibility, and compiler-metadata removal planning |
| [src/mangle.rs](../src/mangle.rs) | Library identity rewriting and declared ELF symbol transformations |
| [tests/symbols/mod.rs](../tests/symbols/mod.rs) | Native ELF loader, hash/version, exclusion, collision, and metadata-reference boundaries |
| [tests/cli.rs](../tests/cli.rs) | Bootstrap, temporary profiles, manifest preservation, runtime parity, relocation, and required packed dependencies |

## Compatibility and limits

- Native x86_64 Linux GNU execution is covered by the owned workspace fixture.
  The fork's native Windows CI covers UPX executable startup and DLL loading, but the complete Windows MSVC build pipeline remains unverified.
  Cross-compilation and other hosts are rejected.
- Rust `dylib` linkage preserves Rust-to-Rust interfaces without source adapters.
  It does not create a stable C ABI or move executable source into a library.
  Generics and inlining can leave dependency code in executables.
  Incompatible dependency crate types fail instead of silently using static linkage.
- Precompiled standard-library code and native dependencies do not receive the target LLVM passes.
  Required system libraries remain platform prerequisites.
  The bundle does not discover or copy arbitrary native shared dependencies.
- Required dynamic exports and unwind data remain.
  Rust runtime strings, type names, and recognizable code can remain.
  Metadata removal does not guarantee removal of every Rust fingerprint.
- UPX packing is reversible, not encryption.
  Removing known signatures does not hide every loader pattern or prevent manual recovery with the patched packer.
  Modified UPX and stubs are restricted to GPL-compatible programs by the [upstream license](../thirdparty/upx/LICENSE).
  Integrity checks do not establish resistance to reverse engineering or application behavior beyond the exercised fixtures.
- Application builds execute their own build scripts and procedural macros.
  The pipeline is not a sandbox for untrusted projects.

## Source development

### Setup

Building `cargo-rrea` requires Rust/Cargo, Git, curl, CMake, Ninja, and a C/C++ toolchain with C++17 support.

Initialize the submodules from the repository root:

```sh
git submodule update --init --recursive
```

### Updating UPX

Advance the fork branch and initialize the vendor revisions pinned by that commit:

```sh
git submodule update --remote thirdparty/upx
git -C thirdparty/upx submodule update --init --recursive
```

Review and record the changed gitlink with the consuming code, and update the source pin's revision and digest together.
Maintain source changes and regenerated native headers/maps in the fork using the [maintenance procedure](upx-maintenance.md).
Do not merge generated numeric arrays or restore local overlays in the consuming repository.
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

### Verification commands

Run from the repository root:

```sh
cargo fmt --check
cargo test
cargo test --test cli real_native_pipeline -- --ignored --nocapture
cargo test --test cli real_obfuscation_opt_out_pipeline -- --ignored --nocapture
cargo test --test cli real_quiet_pipeline -- --ignored --nocapture
cargo build --release
cargo run -- --help
cargo clippy --all-targets
cargo deny check licenses
cargo package --locked
```

### Test coverage

| Suite | Boundaries |
| --- | --- |
| Regular tests | Compiler/sysroot bootstrap, CLI and invalid-manifest rejection, archive extraction, detached offline cache misses, unsupported targets, and UPX cache integrity |
| PE64 fixture | Import/export/code preservation and byte-identical normal and opaque packing recovery before and after name mangling, including differing IAT and lookup contents |
| ELF symbol fixture | GNU-only, SysV-only, and combined hashes; versioned exports, TLS, IFUNCs, weak symbols, and a retained `dlsym` name using the native compiler/linker |
| Marker regression | Application/overlay strings preserved; old tags and corrupted checksums rejected at actual header locations |
| [Base116 evaluator](../tests/evaluation/mod.rs) | Behavior, artifact inventories, and round-robin timings for six prepared variants; see [results and evidence limits](evaluation.md) |

The ignored native Rust suite runs `cargo rrea build --release` with the bootstrap compiler offline and an initially empty user cache.
Its coverage includes:

- **Selection and profiles:** default workspace and explicit executable-package selection, member manifests, custom target directories, and single-executable builds. Workspace and member manifests remain byte-identical after success and failure, including when they request `panic = "abort"`, debug information, and unstripped output.
- **Panic behavior:** override inherited abort-profile settings, reject conflicting panic compiler flags, and compare both executables with an ordinary development build. Check unwind behavior, the reported strategy, and actual target compiler arguments.
- **Rust dependencies:** allocate through a `no_std` dependency and its transitive `no_std` library.
- **Native ownership:** direct and transitive Rust calls share one bindings-only C archive's mutable counter while unaffected dependencies stay dynamic. Omit optional Cargo `links` metadata to test discovery from compiler arguments. Use CMake/Ninja and remapped C debug paths to preserve offline cache-miss and no-strip path-audit coverage.
- **Publication:** relocated loading, missing packed transitive dependencies, stale-file removal, republication, and preservation of published output after compilation failure.
- **Packing:** tight ELF layout, removal of search-path padding, coordinated headers, fork provenance, default opaque executables and shared libraries, `--no-opaque`, and byte-identical recovery.
- **Symbol policy:** both filename policies, unknown keep-symbol rejection, and default → `--no-fla` → default transitions in a populated cache. Check actual compiler invocations and explicit symbol retention.
- **Opt-outs:** every feature individually and all disabled together. Check behavior, packing/stripping state, exact reports, relocation, and failed publication.
- **Strings:** a length-delimited UTF-8 string with an embedded NUL in loaded segments after unpacking, requiring plaintext only when CSE is disabled.

Failed native runs retain the fixture directory and full process exit statuses for investigation.

### Lint and packaging gates

- Run the maximum, uncapped Clippy preset across the CLI, Cargo build script, shared modules, and tests without changing lint levels or thresholds.
- Packaging verification must build the extracted crate with UPX Git metadata absent. `--no-verify` skips this boundary.
- Use `--allow-dirty` only for intentional local checks of uncommitted packaging changes.
