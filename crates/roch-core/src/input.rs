//! Loading BIOS files and vendor ZIP packages; vendor and filename suggestions.

use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::error::require;
use crate::firmware::{self, recipe, FirmwareInfo, MAX_IMAGE_SIZE};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct FirmwareInput {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub info: FirmwareInfo,
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `Path.GetExtension`: the last `.` suffix of the final component, or empty.
pub(crate) fn extension_of(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i + 1 < base.len() => base[i..].to_string(),
        _ => String::new(),
    }
}

pub(crate) fn msi_name_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^E[0-9A-Z]{4}[IA]MS\.[0-9A-Z]{3}$").expect("valid regex"))
}

fn gigabyte_name_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\.F[A-Z0-9]{1,3}$").expect("valid regex"))
}

/// Case-insensitive ASCII search over raw bytes (non-ASCII bytes never match letters).
fn contains_ascii_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// Equivalent of the regex `(?<![A-Z0-9])[A-Z0-9]{3,8}\.CAP` evaluated over the
/// ASCII decoding of the image (leftmost, non-overlapping matches).
fn asus_cap_names(bytes: &[u8]) -> Vec<String> {
    let alnum = |c: u8| c.is_ascii_uppercase() || c.is_ascii_digit();
    let mut names: Vec<String> = Vec::new();
    let n = bytes.len();
    let mut pos = 0usize;
    while pos < n {
        if !alnum(bytes[pos]) || (pos > 0 && alnum(bytes[pos - 1])) {
            pos += 1;
            continue;
        }
        let mut run = 0usize;
        while pos + run < n && run < 9 && alnum(bytes[pos + run]) {
            run += 1;
        }
        let tail = pos + run;
        if (3..=8).contains(&run) && tail + 4 <= n && &bytes[tail..tail + 4] == b".CAP" {
            let name = String::from_utf8_lossy(&bytes[pos..tail + 4]).into_owned();
            if !names.contains(&name) {
                names.push(name);
            }
            pos = tail + 4;
        } else {
            pos += 1;
        }
    }
    names
}

impl FirmwareInput {
    pub fn from_bytes(file_name: impl Into<String>, bytes: Vec<u8>) -> Self {
        let file_name = file_name.into();
        let info = firmware::inspect(&bytes, &file_name);
        Self { file_name, bytes, info }
    }

