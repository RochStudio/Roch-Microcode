//! Intel microcode discovery and the pinned ASUS reference recipe.

use std::fmt;
use std::io::Read;
use std::path::Path;

use crate::format::{decimals, sha256_hex};
use crate::{Error, Result};

/// Largest firmware image accepted anywhere in the app.
pub const MAX_IMAGE_SIZE: usize = 128 * 1024 * 1024;

/// One uncompressed Intel microcode update found in an image.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Microcode {
    pub offset: usize,
    pub cpuid: u32,
    pub platforms: u32,
    pub revision: u32,
    pub size: usize,
    pub date: String,
    pub checksum_valid: bool,
    pub known_oem_header: bool,
}

impl Microcode {
    pub fn cpu_text(&self) -> String {
        format!("{:05X}", self.cpuid)
    }
    pub fn revision_text(&self) -> String {
        format!("0x{:X}", self.revision)
    }
    pub fn platform_text(&self) -> String {
        format!("0x{:02X}", self.platforms)
    }
    pub fn offset_text(&self) -> String {
        format!("0x{:X}", self.offset)
    }
    pub fn size_text(&self) -> String {
        format!("{} KB", decimals(self.size as f64 / 1024.0, 1))
    }
    pub fn status(&self) -> &'static str {
        if self.checksum_valid {
            "Valid checksum"
        } else if self.known_oem_header {
            "ASUS alternate header"
        } else {
            "Invalid checksum"
        }
    }
    pub fn display(&self) -> String {
        format!(
            "{}/{:02X} • {} • {} • @{:X}",
            self.cpu_text(),
            self.platforms,
            self.revision_text(),
            self.size_text(),
            self.offset
        )
    }
}

impl fmt::Display for Microcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirmwareInfo {
    pub file_name: String,
    pub sha256: String,
    pub size: usize,
    pub identity: String,
    pub microcodes: Vec<Microcode>,
}

impl FirmwareInfo {
    pub fn is_supported_base(&self) -> bool {
        self.sha256 == recipe::BASE_HASH
    }
    pub fn is_known_candidate(&self) -> bool {
        self.sha256 == recipe::RESULT_HASH
    }
}

/// Lowercase SHA-256 of the bytes.
pub fn hash(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

#[inline]
pub fn u16_at(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}
#[inline]
pub fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().expect("4 bytes"))
}
#[inline]
pub fn u64_at(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().expect("8 bytes"))
}

