//! Write synthetic (non-bootable) test images for UI smoke runs.
//!
//! Usage: `roch-make-synthetic <folder>` creates `E7E01IMS.P00` (donor, B0671
//! 11F, platform mask 0x32) and `E7E01IMS.P90` (target, B0671 137, mask 0x36,
//! plus another CPU's microcode). These images contain only the structures the
//! engine validates and must never be flashed.

use roch_core::synthetic::{build_image, SyntheticLayout, SyntheticMicrocode};
use roch_fixture_tests::fail;

fn main() {
    let Some(dir) = std::env::args().nth(1) else { fail("Pass an output folder.") };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(&e.to_string()));
    let old = build_image(
        &[SyntheticMicrocode::new(0x906a4, 0x80, 0x423, 0x2000), SyntheticMicrocode::new(0xb0671, 0x32, 0x11f, 0x3000)],
        &SyntheticLayout { seed: 21, ..Default::default() },
    );
    let new = build_image(
        &[SyntheticMicrocode::new(0x906a4, 0x80, 0x434, 0x2000), SyntheticMicrocode::new(0xb0671, 0x36, 0x137, 0x3400)],
        &SyntheticLayout { seed: 22, ..Default::default() },
    );
    for (name, image) in [("E7E01IMS.P00", &old), ("E7E01IMS.P90", &new)] {
        std::fs::write(dir.join(name), &image.bytes).unwrap_or_else(|e| fail(&e.to_string()));
        println!("{}", dir.join(name).display());
    }
}
