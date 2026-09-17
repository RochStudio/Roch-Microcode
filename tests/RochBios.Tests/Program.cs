using RochBios.Core;

int passed = 0;
void Check(string name, Action action) { action(); passed++; Console.WriteLine($"PASS {name}"); }
void Assert(bool condition, string message = "Assertion failed") { if (!condition) throw new Exception(message); }
void Reject(Action action) { try { action(); } catch (Exception ex) when (ex is IOException or InvalidDataException) { return; } throw new Exception("Unsafe input was accepted"); }
var root = Path.Combine(Path.GetTempPath(), "roch-bios-tests-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(root);
var goodUsb = new UsbDrive(3, "stable-disk-id", "R:\\", "TEST", "USB", "MBR", "FAT32", 1, false, false, 16000000000, 1000000000, 1048576, "volume-id");
Check("eligible USB", () => { Assert(goodUsb.Rejection is null); goodUsb.RequireSameEligible(goodUsb); });
Check("reject boot USB", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { IsBoot = true })));
Check("reject system USB", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { IsSystem = true })));
Check("reject non USB", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { Bus = "SATA" })));
Check("reject GPT", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { PartitionStyle = "GPT" })));
Check("reject NTFS", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { FileSystem = "NTFS" })));
Check("reject multiple partitions", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { PartitionCount = 2 })));
Check("reject full disk", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { Free = 100 })));
Check("allow reduced space after staging", () => goodUsb.RequireSameEligible(goodUsb with { Free = 100 }, false));
Check("reject replaced disk", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { UniqueId = "different" })));
Check("reject replaced volume", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { VolumeId = "different" })));
Check("reject changed offset", () => Reject(() => goodUsb.RequireSameEligible(goodUsb with { Offset = 0 })));
Check("reject ambiguous identity", () => Assert((goodUsb with { UniqueId = "" }).Rejection is not null));
Check("reject invalid root", () => Assert((goodUsb with { Root = "R:\\directory" }).Rejection is not null));
Check("empty image scanner", () => Assert(Firmware.Inspect([], "empty").Microcodes.Count == 0));
Check("truncated image read", () => { string p = Path.Combine(root, "tiny.bin"); File.WriteAllBytes(p, [1, 2, 3]); Reject(() => Firmware.ReadImage(p)); });
Check("oversized image bounded", () => { string p = Path.Combine(root, "large.bin"); using (var f = File.Create(p)) f.SetLength(Firmware.MaxImageSize + 1L); Reject(() => Firmware.ReadImage(p)); File.Delete(p); });
Check("verified file write preserves existing", () => { byte[] b = [1, 2, 3, 4]; string p = Path.Combine(root, "write.bin"); Builder.WriteNewVerified(p, b, Firmware.Hash(b)); Reject(() => Builder.WriteNewVerified(p, [5, 6], Firmware.Hash([5, 6]))); Assert(File.ReadAllBytes(p).SequenceEqual(b)); });
Check("wrong write hash rejected", () => Reject(() => Builder.WriteNewVerified(Path.Combine(root, "bad.bin"), [1, 2], Recipe.PatchHash)));
if (args.Length < 2) throw new Exception("Pass original 3202 CAP and donor 1904 CAP paths to run required firmware integration tests.");
var original = Firmware.ReadImage(args[0]); var donor = Firmware.ReadImage(args[1]);
BuildResult? built = null;
Check("exact official fixture hashes", () => Assert(Firmware.Hash(original) == Recipe.BaseHash && Firmware.Hash(donor) == Recipe.DonorHash));
Check("official MCU inventory", () => { var i = Firmware.Inspect(original, "3202"); Assert(i.Microcodes.Count == 4); Assert(i.Microcodes[2].Revision == 0x133); Assert(i.Microcodes[3].KnownOemHeader); });
Check("build byte-identical working reference", () => { built = Builder.Build(original, donor); Assert(built.Sha256 == Recipe.ResultHash); Assert(built.Checks.Count == 7); });
Check("source buffers remain unchanged", () => Assert(Firmware.Hash(original) == Recipe.BaseHash && Firmware.Hash(donor) == Recipe.DonorHash));
Check("only permitted slot changes", () => { int changed = 0; for (int i = 0; i < original.Length; i++) if (original[i] != built!.Image[i]) { Assert(i >= Recipe.Start && i < Recipe.Start + Recipe.OldSize); changed++; } Assert(changed > 0); });
Check("raw MCU donor supported", () => Assert(Builder.Build(original, donor.AsSpan(Recipe.DonorStart, Recipe.NewSize).ToArray()).Sha256 == Recipe.ResultHash));
Check("wrong motherboard/source rejected", () => Reject(() => Builder.Build(donor, donor)));
Check("wrong donor rejected", () => Reject(() => Builder.Build(original, original)));
Check("source tampering rejected", () => { var b = original.ToArray(); b[100] ^= 1; Reject(() => Builder.Build(b, donor)); });
Check("patch tampering rejected", () => { var b = built!.Image.ToArray(); b[Recipe.Start + 48] ^= 1; Reject(() => Builder.Validate(original, b)); });
Check("outside slot edit rejected", () => { var b = built!.Image.ToArray(); b[100] ^= 1; Reject(() => Builder.Validate(original, b)); });
Check("FIT relocation rejected", () => { var b = built!.Image.ToArray(); b[0x1091140] ^= 1; Reject(() => Builder.Validate(original, b)); });
Check("padding change rejected", () => { var b = built!.Image.ToArray(); b[Recipe.Start + Recipe.NewSize] = 0; Reject(() => Builder.Validate(original, b)); });
Check("truncated build rejected", () => Reject(() => Builder.Validate(original, built!.Image[..^1])));
Check("unknown file gets no ASUS header exemption", () => { var b = original.ToArray(); b[100] ^= 1; Assert(!Firmware.Inspect(b, "unknown").Microcodes[3].KnownOemHeader); });
Check("export contains recovery and report", () => { var p = Builder.Export(built!, root); Assert(Firmware.Hash(Firmware.ReadImage(p.CandidatePath)) == Recipe.ResultHash); Assert(Firmware.Hash(Firmware.ReadImage(p.RecoveryPath)) == Recipe.BaseHash); Assert(File.Exists(p.ReportPath) && File.Exists(Path.Combine(p.Directory, "READ-ME.txt"))); });
Check("export boundary catches mutation", () => { var b = built!.Image.ToArray(); b[0] ^= 1; Reject(() => Builder.Export(built with { Image = b }, root)); });
if (args.Contains("--online"))
{
    string cache = Path.Combine(root, "downloads");
    foreach (var spec in new[] { Recipe.BaseDownload, Recipe.DonorDownload })
    {
        string path = await OfficialDownloads.FetchAsync(spec, cache, null, CancellationToken.None);
        Check($"official {spec.Version} download and extraction", () => Assert(Firmware.Hash(Firmware.ReadImage(path)) == spec.CapHash));
        string cached = await OfficialDownloads.FetchAsync(spec, cache, null, CancellationToken.None);
        Check($"verified {spec.Version} cache", () => Assert(cached == path));
    }
    File.WriteAllBytes(Path.Combine(cache, Recipe.DonorDownload.CapName), [1, 2, 3]);
    string repaired = await OfficialDownloads.FetchAsync(Recipe.DonorDownload, cache, null, CancellationToken.None);
    Check("truncated cache repaired from verified download", () => Assert(Firmware.Hash(Firmware.ReadImage(repaired)) == Recipe.DonorHash));
    string rejectedCache = Path.Combine(root, "rejected-download");
    try { await OfficialDownloads.FetchAsync(Recipe.DonorDownload with { ZipHash = Recipe.BaseHash }, rejectedCache, null, CancellationToken.None); throw new Exception("Wrong ZIP hash accepted"); }
    catch (InvalidDataException) { Check("wrong download hash rejected without accepting CAP", () => Assert(!Directory.EnumerateFiles(rejectedCache).Any())); }
    using var cancel = new CancellationTokenSource(); cancel.Cancel();
    try { await OfficialDownloads.FetchAsync(Recipe.BaseDownload, cache, null, cancel.Token); throw new Exception("Cancellation ignored"); }
    catch (OperationCanceledException) { passed++; Console.WriteLine("PASS cancellation"); }
}
Console.WriteLine($"{passed} tests passed. Evidence and isolated output: {root}");
