use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result, ensure};
use cargo_metadata::{Message, Metadata, MetadataCommand, TargetKind};
use fs2::FileExt;
use object::{Object, ObjectSection};
use serde::Serialize;

use crate::{
    BuildOptions,
    audit,
    dynamic,
    mangle,
    obfuscation::{Feature, Obfuscation},
    project,
    support::progress,
    symbols,
    tools,
};

const PANIC_STRATEGY: &str = "unwind";

pub fn run(options: &BuildOptions) -> Result<()> {
    let target = check_target(options)?;
    let obfuscation = &options.obfuscation;
    let metadata = metadata(options)?;
    let manifest_directory = manifest_directory(&metadata)?;
    let distribution = manifest_directory.join("target/release/dist");
    let root = metadata.target_directory.as_std_path().join("rrea");
    fs::create_dir_all(&root)?;
    let publication_root = manifest_directory.join("target/rrea");
    fs::create_dir_all(&publication_root)?;
    let _publication_lock = project::lock(&publication_root.join("publish.lock"))?;
    let _lock = project::lock(&root.join("build.lock"))?;
    let tools = tools::acquire(options.network.offline || options.network.frozen)?;
    let compiler_version = check_tools(&tools, target, obfuscation)?;
    let sysroot = tools
        .compiler
        .parent()
        .and_then(Path::parent)
        .context("compiler has no sysroot")?;
    let mut prepared = prepare_build(&root, &metadata, options, &tools, sysroot, target)?;
    let log = prepared.directory.join("cargo.jsonl");
    progress(format_args!(
        "building with {} (panic={PANIC_STRATEGY}); log: {}",
        tools.compiler_release.tag,
        log.display()
    ))?;
    let CollectedArtifacts {
        mut artifacts,
        active_rlibs,
    } = collect_artifacts(&mut prepared.command, &metadata, &log, options.quiet > 0)?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&root)?;
    let executables: Vec<_> = artifacts.keys().cloned().collect();
    let names = add_runtime_libraries(
        &mut artifacts,
        &active_rlibs,
        &prepared.directory.join("libraries.jsonl"),
        sysroot,
        options,
        &compiler_version,
    )?;
    let prohibited: Vec<_> = prepared
        .prohibited_paths
        .iter()
        .filter(|_| obfuscation.enabled(Feature::RemapPaths))
        .map(PathBuf::as_path)
        .collect();
    let symbol_plan = symbols::plan(
        &artifacts,
        obfuscation
            .enabled(Feature::MangleSymbols)
            .then_some(options.keep_symbols.as_slice()),
        obfuscation.enabled(Feature::Strip),
    )?;
    let mut outputs = prepare_artifacts(
        artifacts.into_iter().map(|(relative, original)| {
            let executable = executables.contains(&relative);
            (relative, original, executable)
        }),
        staging.path(),
        &tools,
        &names,
        &symbol_plan,
        &prohibited,
        obfuscation,
    )?;
    for output in &mut outputs {
        output.path = distribution.join(&output.path);
    }
    let report = BuildReport {
        target,
        compiler: &tools.compiler_release,
        upx: &tools.upx_build,
        compiler_version: &compiler_version,
        panic_strategy: PANIC_STRATEGY,
        obfuscation: &obfuscation.llvm_flags(),
        disabled_obfuscation: &obfuscation.disabled,
        mangle_library: obfuscation.enabled(Feature::MangleLibrary),
        symbol_mangling: obfuscation
            .enabled(Feature::MangleSymbols)
            .then_some(&symbol_plan),
        removed_metadata_symbols: &symbol_plan.removed_metadata,
        cargo_log: Path::new("cargo.jsonl"),
        compiler_invocations: Path::new("rustc-invocations.jsonl"),
        artifacts: &outputs,
        limitations: [
            "Precompiled standard libraries and native dependencies are not fully covered by these LLVM passes.",
            "Rust runtime strings and code patterns can remain. Modified UPX packing is reversible.",
            "Metadata and UPX checks do not prove functional equivalence or resistance to reverse engineering.",
        ],
    };
    let destination = publish(
        &root,
        staging.path(),
        &prepared.directory,
        &distribution,
        &report,
    )?;
    print_distribution(&distribution, &executables, &destination)
}

