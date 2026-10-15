use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, ensure};
use cargo_metadata::{Message, Metadata, MetadataCommand, TargetKind};
use fs2::FileExt;
use serde::Serialize;

use crate::{BuildOptions, audit, dynamic, project, tools};

const OBFUSCATION: &[&str] = &[
    "-Cllvm-args=--irobf",
    "-Cllvm-args=--irobf-indbr",
    "-Cllvm-args=--irobf-icall",
    "-Cllvm-args=--irobf-indgv",
    "-Cllvm-args=--irobf-cse",
    "-Cllvm-args=--irobf-fla",
    "-Cllvm-args=--irobf-cie",
    "-Cllvm-args=--irobf-cfe",
];

pub fn run(options: &BuildOptions) -> Result<()> {
    let target = tools::host()?;
    ensure!(
        options
            .target
            .as_deref()
            .is_none_or(|requested| requested == target),
        "cross-compilation is not supported; target must be {target}"
    );

    let metadata = metadata(options)?;
    let workspace = fs::canonicalize(&metadata.workspace_root)?;
    let root = metadata.target_directory.as_std_path().join("rrea");
    fs::create_dir_all(&root)?;
    let _lock = project::lock(&root.join("build.lock"))?;
    let tools = tools::acquire(options.offline || options.frozen)?;
    project::checked_output(Command::new(&tools.objcopy).arg("--version"))
        .context("downloaded llvm-objcopy cannot sanitize executable metadata")?;
    let compiler_version = project::checked_output(Command::new(&tools.compiler).arg("-vV"))?;
    let compiler_version = String::from_utf8(compiler_version.stdout)?;
    ensure!(
        compiler_version
            .lines()
            .any(|line| line == format!("host: {target}")),
        "downloaded compiler host does not match {target}"
    );
    project::checked_output(Command::new(&tools.upx).arg("--version"))?;
    let wrapper = std::env::current_exe()?.canonicalize()?;
    let wrapper_hash = tools::sha256(&wrapper)?;
    let policy = dynamic::Policy::from_metadata(&metadata, options)?;
    let policy = serde_json::to_vec(&policy)?;
    let mut build_key = format!("{}:{wrapper_hash}:", tools.compiler_release.sha256).into_bytes();
    build_key.extend_from_slice(&policy);
    let build_directory = root
        .join("build")
        .join(tools::digest(build_key.as_slice())?);
    fs::create_dir_all(&build_directory)?;
    let build_directory = build_directory.canonicalize()?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
        .context("cannot determine Cargo home")?;
    let cargo_home = fs::canonicalize(cargo_home)?;
    let sysroot = tools
        .compiler
        .parent()
        .and_then(Path::parent)
        .context("compiler has no sysroot")?;
    let prohibited = [
        workspace.as_path(),
        cargo_home.as_path(),
        build_directory.as_path(),
        sysroot,
    ];
    let invocations = build_directory.join("rustc-invocations.jsonl");
    let policy_path = build_directory.join("policy.json");
    fs::write(&policy_path, policy)?;
    let library_log = build_directory.join("libraries.jsonl");
    let mut command = project::cargo();
    command
        .args([
            "build",
            "--release",
            "--message-format=json-render-diagnostics",
            "--target",
            target,
            "--target-dir",
        ])
        .arg(&build_directory)
        .env("RUSTC", &tools.compiler)
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env("CARGO_INCREMENTAL", "0")
        .env("RREA_COMPILER", &tools.compiler)
        .env("RREA_TARGET", target)
        .env("RREA_WORKSPACE", &workspace)
        .env("RREA_CARGO_HOME", &cargo_home)
        .env("RREA_BUILD_DIR", &build_directory)
        .env("RREA_SYSROOT", sysroot)
        .env("RREA_POLICY", &policy_path)
        .env("RREA_LIBRARIES", &library_log)
        .env("RREA_INVOCATIONS", &invocations);
    for (key, value) in [
        ("build.rustc", path_value(&tools.compiler)?),
        ("build.rustc-wrapper", path_value(&wrapper)?),
        ("build.rustc-workspace-wrapper", "\"\"".to_owned()),
        ("profile.release.debug", "0".to_owned()),
        ("profile.release.strip", "\"symbols\"".to_owned()),
        ("profile.release.incremental", "false".to_owned()),
    ] {
        command.arg("--config").arg(format!("{key}={value}"));
    }
    build_arguments(&mut command, options);
    let log = build_directory.join("cargo.jsonl");
    eprintln!(
        "building with {}; log: {}",
        tools.compiler_release.tag,
        log.display()
    );
    let (mut artifacts, active_rlibs) = collect_artifacts(&mut command, &metadata, &log)?;
    ensure!(
        !artifacts.is_empty(),
        "Cargo produced no selected executables"
    );
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&root)?;
    let executables: BTreeSet<_> = artifacts.keys().cloned().collect();
    let libraries = dynamic::read_libraries(&library_log)?;
    let mut runtime_libraries: BTreeSet<_> = libraries
        .into_iter()
        .filter(|library| active_rlibs.contains(&library.rlib))
        .map(|library| library.dylib)
        .collect();
    runtime_libraries.insert(dynamic::standard_library(sysroot, target)?);
    let directories: BTreeSet<_> = executables
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    for directory in directories {
        for library in &runtime_libraries {
            let name = library
                .file_name()
                .context("shared library has no filename")?;
            ensure!(
                artifacts
                    .insert(directory.join(name), library.clone())
                    .is_none(),
                "shared library output collides with an executable"
            );
        }
    }
    let mut outputs = Vec::new();
    for (relative, original) in artifacts {
        let packed_library = !executables.contains(&relative);
        eprintln!("sanitizing {}", relative.display());
        let original_audit = audit::inspect(&original, false, &[])?;
        let clean = staging.path().join(&relative);
        fs::create_dir_all(clean.parent().context("artifact has no parent")?)?;
        let data = fs::read(&original)?;
        use object::{Object, ObjectSection};
        let object = object::File::parse(data.as_slice())?;
        let mut strip = Command::new(&tools.objcopy);
        strip.arg("--strip-all");
        for section in object.sections() {
            let name = section.name()?;
            if audit::unwanted_section(name) {
                strip.arg(format!("--remove-section={name}"));
            }
        }
        strip.arg(&original).arg(&clean);
        project::checked_output(&mut strip)?;
        let unpacked_audit = audit::inspect(&clean, true, &prohibited)?;
        audit::same_runtime_metadata(&original_audit, &unpacked_audit)?;
        let unpacked_sha256 = tools::sha256(&clean)?;
        let unpacked_bytes = fs::metadata(&clean)?.len();
        if !unpacked_audit.rust_string_hints.is_empty() {
            eprintln!(
                "warning: {} retains Rust runtime string hints: {}",
                relative.display(),
                unpacked_audit.rust_string_hints.join(", ")
            );
        }
        if packed_library {
            eprintln!("packing {}", relative.display());
            let packed = clean.with_extension("rrea-packed");
            let mut pack = Command::new(&tools.upx);
            pack.args(["--no-env", "--best", "--overlay=copy"]);
            if target.ends_with("windows-msvc") {
                pack.args([
                    "--strip-relocs=0",
                    "--compress-exports=0",
                    "--compress-resources=0",
                ]);
            }
            project::checked_output(pack.arg("-o").arg(&packed).arg(&clean))?;
            project::checked_output(
                Command::new(&tools.upx)
                    .args(["--no-env", "--test"])
                    .arg(&packed),
            )?;
            audit::inspect(&packed, true, &prohibited)?;
            let recovered = clean.with_extension("rrea-recovered");
            project::checked_output(
                Command::new(&tools.upx)
                    .args(["--no-env", "--decompress", "-o"])
                    .arg(&recovered)
                    .arg(&packed),
            )?;
            let recovered_audit = audit::inspect(&recovered, true, &prohibited)?;
            audit::same_runtime_metadata(&unpacked_audit, &recovered_audit)?;
            fs::remove_file(&clean)?;
            fs::remove_file(&recovered)?;
            fs::rename(&packed, &clean)?;
        }
        outputs.push(ArtifactReport {
            path: relative,
            packed: packed_library,
            unpacked_bytes,
            unpacked_sha256,
            output_bytes: fs::metadata(&clean)?.len(),
            output_sha256: tools::sha256(&clean)?,
            unpacked_audit,
        });
    }
    let report = BuildReport {
        target,
        compiler: &tools.compiler_release,
        upx: &tools.upx_build,
        compiler_version: &compiler_version,
        obfuscation: OBFUSCATION,
        cargo_log: Path::new("cargo.jsonl"),
        compiler_invocations: Path::new("rustc-invocations.jsonl"),
        artifacts: &outputs,
        limitations: [
            "Precompiled standard libraries and native dependencies are not fully covered by these LLVM passes.",
            "Rust runtime strings and code patterns can remain. UPX packing is reversible.",
            "Metadata and UPX checks do not prove functional equivalence or resistance to reverse engineering.",
        ],
    };
    fs::copy(&log, staging.path().join("cargo.jsonl"))?;
    fs::copy(&invocations, staging.path().join("rustc-invocations.jsonl"))?;
    let mut report_file = File::create(staging.path().join("build.json"))?;
    serde_json::to_writer_pretty(&mut report_file, &report)?;
    report_file.write_all(b"\n")?;
    report_file.sync_all()?;
    drop(report_file);
    let suffix = staging
        .path()
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_prefix(".staging-"))
        .context("invalid staging directory")?;
    let destination = root.join(format!("dist-{suffix}"));
    ensure!(!destination.exists(), "output directory already exists");
    fs::rename(staging.path(), &destination)?;
    for executable in executables {
        println!("{}", destination.join(executable).display());
    }
    eprintln!("report: {}", destination.join("build.json").display());
    Ok(())
}

