use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use cargo_metadata::{DependencyKind, Metadata, TargetKind};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{BuildOptions, project};

#[derive(Deserialize, Serialize)]
pub struct Policy {
    sources: BTreeMap<PathBuf, String>,
    mangle_library: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Library {
    pub identity: String,
    pub rlib: PathBuf,
    pub dylib: PathBuf,
}

impl Policy {
    pub fn from_metadata(metadata: &Metadata, options: &BuildOptions) -> Result<Self> {
        let resolve = metadata
            .resolve
            .as_ref()
            .context("Cargo returned no dependency graph")?;
        let selection = &options.selection;
        let mut selected = if selection.workspace {
            metadata.workspace_members.clone()
        } else if selection.packages.is_empty() {
            metadata.workspace_default_members.iter().cloned().collect()
        } else {
            Vec::new()
        };
        for spec in &selection.packages {
            let mut command = project::cargo();
            command.args(["pkgid", "--package", spec]);
            if let Some(path) = &options.manifest_path {
                command.arg("--manifest-path").arg(path);
            }
            let output = project::checked_output(&mut command)?;
            let id = String::from_utf8(output.stdout)?;
            let package = metadata
                .packages
                .iter()
                .find(|package| package.id.repr == id.trim())
                .context("selected package is absent from metadata")?;
            ensure!(
                metadata.workspace_members.contains(&package.id),
                "selected package is not a workspace member"
            );
            selected.push(package.id.clone());
        }
        selected.retain(|id| {
            !metadata.packages.iter().any(|package| {
                package.id == *id
                    && selection
                        .exclude
                        .iter()
                        .any(|spec| spec == package.name.as_str() || spec == &package.id.repr)
            })
        });
        let mut libraries = Vec::new();
        while let Some(id) = selected.pop() {
            if libraries.contains(&id) {
                continue;
            }
            // Shared consumers must agree on one dynamic copy of every transitive crate.
            libraries.push(id.clone());
            let node = resolve
                .nodes
                .iter()
                .find(|node| node.id == id)
                .context("missing dependency node")?;
            for dependency in &node.deps {
                if dependency
                    .dep_kinds
                    .iter()
                    .any(|kind| kind.kind == DependencyKind::Normal)
                {
                    selected.push(dependency.pkg.clone());
                }
            }
        }
        let mut sources = BTreeMap::new();
        for package in &metadata.packages {
            if !libraries.contains(&package.id) {
                continue;
            }
            for target in &package.targets {
                if target.is_lib() && !target.kind.contains(&TargetKind::ProcMacro) {
                    sources.insert(
                        target.src_path.as_std_path().canonicalize()?,
                        format!("{}@{}:{}", package.name, package.version, target.name),
                    );
                }
            }
        }

        Ok(Self {
            sources,
            mangle_library: options.mangle_library,
        })
    }
}

pub fn value<'a>(args: &'a [OsString], key: &str) -> Option<&'a str> {
    for (index, argument) in args.iter().enumerate() {
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if argument == key {
            return args.get(index + 1)?.to_str();
        }
        if let Some(value) = argument
            .strip_prefix(key)
            .and_then(|tail| tail.strip_prefix('='))
        {
            return Some(value);
        }
        if argument == "-C"
            && let Some(codegen) = key.strip_prefix("-C")
            && let Some(value) = args
                .get(index + 1)?
                .to_str()?
                .strip_prefix(codegen)
                .and_then(|tail| tail.strip_prefix('='))
        {
            return Some(value);
        }
    }
    None
}

