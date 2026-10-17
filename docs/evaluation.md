# Native evaluation

## Scope and provenance

This is an executed Linux x86_64 evaluation, not a reconstruction of the earlier reports' missing prompts.
The [machine-readable snapshot](evaluation.json) records the measured artifact identities and aggregate results.
Full local evidence is retained under `target/evaluation/`: distributions, recovery outputs, build reports, string/ELF/disassembly inventories, timing samples, and Ghidra projects/logs.
That directory is generated evidence, not a portable source link or a promise that `cargo clean` preserves it.

- Application: [Base116 CLI](https://github.com/ParkSnoopy/base116_cli/tree/595202db20655cd1a510718eed94bc0d7e46e5f8), revision `595202db20655cd1a510718eed94bc0d7e46e5f8`.
- Resolution: [retained lockfile](../tests/fixtures/base116.lock), including `base116 0.1.11`.
- Compiler for both build paths: acquired `R1990-L2311`, `rustc 1.99.0-dev`, LLVM `23.1.1`; the compiler reports its commit as unknown, not a fabricated source revision.
- UPX: pinned upstream revision plus the source patch and generated-stub hashes recorded in the snapshot.
- Inspection: GNU Binutils, `file`, stock UPX `5.3.0`, and Ghidra `12.1.4`.

The Ghidra distribution came from the [official release](https://github.com/NationalSecurityAgency/ghidra/releases/tag/Ghidra_12.1.4_build).
Its archive SHA-256 was verified as `ddac49f903da9d5bac833e5cc79395098b9c33cfd3279be5f31bd00387d2d4db`.

## Variants and behavior

| Variant | Construction | Runtime files | Total bytes |
| --- | --- | ---: | ---: |
| baseline | Same Arkari compiler, ordinary release, LTO disabled, no target obfuscation passes | 1 | 1,153,088 |
| stripped | Baseline plus `llvm-objcopy --strip-all --remove-section=.comment` | 1 | 889,736 |
| packed | Stripped baseline plus patched UPX `--best` | 1 | 374,072 |
| recovered | Byte-identical recovery of the default protected bundle | 19 | 10,410,536 |
| unmangled | Full protection with `--no-mangle-library` | 19 | 5,951,888 |
| mangled | Full default protection | 19 | 5,968,316 |

The baseline retains the application's manifest profile except for the command-local LTO override needed to match the protected build's LTO setting.
The manifest is not edited.
The protected variants retain all eight LLVM switches and the complete shared Rust dependency graph.
The baseline has ordinary Rust linkage, so baseline-versus-protected timings do **not** isolate compiler-obfuscation cost from linkage differences.
Stripped/packed variants isolate post-link transformations on one baseline, and recovered/mangled variants isolate packing on one protected bundle.
Mangled/unmangled builds also differ in metadata reservation and are separate compiler invocations, so tiny differences are not a universal filename-mangling cost.

The [maintained evaluator](../tests/evaluation/mod.rs) passed 52 binary/wrapper cases and 20 malformed-input cases per variant.
Binary cases cover empty data, every partial-chunk length through 16 bytes, boundaries around 32, 256, and 4096 bytes, all byte values, UTF-8 output, and required/optional wrappers.
It compares exit status, stdout, and stderr with the baseline, and independently requires decoded output to equal the original bytes.
Malformed cases include invalid/truncated UTF-8, incomplete payloads, whitespace, unsupported characters, and incomplete wrappers.
These are regression cases, not a blinded analyst's withheld recovery corpus.

The native workspace suite separately exercises allocation, transitive `no_std` dependencies, relocation, missing-library failure, panic/unwind, and publication rollback.
The packed cargo-rrea distribution contains 45 runtime files, starts successfully, and itself built and ran that fixture, including a caught panic.
Its first offline self-build exposed an empty nested compiler cache; validation seeded that task-owned cache from the already verified compiler and retried without changing the download policy.

## Timing and memory

The snapshot contains nine measured samples per variant after a warm-up round, taken round-robin.
The timed payload is 1,048,576 deterministic pseudorandom bytes, with exact output checks after every encoding and decoding run.
Startup measures a complete `--version` process.
Encode/decode timings include startup and file I/O.
Measurements were repeated after the build/download workload finished.
This is one host and one workload, not an isolated benchmark lab or a general slowdown guarantee.
The 1 ms process-polling interval limits resolution for short startup measurements.
RSS is a sampled high-water lower bound from `/proc`, not a guaranteed exact peak or aggregate multi-process memory measurement.

| Variant | Startup median (ms) | Encode median (ms) | Decode median (ms) | Sampled peak RSS (KiB) |
| --- | ---: | ---: | ---: | ---: |
| baseline | 1.50 | 21.73 | 28.05 | 2,764 |
| stripped | 1.31 | 17.83 | 28.06 | 2,764 |
| packed | 6.69 | 22.84 | 32.16 | 2,764 |
| recovered | 2.37 | 1,039.93 | 2,181.42 | 9,888 |
| unmangled | 46.06 | 1,059.31 | 2,222.53 | 9,360 |
| mangled | 44.15 | 1,083.80 | 2,232.48 | 9,320 |

See `timings` in the [snapshot](evaluation.json) for exact medians and derived throughput.
The protected encoder/decoder are substantially slower than the ordinary baseline in this workload, and the complete protected distribution is substantially larger even after packing.
Recovering the bundle removes most of the measured startup overhead but leaves the expensive protected computation.
No pass was disabled or protection default relaxed to improve these results.
Initial build observations are retained in local logs, but cache state and concurrent work differ, so they are not presented as comparable build-time benchmarks.

## Fingerprints, symbols, and recoverability

The self-packing failure was an audit false positive.
The original scan rejected any `RREA` byte sequence anywhere in the compressed file.
Inspection located that text in cargo-rrea's own audit vocabulary, retained as literal bytes within compressed application data.
The corrected audit checks the actual checksum-valid pack header, ELF loader record, and PE section labels.
It records whole-file literal hints with offsets rather than declaring arbitrary application or compressed bytes to be packer metadata.
The self-build report contains both `UPX!` and `RREA` payload hints while its real header uses the coordinated non-text tag.
Regression tests keep those application/overlay strings and reject old tags or damaged checksums at actual headers.

Every artifact in the measured default Base116 bundle has an empty packed literal-hint list.
That does not establish absence of all fingerprints.
The packed executable has no section table, two load segments starting at file offset zero, a writable expansion segment, and a separate executable loader mapping.
Its fixed replacement tags and decompressor code are themselves recognizable structure.
Live mapping probes observed one anonymous memfd mapping in the packed baseline and 19 in each protected bundle, with no `memfd:upx` name.
Removing a mapping label does not hide that a decompression loader is present.

The matching patched packer recovered the complete default bundle, and every recovered artifact's bytes matched its recorded pre-packing hash.
Stock UPX `5.3.0` reported `NotPackedException` on the packed baseline and all 19 default protected artifacts.
That is format incompatibility, not cryptographic protection.

After recovery the bundle exposes 2,556 dynamic export entries, of which 2,537 have Rust-style mangled spellings.
Filename mangling and its opt-out produced the identical exported-symbol multiset.
The Base116 library is identifiable from its exported decoder names despite its hashed filename.
Recovered disassembly of `CharsToUnflatDigits::next::to_digit` contains a large constant/state dispatcher.
This provides a concrete obfuscated decoder example rather than only a compiler-flag log.
A complete encode/decode coverage study and independent semantic reconstruction remain open.

String inventories also retain dependency source identities and `library/core/src` in generated code, and panic/std diagnostics in the precompiled standard library.
Required dynamic interfaces and diagnostic behavior remain intact.
The [Rust fingerprint policy and rename design](rust-fingerprints.md) separates those obligations from avoidable metadata and defines the requirements for future closed-bundle symbol renaming.

## Ghidra import is not a recovery score

Headless Ghidra was run with `-analysisTimeoutPerFile 30 -max-cpu 2` over the baseline executable, packed baseline executable, every default packed artifact, and every recovered artifact.
All 40 files imported and were saved as projects.

| Input | Imports | Analysis reported complete | Timed out |
| --- | ---: | ---: | ---: |
| baseline | 1 | 0 | 1 |
| packed baseline | 1 | 1 | 0 |
| protected packed bundle | 19 | 19 | 0 |
| recovered protected bundle | 19 | 17 | 2 |

The recovered timeouts were the standard library and `clap_builder`.
Baseline and recovered analysis produced exception-table/disassembly errors, and recovered relocation warnings still exposed Rust names such as `panic_bounds_check` and `___rust_no_alloc_shim_is_unstable_v2`.
The baseline also timed out, so a timeout or warning count cannot be scored as protection success.
“Analysis succeeded” on packed files primarily concerns the visible loader, not reconstructed application semantics.
This run does not claim successful full decompilation, failure of an expert analyst, or an automated structural-detector benchmark.

## Reproduction

Use a fresh evaluation root beneath `TMPDIR` or the repository's generated `target/evaluation` directory.
Build this checkout normally and capture Cargo's JSON output to obtain the exact `RREA_UPX_PATH` and `RREA_DOWNLOADS` build-script values.
Resolve the compiler and objcopy paths from that verified download record.
Clone the pinned application revision and copy the retained lockfile before building with `--locked`.
Use the same acquired compiler for the baseline, `--release --config profile.release.lto=false`, and a separate target directory.
Do not replace it with the active system compiler and call the result matched.

Create `variants/baseline`, `variants/stripped`, and `variants/packed` as defined in the table.
Run cargo-rrea's normal default build and copy its complete distribution to `variants/mangled` before running the opt-out build and copying that to `variants/unmangled`.
Decompress every member of the mangled snapshot to `variants/recovered` using the exact matching patched packer.
Record `source-revision.txt`, `source-at-evaluation.patch`, `base116-revision.txt`, and `base116/Cargo.lock` beneath the evaluation root.
The source diff and lockfile are provenance inputs, not substitutes for retaining the produced artifact hashes.

From the cargo-rrea repository, with that root assigned to `RREA_EVALUATION_ROOT`:

```sh
cargo test --locked --offline base116_behavior_inventory_and_costs -- --ignored --nocapture
cargo test --locked --offline base116_runtime_mapping_names -- --ignored --nocapture
```

The first writes `evaluation.json` with raw samples, inspections, sizes, and hashes.
The second writes `runtime-mappings.json` on Linux.
Keep host builds and other heavy analysis out of the timed run.
For Ghidra, use a project path with no dot-prefixed path component, such as `target/evaluation/ghidra-projects`, and set Java's temporary directory explicitly to the approved scratch directory.
Import success, per-file timeout, and warnings must be read from logs, not inferred from the headless process's exit status.

## Future blinded study

Give independent analysts the same operating system, tool versions, time budget, invocation examples, and non-withheld sample inputs.
Randomize artifact labels and withhold the protection recipe and original source equally.
Ask each analyst to identify behavior, recover chunk/radix/Unicode/wrapper/error rules, and submit a standalone encoder and decoder that do not invoke the supplied program or original library.
Record wall time, tool failures, manual interventions, and the submitted implementation.
Evaluate both submissions against the same separately withheld binary and malformed-input corpus.
Publish the exact briefing and rubric with the results.
No such analyst study was run here, and the historical prompts remain unavailable.
