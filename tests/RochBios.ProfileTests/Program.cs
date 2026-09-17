using System.Text.Json;
using RochBios.Core;

int passed = 0;
void Check(string name, Action test) { test(); Console.WriteLine("PASS " + name); passed++; }
void Assert(bool value) { if (!value) throw new Exception("Assertion failed"); }
void Reject(Action test) { try { test(); } catch (InvalidDataException) { return; } throw new Exception("Invalid profile accepted"); }
var profile = new FlashProfile("MSI", "Z790 MPOWER", "E7E01IMS.PA0")
{ Method = FlashProfile.MFlash, OriginalFileName = "E7E01IMS.PA0" };
Check("M-FLASH preserves PA extension", profile.Validate);
Check("MSI defaults to M-FLASH", () => Assert(FlashProfile.Methods("MSI")[0] == FlashProfile.MFlash));
Check("target filename suggested", () => Assert(FlashProfile.SuggestedName("MSI", FlashProfile.MFlash, "E7E01IMS.PA0") == "E7E01IMS.PA0"));
Check("button retains MSI.ROM", () => Assert(FlashProfile.SuggestedName("MSI", FlashProfile.FlashBack, "E7E01IMS.PA0") == "MSI.ROM"));
Check("M-FLASH rejects button name", () => Reject(() => (profile with { FileName = "MSI.ROM", OriginalFileName = "MSI.ROM" }).Validate()));
Check("M-FLASH rejects donor filename", () => Reject(() => (profile with { FileName = "E7E07IMS.190" }).Validate()));
Check("M-FLASH rejects wrong version", () => Reject(() => (profile with { FileName = "E7E01IMS.P90" }).Validate()));
Check("M-FLASH requires original name", () => Reject(() => (profile with { OriginalFileName = null }).Validate()));
Check("M-FLASH rejects traversal", () => Reject(() => (profile with { FileName = "../E7E01IMS.PA0" }).Validate()));
Check("button rejects M-FLASH name", () => Reject(() => (profile with { Method = FlashProfile.FlashBack }).Validate()));
Check("other vendors reject M-FLASH", () => Reject(() => (profile with { Vendor = "ASUS" }).Validate()));
Check("unknown method rejected", () => Reject(() => (profile with { Method = "unknown" }).Validate()));
Check("null method rejected", () => Reject(() => (profile with { Method = null! }).Validate()));
Check("M-FLASH guide and instructions", () => Assert(profile.GuideUrl.EndsWith("mb_bios_update") && profile.Instructions.Contains("Enter BIOS, open M-FLASH") && !profile.Instructions.Contains("Usually the PC is off")));
Check("JSON preserves method and original filename", () => { var restored = JsonSerializer.Deserialize<FlashProfile>(JsonSerializer.Serialize(profile))!; restored.Validate(); Assert(restored == profile); });
Check("legacy JSON keeps button semantics", () => {
    var restored = JsonSerializer.Deserialize<FlashProfile>("""{"Vendor":"MSI","Board":"Z790 MPOWER","FileName":"MSI.ROM"}""")!;
    restored.Validate(); Assert(restored.Method == FlashProfile.FlashBack && restored.GuideUrl.EndsWith("MB_Flash_BIOS_Button"));
});
foreach (var (vendor, name) in new[] { ("ASUS", "SZ790AD4.CAP"), ("ASRock", "CREATIVE.ROM"), ("Gigabyte", "GIGABYTE.bin") })
    Check(vendor + " naming unchanged", () => new FlashProfile(vendor, "Test board", name).Validate());
if (args.Length > 0) Check("existing MSI package still revalidates", () => {
    var package = TransferPackage.Open(args[0]); Assert(package.Manifest.Profile.Method == FlashProfile.FlashBack && Path.GetFileName(package.CandidatePath) == "MSI.ROM");
});
Console.WriteLine($"{passed} profile checks passed.");
