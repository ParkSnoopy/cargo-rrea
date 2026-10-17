# Rust fingerprint policy and symbol renaming

## Current policy

Library filename mangling and Rust symbol renaming are independent policies.
Both are enabled by default under the [maximum-feature policy](architecture.md#obfuscation-policy).
`--no-mangle-library` and `--no-mangle-symbols` disable them independently.
Symbol renaming covers eligible ELF64 Rust dynamic symbols across the reachable bundle using [symbol planning](../src/symbols.rs) and the existing [metadata writer](../src/mangle.rs).
Windows requires explicit `--no-mangle-symbols`; otherwise the unsupported feature fails before project builds or tool acquisition.
The historical [evaluation](evaluation.md) predates symbol renaming and found the same exported-symbol multiset with and without filename mangling.

The default pipeline rejects retained static/debug metadata, prohibited local build paths, undeclared runtime import/export changes, and malformed packing metadata.
Explicit stripping, remapping, or packing opt-outs disable only their corresponding checks, never runtime-metadata preservation.
ELF audits also compare symbol addresses, sizes, binding/type/visibility, section ownership, GNU versions, and relocation targets/addends against the declared transformation.
It reports Rust runtime hints without overwriting loaded bytes.
A substring alone does not identify its origin: cargo-rrea itself contains the audit's search literals.
The report's imports/exports and section inventories, plus offset-bearing string scans, provide the evidence needed to classify a finding.

| Origin | Evidence and present treatment |
| --- | --- |
| Local workspace, Cargo cache, compiler, build directories | Compiler path remapping and exact-path rejection prevent local absolute-path leakage. Remapped relative package/source identities can still remain. |
| Application CLI and errors | Help, encoding wrappers, public output, and error messages are behavior. Keep them unless the application's specification changes. |
| Dependency and monomorphized core/alloc code | Recovered binaries can contain `library/core/src/...`, `library/alloc/src/...`, and `/deps/registry/src/...`. Enabled obfuscation passes do not establish complete string coverage. |
| Precompiled standard library | Its runtime diagnostics and Rust symbols survive stripping. It was not rebuilt with the target LLVM passes. |
| Dynamic symbol tables | Eligible names change only under the explicit closed-bundle policy. Versioned, weak, ambiguous, TLS/IFUNC, external, and explicitly retained names remain. |
| Compiler metadata exports | Non-loaded `.rustc` objects named `rust_metadata_*` are removed before stripping, after checking the complete bundle for imports and relocation references. Their names appear separately in `removed_metadata_symbols`. |
| Audit vocabulary and compressed payloads | Literal `RREA`, `UPX!`, `core::`, and similar matches can be application data. Record them rather than attributing them to the loader without structural evidence. |

Do not silently add `-Zfmt-debug=none`, disable panic/unwind, alter caller-location semantics, or overwrite string bytes to reduce these counts.
Those changes can break observable behavior, invalidate pointers/lengths, or destroy diagnostics while leaving other fingerprints intact.
A future compile-time path/string policy needs explicit semantics, pinned compiler support, and representative diagnostic tests.
Rebuilding the standard library would also need a pinned source/toolchain workflow and its own behavior/performance evidence.

## Closed-bundle symbol map

The default build contract requires all selected Rust interfaces to be internal to the bundle except names explicitly preserved with repeated `--keep-symbol NAME` arguments.
`--no-mangle-symbols` opts out of this contract, and cannot be combined with `--keep-symbol`.
This includes runtime lookup, plugins, external consumers, and intentional Rust-symbol interposition.
The tool cannot infer those consumers from the static dependency graph.
Keep-symbol values must identify existing Rust-mangled dynamic names; misspellings fail before publication.
Unmangled C/system ABI names are never selected.
The report records renamed names and exclusion reasons, including known `rust_`/`__rust_` ABI names, outside the runtime distribution.

The ELF implementation selects only demangler-validated v0 or hashed legacy names with exactly one loaded definition, strong global/default-visible binding, no GNU version, and function or object type.
Undefined consumers can use `STT_NOTYPE` and the unversioned zero GNU version index without being treated as local definitions.
TLS, IFUNC, weak/unique bindings, non-default visibility, versioned names, and multiply defined names are retained rather than guessed at.
Names use a domain-separated SHA-256 of the original provider filename, including its Cargo suffix, and the original symbol.
An `s` prefix and at most 32 hexadecimal characters never exceed the original string length.
Absolute paths, addresses, and traversal order do not enter the hash.
Collisions with original names or another replacement fail before artifact writes.

The complete cross-platform contract remains:

1. Build the complete reachable distribution before choosing any names.
   Reuse [runtime-library discovery](../src/build.rs) rather than adding an independent dependency graph.
2. Identify definition/consumer pairs using actual binding, visibility, version, ordinal, and relocation metadata.
   Do not select a symbol solely because its spelling resembles `_R...` or legacy `_ZN...17h...E`.
   Preserve external/system symbols, public C ABI, weak/interposable or ambiguous definitions, and unaccounted runtime lookup names.
3. Require an explicit closed-world contract for eligible Rust interfaces.
   Dynamic plugins, `dlsym`/`GetProcAddress`, forwarders, and externally loaded libraries can be invisible to the static import graph.
   Unknown consumers must keep their names or fail selection, not silently receive a broken bundle.
4. Derive an ASCII identifier from a domain-separated hash of provider identity and original symbol bytes.
   The ELF subset excludes versioned names; any future support must include version identity in resolution and naming.
   Exclude file paths, link addresses, and build-machine state.
   Use one shared mapping for each resolved definition and all consumers, check collisions across the complete bundle, and fail rather than selecting an ad hoc fallback.
5. Keep that map in the build report, outside the distribution.
   It is reproducibility/audit metadata, not a secret key or encryption layer.

## Format-specific obligations

### ELF64

The implementation extends the existing `object::build::elf::Builder` boundary in [mangle.rs](../src/mangle.rs), with audits using the same crate's read API.
Rebuild `.dynstr`, `.dynsym`, both GNU and SysV hash tables when present, and version definition/requirement records consistently.
GNU hash bucket ordering can require symbol reindexing.
If indices move, update every relocation, version-index array, and other symbol-index consumer together.
Preserve binding, visibility, type, size, address, TLS semantics, IFUNC behavior, and relocation addends.
Do not overwrite substrings in a pooled string table or move executable code/unwind addresses to make room.
Reuse the existing linker-time metadata reserve and fail if a valid layout cannot be produced.

Removing `.rustc` with `llvm-objcopy` alone can leave a dynamic compiler-metadata export with an invalid section index.
The planner therefore identifies only unreferenced, non-loaded, zero-address `rust_metadata_*` objects in that section, and the writer removes their dynamic records and rebuilds hashes before stripping.
Unexpected exported metadata or a bundled consumer/relocation referencing it fails instead of leaving a dangling export or deleting a runtime interface.
This cleanup applies while stripping is enabled, even when symbol renaming and library filename mangling are disabled.
`--no-strip` preserves compiler metadata and its exports together instead of producing dangling references.

### PE32+

PE symbol renaming is not implemented; these remain acceptance requirements, not claims about the current library-filename rewriter.

Update named imports, delay imports, export names, hints, forwarders, and the export-name/ordinal ordering together.
Keep ordinal-only imports and externally required exports stable.
Preserve IAT addresses and entries, relocation data, TLS callbacks, resources, and unwind tables.
Regress bound-style IAT/lookup differences independently of import-name changes.
Retain exact decompression equality using the patched packer.
Cross-platform parsing is not a substitute for native Windows loading tests.

## Atomicity and acceptance

The selected map is advisory until every member has been rewritten and checked in staging.
Transform the expected inspection through the map, then compare actual definitions, consumers, bindings, and versions against it.
Do not disable or merely loosen `same_runtime_metadata`.
Every selected old name must disappear from the relevant dynamic-symbol records, while every excluded name remains unchanged.
An artifact failure must preserve the previous complete published distribution.

Acceptance needs default and opt-out policies, determinism, collision rejection, ambiguous/weak definitions, ELF versioned symbols, TLS/IFUNCs, PE delay imports/forwarders/ordinals, and runtime-name lookup exclusions.
Run real allocation, relocation, panic/unwind, missing-dependency, and behavior fixtures on both native platforms.
Evaluate loader cost and whole-bundle size, not only the executable filename.
Default-on selection does not establish these remaining cross-platform or cost-evaluation gates.
The [ELF loader fixtures](../tests/symbols/mod.rs) exercise GNU-only, SysV-only, and combined hashes, versioned symbols, weak bindings, TLS, IFUNCs, explicit `dlsym` preservation, ambiguous/external definitions, collisions, and referenced-metadata rejection.
They compare loaded code/data/unwind bytes and addresses and run both eager and lazy symbol binding.
The [packed Rust fixture](../tests/cli.rs) exercises symbol renaming with both filename policies, allocation, panic/unwind, relocation, missing dependencies, republication, and failed-plan preservation.
Its exercised bundle renames 1,820 Rust symbols per filename policy.
These fixtures do not replace a new full Base116 cost evaluation or native Windows acceptance.
Retained runtime strings, stable ABI names, and recognizable code still prevent a fingerprint-free claim.
