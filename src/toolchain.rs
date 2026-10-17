use std::{
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::support::{checked_output, lock};

const COMPILER_REPO: &str = "ParkSnoopy/rust_llvm-arkari_ollvm";

const MAX_ARCHIVE: u64 = 1024 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 4 * MAX_ARCHIVE;

pub fn host() -> Result<&'static str> {
    if cfg!(all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "gnu"
    )) {
        Ok("x86_64-unknown-linux-gnu")
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        Ok("x86_64-pc-windows-msvc")
    } else {
        bail!("only native x86_64 Linux GNU and Windows MSVC builds are supported")
    }
}


#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAsset {
    pub repository: String,
    pub tag: String,
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

impl ReleaseAsset {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/releases/download/{}/{}",
            self.repository, self.tag, self.name
        )
    }

    fn validate(&self, host: &str) -> Result<()> {
        ensure!(
            self.repository == COMPILER_REPO,
            "unexpected cached repository"
        );
        ensure!(safe_name(&self.tag), "invalid release tag");
        ensure!(
            self.name == format!("{host}.zip"),
            "unexpected release asset"
        );
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "missing or invalid SHA-256 digest"
        );
        ensure!(
            self.bytes > 0 && self.bytes <= MAX_ARCHIVE,
            "invalid release asset size"
        );
        Ok(())
    }
}

pub fn acquire(root: &Path, host: &str, offline: bool) -> Result<(PathBuf, PathBuf, ReleaseAsset)> {
    fs::create_dir_all(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    }
    let _lock = lock(&root.join("tools.lock"))?;
    let (compiler_root, compiler_release) = install_compiler(root, host, offline)?;
    let compiler = compiler_root.join("stage1/bin").join(executable("rustc"));
    ensure!(
        compiler.is_file(),
        "compiler archive has no stage1/bin/rustc"
    );
    let library = compiler_root.join("stage1/lib/rustlib").join(host);
    ensure!(
        library.join("lib").is_dir(),
        "compiler archive has no matching sysroot"
    );
    let objcopy = library.join("bin").join(executable("llvm-objcopy"));
    ensure!(
        objcopy.is_file(),
        "compiler archive has no matching llvm-objcopy"
    );
    Ok((compiler, objcopy, compiler_release))
}

pub fn executable(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}


fn install_compiler(root: &Path, host: &str, offline: bool) -> Result<(PathBuf, ReleaseAsset)> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        size: u64,
        digest: Option<String>,
        browser_download_url: String,
    }
    let kind = "compiler";
    let repository = COMPILER_REPO;
    let record_path = root.join("compiler.json");
    if record_path.exists() {
        let record: ReleaseAsset = serde_json::from_slice(&fs::read(&record_path)?)
            .with_context(|| format!("invalid cache record {}", record_path.display()))?;
        record.validate(host)?;
        let destination = root.join(format!("{kind}-{}", record.tag));
        ensure!(
            destination.is_dir(),
            "cached {kind} is missing: {}",
            destination.display()
        );
        return Ok((destination, record));
    }
    ensure!(
        !offline,
        "{kind} is not cached; offline mode forbids downloading it"
    );
    writeln!(io::stderr().lock(), "resolving {repository} release")?;
    let response = tempfile::NamedTempFile::new_in(root)?;
    download(
        &format!("https://api.github.com/repos/{repository}/releases/latest"),
        response.path(),
        4 * 1024 * 1024,
    )?;
    let release: Release = serde_json::from_slice(&fs::read(response.path())?)?;
    ensure!(safe_name(&release.tag_name), "invalid release tag");
    let name = format!("{host}.zip");
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("release {} has no {name}", release.tag_name))?;
    let record = ReleaseAsset {
        repository: repository.to_owned(),
        tag: release.tag_name,
        name,
        sha256: asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .context("release asset has no SHA-256 digest")?
            .to_owned(),
        bytes: asset.size,
    };
    record.validate(host)?;
    ensure!(
        asset.browser_download_url == record.url(),
        "unexpected asset download URL"
    );
    let archive = tempfile::NamedTempFile::new_in(root)?;
    writeln!(io::stderr().lock(), "downloading {} {}", kind, record.tag)?;
    download(&record.url(), archive.path(), MAX_ARCHIVE)?;
    ensure!(
        fs::metadata(archive.path())?.len() == record.bytes,
        "downloaded archive size differs from release metadata"
    );
    ensure!(
        sha256(archive.path())? == record.sha256,
        "downloaded archive SHA-256 mismatch"
    );
    let staging = tempfile::tempdir_in(root)?;
    extract_zip(archive.path(), staging.path())?;
    let expected = staging.path().join("stage1/bin").join(executable("rustc"));
    ensure!(expected.is_file(), "{kind} archive layout is unsupported");
    let destination = root.join(format!("{kind}-{}", record.tag));
    // An interrupted install can leave an unreferenced directory, never a trusted record.
    ensure!(
        !destination.exists(),
        "unreferenced tool directory exists; inspect and remove {}",
        destination.display()
    );
    fs::rename(staging.path(), &destination)?;
    let mut record_file = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut record_file, &record)?;
    record_file.write_all(b"\n")?;
    record_file.as_file().sync_all()?;
    record_file
        .persist(&record_path)
        .map_err(|error| error.error)?;
    Ok((destination, record))
}

