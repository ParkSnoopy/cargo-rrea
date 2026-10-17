use std::{fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use object::{Object, ObjectSection};
use serde::Serialize;

use crate::symbols;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Inspection {
    pub libraries: Vec<String>,
    pub imports: Vec<(String, SymbolName)>,
    pub exports: Vec<SymbolName>,
    pub elf: Option<symbols::Metadata>,
    pub hardening: Hardening,
    pub rust_string_hints: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub enum Hardening {
    Elf {
        pie: bool,
        executable_stack: bool,
        relro: bool,
    },
    Pe {
        dll_characteristics: u16,
    },
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum SymbolName {
    Name(String),
    Ordinal(u16),
}

fn symbol_name(name: object::NameOrOrdinal<&[u8]>) -> Result<SymbolName> {
    Ok(match name {
        object::NameOrOrdinal::Name(name) => {
            SymbolName::Name(std::str::from_utf8(name)?.to_owned())
        }
        object::NameOrOrdinal::Ordinal(ordinal) => SymbolName::Ordinal(ordinal),
    })
}

pub fn inspect(path: &Path, clean: bool, prohibited_paths: &[&Path]) -> Result<Inspection> {
    let data = fs::read(path)?;
    let file = object::File::parse(data.as_slice()).context("cannot inspect executable")?;
    ensure!(
        file.architecture() == object::Architecture::X86_64,
        "executable is not x86_64"
    );
    ensure!(
        matches!(
            file.kind(),
            object::ObjectKind::Executable | object::ObjectKind::Dynamic
        ),
        "artifact is not an executable"
    );
    let hardening = match &file {
        object::File::Elf64(elf) => {
            use object::read::elf::{FileHeader, ProgramHeader};
            let endian = elf.endian();
            Hardening::Elf {
                pie: elf.elf_header().e_type(endian) == object::elf::ET_DYN,
                executable_stack: elf.elf_program_headers().iter().any(|header| {
                    header.p_type(endian) == object::elf::PT_GNU_STACK
                        && (header.p_flags(endian) & object::elf::PF_X).0 != 0
                }),
                relro: elf
                    .elf_program_headers()
                    .iter()
                    .any(|header| header.p_type(endian) == object::elf::PT_GNU_RELRO),
            }
        }
        object::File::Pe64(pe) => {
            use object::read::pe::{ImageNtHeaders, ImageOptionalHeader};
            if clean
                && let Some(directory) = pe.data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_DEBUG)
            {
                ensure!(
                    directory.size.get(object::LittleEndian) == 0,
                    "PE debug directory remains after stripping"
                );
            }
            if let Some(directory) = pe.data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_SECURITY) {
                ensure!(
                    directory.size.get(object::LittleEndian) == 0,
                    "refusing to modify a signed executable; sign after packing"
                );
            }
            Hardening::Pe {
                dll_characteristics: pe.nt_headers().optional_header().dll_characteristics().0,
            }
        }
        _ => bail!("only ELF64 and PE32+ executables are supported"),
    };
    if clean {
        validate_clean(&file, &data, prohibited_paths)?;
    }
    let mut libraries = Vec::new();
    for library in file.import_libraries()? {
        libraries.push(std::str::from_utf8(library?.name())?.to_owned());
    }
    libraries.sort();
    let mut imports = Vec::new();
    for entry in file.imports()? {
        let entry = entry?;
        imports.push((
            std::str::from_utf8(entry.library())?.to_owned(),
            symbol_name(entry.name())?,
        ));
    }
    imports.sort();
    let mut exports = Vec::new();
    for entry in file.exports()? {
        exports.push(symbol_name(entry?.name())?);
    }
    exports.sort();
    let rust_string_hints = [
        "/rustc/",
        "library/core/src",
        "panicked at",
        "core::",
        "alloc::",
        "std::",
    ]
    .into_iter()
    .filter(|marker| contains_text(&data, marker))
    .map(str::to_owned)
    .collect();
    Ok(Inspection {
        libraries,
        imports,
        exports,
        elf: symbols::inspect(&file)?,
        hardening,
        rust_string_hints,
    })
}

fn validate_clean(file: &object::File<'_>, data: &[u8], prohibited_paths: &[&Path]) -> Result<()> {
    for section in file.sections() {
        let name = section.name()?;
        ensure!(
            !unwanted_section(name),
            "metadata section remains after stripping: {name}"
        );
    }
    ensure!(
        file.symbols().next().is_none(),
        "static symbols remain after stripping"
    );
    for path in prohibited_paths {
        let path = path.to_str().context("build paths must be UTF-8")?;
        ensure!(
            !contains_text(data, path),
            "build path remains in executable: {path}"
        );
    }
    Ok(())
}

pub fn unwanted_section(name: &str) -> bool {
    name.starts_with(".debug")
        || name.starts_with(".zdebug")
        || matches!(
            name,
            ".rustc"
                | ".dep-v0"
                | ".comment"
                | ".llvmbc"
                | ".llvmcmd"
                | ".gdb_index"
                | ".gnu_debuglink"
                | ".gnu_debugaltlink"
        )
}

fn contains_text(data: &[u8], text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let wide: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    data.windows(text.len())
        .any(|window| window == text.as_bytes())
        || data.windows(wide.len()).any(|window| window == wide)
}

#[derive(Debug, Serialize)]
pub struct PackedInspection {
    pub header_offset: usize,
    pub literal_hints: Vec<LiteralHint>,
}

#[derive(Debug, Serialize)]
pub struct LiteralHint {
    pub marker: String,
    pub offset: usize,
}

// RREA: coordinated packer/stub tag, not an application string.
const PACK_TAG: [u8; 4] = 0xb6e4_a291_u32.to_le_bytes();

pub fn validate_packed(path: &Path) -> Result<PackedInspection> {
    let data = fs::read(path)?;
    let file = object::File::parse(data.as_slice())?;
    let (range, format) = match &file {
        // Same bounded trailer search as pinned UPX's PackUnix::canUnpack.
        object::File::Elf64(_) => (data.len().saturating_sub(8265)..data.len(), 22),
        object::File::Pe64(_) => {
            let mut start = data.len();
            for section in file.sections() {
                ensure!(
                    !matches!(section.name()?, "UPX0" | "UPX1" | "UPX2"),
                    "original packer section name remains"
                );
                if let Some((offset, size)) = section.file_range()
                    && size != 0
                {
                    start = start.min(usize::try_from(offset)?);
                }
            }
            (
                start.saturating_sub(64)..start.saturating_add(1024).min(data.len()),
                36,
            )
        }
        _ => bail!("unsupported packed format"),
    };
    let headers: Vec<_> = data
        .get(range.clone())
        .context("invalid packed header range")?
        .windows(32)
        .enumerate()
        .filter(|(_, header)| valid_pack_header(header, format))
        .map(|(offset, _)| range.start + offset)
        .collect();
    ensure!(
        headers.len() == 1,
        "missing or ambiguous patched pack header"
    );
    let header_offset = headers[0];
    if format == 22 {
        let offset = data
            .get(header_offset + 32..header_offset + 36)
            .context("missing packed data offset")?;
        let offset = u32::from_le_bytes(offset.try_into()?) as usize;
        let loader = offset
            .checked_sub(12)
            .and_then(|start| data.get(start..offset))
            .context("invalid packed loader offset")?;
        ensure!(loader[4..8] == PACK_TAG, "unexpected packed loader tag");
        ensure!(
            loader[10..12] == [14, format],
            "unexpected packed loader version"
        );
    }
    // A literal anywhere in payload/overlay bytes is evidence, not proof of a header.
    let literal_hints = [
        "UPX!",
        "UPX0",
        "UPX1",
        "UPX2",
        "UPX ",
        "upx.sf.net",
        "RREA",
        "upX\0",
        "\0upx\0",
    ]
    .into_iter()
    .filter_map(|marker| {
        data.windows(marker.len())
            .position(|bytes| bytes == marker.as_bytes())
            .map(|offset| {
                LiteralHint {
                    marker: marker.to_owned(),
                    offset,
                }
            })
    })
    .collect();
    Ok(PackedInspection {
        header_offset,
        literal_hints,
    })
}

fn valid_pack_header(header: &[u8], format: u8) -> bool {
    let version = if format == 22 { 14 } else { 13 };
    header[..4] == PACK_TAG
        && header[4..6] == [version, format]
        && matches!(header[6], 2..=10 | 14)
        && (1..=10).contains(&header[7])
        && header[4..31]
            .iter()
            .map(|byte| u32::from(*byte))
            .sum::<u32>()
            % 251
            == u32::from(header[31])
}

pub fn same_runtime_metadata(before: &Inspection, after: &Inspection) -> Result<()> {
    ensure!(
        before.libraries == after.libraries,
        "executable dependencies changed"
    );
    ensure!(
        before.imports == after.imports,
        "executable imports changed"
    );
    ensure!(
        before.exports == after.exports,
        "executable exports changed"
    );
    ensure!(
        before.hardening == after.hardening,
        "executable hardening headers changed"
    );
    ensure!(
        before.elf == after.elf,
        "ELF dynamic symbol attributes or relocation targets changed"
    );
    Ok(())
}

#[cfg(test)]
#[path = "../tests/audit/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/evaluation/mod.rs"]
mod evaluation;
