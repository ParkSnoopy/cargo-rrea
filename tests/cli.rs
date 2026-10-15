//! Integration boundaries for manifest initialization and the native pipeline.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-rrea"))
}

fn cargo_cli() -> Result<Command> {
    let plugin_directory = Path::new(env!("CARGO_BIN_EXE_cargo-rrea"))
        .parent()
        .context("missing Cargo plugin directory")?;
    let path = std::env::var_os("PATH").context("missing PATH")?;
    let paths = std::iter::once(plugin_directory.to_path_buf()).chain(std::env::split_paths(&path));
    let mut command = Command::new("cargo");
    command
        .arg("rrea")
        .env("PATH", std::env::join_paths(paths)?);
    Ok(command)
}

fn success(command: &mut Command) -> Result<Output> {
    let output = command.output()?;
    ensure!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn fixture() -> Result<tempfile::TempDir> {
    let root = tempfile::tempdir()?;
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace"),
        root.path(),
    )?;
    Ok(root)
}

#[test]
fn init_updates_only_the_workspace_root_and_rejects_invalid_input() -> Result<()> {
    let root = fixture()?;
    let manifest = root.path().join("Cargo.toml");
    let member = root.path().join("app/Cargo.toml");
    let member_before = fs::read(&member)?;
    success(cli().args(["rrea", "init", "--manifest-path"]).arg(&member))?;
    let updated = fs::read_to_string(&manifest)?;
    ensure!(
        updated.contains("# Preserve the application's panic strategy."),
        "init removed the panic strategy comment"
    );
    let document = updated.parse::<toml_edit::DocumentMut>()?;
    ensure!(
        document["profile"]["release"]["debug"].as_integer() == Some(0),
        "init did not disable release debug information: {document}"
    );
    ensure!(
        document["profile"]["release"]["strip"].as_str() == Some("symbols"),
        "init did not enable symbol stripping: {document}"
    );
    ensure!(
        document["profile"]["release"]["panic"].as_str() == Some("unwind"),
        "init changed the panic strategy: {document}"
    );
    ensure!(
        fs::read(&member)? == member_before,
        "init changed the member manifest"
    );
    let repeated = success(cli().args(["init", "--manifest-path"]).arg(&manifest))?;
    ensure!(
        String::from_utf8(repeated.stdout)?.starts_with("unchanged "),
        "repeated init did not report unchanged content"
    );
    ensure!(
        fs::read_to_string(&manifest)? == updated,
        "init is not idempotent"
    );
    fs::write(&manifest, "[workspace\n")?;
    let invalid = cli()
        .args(["init", "--manifest-path"])
        .arg(&manifest)
        .output()?;
    ensure!(!invalid.status.success(), "init accepted invalid TOML");
    ensure!(
        fs::read_to_string(&manifest)? == "[workspace\n",
        "init changed invalid TOML"
    );
    ensure!(
        !cli().arg("--definitely-unknown").output()?.status.success(),
        "CLI accepted an unknown option"
    );
    ensure!(
        !cli()
            .args(["--workspace", "init"])
            .output()?
            .status
            .success(),
        "init accepted build selections"
    );
    Ok(())
}

