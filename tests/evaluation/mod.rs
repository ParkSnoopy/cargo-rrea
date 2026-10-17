use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use object::{Object, ObjectSection};
use serde_json::{Value, json};

use super::{inspect, validate_packed};
use crate::{support::checked_output, tools};

const VARIANTS: [&str; 6] = [
    "baseline",
    "stripped",
    "packed",
    "recovered",
    "unmangled",
    "mangled",
];

fn binary(root: &Path, variant: &str) -> PathBuf {
    root.join("variants").join(variant).join("b116")
}

fn transform(binary: &Path, operation: &str, input: &Path, wrapped: bool) -> Command {
    let mut command = Command::new(binary);
    command.args([operation, "--in"]).arg(input);
    if wrapped {
        command.arg("--with-wrapper");
    }
    command
}

fn same_output(expected: &Output, actual: &Output, label: &str) -> Result<()> {
    ensure!(
        actual.status.code() == expected.status.code()
            && actual.stdout == expected.stdout
            && actual.stderr == expected.stderr,
        "behavior differs for {label}: status {:?}/{:?}, stdout lengths {}/{}, stderr {:?}/{:?}",
        expected.status.code(),
        actual.status.code(),
        expected.stdout.len(),
        actual.stdout.len(),
        String::from_utf8_lossy(&expected.stderr),
        String::from_utf8_lossy(&actual.stderr)
    );
    Ok(())
}

fn binary_cases(root: &Path, work: &Path) -> Result<usize> {
    let input = work.join("input");
    let encoded = work.join("encoded");
    let baseline = binary(root, "baseline");
    let mut cases = 0;
    for len in (0..=16).chain([31, 32, 33, 255, 256, 257, 4095, 4096, 4097]) {
        let bytes: Vec<u8> = (0..len)
            .map(|index| (index * 73 + index / 256) as u8)
            .collect();
        fs::write(&input, &bytes)?;
        for wrapped in [false, true] {
            let expected = checked_output(&mut transform(&baseline, "encode", &input, wrapped))?;
            let text = std::str::from_utf8(&expected.stdout)?;
            ensure!(
                !wrapped || (text.starts_with('Ǳ') && text.ends_with('ǲ')),
                "missing wrapper"
            );
            fs::write(&encoded, &expected.stdout)?;
            let decoded = checked_output(&mut transform(&baseline, "decode", &encoded, wrapped))?;
            ensure!(
                decoded.stdout == bytes,
                "baseline did not round trip length {len}"
            );
            for variant in VARIANTS {
                let path = binary(root, variant);
                same_output(
                    &expected,
                    &transform(&path, "encode", &input, wrapped).output()?,
                    variant,
                )?;
                same_output(
                    &decoded,
                    &transform(&path, "decode", &encoded, wrapped).output()?,
                    variant,
                )?;
            }
            cases += 1;
        }
    }
    Ok(cases)
}

fn malformed_cases(root: &Path, work: &Path) -> Result<usize> {
    let input = work.join("malformed");
    let cases: &[&[u8]] = &[
        b"\xff",
        b"\xc0\x80",
        b"\xe2\x82",
        b"A",
        b"ABC",
        b" \t\r\n",
        "Ǳ".as_bytes(),
        "ǲ".as_bytes(),
        "☃".as_bytes(),
        "ǱABCǲ".as_bytes(),
    ];
    for bytes in cases {
        fs::write(&input, bytes)?;
        for wrapped in [false, true] {
            let expected =
                transform(&binary(root, "baseline"), "decode", &input, wrapped).output()?;
            for variant in VARIANTS {
                same_output(
                    &expected,
                    &transform(&binary(root, variant), "decode", &input, wrapped).output()?,
                    variant,
                )?;
            }
        }
    }
    Ok(cases.len() * 2)
}

fn inventory(root: &Path, variant: &str) -> Result<Vec<Value>> {
    let mut paths = fs::read_dir(root.join("variants").join(variant))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let mut artifacts = Vec::new();
    for path in paths {
        let data = fs::read(&path)?;
        let object = object::File::parse(data.as_slice())?;
        let sections = object.sections().map(|section| {
            Ok(json!({"name": section.name()?, "file_range": section.file_range(), "size": section.size()}))
        }).collect::<Result<Vec<_>>>()?;
        let packed = matches!(variant, "packed" | "mangled" | "unmangled");
        artifacts.push(json!({
            "name": path.file_name().and_then(|name| name.to_str()).context("artifact name is not UTF-8")?,
            "bytes": data.len(), "sha256": tools::digest(data.as_slice())?,
            "inspection": inspect(&path, variant != "baseline", &[])?,
            "packed_inspection": if packed { Some(validate_packed(&path)?) } else { None },
            "sections": sections,
        }));
    }
    Ok(artifacts)
}

