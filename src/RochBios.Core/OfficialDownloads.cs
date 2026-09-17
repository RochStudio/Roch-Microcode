using System.IO.Compression;

namespace RochBios.Core;

public static class OfficialDownloads
{
    private static readonly HttpClient Client = new(new HttpClientHandler { AllowAutoRedirect = false }) { Timeout = TimeSpan.FromMinutes(4) };
    public static async Task<string> FetchAsync(DownloadSpec spec, string cache, IProgress<string>? progress, CancellationToken ct)
    {
        ct.ThrowIfCancellationRequested();
        Directory.CreateDirectory(cache);
        string cap = Path.Combine(cache, spec.CapName);
        if (File.Exists(cap))
        {
            try
            {
                if (Firmware.Hash(Firmware.ReadImage(cap)) == spec.CapHash)
                { progress?.Report($"BIOS {spec.Version}: verified cached file"); return cap; }
            }
            catch (InvalidDataException) { /* A truncated cache is replaced only after a fresh download verifies. */ }
            progress?.Report($"BIOS {spec.Version}: cached file needs replacement");
        }
        string zipPath = Path.Combine(cache, $"asus-{spec.Version}-{Guid.NewGuid():N}.download");
        string capTemp = Path.Combine(cache, $"{Guid.NewGuid():N}.cap.tmp");
        try
        {
            using var response = await Client.GetAsync(spec.Url, HttpCompletionOption.ResponseHeadersRead, ct);
            response.EnsureSuccessStatusCode();
            using (var input = await response.Content.ReadAsStreamAsync(ct))
            using (var output = new FileStream(zipPath, FileMode.CreateNew, FileAccess.Write, FileShare.None))
            {
                byte[] buffer = new byte[65536]; long total = 0; int n;
                while ((n = await input.ReadAsync(buffer, ct)) > 0)
                {
                    total += n;
                    if (total > 64 * 1024 * 1024) throw new InvalidDataException("Download exceeded the expected size limit.");
                    await output.WriteAsync(buffer.AsMemory(0, n), ct);
                    progress?.Report($"Downloading BIOS {spec.Version} • {total / 1048576.0:0.0} MB");
                }
            }
            ct.ThrowIfCancellationRequested();
            if (Firmware.Hash(await File.ReadAllBytesAsync(zipPath, ct)) != spec.ZipHash)
                throw new InvalidDataException($"BIOS {spec.Version} ZIP checksum does not match the pinned ASUS package. No file was accepted.");
            using var zip = ZipFile.OpenRead(zipPath);
            var entry = zip.Entries.SingleOrDefault(e => e.FullName.Equals(spec.CapName, StringComparison.OrdinalIgnoreCase))
                ?? throw new InvalidDataException("Expected CAP not found in the ASUS package.");
            if (entry.Length != 33558528) throw new InvalidDataException("Unexpected CAP size.");
            using var entryStream = entry.Open();
            using var ms = new MemoryStream();
            byte[] chunk = new byte[65536]; int count;
            while ((count = await entryStream.ReadAsync(chunk, ct)) > 0)
            {
                if (ms.Length + count > 33558528) throw new InvalidDataException("CAP exceeds expected size.");
                ms.Write(chunk, 0, count);
            }
            var bytes = ms.ToArray();
            if (Firmware.Hash(bytes) != spec.CapHash) throw new InvalidDataException("Extracted CAP hash is incorrect.");
            ct.ThrowIfCancellationRequested();
            Builder.WriteNewVerified(capTemp, bytes, spec.CapHash);
            File.Move(capTemp, cap, true);
            progress?.Report($"BIOS {spec.Version} downloaded and SHA-256 verified");
            return cap;
        }
        finally
        {
            if (File.Exists(zipPath)) File.Delete(zipPath);
            if (File.Exists(capTemp)) File.Delete(capTemp);
        }
    }
}