fn download(url: &str, destination: &Path, max_bytes: u64) -> Result<()> {
    checked_output(
        Command::new("curl")
            .args([
                "--disable",
                "-fsSL",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "30",
                "--max-time",
                "1800",
                "--retry",
                "2",
                "--user-agent",
                "cargo-rrea",
            ])
            .arg("--max-filesize")
            .arg(max_bytes.to_string())
            .arg("--output")
            .arg(destination)
            .arg(url),
    )?;
    ensure!(
        fs::metadata(destination)?.len() <= max_bytes,
        "download exceeds size limit"
    );
    Ok(())
}

pub fn sha256(path: &Path) -> Result<String> {
    digest(File::open(path)?)
}

pub fn digest(mut reader: impl Read) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let mut hex = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(hex, "{byte:02x}")?;
    }
    Ok(hex)
}

fn archive_path(name: &Path) -> Result<&Path> {
    let text = name.to_str().context("archive path is not UTF-8")?;
    ensure!(!text.contains(['\\', ':']), "invalid archive path: {text}");
    ensure!(
        !text.is_empty()
            && name
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
        "unsafe archive path: {text}"
    );
    Ok(name)
}

fn extract_file(
    reader: &mut impl Read,
    destination: &Path,
    bytes: u64,
    executable: bool,
) -> Result<()> {
    fs::create_dir_all(
        destination
            .parent()
            .context("archive entry has no parent")?,
    )?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = io::copy(&mut reader.take(bytes + 1), &mut file)?;
    ensure!(copied == bytes, "archive entry size mismatch");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(if executable {
            0o755
        } else {
            0o644
        }))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

pub fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
    ensure!(archive.len() <= 100_000, "too many archive entries");
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let path = destination.join(archive_path(Path::new(entry.name()?.as_ref()))?);
        let mode = entry.unix_mode().unwrap_or(0);
        let file_type = mode & 0o170_000;
        ensure!(
            matches!(file_type, 0 | 0o100_000 | 0o040_000),
            "archive links and special files are unsupported"
        );
        total = total
            .checked_add(entry.size())
            .context("archive size overflow")?;
        ensure!(total <= MAX_EXTRACTED, "archive expands beyond size limit");
        if entry.is_dir() {
            fs::create_dir_all(path)?;
        } else {
            let bytes = entry.size();
            extract_file(&mut entry, &path, bytes, mode & 0o111 != 0)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn extraction_rejects_traversal_and_links() -> Result<()> {
        use zip::write::SimpleFileOptions;
        for name in ["../escape", "/absolute", "C:/escape", "dir\\escape"] {
            ensure!(
                archive_path(Path::new(name)).is_err(),
                "accepted unsafe archive path {name}"
            );
        }
        let root = tempfile::tempdir()?;
        for (name, link) in [("../escape", false), ("link", true)] {
            let archive = tempfile::NamedTempFile::new_in(root.path())?;
            let mut writer = zip::ZipWriter::new(File::create(archive.path())?);
            if link {
                writer.add_symlink(name, "../escape", SimpleFileOptions::default())?;
            } else {
                writer.start_file(name, SimpleFileOptions::default())?;
                writer.write_all(b"payload")?;
            }
            writer.finish()?;
            let destination = tempfile::tempdir_in(root.path())?;
            ensure!(
                extract_zip(archive.path(), destination.path()).is_err(),
                "accepted unsafe archive entry {name}"
            );
        }
        Ok(())
    }
}
