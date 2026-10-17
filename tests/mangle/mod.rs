use std::process::Command;

use object::{
    Object,
    ObjectSection,
    endian::U64,
    write::pe::{NtHeaders, Writer},
};

use super::*;

#[test]
fn names_follow_versions_not_paths_or_cargo_suffixes() -> Result<()> {
    let identity = "zlib-rs@0.6.8:zlib_rs";
    let digest = tools::digest(identity.as_bytes())?;
    for (original, expected) in [
        (
            "libzlib_rs-aed67000.so",
            format!("lib{}-aed67000.so", &digest[..16]),
        ),
        (
            "zlib_rs-aed67000.dll",
            format!("{}-aed67000.dll", &digest[..16]),
        ),
        (
            "libzlib_rs-1234.so",
            format!("lib{}-1234.so", &digest[..16]),
        ),
    ] {
        ensure!(
            name(original, identity)? == expected,
            "unstable library identity"
        );
    }
    ensure!(
        name("libzlib_rs-aed67000.so", identity)?
            != name("libzlib_rs-aed67000.so", "zlib-rs@0.6.9:zlib_rs")?,
        "version change did not change the mangled name"
    );
    for invalid in [
        "libzlib_rs.so",
        "libzlib_rs-.so",
        "libzlib_rs-nothex.so",
        "../libzlib_rs.dll",
    ] {
        ensure!(
            name(invalid, identity).is_err(),
            "invalid library filename was accepted"
        );
    }
    Ok(())
}

#[test]
fn pe_mangling_preserves_imports_exports_code_and_packing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("std-abcd.dll");
    let original = pe_fixture()?;
    fs::write(&path, &original)?;
    check_pe_packing(&path, &original)?;
    let mut expected = audit::inspect(&path, true, &[])?;
    let renamed = name("std-abcd.dll", "std@fixture-version")?;
    let names = BTreeMap::from([("std-abcd.dll".to_owned(), renamed.clone())]);
    rewrite(
        &path,
        &names,
        &crate::symbols::Plan::default(),
        &mut expected,
    )?;
    audit::same_runtime_metadata(&expected, &audit::inspect(&path, true, &[])?)?;
    let data = fs::read(&path)?;
    let before = PeFile64::parse(original.as_slice())?;
    let after = PeFile64::parse(data.as_slice())?;
    ensure!(
        before
            .section_by_name(".text")
            .context("missing original code")?
            .data()?
            == after
                .section_by_name(".text")
                .context("missing rewritten code")?
                .data()?,
        "PE code changed"
    );

    let table = after.import_table()?.context("missing rewritten imports")?;
    let mut descriptors = table.descriptors()?;
    for (index, expected_name) in [renamed.as_str(), "KERNEL32.dll"].into_iter().enumerate() {
        let descriptor = descriptors.next()?.context("missing import descriptor")?;
        ensure!(
            table.name(descriptor.name.get(LE))? == expected_name.as_bytes(),
            "wrong PE import name"
        );
        ensure!(
            descriptor.first_thunk.get(LE)
                == before
                    .section_by_name(".idata")
                    .context("missing imports")?
                    .address() as u32
                    - before.relative_address_base() as u32
                    + 224
                    + index as u32 * 32,
            "PE IAT address changed"
        );
        let iat = after
            .section_table()
            .pe_data_at(data.as_slice(), descriptor.first_thunk.get(LE))
            .and_then(|bytes| bytes.get(..24))
            .context("missing IAT contents")?;
        let lookup = after
            .section_table()
            .pe_data_at(data.as_slice(), descriptor.original_first_thunk.get(LE))
            .and_then(|bytes| bytes.get(..24))
            .context("missing lookup contents")?;
        ensure!(iat == lookup, "PE IAT and lookup entries disagree");
    }
    let export = after.export_table()?.context("missing PE exports")?;
    let export_name = after
        .section_table()
        .pe_data_at(data.as_slice(), export.directory().name.get(LE))
        .context("missing DLL name")?;
    ensure!(
        export_name.starts_with(format!("{renamed}\0").as_bytes()),
        "PE export DLL name was not rewritten"
    );
    check_pe_packing(&path, &data)?;
    let descriptor = table.descriptors()?.next()?.context("missing PE import")?;
    let (iat, _) = after
        .section_table()
        .pe_file_range_at(descriptor.first_thunk.get(LE))
        .context("missing PE IAT range")?;
    // Bound-style IAT bytes must survive independently of the lookup table.
    let mut bound = data.clone();
    put(&mut bound, iat as usize, &U64::new(LE, after.entry()))?;
    fs::write(&path, &bound)?;
    check_pe_packing(&path, &bound)
}

