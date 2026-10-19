# Evaluation and protection follow-ups

Work arising from the [reverse-engineering comparison](comparison.md) and supplied terminal screenshots.
The [executed evaluation](evaluation.md) and [machine-readable results](evaluation.json) distinguish measured findings from proposed protection work.

- [Comparable evidence](#establish-comparable-evidence)
- [Packer and language fingerprints](#fix-immediate-packer-and-language-fingerprints)
- [Remaining exposure](#investigate-remaining-exposure)
- [Costs and behavior](#measure-costs-and-preserve-behavior)

## Establish comparable evidence

- [x] Record source revision, compiler versions, profile settings, artifact hashes, and analysis tools for both builds.
  **Evidence:** six variants and every bundled artifact, with the pinned application [lockfile](../tests/fixtures/base116.lock).

- [ ] Give analysts equivalent instructions and information budgets, without identifying the protected build in advance.
  **Remaining:** execute the [blinded-study protocol](evaluation.md#future-blinded-study) with independent analysts.

- [ ] Use identical recovery tasks: identify behavior, reconstruct the encoding and decoding rules, and validate recovered implementations against withheld inputs.

- [ ] Record time, tool failures, manual interventions, and correctness rather than comparing report length or emphasis.
  **Remaining:** analyst recovery time and independent implementation correctness. Runtime measurements and tool failures are recorded.

- [ ] Add controlled build variants to separate stripping, packing, filename mangling, and compiler-obfuscation effects without changing the default protection policy.
  **Evidence:** six variants isolate post-link transformations and compare filename policies.
  **Remaining:** rerun with same-linkage, same-profile compiler-pass controls using the per-feature LLVM, stripping, packing, and mangling opt-outs.

## Fix immediate packer and language fingerprints

### Evidence retained from the screenshots

- Stock UPX 4.2.4 reports `NotPackedException`, but executable inspection exposes `UPX 5.30`, `UPX Team`, and `UPX!` alongside `RREA`.
- Inspection of a hash-named shared library also exposes UPX branding and `UPX!`.
- Ghidra relocation diagnostics expose Rust-mangled names containing `panic_bounds_check` and `rust_no_alloc_shim_is_unstable_v2`.

These observations establish visible fingerprints, not successful decompilation or a particular automatic language-classification result.
The screenshots predate the native packer changes now owned by the [UPX fork](upx-maintenance.md).

The [current audit](../src/audit.rs) rejects old tags at actual header locations, records offset-bearing payload hints, preserves runtime imports/exports, and reports selected Rust string hints.

### UPX identifiers and structure

- [x] Inventory known identifying strings, magic values, the fixed `RREA` label, loader bytes, and packed ELF layout across the executable and every bundled library.
  **Evidence:** all 19 protected Base116 artifacts and recovered payloads, plus cargo-rrea's own application marker strings in the self-build.

- [x] Trace known emitted identifiers to optional branding, required attribution, format metadata, compressed data, or runtime markers before changing them.
  Preserve upstream attribution and licenses. Permit audit vocabulary inside application payloads.

- [x] Remove emitted native UPX banners, PE section labels, and Linux memfd names through a source overlay with rebuilt x86_64 stubs.
  Preserve source notices and licenses, verify the [fork source pin](../thirdparty/upx-source.json), and record the fork revision and executable hash.

- [x] Coordinate native packer/stub format tags and remove post-packing `RREA` rewriting instead of blindly replacing runtime bytes.

- [x] Keep the source patch manageable for upstream updates.
  The fork now owns source changes and matching generated headers/maps. Follow its [pinning and acceptance procedure](upx-maintenance.md).

- [ ] Evaluate structural detection separately from string detection, including stub instruction patterns and segment layout.
  **Evidence:** layout/disassembly and memfd probes show a recognizable loader despite empty mapping names.
  **Remaining:** automated structural-detector coverage and alternative-backend assessment, scoped separately from a loader rewrite.

### Rust symbols and strings

- [x] Classify Rust hints by origin: application, generated dependencies, precompiled standard library, dynamic symbols, diagnostics, and source paths.
  The [fingerprint policy](rust-fingerprints.md#current-policy) records observed origins and the limits of substring attribution.

- [x] Plan deterministic opaque renaming for internal Rust dynamic symbols across the complete bundle, updating definitions, consumers, symbol hashes, version metadata, and relocations consistently before packing.
  The [design](rust-fingerprints.md#closed-bundle-symbol-map) covers ELF/PE obligations, atomic publication, explicit exclusions, and exact metadata audits.
  Preserve ABI names and runtime lookups unless every consumer is accounted for.

- [ ] Implement and verify the closed-bundle symbol map on both native platforms.
  - [x] Implement default-on ELF renaming with exact-name exclusions, explicit opt-out, and collision rejection.
    **Evidence:** 1,820 renamed symbols per filename policy with loading, allocation, panic/unwind, relocation, and failed-publication preservation. Loader tests cover GNU/SysV hashes, versioned/weak symbols, TLS/IFUNC, ambiguous providers, and `dlsym`.
  - [ ] Implement PE symbol renaming and establish native Windows acceptance.

- [ ] Remove avoidable source paths and implementation strings at compilation or generation time, investigating pass coverage where strings remain.
  **Evidence:** default path/metadata rejection, eligible Rust byte-string encryption with runtime/plaintext checks, and removal of unreferenced `rust_metadata_*` exports with their `.rustc` payload.
  **Remaining:** precompiled standard-library strings, unsupported uses, and the upstream concurrent-first-use risk.
  Preserve loaded data, public CLI text, and required diagnostics.

### Acceptance gates

- [ ] Extend artifact audits to detect the observed UPX/RREA and Rust symbol fingerprints before publication, rejecting avoidable leaks rather than only warning.
  Record necessary retained identifiers explicitly. PE symbol policy and structural detection remain open.
  - [x] Validate ELF symbol attributes, versions, and relocation targets against the declared rename/removal plan.
    Exact comparison rejects selected old names, removed compiler exports, and unexpected or referenced metadata. Reports retain exclusions and removal records.

- [x] Reject old native pack-header tags and PE section labels before publication and verify direct recovery with the patched packer.
  **Evidence:** legitimate payload markers, corrupt headers, loader tags, PE recovery, and cargo-rrea building a working fixture from its own packed distribution.

- [x] Repeat known-string scans, symbol inspection, pack-format recognition checks, and Ghidra import using recorded tool versions on final executables, libraries, and recovered payloads.
  **Evidence:** matching-packer scans and all 40 Ghidra imports. The baseline and two recovered libraries timed out; semantic decompilation remains unverified.

- [ ] Require successful loading, relocation, allocation, panic/unwind, and behavior parity after each transformation.
  **Evidence:** Linux fixtures, the six-variant Base116 evaluation, and PE parsing/rewriting/packing/recovery fixtures.
  **Remaining:** native Windows execution.

- [ ] Diagnose intermittent signal termination of the packed `secondary` native fixture.
  Failures occurred with `--no-icall` and the tight ELF layout. Later isolated and full-matrix passes did not establish the cause.
  Use retained failed fixtures and full exit statuses for packed/recovered comparison before assigning ownership to LLVM, metadata rewriting, or UPX.

## Investigate remaining exposure

- [x] Inventory recognizable routine names, exported symbols, strings, and library roles after unpacking the executable and its dependencies.
  **Evidence:** 2,556 export entries, including 2,537 Rust-style names, in the recovered historical bundle.

- [x] Separate required runtime interfaces and public CLI text from avoidable implementation leakage before changing visibility or metadata.

- [ ] Check representative encoding and decoding functions for actual obfuscation coverage after unpacking, rather than inferring protection from enabled compiler flags.
  **Evidence:** a large constant/state dispatcher in decoder disassembly.
  **Remaining:** equivalent encoder study and broader pass-coverage analysis.

- [x] Measure what filename mangling contributes when routine names and library roles remain identifiable.
  **Evidence:** both historical filename policies preserve the same export multiset and reveal the Base116 library's role.

- [ ] Reproduce the report's unpacking and decompilation claims, distinguishing UPX behavior from changes implemented by `cargo-rrea`.
  **Evidence:** byte-exact whole-bundle recovery and Ghidra import/bounded analysis.
  **Remaining:** complete semantic decompilation.

## Measure costs and preserve behavior

- [ ] Compare build time, startup latency, throughput, memory use, and complete distribution size on matched workloads.
  **Evidence:** startup, encode/decode wall time, throughput, sampled RSS, and bundle sizes.
  **Remaining:** comparable clean build times with matched cache states and concurrent workloads.

- [x] Validate the Base116 case with binary round trips, partial chunks, Unicode mappings, wrapper handling, and malformed input.
  The [evaluator](../tests/evaluation/mod.rs) passes 52 binary/wrapper and 20 malformed-input cases per variant.

- [x] Retain relocation, dependency loading, and panic/unwind coverage in the [native integration tests](../tests/cli.rs).

- [ ] Publish measured results with original prompts and supporting logs, then revise the comparison's qualitative conclusions where warranted.
  **Evidence:** the [measured report](evaluation.md), hash/timing snapshot, and comparison, with generated evidence local to `target/evaluation`.
  **Remaining:** unavailable historical prompts cannot be reconstructed as evidence.