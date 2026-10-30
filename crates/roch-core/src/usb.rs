//! USB destination eligibility and identity policy.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::builder::write_new_verified;
use crate::firmware::{self, read_image};
use crate::format::{decimals, guid_n, local_timestamp};
use crate::package::TransferPackage;
use crate::{report, verification, CancelToken, Error, Progress, Result};

/// Minimum free space for an initial eligibility check (largest supported capsule + 1 MB).
pub const MIN_FREE: i64 = 33_558_528 + 1_048_576;

/// One mounted volume on a USB disk, as reported by the Windows Storage cmdlets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct UsbDrive {
    pub disk_number: i32,
    #[serde(deserialize_with = "null_string")]
    pub unique_id: String,
    #[serde(deserialize_with = "null_string")]
    pub root: String,
    #[serde(deserialize_with = "null_string")]
    pub label: String,
    #[serde(deserialize_with = "null_string")]
    pub bus: String,
    #[serde(deserialize_with = "null_string")]
    pub partition_style: String,
    #[serde(deserialize_with = "null_string")]
    pub file_system: String,
    pub partition_count: i32,
    pub is_boot: bool,
    pub is_system: bool,
    pub size: i64,
    pub free: i64,
    pub offset: i64,
    #[serde(deserialize_with = "null_string")]
    pub volume_id: String,
}

fn null_string<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

impl UsbDrive {
    pub fn display(&self) -> String {
        format!(
            "{}  {}  •  {} GB  •  {} / {}",
            self.root,
            self.label,
            decimals(self.size as f64 / 1_073_741_824.0, 1),
            self.file_system,
            self.partition_style
        )
    }

    /// Disk, volume, drive letter and partition offset must all stay the same.
    pub fn identity(&self) -> String {
        format!("{}|{}|{}|{}|{}", self.disk_number, self.unique_id, self.root, self.offset, self.volume_id)
    }

    /// Why this drive cannot be used, or `None` when it is eligible.
    pub fn rejection(&self) -> Option<&'static str> {
        self.get_rejection(true)
    }

    fn get_rejection(&self, require_space: bool) -> Option<&'static str> {
        let root: Vec<char> = self.root.chars().collect();
        if self.bus != "USB" {
            Some("Only USB disks are supported.")
        } else if self.is_boot || self.is_system {
            Some("System or boot disks cannot be used.")
        } else if self.unique_id.trim().is_empty() || self.volume_id.trim().is_empty() {
            Some("Drive identity could not be verified.")
        } else if root.len() != 3 || !root[0].is_ascii_alphabetic() || root[1] != ':' || root[2] != '\\' {
            Some("A drive letter is required.")
        } else if self.partition_style != "MBR" || self.file_system != "FAT32" || self.partition_count != 1 {
            Some("Use a FAT32 USB drive with MBR and one partition.")
        } else if require_space && self.free < MIN_FREE {
            Some("At least 33 MB free is required.")
        } else {
            None
        }
    }

    /// Re-check a fresh inventory entry against the selected drive.
    pub fn require_same_eligible(&self, fresh: &UsbDrive, require_space: bool) -> Result<()> {
        if fresh.identity() != self.identity() {
            return Err(Error::io("The selected drive changed. Refresh and select it again."));
        }
        if let Some(reason) = fresh.get_rejection(require_space) {
            return Err(Error::io(reason));
        }
        Ok(())
    }
}

fn exists_no_follow(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir())
}

fn is_link(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                // FILE_ATTRIBUTE_REPARSE_POINT
                m.file_attributes() & 0x400 != 0
            }
            #[cfg(not(windows))]
            {
                m.file_type().is_symlink()
            }
        }
        Err(_) => false,
    }
}

