//! Transfer engine tests on synthetic firmware images.
//!
//! These mirror the rules exercised by `tests/RochBios.TransferTests` against
//! real vendor images (which are not redistributable), using small images
//! with the same FIT / FV / FFS / microcode structures.

use roch_core::firmware::{self, Microcode};
use roch_core::synthetic::{build_image, microcode_bytes, sample_pair, SyntheticLayout, SyntheticMicrocode};
use roch_core::transfer::{self, can_replace, compatibility_detail};

fn codes(bytes: &[u8]) -> Vec<Microcode> {
    firmware::inspect(bytes, "test").microcodes
}

fn rejected<T: std::fmt::Debug>(r: roch_core::Result<T>) -> bool {
    match r {
        Err(e) => e.is_invalid_data(),
        Ok(v) => panic!("unsupported modification accepted: {v:?}"),
    }
}

#[test]
fn inventory_and_structures_found() {
    let (older, newer) = sample_pair();
    let n = codes(&newer.bytes);
    assert_eq!(n.len(), 3);
    assert!(n.iter().all(|m| m.checksum_valid));
    assert_eq!(n.iter().map(|m| m.offset).collect::<Vec<_>>(), newer.microcode_offsets);
    assert_eq!(n[1].cpuid, 0xb0671);
    assert_eq!(n[1].revision, 0x12f);
    assert_eq!(n[1].date, "2024-01-29");
    let files = transfer::containers(&newer.bytes);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].offset, newer.ffs_offset);
    let fits = transfer::fits(&newer.bytes, &n).unwrap();
    assert_eq!(fits.len(), 1);
    assert_eq!(fits[0].offset, newer.fit_offset);
    assert_eq!(fits[0].microcode_offsets, newer.microcode_offsets);
    assert_eq!(codes(&older.bytes).len(), 3);
}

#[test]
fn donor_transferred_and_everything_else_preserved() {
    let (older, newer) = sample_pair();
    let d = codes(&older.bytes)[1].clone();
    let t = codes(&newer.bytes)[1].clone();
    let original_hash = firmware::hash(&newer.bytes);
    let built = transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset).unwrap();
    // Donor transferred.
    let after = codes(&built.image);
    assert_eq!(after.iter().find(|m| m.offset == t.offset).unwrap().revision, d.revision);
    assert_eq!(&built.image[t.offset..t.offset + d.size], &older.bytes[d.offset..d.offset + d.size]);
    assert!(built.image[t.offset + d.size..t.offset + t.size].iter().all(|&b| b == 0xff));
    // Input preserved.
    assert_eq!(firmware::hash(&newer.bytes), original_hash);
    assert_eq!(built.original, newer.bytes);
    // Only the slot changed.
    for (i, (a, b)) in newer.bytes.iter().zip(&built.image).enumerate() {
        if a != b {
            assert!(i >= t.offset && i < t.offset + t.size, "unexpected change at {i:#x}");
        }
    }
    assert_eq!(built.old_revision, 0x12f);
    assert_eq!(built.new_revision, 0x11f);
    assert_eq!(built.ranges.len(), 1);
    assert_eq!(built.checks.len(), 6);
    assert_eq!(built.checks[0].name, "CPU compatibility");
    assert!(built.checks[0].detail.contains("platform mask 0x32 matches exactly"));
    assert_eq!(built.checks[5].detail, "All 2 unselected microcodes remain byte-identical");
    // Repeatable build.
    assert_eq!(transfer::revalidate(&built).unwrap().sha256(), built.sha256());
}

#[test]
fn rejection_rules() {
    let (older, newer) = sample_pair();
    let o = codes(&older.bytes);
    let n = codes(&newer.bytes);
    let (d, t) = (&o[1], &n[1]);
    // Mismatched CPU.
    assert!(rejected(transfer::build(&newer.bytes, &older.bytes, o[0].offset, t.offset)));
    // Same revision.
    assert!(rejected(transfer::build(&newer.bytes, &newer.bytes, t.offset, t.offset)));
    // Oversized reverse replacement.
    assert!(rejected(transfer::build(&older.bytes, &newer.bytes, t.offset, d.offset)));
    // Selected microcode no longer exists.
    assert!(rejected(transfer::build(&newer.bytes, &older.bytes, d.offset + 4, t.offset)));
    assert!(rejected(transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset + 4)));
    // Output tampering.
    let built = transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset).unwrap();
    let mut changed = built.clone();
    changed.image[200] ^= 1;
    assert!(rejected(transfer::revalidate(&changed)));
    // Tampering inside the slot padding.
    let mut changed = built.clone();
    changed.image[t.offset + d.size] = 0;
    assert!(rejected(transfer::revalidate(&changed)));
    // Mutated donor (checksum no longer valid).
    let mut patch = built.patch.clone();
    patch[80] ^= 1;
    assert!(rejected(transfer::build(&newer.bytes, &patch, 0, t.offset)));
    // Missing active FIT pointer.
    let fit = transfer::fits(&newer.bytes, &n).unwrap().remove(0);
    let mut no_fit = newer.bytes.clone();
    let pointer = (0xffff_ffc0_i64 + fit.address_delta) as usize;
    no_fit[pointer..pointer + 8].fill(0);
    assert!(rejected(transfer::build(&no_fit, &older.bytes, d.offset, t.offset)));
    // Corrupted FIT header checksum when the checksum-valid bit is set.
    let mut bad_fit = newer.bytes.clone();
    bad_fit[newer.fit_offset + 14] = 0x80;
    bad_fit[newer.fit_offset + 15] = 0x01;
    assert!(rejected(transfer::build(&bad_fit, &older.bytes, d.offset, t.offset)));
    // Invalid original FFS data checksum.
    let mut bad_ffs = newer.bytes.clone();
    bad_ffs[newer.ffs_offset + 17] = 0x55;
    assert!(rejected(transfer::build(&bad_ffs, &older.bytes, d.offset, t.offset)));
}

