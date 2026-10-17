use std::{collections::BTreeMap, fs, mem, path::Path};

use anyhow::{Context, Result, ensure};
use object::{
    LittleEndian as LE,
    build::elf::{Builder, Dynamic, SectionData},
    endian::U32,
    pe,
    pod::{self, Pod},
    read::pe::{ImageNtHeaders, ImageOptionalHeader, ImageThunkData, PeFile64},
};

use crate::{audit, tools};

pub fn name(original: &str, identity: &str) -> Result<String> {
    let (stem, prefix, extension) = if let Some(stem) = original.strip_suffix(".so") {
        (
            stem.strip_prefix("lib")
                .context("invalid ELF library name")?,
            "lib",
            "so",
        )
    } else {
        (
            original
                .strip_suffix(".dll")
                .context("invalid PE library name")?,
            "",
            "dll",
        )
    };
    let (_, suffix) = stem
        .rsplit_once('-')
        .context("library name has no Cargo suffix")?;
    ensure!(
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid Cargo library suffix"
    );
    let digest = tools::digest(identity.as_bytes())?;
    Ok(format!("{prefix}{}-{suffix}.{extension}", &digest[..16]))
}

pub fn rewrite(
    path: &Path,
    names: &BTreeMap<String, String>,
    symbols: &crate::symbols::Plan,
    expected: &mut audit::Inspection,
) -> Result<()> {
    if names.is_empty() && symbols.is_empty() {
        return Ok(());
    }
    let data = fs::read(path)?;
    let output = match object::File::parse(data.as_slice())? {
        object::File::Elf64(_) => rewrite_elf(&data, names, symbols)?,
        object::File::Pe64(pe) => {
            ensure!(
                symbols.is_empty(),
                "symbol mangling requires ELF64 artifacts"
            );
            rewrite_pe(&data, &pe, names)?
        }
        _ => anyhow::bail!("unsupported library mangling format"),
    };
    fs::write(path, output)?;
    for library in &mut expected.libraries {
        if let Some(name) = names.get(library) {
            library.clone_from(name);
        }
    }
    for (library, _) in &mut expected.imports {
        if let Some(name) = names.get(library) {
            library.clone_from(name);
        }
    }
    for symbol in expected
        .imports
        .iter_mut()
        .map(|(_, name)| name)
        .chain(&mut expected.exports)
    {
        if let audit::SymbolName::Name(name) = symbol
            && let Some(replacement) = symbols.renamed.get(name)
        {
            name.clone_from(replacement);
        }
    }
    expected.exports.retain(|symbol| {
        !matches!(symbol,
        audit::SymbolName::Name(name) if symbols.removed_metadata.contains(name))
    });
    if let Some(metadata) = &mut expected.elf {
        metadata
            .symbols
            .retain(|symbol| !symbols.removed_metadata.contains(&symbol.reference.name));
        metadata.rename(names, &symbols.renamed);
    }
    expected.libraries.sort();
    expected.imports.sort();
    expected.exports.sort();
    Ok(())
}

fn rewrite_elf(
    data: &[u8],
    names: &BTreeMap<String, String>,
    symbols: &crate::symbols::Plan,
) -> Result<Vec<u8>> {
    let mut builder = Builder::read(data)?;
    for symbol in &mut builder.dynamic_symbols {
        let original = std::str::from_utf8(symbol.name.as_slice())?;
        symbol.delete = symbols.removed_metadata.contains(original);
        if let Some(name) = symbols.renamed.get(original) {
            symbol.name = name.as_bytes().to_vec().into();
        }
    }
    for entry in builder
        .dynamic_data_mut()
        .context("missing ELF dynamic table")?
    {
        if let Dynamic::String { tag, val } = entry
            && matches!(*tag, object::elf::DT_RPATH | object::elf::DT_RUNPATH)
        {
            let paths = std::str::from_utf8(val.as_slice())?
                .split(':')
                .map(|path| {
                    if path.trim_end_matches("/.") == "$ORIGIN" {
                        "$ORIGIN"
                    } else {
                        path
                    }
                })
                .collect::<Vec<_>>()
                .join(":");
            *val = paths.into_bytes().into();
        }
        if let Dynamic::String { tag, val } = entry
            && matches!(*tag, object::elf::DT_NEEDED | object::elf::DT_SONAME)
            && let Some(name) = names.get(std::str::from_utf8(val.as_slice())?)
        {
            *val = name.as_bytes().to_vec().into();
        }
    }
    for file in &mut builder.version_files {
        if let Some(name) = names.get(std::str::from_utf8(file.name.as_slice())?) {
            file.name = name.as_bytes().to_vec().into();
        }
    }
    if let Some(base) = &mut builder.version_base
        && let Some(name) = names.get(std::str::from_utf8(base.as_slice())?)
    {
        *base = name.as_bytes().to_vec().into();
    }
    layout_elf_metadata(&mut builder)?;
    let mut output = Vec::new();
    builder.write(&mut output)?;
    Ok(output)
}