fn manifest_directory(metadata: &Metadata) -> Result<&Path> {
    match metadata.root_package() {
        Some(package) => {
            Ok(package
                .manifest_path
                .parent()
                .context("manifest has no parent")?
                .as_std_path())
        }
        None => Ok(metadata.workspace_root.as_std_path()),
    }
}

fn print_distribution(
    distribution: &Path,
    executables: &[PathBuf],
    destination: &Path,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    for executable in executables {
        writeln!(stdout, "{}", distribution.join(executable).display())?;
    }
    progress(format_args!(
        "report: {}",
        destination.join("build.json").display()
    ))?;
    Ok(())
}

fn check_target(options: &BuildOptions) -> Result<&'static str> {
    let target = tools::host()?;
    ensure!(
        options
            .target
            .as_deref()
            .is_none_or(|requested| requested == target),
        "cross-compilation is not supported; target must be {target}"
    );
    ensure!(
        !options.obfuscation.enabled(Feature::MangleSymbols)
            || target == "x86_64-unknown-linux-gnu",
        "symbol mangling is supported only for Linux ELF64 closed bundles; use --no-mangle symbol on this target"
    );
    ensure!(
        !options.obfuscation.enabled(Feature::Upx)
            || !options.obfuscation.enabled(Feature::Opaque)
            || target == "x86_64-unknown-linux-gnu",
        "opaque packing is supported only for Linux x86_64 executables; use --no-opaque on this target"
    );
    Ok(target)
}

fn check_tools(tools: &tools::Tools, target: &str, obfuscation: &Obfuscation) -> Result<String> {
    if obfuscation.enabled(Feature::Strip) {
        project::checked_output(Command::new(&tools.objcopy).arg("--version"))
            .context("downloaded llvm-objcopy cannot sanitize executable metadata")?;
    }
    let version = project::checked_output(Command::new(&tools.compiler).arg("-vV"))?;
    let version = String::from_utf8(version.stdout)?;
    ensure!(
        version
            .lines()
            .any(|line| line == format!("host: {target}")),
        "downloaded compiler host does not match {target}"
    );
    if obfuscation.enabled(Feature::Upx) {
        project::checked_output(Command::new(&tools.upx).arg("--version"))?;
    }
    Ok(version)
}

struct PreparedBuild {
    command: Command,
    directory: PathBuf,
    prohibited_paths: Vec<PathBuf>,
}

