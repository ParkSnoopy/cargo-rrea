# Reverse-engineering comparison

## Scope

This case study compares two supplied reverse-engineering reports about [`b116`](https://github.com/ParkSnoopy/base116_cli):

- **base:** “Architecture of `b116`”, from `cargo build --release`.
- **rrea:** “Architecture & Reverse Engineering Analysis: `b116`”, from `cargo rrea build --mangle-library --release`.

The original reports are unavailable in this repository.
Section references below refer to those reports, which describe the earlier header-masked build.

**CLI compatibility:** the historical `--mangle-library` flag is retired.
Current builds mangle library names by default and accept `--no-mangle library` to opt out.

The separate [native evaluation](evaluation.md) records executed measurements, hashes, behavior, and analysis failures.

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
It still identifies the program, interface, core-library role, and specific routines.

Supplied terminal screenshots also show plaintext UPX branding, remaining `UPX!` bytes beside `RREA`, and Rust runtime names in Ghidra relocation diagnostics.
The evaluated UPX overlay addressed known native packer identifiers and is now maintained in the [UPX fork](upx-maintenance.md), while Rust-symbol and structural-detection work remains in the [backlog](todo.md#fix-immediate-packer-and-language-fingerprints).

### Packer versions and attribution

- **Historical build:** UPX with `RREA` header masking.
- **Current [pipeline](../src/build.rs):** coordinated packer/stub tags validated at actual header locations, with marker text elsewhere recorded as literal evidence.
- **Report interpretations:** “custom unpacker” and separate “segment hollowing” defense are attributed descriptions, not implementation guarantees.

Neither header scheme establishes encryption or a separately engineered anti-debugging system.
The [architecture](architecture.md) defines the implementation contract.

## Bias and evidence limits

### Briefing and objectives

The base report reconstructs the application; rrea emphasizes obfuscation and recovery and names `cargo-rrea` and Arkari at the outset.
The original prompts are unavailable, so prior disclosure of the protection is unknown.
Narrower algorithm coverage is not evidence of obfuscation success, and naming the protection is not independent discovery.

### Report accuracy

- The rrea report gives an incorrect value for `116^7`; the correct value is `282,621,973,446,656`.
- Its claim that the alphabet avoids whitespace conflicts with the base report's explicit newline and tab mappings.

### Missing controls

No matched binary hashes, analysis times, recovery-success tests, or performance measurements are supplied in these documents.
The comparison also does not isolate stripping, packing, library mangling, and compiler obfuscation from one another.

A slowdown multiplier or protection score requires matched builds, equivalent briefings, identical recovery tasks, and recorded time and correctness results.

## Current measured follow-up

The [native evaluation](evaluation.md) compares six variants of one pinned Base116 revision using the same acquired compiler.
That snapshot predates symbol renaming.

- **Behavior:** binary/wrapper and malformed-input results match across all variants.
- **Recovery:** the matching patched packer restores the complete protected bundle byte-for-byte.
- **Filename mangling:** dependency labels change, but the exported-symbol multiset still identifies Rust and Base116 routines.
- **Ghidra:** every selected input imports. Analysis times out on the baseline and two recovered libraries; importing a loader does not establish semantic recovery.
- **Cost:** the protected bundle is larger and its encoder/decoder are substantially slower in this workload. Different linkage prevents attributing the entire cost to LLVM obfuscation.

Independent blinded reconstruction, broader pass coverage, native Windows execution, and structural-detector measurements remain open.
See [follow-up work](todo.md) for the evaluation and protection backlog.
