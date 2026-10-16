# Evaluation and protection follow-ups

Proposed work arising from the [reverse-engineering comparison](comparison.md) and supplied terminal screenshots.
These are open tasks, not implemented protections or measured results.

## Establish comparable evidence

- [ ] Record source revision, compiler versions, profile settings, artifact hashes, and analysis tools for both builds.
- [ ] Give analysts equivalent instructions and information budgets, without identifying the protected build in advance.
- [ ] Use identical recovery tasks: identify behavior, reconstruct the encoding and decoding rules, and validate recovered implementations against withheld inputs.
- [ ] Record time, tool failures, manual interventions, and correctness rather than comparing report length or emphasis.
- [ ] Add controlled build variants to separate stripping, packing, filename mangling, and compiler-obfuscation effects without changing the default protection policy.

## Fix immediate packer and language fingerprints

### Evidence retained from the screenshots

- Stock UPX 4.2.4 reports `NotPackedException`, but executable inspection exposes `UPX 5.30`, `UPX Team`, and `UPX!` alongside `RREA`.
- Inspection of a hash-named shared library also exposes UPX branding and `UPX!`.
- Ghidra relocation diagnostics expose Rust-mangled names containing `panic_bounds_check` and `rust_no_alloc_shim_is_unstable_v2`.

These observations establish visible fingerprints, not successful decompilation or a particular automatic language-classification result.
The [current header masking](../src/build.rs) deliberately leaves compressed data and runtime EOF markers unchanged.
The [current audit](../src/audit.rs) rejects static symbols but retains runtime imports/exports and reports selected Rust string hints.
Neither filename mangling nor static-symbol stripping addresses all of these leaks.

### UPX identifiers and structure

- [ ] Inventory identifying strings, magic values, the fixed `RREA` label, loader-stub signatures, and packed ELF layout across the executable and every bundled library.
- [ ] Trace each occurrence to optional branding, required attribution, format metadata, compressed data, or runtime markers before changing it.
- [ ] Remove avoidable identifiers at their source and rebuild the packer/stub, retaining legally required notices and updating the [verified source pin](../thirdparty/upx-source.json) for intentional source changes.
- [ ] For runtime-required signatures, design coordinated writer/reader changes instead of global byte replacement or another fixed identifying label.
- [ ] Evaluate structural detection separately from string detection, including stub instruction patterns and segment layout.
  If these still identify UPX, assess a more substantially changed stub or alternative packing backend before claiming the structure is concealed.

### Rust symbols and strings

- [ ] Classify Rust hints by origin: application, generated dependencies, precompiled standard library, dynamic symbols, diagnostics, and source paths.
- [ ] Plan deterministic opaque renaming for internal Rust dynamic symbols across the complete bundle, updating definitions, consumers, symbol hashes, version metadata, and relocations consistently before packing.
  Preserve externally required ABI names and runtime name lookups unless every consumer is accounted for.
  Adapt metadata audits to validate the declared rename map rather than weakening import/export checks.
- [ ] Remove avoidable source paths and implementation strings at compilation or generation time, investigating pass coverage where strings remain.
  Do not blindly overwrite loaded data, public CLI text, or required diagnostics.

### Acceptance gates

- [ ] Extend artifact audits to detect the observed UPX/RREA and Rust symbol fingerprints before publication, rejecting avoidable leaks rather than only warning.
  Record any necessary retained identifiers explicitly instead of reporting a fingerprint-free result.
- [ ] Repeat string scans, symbol inspection, signature detection, and Ghidra import using recorded tool versions on final executables, libraries, and recovered payloads.
  Include the matching bundled UPX version rather than relying only on the older tool shown in the screenshot.
- [ ] Require successful loading, relocation, allocation, panic/unwind, and behavior parity after each transformation.
  Failed stock unpacking alone is not acceptance, and removal of known signatures is not proof that Rust or UPX is unrecognizable.

## Investigate remaining exposure

- [ ] Inventory recognizable routine names, exported symbols, strings, and library roles after unpacking the executable and its dependencies.
- [ ] Separate required runtime interfaces and public CLI text from avoidable implementation leakage before changing visibility or metadata.
- [ ] Check representative encoding and decoding functions for actual obfuscation coverage after unpacking, rather than inferring protection from enabled compiler flags.
- [ ] Measure what filename mangling contributes when routine names and library roles remain identifiable.
- [ ] Reproduce the report's unpacking and decompilation claims, distinguishing UPX behavior from changes implemented by `cargo-rrea`.

## Measure costs and preserve behavior

- [ ] Compare build time, startup latency, throughput, memory use, and complete distribution size on matched workloads.
- [ ] Validate the Base116 case with binary round trips, partial chunks, Unicode mappings, wrapper handling, and malformed input.
- [ ] Retain relocation, dependency loading, and panic/unwind coverage in the [native integration tests](../tests/cli.rs).
- [ ] Publish measured results with original prompts and supporting logs, then revise the comparison's qualitative conclusions where warranted.