fn path_value(path: &Path) -> Result<String> {
    Ok(toml_edit::Value::from(path.to_str().context("tool paths must be UTF-8")?).to_string())
}

fn metadata(options: &BuildOptions) -> Result<Metadata> {
    let mut command = MetadataCommand::new();
    command.cargo_path(project::cargo().get_program());

    if let Some(path) = &options.manifest_path {
        command.manifest_path(path);
    }
    let mut flags = vec!["--filter-platform".to_owned(), tools::host()?.to_owned()];
    if options.all_features {
        command.features(cargo_metadata::CargoOpt::AllFeatures);
    }
    if options.no_default_features {
        command.features(cargo_metadata::CargoOpt::NoDefaultFeatures);
    }
    if !options.features.is_empty() {
        command.features(cargo_metadata::CargoOpt::SomeFeatures(
            options.features.clone(),
        ));
    }
    if options.offline {
        flags.push("--offline".to_owned());
    }
    if options.locked {
        flags.push("--locked".to_owned());
    }
    if options.frozen {
        flags.push("--frozen".to_owned());
    }
    command.other_options(flags);
    Ok(command.exec()?)
}

fn build_arguments(command: &mut Command, options: &BuildOptions) {
    if let Some(path) = &options.manifest_path {
        command.arg("--manifest-path").arg(path);
    }
    for (flag, values) in [
        ("--package", &options.packages),
        ("--bin", &options.bin),
        ("--exclude", &options.exclude),
        ("--features", &options.features),
    ] {
        for value in values {
            command.arg(flag).arg(value);
        }
    }
    for (enabled, flag) in [
        (options.bins, "--bins"),
        (options.workspace, "--workspace"),
        (options.all_features, "--all-features"),
        (options.no_default_features, "--no-default-features"),
        (options.locked, "--locked"),
        (options.offline, "--offline"),
        (options.frozen, "--frozen"),
    ] {
        if enabled {
            command.arg(flag);
        }
    }
    if let Some(jobs) = options.jobs {
        command.arg("--jobs").arg(jobs.to_string());
    }
}

