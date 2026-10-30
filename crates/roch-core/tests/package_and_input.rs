//! Package format, input loading and file-safety tests (no real firmware required).

use std::io::Write;
use std::path::{Path, PathBuf};

use roch_core::builder::write_new_verified;
use roch_core::firmware::{self, read_image, MAX_IMAGE_SIZE};
use roch_core::input::FirmwareInput;
use roch_core::package::{PackageManifest, TransferPackage, SCHEMA};
use roch_core::profile::{FlashProfile, M_FLASH};
use roch_core::synthetic::{build_image, sample_pair, SyntheticLayout, SyntheticMicrocode};
use roch_core::transfer::{self, TransferResult};
use roch_core::{CancelToken, Error};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("roch-tests-{tag}-{}", roch_core::format::guid_n()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample_result() -> TransferResult {
    let (older, newer) = sample_pair();
    let d = firmware::inspect(&older.bytes, "o").microcodes[1].clone();
    let t = firmware::inspect(&newer.bytes, "n").microcodes[1].clone();
    transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset).unwrap()
}

/// Write a package with the exact export layout, without running the external verifiers.
fn write_package(dir: &Path, result: &TransferResult, profile: &FlashProfile) -> TransferPackage {
    std::fs::create_dir_all(dir.join("modified")).unwrap();
    std::fs::create_dir_all(dir.join("recovery")).unwrap();
    let manifest = PackageManifest {
        schema: SCHEMA,
        profile: profile.clone(),
        original_hash: firmware::hash(&result.original),
        candidate_hash: result.sha256(),
        patch_hash: firmware::hash(&result.patch),
        target_offset: result.target_offset().unwrap(),
        cpuid: result.cpuid,
        from_revision: result.old_revision,
        to_revision: result.new_revision,
        checks: result.checks.clone(),
    };
    let package = TransferPackage { directory: dir.to_path_buf(), manifest };
    write_new_verified(package.candidate_path(), &result.image, &package.manifest.candidate_hash).unwrap();
    write_new_verified(package.recovery_path(), &result.original, &package.manifest.original_hash).unwrap();
    write_new_verified(package.patch_path(), &result.patch, &package.manifest.patch_hash).unwrap();
    std::fs::write(dir.join("package.json"), roch_core::json::to_indented(&package.manifest).unwrap()).unwrap();
    package
}

fn rejected<T: std::fmt::Debug>(r: roch_core::Result<T>) -> bool {
    match r {
        Err(e) => e.is_invalid_data() || e.is_io(),
        Ok(v) => panic!("unsafe input accepted: {v:?}"),
    }
}

