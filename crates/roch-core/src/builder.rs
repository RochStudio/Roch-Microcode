//! Verified file writes and the pinned, byte-exact ASUS 3202 + 11F reference recipe.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::require;
use crate::firmware::{self, recipe, u32_at, u64_at};
use crate::format::{guid_n, local_timestamp, sha256_hex};
use crate::transfer::ValidationCheck;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildResult {
    pub image: Vec<u8>,
    pub original: Vec<u8>,
    pub sha256: String,
    pub checks: Vec<ValidationCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildPackage {
    pub directory: PathBuf,
    pub candidate_path: PathBuf,
    pub recovery_path: PathBuf,
    pub report_path: PathBuf,
}

/// Create a new file (never overwriting), write it through to the device,
/// flush, then read it back and require the expected SHA-256.
pub fn write_new_verified(path: impl AsRef<Path>, data: &[u8], hash: &str) -> Result<()> {
    let path = path.as_ref();
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_WRITE_THROUGH, as FileOptions.WriteThrough.
            options.custom_flags(0x8000_0000);
            // FileShare.None.
            options.share_mode(0);
        }
        let mut file = options.open(path)?;
        file.write_all(data)?;
        file.flush()?;
        file.sync_all()?;
    }
    let read_back = std::fs::read(path)?;
    require(sha256_hex(&read_back) == hash, "Written file failed SHA-256 verification.")
}

/// Build the reference image from the exact official ASUS 3202 and either the
/// official 1904 BIOS or its verified B0671 / 11F patch.
pub fn build(source: &[u8], donor: &[u8]) -> Result<BuildResult> {
    require(
        firmware::hash(source) == recipe::BASE_HASH,
        "This build recipe requires the exact official Z790-A D4 BIOS 3202. Other images can be inspected, but not modified.",
    )?;
    let dh = firmware::hash(donor);
    require(
        dh == recipe::DONOR_HASH || dh == recipe::PATCH_HASH,
        "Choose official Z790-A D4 BIOS 1904 or its verified B0671 / 11F patch.",
    )?;
    let patch = if dh == recipe::DONOR_HASH {
        donor[recipe::DONOR_START..recipe::DONOR_START + recipe::NEW_SIZE].to_vec()
    } else {
        donor.to_vec()
    };
    require(firmware::hash(&patch) == recipe::PATCH_HASH, "Donor microcode hash does not match.")?;
    require(firmware::sum32_valid(&patch), "Donor checksum is invalid.")?;
    let mut image = source.to_vec();
    image[recipe::START..recipe::START + recipe::NEW_SIZE].copy_from_slice(&patch);
    image[recipe::START + recipe::NEW_SIZE..recipe::START + recipe::OLD_SIZE].fill(0xff);
    validate(source, &image)
}

