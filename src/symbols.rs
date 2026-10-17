use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use anyhow::{Context, Result, ensure};
use object::{
    Object,
    ObjectSection,
    RelocationFlags,
    RelocationTarget,
    elf,
    read::elf::{SectionHeader, Sym},
};
use serde::Serialize;

use crate::tools;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Metadata {
    pub symbols: Vec<Symbol>,
    pub relocations: Vec<Relocation>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Symbol {
    #[serde(flatten)]
    pub reference: Reference,
    pub section: Section,
    pub address: u64,
    pub size: u64,
    pub info: u8,
    pub other: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Reference {
    pub name: String,
    pub version: Option<String>,
    pub version_file: Option<String>,
    pub version_hidden: bool,
    pub version_local: bool,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Section {
    Named { name: String, allocated: bool },
    Special(u16),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Relocation {
    pub address: u64,
    pub kind: u32,
    pub addend: i64,
    pub symbol: Reference,
}

pub fn inspect(file: &object::File<'_>) -> Result<Option<Metadata>> {
    let object::File::Elf64(file) = file else {
        return Ok(None);
    };
    let endian = file.endian();
    let table = file.elf_dynamic_symbol_table();
    let versions = file
        .elf_section_table()
        .versions(endian, file.data())?
        .unwrap_or_default();
    let mut symbols = Vec::new();
    for (index, symbol) in table.enumerate().skip(1) {
        let version_index = versions.version_index(endian, index);
        let version = versions.version(version_index.index())?;
        let section = match table.symbol_section(endian, symbol, index)? {
            Some(index) => {
                Section::Named {
                    name: file.section_by_index(index)?.name()?.to_owned(),
                    allocated: file
                        .elf_section_table()
                        .section(index)?
                        .sh_flags(endian)
                        .contains(elf::SHF_ALLOC),
                }
            }
            None => Section::Special(symbol.st_shndx(endian).0),
        };
        symbols.push(Symbol {
            section,
            address: symbol.st_value(endian),
            size: symbol.st_size(endian),
            info: symbol.st_info().0,
            other: symbol.st_other().0,
            reference: Reference {
                name: std::str::from_utf8(table.symbol_name(endian, symbol)?)?.to_owned(),
                version: version
                    .map(|v| std::str::from_utf8(v.name()).map(str::to_owned))
                    .transpose()?,
                version_file: version
                    .and_then(object::read::elf::Version::file)
                    .map(|name| std::str::from_utf8(name).map(str::to_owned))
                    .transpose()?,
                version_hidden: version_index.is_hidden(),
                version_local: version_index.index() == elf::VER_NDX_LOCAL,
            },
        });
    }
    let mut relocations = Vec::new();
    for (address, relocation) in file.dynamic_relocations().into_iter().flatten() {
        let RelocationFlags::Elf { r_type } = relocation.flags() else {
            anyhow::bail!("invalid ELF relocation flags");
        };
        let symbol = match relocation.target() {
            RelocationTarget::Symbol(object::SymbolIndex(0)) | RelocationTarget::Absolute => {
                Reference::default()
            }
            RelocationTarget::Symbol(index) => {
                symbols
                    .get(index.0 - 1)
                    .context("invalid dynamic relocation symbol")?
                    .reference
                    .clone()
            }
            _ => anyhow::bail!("unsupported ELF dynamic relocation target"),
        };
        relocations.push(Relocation {
            address,
            kind: r_type.0,
            addend: relocation.addend(),
            symbol,
        });
    }
    symbols.sort();
    relocations.sort();
    Ok(Some(Metadata {
        symbols,
        relocations,
    }))
}

#[derive(Debug, Default, Serialize)]
pub struct Plan {
    pub renamed: BTreeMap<String, String>,
    pub retained: BTreeMap<String, Retention>,
    // Reported separately even when optional renaming is disabled.
    #[serde(skip)]
    pub removed_metadata: BTreeSet<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.renamed.is_empty() && self.removed_metadata.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum Retention {
    Requested,
    StableAbi,
    External,
    AmbiguousDefinition,
    Versioned,
    BindingOrType,
}

pub fn plan(artifacts: &BTreeMap<PathBuf, PathBuf>, keep: Option<&[String]>) -> Result<Plan> {
    let mut occurrences: BTreeMap<String, Vec<(String, Symbol)>> = BTreeMap::new();
    let mut referenced = BTreeSet::new();
    for (relative, path) in artifacts {
        let data = fs::read(path)?;
        let file = object::File::parse(data.as_slice())?;
        let Some(metadata) = inspect(&file)? else {
            ensure!(keep.is_none(), "symbol mangling requires ELF64 artifacts");
            continue;
        };
        referenced.extend(
            metadata
                .relocations
                .into_iter()
                .map(|relocation| relocation.symbol.name),
        );
        for symbol in metadata.symbols {
            occurrences
                .entry(symbol.reference.name.clone())
                .or_default()
                .push((
                    relative
                        .to_str()
                        .context("artifact name is not UTF-8")?
                        .to_owned(),
                    symbol,
                ));
        }
    }
    let mut result = Plan {
        removed_metadata: metadata_removals(&occurrences, &referenced)?,
        ..Plan::default()
    };
    let Some(keep) = keep else {
        return Ok(result);
    };
    for name in keep {
        ensure!(
            is_rust(name) && occurrences.contains_key(name),
            "unknown Rust keep-symbol: {name}"
        );
    }
    let mut assigned = BTreeSet::new();
    for (name, entries) in &occurrences {
        if !is_rust(name) {
            if !result.removed_metadata.contains(name)
                && (name.starts_with("rust_") || name.starts_with("__rust_"))
            {
                result.retained.insert(name.clone(), Retention::StableAbi);
            }
            continue;
        }
        if keep.contains(name) {
            result.retained.insert(name.clone(), Retention::Requested);
            continue;
        }
        let provider = match eligible_provider(entries) {
            Ok(provider) => provider,
            Err(reason) => {
                result.retained.insert(name.clone(), reason);
                continue;
            }
        };
        let digest = tools::digest(format!("symbol-v1\0{provider}\0{name}").as_bytes())?;
        // Never enlarge an individual symbol string; collisions fail before writing artifacts.
        let replacement = format!("s{}", &digest[..name.len().saturating_sub(1).min(32)]);
        ensure!(
            !occurrences.contains_key(&replacement) && assigned.insert(replacement.clone()),
            "mangled symbol collision"
        );
        result.renamed.insert(name.clone(), replacement);
    }
    Ok(result)
}

fn metadata_removals(
    occurrences: &BTreeMap<String, Vec<(String, Symbol)>>,
    referenced: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut removed = BTreeSet::new();
    for (name, entries) in occurrences {
        if !entries.iter().any(|(_, symbol)| {
            matches!(&symbol.section,
            Section::Named { name, .. } if crate::audit::unwanted_section(name))
        }) {
            continue;
        }
        ensure!(!referenced.contains(name) && entries.iter().all(|(_, symbol)| {
            name.starts_with("rust_metadata_") && symbol.address == 0
                && elf::SymbolInfo(symbol.info).st_type() == elf::STT_OBJECT
                && symbol.reference.version.is_none()
                && matches!(&symbol.section, Section::Named { name, allocated: false } if name == ".rustc")
        }), "cannot remove referenced or unexpected metadata symbol: {name}");
        removed.insert(name.clone());
    }
    Ok(removed)
}

pub fn is_rust(name: &str) -> bool {
    let legacy = name
        .strip_suffix('E')
        .and_then(|name| name.rsplit_once("17h"))
        .is_some_and(|(_, hash)| {
            hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
    (name.starts_with("_R") || (name.starts_with("_ZN") && legacy))
        && rustc_demangle::try_demangle(name).is_ok()
}

fn eligible_provider(entries: &[(String, Symbol)]) -> Result<&str, Retention> {
    let mut definitions = entries
        .iter()
        .filter(|(_, symbol)| symbol.section != Section::Special(0));
    let Some((provider, _)) = definitions.next() else {
        return Err(Retention::External);
    };
    if definitions.next().is_some() {
        return Err(Retention::AmbiguousDefinition);
    }
    for (_, symbol) in entries {
        let defined = symbol.section != Section::Special(elf::SHN_UNDEF.0);
        if symbol.reference.version.is_some()
            || symbol.reference.version_hidden
            || (defined && symbol.reference.version_local)
        {
            return Err(Retention::Versioned);
        }
        let info = elf::SymbolInfo(symbol.info);
        if info.st_bind() != elf::STB_GLOBAL
            || symbol.other != elf::STV_DEFAULT.0
            || matches!(
                symbol.section,
                Section::Named {
                    allocated: false,
                    ..
                }
            )
            || (defined && !matches!(info.st_type(), elf::STT_FUNC | elf::STT_OBJECT))
            || !matches!(
                info.st_type(),
                elf::STT_FUNC | elf::STT_OBJECT | elf::STT_NOTYPE
            )
            || matches!(symbol.section, Section::Special(index) if index != elf::SHN_UNDEF.0)
        {
            return Err(Retention::BindingOrType);
        }
    }
    Ok(provider)
}

impl Metadata {
    pub fn rename(
        &mut self,
        libraries: &BTreeMap<String, String>,
        symbols: &BTreeMap<String, String>,
    ) {
        for symbol in self
            .symbols
            .iter_mut()
            .map(|symbol| &mut symbol.reference)
            .chain(
                self.relocations
                    .iter_mut()
                    .map(|relocation| &mut relocation.symbol),
            )
        {
            if let Some(name) = symbols.get(&symbol.name) {
                symbol.name.clone_from(name);
            }
            if let Some(file) = &mut symbol.version_file
                && let Some(name) = libraries.get(file)
            {
                file.clone_from(name);
            }
        }
        self.symbols.sort();
        self.relocations.sort();
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "../tests/symbols/mod.rs"]
mod tests;