fn layout_elf_metadata(builder: &mut Builder<'_>) -> Result<()> {
    let strings = builder
        .sections
        .iter()
        .find(|section| matches!(section.data, SectionData::DynamicString))
        .context("missing ELF dynamic strings")?
        .id();
    let start = builder.sections.get(strings).sh_addr;
    let load = builder
        .segments
        .iter()
        .find(|segment| segment.is_load() && segment.sections.contains(&strings))
        .context("ELF dynamic strings have no load segment")?
        .id();
    let mut tail = builder.segments.get(load).sections.clone();
    tail.sort_by_key(|id| builder.sections.get(*id).sh_addr);
    let moving: Vec<_> = tail
        .into_iter()
        .skip_while(|id| *id != strings)
        .take_while(|id| {
            matches!(
                builder.sections.get(*id).data,
                SectionData::DynamicString
                    | SectionData::DynamicSymbol
                    | SectionData::DynamicRelocation(_)
                    | SectionData::Hash
                    | SectionData::GnuHash
                    | SectionData::GnuVersym
                    | SectionData::GnuVerdef
                    | SectionData::GnuVerneed
            )
        })
        .collect();
    let boundary = builder
        .sections
        .iter()
        .filter(|section| {
            section.is_alloc() && section.sh_addr >= start && !moving.contains(&section.id())
        })
        .map(|section| section.sh_addr)
        .chain(
            builder
                .segments
                .iter()
                .filter(|segment| segment.is_load() && segment.p_vaddr > start)
                .map(|segment| segment.p_vaddr),
        )
        .min()
        .context("missing ELF metadata boundary")?;
    builder.set_section_sizes();
    let segment = builder.segments.get(load);
    ensure!(
        segment.p_filesz == segment.p_memsz,
        "ELF metadata segment contains uninitialized data"
    );
    let (base_address, base_offset) = (segment.p_vaddr, segment.p_offset);
    let mut address = start;
    let mut relocated = BTreeMap::new();
    // UPX needs loader metadata below its stub; never move code, data, or unwind tables.
    for id in moving {
        let section = builder.sections.get_mut(id);
        address = address
            .checked_next_multiple_of(section.sh_addralign.max(1))
            .context("ELF metadata alignment overflow")?;
        relocated.insert(section.sh_addr, address);
        section.sh_addr = address;
        section.sh_offset = base_offset
            .checked_add(address - base_address)
            .context("ELF metadata offset overflow")?;
        address = address
            .checked_add(section.sh_size)
            .context("ELF metadata size overflow")?;
    }
    ensure!(
        address <= boundary,
        "insufficient space for mangled ELF metadata before code"
    );
    let segment = builder.segments.get_mut(load);
    segment.p_memsz = segment.p_memsz.max(address - base_address);
    segment.p_filesz = segment.p_memsz;
    for entry in builder
        .dynamic_data_mut()
        .context("missing ELF dynamic table")?
    {
        if let Dynamic::Integer { tag, val } = entry
            && matches!(
                *tag,
                object::elf::DT_REL | object::elf::DT_RELA | object::elf::DT_JMPREL
            )
            && let Some(address) = relocated.get(val)
        {
            *val = *address;
        }
    }
    Ok(())
}

