//! Port of `tests/RochBios.TransferTests`: real cross-vendor BIOS pairs,
//! independent verification and package reload.
//!
//! Usage: `roch-transfer-tests <ASUS-fixture-folder> <cross-vendor-fixture-folder>`
//! Runs the bundled UEFIExtract and MCExtractor. Never writes to USB or flashes.

use std::io::Write;
use std::path::{Path, PathBuf};

use roch_core::firmware::{self, read_image, recipe, Microcode};
use roch_core::input::FirmwareInput;
use roch_core::package::TransferPackage;
use roch_core::profile::FlashProfile;
use roch_core::transfer::{self, can_replace};
use roch_core::CancelToken;
use roch_fixture_tests::{ensure, fail, find_single, ok, reject_invalid, Suite};

struct Case {
    vendor: &'static str,
    board: &'static str,
    old_path: PathBuf,
    new_path: PathBuf,
    flash_name: &'static str,
}

fn best(codes: &[Microcode], cpuid: u32) -> Microcode {
    // OrderByDescending(Revision).First(): the first of the highest revisions.
    let mut best: Option<&Microcode> = None;
    for m in codes.iter().filter(|m| m.cpuid == cpuid && m.checksum_valid) {
        if best.is_none_or(|b| m.revision > b.revision) {
            best = Some(m);
        }
    }
    best.cloned().unwrap_or_else(|| fail(&format!("No valid {cpuid:X} microcode found.")))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        fail("Pass the ASUS research fixture folder and cross-vendor fixture folder.");
    }
    let asus = PathBuf::from(&args[0]);
    let fixtures = std::path::absolute(&args[1]).unwrap_or_else(|_| PathBuf::from(&args[1]));
    let cases = [
        Case {
            vendor: "ASUS",
            board: "ROG STRIX Z790-A GAMING WIFI D4",
            old_path: find_single(&asus, "ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-1904.CAP"),
            new_path: find_single(&asus, "ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-3202.CAP"),
            flash_name: "SZ790AD4.CAP",
        },
        Case {
            vendor: "MSI",
            board: "Z790 MPOWER (7E01)",
            old_path: find_single(&fixtures, "E7E01IMS.P00"),
            new_path: find_single(&fixtures, "E7E01IMS.P90"),
            flash_name: "MSI.ROM",
        },
        Case {
            vendor: "ASRock",
            board: "Z790 Taichi",
            old_path: find_single(&fixtures, "Z790-Taichi_10.01.ROM"),
            new_path: find_single(&fixtures, "Z790-Taichi_20.01.ROM"),
            flash_name: "CREATIVE.ROM",
        },
        Case {
            vendor: "Gigabyte",
            board: "Z790 AORUS ELITE AX rev 1.x",
            old_path: find_single(&fixtures, "Z790AORUSELITEAX11.FF"),
            new_path: find_single(&fixtures, "Z790AORUSELITEAX11.FH"),
            flash_name: "GIGABYTE.bin",
        },
        Case {
            vendor: "MSI",
            board: "PRO Z790-A WIFI DDR4 (7E07)",
            old_path: find_single(&fixtures, "E7E07IMS.190"),
            new_path: find_single(&fixtures, "E7E07IMS.1K0"),
            flash_name: "MSI.ROM",
        },
    ];
    let mut s = Suite::default();
    let output = fixtures.join(format!("transfer-tests-{}", roch_core::format::guid_n()));
    std::fs::create_dir_all(&output).unwrap_or_else(|e| fail(&e.to_string()));
    let load = |p: &Path| FirmwareInput::load(p).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())));
    for case in &cases {
        let (vendor, board) = (case.vendor, case.board);
        let old = load(&case.old_path);
        let newer = load(&case.new_path);
        let donor = best(&old.info.microcodes, 0xb0671);
        let target = best(&newer.info.microcodes, donor.cpuid);
        let fits = transfer::fits(&newer.bytes, &newer.info.microcodes).map(|f| f.len()).unwrap_or(0);
        println!(
            "{vendor} old={}, new={}, raw files={}, FITs={fits}",
            donor.revision_text(),
            target.revision_text(),
            transfer::containers(&newer.bytes).len()
        );
        let built = transfer::build(&newer.bytes, &old.bytes, donor.offset, target.offset)
            .unwrap_or_else(|e| fail(&format!("{vendor} build failed: {e}")));
        let pro = board.starts_with("PRO Z790");
        if pro {
            s.check("MSI 1K real mask change reproduced", || {
                ensure(
                    donor.platforms == 0x32
                        && target.platforms == 0x36
                        && donor.revision == 0x11f
                        && target.revision == 0x137,
                )
            });
            s.check("MSI 1K narrower donor is selectable", || ensure(can_replace(&donor, &target)));
            s.check("MSI 1K result declares donor scope", || {
                ensure(
                    built.platforms == 0x32
                        && built.checks[0].detail.contains("0x36 to 0x32")
                        && built.checks[0].detail.contains("0x04"),
                )
            });
            s.check("MSI 1K preserves every donor byte and mask", || {
                ensure(
                    built.image[target.offset..target.offset + donor.size]
                        == old.bytes[donor.offset..donor.offset + donor.size],
                )
            });
            s.check("MSI 1K fills only unused slot tail", || {
                ensure(built.image[target.offset + donor.size..target.offset + target.size].iter().all(|&b| b == 0xff))
            });
            for flags in [0u32, 0x08, 0x12 | 0x08, 0x3e] {
                s.check(format!("MSI 1K incompatible donor mask {flags:02X} rejected by builder"), || {
                    let mut patch = built.patch.clone();
                    // Keep additive checksums valid so this specifically exercises the mask guard.
                    patch[24..28].copy_from_slice(&flags.to_le_bytes());
                    let sum = firmware::u32_at(&patch, 16).wrapping_add(donor.platforms).wrapping_sub(flags);
                    patch[16..20].copy_from_slice(&sum.to_le_bytes());
                    let inv = firmware::inspect(&patch, "synthetic test donor").microcodes;
                    ensure(inv.len() == 1 && inv[0].checksum_valid)?;
                    reject_invalid(transfer::build(&newer.bytes, &patch, 0, target.offset))
                });
            }
            s.check("subset still rejects invalid checksum", || {
                ensure(!can_replace(&Microcode { checksum_valid: false, ..donor.clone() }, &target))
            });
            s.check("subset still rejects different CPU", || {
                ensure(!can_replace(&Microcode { cpuid: 0xd0670, ..donor.clone() }, &target))
            });
        }
        s.check(format!("{vendor} donor transferred"), || {
            let after = firmware::inspect(&built.image, "result").microcodes;
            let m: Vec<_> = after.iter().filter(|m| m.offset == target.offset).collect();
            ensure(m.len() == 1 && m[0].revision == donor.revision)
        });
        s.check(format!("{vendor} input preserved"), || ensure(firmware::hash(&newer.bytes) == newer.info.sha256));
        s.check(format!("{vendor} repeatable build"), || {
            ensure(ok(transfer::revalidate(&built))?.sha256() == built.sha256())
        });
        s.check(format!("{vendor} mismatched CPU rejected"), || {
            let other = old
                .info
                .microcodes
                .iter()
                .find(|m| m.cpuid != target.cpuid && m.checksum_valid)
                .ok_or("no other CPU microcode")?;
            reject_invalid(transfer::build(&newer.bytes, &old.bytes, other.offset, target.offset))
        });
        s.check(format!("{vendor} same revision rejected"), || {
            reject_invalid(transfer::build(&newer.bytes, &newer.bytes, target.offset, target.offset))
        });
        s.check(format!("{vendor} output tampering rejected"), || {
            let mut changed = built.clone();
            changed.image[200] ^= 1;
            reject_invalid(transfer::revalidate(&changed))
        });
        s.check(format!("{vendor} missing active FIT rejected"), || {
            let mut changed = newer.bytes.clone();
            let fits = ok(transfer::fits(&changed, &newer.info.microcodes))?;
            ensure(fits.len() == 1)?;
            let pointer = (0xffff_ffc0_i64 + fits[0].address_delta) as usize;
            changed[pointer..pointer + 8].fill(0);
            reject_invalid(transfer::build(&changed, &old.bytes, donor.offset, target.offset))
        });
        s.check(format!("{vendor} oversized reverse replacement rejected"), || {
            reject_invalid(transfer::build(&old.bytes, &newer.bytes, target.offset, donor.offset))
        });
        s.check(format!("{vendor} mutated donor rejected"), || {
            let mut patch = built.patch.clone();
            patch[80] ^= 1;
            reject_invalid(transfer::build(&newer.bytes, &patch, 0, target.offset))
        });
        s.check(format!("{vendor} vendor detection"), || ensure(newer.suggested_vendor() == vendor));
        if vendor == "ASUS" {
            s.check("general engine matches working ASUS reference", || ensure(built.sha256() == recipe::RESULT_HASH));
        }
        let profile = FlashProfile::new(vendor, board, case.flash_name);
        let progress = |text: &str| println!("{text}");
        let package = TransferPackage::export(&built, &profile, &output, Some(&progress), &CancelToken::none())
            .unwrap_or_else(|e| fail(&format!("{vendor} export failed: {e}")));
        s.check(format!("{vendor} independent verification and package reload"), || {
            ensure(ok(ok(TransferPackage::open(&package.directory))?.revalidate())?.sha256() == built.sha256())
        });
        s.check(format!("{vendor} report evidence saved"), || {
            let count = std::fs::read_dir(package.directory.join("verification"))
                .map_err(|e| e.to_string())?
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
                .count();
            ensure(count >= 7)
        });
        s.check(format!("{vendor} correct recovery and USB filename"), || {
            ensure(
                package.recovery_path().file_name().is_some_and(|n| n == case.flash_name)
                    && firmware::hash(&ok(read_image(package.recovery_path()))?) == newer.info.sha256,
            )
        });
        if pro {
            s.check("MSI narrower scope survives package reload and README", || {
                let reopened = ok(ok(TransferPackage::open(&package.directory))?.revalidate())?;
                let readme =
                    std::fs::read_to_string(package.directory.join("READ-ME.txt")).map_err(|e| e.to_string())?;
                ensure(reopened.platforms == 0x32 && readme.contains("0x36 to 0x32"))
            });
        }
        s.check(format!("{vendor} package metadata tampering rejected"), || {
            let mut tampered = package.clone();
            tampered.manifest.to_revision = 0;
            reject_invalid(tampered.revalidate())
        });
        s.check(format!("{vendor} filename traversal rejected"), || {
            let mut p = profile.clone();
            p.file_name = "../other.CAP".into();
            reject_invalid(p.validate())
        });
    }
    for zip in ["msi-P0.zip", "msi-P9.zip", "asrock-old.zip", "asrock-new.zip", "gigabyte-ff.zip", "gigabyte-fh.zip"] {
        s.check(format!("{zip} direct ZIP input"), || {
            ensure(!ok(FirmwareInput::load(fixtures.join(zip)))?.info.microcodes.is_empty())
        });
    }
    s.check("reserved Windows filename rejected", || {
        reject_invalid(FlashProfile::new("ASUS", "Test", "COM9.CAP").validate())
    });
    s.check("ambiguous ZIP rejected", || {
        let file = output.join("ambiguous.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).map_err(|e| e.to_string())?);
        for (name, path) in [("old.cap", &cases[0].old_path), ("new.cap", &cases[0].new_path)] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).map_err(|e| e.to_string())?;
            zip.write_all(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        reject_invalid(FirmwareInput::load(&file))
    });
    s.check("FFS checksummed payload repair", || {
        let a = load(&cases[0].new_path);
        let d = load(&cases[0].old_path);
        let single = |codes: &[Microcode]| -> Result<Microcode, String> {
            let v: Vec<_> = codes.iter().filter(|m| m.cpuid == 0xb0671).collect();
            if v.len() == 1 {
                Ok(v[0].clone())
            } else {
                Err("expected one B0671 microcode".into())
            }
        };
        let t = single(&a.info.microcodes)?;
        let donor = single(&d.info.microcodes)?;
        let files: Vec<_> = transfer::containers(&a.bytes)
            .into_iter()
            .filter(|f| t.offset >= f.body() && t.offset + t.size <= f.end())
            .collect();
        ensure(files.len() == 1)?;
        let f = files[0];
        let mut b = a.bytes.clone();
        b[f.offset + 19] |= 0x40;
        let sum = b[f.body()..f.end()].iter().fold(0u8, |s, x| s.wrapping_add(*x));
        b[f.offset + 17] = 0u8.wrapping_sub(sum);
        b[f.offset + 16] = 0;
        let header = (f.offset..f.body())
            .filter(|&i| i != f.offset + 17 && i != f.offset + 23)
            .fold(0u8, |s, i| s.wrapping_add(b[i]));
        b[f.offset + 16] = 0u8.wrapping_sub(header);
        let result = ok(transfer::build(&b, &d.bytes, donor.offset, t.offset))?;
        let sum = result.image[f.body()..f.end()].iter().fold(result.image[f.offset + 17], |s, x| s.wrapping_add(*x));
        ensure(sum == 0 && ok(transfer::revalidate(&result))?.sha256() == result.sha256())
    });
    println!("{} transfer tests passed. Output: {}", s.passed, output.display());
}