fn prepare_build(
    root: &Path,
    metadata: &Metadata,
    options: &BuildOptions,
    tools: &tools::Tools,
    sysroot: &Path,
    target: &str,
) -> Result<PreparedBuild> {
    let workspace = fs::canonicalize(&metadata.workspace_root)?;
    let wrapper = std::env::current_exe()?.canonicalize()?;
    let wrapper_hash = tools::sha256(&wrapper)?;
    let policy = serde_json::to_vec(&dynamic::Policy::from_metadata(metadata, options)?)?;
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
    let invocations = build_directory.join("rustc-invocations.jsonl");
    let policy_path = build_directory.join("policy.json");
    fs::write(&policy_path, policy)?;
    let library_log = build_directory.join("libraries.jsonl");

    let mut command = project::cargo();
    command
        .args([
            "build",
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
        .env("CARGO_PROFILE_RELEASE_PANIC", PANIC_STRATEGY)
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
        ("profile.release.incremental", "false".to_owned()),
        ("profile.release.lto", "false".to_owned()),
    ] {
        command.arg("--config").arg(format!("{key}={value}"));
    }
    if options.obfuscation.enabled(Feature::Strip) {
        command.args([
            "--config",
            "profile.release.debug=0",
            "--config",
            "profile.release.strip=\"symbols\"",
        ]);
    }
    build_arguments(&mut command, options);
    Ok(PreparedBuild {
        command,
        prohibited_paths: vec![
            workspace,
            cargo_home,
            build_directory.clone(),
            sysroot.to_path_buf(),
        ],
        directory: build_directory,
    })
}

fn add_runtime_libraries(
    artifacts: &mut BTreeMap<PathBuf, PathBuf>,
    active_rlibs: &BTreeSet<PathBuf>,
    library_log: &Path,
    sysroot: &Path,
    options: &BuildOptions,
    compiler_version: &str,
) -> Result<BTreeMap<String, String>> {
    let mut libraries: BTreeMap<_, _> = dynamic::read_libraries(library_log)?
        .into_iter()
        .filter(|library| active_rlibs.contains(&library.rlib))
        .filter_map(|library| {
            match library.linkage {
                dynamic::Linkage::Static => None,
                dynamic::Linkage::Dynamic(path) => Some((path, library.identity)),
            }
        })
        .collect();
    libraries.insert(
        dynamic::standard_library(sysroot, tools::host()?)?,
        format!("std@{compiler_version}"),
    );
    let mut candidates = BTreeMap::new();
    for (library, identity) in libraries {
        let name = PathBuf::from(
            library
                .file_name()
                .context("shared library has no filename")?,
        );
        ensure!(
            candidates.insert(name, (library, identity)).is_none(),
            "duplicate runtime library"
        );
    }
    let mut pending: Vec<_> = artifacts.values().cloned().collect();
    let mut names = BTreeMap::new();
    while let Some(artifact) = pending.pop() {
        for dependency in audit::inspect(&artifact, false, &[])?.libraries {
            let name = PathBuf::from(&dependency);
            if let Some((library, identity)) = candidates.remove(&name) {
                if options.obfuscation.enabled(Feature::MangleLibrary) {
                    let mangled = mangle::name(&dependency, &identity)?;
                    ensure!(
                        !artifacts.contains_key(Path::new(&mangled)),
                        "mangled library collides with an executable"
                    );
                    names.insert(dependency, mangled);
                }
                pending.push(library.clone());
                ensure!(
                    artifacts.insert(name, library).is_none(),
                    "duplicate runtime output"
                );
            }
        }
    }
    ensure!(
        names.values().collect::<BTreeSet<_>>().len() == names.len(),
        "mangled library name collision"
    );
    Ok(names)
}

fn prepare_artifacts(
    artifacts: impl IntoIterator<Item = (PathBuf, PathBuf, bool)>,
    staging: &Path,
    tools: &tools::Tools,
    names: &BTreeMap<String, String>,
    symbols: &symbols::Plan,
    prohibited: &[&Path],
    obfuscation: &Obfuscation,
) -> Result<Vec<ArtifactReport>> {
    let mut outputs = Vec::new();

    for (relative, original, executable) in artifacts {
        let name = relative.to_str().context("artifact name is not UTF-8")?;
        let relative = Path::new(names.get(name).map_or(name, String::as_str));
        progress(format_args!("sanitizing {}", relative.display()))?;
        let clean = staging.join(relative);
        let strip = obfuscation.enabled(Feature::Strip);
        let unpacked_audit = sanitize(&original, &clean, tools, names, symbols, prohibited, strip)?;
        let unpacked_sha256 = tools::sha256(&clean)?;
        let unpacked_bytes = fs::metadata(&clean)?.len();
        if !unpacked_audit.rust_string_hints.is_empty() {
            progress(format_args!(
                "note: {} retains Rust runtime hints (not a stripping failure): {}; details in build report",
                relative.display(),
                unpacked_audit.rust_string_hints.join(", ")
            ))?;
        }
        let packed_audit = obfuscation
            .enabled(Feature::Upx)
            .then(|| {
                pack_artifact(
                    &clean,
                    tools,
                    &unpacked_audit,
                    &unpacked_sha256,
                    prohibited,
                    strip,
                    executable && obfuscation.enabled(Feature::Opaque),
                )
            })
            .transpose()?;
        outputs.push(ArtifactReport {
            path: relative.to_path_buf(),
            packed: packed_audit.is_some(),
            unpacked_bytes,
            unpacked_sha256,
            output_bytes: fs::metadata(&clean)?.len(),
            output_sha256: tools::sha256(&clean)?,
            unpacked_audit,
            packed_audit,
        });
    }
    Ok(outputs)
}

fn pack_artifact(
    clean: &Path,
    tools: &tools::Tools,
    unpacked_audit: &audit::Inspection,
    unpacked_sha256: &str,
    prohibited: &[&Path],
    strip: bool,
    opaque: bool,
) -> Result<audit::PackedInspection> {
    progress(format_args!("packing {}", clean.display()))?;
    let packed = clean.with_extension("rrea-packed");
    let mut pack = Command::new(&tools.upx);
    pack.args(["--no-env", "--best", "--overlay=copy"]);
    if opaque {
        pack.arg("--opaque");
    }
    if cfg!(windows) {
        pack.args([
            "--strip-relocs=0",
            "--compress-exports=0",
            "--compress-resources=0",
        ]);
    }
    project::checked_output(pack.arg("-o").arg(&packed).arg(clean))?;
    project::checked_output(
        Command::new(&tools.upx)
            .args(["--no-env", "--test"])
            .arg(&packed),
    )?;
    audit::inspect(&packed, strip, prohibited)?;
    let recovered = clean.with_extension("rrea-recovered");
    project::checked_output(
        Command::new(&tools.upx)
            .args(["--no-env", "--decompress", "-o"])
            .arg(&recovered)
            .arg(&packed),
    )?;
    let recovered_audit = audit::inspect(&recovered, strip, prohibited)?;
    audit::same_runtime_metadata(unpacked_audit, &recovered_audit)?;
    ensure!(
        tools::sha256(&recovered)? == unpacked_sha256,
        "UPX recovery changed executable bytes"
    );
    let packed_audit = audit::validate_packed(&packed)?;
    ensure!(
        packed_audit.opaque == opaque,
        "UPX opaque mode does not match the selected policy"
    );
    fs::remove_file(clean)?;
    fs::remove_file(&recovered)?;
    fs::rename(&packed, clean)?;
    Ok(packed_audit)
}

fn sanitize(
    original: &Path,
    clean: &Path,
    tools: &tools::Tools,
    names: &BTreeMap<String, String>,
    symbols: &symbols::Plan,
    prohibited: &[&Path],
    strip: bool,
) -> Result<audit::Inspection> {
    let mut original_audit = audit::inspect(original, false, &[])?;
    fs::create_dir_all(clean.parent().context("artifact has no parent")?)?;
    let mangled = clean.with_extension("rrea-mangled");
    let source = if names.is_empty() && symbols.is_empty() {
        original
    } else {
        fs::copy(original, &mangled)?;
        mangle::rewrite(&mangled, names, symbols, &mut original_audit)?;
        mangled.as_path()
    };
    if strip {
        let data = fs::read(source)?;
        let object = object::File::parse(data.as_slice())?;
        let mut strip = Command::new(&tools.objcopy);
        strip.arg("--strip-all");
        for section in object.sections() {
            let name = section.name()?;
            if audit::unwanted_section(name) {
                strip.arg(format!("--remove-section={name}"));
            }
        }
        project::checked_output(strip.arg(source).arg(clean))?;
    } else {
        fs::copy(source, clean)?;
    }
    let unpacked_audit = audit::inspect(clean, strip, prohibited)?;
    audit::same_runtime_metadata(&original_audit, &unpacked_audit)?;
    if !names.is_empty() || !symbols.is_empty() {
        fs::remove_file(mangled)?;
    }
    Ok(unpacked_audit)
}

fn publish(
    root: &Path,
    staging: &Path,
    build_directory: &Path,
    distribution: &Path,
    report: &BuildReport<'_>,
) -> Result<PathBuf> {
    let logs = tempfile::Builder::new()
        .prefix(".reports-")
        .tempdir_in(root)?;
    fs::copy(
        build_directory.join("cargo.jsonl"),
        logs.path().join("cargo.jsonl"),
    )?;
    fs::copy(
        build_directory.join("rustc-invocations.jsonl"),
        logs.path().join("rustc-invocations.jsonl"),
    )?;
    let mut report_file = File::create(logs.path().join("build.json"))?;
    serde_json::to_writer_pretty(&mut report_file, report)?;
    report_file.write_all(b"\n")?;
    report_file.sync_all()?;
    drop(report_file);
    let parent = distribution
        .parent()
        .context("distribution has no parent")?;
    fs::create_dir_all(parent)?;
    let binaries = tempfile::Builder::new()
        .prefix(".dist-")
        .tempdir_in(parent)?;
    for artifact in report.artifacts {
        let name = artifact
            .path
            .file_name()
            .context("artifact has no filename")?;
        fs::copy(staging.join(name), binaries.path().join(name))?;
    }
    let suffix = staging
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_prefix(".staging-"))
        .context("invalid staging directory")?;
    let destination = root.join(format!("dist-{suffix}"));
    ensure!(!destination.exists(), "output directory already exists");
    let previous = tempfile::Builder::new()
        .prefix(".previous-")
        .tempdir_in(parent)?;
    let backup = previous.path().join("dist");
    if distribution.exists() {
        fs::rename(distribution, &backup)?;
    }
    if let Err(error) = fs::rename(binaries.path(), distribution) {
        let previous = previous.keep();
        if backup.exists() {
            fs::rename(&backup, distribution).with_context(|| {
                format!(
                    "cannot restore the previous distribution; preserved at {}",
                    backup.display()
                )
            })?;
        }
        fs::remove_dir_all(previous)?;
        return Err(error.into());
    }
    if let Err(error) = fs::rename(logs.path(), &destination) {
        let previous = previous.keep();
        fs::remove_dir_all(distribution)?;
        if backup.exists() {
            fs::rename(&backup, distribution).with_context(|| {
                format!(
                    "cannot restore the previous distribution; preserved at {}",
                    backup.display()
                )
            })?;
        }
        fs::remove_dir_all(previous)?;
        return Err(error.into());
    }
    Ok(destination)
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
    if options.features.all {
        command.features(cargo_metadata::CargoOpt::AllFeatures);
    }
    if options.features.no_default {
        command.features(cargo_metadata::CargoOpt::NoDefaultFeatures);
    }
    if !options.features.enabled.is_empty() {
        command.features(cargo_metadata::CargoOpt::SomeFeatures(
            options.features.enabled.clone(),
        ));
    }
    if options.network.offline {
        flags.push("--offline".to_owned());
    }
    if options.network.locked {
        flags.push("--locked".to_owned());
    }
    if options.network.frozen {
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
        ("--package", &options.selection.packages),
        ("--bin", &options.selection.bin),
        ("--exclude", &options.selection.exclude),
        ("--features", &options.features.enabled),
    ] {
        for value in values {
            command.arg(flag).arg(value);
        }
    }
    for (enabled, flag) in [
        (options.release, "--release"),
        (options.quiet > 0, "--quiet"),
        (options.selection.bins, "--bins"),
        (options.selection.workspace, "--workspace"),
        (options.features.all, "--all-features"),
        (options.features.no_default, "--no-default-features"),
        (options.network.locked, "--locked"),
        (options.network.offline, "--offline"),
        (options.network.frozen, "--frozen"),
    ] {
        if enabled {
            command.arg(flag);
        }
    }
    if let Some(jobs) = options.jobs {
        command.arg("--jobs").arg(jobs.to_string());
    }
}

