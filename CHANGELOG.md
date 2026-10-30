# Changelog

## Unreleased — Rust rewrite (`rust-rewrite` branch)

- Rewritten in Rust: `roch-core` (engine, package format, bundled verifiers, USB policy) and `roch-app` (egui interface). `RochMicrocode.exe` is now a native single executable; no .NET runtime is required.
- Same firmware rules, rejection messages, version 2 package layout and JSON format, M-FLASH / FlashBack naming, USB safety checks, data folder (including the legacy `Roch BIOS` fallback), saved theme and footer links. The bundled UEFIExtract / MCExtractor tools and their invocation are unchanged.
- Build, tests and workflows use Cargo. Synthetic-firmware tests now run without real BIOS files; the real-firmware suites keep their arguments.
- The real-firmware suites listed in `VALIDATION.md` have not yet been re-run against the Rust implementation.

## 1.0.1

- Darker neutral background, inset inputs and subtler card borders.
- Native title bar matches the active theme on Windows 11, including theme changes.
- Smaller header, simpler donor/target labels and understated navigation tabs.
- Removed the ASUS example button and download action; users supply their own BIOS files.
- Preserved light theme, social links, M-FLASH naming and existing package compatibility.

## 1.0.0 — Roch Microcode

- MSI now defaults to M-FLASH with the original new BIOS filename; the separate Flash BIOS Button option uses MSI.ROM. Package instructions and USB preparation follow the chosen method. Existing packages retain their original method.

- Added clickable YouTube, X and Discord links below the app status line, visible on every tab in both themes.

- Licensed GPL-3.0-or-later, with the license and corresponding source supplied alongside the Windows release.

- Intel microcode transfer across supported ASUS, MSI, ASRock and Gigabyte firmware layouts.
- Bundled native checks, UEFIExtract and MCExtractor verification, recovery packages and verified USB preparation.
- Compact pages with primary actions visible at 1000 × 720 or larger.
- Light and dark themes with a saved preference; all eight verification checks displayed together.
- Support for narrower donor platform masks with the resulting CPU/platform scope shown throughout the workflow.
- Compatible microcode donors may come from another motherboard; the new firmware must match the target board.
- Preserves existing Roch BIOS settings, verifier caches, USB backups and version 2 BIOS packages.
- Starts the Roch Microcode release numbering at 1.0.0. The initial 2.1.1 label is superseded; firmware-processing behavior is unchanged.
