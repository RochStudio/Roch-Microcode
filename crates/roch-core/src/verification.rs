//! Independent verification with the bundled third-party tools.
//!
//! UEFIExtract NE A75 and MCExtractor 1.104.0 / r352 (with a private embedded
//! CPython 3.11.9) are shipped unmodified inside `assets/verification-bundle.zip`,
//! which is embedded in the executable. The archive is extracted into the local
//! data folder under a directory named after its SHA-256, every file is
//! re-hashed against the embedded copy on each use, and the tools are invoked
//! with exactly the same arguments and comparison rules as the original app.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::builder::write_new_verified;
use crate::error::require;
use crate::firmware::{self, Microcode};
use crate::transfer::{self, ChangeRange, TransferResult, ValidationCheck, SLOT_REASON};
use crate::{report, CancelToken, Error, Progress, Result};

/// The unmodified third-party verifier bundle.
pub static VERIFICATION_BUNDLE: &[u8] = include_bytes!("../../../assets/verification-bundle.zip");

/// Per-tool time limit.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationEvidence {
    pub directory: PathBuf,
    pub checks: Vec<ValidationCheck>,
}

fn bundle_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| firmware::hash(VERIFICATION_BUNDLE))
}

static INSTALL_LOCK: Mutex<()> = Mutex::new(());

/// Extract (or re-verify) the bundled tools and return their folder.
pub fn tools(ct: &CancelToken) -> Result<PathBuf> {
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    ct.check()?;
    let root = crate::paths::data_directory().join("verifiers").join(&bundle_hash()[..16]);
    std::fs::create_dir_all(&root)?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(VERIFICATION_BUNDLE))
        .map_err(|_| Error::io("The bundled verification tools are missing. Reinstall Roch Microcode."))?;
    for i in 0..zip.len() {
        ct.check()?;
        let mut entry = zip.by_index(i).map_err(|e| Error::io(format!("Bundled tool archive is damaged: {e}")))?;
        if entry.name().ends_with('/') || entry.is_dir() {
            continue;
        }
        // Equivalent of the original full-path containment check.
        let relative = entry.enclosed_name().ok_or_else(|| Error::invalid("Invalid bundled tool path."))?;
        require(
            relative.components().all(|c| matches!(c, std::path::Component::Normal(_))),
            "Invalid bundled tool path.",
        )?;
        let path = root.join(relative);
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| Error::io(format!("Bundled tool archive is damaged: {e}")))?;
        if path.is_file() {
            if let Ok(existing) = std::fs::read(&path) {
                if firmware::hash(&existing) == firmware::hash(&bytes) {
                    continue;
                }
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &bytes)?;
    }
    Ok(root)
}

fn ansi_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\x1B\[[0-?]*[ -/]*[@-~]").expect("valid regex"))
}

fn decode(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).into_owned();
    text.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(text)
}

#[cfg(windows)]
fn kill_tree(child: &mut std::process::Child) {
    use std::os::windows::process::CommandExt;
    // Process.Kill(entireProcessTree: true).
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let _ = Command::new(system_root.join("System32").join("taskkill.exe"))
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(windows))]
fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Run a verifier hidden, capturing UTF-8 output, with a two-minute limit.
pub fn run(exe: &Path, args: &[&std::ffi::OsStr], work: &Path, ct: &CancelToken) -> Result<String> {
    let mut command = Command::new(exe);
    command.args(args).current_dir(work).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| Error::io(format!("Could not launch verification tool. {e}")))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stdout.read_to_end(&mut v);
        v
    });
    let err_thread = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stderr.read_to_end(&mut v);
        v
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if ct.is_cancelled() {
            kill_tree(&mut child);
            return Err(Error::Cancelled);
        }
        if started.elapsed() > TOOL_TIMEOUT {
            kill_tree(&mut child);
            return Err(Error::io("A verification tool timed out; no verified package was produced."));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let output = decode(&out_thread.join().unwrap_or_default());
    let error = decode(&err_thread.join().unwrap_or_default());
    if !status.success() {
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
        let excerpt: String = output.chars().take(1200).collect();
        return Err(Error::invalid(format!("{name} failed ({code}). {error} {excerpt}")));
    }
    require(!error.to_lowercase().contains("traceback"), "MCExtractor reported an exception.")?;
    let combined = if error.is_empty() { output } else { format!("{output}\n{error}") };
    Ok(ansi_regex().replace_all(&combined, "").into_owned())
}