    /// Load a BIOS file, or a vendor ZIP that contains exactly one identifiable
    /// Intel image. Bundled flash utilities are never executed.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let name = file_name_of(path);
        if !extension_of(&name).eq_ignore_ascii_case(".zip") {
            let bytes = firmware::read_image(path)?;
            return Ok(Self::from_bytes(name, bytes));
        }
        let file = std::fs::File::open(path)?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| Error::invalid(format!("The ZIP file could not be read: {e}")))?;
        const SKIP: [&str; 6] = [".exe", ".efi", ".pdf", ".txt", ".dll", ".zip"];
        let mut entries = Vec::new();
        for i in 0..archive.len() {
            let entry =
                archive.by_index_raw(i).map_err(|e| Error::invalid(format!("The ZIP file could not be read: {e}")))?;
            let full = entry.name().to_string();
            let entry_name = full.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            let len = entry.size();
            if len >= 1024 * 1024
                && len <= MAX_IMAGE_SIZE as u64
                && !SKIP.contains(&extension_of(&entry_name).to_ascii_lowercase().as_str())
            {
                entries.push((i, entry_name));
            }
        }
        require(
            !entries.is_empty() && entries.len() <= 12,
            "ZIP has no clear firmware candidate, or too many large entries. Extract it and choose the BIOS file.",
        )?;
        let mut candidates = Vec::new();
        for (index, entry_name) in entries {
            let entry =
                archive.by_index(index).map_err(|e| Error::invalid(format!("The ZIP entry could not be read: {e}")))?;
            let mut bytes = Vec::new();
            let mut limited = entry.take(MAX_IMAGE_SIZE as u64 + 1);
            let mut buffer = vec![0u8; 65536];
            loop {
                let n = limited
                    .read(&mut buffer)
                    .map_err(|e| Error::invalid(format!("The ZIP entry could not be read: {e}")))?;
                if n == 0 {
                    break;
                }
                require(bytes.len() + n <= MAX_IMAGE_SIZE, "ZIP entry exceeds 128 MB.")?;
                bytes.extend_from_slice(&buffer[..n]);
            }
            let input = Self::from_bytes(entry_name, bytes);
            if !input.info.microcodes.is_empty() {
                candidates.push(input);
            }
        }
        require(
            candidates.len() == 1,
            if candidates.is_empty() {
                "No uncompressed Intel microcode found in this ZIP. Extract it and inspect the BIOS file."
            } else {
                "ZIP contains more than one BIOS image. Extract it and choose the exact image."
            },
        )?;
        Ok(candidates.remove(0))
    }

    /// Manufacturer suggestion from known hashes, vendor filename patterns or image strings.
    pub fn suggested_vendor(&self) -> &'static str {
        let h = self.info.sha256.as_str();
        if h == recipe::BASE_HASH
            || h == recipe::DONOR_HASH
            || h == recipe::RESULT_HASH
            || self.file_name.to_ascii_uppercase().contains("-ASUS-")
        {
            "ASUS"
        } else if msi_name_regex().is_match(&self.file_name) {
            "MSI"
        } else if gigabyte_name_regex().is_match(&self.file_name) {
            "Gigabyte"
        } else if contains_ascii_ci(&self.bytes, b"ASRock") {
            "ASRock"
        } else {
            ""
        }
    }

    /// The ASUS FlashBack name, when exactly one model-specific `.CAP` name is embedded.
    pub fn suggested_asus_name(&self) -> Option<String> {
        if self.suggested_vendor() != "ASUS" {
            return None;
        }
        let names: Vec<String> = asus_cap_names(&self.bytes).into_iter().filter(|n| n != "ASUS.CAP").collect();
        if names.len() == 1 {
            names.into_iter().next()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(extension_of("a.zip"), ".zip");
        assert_eq!(extension_of("dir.v1/file"), "");
        assert_eq!(extension_of("E7E01IMS.PA0"), ".PA0");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("trailing."), "");
    }

    #[test]
    fn asus_names() {
        assert_eq!(asus_cap_names(b"xx SZ790AD4.CAP yy"), vec!["SZ790AD4.CAP"]);
        assert!(asus_cap_names(b"ABCDEFGHI.CAP").is_empty(), "nine characters is too long");
        assert!(
            asus_cap_names(b"aSZ790.CAP").contains(&"SZ790.CAP".to_string()),
            "lowercase is not alphanumeric for the pattern"
        );
        assert!(asus_cap_names(b"AB.CAP").is_empty());
        assert_eq!(asus_cap_names(b"ABC.CAPXYZ.CAP"), vec!["ABC.CAP"]);
        assert_eq!(asus_cap_names(b"ASUS.CAP\0SZ790AD4.CAP\0SZ790AD4.CAP"), vec!["ASUS.CAP", "SZ790AD4.CAP"]);
    }

    #[test]
    fn vendor_suggestions() {
        let mk = |name: &str, bytes: &[u8]| FirmwareInput::from_bytes(name, bytes.to_vec());
        assert_eq!(mk("ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-3202.CAP", b"").suggested_vendor(), "ASUS");
        assert_eq!(mk("E7E01IMS.PA0", b"").suggested_vendor(), "MSI");
        assert_eq!(mk("e7e07ams.1k0", b"").suggested_vendor(), "MSI");
        assert_eq!(mk("Z790AORUSELITEAX11.FH", b"").suggested_vendor(), "Gigabyte");
        assert_eq!(mk("Z790-Taichi_20.01.ROM", b"..asrock..").suggested_vendor(), "ASRock");
        assert_eq!(mk("bios.bin", b"nothing").suggested_vendor(), "");
        let asus = mk("X-ASUS-1.CAP", b"\0ASUS.CAP\0SZ790AD4.CAP\0");
        assert_eq!(asus.suggested_asus_name().as_deref(), Some("SZ790AD4.CAP"));
        let ambiguous = mk("X-ASUS-1.CAP", b"\0AAA1.CAP\0BBB2.CAP\0");
        assert_eq!(ambiguous.suggested_asus_name(), None);
        assert_eq!(mk("bios.bin", b"\0SZ790AD4.CAP\0").suggested_asus_name(), None);
    }
}
