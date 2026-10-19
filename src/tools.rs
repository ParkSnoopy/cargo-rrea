use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::{support::lock, toolchain};
pub use crate::toolchain::{ReleaseAsset, digest, host, sha256};

const UPX: &[u8] = include_bytes!(env!("RREA_UPX_PATH"));

pub struct Tools {
    pub compiler: PathBuf,
    pub objcopy: PathBuf,
    pub upx: PathBuf,
    pub compiler_release: ReleaseAsset,
    pub upx_build: UpxBuild,
}

#[derive(Serialize)]
pub struct UpxBuild {
    pub repository: &'static str,
    pub revision: &'static str,
    pub sha256: String,
}

pub fn acquire(offline: bool) -> Result<Tools> {
    let host = host()?;
    let root = dirs::cache_dir()
        .context("cannot determine user cache directory")?
        .join("cargo-rrea")
        .join(host);
    let executable = std::env::current_exe()?;
    let compiler_cache = executable
        .ancestors()
        .skip(1)
        .map(|directory| directory.join("downloads").join(host))
        .find(|directory| directory.is_dir())
        .unwrap_or_else(|| root.clone());
    let (compiler, objcopy, compiler_release) = toolchain::acquire(&compiler_cache, host, offline)?;
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    let _lock = lock(&root.join("tools.lock"))?;
    let (upx, upx_build) = embedded_upx(&root)?;
    Ok(Tools {
        compiler,
        objcopy,
        upx,
        compiler_release,
        upx_build,
    })
}

fn embedded_upx(root: &Path) -> Result<(PathBuf, UpxBuild)> {
    let build = UpxBuild {
        repository: "ParkSnoopy/upx",
        revision: env!("RREA_UPX_REVISION"),
        sha256: digest(UPX)?,
    };
    let directory = root.join(format!("upx-{}", build.sha256));
    fs::create_dir_all(&directory)?;
    let executable = directory.join(toolchain::executable("upx"));
    if executable.exists() {
        ensure!(
            sha256(&executable)? == build.sha256,
            "cached UPX SHA-256 mismatch"
        );
    } else {
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(UPX)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        file.as_file().sync_all()?;
        file.persist_noclobber(&executable)
            .map_err(|error| error.error)?;
    }
    Ok((executable, build))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::support::checked_output;

    #[test]
    fn embedded_upx_reuses_verified_cache_and_rejects_tampering() -> Result<()> {
        let root = tempfile::tempdir()?;
        let (path, build) = embedded_upx(root.path())?;
        checked_output(Command::new(&path).arg("--version"))?;
        ensure!(sha256(&path)? == build.sha256, "embedded UPX hash differs");
        ensure!(
            embedded_upx(root.path())?.0 == path,
            "cached UPX was not reused"
        );
        fs::write(path, b"corrupted cache")?;
        ensure!(
            embedded_upx(root.path()).is_err(),
            "corrupted UPX was accepted"
        );
        Ok(())
    }
}