fn read_text(path: &Path) -> Result<String> {
    Ok(decode(&std::fs::read(path)?))
}

/// Rebuild the transfer, then check the original and modified images with
/// UEFIExtract and the donor and both inventories with MCExtractor.
pub fn verify(
    result: &TransferResult,
    work: &Path,
    progress: Progress<'_>,
    ct: &CancelToken,
) -> Result<VerificationEvidence> {
    let result = transfer::revalidate(result)?;
    std::fs::create_dir_all(work)?;
    report(progress, "Preparing bundled UEFIExtract and MCExtractor…");
    let tools = tools(ct)?;
    let original = work.join("new-original.bin");
    let candidate = work.join("modified.bin");
    let donor = work.join("donor-microcode.bin");
    write_new_verified(&original, &result.original, &firmware::hash(&result.original))?;
    write_new_verified(&candidate, &result.image, &result.sha256())?;
    write_new_verified(&donor, &result.patch, &firmware::hash(&result.patch))?;
    report(progress, "UEFIExtract: comparing original and modified firmware structure…");
    let uefi = tools.join("UEFIExtract.exe");
    let uefi_original = run(&uefi, &[original.as_os_str(), "report".as_ref()], work, ct)?;
    let uefi_candidate = run(&uefi, &[candidate.as_os_str(), "report".as_ref()], work, ct)?;
    std::fs::write(work.join("original.uefi.log.txt"), &uefi_original)?;
    std::fs::write(work.join("modified.uefi.log.txt"), &uefi_candidate)?;
    compare_messages(&uefi_original, &uefi_candidate, false)?;
    let report_of = |p: &Path| {
        let mut s = p.as_os_str().to_owned();
        s.push(".report.txt");
        PathBuf::from(s)
    };
    compare_structure(&read_text(&report_of(&original))?, &read_text(&report_of(&candidate))?, &result.ranges)?;
    report(progress, "MCExtractor: checking donor, original inventory and modified inventory…");
    let mce = |input: &Path, label: &str| -> Result<String> {
        // MCE writes extraction files next to its script. Each invocation gets a private working copy.
        let dir = work.join(format!("mce-work-{label}"));
        std::fs::create_dir_all(&dir)?;
        for name in ["MCE.py", "MCE.db"] {
            let target = dir.join(name);
            if target.exists() {
                return Err(Error::io(format!("The file '{}' already exists.", target.display())));
            }
            std::fs::copy(tools.join("MCE").join(name), &target)?;
        }
        let python = tools.join("python").join("python.exe");
        let script = dir.join("MCE.py");
        let args: [&std::ffi::OsStr; 9] = [
            "-I".as_ref(),
            "-B".as_ref(),
            "-X".as_ref(),
            "utf8".as_ref(),
            script.as_os_str(),
            input.as_os_str(),
            "-skip".as_ref(),
            "-exit".as_ref(),
            "-duc".as_ref(),
        ];
        let output = run(&python, &args, &dir, ct)?;
        std::fs::write(work.join(format!("{label}.mce.log.txt")), &output)?;
        Ok(output)
    };
    let mce_original = mce(&original, "original")?;
    let mce_candidate = mce(&candidate, "modified")?;
    let mce_donor = mce(&donor, "donor")?;
    compare_messages(&mce_original, &mce_candidate, true)?;
    compare_messages("", &mce_donor, true)?;
    match_mce(&mce_original, &firmware::inspect(&result.original, "original").microcodes)?;
    match_mce(&mce_candidate, &firmware::inspect(&result.image, "modified").microcodes)?;
    match_mce(&mce_donor, &firmware::inspect(&result.patch, "donor").microcodes)?;
    Ok(VerificationEvidence {
        directory: work.to_path_buf(),
        checks: vec![
            ValidationCheck::new(
                "UEFIExtract A75",
                "No new parser diagnostics; unchanged structure and CRCs outside the authorized changes",
            ),
            ValidationCheck::new(
                "MCExtractor 1.104.0 / r352",
                "Donor and both inventories independently matched; no new warnings or errors",
            ),
        ],
    })
}

