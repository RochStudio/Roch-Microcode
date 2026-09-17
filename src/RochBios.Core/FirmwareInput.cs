using System.IO.Compression;
using System.Text;
using System.Text.RegularExpressions;

namespace RochBios.Core;

public sealed record FirmwareInput(string FileName, byte[] Bytes, FirmwareInfo Info)
{
    public static FirmwareInput Load(string path)
    {
        if (!Path.GetExtension(path).Equals(".zip", StringComparison.OrdinalIgnoreCase))
        {
            var bytes = Firmware.ReadImage(path); return new(Path.GetFileName(path), bytes, Firmware.Inspect(bytes, Path.GetFileName(path)));
        }
        using var archive = ZipFile.OpenRead(path);
        var entries = archive.Entries.Where(e => e.Length >= 1024 * 1024 && e.Length <= Firmware.MaxImageSize &&
            !new[] { ".exe", ".efi", ".pdf", ".txt", ".dll", ".zip" }.Contains(Path.GetExtension(e.Name).ToLowerInvariant())).ToArray();
        Transfer.Require(entries.Length is > 0 and <= 12, "ZIP has no clear firmware candidate, or too many large entries. Extract it and choose the BIOS file.");
        var candidates = new List<FirmwareInput>();
        foreach (var entry in entries)
        {
            using var stream = entry.Open(); using var memory = new MemoryStream();
            var buffer = new byte[65536]; int n;
            while ((n = stream.Read(buffer)) != 0)
            { Transfer.Require(memory.Length + n <= Firmware.MaxImageSize, "ZIP entry exceeds 128 MB."); memory.Write(buffer, 0, n); }
            var bytes = memory.ToArray(); var info = Firmware.Inspect(bytes, entry.Name);
            if (info.Microcodes.Count != 0) candidates.Add(new(entry.Name, bytes, info));
        }
        Transfer.Require(candidates.Count == 1, candidates.Count == 0 ? "No uncompressed Intel microcode found in this ZIP. Extract it and inspect the BIOS file." : "ZIP contains more than one BIOS image. Extract it and choose the exact image.");
        return candidates[0];
    }
    public string SuggestedVendor => Info.Sha256 is Recipe.BaseHash or Recipe.DonorHash or Recipe.ResultHash || FileName.Contains("-ASUS-", StringComparison.OrdinalIgnoreCase) ? "ASUS"
        : Regex.IsMatch(FileName, @"^E[0-9A-Z]{4}[IA]MS\.[0-9A-Z]{3}$", RegexOptions.IgnoreCase) ? "MSI"
        : Regex.IsMatch(FileName, @"\.F[A-Z0-9]{1,3}$", RegexOptions.IgnoreCase) ? "Gigabyte"
        : Encoding.ASCII.GetString(Bytes).Contains("ASRock", StringComparison.OrdinalIgnoreCase) ? "ASRock" : "";
    public string? SuggestedAsusName
    {
        get
        {
            if (SuggestedVendor != "ASUS") return null;
            var names = Regex.Matches(Encoding.ASCII.GetString(Bytes), @"(?<![A-Z0-9])[A-Z0-9]{3,8}\.CAP").Select(m => m.Value).Where(n => n != "ASUS.CAP").Distinct().ToArray();
            return names.Length == 1 ? names[0] : null;
        }
    }
}

