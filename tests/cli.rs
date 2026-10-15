//! Integration boundaries for manifest initialization and the native pipeline.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-rrea"))
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
    assert!(updated.contains("# Preserve the application's panic strategy."));
    let document = updated.parse::<toml_edit::DocumentMut>()?;
    assert_eq!(
        document["profile"]["release"]["debug"].as_integer(),
        Some(0)
    );
    assert_eq!(
        document["profile"]["release"]["strip"].as_str(),
        Some("symbols")
    );
    assert_eq!(
        document["profile"]["release"]["panic"].as_str(),
        Some("unwind")
    );
    assert_eq!(fs::read(&member)?, member_before);
    let repeated = success(cli().args(["init", "--manifest-path"]).arg(&manifest))?;
    assert!(String::from_utf8(repeated.stdout)?.starts_with("unchanged "));
    assert_eq!(fs::read_to_string(&manifest)?, updated);
    fs::write(&manifest, "[workspace\n")?;
    let invalid = cli()
        .args(["init", "--manifest-path"])
        .arg(&manifest)
        .output()?;
    assert!(!invalid.status.success());
    assert_eq!(fs::read_to_string(&manifest)?, "[workspace\n");
    assert!(!cli().arg("--definitely-unknown").output()?.status.success());
    assert!(
        !cli()
            .args(["--workspace", "init"])
            .output()?
            .status
            .success()
    );
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
        .arg("--offline")
        .env("XDG_CACHE_HOME", cache.path())
        .env("LOCALAPPDATA", cache.path())
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not cached"));
    assert!(distributions(root.path())?.is_empty());
    let output = cli()
        .current_dir(root.path())
        .args(["--target", "aarch64-unknown-linux-gnu"])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cross-compilation is not supported"));
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
    let cache = tempfile::tempdir()?;
    let output = success(
        cli()
            .current_dir(root.path())
            .env("XDG_CACHE_HOME", cache.path())
            .env("LOCALAPPDATA", cache.path())
            .args([
                "--package",
                "fixture-app",
                "--bins",
                "--locked",
                "--offline",
            ]),
    )?;
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    let paths: Vec<_> = String::from_utf8(output.stdout)?
        .lines()
        .map(std::path::PathBuf::from)
        .collect();
    assert_eq!(paths.len(), 2);
    let suffix = std::env::consts::EXE_SUFFIX;
    for path in &paths {
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
            assert_eq!(
                actual.status.code(),
                expected.status.code(),
                "exit: {path:?} {argument}"
            );
            assert_eq!(
                actual.stdout, expected.stdout,
                "stdout: {path:?} {argument}"
            );
            if argument == "--panic" {
                assert!(String::from_utf8_lossy(&actual.stderr).contains("fixture panic marker"));
            }
        }
    }
    let published = distributions(root.path())?;
    assert_eq!(published.len(), 1);
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
    assert_eq!(artifacts.len(), 4);
    assert_eq!(report["upx"]["repository"], "upx/upx");
    assert_eq!(report["upx"]["revision"], env!("RREA_UPX_REVISION"));
    let mut packed_libraries = Vec::new();
    for artifact in artifacts {
        let path = published[0].join(artifact["path"].as_str().context("missing artifact path")?);
        let data = fs::read(&path)?;
        let packed = artifact["packed"]
            .as_bool()
            .context("missing packing status")?;
        assert_eq!(data.windows(4).any(|bytes| bytes == b"UPX!"), packed);
        assert_eq!(packed, !paths.contains(&path));
        if packed {
            packed_libraries.push(path);
        }
    }
    assert_eq!(packed_libraries.len(), 2);
    // Relocation and a missing dependency check exclude accidental build-tree loading.
    let relocated = root.path().join("relocated");
    copy_tree(&published[0].join("fixture-app"), &relocated)?;
    for path in &paths {
        let filename = path.file_name().context("missing executable name")?;
        let expected = success(Command::new(path).arg("7"))?;
        let actual = success(
            Command::new(relocated.join(filename))
                .current_dir(root.path())
                .env_remove("LD_LIBRARY_PATH")
                .env_remove("LD_PRELOAD")
                .arg("7"),
        )?;
        assert_eq!(actual.stdout, expected.stdout);
    }
    let dependency = packed_libraries
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains("fixture_logic"))
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
    assert!(
        !missing.status.success(),
        "executable did not require its packed dependency"
    );
    assert_eq!(
        report["obfuscation"]
            .as_array()
            .context("missing flags")?
            .len(),
        8
    );
    let invocation_log = report["compiler_invocations"]
        .as_str()
        .context("missing invocation log")?;
    let log = fs::read_to_string(published[0].join(invocation_log))?;
    assert!(log.contains("--irobf-fla"));
    assert!(log.contains("fixture_logic"));
    assert!(!log.contains("--crate-name\",\"fixture_literal"));
    assert!(!log.contains("--crate-name\",\"build_script_build"));
    let published_paths: Vec<_> = artifacts
        .iter()
        .map(|artifact| {
            Ok(published[0].join(artifact["path"].as_str().context("missing artifact path")?))
        })
        .collect::<Result<_>>()?;
    let prior_contents: Vec<_> = published_paths
        .iter()
        .map(fs::read)
        .collect::<std::io::Result<_>>()?;
    let failed = cli()
        .current_dir(root.path())
        .args(["--package", "fixture-app", "--bins", "--offline"])
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", "-Cdebuginfo=2")
        .output()?;
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("conflicts with rrea protection"));
    assert_eq!(distributions(root.path())?, published);
    for (path, original) in published_paths.iter().zip(prior_contents) {
        assert_eq!(fs::read(path)?, original);
    }
    eprintln!(
        "verified compiler/UPX pipeline, both executables, panic/unwind behavior, and failed-publication boundary"
    );
    Ok(())
}
