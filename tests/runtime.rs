//! Shared runtime layout and actual Windows side-by-side loader coverage.

use std::{fs, path::Path};

use anyhow::{Result, ensure};

#[path = "../src/runtime.rs"]
mod runtime;

#[test]
fn runtime_manifest_lists_final_dll_names() -> Result<()> {
    for name in ["libstd-abcd.so", "std-abcd.dll", "dependency.DLL"] {
        ensure!(runtime::is_library(Path::new(name)), "missing library kind");
    }
    for name in ["program.exe", "dependency.lib", "libstatic.a"] {
        ensure!(
            !runtime::is_library(Path::new(name)),
            "not a runtime library"
        );
    }
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("libs"))?;
    fs::write(source.join("libs/renamed&abcd.dll"), b"fixture")?;
    let destination = root.path().join("distribution");
    let entries = runtime::stage(
        &source,
        &destination,
        &[Path::new("libs/renamed&abcd.dll")],
        true,
    )?;
    ensure!(
        entries.len() == 1 && entries.contains(Path::new("libs")),
        "runtime ownership omits libs"
    );
    let manifest = fs::read_to_string(destination.join("libs/libs.manifest"))?;
    ensure!(manifest.contains("name='libs'"), "wrong assembly identity");
    ensure!(
        manifest.contains("<file name=\"renamed&amp;abcd.dll\"/>"),
        "DLL name was not escaped"
    );
    ensure!(
        runtime::windows_link_arguments()[1].contains("name='libs'"),
        "linker dependency differs from assembly identity"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
#[ignore = "requires clang, RREA_PE_LINKER (lld-link or rust-lld), and Wine"]
fn windows_bundle_loads_transitive_dlls_after_relocation() -> Result<()> {
    let root = tempfile::tempdir()?;
    let objects = root.path().join("objects");
    fs::create_dir_all(&objects)?;
    let prefix = root.path().join("wine");
    for names in [
        ["leaf-abcd.dll", "middle-abcd.dll"],
        ["0123456789abcdef-abcd.dll", "fedcba9876543210-abcd.dll"],
    ] {
        build_windows_bundle(&objects, names)?;
        for mode in ["unpacked", "normal", "opaque"] {
            let bundle = root.path().join("bundle");
            let libs = bundle.join("libs");
            fs::create_dir_all(&libs)?;
            for name in [names[0], names[1], "program.exe"] {
                let destination = if runtime::is_library(Path::new(name)) {
                    libs.join(name)
                } else {
                    bundle.join(name)
                };
                if mode == "unpacked" {
                    fs::copy(objects.join(name), destination)?;
                } else {
                    let mut pack = std::process::Command::new(env!("RREA_UPX_PATH"));
                    pack.args([
                        "--no-env",
                        "--best",
                        "--strip-relocs=0",
                        "--compress-exports=0",
                        "--compress-resources=0",
                    ]);
                    if mode == "opaque" {
                        pack.arg("--opaque");
                    }
                    checked(pack.arg(objects.join(name)).arg("-o").arg(destination))?;
                }
            }
            let relocated = root.path().join("relocated");
            let paths = [
                Path::new("libs").join(names[0]),
                Path::new("libs").join(names[1]),
                "program.exe".into(),
            ];
            runtime::stage(
                &bundle,
                &relocated,
                &paths.each_ref().map(std::path::PathBuf::as_path),
                true,
            )?;
            fs::remove_dir_all(bundle)?;
            run_windows_bundle(root.path(), &prefix, &relocated, true)?;
            fs::remove_file(relocated.join("libs").join(names[0]))?;
            run_windows_bundle(root.path(), &prefix, &relocated, false)?;
            fs::remove_dir_all(relocated)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn build_windows_bundle(directory: &Path, names: [&str; 2]) -> Result<()> {
    use std::process::Command;

    let linker = std::env::var_os("RREA_PE_LINKER")
        .ok_or_else(|| anyhow::anyhow!("missing RREA_PE_LINKER"))?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/windows-runtime.c");
    for (kind, name) in [
        ("LEAF", names[0]),
        ("MIDDLE", names[1]),
        ("MAIN", "program.exe"),
    ] {
        let object = directory.join(format!("{kind}.obj"));
        checked(
            Command::new("clang")
                .args([
                    "--target=x86_64-pc-windows-msvc",
                    "-O2",
                    "-c",
                    "-fno-stack-protector",
                ])
                .arg(format!("-D{kind}"))
                .arg(&source)
                .arg("-o")
                .arg(&object),
        )?;
        let mut link = Command::new(&linker);
        if Path::new(&linker)
            .file_stem()
            .is_some_and(|stem| stem == "rust-lld")
        {
            link.args(["-flavor", "link"]);
        }
        link.args(["/nologo", "/nodefaultlib"])
            .arg(&object)
            .arg(format!("/out:{}", directory.join(name).display()));
        if kind == "MAIN" {
            link.args(["/entry:entry", "/subsystem:console"]);
            link.args(
                runtime::windows_link_arguments()
                    .iter()
                    .filter_map(|argument| argument.strip_prefix("-Clink-arg=")),
            );
        } else {
            link.args(["/dll", "/noentry"]).arg(format!(
                "/implib:{}",
                directory.join(format!("{name}.lib")).display()
            ));
        }
        if kind != "LEAF" {
            link.arg(directory.join(format!(
                "{}.lib",
                if kind == "MIDDLE" { names[0] } else { names[1] }
            )));
        }
        checked(&mut link)?;
    }
    Ok(())
}

#[cfg(unix)]
fn run_windows_bundle(cwd: &Path, prefix: &Path, bundle: &Path, expected: bool) -> Result<()> {
    let output = std::process::Command::new("wine")
        .current_dir(cwd)
        .env("WINEPREFIX", prefix)
        .env("WINEDEBUG", "-all")
        .arg(bundle.join("program.exe"))
        .output()?;
    ensure!(
        output.status.success() == expected,
        "Windows bundle load: expected success={expected}, status={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[cfg(unix)]
fn checked(command: &mut std::process::Command) -> Result<()> {
    let output = command.output()?;
    ensure!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
