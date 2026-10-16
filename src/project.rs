use std::{ffi::OsString, process::Command};

pub use crate::support::{checked_output, lock};

pub fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
}
