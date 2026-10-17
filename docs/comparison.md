# Reverse-engineering comparison

## Scope

This case study compares two supplied reverse-engineering reports about [`b116`](https://github.com/ParkSnoopy/base116_cli):

- **base:** “Architecture of `b116`”, from `cargo build --release`.
- **rrea:** “Architecture & Reverse Engineering Analysis: `b116`”, from `cargo rrea build --mangle-library --release`.

The original reports are not included in the maintained documentation.
Section references below identify evidence in those reports.
This comparison does not rerun their binary analysis or independently validate their tool results.
The reports and screenshots describe the earlier header-masked build, not a new assessment of the current patched packer.
The historical `--mangle-library` flag is no longer needed or accepted: current builds mangle names by default and accept `--no-mangle-library` to opt out.
The separate [native evaluation](evaluation.md) tests the current implementation and records hashes, behavior, timings, and analysis failures.

## What the reports recovered

| Area | base report | rrea report | Interpretation |
| --- | --- | --- | --- |
| Application behavior | Identifies Base116, CLI options, streams, wrappers, and errors (§§1–2, 7–8). | Identifies the same purpose, commands, streams, and wrapper markers (§1). | High-level behavior remains discoverable. |
| Algorithm detail | Reconstructs byte grouping, radix arithmetic, Unicode mapping tiers, EOF handling, and decoding (§§4–6). | Gives a broad algorithm description and a flattened decoder-function example (§4). | Less algorithm detail is documented, but this does not establish that the missing detail could not be recovered. |
| Static inspection | Presents application architecture without describing a loader-recovery stage. | Reports missing section headers, unusable initial disassembly, and packed executable mappings (§2). | Suggests additional preparation before conventional static analysis. |
| Control flow | Describes direct encoding and decoding pipelines (§§5–6). | Describes state dispatchers, opaque predicates, and indirect targets (§4). | Consistent with harder-to-follow function structure, not proof that semantics are unrecoverable. |
| Library identity | Names recognizable dependencies such as `clap` and `clio` (§2). | Assigns roles to hash-named libraries and identifies decoder routine names (§§4–5). | Filename mangling obscures immediate labels but does not prevent component identification. |
| Recovery | Focuses on the encoding implementation. | Lists recovery methods and an unpacked executable artifact (§§2, 5). | The report itself describes routes past the packing layer. |

## Effectiveness

The strongest supported conclusion is **added static-analysis friction, not prevention of reverse engineering**.
The protected report describes work on loader layout, unpacked memory, and flattened control flow before discussing application internals.
Nevertheless, it still identifies the program, interface, core-library role, and specific routines.
Packing and mangled filenames did not keep those facts hidden.

Supplied terminal screenshots also show plaintext UPX branding, remaining `UPX!` bytes beside `RREA`, and Rust runtime names in Ghidra relocation diagnostics.
These are concrete fingerprint leaks despite stock UPX refusing to unpack the executable.
The [UPX overlay](../thirdparty/upx.patch) addresses known native packer identifiers, while Rust-symbol and structural-detection work remains in the [backlog](todo.md#fix-immediate-packer-and-language-fingerprints).

The evaluated build used UPX with `RREA` header masking.
The current [build pipeline](../src/build.rs) instead uses coordinated patched packer/stub tags and validates their actual header locations.
It records marker text elsewhere as evidence, not proof of a packer fingerprint.
Neither approach establishes encryption or a separately engineered anti-debugging system.
The report's descriptions of a custom unpacker and a separate “segment hollowing” defense should remain attributed interpretations, not additional project guarantees.
See [architecture and limits](architecture.md) for the implementation contract.

## Bias and evidence limits

The reports have different objectives: base reconstructs the application, while rrea emphasizes obfuscation and recovery.
The rrea report names `cargo-rrea` and Arkari at the outset.
It may have been briefed about the protection, but the original prompts are unavailable here.
Its narrower algorithm coverage cannot therefore be scored as an obfuscation success, and naming the protection cannot be scored as independent discovery.

Report accuracy also needs caution.
The rrea report gives an incorrect value for `116^7`; the correct value is `282,621,973,446,656`.
Its claim that the alphabet avoids whitespace conflicts with the base report's explicit newline and tab mappings.
These issues weaken confidence in unsupported technical assertions.

No matched binary hashes, analysis times, recovery-success tests, or performance measurements are supplied in these documents.
The comparison also does not isolate stripping, packing, library mangling, and compiler obfuscation from one another.
It cannot justify a slowdown multiplier, protection score, or claim of resistance to a determined analyst.
A stronger evaluation would use matched builds, equally briefed analysts, identical recovery tasks, and recorded time and correctness results.

## Current measured follow-up

The [new evaluation](evaluation.md) exercises six variants of one pinned Base116 revision using the same acquired compiler.
Binary/wrapper and malformed-input behavior match across all variants.
The matching patched packer recovers the complete protected bundle byte-for-byte.
Filename mangling changes dependency labels but not the exported-symbol multiset, which still identifies Rust and Base116 routines.
All selected Ghidra inputs import, although bounded analysis times out on the baseline and two recovered libraries.
Importing a loader does not establish recovery of the protected application's semantics.

The full protected bundle is larger and its encoder/decoder are substantially slower in this measured workload.
Different linkage prevents attributing the entire cost to LLVM obfuscation alone.
These findings support reversible packing and retained implementation clues, not a protection score or prevention claim.
Independent blinded reconstruction, broader pass coverage, native Windows execution, and structural-detector measurements remain open.

See [follow-up work](todo.md) for the evaluation and protection backlog.
