//! Integration boundaries for temporary build profiles and the native pipeline.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

const OPT_OUTS: &[&str] = &[
    "--no-indbr",
    "--no-icall",
    "--no-indgv",
    "--no-cse",
    "--no-fla",
    "--no-cie",
    "--no-cfe",
    "--no-mangle-library",
    "--no-mangle-symbols",
    "--no-upx",
    "--no-strip",
    "--no-remap-paths",
];

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
fn build_rejects_invalid_manifests_without_modifying_them() -> Result<()> {
    let root = fixture()?;
    let manifest = root.path().join("Cargo.toml");
    let member = root.path().join("app/Cargo.toml");
    let member_before = fs::read(&member)?;
    fs::write(&manifest, "[workspace\n")?;
    let invalid = cli()
        .args(["build", "--release", "--manifest-path"])
        .arg(&member)
        .output()?;
    ensure!(!invalid.status.success(), "build accepted invalid TOML");
    ensure!(
        fs::read_to_string(&manifest)? == "[workspace\n",
        "build changed invalid TOML"
    );
    ensure!(
        fs::read(&member)? == member_before,
        "failed build changed the member manifest"
    );
    Ok(())
}

#[test]
fn build_subcommand_exposes_options_and_rejects_invalid_invocations() -> Result<()> {
    for args in [
        vec![],
        vec!["init"],
        vec!["rrea", "init"],
        vec!["--bins"],
        vec!["build"],
        vec!["build", "--release", "--definitely-unknown"],
        vec!["build", "--release", "--bin", "fixture", "--bins"],
        vec!["build", "--release", "--exclude", "fixture-app"],
        vec!["build", "--release", "--mangle-library"],
        vec!["build", "--no-mangle-library"],
        vec!["build", "--release", "--mangle-symbols"],
        vec![
            "build",
            "--release",
            "--no-mangle-symbols",
            "--keep-symbol",
            "missing",
        ],
    ] {
        let output = cli().args(&args).output()?;
        ensure!(
            output.status.code() == Some(2),
            "invalid invocation {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let root_help = success(cli().arg("--help"))?;
    ensure!(
        !String::from_utf8(root_help.stdout)?.contains("init"),
        "help still advertises manifest initialization"
    );
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
        "--keep-symbol",
    ]
    .into_iter()
    .chain(OPT_OUTS.iter().copied())
    {
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
#[ignore = "compiles an isolated cargo install with a missing native compiler"]
fn cargo_install_rejects_missing_native_compiler_before_downloading() -> Result<()> {
    let root = tempfile::tempdir()?;
    let output = Command::new("cargo")
        .args(["install", "--debug", "--offline", "--locked", "--path"])
        .arg(env!("CARGO_MANIFEST_DIR"))
        .arg("--root")
        .arg(root.path().join("installation"))
        .arg("--target-dir")
        .arg(root.path().join("target"))
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .env("CC", root.path().join("missing-c-compiler"))
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;
    let stderr = String::from_utf8(output.stderr)?;
    ensure!(!output.status.success(), "missing compiler was accepted");
    ensure!(
        stderr.contains("UPX configuration requires CMake, Ninja, and working C and C++ compilers"),
        "native prerequisites were not diagnosed before compiler acquisition: {stderr}"
    );
    ensure!(
        !root.path().join("target/downloads").exists()
            && !stderr.contains("resolving ParkSnoopy/")
            && !stderr.contains("downloading compiler"),
        "missing native prerequisites reached compiler acquisition: {stderr}"
    );
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
        .args(cfg!(windows).then_some("--no-mangle-symbols"))
        .env("XDG_CACHE_HOME", cache.path())
        .env("LOCALAPPDATA", cache.path())
        .output()?;
    ensure!(!output.status.success(), "offline cache miss succeeded");
    ensure!(
        String::from_utf8_lossy(&output.stderr).contains("not cached"),
        "offline miss did not report a missing cache"
    );
    ensure!(
        distributions(&root.path().join("target"))?.is_empty(),
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

fn distributions(target: &Path) -> Result<Vec<std::path::PathBuf>> {
    let directory = target.join("rrea");
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
            "--manifest-path",
            "app/Cargo.toml",
        ],
        &["--no-mangle-library"],
    ] {
        let mut selection = selection.to_vec();
        if cfg!(windows) {
            selection.push("--no-mangle-symbols");
        }
        verify_native_pipeline(&selection)
            .with_context(|| format!("native build selection: {selection:?}"))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "runs every obfuscation opt-out and the all-disabled policy through the native pipeline"]
fn real_obfuscation_opt_out_pipeline_preserves_behavior() -> Result<()> {
    for flag in OPT_OUTS {
        verify_native_pipeline(&[flag]).with_context(|| format!("opt-out {flag}"))?;
        writeln!(io::stderr().lock(), "verified opt-out {flag}")?;
    }
    verify_native_pipeline(OPT_OUTS)?;
    writeln!(io::stderr().lock(), "verified all-disabled policy")?;
    Ok(())
}

fn verify_native_pipeline(selection: &[&str]) -> Result<()> {
    let root = fixture()?;
    let manifest = fs::read(root.path().join("Cargo.toml"))?;
    let member = fs::read(root.path().join("app/Cargo.toml"))?;
    let target = root.path().join(if selection.is_empty() {
        "target"
    } else {
        "custom-target"
    });
    let manifest_directory = if selection.contains(&"--manifest-path") {
        root.path().join("app")
    } else {
        root.path().to_path_buf()
    };
    let dist = manifest_directory.join("target/release/dist");
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

    if cfg!(target_os = "linux") && selection.contains(&"--manifest-path") {
        fs::create_dir_all(root.path().join(".cargo"))?;
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/elf-tight-layout.toml"),
            root.path().join(".cargo/config.toml"),
        )?;
    }
    let cache = tempfile::tempdir()?;
    let output = success(
        cargo_cli()?
            .current_dir(root.path())
            .env("XDG_CACHE_HOME", cache.path())
            .env("LOCALAPPDATA", cache.path())
            .env("CARGO_NET_OFFLINE", "true")
            .env("CARGO_PROFILE_RELEASE_PANIC", "abort")
            .env("CARGO_TARGET_DIR", &target)
            .args(["build", "--release"])
            .args(selection),
    )?;
    io::stderr().write_all(&output.stderr)?;
    let paths: Vec<_> = String::from_utf8(output.stdout)?
        .lines()
        .map(std::path::PathBuf::from)
        .collect();
    ensure!(
        paths.len() == 2,
        "expected both fixture executables, got {paths:?}"
    );
    verify_behavior(&baseline, &paths)?;
    let distribution = verify_distribution(&target, &dist, &paths)?;
    verify_symbol_report(
        &distribution.report,
        !selection.contains(&"--no-mangle-symbols"),
    )?;
    ensure!(
        distribution.report["mangle_library"] == !selection.contains(&"--no-mangle-library"),
        "incorrect mangling report"
    );
    verify_relocation(root.path(), &distribution, &paths)?;
    verify_obfuscation(&distribution, selection)?;
    verify_republication(
        &manifest_directory,
        &target,
        &baseline,
        &paths,
        &distribution,
    )?;
    if selection.is_empty() {
        verify_policy_switch(
            &manifest_directory,
            &target,
            &baseline,
            &paths,
            &distribution,
        )?;
    }
    ensure!(
        fs::read(root.path().join("Cargo.toml"))? == manifest
            && fs::read(root.path().join("app/Cargo.toml"))? == member,
        "successful or failed build changed a project manifest"
    );
    Ok(())
}

fn verify_policy_switch(
    root: &Path,
    target: &Path,
    baseline: &Path,
    paths: &[PathBuf],
    original: &Distribution,
) -> Result<()> {
    let kept = original.report["symbol_mangling"]["renamed"]
        .as_object()
        .context("missing symbol map")?
        .keys()
        .find(|name| name.contains("fixture_logic"))
        .context("missing fixture Rust symbol")?;
    for selection in [&["--no-fla"][..], &[]] {
        let before = distributions(target)?;
        success(
            cargo_cli()?
                .current_dir(root)
                .env("CARGO_TARGET_DIR", target)
                .args([
                    "build",
                    "--release",
                    "--offline",
                    "--bins",
                    "--keep-symbol",
                    kept,
                ])
                .args(selection),
        )?;
        let reports: Vec<_> = distributions(target)?
            .into_iter()
            .filter(|path| !before.contains(path))
            .collect();
        ensure!(
            reports.len() == 1,
            "policy switch did not publish one report"
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(reports[0].join("build.json"))?)?;
        ensure!(
            report["symbol_mangling"]["retained"][kept] == "Requested",
            "default-on keep-symbol was not honored"
        );
        verify_symbol_report(&report, true)?;
        verify_obfuscation(
            &Distribution {
                path: reports[0].clone(),
                artifacts: original.artifacts.clone(),
                report,
            },
            selection,
        )?;
        verify_behavior(baseline, paths)?;
    }
    Ok(())
}

fn verify_obfuscation(distribution: &Distribution, selection: &[&str]) -> Result<()> {
    let report = &distribution.report;
    ensure!(
        report["panic_strategy"] == "unwind",
        "incorrect shared-runtime panic strategy"
    );
    let disabled: BTreeSet<_> = selection
        .iter()
        .filter_map(|flag| flag.strip_prefix("--no-"))
        .collect();
    let reported: BTreeSet<_> = report["disabled_obfuscation"]
        .as_array()
        .context("missing disabled features")?
        .iter()
        .map(|name| name.as_str().context("invalid feature"))
        .collect::<Result<_>>()?;
    ensure!(disabled == reported, "incorrect disabled feature report");
    let mut expected: Vec<_> = ["indbr", "icall", "indgv", "cse", "fla", "cie", "cfe"]
        .into_iter()
        .filter(|name| !disabled.contains(name))
        .map(|name| format!("-Cllvm-args=--irobf-{name}"))
        .collect();
    if !expected.is_empty() {
        expected.insert(0, "-Cllvm-args=--irobf".to_owned());
    }
    ensure!(
        report["obfuscation"] == serde_json::to_value(&expected)?,
        "incorrect LLVM report"
    );
    let invocation_log = report["compiler_invocations"]
        .as_str()
        .context("missing invocation log")?;
    let log = fs::read_to_string(distribution.path.join(invocation_log))?;
    ensure!(
        log.contains("fixture_logic"),
        "dependency invocations are missing"
    );
    for line in log.lines() {
        let args: Vec<String> = serde_json::from_str(line)?;
        ensure!(
            !args
                .iter()
                .any(|arg| matches!(arg.as_str(), "panic=abort" | "-Cpanic=abort")),
            "abort strategy reached a target compiler invocation"
        );
        let actual: Vec<_> = args
            .iter()
            .filter(|arg| arg.starts_with("-Cllvm-args="))
            .collect();
        ensure!(
            actual == expected.iter().collect::<Vec<_>>(),
            "compiler policy drift"
        );
        ensure!(
            args.iter()
                .any(|arg| arg.starts_with("--remap-path-prefix="))
                != disabled.contains("remap-paths"),
            "incorrect path remapping"
        );
    }
    ensure!(
        !log.contains("--crate-name\",\"fixture_literal"),
        "procedural macro received target passes"
    );
    ensure!(
        !log.contains("--crate-name\",\"build_script_build"),
        "build script received target passes"
    );
    Ok(())
}

fn verify_republication(
    manifest_directory: &Path,
    target: &Path,
    baseline: &Path,
    paths: &[PathBuf],
    distribution: &Distribution,
) -> Result<()> {
    let dist = manifest_directory.join("target/release/dist");
    let opt_outs = policy_arguments(&distribution.report)?;
    fs::write(dist.join("stale.pdb"), b"not a runtime artifact")?;
    let repeated = success(
        cargo_cli()?
            .current_dir(manifest_directory)
            .env("CARGO_TARGET_DIR", target)
            .args([
                "build",
                "--release",
                "--offline",
                "--package",
                "fixture-app",
                "--bins",
            ])
            .args(&opt_outs),
    )?;
    ensure!(
        String::from_utf8(repeated.stdout)?
            .lines()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
            == paths,
        "rebuild changed executable paths"
    );
    ensure!(
        !dist.join("stale.pdb").exists(),
        "rebuild retained a nonessential file"
    );
    verify_behavior(baseline, paths)?;
    verify_rebuilt_policy(target, distribution)?;
    ensure!(
        distribution.artifacts.iter().all(|path| path.is_file())
            && fs::read_dir(&dist)?.count() == distribution.artifacts.len(),
        "rebuild changed library names or retained stale files"
    );
    verify_failed_publication(manifest_directory, target, distribution)?;
    success(
        cargo_cli()?
            .current_dir(manifest_directory)
            .env("CARGO_TARGET_DIR", target)
            .args([
                "build",
                "--release",
                "--offline",
                "--package",
                "fixture-app",
                "--bin",
                "secondary",
            ])
            .args(&opt_outs),
    )?;
    ensure!(
        fs::read_dir(&dist)?.count() == 4
            && !dist
                .join(format!("fixture{}", std::env::consts::EXE_SUFFIX))
                .exists(),
        "single-binary build retained unselected runtime artifacts"
    );
    verify_behavior(
        baseline,
        &[dist.join(format!("secondary{}", std::env::consts::EXE_SUFFIX))],
    )?;
    Ok(())
}

fn policy_arguments(report: &serde_json::Value) -> Result<Vec<String>> {
    report["disabled_obfuscation"]
        .as_array()
        .context("missing disabled features")?
        .iter()
        .map(|name| {
            Ok(format!(
                "--no-{}",
                name.as_str().context("invalid feature")?
            ))
        })
        .collect()
}

fn verify_rebuilt_policy(target: &Path, original: &Distribution) -> Result<()> {
    let reports: Vec<_> = distributions(target)?
        .into_iter()
        .filter(|path| *path != original.path)
        .collect();
    ensure!(reports.len() == 1, "expected one repeated build");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(reports[0].join("build.json"))?)?;
    ensure!(
        report["disabled_obfuscation"] == original.report["disabled_obfuscation"]
            && report["obfuscation"] == original.report["obfuscation"],
        "republication changed policy"
    );
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
    report: serde_json::Value,
}

fn verify_distribution(target: &Path, dist: &Path, paths: &[PathBuf]) -> Result<Distribution> {
    let published = distributions(target)?;
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
        "expected two executables and three runtime libraries, got {artifacts:?}"
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
    ensure!(
        report["upx"]["patch_sha256"]
            == digest(include_bytes!("../thirdparty/upx.patch").as_slice())?,
        "unexpected UPX patch provenance"
    );
    ensure!(
        report["upx"]["stubs_sha256"]
            == digest(include_bytes!("../thirdparty/upx-stubs.zip").as_slice())?,
        "unexpected UPX generated-stub provenance"
    );
    let mut published_paths = Vec::new();
    for artifact in artifacts {
        let path = published[0]
            .join(artifact["path"].as_str().context("missing artifact path")?)
            .canonicalize()?;

        ensure!(
            path.parent() == Some(dist),
            "runtime artifact is not in the manifest's distribution directory: {path:?}"
        );
        verify_packing(&path, artifact, &report, paths.contains(&path))?;
        verify_library_name(&path, artifact, &report)?;
        published_paths.push(path);
    }
    ensure!(
        fs::read_dir(dist)?.count() == published_paths.len(),
        "distribution contains nonessential files"
    );
    ensure!(
        paths.iter().all(|path| published_paths.contains(path)),
        "report omits an executable"
    );
    ensure!(
        fs::read_dir(&published[0])?.count() == 3,
        "report directory contains runtime artifacts"
    );
    Ok(Distribution {
        path: published[0].clone(),
        artifacts: published_paths,
        report,
    })
}

fn original_exports(artifact: &serde_json::Value, report: &serde_json::Value) -> Result<String> {
    let mut reverse = BTreeMap::new();
    if let Some(names) = report["symbol_mangling"]["renamed"].as_object() {
        for (original, name) in names {
            reverse.insert(
                name.as_str().context("invalid symbol map")?,
                original.as_str(),
            );
        }
    }
    Ok(artifact["unpacked_audit"]["exports"]
        .as_array()
        .context("missing exports")?
        .iter()
        .filter_map(|entry| entry["Name"].as_str())
        .map(|name| reverse.get(name).copied().unwrap_or(name))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn verify_symbol_report(report: &serde_json::Value, enabled: bool) -> Result<()> {
    ensure!(
        report["symbol_mangling"].is_null() != enabled,
        "incorrect symbol policy report"
    );
    if !enabled {
        return Ok(());
    }
    let plan = &report["symbol_mangling"];
    let renamed = plan["renamed"].as_object().context("missing symbol map")?;
    ensure!(
        !renamed.is_empty() && renamed.keys().any(|name| name.starts_with("_R")),
        "no Rust v0 symbols renamed"
    );
    let mut names = BTreeSet::new();
    for artifact in report["artifacts"]
        .as_array()
        .context("missing artifacts")?
    {
        for symbol in artifact["unpacked_audit"]["elf"]["symbols"]
            .as_array()
            .context("missing ELF symbols")?
        {
            names.insert(symbol["name"].as_str().context("missing symbol name")?);
        }
    }
    for (old, new) in renamed {
        ensure!(
            !names.contains(old.as_str())
                && names.contains(new.as_str().context("invalid symbol name")?),
            "incomplete bundle symbol rewrite: {old}"
        );
    }
    for name in plan["retained"]
        .as_object()
        .context("missing exclusions")?
        .keys()
    {
        ensure!(
            names.contains(name.as_str()),
            "excluded symbol disappeared: {name}"
        );
    }
    writeln!(
        io::stderr().lock(),
        "renamed {} Rust symbols across the fixture bundle",
        renamed.len()
    )?;
    Ok(())
}

fn verify_library_name(
    path: &Path,
    artifact: &serde_json::Value,
    report: &serde_json::Value,
) -> Result<()> {
    if !matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("so" | "dll")
    ) {
        return Ok(());
    }
    let exports = original_exports(artifact, report)?;
    let (original, identity) = if exports.contains("fixture_shared") {
        (
            "fixture_shared",
            "fixture-shared@0.1.0:fixture_shared".to_owned(),
        )
    } else if exports.contains("fixture_logic") {
        (
            "fixture_logic",
            "fixture-logic@0.1.0:fixture_logic".to_owned(),
        )
    } else {
        (
            "std",
            format!(
                "std@{}",
                report["compiler_version"]
                    .as_str()
                    .context("missing compiler version")?
            ),
        )
    };
    let digest = digest(identity.as_bytes())?;
    let expected = if report["mangle_library"] == true {
        &digest[..16]
    } else {
        original
    };
    let filename = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("missing library stem")?;
    let filename = if cfg!(windows) {
        filename
    } else {
        filename
            .strip_prefix("lib")
            .context("missing library prefix")?
    };
    let (stem, suffix) = filename.rsplit_once('-').context("missing Cargo suffix")?;
    ensure!(
        stem == expected
            && suffix.len() == 16
            && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "incorrect library name: {path:?}"
    );
    Ok(())
}

fn verify_packing(
    path: &Path,
    artifact: &serde_json::Value,
    report: &serde_json::Value,
    executable: bool,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let recovered = directory.path().join("unpacked");
    let data = fs::read(path)?;
    let disabled = policy_arguments(report)?;
    let packed = artifact["packed"]
        .as_bool()
        .context("missing packing status")?;
    ensure!(
        packed != disabled.iter().any(|arg| arg == "--no-upx"),
        "incorrect packing policy"
    );
    let unpacked = if packed {
        let header = artifact["packed_audit"]["header_offset"]
            .as_u64()
            .context("missing pack header")? as usize;
        // RREA: coordinated format tag at the validated header, not arbitrary payload data.
        let tag = 0xb6e4_a291_u32.to_le_bytes();
        ensure!(
            data.get(header..header + 4) == Some(tag.as_slice()),
            "incorrect packed header"
        );
        success(
            Command::new(env!("RREA_UPX_PATH"))
                .args(["--no-env", "--decompress", "-o"])
                .arg(&recovered)
                .arg(path),
        )?;
        fs::read(recovered)?
    } else {
        ensure!(
            artifact["packed_audit"].is_null()
                && artifact["output_sha256"] == artifact["unpacked_sha256"],
            "unpacked report claims packing or changed bytes"
        );
        data
    };
    if executable {
        use object::Object;
        let file = object::File::parse(unpacked.as_slice())?;
        ensure!(
            file.symbols().next().is_some() == disabled.iter().any(|arg| arg == "--no-strip"),
            "incorrect static-symbol stripping"
        );
    }
    if let object::File::Elf64(elf) = object::File::parse(unpacked.as_slice())? {
        let table = elf
            .elf_section_table()
            .dynamic_table(elf.endian(), unpacked.as_slice())?;
        for entry in &table {
            if matches!(entry.tag, object::elf::DT_RPATH | object::elf::DT_RUNPATH) {
                ensure!(
                    !table.string(entry)?.windows(3).any(|bytes| bytes == b"/./"),
                    "ELF search-path padding remains"
                );
            }
        }
    }
    ensure!(
        artifact["unpacked_sha256"] == digest(&unpacked)?,
        "packing changed the original executable"
    );
    Ok(())
}

fn digest(data: &[u8]) -> Result<String> {
    let mut digest = String::new();
    for byte in Sha256::digest(data) {
        write!(digest, "{byte:02x}")?;
    }
    Ok(digest)
}

fn verify_relocation(root: &Path, distribution: &Distribution, paths: &[PathBuf]) -> Result<()> {
    // Copy only runtime artifacts to exclude dependencies on build output and logs.
    let relocated = root.join("relocated");
    fs::create_dir_all(&relocated)?;
    for artifact in &distribution.artifacts {
        fs::copy(
            artifact,
            relocated.join(artifact.file_name().context("missing artifact name")?),
        )?;
    }
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
    let exports: Vec<_> = distribution.report["artifacts"]
        .as_array()
        .context("missing artifact report")?
        .iter()
        .map(|artifact| original_exports(artifact, &distribution.report))
        .collect::<Result<_>>()?;
    let index = exports
        .iter()
        .position(|names| names.contains("fixture_shared"))
        .context("missing transitive runtime library")?;
    let dependency = distribution
        .artifacts
        .get(index)
        .context("missing transitive artifact")?;
    fs::remove_file(relocated.join(dependency.file_name().context("missing library name")?))?;
    let missing =
        Command::new(relocated.join(paths[0].file_name().context("missing executable name")?))
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .arg("7")
            .output()?;
    ensure!(
        !missing.status.success(),
        "relocated executable loaded a library outside its bundle"
    );
    Ok(())
}

fn verify_failed_publication(
    root: &Path,
    target: &Path,
    distribution: &Distribution,
) -> Result<()> {
    let prior_reports = distributions(target)?;
    let prior_contents: Vec<_> = distribution
        .artifacts
        .iter()
        .map(fs::read)
        .collect::<std::io::Result<_>>()?;
    for (flags, diagnostic) in [
        (
            "-Cllvm-args=--inline-threshold=1",
            "external LLVM arguments conflict",
        ),
        ("-Cpanic=abort", "requires panic=unwind"),
    ] {
        let failed = cli()
            .current_dir(root)
            .env("CARGO_TARGET_DIR", target)
            .args([
                "build",
                "--release",
                "--package",
                "fixture-app",
                "--bins",
                "--offline",
            ])
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .args(policy_arguments(&distribution.report)?)
            .env("RUSTFLAGS", flags)
            .output()?;
        ensure!(
            !failed.status.success(),
            "conflicting protection flags succeeded: {flags}"
        );
        ensure!(
            String::from_utf8_lossy(&failed.stderr).contains(diagnostic),
            "missing protection conflict diagnostic: {}",
            String::from_utf8_lossy(&failed.stderr)
        );
    }
    if !distribution.report["symbol_mangling"].is_null() {
        let failed = cli()
            .current_dir(root)
            .env("CARGO_TARGET_DIR", target)
            .args([
                "build",
                "--release",
                "--offline",
                "--package",
                "fixture-app",
                "--bins",
                "--keep-symbol",
                "missing-symbol",
            ])
            .args(policy_arguments(&distribution.report)?)
            .output()?;
        ensure!(
            !failed.status.success()
                && String::from_utf8_lossy(&failed.stderr).contains("unknown Rust keep-symbol"),
            "unknown keep-symbol did not fail after linking: {}",
            String::from_utf8_lossy(&failed.stderr)
        );
    }
    ensure!(
        distributions(target)? == prior_reports,
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