pub fn adapt(
    args: &mut Vec<OsString>,
    policy_path: &Path,
    library_log: &Path,
) -> Result<Option<Library>> {
    let policy: Policy = serde_json::from_slice(&fs::read(policy_path)?)?;
    let direct = args
        .iter()
        .filter_map(|arg| arg.to_str())
        .filter(|arg| {
            Path::new(arg)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
        })
        .filter_map(|arg| Path::new(arg).canonicalize().ok())
        .find_map(|source| policy.sources.get(&source));
    let libraries = read_libraries(library_log)?;
    for argument in args.iter_mut() {
        let Some(text) = argument.to_str() else {
            continue;
        };
        // Cargo must wait for linkable dependencies, not just their pipelined metadata.
        if let Some(options) = text.strip_prefix("--json=") {
            *argument = format!(
                "--json={}",
                options
                    .split(',')
                    .filter(|option| *option != "artifacts")
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .into();
            continue;
        }
        if let Some((name, path)) = text.split_once('=')
            && !name.starts_with('-')
            && matches!(
                Path::new(path)
                    .extension()
                    .and_then(|extension| extension.to_str()),
                Some("rlib" | "rmeta")
            )
        {
            let rlib = Path::new(path).with_extension("rlib");
            let linked = libraries
                .iter()
                .find(|library| library.rlib == rlib)
                .map_or(rlib.as_path(), |library| library.dylib.as_path());
            *argument = format!(
                "{name}={}",
                linked.to_str().context("library path is not UTF-8")?
            )
            .into();
        }
    }
    // Newer Cargo versions omit embedded metadata from rlibs by default.
    args.push("-Zembed-metadata=yes".into());
    args.push("-Cprefer-dynamic=yes".into());
    if cfg!(target_os = "linux") {
        // Reserve 16 bytes per possible renamed dependency, plus alignment slack.
        // Repeated /. components resolve to the same directory and are removed later.
        let padding = if policy.mangle_library {
            "/.".repeat(8 * (policy.sources.len() + 4))
        } else {
            String::new()
        };
        args.push(format!("-Clink-arg=-Wl,-rpath,$ORIGIN{padding}").into());
    }
    let Some(identity) = direct else {
        return Ok(None);
    };
    let kinds: Vec<_> = args
        .windows(2)
        .filter(|pair| pair[0] == "--crate-type")
        .filter_map(|pair| pair[1].to_str())
        .collect();
    ensure!(
        kinds
            .iter()
            .all(|kind| matches!(*kind, "lib" | "rlib" | "dylib")),
        "a direct Rust dependency has an incompatible crate type"
    );
    let name = value(args, "--crate-name").context("missing crate name")?;
    let output =
        PathBuf::from(value(args, "--out-dir").context("missing library output directory")?);
    let extra = value(args, "-Cextra-filename").unwrap_or("");
    let stem = format!("{name}{extra}");
    let library = Library {
        identity: identity.clone(),
        rlib: output.join(format!("lib{stem}.rlib")),
        dylib: output.join(if cfg!(windows) {
            format!("{stem}.dll")
        } else {
            format!("lib{stem}.so")
        }),
    };
    if !kinds.contains(&"dylib") {
        args.push("--crate-type=dylib".into());
    }
    // A no_std library still needs the application's allocator and panic runtime.
    args.extend([
        "-Zunstable-options".into(),
        "--extern".into(),
        "force,nounused:std".into(),
    ]);
    Ok(Some(library))
}

pub fn read_libraries(path: &Path) -> Result<Vec<Library>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock_shared()?;
    let mut libraries = Vec::new();
    for line in BufReader::new(&file).lines() {
        libraries.push(serde_json::from_str(&line?)?);
    }
    Ok(libraries)
}

pub fn record(path: &Path, library: &Library) -> Result<()> {
    ensure!(
        library.dylib.is_file(),
        "compiler produced no expected shared library: {}",
        library.dylib.display()
    );
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.lock_exclusive()?;
    serde_json::to_writer(&mut file, library)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn standard_library(sysroot: &Path, target: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for directory in [
        sysroot.join("lib/rustlib").join(target).join("lib"),
        sysroot.join("bin"),
    ] {
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.starts_with("libstd-") && name.ends_with(".so"))
                || (name.starts_with("std-") && name.ends_with(".dll"))
            {
                candidates.push(entry.path());
            }
        }
    }
    ensure!(
        candidates.len() == 1,
        "compiler sysroot must contain one dynamic standard library"
    );
    candidates.pop().context("missing standard library")
}
