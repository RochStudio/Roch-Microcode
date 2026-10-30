//! Synthetic Intel firmware images for tests and offline smoke runs.
//!
//! Real vendor BIOS fixtures are not redistributable, so the unit tests build
//! small images with the exact structures the engine validates: a top-of-4 GB
//! mapping with the FIT pointer at `0xFFFFFFC0`, a FIT with microcode entries,
//! an FFS2 firmware volume with a checksummed header and a raw FFS file whose
//! body holds Intel microcode updates with valid additive checksums.
//! These images are not bootable firmware and must never be flashed.

/// One microcode to place in a synthetic image.
#[derive(Clone, Debug)]
pub struct SyntheticMicrocode {
    pub cpuid: u32,
    pub platforms: u32,
    pub revision: u32,
    /// Total size, a multiple of 1024 and at least 2048.
    pub size: usize,
    /// Intel date dword, e.g. `0x0129_2024`.
    pub date: u32,
    /// Referenced by the FIT.
    pub in_fit: bool,
}

impl SyntheticMicrocode {
    pub fn new(cpuid: u32, platforms: u32, revision: u32, size: usize) -> Self {
        Self { cpuid, platforms, revision, size, date: 0x0129_2024, in_fit: true }
    }
}

#[derive(Clone, Debug)]
pub struct SyntheticImage {
    pub bytes: Vec<u8>,
    /// Offset of each microcode, in the order given.
    pub microcode_offsets: Vec<usize>,
    pub fv_offset: usize,
    pub ffs_offset: usize,
    pub fit_offset: usize,
}

/// Image layout options.
#[derive(Clone, Debug)]
pub struct SyntheticLayout {
    pub image_size: usize,
    /// Set FFS_ATTRIB_CHECKSUM on the raw file and store a real data checksum.
    pub ffs_data_checksum: bool,
    /// Fill byte for microcode payloads (varies the bytes between images).
    pub seed: u32,
    /// Write the FIT pointer at 0xFFFFFFC0.
    pub fit_pointer: bool,
}

impl Default for SyntheticLayout {
    fn default() -> Self {
        Self { image_size: 0x10_0000, ffs_data_checksum: false, seed: 1, fit_pointer: true }
    }
}

const FFS2_GUID: [u8; 16] =
    [0x78, 0xE5, 0x8C, 0x8C, 0x3D, 0x8A, 0x1C, 0x4F, 0x99, 0x35, 0x89, 0x61, 0x85, 0xC3, 0x2D, 0xD3];

fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes());
}