#[test]
fn ffs_header_corruption_hides_container() {
    let (older, newer) = sample_pair();
    let (d, t) = (codes(&older.bytes)[1].clone(), codes(&newer.bytes)[1].clone());
    let mut bad = newer.bytes.clone();
    bad[newer.ffs_offset + 16] ^= 1;
    assert!(transfer::containers(&bad).is_empty());
    assert!(rejected(transfer::build(&bad, &older.bytes, d.offset, t.offset)));
    // A corrupted volume header checksum is rejected too.
    let mut bad_fv = newer.bytes.clone();
    bad_fv[newer.fv_offset + 50] ^= 1;
    assert!(transfer::containers(&bad_fv).is_empty());
    // Compressed / non-raw file types are never edited.
    let mut not_raw = newer.bytes.clone();
    not_raw[newer.ffs_offset + 18] = 0x07;
    let fixed = 0u8.wrapping_sub(
        (0..24)
            .filter(|&i| i != 16 && i != 17 && i != 23)
            .fold(0u8, |s, i| s.wrapping_add(not_raw[newer.ffs_offset + i])),
    );
    not_raw[newer.ffs_offset + 16] = fixed;
    assert!(transfer::containers(&not_raw).is_empty());
    assert!(rejected(transfer::build(&not_raw, &older.bytes, d.offset, t.offset)));
}

#[test]
fn narrower_donor_mask() {
    let older = build_image(
        &[SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000)],
        &SyntheticLayout { seed: 3, ..Default::default() },
    );
    let newer = build_image(
        &[SyntheticMicrocode::new(0x906a4, 0x80, 0x434, 0x2000), SyntheticMicrocode::new(0xb0671, 0x36, 0x137, 0x3400)],
        &SyntheticLayout { seed: 4, ..Default::default() },
    );
    let d = codes(&older.bytes)[0].clone();
    let t = codes(&newer.bytes)[1].clone();
    assert!(can_replace(&d, &t), "narrower donor is selectable");
    let built = transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset).unwrap();
    assert_eq!(built.platforms, 0x32);
    assert!(built.checks[0].detail.contains("0x36 to 0x32"));
    assert!(built.checks[0].detail.contains("0x04"));
    assert_eq!(built.checks[0].detail, compatibility_detail(&d, &t));
    assert_eq!(&built.image[t.offset..t.offset + d.size], &older.bytes[d.offset..d.offset + d.size]);
    assert!(built.image[t.offset + d.size..t.offset + t.size].iter().all(|&b| b == 0xff));
    // Incompatible donor masks are rejected even with valid additive checksums.
    for flags in [0u32, 0x08, 0x12 | 0x08, 0x3e] {
        let mut patch = built.patch.clone();
        patch[24..28].copy_from_slice(&flags.to_le_bytes());
        let checksum =
            u32::from_le_bytes(patch[16..20].try_into().unwrap()).wrapping_add(d.platforms).wrapping_sub(flags);
        patch[16..20].copy_from_slice(&checksum.to_le_bytes());
        let inv = codes(&patch);
        assert_eq!(inv.len(), 1);
        assert!(inv[0].checksum_valid);
        assert!(rejected(transfer::build(&newer.bytes, &patch, 0, t.offset)), "mask {flags:02X}");
    }
    assert!(!can_replace(&Microcode { checksum_valid: false, ..d.clone() }, &t));
    assert!(!can_replace(&Microcode { cpuid: 0xd0670, ..d.clone() }, &t));
    assert!(!can_replace(&d, &Microcode { checksum_valid: false, ..t.clone() }));
}

