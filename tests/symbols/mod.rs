use std::{path::Path, process::Command};

use super::*;
use crate::{audit, mangle, support::checked_output};

const STRONG: &str = "_ZN7fixture6strong17h1111111111111111E";
const STATE: &str = "_ZN7fixture5state17h2222222222222222E";
const LOOKUP: &str = "_ZN7fixture6lookup17h7777777777777777E";

#[test]
fn elf_bundle_renaming_preserves_loading_versions_and_exclusions() -> Result<()> {
    for hash in ["sysv", "gnu", "both"] {
        let root = tempfile::tempdir()?;
        let artifacts = fixture(root.path(), hash)?;
        let executable = root.path().join("consumer");
        let baseline = checked_output(Command::new(&executable).env("LD_BIND_NOW", "1"))?;
        let selected = plan(&artifacts, Some(&[LOOKUP.to_owned()]))?;
        let default = plan(&artifacts, None)?;
        ensure!(
            default.renamed.is_empty()
                && default.removed_metadata == selected.removed_metadata
                && default.removed_metadata == BTreeSet::from(["rust_metadata_fixture".to_owned()]),
            "compiler metadata policy depends on renaming"
        );
        ensure!(
            selected.renamed.len() == 2,
            "unexpected eligibility: {selected:?}"
        );
        for name in [STRONG, STATE] {
            ensure!(
                selected.renamed.contains_key(name),
                "eligible symbol missing: {name}"
            );
        }
        for (name, reason) in [
            (LOOKUP, Retention::Requested),
            (
                "_ZN7fixture4weak17h3333333333333333E",
                Retention::BindingOrType,
            ),
            (
                "_ZN7fixture3tls17h4444444444444444E",
                Retention::BindingOrType,
            ),
            (
                "_ZN7fixture8indirect17h5555555555555555E",
                Retention::BindingOrType,
            ),
            (
                "_ZN7fixture9versioned17h6666666666666666E",
                Retention::Versioned,
            ),
            (
                "_ZN7fixture8external17h8888888888888888E",
                Retention::External,
            ),
        ] {
            ensure!(
                serde_json::to_value(selected.retained.get(name))? == serde_json::to_value(reason)?,
                "wrong retained-symbol reason: {name}"
            );
        }
        ensure!(
            !selected.renamed.contains_key("_ZN3foo3barE"),
            "C++ name selected"
        );
        verify_rejected_and_relocated_plans(&artifacts, &selected)?;
        for path in artifacts.values() {
            let before = fs::read(path)?;
            let mut expected = audit::inspect(path, false, &[])?;
            mangle::rewrite(path, &BTreeMap::new(), &selected, &mut expected)?;
            let actual = audit::inspect(path, false, &[])?;
            audit::same_runtime_metadata(&expected, &actual)?;
            compare_loaded_sections(&before, &fs::read(path)?)?;
            let metadata = actual.elf.context("missing ELF audit")?;
            ensure!(
                metadata
                    .symbols
                    .iter()
                    .all(|symbol| !selected.renamed.contains_key(&symbol.reference.name)),
                "old selected symbol survived"
            );
        }
        for eager in [false, true] {
            let output = checked_output(
                Command::new(&executable).env("LD_BIND_NOW", if eager { "1" } else { "" }),
            )?;
            ensure!(
                output.stdout == baseline.stdout,
                "loader behavior changed for {hash}"
            );
        }
        verify_audit_rejects_changes(&executable)?;
    }
    Ok(())
}

fn fixture(root: &Path, hash: &str) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/symbols");
    let library = root.join("libfixture.so");
    let executable = root.join("consumer");
    checked_output(
        Command::new("cc")
            .args([
                "-shared",
                "-fPIC",
                "-Wl,-soname,libfixture.so",
                "-Wl,-z,relro,-z,now",
            ])
            .arg(format!("-Wl,--hash-style={hash}"))
            .arg(format!(
                "-Wl,--version-script={}",
                source.join("versions.map").display()
            ))
            .arg(source.join("library.c"))
            .arg(source.join("metadata.S"))
            .arg("-o")
            .arg(&library),
    )?;
    checked_output(
        Command::new("cc")
            .args(["-fPIC", "-pie", "-Wl,-rpath,$ORIGIN", "-Wl,-z,relro"])
            .arg(format!("-Wl,--hash-style={hash}"))
            .arg(source.join("main.c"))
            .arg("-L")
            .arg(root)
            .args(["-lfixture", "-ldl", "-o"])
            .arg(&executable),
    )?;
    Ok(BTreeMap::from([
        (PathBuf::from("consumer"), executable),
        (PathBuf::from("libfixture.so"), library),
    ]))
}

