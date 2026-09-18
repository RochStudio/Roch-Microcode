# Roch Microcode verification

## 1.0.1 UI refresh

The darker theme and simplified interface passed a real MPOWER PA / 11F transfer with both independent verifiers, M-FLASH/button selection checks and package reload. All four tabs fit at 1180 × 800 and 1000 × 720 in both themes; screenshots were reviewed. No USB or motherboard writes occurred. Evidence: `test-results/ui-v1.0.1/`. The Windows 11 title bar uses the active theme's background and text colours; older Windows versions retain their native frame where these attributes are unsupported.

## Earlier verification

The M-FLASH update passed 20 filename/method checks, including loading an existing MSI button package, JSON compatibility, incorrect target/version rejection and unchanged naming for other vendors. A real MPOWER PA / PRO Z790-A 11F transfer passed both independent verifiers, package reload and layout checks across four tabs, two themes and two window sizes. The UI test also verifies switching between M-FLASH and the button filename. No USB writes or hardware flashing were performed. Screenshots and evidence: `test-results/ui-mflash/`.

Verified on Windows x64. Release 1.0.0 establishes the Roch Microcode version numbering; firmware behavior is unchanged from the initial 2.1.1 label. Existing app data and version 2 package compatibility are preserved. UI smoke testing runs a real independently verified transfer and captures all four tabs in both themes at 1180 × 800 and 1000 × 720, with control-boundary checks and visual review. No page scrollbars; long inventories retain table scrolling. **97 transfer tests and 36 regression/USB-policy tests passed for the same implementation before the version-only change.** The seven optional online-download tests passed in 2.0.0; downloader code is unchanged and those network tests were not repeated.

The general engine built and exported these real BIOS pairs, each checked by bundled UEFIExtract A75 and MCExtractor 1.104.0/r352:

| Vendor and board | Old donor | New BIOS | Selected transfer |
| --- | --- | --- | --- |
| ASUS ROG STRIX Z790-A GAMING WIFI D4 | 1904 | 3202 | B0671 `133 → 11F` |
| MSI Z790 MPOWER, E7E01IMS | P00 | P90 | B0671 `133 → 120` |
| MSI Z790 MPOWER with PRO Z790-A donor | E7E07IMS.190 | E7E01IMS.P90 | B0671 `133 → 11F` (UI integration test) |
| MSI PRO Z790-A WIFI DDR4, E7E07IMS | 190 | 1K0 | B0671 `137 → 11F`, mask `36 → 32` |
| ASRock Z790 Taichi | 10.01 | 20.01 | B0671 `133 → 11D` |
| Gigabyte Z790 AORUS ELITE AX rev 1.x | FF | FH | B0671 `11D → 115` |

These establish file-construction and verification coverage for representative layouts, not universal board coverage or hardware boot tests. The selected sample versions are test fixtures, not recommendations to install those releases.

ASUS output SHA-256 remains `4b7bf8c4550e5cbe4c8d4594ce8e90368f8436064ec8f80ba3a494c2d3656b5a`, matching the image previously reported by the user to boot with BIOS 3202 and microcode 11F.

The MSI 1K regression checks donor bytes, FF slot padding, reduced scope in reports and package reload, and rejection of zero, disjoint, partially overlapping and broader donor masks despite valid additive checksums. Negative tests cover mismatched CPUs, unchanged revisions, missing active FIT, oversized patches, donor corruption, candidate edits outside the slot, package metadata tampering, path traversal, Windows reserved filenames and ambiguous ZIPs. Other checks cover input preservation, repeatable reconstruction, correct recovery filenames, six real vendor ZIP imports, independent report export and FFS payload-checksum repair. Regression tests retain the pinned ASUS reference, download hash rejection, cache recovery, cancellation and USB eligibility/identity checks.

No physical USB writing or motherboard flashing was performed during this update. USB storage policy and verified file writes were tested with isolated fixtures. The app remains unsigned. File checks cannot prove capsule signature validity, Boot Guard or rollback acceptance, successful FlashBack, recovery or stability.

Local test logs and generated packages are in `test-results/`. UI screenshots and the published EXE smoke-test result are under `test-results/ui-v1.0.0/`.

## Official fixture sources

- [ASUS 1904](https://dlcdnets.asus.com/pub/ASUS/mb/BIOS/ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-1904.zip) and [3202](https://dlcdnets.asus.com/pub/ASUS/mb/BIOS/ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-3202.zip)
- [MSI P00](https://download.msi.com/bos_exe/mb/7E01vP0.zip) and [P90](https://download.msi.com/bos_exe/mb/7E01vP9.zip)
- [ASRock 10.01](https://download.asrock.com/BIOS/1700/Z790%20Taichi%2810.01%29ROM.zip) and [20.01](https://download.asrock.com/BIOS/1700/Z790%20Taichi%2820.01%29ROM.zip)
- [Gigabyte FF](https://download.gigabyte.com/FileList/BIOS/mb_bios_z790-aorus-elite-ax_8arpt050_ff.zip) and [FH](https://download.gigabyte.com/FileList/BIOS/mb_bios_z790-aorus-elite-ax_8arpt050_fh.zip)
