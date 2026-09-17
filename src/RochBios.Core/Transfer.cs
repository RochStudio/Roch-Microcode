using System.Buffers.Binary;

namespace RochBios.Core;

public sealed record FfsContainer(int Offset, int HeaderSize, int Size, byte Attributes)
{
    public int Body => Offset + HeaderSize;
    public int End => Offset + Size;
}
public sealed record FitTable(int Offset, int Size, long AddressDelta, IReadOnlyList<int> MicrocodeOffsets);
public sealed record ChangeRange(int Offset, int Size, string Reason);
public sealed record TransferResult(byte[] Original, byte[] Patch, byte[] Image, uint Cpuid, uint Platforms,
    uint OldRevision, uint NewRevision, IReadOnlyList<ChangeRange> Ranges, IReadOnlyList<ValidationCheck> Checks)
{
    public string Sha256 => Firmware.Hash(Image);
}

public static class Transfer
{
    // A platform mask is a set of platform IDs, not an indivisible identifier.
    // A narrower donor is usable only within its own declared primary-CPU scope.
    // Do not accept arbitrary overlap or infer compatibility from extended aliases.
    public static bool CanReplace(Microcode donor, Microcode target) =>
        donor.ChecksumValid && target.ChecksumValid && donor.Cpuid == target.Cpuid &&
        donor.Platforms != 0 && (donor.Platforms & target.Platforms) == donor.Platforms;

    public static string CompatibilityDetail(Microcode donor, Microcode target) =>
        donor.Platforms == target.Platforms
            ? $"Primary CPUID {donor.Cpuid:X}; platform mask 0x{donor.Platforms:X2} matches exactly. Extended CPU coverage is not guaranteed."
            : $"Primary CPUID {donor.Cpuid:X}; platform mask narrows from 0x{target.Platforms:X2} to 0x{donor.Platforms:X2}. " +
              $"Use only for that CPU signature on a platform covered by 0x{donor.Platforms:X2}. " +
              $"This patch no longer covers platform bits 0x{target.Platforms & ~donor.Platforms:X2}. Extended CPU coverage is not guaranteed.";

