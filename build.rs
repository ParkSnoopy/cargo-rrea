//! Build the pinned UPX submodule and embed its native executable.

use std::{
    env,
    error::Error,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn git(source: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cannot inspect UPX submodule: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn checked(command: &mut Command) -> Result<()> {
    if !command.status()?.success() {
        return Err(format!("UPX build command failed: {command:?}").into());
    }
    Ok(())
}

fn main() -> Result<()> {
    let source = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("thirdparty/upx");
    if !source.join("CMakeLists.txt").is_file() || !source.join(".git").exists() {
        return Err("UPX submodule is missing; run git submodule update --init --recursive".into());
    }
    if env::var("HOST")? != env::var("TARGET")? {
        return Err("cross-compilation is not supported".into());
    }
    if Path::new(&git(&source, &["rev-parse", "--show-toplevel"])?).canonicalize()?
        != source.canonicalize()?
    {
        return Err("thirdparty/upx is not an initialized Git submodule".into());
    }
    if !git(
        &source,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?
    .is_empty()
    {
        return Err("UPX submodule or its vendor dependencies have uncommitted changes".into());
    }
    let revision = git(&source, &["rev-parse", "HEAD"])?;
    let git_directory = PathBuf::from(git(&source, &["rev-parse", "--absolute-git-dir"])?);
    let mut output = io::stdout().lock();
    writeln!(output, "cargo:rerun-if-changed={}", source.display())?;
    for path in [git_directory.join("HEAD"), git_directory.join("refs")] {
        writeln!(output, "cargo:rerun-if-changed={}", path.display())?;
    }
    for variable in ["CC", "CXX", "CFLAGS", "CXXFLAGS"] {
        writeln!(output, "cargo:rerun-if-env-changed={variable}")?;
    }
    let build = PathBuf::from(env::var("OUT_DIR")?).join("upx");
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
    if !executable.is_file() {
        return Err("UPX build produced no expected executable".into());
    }
    writeln!(
        output,
        "cargo:rustc-env=RREA_UPX_PATH={}",
        executable.display()
    )?;
    writeln!(output, "cargo:rustc-env=RREA_UPX_REVISION={revision}")?;
    Ok(())
}
