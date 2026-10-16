//! Prepare the obfuscating toolchain and embed the pinned UPX executable.

#[path = "src/support.rs"]
mod support;
#[path = "src/toolchain.rs"]
mod toolchain;

use std::{
    env,
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpxSource {
    revision: String,
    sha256: String,
}

fn source_files(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name() == OsStr::new(".git") {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            source_files(root, &path, files)?;
        } else {
            ensure!(
                kind.is_file(),
                "UPX source contains a link or special file: {}",
                path.display()
            );
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("UPX source path must be UTF-8")?
                .replace('\\', "/");
            ensure!(
                !relative.contains(['\t', '\r', '\n']),
                "invalid UPX source path: {relative}"
            );
            files.push(relative);
        }
    }
    Ok(())
}

fn source_revision(source: &Path, output: &mut impl Write) -> Result<String> {
    let pin_path = source.with_file_name("upx-source.json");
    writeln!(output, "cargo:rerun-if-changed={}", pin_path.display())?;
    let pin: UpxSource = serde_json::from_slice(&fs::read(pin_path)?)?;
    if source.join(".git").exists() {
        ensure!(
            Path::new(&git(source, &["rev-parse", "--show-toplevel"])?).canonicalize()?
                == source.canonicalize()?,
            "thirdparty/upx is not an initialized Git submodule"
        );
        ensure!(
            git(
                source,
                &[
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--ignore-submodules=none"
                ]
            )?
            .is_empty(),
            "UPX submodule or its vendor dependencies have uncommitted changes"
        );
        ensure!(
            git(source, &["rev-parse", "HEAD"])? == pin.revision,
            "UPX checkout does not match the pinned revision {}",
            pin.revision
        );
        let git_directory = PathBuf::from(git(source, &["rev-parse", "--absolute-git-dir"])?);
        for path in [git_directory.join("HEAD"), git_directory.join("refs")] {
            writeln!(output, "cargo:rerun-if-changed={}", path.display())?;
        }
    }
    let mut files = Vec::new();
    source_files(source, source, &mut files)?;
    files.sort();
    let mut manifest = Vec::new();
    for path in files {
        writeln!(
            manifest,
            "{path}\t{}",
            toolchain::sha256(&source.join(&path))?
        )?;
    }
    let digest = toolchain::digest(manifest.as_slice())?;
    ensure!(
        digest == pin.sha256,
        "UPX source checksum mismatch for {}: expected {}, got {digest}",
        pin.revision,
        pin.sha256
    );
    Ok(pin.revision)
}

fn git(source: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "cannot inspect UPX submodule: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn patched_source(source: &Path, directory: &Path, overlay: &[u8]) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let staged = tempfile::Builder::new()
        .prefix(".source-")
        .tempdir_in(directory)?;
    let mut files = Vec::new();
    source_files(source, source, &mut files)?;
    for relative in files {
        let target = staged.path().join(&relative);
        fs::create_dir_all(target.parent().context("UPX source has no parent")?)?;
        fs::copy(source.join(relative), target)?;
    }
    let patch = directory.join("upx.patch");
    fs::write(&patch, overlay)?;
    support::checked_output(
        Command::new("git")
            .current_dir(staged.path())
            .env("GIT_CEILING_DIRECTORIES", directory)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["apply", "--whitespace=error-all"])
            .arg(&patch),
    )
    .context(
        "cannot apply thirdparty/upx.patch; Git and matching pinned UPX sources are required",
    )?;
    let destination = directory.join("source");
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::rename(staged.path(), &destination)?;
    Ok(destination)
}

fn checked(command: &mut Command) -> Result<()> {
    ensure!(
        command.status()?.success(),
        "UPX build command failed: {command:?}"
    );
    Ok(())
}