fn rewrite_pe(
    data: &[u8],
    file: &PeFile64<'_>,
    names: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let (header_offset, address) = pe_layout(data, file)?;
    let mut payload = Vec::new();
    let imports = rebuild_pe_imports(file, names, address, &mut payload)?;
    let exports = rebuild_pe_exports(data, file, names, address, &mut payload)?;
    if let Some(table) = file.delay_load_import_table()? {
        let mut descriptors = table.descriptors()?;
        while let Some(descriptor) = descriptors.next()? {
            let name = std::str::from_utf8(table.name(descriptor.dll_name_rva.get(LE))?)?;
            ensure!(
                !names.contains_key(name),
                "cannot mangle a delay-loaded bundled DLL"
            );
        }
    }
    if payload.is_empty() {
        return Ok(data.to_vec());
    }
    let mut output =
        append_pe_section(data, file, header_offset, address, &payload, data.to_vec())?;
    for (offset, thunks) in imports {
        let end = offset
            .checked_add(thunks.len())
            .context("PE IAT range overflow")?;
        output
            .get_mut(offset..end)
            .context("PE IAT is outside the image")?
            .copy_from_slice(&thunks);
    }
    if let Some(exports) = exports {
        let directory = file
            .data_directory(pe::IMAGE_DIRECTORY_ENTRY_EXPORT)
            .context("missing PE export directory")?;
        put(&mut output, offset(data, directory)?, &exports)?;
    }
    Ok(output)
}

fn rebuild_pe_exports(
    data: &[u8],
    file: &PeFile64<'_>,
    names: &BTreeMap<String, String>,
    base: u32,
    payload: &mut Vec<u8>,
) -> Result<Option<pe::ImageDataDirectory>> {
    let Some(table) = file.export_table()? else {
        return Ok(None);
    };
    let source = file
        .data_directory(pe::IMAGE_DIRECTORY_ENTRY_EXPORT)
        .context("missing export directory")?;
    let address = source.virtual_address.get(LE);
    let size = source.size.get(LE);
    let (file_offset, _) = file
        .section_table()
        .pe_file_range_at(address)
        .context("invalid PE export range")?;
    let bytes = file
        .section_table()
        .pe_data_at(data, address)
        .context("missing PE export bytes")?
        .get(..size as usize)
        .context("truncated PE export bytes")?;
    payload.resize(payload.len().next_multiple_of(4), 0);
    let start = payload.len();
    let destination = base
        .checked_add(u32::try_from(start)?)
        .context("PE export address overflow")?;
    let relocate = |pointer: u32| -> Result<u32> {
        let relative = pointer
            .checked_sub(address)
            .context("PE export pointer precedes the directory")?;
        ensure!(
            relative < size,
            "PE export pointer is outside the directory"
        );
        destination
            .checked_add(relative)
            .context("PE export pointer overflow")
    };
    payload.extend_from_slice(bytes);
    let mut directory = *table.directory();
    for pointer in [
        &mut directory.address_of_functions,
        &mut directory.address_of_names,
        &mut directory.address_of_name_ordinals,
    ] {
        if pointer.get(LE) != 0 {
            pointer.set(LE, relocate(pointer.get(LE))?);
        }
    }
    for pointer in table.name_pointers() {
        put(
            payload,
            start + offset(data, pointer)? - file_offset as usize,
            &U32::new(LE, relocate(pointer.get(LE))?),
        )?;
    }
    for pointer in table.addresses() {
        if pointer
            .get(LE)
            .checked_sub(address)
            .is_some_and(|relative| relative < size)
        {
            put(
                payload,
                start + offset(data, pointer)? - file_offset as usize,
                &U32::new(LE, relocate(pointer.get(LE))?),
            )?;
        }
    }
    let name = file
        .section_table()
        .pe_data_at(data, directory.name.get(LE))
        .context("invalid PE export DLL name")?;
    let name = std::str::from_utf8(
        name.split(|byte| *byte == 0)
            .next()
            .context("missing export DLL name")?,
    )?;
    let name = names.get(name).map_or(name, String::as_str);
    directory
        .name
        .set(LE, append_string(payload, base, name.as_bytes())?);
    put(payload, start, &directory)?;
    Ok(Some(pe::ImageDataDirectory {
        virtual_address: U32::new(LE, destination),
        size: U32::new(LE, u32::try_from(payload.len() - start)?),
    }))
}