fn collect_artifacts(
    command: &mut Command,
    metadata: &Metadata,
    log: &Path,
) -> Result<(BTreeMap<PathBuf, PathBuf>, BTreeSet<PathBuf>)> {
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .context("cannot start Cargo build")?;
    let stdout = child.stdout.take().context("Cargo stdout is not piped")?;
    let mut log_file = File::create(log)?;
    let mut artifacts = BTreeMap::new();
    let mut active_rlibs = BTreeSet::new();
    let mut build_finished = false;
    let mut read_error = None;
    for line in BufReader::new(stdout).lines() {
        let parsed = (|| -> Result<()> {
            let line = line?;
            writeln!(log_file, "{line}")?;
            let Ok(message) = serde_json::from_str::<Message>(&line) else {
                eprintln!("{line}");
                return Ok(());
            };
            match message {
                Message::CompilerArtifact(artifact) => {
                    for filename in &artifact.filenames {
                        if filename.extension() == Some("rlib") {
                            active_rlibs.insert(filename.clone().into_std_path_buf());
                        }
                    }
                    if artifact.target.kind.contains(&TargetKind::Bin)
                        && !artifact.profile.test
                        && let Some(path) = artifact.executable
                    {
                        let package = metadata
                            .packages
                            .iter()
                            .find(|package| package.id == artifact.package_id)
                            .context("executable package is not a workspace package")?;
                        ensure!(
                            metadata.workspace_members.contains(&package.id),
                            "refusing non-workspace executable"
                        );
                        let filename = path.file_name().context("executable has no filename")?;
                        let relative = PathBuf::from(package.name.as_str()).join(filename);
                        ensure!(
                            artifacts
                                .insert(relative, path.into_std_path_buf())
                                .is_none(),
                            "duplicate executable output"
                        );
                    }
                }
                Message::CompilerMessage(message) => {
                    if let Some(rendered) = message.message.rendered {
                        eprint!("{rendered}");
                    }
                }
                Message::BuildFinished(result) => build_finished = result.success,
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = parsed {
            read_error = Some(error);
            let _ = child.kill();
            break;
        }
    }
    let status = child.wait()?;
    if let Some(error) = read_error {
        return Err(error);
    }
    ensure!(
        status.success() && build_finished,
        "Cargo build failed; log: {}",
        log.display()
    );
    Ok((artifacts, active_rlibs))
}

pub fn rustc_wrapper() -> Result<()> {
    let compiler =
        PathBuf::from(std::env::var_os("RREA_COMPILER").context("missing compiler path")?);
    let mut args = std::env::args_os().skip(1);
    let requested = args
        .next()
        .context("compiler wrapper requires a compiler argument")?;
    ensure!(
        Path::new(&requested).canonicalize()? == compiler.canonicalize()?,
        "Cargo selected an unexpected compiler"
    );
    let mut args: Vec<OsString> = args.collect();
    let mut generated_library = None;
    let target = std::env::var("RREA_TARGET")?;
    let target_build = args
        .windows(2)
        .any(|pair| pair[0] == "--target" && pair[1] == target.as_str())
        || args
            .iter()
            .any(|arg| arg == OsStr::new(&format!("--target={target}")));
    if target_build {
        validate_flags(&args)?;
        generated_library = dynamic::adapt(
            &mut args,
            Path::new(&std::env::var_os("RREA_POLICY").context("missing dynamic policy")?),
            Path::new(&std::env::var_os("RREA_LIBRARIES").context("missing library log")?),
        )?;
        args.extend(OBFUSCATION.iter().map(OsString::from));
        // Order matters: more specific generated paths must override the workspace prefix.
        for (variable, replacement) in [
            ("RREA_WORKSPACE", "/src"),
            ("RREA_CARGO_HOME", "/deps"),
            ("RREA_SYSROOT", "/toolchain"),
            ("RREA_BUILD_DIR", "/build"),
        ] {
            let from = std::env::var(variable)?;
            args.push(format!("--remap-path-prefix={from}={replacement}").into());
            if cfg!(windows) {
                args.push(
                    format!(
                        "--remap-path-prefix={}={replacement}",
                        from.replace('\\', "/")
                    )
                    .into(),
                );
            }
        }
        let log =
            std::env::var_os("RREA_INVOCATIONS").context("missing compiler invocation log")?;
        let mut log = OpenOptions::new().create(true).append(true).open(log)?;
        log.lock_exclusive()?;
        let readable_args: Vec<_> = args
            .iter()
            .map(|argument| {
                argument
                    .to_str()
                    .context("compiler arguments must be UTF-8")
            })
            .collect::<Result<_>>()?;
        serde_json::to_writer(&mut log, &readable_args)?;
        log.write_all(b"\n")?;
    }
    let status = Command::new(&compiler)
        .args(&args)
        .status()
        .context("cannot start obfuscating compiler")?;
    ensure!(status.success(), "obfuscating compiler failed ({status})");
    if let Some(library) = generated_library {
        if dynamic::value(&args, "--emit")
            .is_some_and(|emit| emit.split(',').any(|part| part == "link"))
        {
            dynamic::record(
                Path::new(&std::env::var_os("RREA_LIBRARIES").context("missing library log")?),
                &library,
            )?;
        }
    }
    Ok(())
}

fn validate_flags(args: &[OsString]) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        let value = args[index]
            .to_str()
            .context("compiler arguments must be UTF-8")?;
        ensure!(
            value != "-g",
            "debug information conflicts with rrea protection"
        );
        ensure!(
            value != "--sysroot" && !value.starts_with("--sysroot="),
            "custom sysroots conflict with the selected compiler"
        );
        let codegen = if value == "-C" {
            index += 1;
            Some(
                args.get(index)
                    .and_then(|arg| arg.to_str())
                    .context("missing codegen argument")?,
            )
        } else {
            value.strip_prefix("-C")
        };
        if let Some(option) = codegen {
            if let Some(debug) = option.strip_prefix("debuginfo=") {
                ensure!(
                    matches!(debug, "0" | "none"),
                    "debuginfo={debug} conflicts with rrea protection"
                );
            }
            if let Some(strip) = option.strip_prefix("strip=") {
                ensure!(
                    strip == "symbols",
                    "strip={strip} conflicts with rrea protection"
                );
            }
            ensure!(
                !option.starts_with("llvm-args"),
                "external LLVM arguments conflict with the fixed obfuscation pipeline"
            );
        }
        index += 1;
    }
    Ok(())
}

#[derive(Serialize)]
struct ArtifactReport {
    path: PathBuf,
    packed: bool,
    unpacked_bytes: u64,
    unpacked_sha256: String,
    output_bytes: u64,
    output_sha256: String,
    unpacked_audit: audit::Inspection,
}

#[derive(Serialize)]
struct BuildReport<'a> {
    target: &'a str,
    compiler: &'a tools::ReleaseAsset,
    upx: &'a tools::UpxBuild,
    compiler_version: &'a str,
    obfuscation: &'a [&'a str],
    cargo_log: &'a Path,
    compiler_invocations: &'a Path,
    artifacts: &'a [ArtifactReport],
    limitations: [&'a str; 3],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_conflicts_are_rejected() {
        for args in [
            vec!["-g"],
            vec!["-C", "debuginfo=2"],
            vec!["-Cstrip=none"],
            vec!["-Cllvm-args=--irobf=false"],
            vec!["--sysroot", "/other"],
        ] {
            let args: Vec<_> = args.into_iter().map(OsString::from).collect();
            assert!(validate_flags(&args).is_err());
        }
        let args: Vec<_> = [
            "-C",
            "debuginfo=0",
            "-Cstrip=symbols",
            "-Clink-arg=-pthread",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert!(validate_flags(&args).is_ok());
    }
}
