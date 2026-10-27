use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const WINDOWS_ASSEMBLY: &str =
    "type='win32' name='libs' version='1.0.0.0' processorArchitecture='amd64'";

pub fn is_library(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("so") || extension.eq_ignore_ascii_case("dll")
        })
}

pub fn windows_link_arguments() -> [String; 2] {
    [
        "-Clink-arg=/MANIFEST:EMBED".to_owned(),
        format!("-Clink-arg=/MANIFESTDEPENDENCY:{WINDOWS_ASSEMBLY} language='*'"),
    ]
}

pub fn stage(
    source: &Path,
    destination: &Path,
    paths: &[&Path],
    windows: bool,
) -> Result<BTreeSet<PathBuf>> {
    let mut entries = BTreeSet::new();
    for path in paths {
        let name = path
            .components()
            .next()
            .context("artifact has no filename")?;
        entries.insert(PathBuf::from(name.as_os_str()));
        let target = destination.join(path);
        fs::create_dir_all(target.parent().context("artifact has no parent")?)?;
        fs::copy(source.join(path), target)?;
    }
    if windows {
        write_windows_manifest(
            &destination.join("libs"),
            paths.iter().copied().filter(|path| is_library(path)),
        )?;
        entries.insert(PathBuf::from("libs"));
    }
    Ok(entries)
}

fn write_windows_manifest<'a>(
    directory: &Path,
    libraries: impl IntoIterator<Item = &'a Path>,
) -> Result<()> {
    let mut manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\n\
         <assemblyIdentity {WINDOWS_ASSEMBLY}/>\n"
    );
    for library in libraries {
        let name = library
            .file_name()
            .and_then(|name| name.to_str())
            .context("runtime library name is not UTF-8")?;
        let name = name
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        writeln!(manifest, "<file name=\"{name}\"/>")?;
    }
    manifest.push_str("</assembly>\n");
    fs::create_dir_all(directory)?;
    fs::write(directory.join("libs.manifest"), manifest)?;
    Ok(())
}