fn pe_layout(data: &[u8], file: &PeFile64<'_>) -> Result<(usize, u32)> {
    let optional = file.nt_headers().optional_header();
    let sections = file.section_table();
    let header_offset = sections
        .iter()
        .map(|section| offset(data, section))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .context("missing PE sections")?
        + mem::size_of::<pe::ImageSectionHeader>();
    let header_end = header_offset + mem::size_of::<pe::ImageSectionHeader>();
    ensure!(
        header_end <= usize::try_from(optional.size_of_headers())?
            && sections
                .iter()
                .filter(|section| section.size_of_raw_data.get(LE) != 0)
                .all(|section| header_end <= section.pointer_to_raw_data.get(LE) as usize),
        "PE headers have no room for mangled import strings"
    );
    let alignment = optional.section_alignment();
    ensure!(
        alignment.is_power_of_two() && optional.file_alignment().is_power_of_two(),
        "invalid PE alignment"
    );
    let address = sections
        .iter()
        .map(|section| {
            section
                .virtual_address
                .get(LE)
                .checked_add(
                    section
                        .virtual_size
                        .get(LE)
                        .max(section.size_of_raw_data.get(LE)),
                )
                .context("PE section address overflow")
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .context("missing PE sections")?
        .checked_next_multiple_of(alignment)
        .context("PE section alignment overflow")?;
    Ok((header_offset, address))
}

fn append_pe_section(
    data: &[u8],
    file: &PeFile64<'_>,
    header_offset: usize,
    address: u32,
    payload: &[u8],
    mut output: Vec<u8>,
) -> Result<Vec<u8>> {
    let optional = file.nt_headers().optional_header();
    let alignment = optional.section_alignment();
    let raw_offset = output
        .len()
        .checked_next_multiple_of(optional.file_alignment() as usize)
        .context("PE file alignment overflow")?;
    output.resize(raw_offset, 0);
    output.extend_from_slice(payload);
    let raw_size = payload
        .len()
        .checked_next_multiple_of(optional.file_alignment() as usize)
        .context("PE section size overflow")?;
    output.resize(
        raw_offset
            .checked_add(raw_size)
            .context("PE file size overflow")?,
        0,
    );
    let section = pe::ImageSectionHeader {
        name: *b".rrea\0\0\0",
        virtual_size: U32::new(LE, u32::try_from(payload.len())?),
        virtual_address: U32::new(LE, address),
        size_of_raw_data: U32::new(LE, u32::try_from(raw_size)?),
        pointer_to_raw_data: U32::new(LE, u32::try_from(raw_offset)?),
        characteristics: U32::new(
            LE,
            pe::IMAGE_SCN_CNT_INITIALIZED_DATA | pe::IMAGE_SCN_MEM_READ,
        ),
        ..pe::ImageSectionHeader::default()
    };
    put(&mut output, header_offset, &section)?;
    let count = file
        .nt_headers()
        .file_header()
        .number_of_sections
        .get(LE)
        .checked_add(1)
        .context("too many PE sections")?;
    put(
        &mut output,
        offset(data, &file.nt_headers().file_header().number_of_sections)?,
        &object::endian::U16::new(LE, count),
    )?;
    let image_size = address
        .checked_add(u32::try_from(payload.len())?)
        .and_then(|size| size.checked_next_multiple_of(alignment))
        .context("PE image size overflow")?;
    put(
        &mut output,
        offset(data, &optional.size_of_image)?,
        &U32::new(LE, image_size),
    )?;
    let initialized = optional
        .size_of_initialized_data
        .get(LE)
        .checked_add(u32::try_from(raw_size)?)
        .context("PE data size overflow")?;
    put(
        &mut output,
        offset(data, &optional.size_of_initialized_data)?,
        &U32::new(LE, initialized),
    )?;
    put(
        &mut output,
        offset(data, &optional.check_sum)?,
        &U32::new(LE, 0_u32),
    )?;
    if let Some(directory) = file.data_directory(pe::IMAGE_DIRECTORY_ENTRY_IMPORT)
        && directory.virtual_address.get(LE) != 0
    {
        put(
            &mut output,
            offset(data, &directory.virtual_address)?,
            &U32::new(LE, address),
        )?;
    }
    if let Some(directory) = file.data_directory(pe::IMAGE_DIRECTORY_ENTRY_BOUND_IMPORT) {
        put(
            &mut output,
            offset(data, directory)?,
            &pe::ImageDataDirectory {
                virtual_address: U32::new(LE, 0),
                size: U32::new(LE, 0),
            },
        )?;
    }
    Ok(output)
}

fn rebuild_pe_imports(
    file: &PeFile64<'_>,
    names: &BTreeMap<String, String>,
    address: u32,
    payload: &mut Vec<u8>,
) -> Result<Vec<(usize, Vec<u8>)>> {
    let Some(table) = file.import_table()? else {
        return Ok(Vec::new());
    };
    let mut iterator = table.descriptors()?;
    let mut descriptors = Vec::new();
    while let Some(descriptor) = iterator.next()? {
        descriptors.push(*descriptor);
    }
    let size = mem::size_of::<pe::ImageImportDescriptor>();
    payload.resize((descriptors.len() + 1) * size, 0);
    let mut patches = Vec::new();
    for (index, mut descriptor) in descriptors.into_iter().enumerate() {
        let original = std::str::from_utf8(table.name(descriptor.name.get(LE))?)?;
        let name = names.get(original).map_or(original, String::as_str);
        descriptor
            .name
            .set(LE, append_string(payload, address, name.as_bytes())?);
        let source = if descriptor.original_first_thunk.get(LE) != 0 {
            descriptor.original_first_thunk.get(LE)
        } else {
            descriptor.first_thunk.get(LE)
        };
        let mut thunks = table.thunks(source)?;
        let mut relocated = Vec::new();
        while let Some(thunk) = thunks.next::<pe::ImageNtHeaders64>()? {
            let value = if thunk.is_ordinal() {
                thunk.raw()
            } else {
                let (hint, name) = table.hint_name(thunk.address())?;
                payload.resize(payload.len().next_multiple_of(2), 0);
                let pointer = address
                    .checked_add(u32::try_from(payload.len())?)
                    .context("PE import address overflow")?;
                payload.extend_from_slice(&hint.to_le_bytes());
                append_string(payload, address, name)?;
                u64::from(pointer)
            };
            relocated.extend_from_slice(&value.to_le_bytes());
        }
        relocated.extend_from_slice(&0_u64.to_le_bytes());
        payload.resize(payload.len().next_multiple_of(8), 0);
        descriptor.original_first_thunk.set(
            LE,
            address
                .checked_add(u32::try_from(payload.len())?)
                .context("PE thunk address overflow")?,
        );
        payload.extend_from_slice(&relocated);
        let (offset, available) = file
            .section_table()
            .pe_file_range_at(descriptor.first_thunk.get(LE))
            .context("invalid PE IAT address")?;
        ensure!(relocated.len() <= available as usize, "PE IAT is truncated");
        patches.push((offset as usize, relocated));
        descriptor.time_date_stamp.set(LE, 0);
        put(payload, index * size, &descriptor)?;
    }
    Ok(patches)
}

fn append_string(output: &mut Vec<u8>, base: u32, value: &[u8]) -> Result<u32> {
    let address = base
        .checked_add(u32::try_from(output.len())?)
        .context("PE string address overflow")?;
    output.extend_from_slice(value);
    output.push(0);
    Ok(address)
}

fn offset<T: Pod>(data: &[u8], value: &T) -> Result<usize> {
    let start = std::ptr::from_ref(value)
        .addr()
        .checked_sub(data.as_ptr().addr())
        .context("invalid PE field offset")?;
    ensure!(
        start
            .checked_add(mem::size_of::<T>())
            .is_some_and(|end| end <= data.len()),
        "PE field is outside the image"
    );
    Ok(start)
}

fn put<T: Pod>(data: &mut [u8], offset: usize, value: &T) -> Result<()> {
    let bytes = pod::bytes_of(value);
    let end = offset
        .checked_add(bytes.len())
        .context("PE field offset overflow")?;
    data.get_mut(offset..end)
        .context("PE field is outside the image")?
        .copy_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
#[path = "../tests/mangle/mod.rs"]
mod tests;
