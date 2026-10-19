# Maintaining the UPX fork dependency

## Ownership

[ParkSnoopy/upx](https://github.com/ParkSnoopy/upx) owns the packer changes and matching generated runtime stubs.
[build.rs](../build.rs) validates the pinned recursive source tree, builds it directly with CMake/Ninja, and embeds the executable.

| Record | Contents |
| --- | --- |
| [Source pin](../thirdparty/upx-source.json) | Fork revision and digest of packer code, generated stubs, and recursive vendors |
| Build report | Fork repository, source revision, and embedded executable SHA-256 |

Preserve the fork's licensing and attribution.

## Updating the dependency

1. Make packer changes in the fork.
2. Regenerate affected native headers and tracked maps/dumps using its [source-build instructions](../thirdparty/upx/README.SRC) and [stub Makefile](../thirdparty/upx/src/stub/Makefile).
3. Publish source changes and their matching generated artifacts together.
4. Resolve that fork commit in the consumer and initialize its recursive vendor revisions.
5. Update the gitlink and source pin together using the [canonical digest procedure](architecture.md#updating-upx).

Do not hand-edit generated numeric arrays or copy stubs from another revision.
Normal Cargo builds must not fetch moving branch updates or apply uncommitted sibling-checkout changes.

### Required format contract

The pinned fork must supply coordinated format tags and lossless PE64 recovery.
Reject incompatible revisions rather than accepting stock headers or weakening byte-identical recovery.

| Selection | Packing mode |
| --- | --- |
| Linux x86_64 executable | Opaque by default |
| `--no-opaque` | Normal executable compression |
| Shared library | Normal compression |
| `--no-upx` | Packing disabled |

The fork's [runtime extension](../thirdparty/upx/src/extension/README.md) defines its startup process-name guard and opaque block format.

## Acceptance

### Source and packaging

- Verify the fork revision and complete source digest before accepting a native build.
- Run ordinary tests, strict all-target Clippy, and the ignored native pipeline.
- Run `cargo package --allow-dirty --locked --offline` for intentional local uncommitted checks, without `--no-verify`.
- Verify the extracted crate contains fork sources and generated stubs, no UPX Git metadata, and no local overlays.
- Change a packaged vendor/source file and require integrity rejection before downloads.

### Runtime and recovery

- Preserve PE recovery equality for original, mangled, and differing-IAT/lookup inputs.
- Preserve Linux loading, relocated dependencies, allocation, and panic/unwind behavior.
- Exercise default opaque executable packing, `--no-opaque`, normally compressed libraries, and `--no-upx`.
- Compare the reported opaque state with the actual validated pack header and preserve byte-identical recovery in both modes.
- Exercise the consuming CLI packing itself, not only the small fixture.
- Keep native Windows runtime verification separate from cross-platform PE parsing and recovery.

### Format audits

- Reject old or corrupted format headers while allowing marker text inside application payloads and overlays.

The format audit follows the pinned UPX header versions and search regions.
An upstream format-version change also requires reviewing [audit.rs](../src/audit.rs), not just regenerating stubs.