/// Validate a reference build against the recipe, byte for byte.
pub fn validate(original: &[u8], candidate: &[u8]) -> Result<BuildResult> {
    use recipe::{NEW_SIZE, OLD_SIZE, START};
    require(firmware::hash(original) == recipe::BASE_HASH, "Original BIOS is not the supported 3202 image.")?;
    require(candidate.len() == original.len(), "Image size changed.")?;
    require(
        original[..START] == candidate[..START] && original[START + OLD_SIZE..] == candidate[START + OLD_SIZE..],
        "Bytes outside the microcode slot changed.",
    )?;
    require(
        firmware::hash(&candidate[START..START + NEW_SIZE]) == recipe::PATCH_HASH,
        "Replacement is not the verified 11F patch.",
    )?;
    require(candidate[START + NEW_SIZE..START + OLD_SIZE].iter().all(|&b| b == 0xff), "Slot padding is invalid.")?;
    let microcodes = firmware::inspect(candidate, "candidate").microcodes;
    require(microcodes.len() == 4, "Expected exactly four microcodes.")?;
    require(
        microcodes[2].cpuid == 0xb0671 && microcodes[2].revision == 0x11f && microcodes[2].checksum_valid,
        "B0671 / 11F is not valid.",
    )?;
    require(microcodes.iter().take(3).all(|m| m.checksum_valid), "A microcode checksum failed.")?;
    let fit = 0x1091100usize;
    require(&candidate[fit..fit + 8] == b"_FIT_   ", "FIT signature missing.")?;
    require(candidate[fit..fit + 96] == original[fit..fit + 96], "FIT changed.")?;
    require(u32_at(candidate, fit + 8) & 0xffffff == 6, "FIT count changed.")?;
    let offsets = [0x1091400usize, 0x10c8c00, START, 0x112ec00];
    for (i, &expected) in offsets.iter().enumerate() {
        let entry = fit + 16 * (i + 1);
        let pos = u64_at(candidate, entry) as i64 - 0xfe00_0000_i64 + 0x1000;
        require(pos == expected as i64 && candidate[entry + 14] & 127 == 1, "A FIT microcode pointer is invalid.")?;
        require(microcodes[i].offset as i64 == pos, "FIT does not resolve to the expected microcode.")?;
    }
    let h = &candidate[0x10913e8..0x10913e8 + 24];
    let sum: i32 = h.iter().map(|&b| b as i32).sum();
    require(
        (sum - h[17] as i32 - h[23] as i32) % 256 == 0 && h[19] & 0x40 == 0 && h[17] == 0xaa,
        "FFS checksum configuration changed.",
    )?;
    let hash = firmware::hash(candidate);
    require(hash == recipe::RESULT_HASH, "Final image differs from the independently checked reference build.")?;
    Ok(BuildResult {
        image: candidate.to_vec(),
        original: original.to_vec(),
        sha256: hash,
        checks: vec![
            ValidationCheck::new("Official source", "Exact ASUS 3202 and 11F donor hashes"),
            ValidationCheck::new("Microcode", "B0671 • platform 32 • revision 11F • checksum valid"),
            ValidationCheck::new("FIT pointers", "All four entries resolve at their original addresses"),
            ValidationCheck::new("Firmware layout", "Same image size; 7,168 bytes of FF padding"),
            ValidationCheck::new("Other components", "Every byte outside the microcode slot is unchanged"),
            ValidationCheck::new("FFS header", "Header checksum and fixed AA data-checksum mode valid"),
            ValidationCheck::new("Reference build", "Byte-identical to the build reported booting by the user"),
        ],
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceReport<'a> {
    app: &'a str,
    created_utc: String,
    board: &'a str,
    bios: &'a str,
    microcode: &'a str,
    cpuid: &'a str,
    platform: &'a str,
    base_sha256: &'a str,
    donor_patch_sha256: &'a str,
    sha256: &'a str,
    checks: &'a [ValidationCheck],
    evidence: &'a str,
    limitations: [&'a str; 3],
}

/// `DateTimeOffset.UtcNow` in System.Text.Json form.
pub(crate) fn utc_now_iso() -> String {
    use chrono::Timelike;
    let now = chrono::Utc::now();
    let ticks = now.nanosecond() % 1_000_000_000 / 100;
    format!("{}.{ticks:07}+00:00", now.format("%Y-%m-%dT%H:%M:%S"))
}

/// Export the reference build with a recovery copy and report.
pub fn export(result: &BuildResult, parent: impl AsRef<Path>) -> Result<BuildPackage> {
    // Revalidate at the export boundary: a caller may have changed the exposed byte arrays.
    let result = validate(&result.original, &result.image)?;
    let parent = parent.as_ref();
    std::fs::create_dir_all(parent)?;
    let dir = parent.join(format!("Roch-Microcode-3202-11F-{}-{}", local_timestamp(), &guid_n()[..6]));
    std::fs::create_dir_all(&dir)?;
    let attempt = || -> Result<BuildPackage> {
        let candidate_dir = dir.join("modified");
        let recovery_dir = dir.join("recovery");
        std::fs::create_dir_all(&candidate_dir)?;
        std::fs::create_dir_all(&recovery_dir)?;
        let candidate = candidate_dir.join(recipe::FLASH_NAME);
        let recovery = recovery_dir.join(recipe::FLASH_NAME);
        write_new_verified(&candidate, &result.image, recipe::RESULT_HASH)?;
        write_new_verified(&recovery, &result.original, recipe::BASE_HASH)?;
        let report = dir.join("validation.json");
        let json = crate::json::to_indented(&ReferenceReport {
            app: "Roch Microcode 1.0.0",
            created_utc: utc_now_iso(),
            board: recipe::BOARD,
            bios: "3202",
            microcode: "11F",
            cpuid: "B0671",
            platform: "32",
            base_sha256: recipe::BASE_HASH,
            donor_patch_sha256: recipe::PATCH_HASH,
            sha256: &result.sha256,
            checks: &result.checks,
            evidence: "User reported BIOS 3202 and MC 11F after boot on one system. Not independent stability testing.",
            limitations: [
                "ASUS capsule signature is invalidated; the image is not re-signed.",
                "FlashBack acceptance and stability on another system are not guaranteed.",
                "11F omits later microcode mitigations. Windows may load newer microcode.",
            ],
        })?;
        std::fs::write(&report, json)?;
        std::fs::write(dir.join("READ-ME.txt"), instructions())?;
        Ok(BuildPackage {
            directory: dir.clone(),
            candidate_path: candidate,
            recovery_path: recovery,
            report_path: report,
        })
    };
    match attempt() {
        Ok(p) => Ok(p),
        Err(e) => {
            // Keep incomplete files for diagnosis, clearly marked; never return them as a completed package.
            let _ = std::fs::write(
                dir.join("INCOMPLETE.txt"),
                "Export did not complete. Do not use these files for flashing.",
            );
            Err(e)
        }
    }
}

/// `READ-ME.txt` of the reference recipe package.
pub fn instructions() -> String {
    [
        "ROCH MICROCODE — ASUS ROG STRIX Z790-A GAMING WIFI D4",
        "",
        "modified/SZ790AD4.CAP = BIOS 3202 with B0671 microcode 11F.",
        "recovery/SZ790AD4.CAP = untouched official BIOS 3202.",
        "These files have the same name. Keep them on separate, clearly labelled USB drives.",
        "",
        "This recipe reproduces the image the user reported flashing and booting successfully on one system.",
        "A successful file check is not a guarantee of boot or long-term stability. Modifying the payload",
        "invalidates the ASUS capsule signature. 11F lacks later microcode mitigations.",
        "",
        "USB BIOS FLASHBACK",
        "1. Use a FAT32 / MBR USB drive with one partition. Save any existing files first.",
        "2. Put the selected SZ790AD4.CAP at the root. Prepare an official recovery drive as well.",
        "3. Save BIOS settings; if BitLocker is enabled, keep the recovery key and suspend protection.",
        "4. Shut down the PC; leave the PSU connected and switched on.",
        "5. Insert USB into the rear port labelled BIOS FlashBack.",
        "6. Hold BIOS FlashBack (not Clear CMOS) for about 3 seconds until the LED blinks 3 times.",
        "7. Wait for the LED to go out. Do not interrupt power, remove USB, power on or clear CMOS.",
        "8. If it boots, check BIOS version and the active microcode in Windows before tuning.",
        "9. Resume BitLocker protection after the process is complete, if applicable.",
        "",
        "A brief flash followed by a solid LED indicates a failed FlashBack attempt.",
        "If flashing completes but the PC cannot boot, use the official recovery image with FlashBack.",
        "Recovery is not guaranteed. Firmware service or an external programmer may be necessary.",
        "",
        "ASUS instructions: https://www.asus.com/us/support/faq/1038568/",
        "This app does not flash firmware, install a driver, change BIOS settings or reboot the PC.",
    ]
    .join("\r\n")
}

/// Map a missing/short image to the same error class the original produced.
pub fn is_rejection(e: &Error) -> bool {
    e.is_invalid_data() || e.is_io()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roch-core-builder-{}", guid_n()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn verified_file_write_preserves_existing() {
        let root = temp_dir();
        let p = root.join("write.bin");
        let b = [1u8, 2, 3, 4];
        write_new_verified(&p, &b, &firmware::hash(&b)).unwrap();
        let second = write_new_verified(&p, &[5, 6], &firmware::hash(&[5, 6]));
        assert!(second.is_err_and(|e| is_rejection(&e)));
        assert_eq!(std::fs::read(&p).unwrap(), b);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn wrong_write_hash_rejected() {
        let root = temp_dir();
        let r = write_new_verified(root.join("bad.bin"), &[1, 2], recipe::PATCH_HASH);
        assert!(r.is_err_and(|e| e.is_invalid_data()));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn recipe_rejects_unknown_images() {
        assert!(build(&[0u8; 64], &[0u8; 64]).is_err_and(|e| e.is_invalid_data()));
        assert!(validate(&[0u8; 64], &[0u8; 64]).is_err_and(|e| e.is_invalid_data()));
    }

    #[test]
    fn iso_timestamp_shape() {
        let t = utc_now_iso();
        assert_eq!(t.len(), "2026-10-29T12:00:00.1234567+00:00".len(), "{t}");
        assert!(t.ends_with("+00:00"));
    }
}
