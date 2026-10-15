use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use toml_edit::{DocumentMut, Item, Table, Value};

pub use crate::support::{checked_output, lock};

pub fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
}


pub fn workspace_manifest(manifest: Option<&Path>) -> Result<PathBuf> {
    let mut command = cargo();
    command.args(["locate-project", "--workspace", "--message-format", "json"]);
    if let Some(path) = manifest {
        command.arg("--manifest-path").arg(path);
    }
    let output = checked_output(&mut command)?;
    #[derive(serde::Deserialize)]
    struct Located {
        root: PathBuf,
    }
    Ok(serde_json::from_slice::<Located>(&output.stdout)?.root)
}

pub fn init(manifest: Option<&Path>) -> Result<()> {
    let path = workspace_manifest(manifest)?;
    let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
    file.lock_exclusive()?;
    let mut original = String::new();
    file.read_to_string(&mut original)?;
    let updated = release_settings(&original)?;
    if updated == original {
        println!("unchanged {}", path.display());
        return Ok(());
    }
    let parent = path.parent().context("manifest has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(file.metadata()?.permissions())?;
    temporary.write_all(updated.as_bytes())?;
    temporary.as_file().sync_all()?;
    ensure!(
        fs::read_to_string(&path)? == original,
        "{} changed during init; retry the command",
        path.display()
    );
    temporary.persist(&path).map_err(|error| error.error)?;
    println!("updated {}", path.display());
    Ok(())
}

fn release_settings(original: &str) -> Result<String> {
    let mut document = original
        .parse::<DocumentMut>()
        .context("invalid Cargo.toml")?;
    if !document.contains_key("profile") {
        let mut profile = Table::new();
        profile.set_implicit(true);
        document.insert("profile", Item::Table(profile));
    }
    let profile = document["profile"]
        .as_table_like_mut()
        .context("profile must be a TOML table")?;
    if !profile.contains_key("release") {
        profile.insert("release", Item::Table(Table::new()));
    }
    let release = profile
        .get_mut("release")
        .and_then(Item::as_table_like_mut)
        .context("profile.release must be a TOML table")?;
    for (key, replacement) in [("debug", Value::from(0)), ("strip", Value::from("symbols"))] {
        if let Some(item) = release.get_mut(key) {
            let old = item
                .as_value()
                .with_context(|| format!("profile.release.{key} must be a value"))?;
            if old.to_string().trim() == replacement.to_string() {
                continue;
            }
            let mut replacement = replacement;
            *replacement.decor_mut() = old.decor().clone();
            *item = Item::Value(replacement);
        } else {
            release.insert(key, Item::Value(replacement));
        }
    }
    Ok(document.to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_preserves_comments_settings_and_is_idempotent() -> Result<()> {
        for source in [
            "# project\n[package]\nname = 'fixture'\nversion = '0.1.0'\n",
            "[workspace]\nmembers = []\n\n[profile.release]\n# retain this\ndebug    = 2 # diagnosis\nstrip    = 'none' # symbols\nlto      = 'thin'\n",
            "profile = { release = { debug = 2, strip = 'none', lto = 'thin' } }\n",
            "[profile.release.package.fixture]\ndebug = 1\n",
            "[profile]\nrelease.debug = 2\nrelease.strip = 'none'\n",
        ] {
            let updated = release_settings(source)?;
            let parsed = updated.parse::<DocumentMut>()?;
            assert_eq!(parsed["profile"]["release"]["debug"].as_integer(), Some(0));
            assert_eq!(
                parsed["profile"]["release"]["strip"].as_str(),
                Some("symbols")
            );
            assert_eq!(release_settings(&updated)?, updated);
            for comment in ["# project", "# retain this", "# diagnosis", "# symbols"] {
                if source.contains(comment) {
                    assert!(updated.contains(comment), "lost {comment}: {updated}");
                }
            }
            if source.contains("lto") {
                assert_eq!(parsed["profile"]["release"]["lto"].as_str(), Some("thin"));
            }
        }
        for invalid in [
            "[profile",
            "profile = 4",
            "[profile]\nrelease = 4",
            "[profile.release.debug]\nx = 1",
        ] {
            assert!(release_settings(invalid).is_err(), "accepted {invalid}");
        }
        Ok(())
    }
}
