//! Port of `tests/RochBios.Tests`: USB policy, file safety and the pinned ASUS
//! reference recipe, plus optional pinned official downloads.
//!
//! Usage: `roch-tests <official-3202.CAP> <official-1904.CAP> [--online]`

use roch_core::builder::{self, BuildResult};
use roch_core::downloads;
use roch_core::firmware::{self, read_image, recipe, MAX_IMAGE_SIZE};
use roch_core::usb::UsbDrive;
use roch_core::{CancelToken, Error};
use roch_fixture_tests::{ensure, fail, ok, reject_any, temp_root, Suite};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut s = Suite::default();
    let root = temp_root("roch-bios-tests");
    let good = UsbDrive {
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
    };
    let with = |f: &dyn Fn(&mut UsbDrive)| {
        let mut d = good.clone();
        f(&mut d);
        d
    };
    s.check("eligible USB", || {
        ensure(good.rejection().is_none())?;
        ok(good.require_same_eligible(&good, true))
    });
    s.check("reject boot USB", || reject_any(good.require_same_eligible(&with(&|d| d.is_boot = true), true)));
    s.check("reject system USB", || reject_any(good.require_same_eligible(&with(&|d| d.is_system = true), true)));
    s.check("reject non USB", || reject_any(good.require_same_eligible(&with(&|d| d.bus = "SATA".into()), true)));
    s.check("reject GPT", || {
        reject_any(good.require_same_eligible(&with(&|d| d.partition_style = "GPT".into()), true))
    });
    s.check("reject NTFS", || reject_any(good.require_same_eligible(&with(&|d| d.file_system = "NTFS".into()), true)));
    s.check("reject multiple partitions", || {
        reject_any(good.require_same_eligible(&with(&|d| d.partition_count = 2), true))
    });
    s.check("reject full disk", || reject_any(good.require_same_eligible(&with(&|d| d.free = 100), true)));
    s.check("allow reduced space after staging", || ok(good.require_same_eligible(&with(&|d| d.free = 100), false)));
    s.check("reject replaced disk", || {
        reject_any(good.require_same_eligible(&with(&|d| d.unique_id = "different".into()), true))
    });
    s.check("reject replaced volume", || {
        reject_any(good.require_same_eligible(&with(&|d| d.volume_id = "different".into()), true))
    });
    s.check("reject changed offset", || reject_any(good.require_same_eligible(&with(&|d| d.offset = 0), true)));
    s.check("reject ambiguous identity", || ensure(with(&|d| d.unique_id = String::new()).rejection().is_some()));
    s.check("reject invalid root", || ensure(with(&|d| d.root = "R:\\directory".into()).rejection().is_some()));
    s.check("empty image scanner", || ensure(firmware::inspect(&[], "empty").microcodes.is_empty()));
    s.check("truncated image read", || {
        let p = root.join("tiny.bin");
        std::fs::write(&p, [1, 2, 3]).map_err(|e| e.to_string())?;
        reject_any(read_image(&p))
    });
    s.check("oversized image bounded", || {
        let p = root.join("large.bin");
        std::fs::File::create(&p).and_then(|f| f.set_len(MAX_IMAGE_SIZE as u64 + 1)).map_err(|e| e.to_string())?;
        reject_any(read_image(&p))?;
        std::fs::remove_file(&p).map_err(|e| e.to_string())
    });
    s.check("verified file write preserves existing", || {
        let b = [1u8, 2, 3, 4];
        let p = root.join("write.bin");
        ok(builder::write_new_verified(&p, &b, &firmware::hash(&b)))?;
        reject_any(builder::write_new_verified(&p, &[5, 6], &firmware::hash(&[5, 6])))?;
        ensure(std::fs::read(&p).map_err(|e| e.to_string())? == b)
    });
    s.check("wrong write hash rejected", || {
        reject_any(builder::write_new_verified(root.join("bad.bin"), &[1, 2], recipe::PATCH_HASH))
    });
    if args.iter().filter(|a| !a.starts_with("--")).count() < 2 {
        fail("Pass original 3202 CAP and donor 1904 CAP paths to run required firmware integration tests.");
    }
    let original = read_image(&args[0]).unwrap_or_else(|e| fail(&format!("{}: {e}", args[0])));
    let donor = read_image(&args[1]).unwrap_or_else(|e| fail(&format!("{}: {e}", args[1])));
    let mut built: Option<BuildResult> = None;
    s.check("exact official fixture hashes", || {
        ensure(firmware::hash(&original) == recipe::BASE_HASH && firmware::hash(&donor) == recipe::DONOR_HASH)
    });
    s.check("official MCU inventory", || {
        let i = firmware::inspect(&original, "3202");
        ensure(i.microcodes.len() == 4)?;
        ensure(i.microcodes[2].revision == 0x133)?;
        ensure(i.microcodes[3].known_oem_header)
    });
    s.check("build byte-identical working reference", || {
        let b = ok(builder::build(&original, &donor))?;
        ensure(b.sha256 == recipe::RESULT_HASH)?;
        ensure(b.checks.len() == 7)?;
        built = Some(b);
        Ok(())
    });
    let built = built.expect("built");
    s.check("source buffers remain unchanged", || {
        ensure(firmware::hash(&original) == recipe::BASE_HASH && firmware::hash(&donor) == recipe::DONOR_HASH)
    });
    s.check("only permitted slot changes", || {
        let mut changed = 0usize;
        for (i, (a, b)) in original.iter().zip(&built.image).enumerate() {
            if a != b {
                ensure((recipe::START..recipe::START + recipe::OLD_SIZE).contains(&i))?;
                changed += 1;
            }
        }
        ensure(changed > 0)
    });
    s.check("raw MCU donor supported", || {
        let raw = &donor[recipe::DONOR_START..recipe::DONOR_START + recipe::NEW_SIZE];
        ensure(ok(builder::build(&original, raw))?.sha256 == recipe::RESULT_HASH)
    });
    s.check("wrong motherboard/source rejected", || reject_any(builder::build(&donor, &donor)));
    s.check("wrong donor rejected", || reject_any(builder::build(&original, &original)));
    s.check("source tampering rejected", || {
        let mut b = original.clone();
        b[100] ^= 1;
        reject_any(builder::build(&b, &donor))
    });
    s.check("patch tampering rejected", || {
        let mut b = built.image.clone();
        b[recipe::START + 48] ^= 1;
        reject_any(builder::validate(&original, &b))
    });
    s.check("outside slot edit rejected", || {
        let mut b = built.image.clone();
        b[100] ^= 1;
        reject_any(builder::validate(&original, &b))
    });
    s.check("FIT relocation rejected", || {
        let mut b = built.image.clone();
        b[0x1091140] ^= 1;
        reject_any(builder::validate(&original, &b))
    });
    s.check("padding change rejected", || {
        let mut b = built.image.clone();
        b[recipe::START + recipe::NEW_SIZE] = 0;
        reject_any(builder::validate(&original, &b))
    });
    s.check("truncated build rejected", || {
        reject_any(builder::validate(&original, &built.image[..built.image.len() - 1]))
    });
    s.check("unknown file gets no ASUS header exemption", || {
        let mut b = original.clone();
        b[100] ^= 1;
        ensure(!firmware::inspect(&b, "unknown").microcodes[3].known_oem_header)
    });
    s.check("export contains recovery and report", || {
        let p = ok(builder::export(&built, &root))?;
        ensure(firmware::hash(&ok(read_image(&p.candidate_path))?) == recipe::RESULT_HASH)?;
        ensure(firmware::hash(&ok(read_image(&p.recovery_path))?) == recipe::BASE_HASH)?;
        ensure(p.report_path.is_file() && p.directory.join("READ-ME.txt").is_file())
    });
    s.check("export boundary catches mutation", || {
        let mut b = built.image.clone();
        b[0] ^= 1;
        reject_any(builder::export(&BuildResult { image: b, ..built.clone() }, &root))
    });
    if args.iter().any(|a| a == "--online") {
        let cache = root.join("downloads");
        let none = CancelToken::none();
        for spec in [recipe::base_download(), recipe::donor_download()] {
            let path = downloads::fetch(&spec, &cache, None, &none).unwrap_or_else(|e| fail(&format!("download: {e}")));
            s.check(format!("official {} download and extraction", spec.version), || {
                ensure(firmware::hash(&ok(read_image(&path))?) == spec.cap_hash)
            });
            let cached = downloads::fetch(&spec, &cache, None, &none).unwrap_or_else(|e| fail(&format!("cache: {e}")));
            s.check(format!("verified {} cache", spec.version), || ensure(cached == path));
        }
        let donor_spec = recipe::donor_download();
        std::fs::write(cache.join(donor_spec.cap_name()), [1, 2, 3]).unwrap_or_else(|e| fail(&e.to_string()));
        let repaired =
            downloads::fetch(&donor_spec, &cache, None, &none).unwrap_or_else(|e| fail(&format!("repair: {e}")));
        s.check("truncated cache repaired from verified download", || {
            ensure(firmware::hash(&ok(read_image(&repaired))?) == recipe::DONOR_HASH)
        });
        let rejected_cache = root.join("rejected-download");
        let wrong = recipe::DownloadSpec { zip_hash: recipe::BASE_HASH.into(), ..recipe::donor_download() };
        match downloads::fetch(&wrong, &rejected_cache, None, &none) {
            Err(Error::InvalidData(_)) => s.check("wrong download hash rejected without accepting CAP", || {
                ensure(std::fs::read_dir(&rejected_cache).map(|d| d.count() == 0).unwrap_or(false))
            }),
            other => fail(&format!("Wrong ZIP hash accepted: {other:?}")),
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        match downloads::fetch(&recipe::base_download(), &cache, None, &cancel) {
            Err(Error::Cancelled) => s.check("cancellation", || Ok(())),
            other => fail(&format!("Cancellation ignored: {other:?}")),
        }
    }
    println!("{} tests passed. Evidence and isolated output: {}", s.passed, root.display());
}
