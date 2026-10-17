# Rust fingerprint policy and symbol-renaming design

## Current policy

This is a design for future symbol renaming, not an implemented protection claim.
[Library mangling](../src/mangle.rs) changes library identities and their references, not function or data symbol names.
The [evaluation](evaluation.md) found the same exported-symbol multiset with and without filename mangling.

The pipeline rejects retained static/debug metadata, prohibited local build paths, changed runtime imports/exports, and malformed packing metadata.
It reports Rust runtime hints without overwriting loaded bytes.
A substring alone does not identify its origin: cargo-rrea itself contains the audit's search literals.
The report's imports/exports and section inventories, plus offset-bearing string scans, provide the evidence needed to classify a finding.

| Origin | Evidence and present treatment |
| --- | --- |
| Local workspace, Cargo cache, compiler, build directories | Compiler path remapping and exact-path rejection prevent local absolute-path leakage. Remapped relative package/source identities can still remain. |
| Application CLI and errors | Help, encoding wrappers, public output, and error messages are behavior. Keep them unless the application's specification changes. |
| Dependency and monomorphized core/alloc code | Recovered binaries can contain `library/core/src/...`, `library/alloc/src/...`, and `/deps/registry/src/...`. Enabled obfuscation passes do not establish complete string coverage. |
| Precompiled standard library | Its runtime diagnostics and Rust symbols survive stripping. It was not rebuilt with the target LLVM passes. |
| Dynamic symbol tables | Names are required by definitions and consumers at load time. Stripping static symbols does not remove these interfaces. |
| Audit vocabulary and compressed payloads | Literal `RREA`, `UPX!`, `core::`, and similar matches can be application data. Record them rather than attributing them to the loader without structural evidence. |

Do not silently add `-Zfmt-debug=none`, disable panic/unwind, alter caller-location semantics, or overwrite string bytes to reduce these counts.
Those changes can break observable behavior, invalidate pointers/lengths, or destroy diagnostics while leaving other fingerprints intact.
A future compile-time path/string policy needs explicit semantics, pinned compiler support, and representative diagnostic tests.
Rebuilding the standard library would also need a pinned source/toolchain workflow and its own behavior/performance evidence.

## Closed-bundle symbol map

1. Build the complete reachable distribution before choosing any names.
   Reuse [runtime-library discovery](../src/build.rs) rather than adding an independent dependency graph.
2. Identify definition/consumer pairs using actual binding, visibility, version, ordinal, and relocation metadata.
   Do not select a symbol solely because its spelling resembles `_R...` or legacy `_ZN...17h...E`.
   Preserve external/system symbols, public C ABI, weak/interposable or ambiguous definitions, and unaccounted runtime lookup names.
3. Require an explicit closed-world contract for eligible Rust interfaces.
   Dynamic plugins, `dlsym`/`GetProcAddress`, forwarders, and externally loaded libraries can be invisible to the static import graph.
   Unknown consumers must keep their names or fail selection, not silently receive a broken bundle.
4. Derive an ASCII identifier from a domain-separated hash of stable library identity, original symbol bytes, and version identity.
   Exclude file paths, link addresses, and build-machine state.
   Use one shared mapping for each resolved definition and all consumers, check collisions across the complete bundle, and fail rather than selecting an ad hoc fallback.
5. Keep that map in the build report, outside the distribution.
   It is reproducibility/audit metadata, not a secret key or encryption layer.

## Format-specific obligations

### ELF64

Extend the existing `object::build::elf::Builder` boundary in [mangle.rs](../src/mangle.rs) rather than adding a second ELF parser.
Rebuild `.dynstr`, `.dynsym`, both GNU and SysV hash tables when present, and version definition/requirement records consistently.
GNU hash bucket ordering can require symbol reindexing.
If indices move, update every relocation, version-index array, and other symbol-index consumer together.
Preserve binding, visibility, type, size, address, TLS semantics, IFUNC behavior, and relocation addends.
Do not overwrite substrings in a pooled string table or move executable code/unwind addresses to make room.
Reuse the existing linker-time metadata reserve and fail if a valid layout cannot be produced.

### PE32+

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
Only then consider changing a default.
Until this contract is implemented and verified, the project must continue to disclose retained Rust dynamic symbols.
