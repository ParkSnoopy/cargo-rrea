# Maintaining the UPX overlay

## Ownership

[upx.patch](../thirdparty/upx.patch) contains source changes only.
It changes native packer tags, emitted branding, native runtime mapping names, and PE64 lossless import/export recovery.
It does not implement a new compressor, hide the loader's structure, or promise compatibility for upstream targets outside cargo-rrea's native x86_64 platforms.
Keep licensing and attribution in the upstream sources unchanged.

[upx-stubs.zip](../thirdparty/upx-stubs.zip) contains regenerated native headers and the corresponding changed tracked maps/dumps.
These are build products, not code to merge by hand.
[upx-stubs.json](../thirdparty/upx-stubs.json) binds the archive SHA-256 to the exact upstream revision and source-patch SHA-256.
[build.rs](../build.rs) verifies pristine sources first, validates that binding, applies the source patch in a disposable copy, then installs the verified generated files.
Archive extraction rejects traversal, links, special files, duplicate output files, and excessive expansion through the shared [extractor](../src/toolchain.rs).
The bundle may replace only existing generated files under `src/stub`, not stub source files.
All inputs are watched and read on each build-script run.
A changed patch with stale stubs fails before compiler acquisition or native compilation.
Reports include separate upstream, patch, stub-archive, and executable identities.

The source-only unified diff is 382 lines / 17,293 bytes, compared with the original 4,716 lines / 348,462 bytes.
The accompanying ZIP is 70,659 bytes.
Its five generated headers and five changed tracked auxiliary files were reproduced byte-for-byte in a fresh clone.
Splitting generated data reduces textual merge conflicts, not the need to regenerate and test it when upstream changes.

## Regeneration

Work in a disposable clone, never in the checked-out submodule.
Start at the revision in [upx-source.json](../thirdparty/upx-source.json) and initialize its recursive vendor submodules.
Apply the source patch with `git apply --whitespace=error-all`.
For an upstream upgrade, resolve source conflicts there and review any changed loader/header conventions before rebuilding.
Do not carry old generated headers into the new source revision.

The current generator is upstream [stubtools v20221212](https://github.com/upx/upx-stubtools/releases/tag/v20221212).
Its `bin-upx-20221212.tar.xz` SHA-256 is `509e06639118a79d8e79489a400e134c6d3ca36bad2c6ec29648d7c1e5b81afa`.
Download from that release, verify the digest, and extract it into the disposable workspace.
The legacy tools require a working 32-bit Linux loader and zlib runtime.
Do not replace them with the host compiler and assume identical output.
The upstream `src/stub/Makefile` owns compiler, linker, object-info embedding, and `bin2h` arguments.

Before formatting or exporting the overlay, run upstream's pinned formatter in check-only mode.
With `STUBTOOLS` naming the verified extracted tools directory, check the current overlay files covered by upstream's formatter target:

```sh
(cd "$UPX_WORK" && UPX_CLANG_FORMAT="$STUBTOOLS/clang-format-15.0.6" \
  bash misc/scripts/upx-clang-format.sh --dry-run --Werror \
  src/conf.h src/packer.cpp src/pefile.cpp src/pefile.h)
```

If this fails, apply the same wrapper with `-i` and `--lines=START:END` only to reported ranges changed by the overlay, then repeat the check.
Compare with pristine upstream to distinguish existing formatting from introduced violations.
Follow the file exclusions in upstream's `src/Makefile` rather than formatting the whole tree, assembly, or generated headers.
This is a formatting check, not a `clang-tidy` analysis.

With `UPX_WORK` naming the patched clone, `STUBTOOLS` the verified extracted tools directory, and `STUB_RUNTIME` the matching legacy runtime library directory, rebuild all native paths:

```sh
PATH="$STUBTOOLS:$PATH" LD_LIBRARY_PATH="$STUB_RUNTIME" \
make -B -C "$UPX_WORK/src/stub" \
  STUBS='amd64-linux.elf-entry.h amd64-linux.elf-so_entry.h amd64-linux.elf-fold.h amd64-linux.elf-so_fold.h amd64-win64.pe.h' all
```

Review the source and generated diffs against the pinned revision in the disposable clone.
Export only the source paths as a unified diff without creating an overlay commit or stale mail headers.
The consuming repository's absolute path is `RREA_ROOT`:

```sh
git -C "$UPX_WORK" diff --binary -- \
  src/conf.h src/packer.cpp src/pefile.cpp src/pefile.h src/stub/src \
  > "$RREA_ROOT/thirdparty/upx.patch"
```

Generate the bundle from the generated files changed against that same pinned revision.
Use a new, empty `STUB_STAGE` directory and a new output archive rather than updating an old ZIP, which could retain obsolete entries:

```sh
git -C "$UPX_WORK" diff --name-only -- \
  src/stub ':!src/stub/src' > "$STUB_STAGE.files"
while IFS= read -r path; do
  mkdir -p "$STUB_STAGE/$(dirname "$path")"
  cp "$UPX_WORK/$path" "$STUB_STAGE/$path"
  TZ=UTC touch -t 198001010000 "$STUB_STAGE/$path"
done < "$STUB_STAGE.files"
(cd "$STUB_STAGE" && TZ=UTC zip -X -9 "$STUB_STAGE.zip" -@ < "$STUB_STAGE.files")
```

Replace the archive and update its manifest with the actual revision, patch hash, and ZIP hash.
For formatting confined to packer C++ code, retain the archive when all stub generator inputs are unchanged and update only its patch binding.
For upstream upgrades, also update the pristine source revision/digest using the [architecture procedure](architecture.md#source-development).
Retain comments identifying project-owned byte constants, such as `/* RREA */`.
Never hand-edit numeric generated arrays or remove a checksum check to accommodate mismatched stubs.
An outer `git diff --check` can flag legitimate context-line spaces inside the patch file.
Check the source diff in the disposable clone and actual strict patch application rather than rewriting patch transport syntax.

## Acceptance

- Rebuild in a second clean clone and compare every bundled file byte-for-byte.
- Verify that the patch, generated archive, and manifest agree before accepting a native build.
- Run ordinary tests, strict all-target Clippy, and the ignored native pipeline.
- Preserve PE recovery equality for original, mangled, and differing-IAT/lookup inputs.
- Preserve Linux loading, relocated dependencies, allocation, and panic/unwind behavior.
- Test old or corrupted format headers as failures, while allowing marker text inside application payloads and overlays.
- Exercise the consuming CLI packing itself, not only the small fixture.
- Run `cargo package --allow-dirty --locked --offline` for intentional local uncommitted checks, without `--no-verify`.
- Verify the extracted crate contains the bundle and no UPX Git metadata.
- Probe corrupt archives and stale patch/bundle pairings against that extracted source and require failure before downloads.
- Keep native Windows runtime verification separate from cross-platform PE parsing and recovery.

The format audit follows the pinned UPX header versions and search regions.
An upstream format-version change also requires reviewing [audit.rs](../src/audit.rs), not just regenerating stubs.