    internal static void Require(bool ok, string error) { if (!ok) throw new InvalidDataException(error); }
    static int Size24(byte[] b, int p) => b[p] | b[p + 1] << 8 | b[p + 2] << 16;
    static int Align8(int p) => (p + 7) & ~7;
    static int Sum8(ReadOnlySpan<byte> b) { int s = 0; foreach (byte x in b) s = (s + x) & 255; return s; }
    public static IReadOnlyList<FfsContainer> Containers(byte[] b)
    {
        var files = new List<FfsContainer>();
        for (int p = 0; p <= b.Length - 72; p += 8)
        {
            if (!b.AsSpan(p + 40, 4).SequenceEqual("_FVH"u8) || b[p + 55] != 2) continue;
            ulong length = Firmware.U64(b, p + 32);
            int header = BinaryPrimitives.ReadUInt16LittleEndian(b.AsSpan(p + 48));
            if (length > (ulong)(b.Length - p) || length < 72 || header < 72 || (ulong)header > length || (header & 1) != 0) continue;
            uint sum = 0; for (int n = 0; n < header; n += 2) sum += BinaryPrimitives.ReadUInt16LittleEndian(b.AsSpan(p + n));
            if ((sum & 65535) != 0) continue;
            var fs = new Guid(b.AsSpan(p + 16, 16));
            if (fs != new Guid("8C8CE578-8A3D-4F1C-9935-896185C32DD3") && fs != new Guid("5473C07A-3DCB-4DCA-BD6F-1E9689E7349A")) continue;
            int end = p + (int)length, first = p + header;
            int ext = BinaryPrimitives.ReadUInt16LittleEndian(b.AsSpan(p + 52));
            if (ext != 0)
            {
                if ((ulong)ext > length - 20) continue;
                uint extSize = Firmware.U32(b, p + ext + 16);
                if (extSize < 20 || extSize > length - (uint)ext) continue;
                first = Math.Max(first, p + ext + (int)extSize);
            }
            for (int f = Align8(first); f <= end - 24;)
            {
                if (b.AsSpan(f, 24).IndexOfAnyExcept((byte)255) < 0 || b.AsSpan(f, 24).IndexOfAnyExcept((byte)0) < 0) break;
                int h = 24, size = Size24(b, f + 20);
                if (size == 0xffffff)
                {
                    if ((b[f + 19] & 1) == 0 || f > end - 32) break;
                    ulong big = Firmware.U64(b, f + 24); if (big > (ulong)(end - f)) break;
                    h = 32; size = (int)big;
                }
                if (size < h || size > end - f) break;
                bool validHeader = ((Sum8(b.AsSpan(f, h)) - b[f + 17] - b[f + 23]) & 255) == 0;
                // Only direct raw FFS bodies are edited. No compressed or guided sections are rebuilt.
                if (validHeader && b[f + 18] == 1) files.Add(new(f, h, size, b[f + 19]));
                f = Align8(f + size);
            }
        }
        return files.Distinct().ToArray();
    }
    public static IReadOnlyList<FitTable> Fits(byte[] b, IReadOnlyList<Microcode> codes)
    {
        var fits = new List<FitTable>();
        for (int p = 0; p <= b.Length - 32; p += 16)
        {
            if (!b.AsSpan(p, 8).SequenceEqual("_FIT_   "u8)) continue;
            uint count = Firmware.U32(b, p + 8) & 0xffffff;
            if (count < 2 || count > 4096 || count * 16 > b.Length - p || (b[p + 14] & 127) != 0) continue;
            int size = (int)count * 16;
            if ((b[p + 14] & 128) != 0 && Sum8(b.AsSpan(p, size)) != 0) continue;
            var addresses = new List<ulong>();
            for (int i = 1; i < count; i++) if ((b[p + i * 16 + 14] & 127) == 1) addresses.Add(Firmware.U64(b, p + i * 16));
            if (addresses.Count == 0 || addresses.Any(a => a > uint.MaxValue || a < 0x80000000)) continue;
            var matches = new List<FitTable>();
            foreach (var c in codes)
            {
                long delta = c.Offset - (long)addresses[0];
                long pointer = 0xffffffc0L + delta;
                if (pointer < 0 || pointer > b.Length - 8 || Firmware.U64(b, (int)pointer) != (ulong)(p - delta)) continue;
                var offsets = addresses.Select(a => (long)a + delta).ToArray();
                if (offsets.All(o => codes.Any(m => m.Offset == o))) matches.Add(new(p, size, delta, offsets.Select(o => (int)o).ToArray()));
            }
            Require(matches.Count <= 1, "The FIT address mapping is ambiguous.");
            fits.AddRange(matches);
        }
        return fits;
    }
    public static TransferResult Build(byte[] newer, byte[] older, int donorOffset, int targetOffset)
    {
        Require(newer.Length <= Firmware.MaxImageSize && older.Length <= Firmware.MaxImageSize, "Image exceeds 128 MB.");
        var targetCodes = Firmware.Inspect(newer, "new BIOS").Microcodes;
        var donorCodes = Firmware.Inspect(older, "old BIOS").Microcodes;
        var donor = donorCodes.SingleOrDefault(m => m.Offset == donorOffset) ?? throw new InvalidDataException("Selected old microcode no longer exists.");
        var target = targetCodes.SingleOrDefault(m => m.Offset == targetOffset) ?? throw new InvalidDataException("Selected new microcode no longer exists.");
        Require(donor.ChecksumValid && target.ChecksumValid, "Selected microcode checksum is invalid. Modified OEM aliases cannot be used as the donor or target.");
        Require(CanReplace(donor, target), "Primary CPU signatures must match and the donor's nonzero platform mask must be equal to or a subset of the target mask. Partial overlap is not supported.");
        Require(donor.Revision != target.Revision, "The selected revisions are already the same.");
        var targets = targetCodes.Where(m => m.Cpuid == target.Cpuid && m.Platforms == target.Platforms && m.Revision == target.Revision).ToArray();
        var files = Containers(newer);
        var fits = Fits(newer, targetCodes);
        Require(fits.Count != 0, "No unambiguous active Intel FIT table was found. This firmware layout is not supported for editing.");
        byte[] patch = older.AsSpan(donor.Offset, donor.Size).ToArray();
        byte[] candidate = newer.ToArray();
        var ranges = new List<ChangeRange>(); var owners = new HashSet<FfsContainer>();
        foreach (var t in targets)
        {
            Require(t.ChecksumValid, "A duplicate target has an invalid checksum.");
            Require(donor.Size <= t.Size, $"Old microcode needs {donor.Size:N0} bytes; the new slot has {t.Size:N0}. Relocation or firmware rebuilding would be needed.");
            Require(fits.Any(f => f.MicrocodeOffsets.Contains(t.Offset)), $"Target at 0x{t.Offset:X} is not referenced by the active FIT. A recovery copy or different layout needs separate support.");
            var containers = files.Where(f => t.Offset >= f.Body && (long)t.Offset + t.Size <= f.End).ToArray();
            Require(containers.Length == 1, $"Target at 0x{t.Offset:X} is not inside one validated, uncompressed raw FFS file.");
            var owner = containers[0];
            bool checksum = (owner.Attributes & 0x40) != 0;
            Require(checksum ? (Sum8(newer.AsSpan(owner.Body, owner.End - owner.Body)) + newer[owner.Offset + 17]) % 256 == 0 : newer[owner.Offset + 17] == 0xaa, "Original FFS data checksum is invalid.");
            patch.CopyTo(candidate, t.Offset);
            Array.Fill(candidate, (byte)255, t.Offset + donor.Size, t.Size - donor.Size);
            ranges.Add(new(t.Offset, t.Size, "Microcode slot and FF padding")); owners.Add(owner);
        }
        foreach (var f in owners.Where(f => (f.Attributes & 0x40) != 0))
        {
            candidate[f.Offset + 17] = (byte)((256 - Sum8(candidate.AsSpan(f.Body, f.End - f.Body))) & 255);
            if (candidate[f.Offset + 17] != newer[f.Offset + 17]) ranges.Add(new(f.Offset + 17, 1, "FFS data checksum"));
        }
        for (int i = 0; i < newer.Length; i++)
            if (newer[i] != candidate[i]) Require(ranges.Any(r => i >= r.Offset && i < r.Offset + r.Size), "Unexpected change outside the authorized ranges.");
        var after = Firmware.Inspect(candidate, "candidate").Microcodes;
        Require(after.Count == targetCodes.Count, "Microcode inventory count changed unexpectedly.");
        foreach (var t in targetCodes)
        {
            var a = after.SingleOrDefault(m => m.Offset == t.Offset);
            Require(a is not null, "A microcode moved or disappeared.");
            if (targets.Contains(t)) Require(a!.ChecksumValid && a.Revision == donor.Revision && a.Size == donor.Size && candidate.AsSpan(a.Offset, a.Size).SequenceEqual(patch), "Replacement validation failed.");
            else Require(newer.AsSpan(t.Offset, t.Size).SequenceEqual(candidate.AsSpan(t.Offset, t.Size)), "An unselected microcode changed.");
        }
        var afterFits = Fits(candidate, after);
        Require(afterFits.Count == fits.Count, "FIT resolution changed after modification.");
        foreach (var fit in fits) Require(newer.AsSpan(fit.Offset, fit.Size).SequenceEqual(candidate.AsSpan(fit.Offset, fit.Size)), "FIT table bytes changed.");
        return new(newer.ToArray(), patch, candidate, donor.Cpuid, donor.Platforms, target.Revision, donor.Revision, ranges,
        [new("CPU compatibility", CompatibilityDetail(donor, target)),
         new("Replacement integrity", $"{targets.Length} matching slot(s); donor bytes and Intel checksums verified"),
         new("Firmware structure", "Validated raw FFS container; data checksum maintained"),
         new("FIT pointers", $"{fits.Count} active table(s); original addresses and entries preserved"),
         new("Unchanged content", "Only selected slots, FF padding and any required FFS checksum byte changed"),
         new("Other microcodes", $"All {targetCodes.Count - targets.Length} unselected microcodes remain byte-identical")]);
    }
    public static TransferResult Revalidate(TransferResult result)
    {
        int target = result.Ranges.First(r => r.Reason == "Microcode slot and FF padding").Offset;
        var rebuilt = Build(result.Original, result.Patch, 0, target);
        Require(rebuilt.Image.AsSpan().SequenceEqual(result.Image), "The candidate was changed after construction.");
        return rebuilt;
    }
}