fn check_pe_packing(path: &Path, data: &[u8]) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let packed = directory.path().join("packed.dll");
    let recovered = directory.path().join("recovered.dll");
    let result = Command::new(env!("RREA_UPX_PATH"))
        .args([
            "--no-env",
            "--best",
            "--strip-relocs=0",
            "--compress-exports=0",
            "--compress-resources=0",
            "-o",
        ])
        .arg(&packed)
        .arg(path)
        .output()?;
    ensure!(
        result.status.success(),
        "PE packing failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    audit::validate_packed(&packed)?;
    let result = Command::new(env!("RREA_UPX_PATH"))
        .args(["--no-env", "--decompress", "-o"])
        .arg(&recovered)
        .arg(&packed)
        .output()?;
    ensure!(
        result.status.success(),
        "PE recovery failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let recovered = fs::read(recovered)?;
    let changed = data
        .iter()
        .zip(&recovered)
        .position(|(before, after)| before != after);
    ensure!(
        recovered == data,
        "PE packing changed source bytes: first difference {changed:?}; sizes {} and {}; before {:?}; after {:?}",
        data.len(),
        recovered.len(),
        changed.and_then(|index| data.get(index..index + 16)),
        changed.and_then(|index| recovered.get(index..index + 16))
    );
    Ok(())
}

fn pe_fixture() -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut writer = Writer::new(true, 4096, 512, &mut output);
    writer.reserve_dos_header();
    writer.reserve_nt_headers(16);
    writer.reserve_section_headers(2);
    let text = writer.reserve_text_section(65_536);
    let imports = writer.reserve_idata_section(512);
    writer.set_data_directory(
        pe::IMAGE_DIRECTORY_ENTRY_EXPORT,
        imports.virtual_address + 300,
        160,
    );
    let mut payload = vec![0_u8; 512];
    for (index, (filename, string_offset)) in [
        (b"std-abcd.dll".as_slice(), 64_u32),
        (b"KERNEL32.dll".as_slice(), 96),
    ]
    .into_iter()
    .enumerate()
    {
        let lookup = 160 + index as u32 * 32;
        let iat = 224 + index as u32 * 32;
        put(
            &mut payload,
            index * mem::size_of::<pe::ImageImportDescriptor>(),
            &pe::ImageImportDescriptor {
                original_first_thunk: U32::new(LE, imports.virtual_address + lookup),
                time_date_stamp: U32::new(LE, 0),
                forwarder_chain: U32::new(LE, 0),
                name: U32::new(LE, imports.virtual_address + string_offset),
                first_thunk: U32::new(LE, imports.virtual_address + iat),
            },
        )?;
        payload[string_offset as usize..string_offset as usize + filename.len()]
            .copy_from_slice(filename);
        for start in [lookup, iat] {
            put(
                &mut payload,
                start as usize,
                &U64::new(LE, u64::from(imports.virtual_address + 128)),
            )?;
            put(
                &mut payload,
                start as usize + 8,
                &U64::new(LE, pe::IMAGE_ORDINAL_FLAG64 | 7),
            )?;
        }
    }
    payload[130..139].copy_from_slice(b"function\0");
    let exports = pe::ImageExportDirectory {
        characteristics: U32::new(LE, 0),
        time_date_stamp: U32::new(LE, 0),
        major_version: 0.into(),
        minor_version: 0.into(),
        name: U32::new(LE, imports.virtual_address + 64),
        base: U32::new(LE, 1),
        number_of_functions: U32::new(LE, 1),
        number_of_names: U32::new(LE, 1),
        address_of_functions: U32::new(LE, imports.virtual_address + 400),
        address_of_names: U32::new(LE, imports.virtual_address + 404),
        address_of_name_ordinals: U32::new(LE, imports.virtual_address + 408),
    };
    put(&mut payload, 300, &exports)?;
    put(&mut payload, 400, &U32::new(LE, text.virtual_address))?;
    put(
        &mut payload,
        404,
        &U32::new(LE, imports.virtual_address + 420),
    )?;
    payload[420..429].copy_from_slice(b"exported\0");
    writer.write_empty_dos_header()?;
    writer.write_nt_headers(NtHeaders {
        machine: pe::IMAGE_FILE_MACHINE_AMD64,
        time_date_stamp: 0,
        characteristics: pe::IMAGE_FILE_EXECUTABLE_IMAGE
            | pe::IMAGE_FILE_LARGE_ADDRESS_AWARE
            | pe::IMAGE_FILE_DLL,
        major_linker_version: 14,
        minor_linker_version: 0,
        address_of_entry_point: text.virtual_address,
        image_base: 0x0001_8000_0000,
        major_operating_system_version: 6,
        minor_operating_system_version: 0,
        major_image_version: 0,
        minor_image_version: 0,
        major_subsystem_version: 6,
        minor_subsystem_version: 0,
        subsystem: pe::IMAGE_SUBSYSTEM_WINDOWS_CUI,
        dll_characteristics: pe::IMAGE_DLLCHARACTERISTICS_NX_COMPAT,
        size_of_stack_reserve: 1_048_576,
        size_of_stack_commit: 4096,
        size_of_heap_reserve: 1_048_576,
        size_of_heap_commit: 4096,
    });
    writer.write_section_headers();
    let mut code = vec![0_u8; text.virtual_size as usize];
    code[..6].copy_from_slice(&[0xb8, 1, 0, 0, 0, 0xc3]);
    writer.write_section(text.file_offset, &code);
    writer.write_section(imports.file_offset, &payload);
    Ok(output)
}