fn measured(command: &mut Command) -> Result<(f64, Option<u64>)> {
    let start = Instant::now();
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut sampled_peak = None;
    while child.try_wait()?.is_none() {
        if start.elapsed() > Duration::from_secs(60) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("evaluation command timed out: {command:?}");
        }
        if let Ok(status) = fs::read_to_string(format!("/proc/{}/status", child.id()))
            && let Some(line) = status.lines().find(|line| line.starts_with("VmHWM:"))
        {
            let rss = line
                .split_whitespace()
                .nth(1)
                .context("missing RSS")?
                .parse::<u64>()?;
            sampled_peak = Some(sampled_peak.unwrap_or(0).max(rss));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "measurement failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok((start.elapsed().as_secs_f64(), sampled_peak))
}

fn measurements(root: &Path, work: &Path) -> Result<BTreeMap<String, Vec<Value>>> {
    let input = work.join("timed-input");
    let encoded = work.join("timed-encoded");
    let output = work.join("timed-output");
    let mut state = 0x4d59_5df4_d0f3_3173_u64;
    let data: Vec<_> = (0..1_048_576)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    fs::write(&input, &data)?;
    let expected = checked_output(&mut transform(
        &binary(root, "baseline"),
        "encode",
        &input,
        false,
    ))?
    .stdout;
    fs::write(&encoded, &expected)?;
    let mut results: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    // Round-robin warmed runs. These are wall-time observations, not CPU-only timings.
    for round in 0..10 {
        for variant in VARIANTS {
            let path = binary(root, variant);
            let (startup, _) = measured(Command::new(&path).arg("--version"))?;
            let (encode, encode_rss) = measured(
                transform(&path, "encode", &input, false)
                    .arg("--out")
                    .arg(&output),
            )?;
            ensure!(
                fs::read(&output)? == expected,
                "timed encoding changed output"
            );
            let (decode, decode_rss) = measured(
                transform(&path, "decode", &encoded, false)
                    .arg("--out")
                    .arg(&output),
            )?;
            ensure!(fs::read(&output)? == data, "timed decoding changed output");
            if round != 0 {
                results.entry(variant.to_owned()).or_default().push(json!({
                    "startup_seconds": startup, "encode_seconds": encode, "decode_seconds": decode,
                    "encode_sampled_peak_rss_kib": encode_rss, "decode_sampled_peak_rss_kib": decode_rss,
                }));
            }
        }
    }
    Ok(results)
}

#[test]
#[ignore = "evaluates six prepared Base116 distributions in RREA_EVALUATION_ROOT"]
fn base116_behavior_inventory_and_costs() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("RREA_EVALUATION_ROOT").context("missing RREA_EVALUATION_ROOT")?,
    );
    for variant in VARIANTS {
        ensure!(
            binary(&root, variant).is_file(),
            "missing evaluation variant {variant}"
        );
    }
    let work = tempfile::tempdir()?;
    let binary_cases = binary_cases(&root, work.path())?;
    let malformed_cases = malformed_cases(&root, work.path())?;
    let artifacts: BTreeMap<_, _> = VARIANTS
        .into_iter()
        .map(|variant| Ok((variant, inventory(&root, variant)?)))
        .collect::<Result<_>>()?;
    let measurements = measurements(&root, work.path())?;
    let tools = tools::acquire(true)?;
    let report = json!({
        "source_revision": fs::read_to_string(root.join("source-revision.txt"))?.trim(),
        "source_diff_sha256": tools::sha256(&root.join("source-at-evaluation.patch"))?,
        "base116_revision": fs::read_to_string(root.join("base116-revision.txt"))?.trim(),
        "base116_lock_sha256": tools::sha256(&root.join("base116/Cargo.lock"))?,
        "compiler": tools.compiler_release, "upx": tools.upx_build,
        "binary_cases_per_variant": binary_cases, "malformed_cases_per_variant": malformed_cases,
        "timed_input_bytes": 1_048_576, "artifacts": artifacts, "measurements": measurements,
        "limitations": ["RSS is a 1ms-sampled lower bound, not exact process peak.", "Baseline and protected builds have different linkage; their difference does not isolate LLVM obfuscation.", "No blinded analysts or decompiler success scores are measured."]
    });
    fs::write(
        root.join("evaluation.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "inspects live mappings of prepared Base116 variants"]
fn base116_runtime_mapping_names() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("RREA_EVALUATION_ROOT").context("missing RREA_EVALUATION_ROOT")?,
    );
    let mut report = BTreeMap::new();
    for variant in VARIANTS {
        let mut child = transform(&binary(&root, variant), "encode", Path::new("-"), false)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let proc = PathBuf::from(format!("/proc/{}", child.id()));
        let mut waiting = false;
        for _ in 0..100 {
            if fs::read_to_string(proc.join("wchan"))?.trim() == "anon_pipe_read" {
                waiting = true;
                break;
            }
            if child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let maps = fs::read_to_string(proc.join("maps"));
        drop(child.stdin.take());
        let output = child.wait_with_output()?;
        ensure!(
            waiting && output.status.success(),
            "variant did not reach stdin: {variant}"
        );
        let maps = maps?;
        let mappings: Vec<_> = maps
            .lines()
            .filter(|line| line.contains("memfd:"))
            .collect();
        ensure!(
            mappings
                .iter()
                .all(|line| !line.to_ascii_lowercase().contains("memfd:upx")),
            "named UPX mapping remains"
        );
        if matches!(variant, "packed" | "mangled" | "unmangled") {
            ensure!(
                !mappings.is_empty(),
                "packed loader mappings were not observed"
            );
        }
        report.insert(
            variant,
            mappings
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>(),
        );
    }
    fs::write(
        root.join("runtime-mappings.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