#[test]
fn package_round_trip_and_tampering() {
    let root = temp_root("package");
    let result = sample_result();
    let profile = FlashProfile::new("ASRock", "Z790 Taichi", "CREATIVE.ROM");
    let dir = root.join("pkg");
    let package = write_package(&dir, &result, &profile);
    let opened = TransferPackage::open(&dir).unwrap();
    assert_eq!(opened.manifest, package.manifest);
    assert_eq!(opened.revalidate().unwrap().sha256(), result.sha256());
    assert_eq!(opened.candidate_path().file_name().unwrap(), "CREATIVE.ROM");
    assert_eq!(opened.recovery_path().file_name().unwrap(), "CREATIVE.ROM");

    // Metadata tampering.
    let mut tampered = package.clone();
    tampered.manifest.to_revision = 0;
    assert!(rejected(tampered.revalidate()));
    let mut tampered = package.clone();
    tampered.manifest.cpuid ^= 1;
    assert!(rejected(tampered.revalidate()));
    let mut tampered = package.clone();
    tampered.manifest.schema = 1;
    assert!(rejected(tampered.revalidate()));
    let mut tampered = package.clone();
    tampered.manifest.profile.file_name = "../other.CAP".into();
    assert!(rejected(tampered.revalidate()));
    let mut tampered = package.clone();
    tampered.manifest.target_offset += 4;
    assert!(rejected(tampered.revalidate()));

    // File tampering: hash mismatch.
    let mut bytes = std::fs::read(package.candidate_path()).unwrap();
    bytes[300] ^= 1;
    std::fs::write(package.candidate_path(), &bytes).unwrap();
    assert!(rejected(TransferPackage::open(&dir)));
    bytes[300] ^= 1;
    std::fs::write(package.candidate_path(), &bytes).unwrap();
    TransferPackage::open(&dir).unwrap();

    // A consistent but unreproducible candidate (hash updated in the manifest).
    let mut other = bytes.clone();
    other[200] ^= 1;
    std::fs::write(package.candidate_path(), &other).unwrap();
    let mut manifest = package.manifest.clone();
    manifest.candidate_hash = firmware::hash(&other);
    std::fs::write(dir.join("package.json"), roch_core::json::to_indented(&manifest).unwrap()).unwrap();
    let err = TransferPackage::open(&dir).unwrap_err();
    assert_eq!(err.message(), "Package cannot be reproduced from its original and donor.");
    std::fs::write(package.candidate_path(), &bytes).unwrap();
    std::fs::write(dir.join("package.json"), roch_core::json::to_indented(&package.manifest).unwrap()).unwrap();

    // INCOMPLETE marker.
    std::fs::write(dir.join("INCOMPLETE.txt"), "x").unwrap();
    let err = TransferPackage::open(&dir).unwrap_err();
    assert!(err.message().contains("marked incomplete"));
    std::fs::remove_file(dir.join("INCOMPLETE.txt")).unwrap();

    // Oversized and invalid manifests.
    let manifest_text = std::fs::read_to_string(dir.join("package.json")).unwrap();
    std::fs::write(dir.join("package.json"), format!("{manifest_text}{}", " ".repeat(1024 * 1024))).unwrap();
    assert!(TransferPackage::open(&dir).unwrap_err().message().contains("too large"));
    std::fs::write(dir.join("package.json"), "{ not json").unwrap();
    assert_eq!(TransferPackage::open(&dir).unwrap_err(), Error::invalid("Invalid package manifest."));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn version_2_manifest_compatibility() {
    // A manifest as written by the .NET app for an MSI button package (no Method field):
    let root = temp_root("compat");
    let result = sample_result();
    let dir = root.join("pkg");
    let package = write_package(&dir, &result, &FlashProfile::new("MSI", "Z790 MPOWER (7E01)", "MSI.ROM"));
    let legacy = format!(
        r#"{{
  "Schema": 2,
  "Profile": {{
    "Vendor": "MSI",
    "Board": "Z790 MPOWER (7E01)",
    "FileName": "MSI.ROM",
    "GuideUrl": "https://www.msi.com/support/technical_details/MB_Flash_BIOS_Button",
    "BoardInstructions": "x",
    "Instructions": "ROCH MICROCODE — MSI"
  }},
  "OriginalHash": "{}",
  "CandidateHash": "{}",
  "PatchHash": "{}",
  "TargetOffset": {},
  "Cpuid": {},
  "FromRevision": {},
  "ToRevision": {},
  "Checks": [ {{ "Name": "CPU compatibility", "Detail": "d", "Passed": true }} ]
}}"#,
        package.manifest.original_hash,
        package.manifest.candidate_hash,
        package.manifest.patch_hash,
        package.manifest.target_offset,
        package.manifest.cpuid,
        package.manifest.from_revision,
        package.manifest.to_revision
    );
    std::fs::write(dir.join("package.json"), format!("\u{feff}{legacy}")).unwrap();
    let opened = TransferPackage::open(&dir).unwrap();
    assert_eq!(opened.manifest.profile.method.as_deref(), Some(roch_core::profile::FLASH_BACK));
    assert_eq!(opened.candidate_path().file_name().unwrap(), "MSI.ROM");
    assert!(opened.manifest.profile.guide_url().ends_with("MB_Flash_BIOS_Button"));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn m_flash_package_keeps_original_name() {
    let root = temp_root("mflash");
    let result = sample_result();
    let profile = FlashProfile::new("MSI", "Z790 MPOWER", "E7E01IMS.PA0")
        .with_method(M_FLASH)
        .with_original(Some("E7E01IMS.PA0"));
    let dir = root.join("pkg");
    write_package(&dir, &result, &profile);
    let opened = TransferPackage::open(&dir).unwrap();
    assert_eq!(opened.manifest.profile, profile);
    assert_eq!(opened.candidate_path().file_name().unwrap(), "E7E01IMS.PA0");
    let text = std::fs::read_to_string(dir.join("package.json")).unwrap();
    assert!(text.contains("\"Method\": \"M-FLASH (inside BIOS)\""));
    assert!(text.contains("\"OriginalFileName\": \"E7E01IMS.PA0\""));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn image_read_bounds() {
    let root = temp_root("read");
    let tiny = root.join("tiny.bin");
    std::fs::write(&tiny, [1, 2, 3]).unwrap();
    assert!(rejected(read_image(&tiny)));
    let large = root.join("large.bin");
    std::fs::File::create(&large).unwrap().set_len(MAX_IMAGE_SIZE as u64 + 1).unwrap();
    assert!(rejected(read_image(&large)));
    std::fs::remove_file(&large).unwrap();
    assert!(rejected(read_image(root.join("missing.bin"))));
    let ok = root.join("ok.bin");
    std::fs::write(&ok, [0u8; 48]).unwrap();
    assert_eq!(read_image(&ok).unwrap().len(), 48);
    std::fs::remove_dir_all(root).ok();
}

fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn zip_inputs() {
    let root = temp_root("zip");
    let (older, newer) = sample_pair();
    let filler = vec![0x5au8; 1024 * 1024];
    // Single identifiable image inside folders, alongside a flash utility.
    let single = root.join("vendor.zip");
    zip_with(&single, &[("BIOS/E7E01IMS.P90", &newer.bytes), ("Flash/AFUWINx64.EXE", &filler), ("readme.txt", b"x")]);
    let input = FirmwareInput::load(&single).unwrap();
    assert_eq!(input.file_name, "E7E01IMS.P90");
    assert_eq!(input.bytes, newer.bytes);
    assert_eq!(input.suggested_vendor(), "MSI");
    // Ambiguous: two images with microcode.
    let ambiguous = root.join("ambiguous.zip");
    zip_with(&ambiguous, &[("old.cap", &older.bytes), ("new.cap", &newer.bytes)]);
    let err = FirmwareInput::load(&ambiguous).unwrap_err();
    assert!(err.is_invalid_data() && err.message().contains("more than one BIOS image"));
    // Large entry without microcode.
    let none = root.join("none.zip");
    zip_with(&none, &[("blob.bin", &filler)]);
    assert!(FirmwareInput::load(&none).unwrap_err().message().contains("No uncompressed Intel microcode"));
    // Only small or excluded entries.
    let empty = root.join("empty.zip");
    zip_with(&empty, &[("tool.efi", &filler), ("small.bin", &newer.bytes[..1000])]);
    assert!(FirmwareInput::load(&empty).unwrap_err().message().contains("no clear firmware candidate"));
    // Too many large entries.
    let many = root.join("many.zip");
    let names: Vec<String> = (0..13).map(|i| format!("part{i}.bin")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), filler.as_slice())).collect();
    zip_with(&many, &entries);
    assert!(FirmwareInput::load(&many).unwrap_err().message().contains("too many large entries"));
    // Not a ZIP at all.
    let broken = root.join("broken.zip");
    std::fs::write(&broken, b"not a zip").unwrap();
    assert!(FirmwareInput::load(&broken).unwrap_err().is_invalid_data());
    // Plain file input.
    let plain = root.join("Z790AORUSELITEAX11.FH");
    std::fs::write(&plain, &newer.bytes).unwrap();
    let input = FirmwareInput::load(&plain).unwrap();
    assert_eq!(input.suggested_vendor(), "Gigabyte");
    assert_eq!(input.info.microcodes.len(), 3);
    std::fs::remove_dir_all(root).ok();
}

