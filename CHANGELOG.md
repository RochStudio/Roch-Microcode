# Changelog

## 2.1.1 — Roch Microcode

- Renamed the application, executable and release package from Roch BIOS to Roch Microcode.
- Retained existing theme settings, verifier caches, USB backups and version 2 BIOS-package compatibility.
- Added the standalone GitHub repository and Windows build workflow.
- Clarified that compatible donor microcode may come from another motherboard, while the new firmware must match the target board.

## 2.1.0

- Compact pages with primary actions visible at 1000 × 720 or larger.
- Light and dark themes with a saved preference.
- All eight verification checks displayed together.

## 2.0.1

- Allowed narrower nonzero donor platform masks for the same primary CPU signature.
- Reported reduced platform coverage before building, in verification and in USB preparation.
- Verified the MSI PRO Z790-A WIFI DDR4 1K / 11F transfer with both independent tools.

## 2.0.0

- General Intel microcode transfer for supported ASUS, MSI, ASRock and Gigabyte firmware layouts.
- Bundled UEFIExtract and MCExtractor verification, recovery packages and verified USB-file preparation.