/// Intel additive checksum: all dwords sum to zero (mod 2^32).
pub fn sum32_valid(bytes: &[u8]) -> bool {
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    bytes.as_chunks::<4>().0.iter().fold(0u32, |sum, c| sum.wrapping_add(u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
        == 0
}

/// Read a firmware file between 48 bytes and 128 MB. The read is bounded even
/// if the file grows after the size check.
pub fn read_image(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let len = std::fs::metadata(path)?.len();
    if len < 48 || len > MAX_IMAGE_SIZE as u64 {
        return Err(Error::invalid("Choose a firmware image between 48 bytes and 128 MB."));
    }
    let mut input = std::fs::File::open(path)?;
    let mut buffer = Vec::with_capacity(len as usize);
    let mut chunk = vec![0u8; 65536];
    loop {
        let n = input.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if buffer.len() + n > MAX_IMAGE_SIZE {
            return Err(Error::invalid("Firmware image exceeds 128 MB."));
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    Ok(buffer)
}

/// Parse the BCD-like Intel date dword (`MMddyyyy` read as hex digits) into
/// `yyyy-MM-dd`, rejecting anything `DateTime.TryParseExact` would reject.
fn parse_intel_date(value: u32) -> Option<String> {
    let text = format!("{value:08X}");
    if !text.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month: u32 = text[0..2].parse().ok()?;
    let day: u32 = text[2..4].parse().ok()?;
    let year: u32 = text[4..8].parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if leap => 29,
        _ => 28,
    };
    if day > days {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Scan for uncompressed Intel microcode updates (4-byte aligned headers).
pub fn inspect(image: &[u8], file_name: &str) -> FirmwareInfo {
    let hash = hash(image);
    let trusted_asus = hash == recipe::BASE_HASH || hash == recipe::RESULT_HASH || hash == recipe::DONOR_HASH;
    let mut found = Vec::new();
    let len = image.len();
    let mut p = 0usize;
    while len >= 48 && p <= len - 48 {
        if u32_at(image, p) != 1 || u32_at(image, p + 20) != 1 {
            p += 4;
            continue;
        }
        let cpu = u32_at(image, p + 12);
        let rev = u32_at(image, p + 4);
        let flags = u32_at(image, p + 24);
        let mut size = u32_at(image, p + 32);
        let mut data_size = u32_at(image, p + 28);
        if size == 0 {
            size = 2048;
        }
        if data_size == 0 {
            data_size = 2000;
        }
        if !(2048..=2 * 1024 * 1024).contains(&size)
            || !size.is_multiple_of(1024)
            || data_size > size - 48
            || !data_size.is_multiple_of(4)
            || size as usize > len - p
        {
            p += 4;
            continue;
        }
        if cpu == 0 || rev == 0 || flags > 255 {
            p += 4;
            continue;
        }
        let Some(date) = parse_intel_date(u32_at(image, p + 8)) else {
            p += 4;
            continue;
        };
        let size = size as usize;
        let valid = sum32_valid(&image[p..p + size]) && sum32_valid(&image[p..p + 48 + data_size as usize]);
        // Known ASUS alternate patch has an intentionally changed CPUID. Never exempt unknown files.
        let oem = trusted_asus && cpu == 0xff0671 && rev == 0x104;
        found.push(Microcode {
            offset: p,
            cpuid: cpu,
            platforms: flags,
            revision: rev,
            size,
            date,
            checksum_valid: valid,
            known_oem_header: oem,
        });
        p += size;
    }
    let identity = match hash.as_str() {
        recipe::BASE_HASH => "ASUS Z790-A D4 • official BIOS 3202",
        recipe::DONOR_HASH => "ASUS Z790-A D4 • official BIOS 1904 donor",
        recipe::PATCH_HASH => "Intel B0671 • 0x11F donor patch",
        recipe::RESULT_HASH => "ASUS Z790-A D4 • BIOS 3202 + 0x11F",
        _ => "Firmware image • compatibility checked when building",
    }
    .to_string();
    FirmwareInfo { file_name: file_name.to_string(), sha256: hash, size: image.len(), identity, microcodes: found }
}

/// The pinned, byte-exact ASUS ROG STRIX Z790-A GAMING WIFI D4 reference recipe.
pub mod recipe {
    pub const BOARD: &str = "ROG STRIX Z790-A GAMING WIFI D4";
    pub const FLASH_NAME: &str = "SZ790AD4.CAP";
    pub const BASE_HASH: &str = "4000ef2f68a14c64b8c7c968478d3414927c325b528266fe4f9ebf3cced42762";
    pub const DONOR_HASH: &str = "d3ba71da21876c403440ca8f633997b606dfc3fde868fe75db42142d1455302c";
    pub const PATCH_HASH: &str = "1d0b38eb084a284961d58a2fb3a90313ff94853dc5da9f7348505f5ef68b57f2";
    pub const RESULT_HASH: &str = "4b7bf8c4550e5cbe4c8d4594ce8e90368f8436064ec8f80ba3a494c2d3656b5a";
    pub const START: usize = 0x10f9400;
    pub const OLD_SIZE: usize = 0x35800;
    pub const NEW_SIZE: usize = 0x33c00;
    pub const DONOR_START: usize = 0x10f8000;
    pub const SUPPORT_URL: &str =
        "https://www.asus.com/supportonly/rog%20strix%20z790-a%20gaming%20wifi%20d4/helpdesk_bios/";

    pub fn base_download() -> DownloadSpec {
        DownloadSpec {
            version: "3202".into(),
            zip_hash: "dfad9bbd7f22aec2d76f384954606d7bc453fc601896ffe02fab2d2ded633d6c".into(),
            cap_hash: BASE_HASH.into(),
        }
    }
    pub fn donor_download() -> DownloadSpec {
        DownloadSpec {
            version: "1904".into(),
            zip_hash: "84664987f88b9f51f3e41e1345e09f0b8e66deb1351a424ffbadc671152d5d42".into(),
            cap_hash: DONOR_HASH.into(),
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct DownloadSpec {
        pub version: String,
        pub zip_hash: String,
        pub cap_hash: String,
    }

    impl DownloadSpec {
        pub fn cap_name(&self) -> String {
            format!("ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-{}.CAP", self.version)
        }
        pub fn url(&self) -> String {
            format!(
                "https://dlcdnets.asus.com/pub/ASUS/mb/BIOS/ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-{}.zip",
                self.version
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intel_dates() {
        assert_eq!(parse_intel_date(0x0129_2024).as_deref(), Some("2024-01-29"));
        assert_eq!(parse_intel_date(0x0229_2024).as_deref(), Some("2024-02-29"));
        assert_eq!(parse_intel_date(0x0229_2023), None);
        assert_eq!(parse_intel_date(0x0229_1900), None);
        assert_eq!(parse_intel_date(0x0229_2000).as_deref(), Some("2000-02-29"));
        assert_eq!(parse_intel_date(0x1301_2024), None);
        assert_eq!(parse_intel_date(0x0100_2024), None);
        assert_eq!(parse_intel_date(0x0101_0000), None);
        assert_eq!(parse_intel_date(0x0A01_2024), None);
        assert_eq!(parse_intel_date(0x0431_2024), None);
    }

    #[test]
    fn sum32() {
        assert!(sum32_valid(&[]));
        assert!(!sum32_valid(&[1, 2, 3]));
        let mut v = vec![0u8; 8];
        v[0..4].copy_from_slice(&5u32.to_le_bytes());
        v[4..8].copy_from_slice(&5u32.wrapping_neg().to_le_bytes());
        assert!(sum32_valid(&v));
        v[0] ^= 1;
        assert!(!sum32_valid(&v));
    }

    #[test]
    fn empty_image_scanner() {
        assert!(inspect(&[], "empty").microcodes.is_empty());
        assert!(inspect(&[0u8; 47], "short").microcodes.is_empty());
    }

    #[test]
    fn microcode_text() {
        let m = Microcode {
            offset: 0x10f9400,
            cpuid: 0xb0671,
            platforms: 0x32,
            revision: 0x11f,
            size: 0x33c00,
            date: "2023-01-01".into(),
            checksum_valid: true,
            known_oem_header: false,
        };
        assert_eq!(m.display(), "B0671/32 • 0x11F • 207 KB • @10F9400");
        assert_eq!(m.platform_text(), "0x32");
        assert_eq!(m.cpu_text(), "B0671");
        assert_eq!(Microcode { cpuid: 0x906a, ..m.clone() }.cpu_text(), "0906A");
        assert_eq!(m.status(), "Valid checksum");
    }
}
