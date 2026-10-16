//! Cargo release builds with verified Arkari tooling, metadata stripping, and UPX packing.

mod audit;
mod build;
mod dynamic;
mod mangle;
mod project;
mod support;
mod toolchain;
mod tools;

use std::{
    ffi::OsString,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    version,
    about = "Build Rust executables with Arkari obfuscation and UPX"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Build release executables and required libraries with Arkari and UPX.
    Build(BuildOptions),
}

#[derive(Args)]
struct BuildOptions {
    /// Build with the release profile (required for protection).
    #[arg(short = 'r', long, required = true)]
    release: bool,
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    #[arg(long, value_name = "TRIPLE")]
    target: Option<String>,
    #[arg(short = 'j', long, value_name = "N")]
    jobs: Option<std::num::NonZeroUsize>,
    /// Keep original library names instead of stable 16-character hashes.
    #[arg(long = "no-mangle-library", action = clap::ArgAction::SetFalse, default_value_t = true)]
    mangle_library: bool,
    #[command(flatten)]
    selection: SelectionOptions,
    #[command(flatten)]
    features: FeatureOptions,
    #[command(flatten)]
    network: NetworkOptions,
}

#[derive(Args)]
struct SelectionOptions {
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
}

#[derive(Args)]
struct FeatureOptions {
    #[arg(short = 'F', long = "features", value_name = "FEATURES")]
    enabled: Vec<String>,
    #[arg(long = "all-features")]
    all: bool,
    #[arg(long = "no-default-features")]
    no_default: bool,
}

#[derive(Args)]
struct NetworkOptions {
    #[arg(long)]
    locked: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long)]
    frozen: bool,
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
        Action::Build(options) => build::run(&options),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