public sealed record FlashProfile(string Vendor, string Board, string FileName)
{
    public const string FlashBack = "FlashBack / BIOS button";
    public const string MFlash = "M-FLASH (inside BIOS)";
    // Missing fields in existing version 2 packages retain the original button workflow.
    public string Method { get; init; } = FlashBack;
    public string? OriginalFileName { get; init; }
    public static string[] Methods(string vendor) => vendor == "MSI" ? [MFlash, FlashBack] : [FlashBack];
    public static string SuggestedName(string vendor, string method, string? originalName, string? asusName = null) =>
        method == MFlash ? originalName ?? "" : vendor == "ASUS" ? asusName ?? "" : DefaultName(vendor);
    public static readonly string[] Vendors = ["ASUS", "MSI", "ASRock", "Gigabyte"];
    public static string DefaultName(string vendor) => vendor switch { "MSI" => "MSI.ROM", "ASRock" => "CREATIVE.ROM", "Gigabyte" => "GIGABYTE.bin", _ => "" };
    public void Validate()
    {
        Transfer.Require(Vendors.Contains(Vendor), "Choose ASUS, MSI, ASRock or Gigabyte.");
        Transfer.Require(Methods(Vendor).Contains(Method), "Choose a supported flash method for this manufacturer.");
        Transfer.Require(!string.IsNullOrWhiteSpace(Board) && Board.Length <= 120 && !Board.Any(char.IsControl), "Enter the target motherboard model and revision.");
        Transfer.Require(Regex.IsMatch(FileName, @"^[A-Za-z0-9_-]{1,24}\.[A-Za-z0-9]{1,4}$") && !Regex.IsMatch(Path.GetFileNameWithoutExtension(FileName), @"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$", RegexOptions.IgnoreCase), "Enter a valid root BIOS filename, without folders.");
        if (Method == MFlash)
        {
            Transfer.Require(Regex.IsMatch(FileName, @"^E[0-9A-Z]{4}[IA]MS\.[0-9A-Z]{3}$", RegexOptions.IgnoreCase), "M-FLASH needs the original MSI BIOS filename. Load the original vendor BIOS or ZIP, not a renamed MSI.ROM.");
            Transfer.Require(FileName.Equals(OriginalFileName, StringComparison.OrdinalIgnoreCase), "M-FLASH must preserve the NEW target BIOS filename, including its version extension.");
        }
        else if (Vendor == "ASUS") Transfer.Require(FileName.EndsWith(".CAP", StringComparison.OrdinalIgnoreCase), "ASUS FlashBack requires the model-specific .CAP name.");
        else Transfer.Require(FileName.Equals(DefaultName(Vendor), StringComparison.OrdinalIgnoreCase), "The filename does not match this vendor's FlashBack naming rule.");
    }
    public string GuideUrl => Vendor switch
    {
        "MSI" => Method == MFlash ? "https://www.msi.com/support/technical_details/mb_bios_update" : "https://www.msi.com/support/technical_details/MB_Flash_BIOS_Button",
        "ASRock" => "https://www.asrock.com/microsite/BIOSFlashback2026/",
        "Gigabyte" => "https://www.gigabyte.com/FileUpload/Global/KeyFeature/3798/index.html",
        _ => "https://www.asus.com/us/support/faq/1038568/"
    };
    public string BoardInstructions => Method == MFlash
        ? "Use M-FLASH inside the BIOS with the original target filename. A visible file does not prove acceptance; M-FLASH may reject modified firmware. Keep the untouched BIOS separately. Do not interrupt power while flashing."
        : "Confirm FlashBack support, filename, USB port and power connections in the board manual. Keep a separate recovery drive. Do not interrupt power while flashing.";
    public string Instructions => $"""
        ROCH MICROCODE — {Vendor} {Board}
        Flash method: {Method}
        USB root filename: {FileName}
        Modified and recovery files use the same name. Keep them on separate labelled drives.

        1. Confirm the NEW BIOS belongs to the exact target motherboard and hardware revision.
           The donor may be from another board; it contributes only its compatible Intel microcode.
           Modified and recovery outputs both use the target board's newer firmware as their base.
        2. {(Method == MFlash ? "Use M-FLASH inside the MSI BIOS. Preserve the original target BIOS filename; MSI.ROM is for the Flash BIOS Button." : "Your model must have a dedicated BIOS FlashBack / Flash BIOS Button / Q-Flash Plus feature. Confirm support in its manual.")}
        3. Use a FAT32 / MBR USB drive with one partition. Roch Microcode never formats it.
        4. Copy the chosen {FileName} to the USB root. Keep the untouched new BIOS on a recovery drive.
        5. Save settings; have the BitLocker recovery key and suspend protection if enabled.
        6. {(Method == MFlash ? "Enter BIOS, open M-FLASH, and select the BIOS file on the USB drive. If rejected, stop; renaming cannot bypass signature checks." : "Follow the EXACT board manual for power connectors, dedicated USB port, button and LED behavior. Usually the PC is off with PSU power connected.")}
           Do not interrupt power during flashing.
        7. After boot, confirm BIOS version and active microcode, then check stability before tuning.
           Resume BitLocker protection when complete.

        Vendor guide: {GuideUrl}
        ASUS: the FlashBack filename is model-specific. Check BIOSRenamer or the board manual.
        Other vendor defaults can also vary by generation; the exact board manual takes precedence.

        Verification checks file structure and integrity, not boot or long-term stability.
        Changing microcode can invalidate capsule/firmware signatures or measured-boot expectations.
        Signature enforcement or rollback protection may reject this image. Roch Microcode does not bypass them.
        Earlier microcode can omit later fixes, mitigations and extended CPU signatures.
        Windows may load a different revision. Recovery is not guaranteed.
        The app never flashes firmware, installs a driver or changes system settings.
        """;
}
