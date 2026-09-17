using RochBios.Core;

if (args.Length < 2) throw new Exception("Pass the ASUS research fixture folder and cross-vendor fixture folder.");
string fixtures = Path.GetFullPath(args[1]);
string Find(string root, string file) => Directory.EnumerateFiles(root, file, SearchOption.AllDirectories).Single();
var cases = new[]
{
    ("ASUS", "ROG STRIX Z790-A GAMING WIFI D4", Find(args[0], "ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-1904.CAP"), Find(args[0], "ROG-STRIX-Z790-A-GAMING-WIFI-D4-ASUS-3202.CAP"), "SZ790AD4.CAP"),
    ("MSI", "Z790 MPOWER (7E01)", Find(fixtures, "E7E01IMS.P00"), Find(fixtures, "E7E01IMS.P90"), "MSI.ROM"),
    ("ASRock", "Z790 Taichi", Find(fixtures, "Z790-Taichi_10.01.ROM"), Find(fixtures, "Z790-Taichi_20.01.ROM"), "CREATIVE.ROM"),
    ("Gigabyte", "Z790 AORUS ELITE AX rev 1.x", Find(fixtures, "Z790AORUSELITEAX11.FF"), Find(fixtures, "Z790AORUSELITEAX11.FH"), "GIGABYTE.bin"),
    ("MSI", "PRO Z790-A WIFI DDR4 (7E07)", Find(fixtures, "E7E07IMS.190"), Find(fixtures, "E7E07IMS.1K0"), "MSI.ROM")
};
int passed = 0;
void Check(string label, Action action) { action(); Console.WriteLine("PASS " + label); passed++; }
void Assert(bool ok) { if (!ok) throw new Exception("Assertion failed"); }
void Reject(Action action) { try { action(); } catch (InvalidDataException) { return; } throw new Exception("Unsupported modification accepted"); }
string output = Path.Combine(fixtures, "transfer-tests-" + Guid.NewGuid().ToString("N")); Directory.CreateDirectory(output);
foreach (var (vendor, board, oldPath, newPath, flashName) in cases)
{
    var old = FirmwareInput.Load(oldPath); var newer = FirmwareInput.Load(newPath);
    var donor = old.Info.Microcodes.Where(m => m.Cpuid == 0xb0671 && m.ChecksumValid).OrderByDescending(m => m.Revision).First();
    var target = newer.Info.Microcodes.Where(m => m.Cpuid == donor.Cpuid && m.ChecksumValid).OrderByDescending(m => m.Revision).First();
    Console.WriteLine($"{vendor} old={donor.RevisionText}, new={target.RevisionText}, raw files={Transfer.Containers(newer.Bytes).Count}, FITs={Transfer.Fits(newer.Bytes, newer.Info.Microcodes).Count}");
    var built = Transfer.Build(newer.Bytes, old.Bytes, donor.Offset, target.Offset);
    if (board.StartsWith("PRO Z790"))
    {
        Check("MSI 1K real mask change reproduced", () => Assert(donor.Platforms == 0x32 && target.Platforms == 0x36 && donor.Revision == 0x11f && target.Revision == 0x137));
        Check("MSI 1K narrower donor is selectable", () => Assert(Transfer.CanReplace(donor, target)));
        Check("MSI 1K result declares donor scope", () => Assert(built.Platforms == 0x32 && built.Checks[0].Detail.Contains("0x36 to 0x32") && built.Checks[0].Detail.Contains("0x04")));
        Check("MSI 1K preserves every donor byte and mask", () => Assert(built.Image.AsSpan(target.Offset, donor.Size).SequenceEqual(old.Bytes.AsSpan(donor.Offset, donor.Size))));
        Check("MSI 1K fills only unused slot tail", () => Assert(built.Image.AsSpan(target.Offset + donor.Size, target.Size - donor.Size).IndexOfAnyExcept((byte)255) < 0));
        foreach (uint flags in new uint[] { 0, 0x08, 0x12 | 0x08, 0x3e })
            Check($"MSI 1K incompatible donor mask {flags:X2} rejected by builder", () =>
            {
                byte[] patch = built.Patch.ToArray();
                // Keep additive checksums valid so this specifically exercises the mask guard.
                System.Buffers.Binary.BinaryPrimitives.WriteUInt32LittleEndian(patch.AsSpan(24), flags);
                System.Buffers.Binary.BinaryPrimitives.WriteUInt32LittleEndian(patch.AsSpan(16), unchecked(Firmware.U32(patch, 16) + donor.Platforms - flags));
                Assert(Firmware.Inspect(patch, "synthetic test donor").Microcodes.Single().ChecksumValid);
                Reject(() => Transfer.Build(newer.Bytes, patch, 0, target.Offset));
            });
        Check("subset still rejects invalid checksum", () => Assert(!Transfer.CanReplace(donor with { ChecksumValid = false }, target)));
        Check("subset still rejects different CPU", () => Assert(!Transfer.CanReplace(donor with { Cpuid = 0xd0670 }, target)));
    }
    Check(vendor + " donor transferred", () => Assert(Firmware.Inspect(built.Image, "result").Microcodes.Single(m => m.Offset == target.Offset).Revision == donor.Revision));
    Check(vendor + " input preserved", () => Assert(Firmware.Hash(newer.Bytes) == newer.Info.Sha256));
    Check(vendor + " repeatable build", () => Assert(Transfer.Revalidate(built).Sha256 == built.Sha256));
    Check(vendor + " mismatched CPU rejected", () => Reject(() => Transfer.Build(newer.Bytes, old.Bytes, old.Info.Microcodes.First(m => m.Cpuid != target.Cpuid && m.ChecksumValid).Offset, target.Offset)));
    Check(vendor + " same revision rejected", () => Reject(() => Transfer.Build(newer.Bytes, newer.Bytes, target.Offset, target.Offset)));
    Check(vendor + " output tampering rejected", () => { var changed = built.Image.ToArray(); changed[200] ^= 1; Reject(() => Transfer.Revalidate(built with { Image = changed })); });
    Check(vendor + " missing active FIT rejected", () => { var changed = newer.Bytes.ToArray(); var fit = Transfer.Fits(changed, newer.Info.Microcodes).Single(); int pointer = (int)(0xffffffc0L + fit.AddressDelta); Array.Fill(changed, (byte)0, pointer, 8); Reject(() => Transfer.Build(changed, old.Bytes, donor.Offset, target.Offset)); });
    Check(vendor + " oversized reverse replacement rejected", () => Reject(() => Transfer.Build(old.Bytes, newer.Bytes, target.Offset, donor.Offset)));
    Check(vendor + " mutated donor rejected", () => { var patch = built.Patch.ToArray(); patch[80] ^= 1; Reject(() => Transfer.Build(newer.Bytes, patch, 0, target.Offset)); });
    Check(vendor + " vendor detection", () => Assert(newer.SuggestedVendor == vendor));
    if (vendor == "ASUS") Check("general engine matches working ASUS reference", () => Assert(built.Sha256 == Recipe.ResultHash));
    var profile = new FlashProfile(vendor, board, flashName);
    var package = await TransferPackage.ExportAsync(built, profile, output, new Progress<string>(Console.WriteLine), CancellationToken.None);
    Check(vendor + " independent verification and package reload", () => Assert(TransferPackage.Open(package.Directory).Revalidate().Sha256 == built.Sha256));
    Check(vendor + " report evidence saved", () => Assert(Directory.GetFiles(Path.Combine(package.Directory, "verification"), "*.txt").Length >= 7));
    Check(vendor + " correct recovery and USB filename", () => Assert(Path.GetFileName(package.RecoveryPath) == flashName && Firmware.Hash(Firmware.ReadImage(package.RecoveryPath)) == newer.Info.Sha256));
    if (board.StartsWith("PRO Z790")) Check("MSI narrower scope survives package reload and README", () => Assert(TransferPackage.Open(package.Directory).Revalidate().Platforms == 0x32 && File.ReadAllText(Path.Combine(package.Directory, "READ-ME.txt")).Contains("0x36 to 0x32")));
    Check(vendor + " package metadata tampering rejected", () => Reject(() => (package with { Manifest = package.Manifest with { ToRevision = 0 } }).Revalidate()));
    Check(vendor + " filename traversal rejected", () => Reject(() => (profile with { FileName = "../other.CAP" }).Validate()));
}
foreach (string zip in new[] { "msi-P0.zip", "msi-P9.zip", "asrock-old.zip", "asrock-new.zip", "gigabyte-ff.zip", "gigabyte-fh.zip" })
    Check(zip + " direct ZIP input", () => Assert(FirmwareInput.Load(Path.Combine(fixtures, zip)).Info.Microcodes.Count > 0));