#[test]
fn build_subcommand_exposes_options_and_rejects_invalid_invocations() -> Result<()> {
    for args in [
        vec![],
        vec!["--bins"],
        vec!["build"],
        vec!["build", "--release", "--definitely-unknown"],
        vec!["build", "--release", "--bin", "fixture", "--bins"],
        vec!["build", "--release", "--exclude", "fixture-app"],
        vec!["init", "--release"],
    ] {
        let output = cli().args(&args).output()?;
        ensure!(
            output.status.code() == Some(2),
            "invalid invocation {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = success(cargo_cli()?.args(["build", "--help"]))?;
    let help = String::from_utf8(output.stdout)?;
    for flag in [
        "--release",
        "--package",
        "--features",
        "--all-features",
        "--no-default-features",
        "--offline",
        "--frozen",
        "--locked",
        "--target",
        "--jobs",
    ] {
        ensure!(help.contains(flag), "build help omits {flag}: {help}");
    }
    Ok(())
}

#[test]
fn cargo_bootstrap_prepares_a_runnable_matching_toolchain() -> Result<()> {
    let cache = Path::new(env!("RREA_DOWNLOADS"));
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(cache.join("compiler.json"))?)?;
    let tag = record["tag"].as_str().context("missing compiler tag")?;
    let toolchain = cache.join(format!("compiler-{tag}/stage1"));
    let host = cache
        .file_name()
        .and_then(|name| name.to_str())
        .context("missing host")?;
    ensure!(
        record["repository"] == "ParkSnoopy/rust_llvm-arkari_ollvm",
        "unexpected compiler source"
    );
    ensure!(
        toolchain
            .join("lib/rustlib")
            .join(host)
            .join("lib")
            .is_dir(),
        "missing matching sysroot"
    );
    let version = success(
        Command::new(toolchain.join(format!("bin/rustc{}", std::env::consts::EXE_SUFFIX)))
            .arg("-vV"),
    )?;
    ensure!(
        String::from_utf8(version.stdout)?
            .lines()
            .any(|line| line == format!("host: {host}")),
        "compiler host mismatch"
    );
    success(
        Command::new(
            toolchain
                .join("lib/rustlib")
                .join(host)
                .join("bin")
                .join(format!("llvm-objcopy{}", std::env::consts::EXE_SUFFIX)),
        )
        .arg("--version"),
    )?;
    Ok(())
}

#[test]
fn offline_cache_miss_and_unsupported_targets_do_not_publish() -> Result<()> {
    let root = fixture()?;
    let cache = tempfile::tempdir()?;
    let detached = root
        .path()
        .join(format!("cargo-rrea{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_cargo-rrea"), &detached)?;
    let output = Command::new(detached)
        .current_dir(root.path())
        .args(["build", "--release", "--offline"])
        .env("XDG_CACHE_HOME", cache.path())
        .env("LOCALAPPDATA", cache.path())
        .output()?;
    ensure!(!output.status.success(), "offline cache miss succeeded");
    ensure!(
        String::from_utf8_lossy(&output.stderr).contains("not cached"),
        "offline miss did not report a missing cache"
    );
    ensure!(
        distributions(root.path())?.is_empty(),
        "offline cache miss published output"
    );
    let output = cli()
        .current_dir(root.path())
        .args([
            "build",
            "--release",
            "--target",
            "aarch64-unknown-linux-gnu",
        ])
        .output()?;
    ensure!(!output.status.success(), "unsupported target succeeded");
    ensure!(
        String::from_utf8_lossy(&output.stderr).contains("cross-compilation is not supported"),
        "unsupported target was not diagnosed"
    );
    Ok(())
}

fn distributions(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let directory = root.join("target/rrea");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut results = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("dist-") {
            results.push(entry.path());
        }
    }
    results.sort();
    Ok(results)
}

#[test]
#[ignore = "compiles and runs owned fixtures with the bootstrapped compiler and bundled UPX"]
fn real_native_pipeline_preserves_behavior_and_failed_build_keeps_outputs() -> Result<()> {
    for selection in [
        &[][..],
        &[
            "--package",
            "fixture-app",
            "--bins",
            "--locked",
            "--offline",
        ],
    ] {
        verify_native_pipeline(selection)
            .with_context(|| format!("native build selection: {selection:?}"))?;
    }
    Ok(())
}