struct CollectedArtifacts {
    artifacts: BTreeMap<PathBuf, PathBuf>,
    active_rlibs: BTreeSet<PathBuf>,
}

fn collect_artifacts(
    command: &mut Command,
    metadata: &Metadata,
    log: &Path,
    quiet: bool,
) -> Result<CollectedArtifacts> {
    let mut log_file = File::create(log)?;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(if quiet {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .context("cannot start Cargo build")?;
    let stdout = child.stdout.take().context("Cargo stdout is not piped")?;
    let stderr = child
        .stderr
        .take()
        .map(|stderr| thread::spawn(move || filter_compiler_output(stderr, io::stderr())));
    let mut artifacts = BTreeMap::new();
    let mut active_rlibs = BTreeSet::new();
    let mut build_finished = false;
    let mut read_error = None;
    for line in BufReader::new(stdout).lines() {
        let parsed = (|| -> Result<()> {
            let line = line?;
            writeln!(log_file, "{line}")?;
            let Ok(message) = serde_json::from_str::<Message>(&line) else {
                if !quiet || !compiler_module_name(&line) {
                    writeln!(io::stderr().lock(), "{line}")?;
                }
                return Ok(());
            };
            match message {
                Message::CompilerArtifact(artifact) => {
                    for filename in &artifact.filenames {
                        // Metadata sidecars identify hashed outputs even for hard-linked root rlibs.
                        if matches!(filename.extension(), Some("rlib" | "rmeta")) {
                            active_rlibs.insert(filename.as_std_path().with_extension("rlib"));
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
                        let relative = PathBuf::from(filename);
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
                        write!(io::stderr().lock(), "{rendered}")?;
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
    if let Some(stderr) = stderr {
        stderr
            .join()
            .map_err(|_| anyhow::anyhow!("compiler output reader panicked"))??;
    }
    if let Some(error) = read_error {
        return Err(error);
    }
    ensure!(
        status.success() && build_finished,
        "Cargo build failed; log: {}",
        log.display()
    );
    ensure!(
        !artifacts.is_empty(),
        "Cargo produced no selected executables"
    );
    Ok(CollectedArtifacts {
        artifacts,
        active_rlibs,
    })
}

fn compiler_module_name(line: &str) -> bool {
    let Some((name, unit)) = line.trim_end_matches(['\r', '\n']).rsplit_once("-cgu.") else {
        return false;
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && !unit.is_empty()
        && unit.bytes().all(|byte| byte.is_ascii_digit())
}

fn filter_compiler_output(input: impl Read, mut output: impl Write) -> Result<()> {
    let mut input = BufReader::new(input);
    let mut line = Vec::new();
    while input.read_until(b'\n', &mut line)? != 0 {
        if !std::str::from_utf8(&line).is_ok_and(compiler_module_name) {
            output.write_all(&line)?;
        }
        line.clear();
    }
    Ok(())
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
        let policy: dynamic::Policy = serde_json::from_slice(&fs::read(
            std::env::var_os("RREA_POLICY").context("missing dynamic policy")?,
        )?)?;
        validate_flags(&args, policy.obfuscation.enabled(Feature::Strip))?;
        generated_library = dynamic::adapt(
            &mut args,
            &policy,
            Path::new(&std::env::var_os("RREA_LIBRARIES").context("missing library log")?),
        )?;
        args.extend(
            policy
                .obfuscation
                .llvm_flags()
                .into_iter()
                .map(OsString::from),
        );
        if policy.obfuscation.enabled(Feature::RemapPaths) {
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
    if let Some(library) = generated_library
        && dynamic::value(&args, "--emit")
            .is_some_and(|emit| emit.split(',').any(|part| part == "link"))
    {
        dynamic::record(
            Path::new(&std::env::var_os("RREA_LIBRARIES").context("missing library log")?),
            &library,
        )?;
    }
    Ok(())
}

fn validate_flags(args: &[OsString], strip_enabled: bool) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        let value = args[index]
            .to_str()
            .context("compiler arguments must be UTF-8")?;
        ensure!(
            !strip_enabled || value != "-g",
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
            let (name, value) = option.split_once('=').unwrap_or((option, "yes"));
            ensure!(
                name != "panic" || value == PANIC_STRATEGY,
                "{option} conflicts with the shared runtime, which requires panic={PANIC_STRATEGY}; remove explicit panic compiler flags"
            );
            ensure!(
                !matches!(name, "lto" | "linker-plugin-lto")
                    || matches!(value, "n" | "no" | "off" | "false"),
                "{option} conflicts with the shared-library build layout; remove explicit LTO compiler flags"
            );
            if let Some(debug) = option.strip_prefix("debuginfo=") {
                ensure!(
                    !strip_enabled || matches!(debug, "0" | "none"),
                    "debuginfo={debug} conflicts with rrea protection"
                );
            }
            if let Some(strip) = option.strip_prefix("strip=") {
                ensure!(
                    !strip_enabled || strip == "symbols",
                    "strip={strip} conflicts with rrea protection"
                );
            }
            ensure!(
                !option.starts_with("llvm-args"),
                "external LLVM arguments conflict with the selected obfuscation policy; use --no-* switches"
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
    packed_audit: Option<audit::PackedInspection>,
}

#[derive(Serialize)]
struct BuildReport<'a> {
    target: &'a str,
    compiler: &'a tools::ReleaseAsset,
    upx: &'a tools::UpxBuild,
    compiler_version: &'a str,
    panic_strategy: &'a str,
    obfuscation: &'a [&'a str],
    disabled_obfuscation: &'a BTreeSet<Feature>,
    mangle_library: bool,
    symbol_mangling: Option<&'a symbols::Plan>,
    removed_metadata_symbols: &'a BTreeSet<String>,
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
            vec!["-C", "linker-plugin-lto"],
            vec!["-Clinker-plugin-lto=yes"],
            vec!["-C", "lto=thin"],
            vec!["-Clto=fat"],
            vec!["-Cpanic=abort"],
            vec!["-C", "panic=abort"],
        ] {
            let args: Vec<_> = args.into_iter().map(OsString::from).collect();
            assert!(validate_flags(&args, true).is_err());
        }
        let args: Vec<_> = [
            "-C",
            "debuginfo=0",
            "-Cstrip=symbols",
            "-Clink-arg=-pthread",
            "-Clto=off",
            "-Clinker-plugin-lto=no",
            "-Cpanic=unwind",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert!(validate_flags(&args, true).is_ok());
    }
}