/// Copy the modified (or untouched recovery) image of a package to the root
/// of an eligible USB drive.
///
/// The package is rebuilt from its own files and both independent verifiers
/// run again first; saved success text is never trusted. The drive identity
/// is re-checked against a fresh `inventory` before writing and again before
/// committing the staged file. An existing root file is backed up to the
/// local data folder and must be unchanged at commit time. The staged file
/// is written through and flushed, and the result is verified by reading it
/// back. The drive is never formatted and nothing is flashed.
pub fn prepare(
    selected: &UsbDrive,
    package: &TransferPackage,
    recovery: bool,
    inventory: &dyn Fn() -> Result<Vec<UsbDrive>>,
    progress: Progress<'_>,
    ct: &CancelToken,
) -> Result<String> {
    // Rebuild from package bytes and rerun both verifiers; saved success text is not trusted.
    let result = package.revalidate()?;
    let work = crate::paths::new_verification_job();
    verification::verify(&result, &work, progress, ct)?;
    let image: &[u8] = if recovery { &result.original } else { &result.image };
    let hash = firmware::hash(image);
    let flash_name = &package.manifest.profile.file_name;
    let needed = image.len() as i64 + 1_048_576;
    let check_identity = |require_space: bool| -> Result<()> {
        let matches: Vec<UsbDrive> = inventory()?.into_iter().filter(|d| d.identity() == selected.identity()).collect();
        if matches.len() != 1 {
            return Err(Error::io("USB was removed or changed. Refresh drives."));
        }
        let fresh = &matches[0];
        selected.require_same_eligible(fresh, false)?;
        if require_space && fresh.free < needed {
            return Err(Error::io(format!(
                "USB needs at least {} MB free for this image.",
                decimals(needed as f64 / 1_048_576.0, 1)
            )));
        }
        Ok(())
    };
    ct.check()?;
    report(progress, "Checking USB identity, backing up the existing file, then copying and verifying…");
    check_identity(true)?;
    let root = Path::new(&selected.root);
    let target = root.join(flash_name);
    let staged = root.join(format!("ROCH-{}.tmp", guid_n()));
    let attempt = || -> Result<String> {
        let mut backup: Option<PathBuf> = None;
        let mut prior_hash: Option<String> = None;
        if exists_no_follow(&target) {
            if is_link(&target) {
                return Err(Error::io("The existing CAP is a link. Remove it manually before preparing this drive."));
            }
            let previous = read_image(&target)?;
            let previous_hash = firmware::hash(&previous);
            let backups = crate::paths::data_directory().join("USB backups");
            std::fs::create_dir_all(&backups)?;
            let path = backups.join(format!("{}-{}-{}", local_timestamp(), guid_n(), flash_name));
            write_new_verified(&path, &previous, &previous_hash)?;
            prior_hash = Some(previous_hash);
            backup = Some(path);
        }
        write_new_verified(&staged, image, &hash)?;
        check_identity(false)?;
        if exists_no_follow(&target) && is_link(&target) {
            return Err(Error::io("Destination changed into a link. Copy stopped."));
        }
        let current_hash = if exists_no_follow(&target) { Some(firmware::hash(&read_image(&target)?)) } else { None };
        if current_hash != prior_hash {
            return Err(Error::io(
                "The existing USB CAP changed while preparing the drive. No replacement was committed. Try again.",
            ));
        }
        std::fs::rename(&staged, &target)?;
        if firmware::hash(&read_image(&target)?) != hash {
            return Err(Error::io("USB read-back verification failed. Do not flash this file."));
        }
        let what = if recovery {
            "Untouched new BIOS / recovery".to_string()
        } else {
            format!("Modified BIOS / microcode 0x{:X}", result.new_revision)
        };
        let mut message =
            format!("Verified {}\n{what} is ready. Eject the drive in Windows before removing it.", target.display());
        if let Some(b) = backup {
            message.push_str(&format!("\nPrevious BIOS file saved to {}", b.display()));
        }
        Ok(message)
    };
    let outcome = attempt();
    // Do not clean up through a drive letter that may now refer to another disk.
    if check_identity(false).is_ok() && staged.exists() {
        let _ = std::fs::remove_file(&staged);
    }
    outcome
}

#[cfg(test)]
mod tests {
    //! Port of the USB policy part of `tests/RochBios.Tests`.
    use super::*;

    fn good() -> UsbDrive {
        UsbDrive {
            disk_number: 3,
            unique_id: "stable-disk-id".into(),
            root: "R:\\".into(),
            label: "TEST".into(),
            bus: "USB".into(),
            partition_style: "MBR".into(),
            file_system: "FAT32".into(),
            partition_count: 1,
            is_boot: false,
            is_system: false,
            size: 16_000_000_000,
            free: 1_000_000_000,
            offset: 1_048_576,
            volume_id: "volume-id".into(),
        }
    }

    fn rejected(fresh: UsbDrive) -> bool {
        good().require_same_eligible(&fresh, true).is_err_and(|e| e.is_io())
    }

    #[test]
    fn eligible_usb() {
        assert!(good().rejection().is_none());
        good().require_same_eligible(&good(), true).unwrap();
    }
    #[test]
    fn rejections() {
        assert!(rejected(UsbDrive { is_boot: true, ..good() }), "boot");
        assert!(rejected(UsbDrive { is_system: true, ..good() }), "system");
        assert!(rejected(UsbDrive { bus: "SATA".into(), ..good() }), "non USB");
        assert!(rejected(UsbDrive { partition_style: "GPT".into(), ..good() }), "GPT");
        assert!(rejected(UsbDrive { file_system: "NTFS".into(), ..good() }), "NTFS");
        assert!(rejected(UsbDrive { partition_count: 2, ..good() }), "multiple partitions");
        assert!(rejected(UsbDrive { free: 100, ..good() }), "full disk");
        assert!(rejected(UsbDrive { unique_id: "different".into(), ..good() }), "replaced disk");
        assert!(rejected(UsbDrive { volume_id: "different".into(), ..good() }), "replaced volume");
        assert!(rejected(UsbDrive { offset: 0, ..good() }), "changed offset");
        assert!(rejected(UsbDrive { disk_number: 4, ..good() }), "changed disk number");
        assert!(rejected(UsbDrive { root: "S:\\".into(), ..good() }), "changed letter");
    }
    #[test]
    fn reduced_space_allowed_after_staging() {
        good().require_same_eligible(&UsbDrive { free: 100, ..good() }, false).unwrap();
    }
    #[test]
    fn ambiguous_identity_and_invalid_root() {
        assert!(UsbDrive { unique_id: String::new(), ..good() }.rejection().is_some());
        assert!(UsbDrive { volume_id: "  ".into(), ..good() }.rejection().is_some());
        assert!(UsbDrive { root: "R:\\directory".into(), ..good() }.rejection().is_some());
        assert!(UsbDrive { root: "1:\\".into(), ..good() }.rejection().is_some());
    }
    #[test]
    fn inventory_json() {
        let json = r#"[{"DiskNumber":3,"UniqueId":"ID","Root":"E:\\","Label":null,"Bus":"USB","PartitionStyle":"MBR","FileSystem":"FAT32","PartitionCount":1,"IsBoot":false,"IsSystem":false,"Size":16000000000,"Free":15000000000,"Offset":1048576,"VolumeId":"\\\\?\\Volume{x}\\"}]"#;
        let drives: Vec<UsbDrive> = serde_json::from_str(json).unwrap();
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].label, "");
        assert!(drives[0].rejection().is_none());
        assert_eq!(drives[0].display(), "E:\\    •  14.9 GB  •  FAT32 / MBR");
    }
}
