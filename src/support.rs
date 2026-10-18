//! Shared child-process and file-lock operations.

use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    process::{Command, Output},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, ensure};
use fs2::FileExt;

pub static HIDE_PROGRESS: AtomicBool = AtomicBool::new(false);

pub fn progress(message: fmt::Arguments<'_>) -> Result<()> {
    if !HIDE_PROGRESS.load(Ordering::Relaxed) {
        writeln!(io::stderr().lock(), "{message}")?;
    }
    Ok(())
}

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
        progress(format_args!("waiting for {}", path.display()))?;
        file.lock_exclusive()?;
    }
    Ok(file)
}
