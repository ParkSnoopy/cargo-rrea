use std::{fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use object::{Object, ObjectSection};
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Inspection {
    pub imports: Vec<(String, SymbolName)>,
    pub exports: Vec<SymbolName>,
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
        imports,
        exports,
        hardening,
        rust_string_hints,
    })
}

pub fn standalone(path: &Path) -> Result<()> {
    let data = fs::read(path)?;
    let file = object::File::parse(data.as_slice())?;
    if let object::File::Elf64(elf) = &file {
        use object::read::elf::{Dyn, ProgramHeader};
        let endian = elf.endian();
        ensure!(
            !elf.elf_program_headers()
                .iter()
                .any(|header| { header.p_type(endian) == object::elf::PT_INTERP }),
            "standalone executable still requires a dynamic loader; static native libraries are required"
        );
        if let Some((entries, _)) = elf.elf_section_table().dynamic(endian, data.as_slice())? {
            ensure!(
                !entries
                    .iter()
                    .any(|entry| entry.d_tag(endian) == object::elf::DT_NEEDED),
                "standalone executable still requires shared libraries"
            );
        }
    }
    Ok(())
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

pub fn same_runtime_metadata(before: &Inspection, after: &Inspection) -> Result<()> {
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
    Ok(())
}