fn diagnostic_lines(text: &str, mce: bool) -> Vec<String> {
    text.split('\n')
        .map(str::trim)
        .filter(|l| {
            let lower = l.to_lowercase();
            !l.is_empty()
                && ((!mce && l.contains("::"))
                    || lower.contains("error:")
                    || lower.contains("warning:")
                    || lower.contains("traceback"))
        })
        .map(str::to_string)
        .collect()
}

/// Every diagnostic in `after` must already exist in `before` (as a multiset).
pub fn compare_messages(before: &str, after: &str, mce: bool) -> Result<()> {
    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for line in diagnostic_lines(before, mce) {
        *counts.entry(line).or_default() += 1;
    }
    for line in diagnostic_lines(after, mce) {
        let count = counts.get_mut(&line);
        match count {
            Some(c) if *c > 0 => *c -= 1,
            _ => return Err(Error::invalid(format!("Independent verifier reported a new diagnostic: {line}"))),
        }
    }
    Ok(())
}

/// `int.TryParse(s, NumberStyles.HexNumber)`: 1-8 significant hex digits, two's complement.
fn parse_hex_i32(s: &str) -> Option<i32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let trimmed = s.trim_start_matches('0');
    if trimmed.len() > 8 {
        return None;
    }
    if trimmed.is_empty() {
        return Some(0);
    }
    u32::from_str_radix(trimmed, 16).ok().map(|v| v as i32)
}

fn normalize_report(text: &str, ranges: &[ChangeRange]) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut c: Vec<String> = line.split('|').map(|v| v.trim().to_string()).collect();
        if c.len() >= 6 && c[2] == "N/A" {
            rows.push(c.join("|"));
            continue;
        }
        if c.len() < 6 {
            continue;
        }
        let (Some(start), Some(size)) = (parse_hex_i32(&c[2]), parse_hex_i32(&c[3])) else {
            continue;
        };
        let start = start as i64;
        let end = start + size as i64;
        let overlaps = ranges.iter().any(|r| start < (r.offset + r.size) as i64 && end > r.offset as i64);
        // UEFIExtract may merge the new FF tail with padding immediately after the old slot.
        // Native byte comparison has already proven that all bytes beyond the slot are unchanged.
        if c[0] == "Padding"
            && ranges
                .iter()
                .any(|r| r.reason == SLOT_REASON && start <= (r.offset + r.size) as i64 && end >= r.offset as i64)
        {
            continue;
        }
        if overlaps && c[0] == "Intel microcode" {
            continue;
        }
        if ranges.iter().any(|r| start <= r.offset as i64 && end >= (r.offset + r.size) as i64) {
            c[4] = "EXPECTED-ANCESTOR-CRC".into();
        }
        rows.push(c.join("|"));
    }
    rows
}

/// Compare UEFIExtract reports: identical tree and CRCs except the replaced
/// microcode, adjacent padding and CRCs of nodes that contain the change.
pub fn compare_structure(before: &str, after: &str, ranges: &[ChangeRange]) -> Result<()> {
    let a = normalize_report(before, ranges);
    let b = normalize_report(after, ranges);
    require(a.len() >= 5, "UEFIExtract did not produce a usable firmware tree.")?;
    require(
        a == b,
        "UEFIExtract found an unexpected firmware tree or CRC difference outside the replaced microcode. Review the reports.",
    )
}

