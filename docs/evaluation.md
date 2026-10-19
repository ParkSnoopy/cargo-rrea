# Native evaluation

## Scope and provenance

This Linux x86_64 evaluation is recorded in the [machine-readable snapshot](evaluation.json).
It predates default-on symbol renaming.

- **Application:** [Base116 CLI](https://github.com/ParkSnoopy/base116_cli/tree/595202db20655cd1a510718eed94bc0d7e46e5f8), revision `595202db20655cd1a510718eed94bc0d7e46e5f8`.
- **Resolution:** [retained lockfile](../tests/fixtures/base116.lock), including `base116 0.1.11`.
- **Compiler for both paths:** acquired `R1990-L2311`, `rustc 1.99.0-dev`, LLVM `23.1.1`. The compiler reports its commit as unknown.
- **UPX:** upstream revision, source patch, and generated-stub hashes recorded in the snapshot.
- **Inspection:** GNU Binutils, `file`, stock UPX `5.3.0`, and Ghidra `12.1.4`.

The Ghidra distribution came from the [official release](https://github.com/NationalSecurityAgency/ghidra/releases/tag/Ghidra_12.1.4_build).
Its archive SHA-256 was verified as `ddac49f903da9d5bac833e5cc79395098b9c33cfd3279be5f31bd00387d2d4db`.

Raw distributions, recovery outputs, build reports, string/ELF/disassembly inventories, timing samples, and Ghidra projects/logs are retained locally under `target/evaluation/`.
This generated evidence is subject to build cleanup.

## Variants and behavior

| Variant | Construction | Runtime files | Total bytes |
| --- | --- | ---: | ---: |
| baseline | Same Arkari compiler, ordinary release, LTO disabled, no target obfuscation passes | 1 | 1,153,088 |
| stripped | Baseline plus `llvm-objcopy --strip-all --remove-section=.comment` | 1 | 889,736 |
| packed | Stripped baseline plus patched UPX `--best` | 1 | 374,072 |
| recovered | Byte-identical recovery of the default protected bundle | 19 | 10,410,536 |
| unmangled | Full protection without library filename mangling | 19 | 5,951,888 |
| mangled | Full default protection | 19 | 5,968,316 |

### Comparison controls

- **Baseline:** retain the application's manifest profile with a command-local LTO override matching the protected build.
- **Protected variants:** retain all eight LLVM switches and the complete shared Rust dependency graph.
- **Stripped/packed:** isolate post-link transformations on one baseline.
- **Recovered/mangled:** isolate packing on one protected bundle.
- **Mangled/unmangled:** compare filename policies across separate compiler invocations and metadata reservations. Small differences are not a universal mangling cost.

**Linkage caveat:** the baseline uses ordinary Rust linkage, so baseline-versus-protected timings do not isolate LLVM obfuscation cost.

### Behavior coverage

The [maintained evaluator](../tests/evaluation/mod.rs) passed 52 binary/wrapper cases and 20 malformed-input cases per variant.

- **Binary inputs:** empty data, every partial-chunk length through 16 bytes, boundaries around 32, 256, and 4096 bytes, all byte values, UTF-8 output, and required/optional wrappers.
- **Malformed inputs:** invalid/truncated UTF-8, incomplete payloads, whitespace, unsupported characters, and incomplete wrappers.
- **Assertions:** matching exit status, stdout, and stderr, plus decoded bytes equal to the original input.

These are regression cases, not a blinded analyst's withheld recovery corpus.

### Native and self-packing checks

The native workspace suite separately exercises allocation, transitive `no_std` dependencies, relocation, missing-library failure, panic/unwind, and publication rollback.
The packed cargo-rrea distribution contains 45 runtime files, starts successfully, and itself built and ran that fixture, including a caught panic.

The offline self-build required seeding its initially empty nested cache from the verified compiler.

## Timing and memory

| Measurement | Method |
| --- | --- |
| Sampling | Nine samples per variant after warm-up, taken round-robin after builds/downloads finished |
| Input | 1,048,576 deterministic pseudorandom bytes, with exact output checks after each encode/decode run |
| Startup | Complete `--version` process |
| Encode/decode | Wall time including startup and file I/O |
| Memory | `/proc` RSS sampled every 1 ms |

Results cover one host and workload.
Polling limits short-startup resolution; sampled RSS is a high-water lower bound, not an exact peak or aggregate multi-process measurement.

| Variant | Startup median (ms) | Encode median (ms) | Decode median (ms) | Sampled peak RSS (KiB) |
| --- | ---: | ---: | ---: | ---: |
| baseline | 1.50 | 21.73 | 28.05 | 2,764 |
| stripped | 1.31 | 17.83 | 28.06 | 2,764 |
| packed | 6.69 | 22.84 | 32.16 | 2,764 |
| recovered | 2.37 | 1,039.93 | 2,181.42 | 9,888 |
| unmangled | 46.06 | 1,059.31 | 2,222.53 | 9,360 |
| mangled | 44.15 | 1,083.80 | 2,232.48 | 9,320 |

See `timings` in the [snapshot](evaluation.json) for exact medians and derived throughput.

- The protected encoder/decoder are substantially slower, and the full distribution is larger even after packing.
- Recovery removes most startup overhead but leaves the protected computation cost.
- Build-time observations remain in local logs. Different cache states and concurrent work prevent a matched build-time comparison.

## Fingerprints, symbols, and recoverability

### Literal markers and audit boundaries

The original self-packing scan falsely rejected `RREA` from cargo-rrea's own audit vocabulary inside compressed application data.
The corrected audit checks checksum-valid pack headers, ELF loader records, and PE section labels, while recording other literal matches with offsets.

The self-build report contains `UPX!` and `RREA` payload hints alongside a valid coordinated non-text header.
Regression tests preserve application/overlay strings and reject old tags or damaged checksums at actual header locations.

### Recognizable loader structure

Every artifact in the measured default Base116 bundle has an empty packed literal-hint list.
Structural fingerprints remain:

- No section table, two load segments starting at file offset zero, a writable expansion segment, and a separate executable loader mapping.
- Fixed replacement tags and recognizable decompressor code.
- One anonymous memfd mapping in the packed baseline and 19 in each protected bundle, despite removal of the `memfd:upx` name.

### Unpacking

The matching patched packer recovered the complete default bundle, and every recovered artifact's bytes matched its recorded pre-packing hash.
Stock UPX `5.3.0` reported `NotPackedException` on the packed baseline and all 19 default protected artifacts.
That is format incompatibility, not cryptographic protection.

### Recovered symbols and strings

After recovery the bundle exposes 2,556 dynamic export entries, of which 2,537 have Rust-style mangled spellings.
Filename mangling and its opt-out produced the identical exported-symbol multiset.
The Base116 library is identifiable from its exported decoder names despite its hashed filename.

Recovered disassembly of `CharsToUnflatDigits::next::to_digit` contains a large constant/state dispatcher.
A complete encode/decode coverage study and independent semantic reconstruction remain open.

String inventories also retain dependency source identities and `library/core/src` in generated code, and panic/std diagnostics in the precompiled standard library.
Required dynamic interfaces and diagnostic behavior remain intact.
The [Rust fingerprint policy](rust-fingerprints.md) separates required behavior from avoidable metadata and defines the closed-bundle symbol contract.

## Ghidra import is not a recovery score

Headless Ghidra was run with `-analysisTimeoutPerFile 30 -max-cpu 2` over the baseline executable, packed baseline executable, every default packed artifact, and every recovered artifact.
All 40 files imported and were saved as projects.

| Input | Imports | Analysis reported complete | Timed out |
| --- | ---: | ---: | ---: |
| baseline | 1 | 0 | 1 |
| packed baseline | 1 | 1 | 0 |
| protected packed bundle | 19 | 19 | 0 |
| recovered protected bundle | 19 | 17 | 2 |

- **Timeouts:** the baseline, recovered standard library, and recovered `clap_builder` reached the analysis limit.
- **Diagnostics:** baseline and recovered inputs produced exception-table/disassembly errors. Recovered relocation warnings still exposed `panic_bounds_check` and `___rust_no_alloc_shim_is_unstable_v2`.
- **Interpretation:** analysis of packed inputs primarily concerns the visible loader. Import success, warning counts, and timeouts do not measure semantic recovery or protection strength.

Full decompilation, expert reconstruction, and automated structural detection were not evaluated.

## Reproduction

### Prepare the toolchain and variants

1. Create a fresh evaluation root beneath `TMPDIR` or the repository's generated `target/evaluation` directory.
2. Build this checkout and capture Cargo's JSON output for the exact `RREA_UPX_PATH` and `RREA_DOWNLOADS` values. Resolve the compiler and objcopy paths from the verified download record.
3. Clone the pinned application revision and copy the retained lockfile before building with `--locked`.
4. Build the baseline with the same acquired compiler, `--release --config profile.release.lto=false`, and a separate target directory.
5. Create `variants/baseline`, `variants/stripped`, and `variants/packed` as defined in the [variant table](#variants-and-behavior).
6. With the current CLI, select `--no-mangle symbol` for the filename-mangled variant and bare `--no-mangle` for the unmangled variant. Copy complete distributions to `variants/mangled` and `variants/unmangled` respectively.
7. Decompress every mangled artifact to `variants/recovered` with the matching patched packer.
8. Retain `source-revision.txt`, `source-at-evaluation.patch`, `base116-revision.txt`, `base116/Cargo.lock`, and produced artifact hashes beneath the evaluation root.

Current CLI selections reproduce the comparison's intended policies, not the recorded snapshot's hashes or timings.

### Run measurements

From the cargo-rrea repository, with that root assigned to `RREA_EVALUATION_ROOT`:

```sh
cargo test --locked --offline base116_behavior_inventory_and_costs -- --ignored --nocapture
cargo test --locked --offline base116_runtime_mapping_names -- --ignored --nocapture
```

| Test | Output |
| --- | --- |
| `base116_behavior_inventory_and_costs` | `evaluation.json`: raw samples, inspections, sizes, and hashes |
| `base116_runtime_mapping_names` | `runtime-mappings.json` on Linux |

- Keep builds and heavy analysis out of the timed run.
- For Ghidra, use a project path without dot-prefixed components, such as `target/evaluation/ghidra-projects`, and explicitly set Java's temporary directory to the approved scratch directory.
- Read import success, per-file timeouts, and warnings from logs rather than the headless process's exit status.

## Future blinded study

This protocol remains unexecuted; the historical prompts are unavailable.

1. Give independent analysts the same operating system, tools, time budget, invocation examples, and non-withheld sample inputs.
2. Randomize artifact labels and withhold the protection recipe and original source equally.
3. Require behavior identification, recovered chunk/radix/Unicode/wrapper/error rules, and standalone encoder/decoder implementations that do not invoke the supplied program or original library.
4. Record wall time, tool failures, manual interventions, and submitted implementations.
5. Evaluate submissions against the same withheld binary and malformed-input corpus.
6. Publish the exact briefing and rubric with the results.
