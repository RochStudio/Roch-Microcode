using System.Text.Json;

namespace RochBios.Core;

public sealed record PackageManifest(int Schema, FlashProfile Profile, string OriginalHash, string CandidateHash, string PatchHash,
    int TargetOffset, uint Cpuid, uint FromRevision, uint ToRevision, IReadOnlyList<ValidationCheck> Checks);
public sealed record TransferPackage(string Directory, PackageManifest Manifest)
{
    public string CandidatePath => Path.Combine(Directory, "modified", Manifest.Profile.FileName);
    public string RecoveryPath => Path.Combine(Directory, "recovery", Manifest.Profile.FileName);
    public string PatchPath => Path.Combine(Directory, "donor-microcode.bin");
    public TransferResult Revalidate()
    {
        Manifest.Profile.Validate();
        Transfer.Require(Manifest.Schema == 2, "Unsupported package format.");
        byte[] original = Firmware.ReadImage(RecoveryPath), candidate = Firmware.ReadImage(CandidatePath), patch = Firmware.ReadImage(PatchPath);
        Transfer.Require(Firmware.Hash(original) == Manifest.OriginalHash && Firmware.Hash(candidate) == Manifest.CandidateHash && Firmware.Hash(patch) == Manifest.PatchHash, "Package file hashes have changed.");
        var result = Transfer.Build(original, patch, 0, Manifest.TargetOffset);
        Transfer.Require(result.Image.AsSpan().SequenceEqual(candidate), "Package cannot be reproduced from its original and donor.");
        Transfer.Require(result.Cpuid == Manifest.Cpuid && result.OldRevision == Manifest.FromRevision && result.NewRevision == Manifest.ToRevision, "Package metadata disagrees with the firmware.");
        return result;
    }
    public static TransferPackage Open(string dir)
    {
        Transfer.Require(!File.Exists(Path.Combine(dir, "INCOMPLETE.txt")), "This package is marked incomplete and cannot be prepared for flashing.");
        string manifest = Path.Combine(dir, "package.json");
        Transfer.Require(new FileInfo(manifest).Length < 1024 * 1024, "Package manifest is too large.");
        var data = JsonSerializer.Deserialize<PackageManifest>(File.ReadAllText(manifest)) ?? throw new InvalidDataException("Invalid package manifest.");
        var p = new TransferPackage(dir, data); p.Revalidate(); return p;
    }
    public static async Task<TransferPackage> ExportAsync(TransferResult result, FlashProfile profile, string parent, IProgress<string>? progress, CancellationToken ct)
    {
        profile.Validate(); result = Transfer.Revalidate(result);
        System.IO.Directory.CreateDirectory(parent);
        string dir = Path.Combine(parent, $"Roch-Microcode-{profile.Vendor}-{result.NewRevision:X}-{DateTime.Now:yyyyMMdd-HHmmss}-{Guid.NewGuid():N}");
        System.IO.Directory.CreateDirectory(dir);
        File.WriteAllText(Path.Combine(dir, "INCOMPLETE.txt"), "Verification is incomplete. Do not flash these files.");
        string work = Path.Combine(AppPaths.DataDirectory, "verification-jobs", Guid.NewGuid().ToString("N"));
        try
        {
            var evidence = await IndependentVerification.VerifyAsync(result, work, progress, ct);
            ct.ThrowIfCancellationRequested();
            var manifest = new PackageManifest(2, profile, Firmware.Hash(result.Original), result.Sha256, Firmware.Hash(result.Patch),
                result.Ranges.First(r => r.Reason == "Microcode slot and FF padding").Offset, result.Cpuid, result.OldRevision, result.NewRevision,
                result.Checks.Concat(evidence.Checks).ToArray());
            var package = new TransferPackage(dir, manifest);
            System.IO.Directory.CreateDirectory(Path.GetDirectoryName(package.CandidatePath)!); System.IO.Directory.CreateDirectory(Path.GetDirectoryName(package.RecoveryPath)!);
            Builder.WriteNewVerified(package.CandidatePath, result.Image, manifest.CandidateHash);
            Builder.WriteNewVerified(package.RecoveryPath, result.Original, manifest.OriginalHash);
            Builder.WriteNewVerified(package.PatchPath, result.Patch, manifest.PatchHash);
            string reports = Path.Combine(dir, "verification"); System.IO.Directory.CreateDirectory(reports);
            foreach (string file in System.IO.Directory.EnumerateFiles(work, "*.txt")) File.Copy(file, Path.Combine(reports, Path.GetFileName(file)), false);
            File.WriteAllText(Path.Combine(dir, "READ-ME.txt"), result.Checks.First(c => c.Name == "CPU compatibility").Detail + "\n\n" + profile.Instructions);
            File.WriteAllText(Path.Combine(dir, "package.json"), JsonSerializer.Serialize(manifest, new JsonSerializerOptions { WriteIndented = true }));
            File.WriteAllText(Path.Combine(dir, "validation.json"), JsonSerializer.Serialize(new { manifest, ranges = result.Ranges, limitations = "Structural/integrity verification only. Vendor signatures, rollback/Boot Guard acceptance and hardware stability are not established." }, new JsonSerializerOptions { WriteIndented = true }));
            package.Revalidate(); File.Delete(Path.Combine(dir, "INCOMPLETE.txt"));
            return package;
        }
        catch
        {
            File.WriteAllText(Path.Combine(dir, "INCOMPLETE.txt"), $"Verification/export failed. Do not flash this folder. Diagnostic work: {work}"); throw;
        }
    }
}