fn verify_native_pipeline(selection: &[&str]) -> Result<()> {
    let root = fixture()?;
    let baseline = root.path().join("baseline");
    success(
        Command::new("cargo")
            .current_dir(root.path())
            .args([
                "build",
                "--package",
                "fixture-app",
                "--bins",
                "--target-dir",
            ])
            .arg(&baseline),
    )?;
    success(cargo_cli()?.current_dir(root.path()).arg("init"))?;
    let cache = tempfile::tempdir()?;
    let output = success(
        cargo_cli()?
            .current_dir(root.path())
            .env("XDG_CACHE_HOME", cache.path())
            .env("LOCALAPPDATA", cache.path())
            .env("CARGO_NET_OFFLINE", "true")
            .args(["build", "--release"])
            .args(selection),
    )?;
    writeln!(
        io::stderr().lock(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    )?;
    let paths: Vec<_> = String::from_utf8(output.stdout)?
        .lines()
        .map(std::path::PathBuf::from)
        .collect();
    ensure!(
        paths.len() == 2,
        "expected both fixture executables, got {paths:?}"
    );
    verify_behavior(&baseline, &paths)?;
    let distribution = verify_distribution(root.path(), &paths)?;
    verify_relocation(root.path(), &distribution, &paths)?;
    let report = &distribution.report;
    ensure!(
        report["obfuscation"]
            .as_array()
            .context("missing flags")?
            .len()
            == 8,
        "unexpected obfuscation flags: {}",
        report["obfuscation"]
    );
    let invocation_log = report["compiler_invocations"]
        .as_str()
        .context("missing invocation log")?;
    let log = fs::read_to_string(distribution.path.join(invocation_log))?;
    ensure!(log.contains("--irobf-fla"), "missing flattening pass");
    ensure!(
        log.contains("fixture_logic"),
        "dependency was not obfuscated"
    );
    ensure!(
        !log.contains("--crate-name\",\"fixture_literal"),
        "procedural macro received target passes"
    );
    ensure!(
        !log.contains("--crate-name\",\"build_script_build"),
        "build script received target passes"
    );
    verify_failed_publication(root.path(), &distribution)?;
    Ok(())
}

fn verify_behavior(baseline: &Path, paths: &[PathBuf]) -> Result<()> {
    let suffix = std::env::consts::EXE_SUFFIX;
    for path in paths {
        let filename = path.file_name().context("missing artifact name")?;
        let original = baseline.join("debug").join(filename);
        let cases = if filename == format!("fixture{suffix}").as_str() {
            vec![
                "0",
                "1",
                "2",
                "7",
                "91",
                "65535",
                "4294967295",
                "--catch",
                "--panic",
            ]
        } else {
            vec!["7"]
        };
        for argument in cases {
            let expected = Command::new(&original).arg(argument).output()?;
            let actual = Command::new(path).arg(argument).output()?;
            ensure!(
                actual.status.code() == expected.status.code(),
                "exit: {path:?} {argument}: actual={:?}, expected={:?}",
                actual.status.code(),
                expected.status.code()
            );
            ensure!(
                actual.stdout == expected.stdout,
                "stdout: {path:?} {argument}: actual={:?}, expected={:?}",
                actual.stdout,
                expected.stdout
            );
            if argument == "--panic" {
                ensure!(
                    String::from_utf8_lossy(&actual.stderr).contains("fixture panic marker"),
                    "missing panic marker: {path:?}"
                );
            }
        }
    }
    Ok(())
}

struct Distribution {
    path: PathBuf,
    artifacts: Vec<PathBuf>,
    packed_libraries: Vec<PathBuf>,
    report: serde_json::Value,
}

fn verify_distribution(root: &Path, paths: &[PathBuf]) -> Result<Distribution> {
    let published = distributions(root)?;
    ensure!(
        published.len() == 1,
        "expected exactly one published distribution, got {published:?}"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(published[0].join("build.json"))?)?;
    let compiler_record: serde_json::Value = serde_json::from_slice(&fs::read(
        Path::new(env!("RREA_DOWNLOADS")).join("compiler.json"),
    )?)?;
    ensure!(
        report["compiler"] == compiler_record,
        "runtime did not reuse the bootstrapped toolchain"
    );
    let artifacts = report["artifacts"]
        .as_array()
        .context("missing artifacts")?;
    ensure!(
        artifacts.len() == 5,
        "expected five published artifacts, got {artifacts:?}"
    );
    ensure!(
        report["upx"]["repository"] == "upx/upx",
        "unexpected UPX repository: {}",
        report["upx"]
    );
    ensure!(
        report["upx"]["revision"] == env!("RREA_UPX_REVISION"),
        "unexpected UPX revision: {}",
        report["upx"]
    );
    let mut packed_libraries = Vec::new();
    let mut published_paths = Vec::new();
    for artifact in artifacts {
        let path = published[0].join(artifact["path"].as_str().context("missing artifact path")?);
        let data = fs::read(&path)?;
        let packed = artifact["packed"]
            .as_bool()
            .context("missing packing status")?;
        ensure!(
            data.windows(4).any(|bytes| bytes == b"UPX!") == packed,
            "packing signature mismatch: {path:?}, packed={packed}"
        );
        ensure!(
            packed != paths.contains(&path),
            "incorrect executable/library packing role: {path:?}, packed={packed}"
        );
        published_paths.push(path.clone());
        if packed {
            packed_libraries.push(path);
        }
    }
    ensure!(
        packed_libraries.len() == 3,
        "expected three packed libraries, got {packed_libraries:?}"
    );
    Ok(Distribution {
        path: published[0].clone(),
        artifacts: published_paths,
        packed_libraries,
        report,
    })
}

fn verify_relocation(root: &Path, distribution: &Distribution, paths: &[PathBuf]) -> Result<()> {
    let suffix = std::env::consts::EXE_SUFFIX;
    // Relocation and a missing dependency check exclude accidental build-tree loading.
    let relocated = root.join("relocated");
    copy_tree(&distribution.path.join("fixture-app"), &relocated)?;
    for path in paths {
        let filename = path.file_name().context("missing executable name")?;
        let expected = success(Command::new(path).arg("7"))?;
        let actual = success(
            Command::new(relocated.join(filename))
                .current_dir(root)
                .env_remove("LD_LIBRARY_PATH")
                .env_remove("LD_PRELOAD")
                .arg("7"),
        )?;
        ensure!(
            actual.stdout == expected.stdout,
            "relocated output mismatch: {path:?}: actual={:?}, expected={:?}",
            actual.stdout,
            expected.stdout
        );
    }
    let dependency = distribution
        .packed_libraries
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains("fixture_shared"))
        })
        .context("missing fixture dependency")?;
    let dependency = relocated.join(dependency.file_name().context("missing library name")?);
    let hidden = dependency.with_extension("held");
    fs::rename(&dependency, &hidden)?;
    let missing = Command::new(relocated.join(format!("fixture{suffix}")))
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .arg("7")
        .output()?;
    fs::rename(&hidden, &dependency)?;
    ensure!(
        !missing.status.success(),
        "executable did not require its packed dependency"
    );
    Ok(())
}

fn verify_failed_publication(root: &Path, distribution: &Distribution) -> Result<()> {
    let prior_contents: Vec<_> = distribution
        .artifacts
        .iter()
        .map(fs::read)
        .collect::<std::io::Result<_>>()?;
    let failed = cli()
        .current_dir(root)
        .args([
            "build",
            "--release",
            "--package",
            "fixture-app",
            "--bins",
            "--offline",
        ])
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", "-Cdebuginfo=2")
        .output()?;
    ensure!(
        !failed.status.success(),
        "conflicting protection flags succeeded"
    );
    ensure!(
        String::from_utf8_lossy(&failed.stderr).contains("conflicts with rrea protection"),
        "missing protection conflict diagnostic"
    );
    ensure!(
        distributions(root)? == [distribution.path.clone()],
        "failed build changed published distributions"
    );
    for (path, original) in distribution.artifacts.iter().zip(prior_contents) {
        ensure!(
            fs::read(path)? == original,
            "failed build changed artifact {path:?}"
        );
    }
    Ok(())
}
