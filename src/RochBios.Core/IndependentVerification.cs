using System.Diagnostics;
using System.Globalization;
using System.IO.Compression;
using System.Text;
using System.Text.RegularExpressions;

namespace RochBios.Core;

public sealed record VerificationEvidence(string Directory, IReadOnlyList<ValidationCheck> Checks);
public static class IndependentVerification
{
    static readonly SemaphoreSlim InstallLock = new(1, 1);
    static async Task<string> ToolsAsync(CancellationToken ct)
    {
        await InstallLock.WaitAsync(ct);
        try
        {
            using var embedded = typeof(IndependentVerification).Assembly.GetManifestResourceStream("RochBios.VerificationBundle.zip")
                ?? throw new IOException("The bundled verification tools are missing. Reinstall Roch Microcode.");
            using var memory = new MemoryStream(); await embedded.CopyToAsync(memory, ct);
            string hash = Firmware.Hash(memory.ToArray()); memory.Position = 0;
            string root = Path.Combine(AppPaths.DataDirectory, "verifiers", hash[..16]);
            Directory.CreateDirectory(root);
            using var zip = new ZipArchive(memory);
            foreach (var entry in zip.Entries)
            {
                ct.ThrowIfCancellationRequested(); if (entry.FullName.EndsWith('/')) continue;
                string path = Path.GetFullPath(Path.Combine(root, entry.FullName));
                Transfer.Require(path.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase), "Invalid bundled tool path.");
                using var data = new MemoryStream(); using (var stream = entry.Open()) await stream.CopyToAsync(data, ct);
                var bytes = data.ToArray();
                if (File.Exists(path) && Firmware.Hash(await File.ReadAllBytesAsync(path, ct)) == Firmware.Hash(bytes)) continue;
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                await File.WriteAllBytesAsync(path, bytes, ct);
            }
            return root;
        }
        finally { InstallLock.Release(); }
    }
    static async Task<string> Run(string exe, IEnumerable<string> args, string work, CancellationToken ct)
    {
        var start = new ProcessStartInfo(exe) { WorkingDirectory = work, UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true, StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8 };
        foreach (string arg in args) start.ArgumentList.Add(arg);
        using var process = Process.Start(start) ?? throw new IOException("Could not launch verification tool.");
        var stdout = process.StandardOutput.ReadToEndAsync(ct); var stderr = process.StandardError.ReadToEndAsync(ct);
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct); timeout.CancelAfter(TimeSpan.FromMinutes(2));
        try { await process.WaitForExitAsync(timeout.Token); }
        catch (OperationCanceledException) { try { process.Kill(true); } catch { } ct.ThrowIfCancellationRequested(); throw new IOException("A verification tool timed out; no verified package was produced."); }
        string output = await stdout, error = await stderr;
        if (process.ExitCode != 0) throw new InvalidDataException($"{Path.GetFileName(exe)} failed ({process.ExitCode}). {error} {output[..Math.Min(1200, output.Length)]}");
        Transfer.Require(!error.Contains("Traceback", StringComparison.OrdinalIgnoreCase), "MCExtractor reported an exception.");
        return Regex.Replace(output + (error.Length == 0 ? "" : "\n" + error), @"\x1B\[[0-?]*[ -/]*[@-~]", "");
    }
    public static async Task<VerificationEvidence> VerifyAsync(TransferResult result, string work, IProgress<string>? progress, CancellationToken ct)
    {
        result = Transfer.Revalidate(result);
        Directory.CreateDirectory(work);
        progress?.Report("Preparing bundled UEFIExtract and MCExtractor…");
        string tools = await ToolsAsync(ct);
        string original = Path.Combine(work, "new-original.bin"), candidate = Path.Combine(work, "modified.bin"), donor = Path.Combine(work, "donor-microcode.bin");
        Builder.WriteNewVerified(original, result.Original, Firmware.Hash(result.Original));
        Builder.WriteNewVerified(candidate, result.Image, result.Sha256);
        Builder.WriteNewVerified(donor, result.Patch, Firmware.Hash(result.Patch));
        progress?.Report("UEFIExtract: comparing original and modified firmware structure…");
        var uefiOriginal = await Run(Path.Combine(tools, "UEFIExtract.exe"), [original, "report"], work, ct);
        var uefiCandidate = await Run(Path.Combine(tools, "UEFIExtract.exe"), [candidate, "report"], work, ct);
        await File.WriteAllTextAsync(Path.Combine(work, "original.uefi.log.txt"), uefiOriginal, ct);
        await File.WriteAllTextAsync(Path.Combine(work, "modified.uefi.log.txt"), uefiCandidate, ct);
        CompareMessages(uefiOriginal, uefiCandidate, false);
        CompareStructure(await File.ReadAllTextAsync(original + ".report.txt", ct), await File.ReadAllTextAsync(candidate + ".report.txt", ct), result.Ranges);
        progress?.Report("MCExtractor: checking donor, original inventory and modified inventory…");
        async Task<string> Mce(string input, string label)
        {
            // MCE writes extraction files next to its script. Each invocation gets a private working copy.
            string dir = Path.Combine(work, "mce-work-" + label); Directory.CreateDirectory(dir);
            foreach (string name in new[] { "MCE.py", "MCE.db" }) File.Copy(Path.Combine(tools, "MCE", name), Path.Combine(dir, name), false);
            string output = await Run(Path.Combine(tools, "python", "python.exe"), ["-I", "-B", "-X", "utf8", Path.Combine(dir, "MCE.py"), input, "-skip", "-exit", "-duc"], dir, ct);
            await File.WriteAllTextAsync(Path.Combine(work, label + ".mce.log.txt"), output, ct);
            return output;
        }
        string mceOriginal = await Mce(original, "original"), mceCandidate = await Mce(candidate, "modified"), mceDonor = await Mce(donor, "donor");
        CompareMessages(mceOriginal, mceCandidate, true); CompareMessages("", mceDonor, true);
        MatchMce(mceOriginal, Firmware.Inspect(result.Original, "original").Microcodes);
        MatchMce(mceCandidate, Firmware.Inspect(result.Image, "modified").Microcodes);
        MatchMce(mceDonor, Firmware.Inspect(result.Patch, "donor").Microcodes);
        return new(work, [new("UEFIExtract A75", "No new parser diagnostics; unchanged structure and CRCs outside the authorized changes"),
            new("MCExtractor 1.104.0 / r352", "Donor and both inventories independently matched; no new warnings or errors")]);
    }
    internal static void CompareMessages(string before, string after, bool mce)
    {
        string[] Lines(string text) => text.Split('\n').Select(l => l.Trim()).Where(l => l.Length != 0 && ((!mce && l.Contains("::")) || l.Contains("Error:", StringComparison.OrdinalIgnoreCase) || l.Contains("Warning:", StringComparison.OrdinalIgnoreCase) || l.Contains("Traceback", StringComparison.OrdinalIgnoreCase))).ToArray();
        var counts = Lines(before).GroupBy(l => l).ToDictionary(g => g.Key, g => g.Count());
        foreach (string line in Lines(after))
        {
            Transfer.Require(counts.TryGetValue(line, out int count) && count > 0, "Independent verifier reported a new diagnostic: " + line);
            counts[line]--;
        }
    }
    internal static void CompareStructure(string before, string after, IReadOnlyList<ChangeRange> ranges)
    {
        List<string> Normalize(string text)
        {
            var rows = new List<string>();
            foreach (string line in text.Split('\n'))
            {
                var c = line.Split('|').Select(v => v.Trim()).ToArray();
                if (c.Length >= 6 && c[2] == "N/A") { rows.Add(string.Join('|', c)); continue; }
                if (c.Length < 6 || !int.TryParse(c[2], NumberStyles.HexNumber, CultureInfo.InvariantCulture, out int start) || !int.TryParse(c[3], NumberStyles.HexNumber, CultureInfo.InvariantCulture, out int size)) continue;
                long end = (long)start + size;
                bool overlaps = ranges.Any(r => start < (long)r.Offset + r.Size && end > r.Offset);
                // UEFIExtract may merge the new FF tail with padding immediately after the old slot.
                // Native byte comparison has already proven that all bytes beyond the slot are unchanged.
                if (c[0] == "Padding" && ranges.Any(r => r.Reason == "Microcode slot and FF padding" && start <= (long)r.Offset + r.Size && end >= r.Offset)) continue;
                if (overlaps && c[0] == "Intel microcode") continue;
                if (ranges.Any(r => start <= r.Offset && end >= (long)r.Offset + r.Size)) c[4] = "EXPECTED-ANCESTOR-CRC";
                rows.Add(string.Join('|', c));
            }
            return rows;
        }
        var a = Normalize(before); var b = Normalize(after);
        Transfer.Require(a.Count >= 5, "UEFIExtract did not produce a usable firmware tree.");
        Transfer.Require(a.SequenceEqual(b), "UEFIExtract found an unexpected firmware tree or CRC difference outside the replaced microcode. Review the reports.");
    }
    internal static void MatchMce(string output, IReadOnlyList<Microcode> expected)
    {
        var seen = new List<(uint Cpu, uint Platforms, uint Revision, int Size, int Offset)>();
        foreach (string line in output.Split('\n'))
        {
            if (line.TrimStart().StartsWith("Extended_", StringComparison.Ordinal)) break;
            var c = line.Split('│').Select(s => s.Trim(' ', '║', '\r')).ToArray();
            if (c.Length < 10 || c[1] != "Microcode") continue;
            uint Hex(string s) => uint.Parse(s.Replace("0x", "", StringComparison.OrdinalIgnoreCase), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
            seen.Add((Hex(c[2]), Hex(c[3].Split(' ')[0]), Hex(c[4]), (int)Hex(c[7]), (int)Hex(c[8])));
        }
        Transfer.Require(seen.Count == expected.Count && expected.All(m => seen.Contains((m.Cpuid, m.Platforms, m.Revision, m.Size, m.Offset))), "MCExtractor inventory disagrees with the native parser.");
    }
}