#[test]
fn ffs_checksummed_payload_repair() {
    let older = build_image(&[SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000)], &SyntheticLayout::default());
    let newer = build_image(
        &[SyntheticMicrocode::new(0xb0671, 0x32, 0x12f, 0x3400), SyntheticMicrocode::new(0x906a4, 0x80, 0x434, 0x2000)],
        &SyntheticLayout { ffs_data_checksum: true, seed: 5, ..Default::default() },
    );
    let d = codes(&older.bytes)[0].clone();
    let t = codes(&newer.bytes)[0].clone();
    let f = transfer::containers(&newer.bytes)[0];
    assert_eq!(f.attributes & 0x40, 0x40);
    let result = transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset).unwrap();
    let sum = result.image[f.body()..f.end()].iter().fold(result.image[f.offset + 17], |s, x| s.wrapping_add(*x));
    assert_eq!(sum, 0);
    assert_eq!(result.ranges.len(), 2);
    assert_eq!(result.ranges[1].reason, "FFS data checksum");
    assert_eq!(result.ranges[1].offset, f.offset + 17);
    assert_eq!(transfer::revalidate(&result).unwrap().sha256(), result.sha256());
}

#[test]
fn duplicate_targets_are_all_replaced_when_fit_referenced() {
    let older = build_image(&[SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000)], &SyntheticLayout::default());
    let dup = SyntheticMicrocode::new(0xb0671, 0x32, 0x12f, 0x3400);
    let newer = build_image(
        &[dup.clone(), SyntheticMicrocode::new(0x906a4, 0x80, 0x434, 0x2000), dup.clone()],
        &SyntheticLayout { seed: 6, ..Default::default() },
    );
    let d = codes(&older.bytes)[0].clone();
    let n = codes(&newer.bytes);
    let result = transfer::build(&newer.bytes, &older.bytes, d.offset, n[0].offset).unwrap();
    assert_eq!(result.ranges.len(), 2);
    assert!(result.checks[1].detail.starts_with("2 matching slot(s)"));
    let after = codes(&result.image);
    assert_eq!(after[0].revision, 0x11f);
    assert_eq!(after[2].revision, 0x11f);
    assert_eq!(after[1].revision, 0x434);

    // A duplicate that the active FIT does not reference (e.g. a recovery copy) is rejected.
    let unreferenced = build_image(
        &[dup.clone(), SyntheticMicrocode { in_fit: false, ..dup.clone() }],
        &SyntheticLayout { seed: 6, ..Default::default() },
    );
    let n = codes(&unreferenced.bytes);
    let err = transfer::build(&unreferenced.bytes, &older.bytes, d.offset, n[0].offset).unwrap_err();
    assert!(err.message().contains("is not referenced by the active FIT"), "{err}");
}

#[test]
fn target_outside_raw_ffs_is_rejected() {
    let older = build_image(&[SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000)], &SyntheticLayout::default());
    let mut newer = build_image(&[SyntheticMicrocode::new(0xb0671, 0x32, 0x12f, 0x3400)], &SyntheticLayout::default());
    // Move the microcode outside the FFS file and repoint the FIT entry.
    let t = newer.microcode_offsets[0];
    let size = 0x3400;
    let outside = 0xC_0000;
    let code: Vec<u8> = newer.bytes[t..t + size].to_vec();
    newer.bytes[outside..outside + size].copy_from_slice(&code);
    newer.bytes[t..t + size].fill(0);
    let base = 0x1_0000_0000u64 - newer.bytes.len() as u64;
    let entry = newer.fit_offset + 16;
    newer.bytes[entry..entry + 8].copy_from_slice(&(base + outside as u64).to_le_bytes());
    let d = codes(&older.bytes)[0].clone();
    let err = transfer::build(&newer.bytes, &older.bytes, d.offset, outside).unwrap_err();
    assert!(err.message().contains("not inside one validated, uncompressed raw FFS file"), "{err}");
}

#[test]
fn invalid_checksums_and_oem_aliases() {
    let m = SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x2000);
    let mut bytes = microcode_bytes(&m, 1);
    assert!(codes(&bytes)[0].checksum_valid);
    bytes[100] ^= 1;
    let found = codes(&bytes);
    assert!(!found[0].checksum_valid);
    assert_eq!(found[0].status(), "Invalid checksum");
    // The ASUS alternate-header exemption only applies to the pinned official images.
    let alias = microcode_bytes(&SyntheticMicrocode::new(0xff0671, 0x32, 0x104, 0x2000), 1);
    let mut alias_bad = alias.clone();
    alias_bad[200] ^= 1;
    assert!(!codes(&alias_bad)[0].known_oem_header);
    // Invalid date, zero CPUID, oversized platform flags and odd sizes are not microcodes.
    let mut no_date = microcode_bytes(&m, 1);
    no_date[8..12].copy_from_slice(&0x1301_2024u32.to_le_bytes());
    assert!(codes(&no_date).is_empty());
    let mut no_cpu = microcode_bytes(&m, 1);
    no_cpu[12..16].fill(0);
    assert!(codes(&no_cpu).is_empty());
    let mut flags = microcode_bytes(&m, 1);
    flags[24..28].copy_from_slice(&0x100u32.to_le_bytes());
    assert!(codes(&flags).is_empty());
    let mut odd = microcode_bytes(&m, 1);
    odd[32..36].copy_from_slice(&0x2100u32.to_le_bytes());
    assert!(codes(&odd).is_empty());
}