/// A standalone Intel microcode update with a valid checksum.
pub fn microcode_bytes(m: &SyntheticMicrocode, seed: u32) -> Vec<u8> {
    assert!(m.size >= 2048 && m.size.is_multiple_of(1024));
    let mut b = vec![0u8; m.size];
    put32(&mut b, 0, 1);
    put32(&mut b, 4, m.revision);
    put32(&mut b, 8, m.date);
    put32(&mut b, 12, m.cpuid);
    put32(&mut b, 20, 1);
    put32(&mut b, 24, m.platforms);
    put32(&mut b, 28, (m.size - 48) as u32);
    put32(&mut b, 32, m.size as u32);
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(m.revision).wrapping_add(m.cpuid);
    for chunk in b[48..].as_chunks_mut::<4>().0 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    let sum =
        b.as_chunks::<4>().0.iter().fold(0u32, |s, c| s.wrapping_add(u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    put32(&mut b, 16, sum.wrapping_neg());
    b
}

/// Build a synthetic firmware image.
pub fn build_image(codes: &[SyntheticMicrocode], layout: &SyntheticLayout) -> SyntheticImage {
    let len = layout.image_size;
    let mut b = vec![0xffu8; len];
    let base = 0x1_0000_0000u64 - len as u64;

    // Firmware volume at 0x1000.
    let fv = 0x1000usize;
    let ffs = fv + 72;
    let body = ffs + 24;
    let total: usize = codes.iter().map(|m| m.size).sum();
    let ffs_size = 24 + total;
    let fv_len = (72 + ffs_size + 0x1000 + 0xfff) & !0xfff;
    assert!(fv + fv_len + 0x1000 < len, "image too small for the requested microcodes");
    b[fv..fv + 16].fill(0);
    b[fv + 16..fv + 32].copy_from_slice(&FFS2_GUID);
    put64(&mut b, fv + 32, fv_len as u64);
    b[fv + 40..fv + 44].copy_from_slice(b"_FVH");
    put32(&mut b, fv + 44, 0x0004_FEFF);
    b[fv + 48..fv + 50].copy_from_slice(&72u16.to_le_bytes());
    b[fv + 50..fv + 52].copy_from_slice(&0u16.to_le_bytes());
    b[fv + 52..fv + 54].copy_from_slice(&0u16.to_le_bytes());
    b[fv + 54] = 0;
    b[fv + 55] = 2;
    put32(&mut b, fv + 56, (fv_len / 0x1000) as u32);
    put32(&mut b, fv + 60, 0x1000);
    put64(&mut b, fv + 64, 0);
    let sum = (0..72).step_by(2).fold(0u16, |s, n| s.wrapping_add(u16::from_le_bytes([b[fv + n], b[fv + n + 1]])));
    b[fv + 50..fv + 52].copy_from_slice(&sum.wrapping_neg().to_le_bytes());

    // Microcodes inside one raw FFS file.
    let mut offsets = Vec::new();
    let mut p = body;
    for m in codes {
        let bytes = microcode_bytes(m, layout.seed);
        b[p..p + m.size].copy_from_slice(&bytes);
        offsets.push(p);
        p += m.size;
    }
    let guid = [0x17u8, 0x08, 0x8c, 0x19, 0x0a, 0x53, 0x4c, 0x4e, 0x94, 0x42, 0x5b, 0x03, 0x99, 0x10, 0x42, 0x7e];
    b[ffs..ffs + 16].copy_from_slice(&guid);
    b[ffs + 18] = 0x01; // EFI_FV_FILETYPE_RAW
    b[ffs + 19] = if layout.ffs_data_checksum { 0x40 } else { 0x00 };
    b[ffs + 20] = (ffs_size & 0xff) as u8;
    b[ffs + 21] = ((ffs_size >> 8) & 0xff) as u8;
    b[ffs + 22] = ((ffs_size >> 16) & 0xff) as u8;
    b[ffs + 23] = 0xf8; // EFI_FILE_DATA_VALID | ...
    b[ffs + 17] = if layout.ffs_data_checksum {
        0u8.wrapping_sub(b[body..ffs + ffs_size].iter().fold(0u8, |s, x| s.wrapping_add(*x)))
    } else {
        0xaa
    };
    b[ffs + 16] = 0;
    let header_sum = (0..24).filter(|&i| i != 17 && i != 23).fold(0u8, |s, i| s.wrapping_add(b[ffs + i]));
    b[ffs + 16] = header_sum.wrapping_neg();

    // FIT near the top of the image, pointer at 0xFFFFFFC0.
    let fit_codes: Vec<usize> = codes.iter().zip(&offsets).filter(|(m, _)| m.in_fit).map(|(_, &o)| o).collect();
    let fit = len - 0x1000;
    let count = fit_codes.len() + 2;
    b[fit..fit + count * 16].fill(0);
    b[fit..fit + 8].copy_from_slice(b"_FIT_   ");
    put32(&mut b, fit + 8, count as u32);
    b[fit + 12..fit + 14].copy_from_slice(&0x0100u16.to_le_bytes());
    b[fit + 14] = 0x00;
    for (i, &o) in fit_codes.iter().enumerate() {
        let e = fit + 16 * (i + 1);
        put64(&mut b, e, base + o as u64);
        b[e + 12..e + 14].copy_from_slice(&0x0100u16.to_le_bytes());
        b[e + 14] = 0x01;
    }
    // A trailing non-microcode entry (startup ACM type 2 placeholder).
    let e = fit + 16 * (fit_codes.len() + 1);
    put64(&mut b, e, base + 0x8000);
    b[e + 12..e + 14].copy_from_slice(&0x0100u16.to_le_bytes());
    b[e + 14] = 0x7f;
    if layout.fit_pointer {
        put64(&mut b, len - 0x40, base + fit as u64);
    }
    SyntheticImage { bytes: b, microcode_offsets: offsets, fv_offset: fv, ffs_offset: ffs, fit_offset: fit }
}

/// Typical pair: an older donor (smaller 11F patch) and a newer target with
/// two other CPU microcodes and a 12F target in the middle.
pub fn sample_pair() -> (SyntheticImage, SyntheticImage) {
    let older = build_image(
        &[
            SyntheticMicrocode::new(0x906a4, 0x80, 0x423, 0x2000),
            SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000),
            SyntheticMicrocode::new(0xb06a2, 0xe0, 0x4122, 0x2400),
        ],
        &SyntheticLayout { seed: 7, ..Default::default() },
    );
    let newer = build_image(
        &[
            SyntheticMicrocode::new(0x906a4, 0x80, 0x434, 0x2000),
            SyntheticMicrocode::new(0xb0671, 0x32, 0x12f, 0x3400),
            SyntheticMicrocode::new(0xb06a2, 0xe0, 0x4128, 0x2400),
        ],
        &SyntheticLayout { seed: 9, ..Default::default() },
    );
    (older, newer)
}
