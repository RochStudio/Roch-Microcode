using System.Buffers.Binary;
using System.Globalization;
using System.Security.Cryptography;

namespace RochBios.Core;

public sealed record Microcode(int Offset, uint Cpuid, uint Platforms, uint Revision, int Size, string Date,
    bool ChecksumValid, bool KnownOemHeader = false)
{
    public string CpuText => $"{Cpuid:X5}";
    public string RevisionText => $"0x{Revision:X}";
    public string PlatformText => $"0x{Platforms:X2}";
    public string OffsetText => $"0x{Offset:X}";
    public string SizeText => $"{Size / 1024.0:0.#} KB";
    public string Status => ChecksumValid ? "Valid checksum" : KnownOemHeader ? "ASUS alternate header" : "Invalid checksum";
    public string Display => $"{CpuText}/{Platforms:X2} • {RevisionText} • {SizeText} • @{Offset:X}";
    public override string ToString() => Display;
}

public sealed record FirmwareInfo(string FileName, string Sha256, int Size, string Identity, IReadOnlyList<Microcode> Microcodes)
{
    public bool IsSupportedBase => Sha256 == Recipe.BaseHash;
    public bool IsKnownCandidate => Sha256 == Recipe.ResultHash;
}

public static class Firmware
{
    public const int MaxImageSize = 128 * 1024 * 1024;
    public static string Hash(ReadOnlySpan<byte> bytes) => Convert.ToHexStringLower(SHA256.HashData(bytes));
    public static uint U32(ReadOnlySpan<byte> b, int p) => BinaryPrimitives.ReadUInt32LittleEndian(b.Slice(p, 4));
    public static ulong U64(ReadOnlySpan<byte> b, int p) => BinaryPrimitives.ReadUInt64LittleEndian(b.Slice(p, 8));
    public static bool Sum32Valid(ReadOnlySpan<byte> bytes)
    {
        if (bytes.Length % 4 != 0) return false;
        uint sum = 0;
        for (int p = 0; p < bytes.Length; p += 4) sum = unchecked(sum + U32(bytes, p));
        return sum == 0;
    }
    public static byte[] ReadImage(string path)
    {
        var info = new FileInfo(path);
        if (info.Length < 48 || info.Length > MaxImageSize) throw new InvalidDataException("Choose a firmware image between 48 bytes and 128 MB.");
        // Bound the read even if the file changes after the size check.
        using var input = File.OpenRead(path);
        using var buffer = new MemoryStream();
        var chunk = new byte[65536]; int n;
        while ((n = input.Read(chunk)) > 0)
        {
            if (buffer.Length + n > MaxImageSize) throw new InvalidDataException("Firmware image exceeds 128 MB.");
            buffer.Write(chunk, 0, n);
        }
        return buffer.ToArray();
    }
    public static FirmwareInfo Inspect(byte[] image, string fileName)
    {
        var hash = Hash(image);
        bool trustedAsus = hash is Recipe.BaseHash or Recipe.ResultHash or Recipe.DonorHash;
        var found = new List<Microcode>();
        for (int p = 0; p <= image.Length - 48; p += 4)
        {
            if (U32(image, p) != 1 || U32(image, p + 20) != 1) continue;
            uint cpu = U32(image, p + 12), rev = U32(image, p + 4), flags = U32(image, p + 24);
            uint size = U32(image, p + 32), dataSize = U32(image, p + 28);
            if (size == 0) size = 2048;
            if (dataSize == 0) dataSize = 2000;
            if (size < 2048 || size > 2 * 1024 * 1024 || size % 1024 != 0 || dataSize > size - 48 || dataSize % 4 != 0 || size > image.Length - p) continue;
            if (cpu == 0 || rev == 0 || flags > 255) continue;
            var dateHex = U32(image, p + 8).ToString("X8");
            if (!DateTime.TryParseExact(dateHex, "MMddyyyy", CultureInfo.InvariantCulture, DateTimeStyles.None, out var date)) continue;
            bool valid = Sum32Valid(image.AsSpan(p, (int)size)) && Sum32Valid(image.AsSpan(p, 48 + (int)dataSize));
            // Known ASUS alternate patch has an intentionally changed CPUID. Never exempt unknown files.
            bool oem = trustedAsus && cpu == 0xff0671 && rev == 0x104;
            found.Add(new(p, cpu, flags, rev, (int)size, date.ToString("yyyy-MM-dd"), valid, oem));
            p += (int)size - 4;
        }
        string identity = hash switch
        {
            Recipe.BaseHash => "ASUS Z790-A D4 • official BIOS 3202",
            Recipe.DonorHash => "ASUS Z790-A D4 • official BIOS 1904 donor",
            Recipe.PatchHash => "Intel B0671 • 0x11F donor patch",
            Recipe.ResultHash => "ASUS Z790-A D4 • BIOS 3202 + 0x11F",
            _ => "Firmware image • compatibility checked when building"
        };
        return new(fileName, hash, image.Length, identity, found);
    }
}

public static class Recipe
{
    public const string Board = "ROG STRIX Z790-A GAMING WIFI D4";
    public const string FlashName = "SZ790AD4.CAP";
    public const string BaseHash = "4000ef2f68a14c64b8c7c968478d3414927c325b528266fe4f9ebf3cced42762";
    public const string DonorHash = "d3ba71da21876c403440ca8f633997b606dfc3fde868fe75db42142d1455302c";
    public const string PatchHash = "1d0b38eb084a284961d58a2fb3a90313ff94853dc5da9f7348505f5ef68b57f2";
    public const string ResultHash = "4b7bf8c4550e5cbe4c8d4594ce8e90368f8436064ec8f80ba3a494c2d3656b5a";
    public const int Start = 0x10f9400, OldSize = 0x35800, NewSize = 0x33c00, DonorStart = 0x10f8000;
    public const string SupportUrl = "https://www.asus.com/supportonly/rog%20strix%20z790-a%20gaming%20wifi%20d4/helpdesk_bios/";
    public static DownloadSpec BaseDownload => new("3202", "dfad9bbd7f22aec2d76f384954606d7bc453fc601896ffe02fab2d2ded633d6c", BaseHash);
    public static DownloadSpec DonorDownload => new("1904", "84664987f88b9f51f3e41e1345e09f0b8e66deb1351a424ffbadc671152d5d42", DonorHash);
}

public sealed record DownloadSpec(string Version, string ZipHash, string CapHash)
{
    public string CapName => $"ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-{Version}.CAP";
    public string Url => $"https://dlcdnets.asus.com/pub/ASUS/mb/BIOS/ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-{Version}.zip";
}
