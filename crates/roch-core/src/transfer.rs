//! General microcode transfer engine: FV/FFS container discovery, FIT
//! resolution, the transfer builder and its revalidation.

use serde::{Deserialize, Serialize};

use crate::error::require;
use crate::firmware::{self, u16_at, u32_at, u64_at, Microcode, MAX_IMAGE_SIZE};
use crate::format::n0;
use crate::{Error, Result};

/// Reason string of the primary change range; also used to locate the target slot.
pub const SLOT_REASON: &str = "Microcode slot and FF padding";
pub const FFS_CHECKSUM_REASON: &str = "FFS data checksum";

/// A validated raw (EFI_FV_FILETYPE_RAW) FFS file whose body may be edited in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfsContainer {
    pub offset: usize,
    pub header_size: usize,
    pub size: usize,
    pub attributes: u8,
}

impl FfsContainer {
    pub fn body(&self) -> usize {
        self.offset + self.header_size
    }
    pub fn end(&self) -> usize {
        self.offset + self.size
    }
}

/// An active Firmware Interface Table whose microcode entries resolve to
/// microcodes found in the image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FitTable {
    pub offset: usize,
    pub size: usize,
    pub address_delta: i64,
    pub microcode_offsets: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChangeRange {
    pub offset: usize,
    pub size: usize,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ValidationCheck {
    pub name: String,
    pub detail: String,
    #[serde(default = "default_true")]
    pub passed: bool,
}

fn default_true() -> bool {
    true
}

impl ValidationCheck {
    pub fn new(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), detail: detail.into(), passed: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferResult {
    /// Untouched new (target) BIOS.
    pub original: Vec<u8>,
    /// Exact donor microcode bytes.
    pub patch: Vec<u8>,
    /// Modified image.
    pub image: Vec<u8>,
    pub cpuid: u32,
    pub platforms: u32,
    pub old_revision: u32,
    pub new_revision: u32,
    pub ranges: Vec<ChangeRange>,
    pub checks: Vec<ValidationCheck>,
}

impl TransferResult {
    pub fn sha256(&self) -> String {
        firmware::hash(&self.image)
    }
    /// Offset of the (first) replaced microcode slot.
    pub fn target_offset(&self) -> Result<usize> {
        self.ranges
            .iter()
            .find(|r| r.reason == SLOT_REASON)
            .map(|r| r.offset)
            .ok_or_else(|| Error::invalid("The transfer has no replaced microcode slot."))
    }
}

/// A platform mask is a set of platform IDs, not an indivisible identifier.
/// A narrower donor is usable only within its own declared primary-CPU scope.
/// Do not accept arbitrary overlap or infer compatibility from extended aliases.
pub fn can_replace(donor: &Microcode, target: &Microcode) -> bool {
    donor.checksum_valid
        && target.checksum_valid
        && donor.cpuid == target.cpuid
        && donor.platforms != 0
        && (donor.platforms & target.platforms) == donor.platforms
}

pub fn compatibility_detail(donor: &Microcode, target: &Microcode) -> String {
    if donor.platforms == target.platforms {
        format!(
            "Primary CPUID {:X}; platform mask 0x{:02X} matches exactly. Extended CPU coverage is not guaranteed.",
            donor.cpuid, donor.platforms
        )
    } else {
        format!(
            "Primary CPUID {:X}; platform mask narrows from 0x{:02X} to 0x{:02X}. Use only for that CPU signature on a platform covered by 0x{:02X}. This patch no longer covers platform bits 0x{:02X}. Extended CPU coverage is not guaranteed.",
            donor.cpuid,
            target.platforms,
            donor.platforms,
            donor.platforms,
            target.platforms & !donor.platforms
        )
    }
}

fn size24(b: &[u8], p: usize) -> usize {
    b[p] as usize | (b[p + 1] as usize) << 8 | (b[p + 2] as usize) << 16
}
fn align8(p: usize) -> usize {
    (p + 7) & !7
}
pub(crate) fn sum8(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |s, x| s.wrapping_add(*x))
}

/// EFI_FIRMWARE_FILE_SYSTEM2_GUID 8C8CE578-8A3D-4F1C-9935-896185C32DD3.
const FFS2_GUID: [u8; 16] =
    [0x78, 0xE5, 0x8C, 0x8C, 0x3D, 0x8A, 0x1C, 0x4F, 0x99, 0x35, 0x89, 0x61, 0x85, 0xC3, 0x2D, 0xD3];
/// EFI_FIRMWARE_FILE_SYSTEM3_GUID 5473C07A-3DCB-4DCA-BD6F-1E9689E7349A.
const FFS3_GUID: [u8; 16] =
    [0x7A, 0xC0, 0x73, 0x54, 0xCB, 0x3D, 0xCA, 0x4D, 0xBD, 0x6F, 0x1E, 0x96, 0x89, 0xE7, 0x34, 0x9A];

/// Every validated raw FFS file inside every validated FFS2/FFS3 firmware volume.
pub fn containers(b: &[u8]) -> Vec<FfsContainer> {
    let mut files: Vec<FfsContainer> = Vec::new();
    let len = b.len();
    if len < 72 {
        return files;
    }
    let mut p = 0usize;
    while p <= len - 72 {
        let fv = p;
        p += 8;
        if &b[fv + 40..fv + 44] != b"_FVH" || b[fv + 55] != 2 {
            continue;
        }
        let length = u64_at(b, fv + 32);
        let header = u16_at(b, fv + 48) as u64;
        if length > (len - fv) as u64 || length < 72 || header < 72 || header > length || header & 1 != 0 {
            continue;
        }
        let mut sum = 0u32;
        let mut n = 0usize;
        while n < header as usize {
            sum = sum.wrapping_add(u16_at(b, fv + n) as u32);
            n += 2;
        }
        if sum & 0xffff != 0 {
            continue;
        }
        let fs = &b[fv + 16..fv + 32];
        if fs != FFS2_GUID && fs != FFS3_GUID {
            continue;
        }
        let end = fv + length as usize;
        let mut first = fv + header as usize;
        let ext = u16_at(b, fv + 52) as u64;
        if ext != 0 {
            if ext > length - 20 {
                continue;
            }
            let ext_size = u32_at(b, fv + ext as usize + 16) as u64;
            if ext_size < 20 || ext_size > length - ext {
                continue;
            }
            first = first.max(fv + (ext + ext_size) as usize);
        }
        let mut f = align8(first);
        while end >= 24 && f <= end - 24 {
            let head = &b[f..f + 24];
            if head.iter().all(|&x| x == 0xff) || head.iter().all(|&x| x == 0) {
                break;
            }
            let mut h = 24usize;
            let mut size = size24(b, f + 20);
            if size == 0xffffff {
                if b[f + 19] & 1 == 0 || end < 32 || f > end - 32 {
                    break;
                }
                let big = u64_at(b, f + 24);
                if big > (end - f) as u64 {
                    break;
                }
                h = 32;
                size = big as usize;
            }
            if size < h || size > end - f {
                break;
            }
            let valid_header = sum8(&b[f..f + h]).wrapping_sub(b[f + 17]).wrapping_sub(b[f + 23]) == 0;
            // Only direct raw FFS bodies are edited. No compressed or guided sections are rebuilt.
            if valid_header && b[f + 18] == 1 {
                let c = FfsContainer { offset: f, header_size: h, size, attributes: b[f + 19] };
                if !files.contains(&c) {
                    files.push(c);
                }
            }
            f = align8(f + size);
        }
    }
    files
}

/// Active FIT tables whose microcode pointers all resolve to discovered microcodes.
pub fn fits(b: &[u8], codes: &[Microcode]) -> Result<Vec<FitTable>> {
    let mut found = Vec::new();
    let len = b.len();
    if len < 32 {
        return Ok(found);
    }
    let mut p = 0usize;
    while p <= len - 32 {
        let at = p;
        p += 16;
        if &b[at..at + 8] != b"_FIT_   " {
            continue;
        }
        let count = (u32_at(b, at + 8) & 0xffffff) as usize;
        if !(2..=4096).contains(&count) || count * 16 > len - at || b[at + 14] & 127 != 0 {
            continue;
        }
        let size = count * 16;
        if b[at + 14] & 128 != 0 && sum8(&b[at..at + size]) != 0 {
            continue;
        }
        let addresses: Vec<u64> =
            (1..count).filter(|i| b[at + i * 16 + 14] & 127 == 1).map(|i| u64_at(b, at + i * 16)).collect();
        if addresses.is_empty() || addresses.iter().any(|&a| a > u32::MAX as u64 || a < 0x8000_0000) {
            continue;
        }
        let mut matches = Vec::new();
        for c in codes {
            let delta = c.offset as i64 - addresses[0] as i64;
            let pointer = 0xffff_ffc0_i64 + delta;
            if pointer < 0 || pointer > len as i64 - 8 || u64_at(b, pointer as usize) != (at as i64 - delta) as u64 {
                continue;
            }
            let offsets: Vec<i64> = addresses.iter().map(|&a| a as i64 + delta).collect();
            if offsets.iter().all(|&o| codes.iter().any(|m| m.offset as i64 == o)) {
                matches.push(FitTable {
                    offset: at,
                    size,
                    address_delta: delta,
                    microcode_offsets: offsets.iter().map(|&o| o as usize).collect(),
                });
            }
        }
        require(matches.len() <= 1, "The FIT address mapping is ambiguous.")?;
        found.extend(matches);
    }
    Ok(found)
}

fn single_at(codes: &[Microcode], offset: usize, missing: &str) -> Result<Microcode> {
    let mut it = codes.iter().filter(|m| m.offset == offset);
    match (it.next(), it.next()) {
        (Some(m), None) => Ok(m.clone()),
        (None, _) => Err(Error::invalid(missing)),
        _ => Err(Error::invalid("Microcode offsets are ambiguous.")),
    }
}

/// Copy the donor microcode at `donor_offset` in `older` over every matching
/// target slot (same CPUID, platform mask and revision as the target at
/// `target_offset`) in `newer`, then prove that nothing else changed.
pub fn build(newer: &[u8], older: &[u8], donor_offset: usize, target_offset: usize) -> Result<TransferResult> {
    require(newer.len() <= MAX_IMAGE_SIZE && older.len() <= MAX_IMAGE_SIZE, "Image exceeds 128 MB.")?;
    let target_codes = firmware::inspect(newer, "new BIOS").microcodes;
    let donor_codes = firmware::inspect(older, "old BIOS").microcodes;
    let donor = single_at(&donor_codes, donor_offset, "Selected old microcode no longer exists.")?;
    let target = single_at(&target_codes, target_offset, "Selected new microcode no longer exists.")?;
    require(
        donor.checksum_valid && target.checksum_valid,
        "Selected microcode checksum is invalid. Modified OEM aliases cannot be used as the donor or target.",
    )?;
    require(
        can_replace(&donor, &target),
        "Primary CPU signatures must match and the donor's nonzero platform mask must be equal to or a subset of the target mask. Partial overlap is not supported.",
    )?;
    require(donor.revision != target.revision, "The selected revisions are already the same.")?;
    let targets: Vec<Microcode> = target_codes
        .iter()
        .filter(|m| m.cpuid == target.cpuid && m.platforms == target.platforms && m.revision == target.revision)
        .cloned()
        .collect();
    let files = containers(newer);
    let fit_tables = fits(newer, &target_codes)?;
    require(
        !fit_tables.is_empty(),
        "No unambiguous active Intel FIT table was found. This firmware layout is not supported for editing.",
    )?;
    let patch = older[donor.offset..donor.offset + donor.size].to_vec();
    let mut candidate = newer.to_vec();
    let mut ranges: Vec<ChangeRange> = Vec::new();
    let mut owners: Vec<FfsContainer> = Vec::new();
    for t in &targets {
        require(t.checksum_valid, "A duplicate target has an invalid checksum.")?;
        require(
            donor.size <= t.size,
            format!(
                "Old microcode needs {} bytes; the new slot has {}. Relocation or firmware rebuilding would be needed.",
                n0(donor.size as i64),
                n0(t.size as i64)
            ),
        )?;
        require(
            fit_tables.iter().any(|f| f.microcode_offsets.contains(&t.offset)),
            format!("Target at 0x{:X} is not referenced by the active FIT. A recovery copy or different layout needs separate support.", t.offset),
        )?;
        let owning: Vec<&FfsContainer> =
            files.iter().filter(|f| t.offset >= f.body() && t.offset + t.size <= f.end()).collect();
        require(
            owning.len() == 1,
            format!("Target at 0x{:X} is not inside one validated, uncompressed raw FFS file.", t.offset),
        )?;
        let owner = *owning[0];
        let checksum = owner.attributes & 0x40 != 0;
        require(
            if checksum {
                sum8(&newer[owner.body()..owner.end()]).wrapping_add(newer[owner.offset + 17]) == 0
            } else {
                newer[owner.offset + 17] == 0xaa
            },
            "Original FFS data checksum is invalid.",
        )?;
        candidate[t.offset..t.offset + donor.size].copy_from_slice(&patch);
        candidate[t.offset + donor.size..t.offset + t.size].fill(0xff);
        ranges.push(ChangeRange { offset: t.offset, size: t.size, reason: SLOT_REASON.into() });
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }
    for f in owners.iter().filter(|f| f.attributes & 0x40 != 0) {
        candidate[f.offset + 17] = 0u8.wrapping_sub(sum8(&candidate[f.body()..f.end()]));
        if candidate[f.offset + 17] != newer[f.offset + 17] {
            ranges.push(ChangeRange { offset: f.offset + 17, size: 1, reason: FFS_CHECKSUM_REASON.into() });
        }
    }
    for (i, (a, c)) in newer.iter().zip(candidate.iter()).enumerate() {
        if a != c {
            require(
                ranges.iter().any(|r| i >= r.offset && i < r.offset + r.size),
                "Unexpected change outside the authorized ranges.",
            )?;
        }
    }
    let after = firmware::inspect(&candidate, "candidate").microcodes;
    require(after.len() == target_codes.len(), "Microcode inventory count changed unexpectedly.")?;
    for t in &target_codes {
        let a = single_at(&after, t.offset, "A microcode moved or disappeared.")?;
        if targets.contains(t) {
            require(
                a.checksum_valid
                    && a.revision == donor.revision
                    && a.size == donor.size
                    && candidate[a.offset..a.offset + a.size] == patch[..],
                "Replacement validation failed.",
            )?;
        } else {
            require(
                newer[t.offset..t.offset + t.size] == candidate[t.offset..t.offset + t.size],
                "An unselected microcode changed.",
            )?;
        }
    }
    let after_fits = fits(&candidate, &after)?;
    require(after_fits.len() == fit_tables.len(), "FIT resolution changed after modification.")?;
    for fit in &fit_tables {
        require(
            newer[fit.offset..fit.offset + fit.size] == candidate[fit.offset..fit.offset + fit.size],
            "FIT table bytes changed.",
        )?;
    }
    let checks = vec![
        ValidationCheck::new("CPU compatibility", compatibility_detail(&donor, &target)),
        ValidationCheck::new(
            "Replacement integrity",
            format!("{} matching slot(s); donor bytes and Intel checksums verified", targets.len()),
        ),
        ValidationCheck::new("Firmware structure", "Validated raw FFS container; data checksum maintained"),
        ValidationCheck::new(
            "FIT pointers",
            format!("{} active table(s); original addresses and entries preserved", fit_tables.len()),
        ),
        ValidationCheck::new(
            "Unchanged content",
            "Only selected slots, FF padding and any required FFS checksum byte changed",
        ),
        ValidationCheck::new(
            "Other microcodes",
            format!("All {} unselected microcodes remain byte-identical", target_codes.len() - targets.len()),
        ),
    ];
    Ok(TransferResult {
        original: newer.to_vec(),
        patch,
        image: candidate,
        cpuid: donor.cpuid,
        platforms: donor.platforms,
        old_revision: target.revision,
        new_revision: donor.revision,
        ranges,
        checks,
    })
}

/// Rebuild from the untouched original and the donor patch, and require a
/// byte-identical image. Used at every export/load boundary.
pub fn revalidate(result: &TransferResult) -> Result<TransferResult> {
    let target = result.target_offset()?;
    let rebuilt = build(&result.original, &result.patch, 0, target)?;
    require(rebuilt.image == result.image, "The candidate was changed after construction.")?;
    Ok(rebuilt)
}
