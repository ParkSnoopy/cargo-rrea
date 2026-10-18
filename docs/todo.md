# Evaluation and protection follow-ups

Work arising from the [reverse-engineering comparison](comparison.md) and supplied terminal screenshots.
The [executed evaluation](evaluation.md) and [machine-readable results](evaluation.json) distinguish measured findings from proposed protection work.
Unchecked tasks remain open.
Completed implementation work does not establish resistance to reverse engineering.

## Establish comparable evidence

- [x] Record source revision, compiler versions, profile settings, artifact hashes, and analysis tools for both builds.
  The snapshot covers six variants and every bundled artifact, with the pinned application [lockfile](../tests/fixtures/base116.lock).
- [ ] Give analysts equivalent instructions and information budgets, without identifying the protected build in advance.
  A [blinded-study protocol](evaluation.md#future-blinded-study) is defined, but no independent analyst study was run.
- [ ] Use identical recovery tasks: identify behavior, reconstruct the encoding and decoding rules, and validate recovered implementations against withheld inputs.
- [ ] Record time, tool failures, manual interventions, and correctness rather than comparing report length or emphasis.
  Runtime measurements and tool failures are recorded, but analyst recovery time and independent implementation correctness remain unmeasured.
- [ ] Add controlled build variants to separate stripping, packing, filename mangling, and compiler-obfuscation effects without changing the default protection policy.
  Six variants isolate the post-link transformations and compare filename policies.
  A same-linkage, same-profile compiler-pass control is still required to isolate obfuscation cost.
  Per-feature opt-outs now provide that build mechanism, including individual LLVM passes, stripping, packing, and both mangling policies.
  The historical six-variant evaluation has not been rerun with these controls.

## Fix immediate packer and language fingerprints

### Evidence retained from the screenshots

- Stock UPX 4.2.4 reports `NotPackedException`, but executable inspection exposes `UPX 5.30`, `UPX Team`, and `UPX!` alongside `RREA`.
- Inspection of a hash-named shared library also exposes UPX branding and `UPX!`.
- Ghidra relocation diagnostics expose Rust-mangled names containing `panic_bounds_check` and `rust_no_alloc_shim_is_unstable_v2`.

These observations establish visible fingerprints, not successful decompilation or a particular automatic language-classification result.
The screenshots predate the [UPX overlay](../thirdparty/upx.patch), which changes native runtime tags together with the packer and removes known identification strings.
The [current audit](../src/audit.rs) validates real pack headers and rejects old tags at those locations.
It records whole-file literal hints with offsets, retains runtime imports/exports, and reports selected Rust string hints.
Arbitrary payload text is not evidence of a packer header.
Neither filename mangling nor static-symbol stripping addresses all of these leaks.

### UPX identifiers and structure

- [x] Inventory known identifying strings, magic values, the fixed `RREA` label, loader bytes, and packed ELF layout across the executable and every bundled library.
  The Base116 inventory covers all 19 protected artifacts and their recovered payloads.
  The self-build additionally verifies cargo-rrea's own marker strings as application data.
- [x] Trace known emitted identifiers to optional branding, required attribution, format metadata, compressed data, or runtime markers before changing them.
  Preserve attribution in upstream sources and licenses.
  Do not reject arbitrary payload occurrences of the audit's own vocabulary.
- [x] Remove emitted native UPX banners, PE section labels, and Linux memfd names through a source overlay with rebuilt x86_64 stubs.
  Preserve upstream source notices and licenses, verify the pristine [source pin](../thirdparty/upx-source.json), and record the overlay hash separately.
- [x] Coordinate native packer/stub format tags and remove post-packing `RREA` rewriting instead of blindly replacing runtime bytes.
- [x] Keep the source patch manageable for upstream updates.
  The patch is 382 lines instead of 4,716, with generated headers/maps in a separately verified archive.
  Follow the [regeneration and acceptance procedure](upx-maintenance.md), including exact source/patch/archive binding and byte-identical regeneration.
- [ ] Evaluate structural detection separately from string detection, including stub instruction patterns and segment layout.
  Layout/disassembly inspection and live memfd probes establish that a recognizable loader remains despite empty native mapping names.
  Automated structural-detector coverage and an alternative-backend assessment remain open.
  Do not expand the source overlay into a loader rewrite or claim structural concealment from changed strings/tags.

### Rust symbols and strings

- [x] Classify Rust hints by origin: application, generated dependencies, precompiled standard library, dynamic symbols, diagnostics, and source paths.
  The [fingerprint policy](rust-fingerprints.md#current-policy) records observed origins and the limits of substring attribution.
- [x] Plan deterministic opaque renaming for internal Rust dynamic symbols across the complete bundle, updating definitions, consumers, symbol hashes, version metadata, and relocations consistently before packing.
  Preserve externally required ABI names and runtime name lookups unless every consumer is accounted for.
  Adapt metadata audits to validate the declared rename map rather than weakening import/export checks.
  The [design](rust-fingerprints.md#closed-bundle-symbol-map) includes ELF/PE obligations, atomic publication, exclusions, and acceptance gates.
- [ ] Implement and verify the closed-bundle symbol map on both native platforms.
  - [x] Implement default-on ELF renaming with exact-name exclusions, explicit opt-out, and collision rejection.
    Packed Rust fixtures rename 1,820 symbols with each library filename policy while preserving loading, allocation, panic/unwind, relocation, and failed publication.
    GNU/SysV hash, versioned, weak, TLS/IFUNC, ambiguous-provider, and `dlsym` boundaries have dedicated loader coverage.
  - [ ] Implement PE symbol renaming and establish native Windows acceptance.
- [ ] Remove avoidable source paths and implementation strings at compilation or generation time, investigating pass coverage where strings remain.
  Do not blindly overwrite loaded data, public CLI text, or required diagnostics.
  Local absolute paths and removable metadata are rejected by the default policy.
  The pinned compiler encrypts eligible Rust byte strings, with runtime parity and loaded-segment plaintext checks under default and opt-out policies.
  Precompiled standard-library strings and unsupported uses remain outside that coverage; concurrent first-use initialization remains an upstream risk.
  Unreferenced ELF `rust_metadata_*` exports are now removed together with their non-loaded `.rustc` payload instead of leaving dangling section references.

### Acceptance gates

- [ ] Extend artifact audits to detect the observed UPX/RREA and Rust symbol fingerprints before publication, rejecting avoidable leaks rather than only warning.
  Record any necessary retained identifiers explicitly instead of reporting a fingerprint-free result.
  - [x] Validate ELF symbol attributes, versions, and relocation targets against the declared rename/removal plan.
    Selected old names and removed compiler exports cannot survive the exact metadata comparison.
    Reports record symbol exclusions and metadata removals; unexpected or referenced compiler metadata fails before publication.
  PE symbol policy and structural-detector coverage remain open.
- [x] Reject old native pack-header tags and PE section labels before publication and verify direct recovery with the patched packer.
  Regress marker literals in legitimate payloads, corrupted headers, loader tags, and PE recovery.
  cargo-rrea now packs itself and builds a working fixture from its packed distribution.
  PE symbol policy and structural-detector coverage remain open.
- [x] Repeat known-string scans, symbol inspection, pack-format recognition checks, and Ghidra import using recorded tool versions on final executables, libraries, and recovered payloads.
  Include the matching bundled UPX version rather than relying only on the older tool shown in the screenshot.
  Ghidra imported all 40 selected inputs, but the baseline and two recovered libraries reached the analysis timeout.
  Import success is not successful semantic decompilation.
- [ ] Require successful loading, relocation, allocation, panic/unwind, and behavior parity after each transformation.
  Linux fixtures and the six-variant Base116 evaluation pass these applicable gates.
  Native Windows execution is still unverified, despite passing PE parsing, rewriting, packing, and exact-recovery fixtures.
  Failed stock unpacking alone is not acceptance, and removal of known signatures is not proof that Rust or UPX is unrecognizable.
- [ ] Diagnose intermittent signal termination of the packed `secondary` native fixture.
  It occurred with `--no-icall` and with the tight ELF layout; subsequent isolated runs and a complete feature matrix passed, which does not explain the earlier failures.
  Failed native fixtures are retained with full exit-status diagnostics for packed/recovered comparison; do not attribute the failure to LLVM, metadata rewriting, or UPX without a retained reproduction.

## Investigate remaining exposure

- [x] Inventory recognizable routine names, exported symbols, strings, and library roles after unpacking the executable and its dependencies.
  The recovered bundle exposes 2,556 export entries, including 2,537 Rust-style names.
- [x] Separate required runtime interfaces and public CLI text from avoidable implementation leakage before changing visibility or metadata.
- [ ] Check representative encoding and decoding functions for actual obfuscation coverage after unpacking, rather than inferring protection from enabled compiler flags.
  Decoder disassembly contains a large constant/state dispatcher.
  An equivalent encoder study and broader pass-coverage analysis remain open.
- [x] Measure what filename mangling contributes when routine names and library roles remain identifiable.
  Both filename policies preserve the same exported-symbol multiset.
  Names change, but Rust exports still reveal the Base116 library's role.
- [ ] Reproduce the report's unpacking and decompilation claims, distinguishing UPX behavior from changes implemented by `cargo-rrea`.
  Matching-packer recovery is byte-exact across the complete bundle.
  Ghidra import and bounded analysis were exercised, but complete semantic decompilation was not established.

## Measure costs and preserve behavior

- [ ] Compare build time, startup latency, throughput, memory use, and complete distribution size on matched workloads.
  Startup, encode/decode wall time, throughput, sampled RSS, and complete bundle sizes are recorded.
  Comparable clean build-time measurements remain open because the initial builds had different cache states and concurrent workloads.
- [x] Validate the Base116 case with binary round trips, partial chunks, Unicode mappings, wrapper handling, and malformed input.
  The [evaluator](../tests/evaluation/mod.rs) passes 52 binary/wrapper and 20 malformed-input cases per variant.
- [x] Retain relocation, dependency loading, and panic/unwind coverage in the [native integration tests](../tests/cli.rs).
- [ ] Publish measured results with original prompts and supporting logs, then revise the comparison's qualitative conclusions where warranted.
  The [measured report](evaluation.md), hash/timing snapshot, and updated comparison are maintained here.
  Full generated evidence is retained locally under `target/evaluation`.
  The original historical prompts are unavailable and must not be reconstructed as evidence.