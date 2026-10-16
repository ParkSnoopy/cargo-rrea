# Evaluation and protection follow-ups

Proposed work arising from the [reverse-engineering comparison](comparison.md).
These are open tasks, not implemented protections or measured results.

## Establish comparable evidence

- [ ] Record source revision, compiler versions, profile settings, artifact hashes, and analysis tools for both builds.
- [ ] Give analysts equivalent instructions and information budgets, without identifying the protected build in advance.
- [ ] Use identical recovery tasks: identify behavior, reconstruct the encoding and decoding rules, and validate recovered implementations against withheld inputs.
- [ ] Record time, tool failures, manual interventions, and correctness rather than comparing report length or emphasis.
- [ ] Add controlled build variants to separate stripping, packing, filename mangling, and compiler-obfuscation effects without changing the default protection policy.

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