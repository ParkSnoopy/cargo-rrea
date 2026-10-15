//! Shared child-process and file-lock operations.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};
use fs2::FileExt;

pub fn checked_output(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("cannot start {}", command.get_program().to_string_lossy()))?;
    ensure!(
        output.status.success(),
        "{} failed ({}): {}",
        command.get_program().to_string_lossy(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

pub fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    if file.try_lock_exclusive().is_err() {
        writeln!(io::stderr().lock(), "waiting for {}", path.display())?;
        file.lock_exclusive()?;
    }
    Ok(file)
}