/// End-to-end export through the real bundled UEFIExtract and MCExtractor.
///
/// Ignored by default because it extracts the verifier bundle into the local
/// data folder and runs the external tools; run with `cargo test -- --ignored`.
#[test]
#[ignore]
fn export_with_bundled_verifiers() {
    let root = temp_root("export");
    let older = build_image(
        &[SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000)],
        &SyntheticLayout { seed: 11, ..Default::default() },
    );
    let newer = build_image(
        &[SyntheticMicrocode::new(0x906a4, 0x80, 0x434, 0x2000), SyntheticMicrocode::new(0xb0671, 0x32, 0x12f, 0x3400)],
        &SyntheticLayout { seed: 12, ..Default::default() },
    );
    let d = firmware::inspect(&older.bytes, "o").microcodes[0].clone();
    let t = firmware::inspect(&newer.bytes, "n").microcodes[1].clone();
    let result = transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset).unwrap();
    let profile = FlashProfile::new("Gigabyte", "Synthetic test board", "GIGABYTE.bin");
    let progress = |s: &str| println!("{s}");
    let package = TransferPackage::export(&result, &profile, &root, Some(&progress), &CancelToken::none())
        .unwrap_or_else(|e| panic!("export failed: {e}"));
    assert!(!package.directory.join("INCOMPLETE.txt").exists());
    assert_eq!(package.manifest.checks.len(), 8);
    let reports: Vec<_> = std::fs::read_dir(package.directory.join("verification")).unwrap().collect();
    assert!(reports.len() >= 7, "{} reports", reports.len());
    for f in ["package.json", "validation.json", "READ-ME.txt", "donor-microcode.bin"] {
        assert!(package.directory.join(f).is_file(), "{f}");
    }
    let readme = std::fs::read_to_string(package.directory.join("READ-ME.txt")).unwrap();
    assert!(readme.starts_with("Primary CPUID B0671; platform mask 0x32 matches exactly."));
    let reopened = TransferPackage::open(&package.directory).unwrap();
    assert_eq!(reopened.revalidate().unwrap().sha256(), result.sha256());

    // Cancellation leaves an INCOMPLETE marker and never a usable package.
    let ct = CancelToken::new();
    ct.cancel();
    let err = TransferPackage::export(&result, &profile, root.join("cancelled"), None, &ct).unwrap_err();
    assert_eq!(err, Error::Cancelled);
    let cancelled: Vec<_> = std::fs::read_dir(root.join("cancelled")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(cancelled.len(), 1);
    assert!(cancelled[0].join("INCOMPLETE.txt").is_file());
    assert!(TransferPackage::open(&cancelled[0]).is_err());
}