Check("reserved Windows filename rejected", () => Reject(() => new FlashProfile("ASUS", "Test", "COM9.CAP").Validate()));
Check("ambiguous ZIP rejected", () =>
{
    string file = Path.Combine(output, "ambiguous.zip");
    using (var zip = System.IO.Compression.ZipFile.Open(file, System.IO.Compression.ZipArchiveMode.Create))
    {
        System.IO.Compression.ZipFileExtensions.CreateEntryFromFile(zip, cases[0].Item3, "old.cap");
        System.IO.Compression.ZipFileExtensions.CreateEntryFromFile(zip, cases[0].Item4, "new.cap");
    }
    Reject(() => FirmwareInput.Load(file));
});
Check("FFS checksummed payload repair", () =>
{
    var a = FirmwareInput.Load(cases[0].Item4); var d = FirmwareInput.Load(cases[0].Item3);
    var t = a.Info.Microcodes.Single(m => m.Cpuid == 0xb0671); var donor = d.Info.Microcodes.Single(m => m.Cpuid == 0xb0671);
    var f = Transfer.Containers(a.Bytes).Single(f => t.Offset >= f.Body && t.Offset + t.Size <= f.End);
    byte[] b = a.Bytes.ToArray(); b[f.Offset + 19] |= 0x40;
    int sum = 0; for (int i = f.Body; i < f.End; i++) sum = (sum + b[i]) & 255; b[f.Offset + 17] = (byte)((256 - sum) & 255);
    b[f.Offset + 16] = 0; sum = 0; for (int i = f.Offset; i < f.Body; i++) if (i != f.Offset + 17 && i != f.Offset + 23) sum += b[i]; b[f.Offset + 16] = (byte)((256 - sum) & 255);
    var result = Transfer.Build(b, d.Bytes, donor.Offset, t.Offset);
    sum = result.Image[f.Offset + 17]; for (int i = f.Body; i < f.End; i++) sum = (sum + result.Image[i]) & 255;
    Assert(sum == 0 && Transfer.Revalidate(result).Sha256 == result.Sha256);
});
Console.WriteLine($"{passed} transfer tests passed. Output: {output}");