fn mce_hex(s: &str) -> Result<u32> {
    let lower = s.to_ascii_lowercase();
    let mut cleaned = String::new();
    let mut rest = s;
    let mut lower_rest = lower.as_str();
    while let Some(i) = lower_rest.find("0x") {
        cleaned.push_str(&rest[..i]);
        rest = &rest[i + 2..];
        lower_rest = &lower_rest[i + 2..];
    }
    cleaned.push_str(rest);
    let t = cleaned.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::invalid(format!("MCExtractor inventory could not be parsed: {s}")));
    }
    let significant = t.trim_start_matches('0');
    if significant.len() > 8 {
        return Err(Error::invalid(format!("MCExtractor inventory could not be parsed: {s}")));
    }
    Ok(if significant.is_empty() { 0 } else { u32::from_str_radix(significant, 16).unwrap_or(0) })
}

/// The primary MCExtractor inventory must equal the native parser's inventory.
pub fn match_mce(output: &str, expected: &[Microcode]) -> Result<()> {
    let mut seen: Vec<(u32, u32, u32, i64, i64)> = Vec::new();
    for line in output.split('\n') {
        if line.trim_start().starts_with("Extended_") {
            break;
        }
        let c: Vec<&str> = line.split('│').map(|s| s.trim_matches([' ', '║', '\r'])).collect();
        if c.len() < 10 || c[1] != "Microcode" {
            continue;
        }
        let platform = c[3].split(' ').next().unwrap_or("");
        seen.push((
            mce_hex(c[2])?,
            mce_hex(platform)?,
            mce_hex(c[4])?,
            mce_hex(c[7])? as i32 as i64,
            mce_hex(c[8])? as i32 as i64,
        ));
    }
    require(
        seen.len() == expected.len()
            && expected
                .iter()
                .all(|m| seen.contains(&(m.cpuid, m.platforms, m.revision, m.size as i64, m.offset as i64))),
        "MCExtractor inventory disagrees with the native parser.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(offset: usize, size: usize, reason: &str) -> ChangeRange {
        ChangeRange { offset, size, reason: reason.into() }
    }

    #[test]
    fn messages_multiset() {
        let before = "Warning: a\nWarning: a\nparse: x :: y\nok\n";
        compare_messages(before, "Warning: a\n", true).unwrap();
        compare_messages(before, "  Warning: a  \r\nWarning: a", true).unwrap();
        assert!(compare_messages(before, "Warning: a\nWarning: a\nWarning: a", true).is_err());
        assert!(compare_messages(before, "ERROR: new", true).is_err_and(|e| e.is_invalid_data()));
        // "::" diagnostics are only collected for UEFIExtract output.
        compare_messages("", "parse: x :: y", true).unwrap();
        assert!(compare_messages("", "parse: x :: y", false).is_err());
        compare_messages(before, "parse: x :: y", false).unwrap();
        assert!(compare_messages("", "Traceback (most recent call last)", true).is_err());
    }

    fn report_line(kind: &str, offset: usize, size: usize, crc: &str) -> String {
        format!("{kind} | Raw | {offset:08X} | {size:08X} | {crc} | name")
    }

    #[test]
    fn structure_comparison_rules() {
        let ranges = vec![range(0x1000, 0x800, SLOT_REASON)];
        let fixed = [
            "Type | Subtype | Base | Size | CRC32 | Name".to_string(),
            report_line("Image", 0, 0x10000, "11111111"),
            report_line("Volume", 0, 0x8000, "22222222"),
            report_line("File", 0x0f00, 0x1000, "33333333"),
            report_line("Section", 0x9000, 0x100, "44444444"),
            report_line("File", 0xa000, 0x100, "55555555"),
        ];
        let before =
            [fixed.to_vec(), vec![report_line("Intel microcode", 0x1000, 0x800, "AAAAAAAA")]].concat().join("\n");
        // Ancestor CRC changes, the microcode node and merged trailing padding are accepted.
        let mut after_rows = fixed.to_vec();
        after_rows[1] = report_line("Image", 0, 0x10000, "99999999");
        after_rows[2] = report_line("Volume", 0, 0x8000, "88888888");
        after_rows[3] = report_line("File", 0x0f00, 0x1000, "77777777");
        after_rows.push(report_line("Intel microcode", 0x1000, 0x700, "BBBBBBBB"));
        after_rows.push(report_line("Padding", 0x1700, 0x200, "CCCCCCCC"));
        compare_structure(&before, &after_rows.join("\r\n"), &ranges).unwrap();
        // A CRC change in an unrelated node is rejected.
        let mut bad = after_rows.clone();
        bad[4] = report_line("Section", 0x9000, 0x100, "DEADBEEF");
        assert!(compare_structure(&before, &bad.join("\n"), &ranges).is_err_and(|e| e.is_invalid_data()));
        // A new node is rejected.
        let mut extra = after_rows.clone();
        extra.push(report_line("File", 0xb000, 0x100, "66666666"));
        assert!(compare_structure(&before, &extra.join("\n"), &ranges).is_err());
        // Too small a tree is rejected even when identical.
        let tiny = fixed[..3].join("\n");
        assert!(compare_structure(&tiny, &tiny, &ranges).is_err());
    }

    #[test]
    fn hex_parsing_like_dotnet() {
        assert_eq!(parse_hex_i32("FFFFFFFF"), Some(-1));
        assert_eq!(parse_hex_i32("0000000010"), Some(16));
        assert_eq!(parse_hex_i32("100000000"), None);
        assert_eq!(parse_hex_i32("0x10"), None);
        assert_eq!(parse_hex_i32(""), None);
        assert_eq!(mce_hex("0x000B0671").unwrap(), 0xb0671);
        assert_eq!(mce_hex("0X11F").unwrap(), 0x11f);
        assert!(mce_hex("zz").is_err());
    }

    #[test]
    fn mce_inventory_matching() {
        let m = Microcode {
            offset: 0x1000,
            cpuid: 0xb0671,
            platforms: 0x32,
            revision: 0x11f,
            size: 0x33c00,
            date: "2023-01-01".into(),
            checksum_valid: true,
            known_oem_header: false,
        };
        let row =
            "║ 1 │ Microcode │ 0x000B0671 │ 0x32 (1,4,5) │ 0x0000011F │ 2023-01-01 │ PRD │ 0x33C00 │ 0x1000 │ No ║";
        let out = format!("╔══╗\n{row}\r\nExtended_Signatures\n║ 2 │ Microcode │ 0x000B0672 │ 0x32 │ 0x1 │ x │ y │ 0x800 │ 0x0 │ z ║\n");
        match_mce(&out, std::slice::from_ref(&m)).unwrap();
        assert!(match_mce(&out, &[]).is_err());
        let wrong = Microcode { revision: 0x120, ..m.clone() };
        assert!(match_mce(&out, &[wrong]).is_err_and(|e| e.is_invalid_data()));
        assert!(match_mce(&format!("{row}\n{row}"), std::slice::from_ref(&m)).is_err());
    }

    #[test]
    fn bundle_is_embedded() {
        assert!(VERIFICATION_BUNDLE.len() > 1_000_000);
        let zip = zip::ZipArchive::new(std::io::Cursor::new(VERIFICATION_BUNDLE)).unwrap();
        let names: Vec<&str> = zip.file_names().collect();
        for required in ["UEFIExtract.exe", "MCE/MCE.py", "MCE/MCE.db", "python/python.exe"] {
            assert!(names.contains(&required), "{required} missing from bundle");
        }
    }
}
