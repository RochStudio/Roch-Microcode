//! Version 2 BIOS package format: export, open and revalidation.
//!
//! Layout (unchanged from the original app):
//! `modified/<filename>`, `recovery/<filename>`, `donor-microcode.bin`,
//! `package.json`, `validation.json`, `verification/*.txt` and `READ-ME.txt`.
//! While an export is in progress, or after it fails, `INCOMPLETE.txt` marks
//! the folder and opening it is refused.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::builder::write_new_verified;
use crate::error::require;
use crate::firmware::{self, read_image};
use crate::format::{guid_n, local_timestamp};
use crate::profile::FlashProfile;
use crate::transfer::{self, ChangeRange, TransferResult, ValidationCheck};
use crate::{verification, CancelToken, Error, Progress, Result};

pub const SCHEMA: i32 = 2;
pub const INCOMPLETE: &str = "INCOMPLETE.txt";
pub const MANIFEST: &str = "package.json";
pub const VALIDATION: &str = "validation.json";
pub const README: &str = "READ-ME.txt";
pub const PATCH_FILE: &str = "donor-microcode.bin";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageManifest {
    #[serde(default)]
    pub schema: i32,
    pub profile: FlashProfile,
    #[serde(default)]
    pub original_hash: String,
    #[serde(default)]
    pub candidate_hash: String,
    #[serde(default)]
    pub patch_hash: String,
    #[serde(default)]
    pub target_offset: usize,
    #[serde(default)]
    pub cpuid: u32,
    #[serde(default)]
    pub from_revision: u32,
    #[serde(default)]
    pub to_revision: u32,
    #[serde(default)]
    pub checks: Vec<ValidationCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferPackage {
    pub directory: PathBuf,
    pub manifest: PackageManifest,
}

#[derive(Serialize)]
struct ValidationReport<'a> {
    manifest: &'a PackageManifest,
    ranges: &'a [ChangeRange],
    limitations: &'a str,
}

impl TransferPackage {
    pub fn candidate_path(&self) -> PathBuf {
        self.directory.join("modified").join(&self.manifest.profile.file_name)
    }
    pub fn recovery_path(&self) -> PathBuf {
        self.directory.join("recovery").join(&self.manifest.profile.file_name)
    }
    pub fn patch_path(&self) -> PathBuf {
        self.directory.join(PATCH_FILE)
    }

    /// Rebuild the modified image from the package's own recovery image and
    /// donor patch; hashes, bytes and metadata must all agree.
    pub fn revalidate(&self) -> Result<TransferResult> {
        self.manifest.profile.validate()?;
        require(self.manifest.schema == SCHEMA, "Unsupported package format.")?;
        let original = read_image(self.recovery_path())?;
        let candidate = read_image(self.candidate_path())?;
        let patch = read_image(self.patch_path())?;
        require(
            firmware::hash(&original) == self.manifest.original_hash
                && firmware::hash(&candidate) == self.manifest.candidate_hash
                && firmware::hash(&patch) == self.manifest.patch_hash,
            "Package file hashes have changed.",
        )?;
        let result = transfer::build(&original, &patch, 0, self.manifest.target_offset)?;
        require(result.image == candidate, "Package cannot be reproduced from its original and donor.")?;
        require(
            result.cpuid == self.manifest.cpuid
                && result.old_revision == self.manifest.from_revision
                && result.new_revision == self.manifest.to_revision,
            "Package metadata disagrees with the firmware.",
        )?;
        Ok(result)
    }

