//! Cargo release builds with verified Arkari tooling, metadata stripping, and UPX packing.

mod audit;
mod build;
mod dynamic;
mod project;
mod support;
mod toolchain;
mod tools;

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    version,
    about = "Build Rust executables with Arkari obfuscation and UPX"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Action>,
    #[command(flatten)]
    build: BuildOptions,
}

#[derive(Subcommand)]
enum Action {
    /// Disable release debug information and strip symbols in Cargo.toml.
    Init {
        #[arg(long, value_name = "PATH")]
        manifest_path: Option<PathBuf>,
    },
}

#[derive(Args, Default, PartialEq, Eq)]
struct BuildOptions {
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    #[arg(short = 'p', long = "package", value_name = "SPEC")]
    packages: Vec<String>,
    #[arg(long, value_name = "NAME", conflicts_with = "bins")]
    bin: Vec<String>,
    #[arg(long)]
    bins: bool,
    #[arg(long)]
    workspace: bool,
    #[arg(long, requires = "workspace", value_name = "SPEC")]
    exclude: Vec<String>,
    #[arg(short = 'F', long, value_name = "FEATURES")]
    features: Vec<String>,
    #[arg(long)]
    all_features: bool,
    #[arg(long)]
    no_default_features: bool,
    #[arg(long, value_name = "TRIPLE")]
    target: Option<String>,
    #[arg(long)]
    locked: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long)]
    frozen: bool,
    #[arg(short = 'j', long, value_name = "N")]
    jobs: Option<std::num::NonZeroUsize>,
}

fn run() -> Result<()> {
    if std::env::var_os("RREA_COMPILER").is_some() {
        return build::rustc_wrapper();
    }
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "rrea") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);
    match cli.command {
        Some(Action::Init { manifest_path }) => {
            // A subcommand must not silently discard build selections.
            if cli.build != BuildOptions::default() {
                bail!("build options cannot be combined with init");
            }
            project::init(manifest_path.as_deref())
        }
        None => build::run(&cli.build),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
