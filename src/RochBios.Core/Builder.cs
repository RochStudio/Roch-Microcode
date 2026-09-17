using System.Text.Json;

namespace RochBios.Core;

public sealed record ValidationCheck(string Name, string Detail, bool Passed = true);
public sealed record BuildResult(byte[] Image, byte[] Original, string Sha256, IReadOnlyList<ValidationCheck> Checks);
public sealed record BuildPackage(string Directory, string CandidatePath, string RecoveryPath, string ReportPath);

public static class Builder
{
    static void Require(bool condition, string message)
    {
        if (!condition) throw new InvalidDataException(message);
    }
    public static BuildResult Build(byte[] source, byte[] donor)
    {
        Require(Firmware.Hash(source) == Recipe.BaseHash, "This build recipe requires the exact official Z790-A D4 BIOS 3202. Other images can be inspected, but not modified.");
        var dh = Firmware.Hash(donor);
        Require(dh == Recipe.DonorHash || dh == Recipe.PatchHash, "Choose official Z790-A D4 BIOS 1904 or its verified B0671 / 11F patch.");
        var patch = dh == Recipe.DonorHash ? donor.AsSpan(Recipe.DonorStart, Recipe.NewSize).ToArray() : donor.ToArray();
        Require(Firmware.Hash(patch) == Recipe.PatchHash, "Donor microcode hash does not match.");
        Require(Firmware.Sum32Valid(patch), "Donor checksum is invalid.");
        var image = source.ToArray();
        patch.CopyTo(image, Recipe.Start);
        Array.Fill(image, (byte)255, Recipe.Start + Recipe.NewSize, Recipe.OldSize - Recipe.NewSize);
        return Validate(source, image);
    }
    public static BuildResult Validate(byte[] original, byte[] candidate)
    {
        Require(Firmware.Hash(original) == Recipe.BaseHash, "Original BIOS is not the supported 3202 image.");
        Require(candidate.Length == original.Length, "Image size changed.");
        Require(original.AsSpan(0, Recipe.Start).SequenceEqual(candidate.AsSpan(0, Recipe.Start)) &&
            original.AsSpan(Recipe.Start + Recipe.OldSize).SequenceEqual(candidate.AsSpan(Recipe.Start + Recipe.OldSize)), "Bytes outside the microcode slot changed.");
        Require(Firmware.Hash(candidate.AsSpan(Recipe.Start, Recipe.NewSize)) == Recipe.PatchHash, "Replacement is not the verified 11F patch.");
        Require(candidate.AsSpan(Recipe.Start + Recipe.NewSize, Recipe.OldSize - Recipe.NewSize).IndexOfAnyExcept((byte)255) < 0, "Slot padding is invalid.");
        var microcodes = Firmware.Inspect(candidate, "candidate").Microcodes;
        Require(microcodes.Count == 4, "Expected exactly four microcodes.");
        Require(microcodes[2].Cpuid == 0xb0671 && microcodes[2].Revision == 0x11f && microcodes[2].ChecksumValid, "B0671 / 11F is not valid.");
        Require(microcodes.Take(3).All(m => m.ChecksumValid), "A microcode checksum failed.");
        int fit = 0x1091100;
        Require(candidate.AsSpan(fit, 8).SequenceEqual("_FIT_   "u8), "FIT signature missing.");
        Require(candidate.AsSpan(fit, 96).SequenceEqual(original.AsSpan(fit, 96)), "FIT changed.");
        Require((Firmware.U32(candidate, fit + 8) & 0xffffff) == 6, "FIT count changed.");
        int[] offsets = [0x1091400, 0x10c8c00, Recipe.Start, 0x112ec00];
        for (int i = 0; i < offsets.Length; i++)
        {
            int entry = fit + 16 * (i + 1);
            long pos = (long)Firmware.U64(candidate, entry) - 0xfe000000L + 0x1000;
            Require(pos == offsets[i] && (candidate[entry + 14] & 127) == 1, "A FIT microcode pointer is invalid.");
            Require(microcodes[i].Offset == pos, "FIT does not resolve to the expected microcode.");
        }
        var h = candidate.AsSpan(0x10913e8, 24);
        int sum = 0; foreach (byte b in h) sum += b;
        Require((sum - h[17] - h[23]) % 256 == 0 && (h[19] & 0x40) == 0 && h[17] == 0xaa, "FFS checksum configuration changed.");
        string hash = Firmware.Hash(candidate);
        Require(hash == Recipe.ResultHash, "Final image differs from the independently checked reference build.");
        return new(candidate.ToArray(), original.ToArray(), hash,
        [
            new("Official source", "Exact ASUS 3202 and 11F donor hashes"),
            new("Microcode", "B0671 • platform 32 • revision 11F • checksum valid"),
            new("FIT pointers", "All four entries resolve at their original addresses"),
            new("Firmware layout", "Same image size; 7,168 bytes of FF padding"),
            new("Other components", "Every byte outside the microcode slot is unchanged"),
            new("FFS header", "Header checksum and fixed AA data-checksum mode valid"),
            new("Reference build", "Byte-identical to the build reported booting by the user")
        ]);
    }
    public static BuildPackage Export(BuildResult result, string parent)
    {
        // Revalidate at the export boundary: a caller may have changed the exposed byte arrays.
        result = Validate(result.Original, result.Image);
        Directory.CreateDirectory(parent);
        string dir = Path.Combine(parent, $"Roch-Microcode-3202-11F-{DateTime.Now:yyyyMMdd-HHmmss}-{Guid.NewGuid().ToString("N")[..6]}");
        Directory.CreateDirectory(dir);
        try
        {
            string candidateDir = Path.Combine(dir, "modified"), recoveryDir = Path.Combine(dir, "recovery");
            Directory.CreateDirectory(candidateDir); Directory.CreateDirectory(recoveryDir);
            string candidate = Path.Combine(candidateDir, Recipe.FlashName), recovery = Path.Combine(recoveryDir, Recipe.FlashName);
            WriteNewVerified(candidate, result.Image, Recipe.ResultHash);
            WriteNewVerified(recovery, result.Original, Recipe.BaseHash);
            string report = Path.Combine(dir, "validation.json");
            File.WriteAllText(report, JsonSerializer.Serialize(new
            {
                app = "Roch Microcode 1.0.0", createdUtc = DateTimeOffset.UtcNow, board = Recipe.Board,
                bios = "3202", microcode = "11F", cpuid = "B0671", platform = "32",
                baseSha256 = Recipe.BaseHash, donorPatchSha256 = Recipe.PatchHash, sha256 = result.Sha256,
                checks = result.Checks, evidence = "User reported BIOS 3202 and MC 11F after boot on one system. Not independent stability testing.",
                limitations = new[] { "ASUS capsule signature is invalidated; the image is not re-signed.", "FlashBack acceptance and stability on another system are not guaranteed.", "11F omits later microcode mitigations. Windows may load newer microcode." }
            }, new JsonSerializerOptions { WriteIndented = true }));
            File.WriteAllText(Path.Combine(dir, "READ-ME.txt"), Instructions);
            return new(dir, candidate, recovery, report);
        }
        catch
        {
            // Keep incomplete files for diagnosis, clearly marked; never return them as a completed package.
            File.WriteAllText(Path.Combine(dir, "INCOMPLETE.txt"), "Export did not complete. Do not use these files for flashing.");
            throw;
        }
    }
    public static void WriteNewVerified(string path, byte[] data, string hash)
    {
        using (var stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None, 65536, FileOptions.WriteThrough))
        { stream.Write(data); stream.Flush(true); }
        using var read = File.OpenRead(path);
        Require(Convert.ToHexStringLower(System.Security.Cryptography.SHA256.HashData(read)) == hash, "Written file failed SHA-256 verification.");
    }
    public const string Instructions = """
        ROCH MICROCODE — ASUS ROG STRIX Z790-A GAMING WIFI D4

        modified/SZ790AD4.CAP = BIOS 3202 with B0671 microcode 11F.
        recovery/SZ790AD4.CAP = untouched official BIOS 3202.
        These files have the same name. Keep them on separate, clearly labelled USB drives.

        This recipe reproduces the image the user reported flashing and booting successfully on one system.
        A successful file check is not a guarantee of boot or long-term stability. Modifying the payload
        invalidates the ASUS capsule signature. 11F lacks later microcode mitigations.

        USB BIOS FLASHBACK
        1. Use a FAT32 / MBR USB drive with one partition. Save any existing files first.
        2. Put the selected SZ790AD4.CAP at the root. Prepare an official recovery drive as well.
        3. Save BIOS settings; if BitLocker is enabled, keep the recovery key and suspend protection.
        4. Shut down the PC; leave the PSU connected and switched on.
        5. Insert USB into the rear port labelled BIOS FlashBack.
        6. Hold BIOS FlashBack (not Clear CMOS) for about 3 seconds until the LED blinks 3 times.
        7. Wait for the LED to go out. Do not interrupt power, remove USB, power on or clear CMOS.
        8. If it boots, check BIOS version and the active microcode in Windows before tuning.
        9. Resume BitLocker protection after the process is complete, if applicable.

        A brief flash followed by a solid LED indicates a failed FlashBack attempt.
        If flashing completes but the PC cannot boot, use the official recovery image with FlashBack.
        Recovery is not guaranteed. Firmware service or an external programmer may be necessary.

        ASUS instructions: https://www.asus.com/us/support/faq/1038568/
        This app does not flash firmware, install a driver, change BIOS settings or reboot the PC.
        """;
}