fn prepare_toolchain(out_dir: &Path, output: &mut impl Write) -> Result<()> {
    let host = toolchain::host()?;
    let mut target_directory = out_dir
        .ancestors()
        .find(|path| path.file_name() == Some(OsStr::new("build")))
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("unexpected Cargo OUT_DIR layout")?;
    if target_directory.file_name() == Some(OsStr::new(host)) {
        target_directory = target_directory
            .parent()
            .context("target directory has no parent")?;
    }
    let downloads = target_directory.join("downloads").join(host);
    writeln!(
        output,
        "cargo:rerun-if-changed={}",
        downloads.join("compiler.json").display()
    )?;
    let offline = env::var("CARGO_NET_OFFLINE").as_deref() == Ok("true");
    let (compiler, objcopy, _) = toolchain::acquire(&downloads, host, offline)?;
    support::checked_output(Command::new(objcopy).arg("--version"))?;
    let version = support::checked_output(Command::new(compiler).arg("-vV"))?;
    ensure!(
        String::from_utf8(version.stdout)?
            .lines()
            .any(|line| line == format!("host: {host}")),
        "downloaded compiler host does not match {host}"
    );
    writeln!(
        output,
        "cargo:rustc-env=RREA_DOWNLOADS={}",
        downloads.display()
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let source = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("thirdparty/upx");
    ensure!(
        source.join("CMakeLists.txt").is_file(),
        "UPX sources are missing; initialize recursive submodules when building from a Git checkout"
    );
    ensure!(
        env::var("HOST")? == env::var("TARGET")?,
        "cross-compilation is not supported"
    );
    let mut output = io::stdout().lock();
    let revision = source_revision(&source, &mut output)?;
    writeln!(output, "cargo:rerun-if-changed={}", source.display())?;
    writeln!(output, "cargo:rerun-if-changed=thirdparty/upx.patch")?;
    for variable in ["CC", "CXX", "CFLAGS", "CXXFLAGS", "CARGO_NET_OFFLINE"] {
        writeln!(output, "cargo:rerun-if-env-changed={variable}")?;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    // Cargo can reuse OUT_DIR when switching between a checkout and its extracted crate.
    let overlay = fs::read(source.with_file_name("upx.patch"))?;
    let patch_sha256 = toolchain::digest(overlay.as_slice())?;
    let source_key = toolchain::digest(
        format!(
            "{}\n{revision}\n{patch_sha256}",
            source.to_str().context("UPX source path must be UTF-8")?
        )
        .as_bytes(),
    )?;
    let build = out_dir.join("upx").join(source_key);
    let source = patched_source(&source, &build, &overlay)?;
    let build = build.join("build");
    let configuration = if env::var("PROFILE")? == "release" {
        "Release"
    } else {
        "Debug"
    };
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&source)
        .arg("-B")
        .arg(&build)
        .args(["-G", "Ninja"])
        .arg(format!("-DCMAKE_BUILD_TYPE={configuration}"));
    if cfg!(target_os = "linux") {
        for (language, variable) in [("C", "CFLAGS"), ("CXX", "CXXFLAGS")] {
            let flags = env::var(variable).unwrap_or_default();
            configure.arg(format!(
                "-DCMAKE_{language}_FLAGS={flags} -ffile-prefix-map=\"{}=/upx\" -ffile-prefix-map=\"{}=/upx-build\"",
                source.display(), build.display()
            ));
        }
    }
    checked(&mut configure).context(
        "UPX configuration requires CMake, Ninja, and working C and C++ compilers; install these prerequisites and check CC/CXX and CFLAGS/CXXFLAGS",
    )?;
    prepare_toolchain(&out_dir, &mut output)?;
    checked(
        Command::new("cmake")
            .arg("--build")
            .arg(&build)
            .args(["--target", "upx", "--parallel"])
            .arg(env::var("NUM_JOBS")?),
    )?;
    let executable = build.join(format!("upx{}", env::consts::EXE_SUFFIX));
    ensure!(
        executable.is_file(),
        "UPX build produced no expected executable"
    );
    writeln!(
        output,
        "cargo:rustc-env=RREA_UPX_PATH={}",
        executable.display()
    )?;
    writeln!(output, "cargo:rustc-env=RREA_UPX_REVISION={revision}")?;
    writeln!(
        output,
        "cargo:rustc-env=RREA_UPX_PATCH_SHA256={patch_sha256}"
    )?;
    Ok(())
}
