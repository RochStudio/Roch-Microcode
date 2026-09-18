# Roch Microcode 1.0.1

**Drop an old BIOS, drop a new BIOS, select a microcode, build and verify, then prepare USB.**

[Download the Windows x64 app](https://github.com/RochStudio/Roch-Microcode/releases/latest) · [Verification coverage](VALIDATION.md) · [Third-party notices](THIRD-PARTY.md)

Previously named **Roch BIOS**. The executable is now **RochMicrocode.exe**. Existing version 2 BIOS packages remain compatible.

Roch Microcode is a Windows x64 app for Intel microcode transfer across supported **ASUS, MSI, ASRock and Gigabyte** firmware layouts. The general engine has no fixed motherboard, BIOS-version, revision or offset allowlist. It validates each input's actual structure. Downloads labelled `runtime-required` need the **.NET 10 Desktop Runtime (x64)**; self-contained builds include it. Python and the verification tools are bundled.

## Use

1. Run `RochMicrocode.exe`. The compact layout fits at 1000 × 720 or larger (Windows logical pixels). Use the header theme button to switch light/dark; the choice is saved for next launch. The four pages do not scroll; long microcode inventories can scroll within their table.
2. Drop the **old donor BIOS** on the left and the **new BIOS to modify** on the right. Browse buttons work too. Vendor ZIP packages containing one identifiable Intel BIOS can be dropped directly; bundled flash utilities are never executed.
3. Select the microcode to copy from the old image. The new-image list contains only patches with the same primary CPU signature and a nonzero donor platform mask equal to or contained in the target mask. A narrower donor shows the reduced CPU/platform scope before building and in the verification and USB screens. Select the intended target when there are alternatives.
4. Check the target manufacturer, exact motherboard model/hardware revision, **flash method** and filename. MSI defaults to **M-FLASH (inside BIOS)** and preserves the new BIOS filename (for example, MPOWER PA uses `E7E01IMS.PA0`). Select **FlashBack / BIOS button** only when using that hardware feature; MSI then uses `MSI.ROM`. Suggestions are conveniences, not motherboard identification guarantees.
5. Click **Build & verify** and choose an output folder. The app checks the structure, runs UEFIExtract and MCExtractor, then exports the modified image, untouched new BIOS for recovery, donor patch and reports.
6. On **Prepare USB**, select an existing FAT32 / MBR USB drive with one partition and choose modified or recovery. Confirm the exact destination. Both verifiers run again before copying. Use separate labelled drives for modified and recovery images.
7. Follow the exact board manual at the motherboard. The app does not flash firmware, format USB, install a driver, change settings or reboot.

The app footer includes YouTube, X and Discord links on every tab; they open in your default browser.

An existing root BIOS file is saved under the app's local data folder in `USB backups` before replacement. New installations use `%LOCALAPPDATA%/Roch Microcode/`; existing installations continue using `%LOCALAPPDATA%/Roch BIOS/` when that folder exists and the new folder does not. This preserves theme settings, tool caches and backups. The USB disk and volume identity are rechecked before writing and committing the staged file. The file is flushed and its SHA-256 verified by reading it back.

## What is supported

- Intel firmware with a resolvable active **Firmware Interface Table (FIT)** and directly addressable microcode in a validated **raw UEFI FFS file**.
- Equal-size or smaller replacement patches with the same primary CPUID and a nonzero donor platform mask equal to or contained in the target mask. Partial overlap and donor masks that add unsupported platform bits are rejected.
- Matching duplicates of the selected target revision, provided each is referenced by the active FIT and independently passes the structural checks.
- The donor may come from another motherboard when its selected Intel microcode meets the CPU/platform checks. The **new BIOS must belong to the target motherboard**; the donor's complete firmware is never flashed onto it.
- Existing flash image/capsule layout and all addresses remain in place. The selected slot is filled with donor bytes and FF padding; an FFS data-checksum byte is repaired if required.

**Brand support does not mean every board or every BIOS is editable or flashable.** AMD microcode/AGESA editing, compressed microcode, oversized replacements requiring relocation, non-FIT layouts, unsupported containers, and ambiguous recovery copies are rejected or inspection-only. The model must support the selected flash method. M-FLASH may reject modified firmware even with the correct filename. Signed capsules, Boot Guard, measured boot and rollback enforcement may reject a structurally valid modification; the app does not disable or bypass these checks.

Old patches can omit later fixes, mitigations or extended CPU signatures. The primary CPUID and donor platform scope are checked; support for every extended signature from the newer patch is not promised. Windows may load a newer revision. File verification does not establish bootability, security, recovery success or long-term stability.

## USB naming profiles

| Vendor | Default root filename | Reference |
| --- | --- | --- |
| ASUS | Model-specific `.CAP`; inferred when one unique name exists in firmware, otherwise enter it | [ASUS FlashBack](https://www.asus.com/us/support/faq/1038568/) |
| MSI M-FLASH (default) | Original **new BIOS** filename, including its version extension | [MSI M-FLASH](https://www.msi.com/support/technical_details/mb_bios_update) |
| MSI Flash BIOS Button | `MSI.ROM` | [MSI Flash BIOS Button](https://www.msi.com/support/technical_details/MB_Flash_BIOS_Button) |
| ASRock | `CREATIVE.ROM` | [ASRock Flashback](https://www.asrock.com/microsite/BIOSFlashback2026/) |
| Gigabyte | `GIGABYTE.bin` | [Gigabyte Q-Flash Plus](https://www.gigabyte.com/FileUpload/Global/KeyFeature/3798/index.html) |

The exact board manual takes precedence over a vendor-wide default. ASUS BIOSRenamer can establish its model-specific filename. Not all boards support hardware FlashBack.

Existing packages keep their original method and filenames. To change an older `MSI.ROM` package to M-FLASH in the app, load the original donor and new vendor BIOS again, choose M-FLASH, then build and verify a new package. Load the original BIOS or vendor ZIP so the version filename is available; the app does not guess it from a renamed `MSI.ROM`.

## Verification and package contents

The native engine checks Intel additive checksums, primary CPUID equality and donor platform-mask containment, validated FFS/FV headers, slot capacity, FIT address resolution, unchanged FIT bytes, unchanged other microcodes and every changed byte. It rebuilds the output again at export/load boundaries.

**UEFIExtract NE A75** parses original and modified images. The app compares the tree, unchanged-node CRCs and parser diagnostics, allowing only the expected microcode/padding changes and containing-node CRC changes. Existing diagnostics can remain; full logs are retained.

**MCExtractor 1.104.0 / database r352** independently checks the donor and both images. The primary inventory's CPUID, platform, revision, size and offsets must agree with the native parser. No new warnings/errors are accepted; donor warnings/errors stop export. Extended-signature tables are processed by MCExtractor.

Packages contain `modified/<filename>`, `recovery/<filename>`, `donor-microcode.bin`, `package.json`, `validation.json`, `verification/` and `READ-ME.txt`. Incomplete exports are marked and rejected on opening. The manifest is not a signed certificate: actual firmware is rebuilt and external verification reruns on opening and before USB preparation. Version 1 package folders must be rebuilt with the version 2 app.

## Build and test

Requires .NET SDK 10 on Windows for source builds:

```powershell
.\build.ps1
dotnet run --project tests/RochBios.Tests -c Release -- "official-3202.CAP" "official-1904.CAP" --online
dotnet run --project tests/RochBios.TransferTests -c Release -- "ASUS-fixture-folder" "cross-vendor-fixture-folder"
```

The cross-vendor suite uses real BIOS pairs listed in `VALIDATION.md`; it does not flash hardware. The app also includes an internal smoke test that loads both images, exports an independently verified package into the test folder and renders all four tabs in both themes at normal (1180 × 800) and compact (1000 × 720) sizes, checking that the key controls remain in bounds:

```powershell
.\dist\RochMicrocode.exe --smoke-test "new-BIOS-file" "old-BIOS-file" "screenshot-output-folder"
```

`tools/Bundle-Verifiers.ps1` rebuilds the embedded tool bundle from upstream UEFIExtract A75, MCExtractor r352, CPython 3.11.9, colorama 0.4.6 and PLTable 1.1.0. It uses pip only when rebuilding that development bundle. Runtime tool extraction and integrity checks are automatic. Licenses are under `THIRD-PARTY/` and embedded with the tools.

No telemetry. Load your own BIOS files or vendor ZIPs; transfer work is offline. Verifier updates are disabled during execution. Tool cache and diagnostic work live in the local data folder described above.

Firmware fixtures, generated BIOS images, local reports and USB backups are not included in this repository or the app download. Integration tests require separately obtained firmware files. The GitHub workflow verifies the Windows build; it does not flash hardware or claim the integration tests have run.

## Licensing

Copyright (C) 2026 Roch Studio. Roch Microcode's own source code is licensed under **GPL-3.0-or-later**, matching the other Roch tools. You may redistribute and modify it under the GNU General Public License version 3 or, at your option, any later version. It is provided without warranty; see [LICENSE](LICENSE) for the full terms.

The Windows release includes the license, and a matching source archive is available alongside it on the [release page](https://github.com/RochStudio/Roch-Microcode/releases/latest). Bundled third-party tools retain their respective licenses and notices; see [THIRD-PARTY.md](THIRD-PARTY.md).

Created for [MateoPcTech](https://www.youtube.com/@MateoPcTech). Not affiliated with motherboard manufacturers or Intel.

[YouTube](https://www.youtube.com/@MateoPcTech) | [X](https://x.com/MateoPCTech) | [Discord](https://discord.gg/KfzExpKQHB)
