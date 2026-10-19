# Rust fingerprint policy and symbol renaming

## Current policy

Library filename mangling and Rust symbol renaming are independent policies.
Both are enabled by default under the [maximum-feature policy](architecture.md#obfuscation-policy).

| Argument | Disabled policy |
| --- | --- |
| `--no-mangle library` | Library filename mangling |
| `--no-mangle symbol` | Rust dynamic-symbol renaming |
| `--no-mangle` | Both |

Symbol renaming covers eligible ELF64 Rust dynamic symbols across the reachable bundle using [symbol planning](../src/symbols.rs) and the existing [metadata writer](../src/mangle.rs).
Windows requires symbol renaming disabled before project builds or tool acquisition.

The historical [evaluation](evaluation.md) predates symbol renaming and found the same exported-symbol multiset with and without filename mangling.

### Audit scope

The default pipeline rejects:

- Retained static/debug metadata.
- Prohibited local build paths.
- Undeclared runtime import/export changes.
- Malformed packing metadata.

Stripping, remapping, and packing opt-outs disable only their corresponding checks.
Runtime-metadata preservation remains mandatory.
ELF audits also compare symbol addresses, sizes, binding/type/visibility, section ownership, GNU versions, and relocation targets/addends against the declared transformation.

Rust runtime hints are reported without overwriting loaded bytes.
Classify findings through imports/exports, section inventories, and offset-bearing string scans rather than substrings alone.

### Fingerprint origins

| Origin | Evidence and present treatment |
| --- | --- |
| Local workspace, Cargo cache, compiler, build directories | Compiler path remapping and exact-path rejection prevent local absolute-path leakage. Remapped relative package/source identities can still remain. |
| Application CLI and errors | Help, encoding wrappers, public output, and error messages are behavior. Keep them unless the application's specification changes. |
| Dependency and monomorphized core/alloc code | CSE encrypts eligible length-delimited constant byte arrays, including Rust strings, before indirect-global rewriting. Unsupported uses and optimizer-generated immediates can still retain plaintext. |
| Precompiled standard library | Its runtime diagnostics and Rust symbols survive stripping. It was not rebuilt with the target LLVM passes. |
| Dynamic symbol tables | Eligible names change only under the explicit closed-bundle policy. Versioned, weak, ambiguous, TLS/IFUNC, external, and explicitly retained names remain. |
| Compiler metadata exports | Non-loaded `.rustc` objects named `rust_metadata_*` are removed before stripping, after checking the complete bundle for imports and relocation references. Their names appear separately in `removed_metadata_symbols`. |
| Audit vocabulary and compressed payloads | Literal `RREA`, `UPX!`, `core::`, and similar matches can be application data. Record them rather than attributing them to the loader without structural evidence. |

### String transformation limits

- Preserve string lengths, embedded NULs, UTF-8, and observable diagnostics. `--no-cse` disables CSE independently of packing.
- Do not add `-Zfmt-debug=none`, disable panic/unwind, alter caller-location semantics, or overwrite loaded strings to reduce marker counts. These changes can break behavior, pointer/length validity, or diagnostics.
- The upstream lazy initializer is unsynchronized on concurrent first use. Single-threaded fixtures do not establish race-free initialization.
- Rebuilding the standard library requires a pinned source/toolchain workflow and separate behavior/performance evidence.

## Closed-bundle symbol map

The default build contract requires all selected Rust interfaces to be internal to the bundle except names explicitly preserved with repeated `--keep-symbol NAME` arguments.
`--no-mangle symbol` and bare `--no-mangle` opt out of this contract and cannot be combined with `--keep-symbol`.

- Explicitly preserve names used by runtime lookup, plugins, external consumers, or intentional Rust-symbol interposition. The static dependency graph cannot identify every such consumer.
- Keep-symbol values must identify existing Rust-mangled dynamic names. Misspellings fail before publication.
- Unmangled C/system ABI names are excluded.
- Reports retain renamed names and exclusion reasons, including known `rust_`/`__rust_` ABI names, outside the distribution.

### ELF eligibility

A selected name must meet every condition:

1. Demangler-validated v0 or hashed legacy spelling.
2. Exactly one loaded definition.
3. Strong global binding and default visibility.
4. Function or object type, without a GNU version.

Undefined consumers may use `STT_NOTYPE` and the unversioned zero GNU version index.
TLS, IFUNC, weak/unique bindings, non-default visibility, versioned names, and multiple definitions remain excluded.

### Name derivation

- Hash the original provider filename, including its Cargo suffix, and original symbol with domain-separated SHA-256.
- Prefix with `s` and use at most 32 hexadecimal characters, never exceeding the original string length.
- Exclude absolute paths, addresses, and traversal order from the hash.
- Reject collisions with original names or other replacements before writing artifacts.

### Cross-platform planning contract

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

- Rebuild `.dynstr`, `.dynsym`, GNU/SysV hash tables when present, and version definition/requirement records consistently.
- Reindex symbols when GNU hash bucket ordering requires it, updating every relocation, version-index array, and other index consumer together.
- Preserve binding, visibility, type, size, address, TLS semantics, IFUNC behavior, and relocation addends.
- Reuse the linker-time metadata reserve and reject invalid layouts. Do not overwrite pooled-string substrings or move executable code/unwind addresses.

#### Compiler metadata cleanup

Removing `.rustc` with `llvm-objcopy` alone can leave a dynamic compiler-metadata export with an invalid section index.
The planner therefore identifies only unreferenced, non-loaded, zero-address `rust_metadata_*` objects in that section, and the writer removes their dynamic records and rebuilds hashes before stripping.
Unexpected exported metadata or a bundled consumer/relocation referencing it fails instead of leaving a dangling export or deleting a runtime interface.

This cleanup applies while stripping is enabled, even when symbol renaming and library filename mangling are disabled.
`--no-strip` preserves compiler metadata and its exports together instead of producing dangling references.

### PE32+

**Not implemented.** PE symbol renaming must satisfy these acceptance requirements:

- Update named imports, delay imports, export names, hints, forwarders, and export-name/ordinal ordering together.
- Keep ordinal-only imports and externally required exports stable.
- Preserve IAT addresses and entries, relocation data, TLS callbacks, resources, and unwind tables.
- Regress bound-style IAT/lookup differences independently of import-name changes.
- Retain exact decompression equality using the patched packer.
- Verify native Windows loading, separately from cross-platform parsing.

## Atomicity and acceptance

The selected map is advisory until every member has been rewritten and checked in staging.

1. Transform the expected inspection through the map and compare actual definitions, consumers, bindings, and versions against it.
2. Retain the strict `same_runtime_metadata` check.
3. Require selected old names to disappear from relevant dynamic-symbol records and excluded names to remain unchanged.
4. Preserve the previous complete distribution if any artifact fails.

### Acceptance matrix

| Area | Required coverage |
| --- | --- |
| Policy | Default and opt-out selections, determinism, collision rejection |
| Resolution | Ambiguous/weak definitions and runtime-name lookup exclusions |
| ELF | Versioned symbols, TLS, and IFUNCs |
| PE | Delay imports, forwarders, ordinals, and native loading |
| Runtime | Allocation, relocation, panic/unwind, missing dependencies, and behavior on both native platforms |
| Cost | Loader cost and whole-bundle size |

### Existing evidence and open gates

- [ELF loader fixtures](../tests/symbols/mod.rs) cover GNU-only, SysV-only, and combined hashes, versioned symbols, weak bindings, TLS, IFUNCs, explicit `dlsym` preservation, ambiguous/external definitions, collisions, and referenced-metadata rejection. They compare loaded code/data/unwind bytes and addresses under eager and lazy binding.
- The [packed Rust fixture](../tests/cli.rs) covers both filename policies, allocation, panic/unwind, relocation, missing dependencies, republication, and failed-plan preservation. Its exercised bundle renames 1,820 Rust symbols per filename policy.
- Native Windows acceptance and a new full Base116 cost evaluation remain open.

Runtime strings, stable ABI names, and recognizable code remain outside the rename policy.
