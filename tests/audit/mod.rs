use std::{fs::OpenOptions, io::Write, process::Command};

use super::*;
use crate::support::checked_output;

#[test]
fn packed_payload_markers_are_not_format_headers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let clean = directory.path().join("clean");
    let packed = directory.path().join("packed");
    let recovered = directory.path().join("recovered");
    let tools = crate::tools::acquire(true)?;
    checked_output(
        Command::new(&tools.objcopy)
            .arg("--strip-all")
            .arg(std::env::current_exe()?)
            .arg(&clean),
    )?;
    // These are application/overlay data, not a packer-owned header or section.
    OpenOptions::new()
        .append(true)
        .open(&clean)?
        .write_all(b"RREA UPX! UPX0 UPX1 UPX2 UPX Team http://upx.sf.net\0upx\0upX\0")?;
    checked_output(
        Command::new(&tools.upx)
            .args(["--no-env", "--fast", "--overlay=copy", "-o"])
            .arg(&packed)
            .arg(&clean),
    )?;
    let original = fs::read(&packed)?;
    // RREA: explicit overlay literals; ELF packing can compress the source's appended bytes.
    OpenOptions::new()
        .append(true)
        .open(&packed)?
        .write_all(b"RREA UPX!")?;
    let inspection = validate_packed(&packed)?;
    ensure!(
        inspection
            .literal_hints
            .iter()
            .any(|hint| hint.marker == "RREA"),
        "payload hints were silently discarded"
    );
    fs::write(&packed, &original)?;
    checked_output(
        Command::new(&tools.upx)
            .args(["--no-env", "--decompress", "-o"])
            .arg(&recovered)
            .arg(&packed),
    )?;
    ensure!(
        fs::read(&clean)? == fs::read(&recovered)?,
        "payload changed"
    );
    for tag in [b"UPX!", b"RREA"] {
        let mut corrupt = original.clone();
        corrupt[inspection.header_offset..inspection.header_offset + 4].copy_from_slice(tag);
        fs::write(&packed, corrupt)?;
        ensure!(
            validate_packed(&packed).is_err(),
            "old header tag was accepted"
        );
    }
    let mut corrupt = original.clone();
    corrupt[inspection.header_offset + 31] ^= 1;
    fs::write(&packed, corrupt)?;
    ensure!(
        validate_packed(&packed).is_err(),
        "bad header checksum was accepted"
    );
    if cfg!(target_os = "linux") {
        let offset = inspection.header_offset + 32;
        let offset = u32::from_le_bytes(original[offset..offset + 4].try_into()?) as usize;
        let mut corrupt = original;
        corrupt[offset - 8..offset - 4].copy_from_slice(b"UPX!");
        fs::write(&packed, corrupt)?;
        ensure!(
            validate_packed(&packed).is_err(),
            "old loader tag was accepted"
        );
    }
    Ok(())
}