fn verify_rejected_and_relocated_plans(
    artifacts: &BTreeMap<PathBuf, PathBuf>,
    selected: &Plan,
) -> Result<()> {
    ensure!(
        plan(artifacts, Some(&["unknown".to_owned()])).is_err(),
        "unknown keep-symbol accepted"
    );
    let library = artifacts
        .get(Path::new("libfixture.so"))
        .context("missing library")?;
    let mut ambiguous = artifacts.clone();
    ambiguous.insert(PathBuf::from("duplicate.so"), library.clone());
    ensure!(
        matches!(
            plan(&ambiguous, Some(&[]))?.retained.get(STRONG),
            Some(Retention::AmbiguousDefinition)
        ),
        "ambiguous provider selected"
    );
    let mut external = artifacts.clone();
    external.remove(Path::new("libfixture.so"));
    ensure!(
        matches!(
            plan(&external, Some(&[]))?.retained.get(STRONG),
            Some(Retention::External)
        ),
        "external symbol selected"
    );
    let relocated = tempfile::tempdir()?;
    let mut copied = BTreeMap::new();
    for (name, path) in artifacts {
        let destination = relocated.path().join(name);
        fs::copy(path, &destination)?;
        copied.insert(name.clone(), destination);
    }
    ensure!(
        plan(&copied, Some(&[LOOKUP.to_owned()]))?.renamed == selected.renamed,
        "absolute paths changed the mapping"
    );
    let collision = copied
        .get(Path::new("libfixture.so"))
        .context("missing copied library")?;
    let mut expected = audit::inspect(collision, false, &[])?;
    let replacement = selected
        .renamed
        .get(STRONG)
        .context("missing selected symbol")?;
    mangle::rewrite(
        collision,
        &BTreeMap::new(),
        &Plan {
            renamed: BTreeMap::from([("plain_api".to_owned(), replacement.clone())]),
            ..Plan::default()
        },
        &mut expected,
    )?;
    let before = fs::read(collision)?;
    let error = plan(&copied, Some(&[]))
        .err()
        .context("symbol collision accepted")?;
    ensure!(
        error.to_string().contains("mangled symbol collision"),
        "wrong rejection: {error}"
    );
    ensure!(
        fs::read(collision)? == before,
        "failed planning mutated a file"
    );
    verify_metadata_references(&copied)?;
    Ok(())
}

fn verify_metadata_references(artifacts: &BTreeMap<PathBuf, PathBuf>) -> Result<()> {
    let path = artifacts
        .get(Path::new("consumer"))
        .context("missing consumer")?;
    let mut expected = audit::inspect(path, false, &[])?;
    mangle::rewrite(
        path,
        &BTreeMap::new(),
        &Plan {
            renamed: BTreeMap::from([("plain_api".to_owned(), "rust_metadata_fixture".to_owned())]),
            ..Plan::default()
        },
        &mut expected,
    )?;
    let before = fs::read(path)?;
    let error = plan(artifacts, None)
        .err()
        .context("referenced metadata was removed")?;
    ensure!(
        error.to_string().contains("cannot remove referenced"),
        "wrong rejection: {error}"
    );
    ensure!(
        fs::read(path)? == before,
        "failed metadata planning mutated a file"
    );
    Ok(())
}

fn compare_loaded_sections(before: &[u8], after: &[u8]) -> Result<()> {
    let before = object::File::parse(before)?;
    let after = object::File::parse(after)?;
    for name in [
        ".text",
        ".rodata",
        ".eh_frame",
        ".eh_frame_hdr",
        ".data",
        ".tdata",
    ] {
        if let Some(section) = before.section_by_name(name) {
            let actual = after
                .section_by_name(name)
                .context("loaded section disappeared")?;
            ensure!(
                section.address() == actual.address() && section.data()? == actual.data()?,
                "loaded section changed: {name}"
            );
        }
    }
    Ok(())
}

fn verify_audit_rejects_changes(path: &Path) -> Result<()> {
    let expected = audit::inspect(path, false, &[])?;
    let mut actual = audit::inspect(path, false, &[])?;
    let metadata = actual.elf.as_mut().context("missing ELF audit")?;
    let symbol = metadata.symbols.first_mut().context("missing symbol")?;
    symbol.size ^= 1;
    ensure!(
        audit::same_runtime_metadata(&expected, &actual).is_err(),
        "symbol size change accepted"
    );
    let mut actual = audit::inspect(path, false, &[])?;
    let relocation = actual
        .elf
        .as_mut()
        .context("missing ELF audit")?
        .relocations
        .first_mut()
        .context("missing relocation")?;
    relocation.symbol.version = Some("changed".to_owned());
    ensure!(
        audit::same_runtime_metadata(&expected, &actual).is_err(),
        "relocation version change accepted"
    );
    Ok(())
}
