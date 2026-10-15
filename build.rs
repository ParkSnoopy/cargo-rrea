//! Prepare the obfuscating toolchain and embed the pinned UPX executable.

#[path = "src/support.rs"]
mod support;
#[path = "src/toolchain.rs"]
mod toolchain;

use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, ensure};

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
        source.join("CMakeLists.txt").is_file() && source.join(".git").exists(),
        "UPX submodule is missing; run git submodule update --init --recursive"
    );
    ensure!(
        env::var("HOST")? == env::var("TARGET")?,
        "cross-compilation is not supported"
    );
    ensure!(
        Path::new(&git(&source, &["rev-parse", "--show-toplevel"])?).canonicalize()?
            == source.canonicalize()?,
        "thirdparty/upx is not an initialized Git submodule"
    );
    ensure!(
        git(
            &source,
            &[
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ],
        )?
        .is_empty(),
        "UPX submodule or its vendor dependencies have uncommitted changes"
    );
    let revision = git(&source, &["rev-parse", "HEAD"])?;
    let git_directory = PathBuf::from(git(&source, &["rev-parse", "--absolute-git-dir"])?);
    let mut output = io::stdout().lock();
    writeln!(output, "cargo:rerun-if-changed={}", source.display())?;
    for path in [git_directory.join("HEAD"), git_directory.join("refs")] {
        writeln!(output, "cargo:rerun-if-changed={}", path.display())?;
    }
    for variable in ["CC", "CXX", "CFLAGS", "CXXFLAGS", "CARGO_NET_OFFLINE"] {
        writeln!(output, "cargo:rerun-if-env-changed={variable}")?;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    prepare_toolchain(&out_dir, &mut output)?;
    let build = out_dir.join("upx");
    let configuration = if env::var("PROFILE")? == "release" {
        "Release"
    } else {
        "Debug"
    };
    checked(
        Command::new("cmake")
            .arg("-S")
            .arg(&source)
            .arg("-B")
            .arg(&build)
            .args(["-G", "Ninja"])
            .arg(format!("-DCMAKE_BUILD_TYPE={configuration}")),
    )?;
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
    Ok(())
}