    /// Open and fully revalidate an existing version 2 package folder.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        require(
            !dir.join(INCOMPLETE).exists(),
            "This package is marked incomplete and cannot be prepared for flashing.",
        )?;
        let manifest = dir.join(MANIFEST);
        let len = std::fs::metadata(&manifest)?.len();
        require(len < 1024 * 1024, "Package manifest is too large.")?;
        let bytes = std::fs::read(&manifest)?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let data: PackageManifest =
            serde_json::from_str(text).map_err(|_| Error::invalid("Invalid package manifest."))?;
        let package = TransferPackage { directory: dir.to_path_buf(), manifest: data };
        package.revalidate()?;
        Ok(package)
    }

    /// Verify independently, then write a complete package. The folder is
    /// marked incomplete until every file is written and the package reloads.
    pub fn export(
        result: &TransferResult,
        profile: &FlashProfile,
        parent: impl AsRef<Path>,
        progress: Progress<'_>,
        ct: &CancelToken,
    ) -> Result<Self> {
        profile.validate()?;
        let result = transfer::revalidate(result)?;
        let parent = parent.as_ref();
        std::fs::create_dir_all(parent)?;
        let dir = parent.join(format!(
            "Roch-Microcode-{}-{:X}-{}-{}",
            profile.vendor,
            result.new_revision,
            local_timestamp(),
            guid_n()
        ));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(INCOMPLETE), "Verification is incomplete. Do not flash these files.")?;
        let work = crate::paths::new_verification_job();
        let attempt = || -> Result<Self> {
            let evidence = verification::verify(&result, &work, progress, ct)?;
            ct.check()?;
            let manifest = PackageManifest {
                schema: SCHEMA,
                profile: profile.clone(),
                original_hash: firmware::hash(&result.original),
                candidate_hash: result.sha256(),
                patch_hash: firmware::hash(&result.patch),
                target_offset: result.target_offset()?,
                cpuid: result.cpuid,
                from_revision: result.old_revision,
                to_revision: result.new_revision,
                checks: result.checks.iter().chain(evidence.checks.iter()).cloned().collect(),
            };
            let package = TransferPackage { directory: dir.clone(), manifest };
            for p in [package.candidate_path(), package.recovery_path()] {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            write_new_verified(package.candidate_path(), &result.image, &package.manifest.candidate_hash)?;
            write_new_verified(package.recovery_path(), &result.original, &package.manifest.original_hash)?;
            write_new_verified(package.patch_path(), &result.patch, &package.manifest.patch_hash)?;
            let reports = dir.join("verification");
            std::fs::create_dir_all(&reports)?;
            let mut logs: Vec<PathBuf> = std::fs::read_dir(&work)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file() && p.extension().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("txt"))
                })
                .collect();
            logs.sort();
            for file in logs {
                let target = reports.join(file.file_name().expect("file name"));
                require(!target.exists(), "A verification report already exists in the package.")?;
                std::fs::copy(&file, &target)?;
            }
            let compatibility = result
                .checks
                .iter()
                .find(|c| c.name == "CPU compatibility")
                .map(|c| c.detail.clone())
                .ok_or_else(|| Error::invalid("CPU compatibility check is missing."))?;
            std::fs::write(dir.join(README), format!("{compatibility}\n\n{}", profile.instructions()))?;
            std::fs::write(dir.join(MANIFEST), crate::json::to_indented(&package.manifest)?)?;
            std::fs::write(
                dir.join(VALIDATION),
                crate::json::to_indented(&ValidationReport {
                    manifest: &package.manifest,
                    ranges: &result.ranges,
                    limitations: "Structural/integrity verification only. Vendor signatures, rollback/Boot Guard acceptance and hardware stability are not established.",
                })?,
            )?;
            package.revalidate()?;
            std::fs::remove_file(dir.join(INCOMPLETE))?;
            Ok(package)
        };
        match attempt() {
            Ok(p) => Ok(p),
            Err(e) => {
                let _ = std::fs::write(
                    dir.join(INCOMPLETE),
                    format!(
                        "Verification/export failed. Do not flash this folder. Diagnostic work: {}",
                        work.display()
                    ),
                );
                Err(e)
            }
        }
    }

    /// Same package with different checks (used after re-verification on load).
    pub fn with_checks(&self, checks: Vec<ValidationCheck>) -> Self {
        let mut p = self.clone();
        p.manifest.checks = checks;
        p
    }
}
