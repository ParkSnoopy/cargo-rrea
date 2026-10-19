use std::collections::BTreeSet;

use clap::{Arg, ArgAction, ArgMatches, Args, Command, FromArgMatches};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feature {
    Indbr,
    Icall,
    Indgv,
    Cse,
    Fla,
    Cie,
    Cfe,
    MangleLibrary,
    MangleSymbols,
    Upx,
    Opaque,
    Strip,
    RemapPaths,
}

const FEATURES: &[(Feature, &str, &str, Option<&str>)] = &[
    (
        Feature::Indbr,
        "no-indbr",
        "Disable Arkari indirect branches",
        Some("-Cllvm-args=--irobf-indbr"),
    ),
    (
        Feature::Icall,
        "no-icall",
        "Disable Arkari indirect calls",
        Some("-Cllvm-args=--irobf-icall"),
    ),
    (
        Feature::Indgv,
        "no-indgv",
        "Disable Arkari global-variable indirection",
        Some("-Cllvm-args=--irobf-indgv"),
    ),
    (
        Feature::Cse,
        "no-cse",
        "Disable Arkari CSE",
        Some("-Cllvm-args=--irobf-cse"),
    ),
    (
        Feature::Fla,
        "no-fla",
        "Disable Arkari control-flow flattening",
        Some("-Cllvm-args=--irobf-fla"),
    ),
    (
        Feature::Cie,
        "no-cie",
        "Disable Arkari CIE",
        Some("-Cllvm-args=--irobf-cie"),
    ),
    (
        Feature::Cfe,
        "no-cfe",
        "Disable Arkari CFE",
        Some("-Cllvm-args=--irobf-cfe"),
    ),
    (
        Feature::MangleLibrary,
        "no-mangle-library",
        "Keep original library filenames",
        None,
    ),
    (
        Feature::MangleSymbols,
        "no-mangle-symbols",
        "Keep original Rust dynamic symbol names (required on Windows)",
        None,
    ),
    (Feature::Upx, "no-upx", "Disable UPX packing", None),
    (
        Feature::Opaque,
        "no-opaque",
        "Use normal UPX compression instead of opaque executable packing (required on Windows)",
        None,
    ),
    (
        Feature::Strip,
        "no-strip",
        "Keep compiler metadata, static symbols, and project debug settings",
        None,
    ),
    (
        Feature::RemapPaths,
        "no-remap-paths",
        "Disable build-path remapping and path-leak rejection",
        None,
    ),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Obfuscation {
    pub disabled: BTreeSet<Feature>,
}

impl Obfuscation {
    pub fn enabled(&self, feature: Feature) -> bool {
        !self.disabled.contains(&feature)
    }

    pub fn llvm_flags(&self) -> Vec<&'static str> {
        let mut flags: Vec<_> = FEATURES
            .iter()
            .filter(|(feature, _, _, _)| self.enabled(*feature))
            .filter_map(|(_, _, _, llvm)| *llvm)
            .collect();
        if !flags.is_empty() {
            flags.insert(0, "-Cllvm-args=--irobf");
        }
        flags
    }
}

impl Args for Obfuscation {
    fn augment_args(mut command: Command) -> Command {
        for (_, flag, help, _) in FEATURES {
            command = command.arg(
                Arg::new(*flag)
                    .long(*flag)
                    .help(*help)
                    .help_heading("Obfuscation (all enabled by default)")
                    .action(ArgAction::SetTrue),
            );
        }
        command
    }

    fn augment_args_for_update(command: Command) -> Command {
        Self::augment_args(command)
    }
}

impl FromArgMatches for Obfuscation {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        Ok(Self {
            disabled: FEATURES
                .iter()
                .filter(|(_, flag, _, _)| matches.get_flag(flag))
                .map(|(feature, _, _, _)| *feature)
                .collect(),
        })
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        self.disabled
            .extend(Self::from_arg_matches(matches)?.disabled);
        Ok(())
    }
}
