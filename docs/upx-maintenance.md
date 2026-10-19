# Maintaining the UPX fork dependency

## Ownership

[ParkSnoopy/upx](https://github.com/ParkSnoopy/upx) owns the packer changes and matching generated runtime stubs.
The consuming repository carries no source patch, generated-stub overlay, or patch-application step.
[build.rs](../build.rs) validates the pinned recursive source tree, builds it directly with CMake/Ninja, and embeds the executable.
The [source pin](../thirdparty/upx-source.json) covers generated stubs and vendor sources as well as packer code.
Reports identify the fork repository, source revision, and embedded executable SHA-256.
Keep licensing and attribution in the fork unchanged.

## Updating the dependency

Make packer changes in the fork, not in the consuming submodule.
Regenerate affected native headers and tracked maps/dumps there before publishing the commit.
Follow the fork's [source-build instructions](../thirdparty/upx/README.SRC) and [stub Makefile](../thirdparty/upx/src/stub/Makefile).
Do not hand-edit generated numeric arrays or copy stubs from another revision.

Resolve the intended fork commit and initialize its recursive vendor revisions.
Update the gitlink and source pin together using the [canonical digest procedure](architecture.md#source-development).
Normal Cargo builds must not fetch moving branch updates or apply uncommitted sibling-checkout changes.

The pinned fork must satisfy cargo-rrea's existing coordinated format-tag and PE64 lossless-recovery contract.
If the fork has not incorporated those changes, keep validation failing rather than accepting stock headers or weakening byte-identical recovery.
Opaque packing is enabled only for Linux x86_64 executables and is disabled by `--no-opaque`.
Shared libraries retain normal compression, and `--no-upx` skips both modes.
The fork's [runtime extension](../thirdparty/upx/src/extension/README.md) defines its startup process-name guard and opaque block format.

## Acceptance

- Verify the fork revision and complete source digest before accepting a native build.
- Run ordinary tests, strict all-target Clippy, and the ignored native pipeline.
- Preserve PE recovery equality for original, mangled, and differing-IAT/lookup inputs.
- Preserve Linux loading, relocated dependencies, allocation, and panic/unwind behavior.
- Exercise default opaque executable packing, `--no-opaque`, normally compressed libraries, and `--no-upx`.
- Compare the reported opaque state with the actual validated pack header and preserve byte-identical recovery in both modes.
- Test old or corrupted format headers as failures, while allowing marker text inside application payloads and overlays.
- Exercise the consuming CLI packing itself, not only the small fixture.
- Run `cargo package --allow-dirty --locked --offline` for intentional local uncommitted checks, without `--no-verify`.
- Verify the extracted crate contains fork sources and generated stubs, no UPX Git metadata, and no local overlays.
- Change a packaged vendor/source file and require integrity rejection before downloads.
- Keep native Windows runtime verification separate from cross-platform PE parsing and recovery.

The format audit follows the pinned UPX header versions and search regions.
An upstream format-version change also requires reviewing [audit.rs](../src/audit.rs), not just regenerating stubs.
