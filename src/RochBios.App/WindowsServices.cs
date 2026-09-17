using System.Diagnostics;
using System.IO;
using System.Text;
using System.Text.Json;
using Microsoft.Win32;
using RochBios.Core;

namespace RochBios.App;

internal static class WindowsServices
{
    static async Task<string> PowerShell(string script)
    {
        var start = new ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), @"WindowsPowerShell\v1.0\powershell.exe"))
        { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true, StandardOutputEncoding = Encoding.UTF8 };
        start.ArgumentList.Add("-NoProfile"); start.ArgumentList.Add("-NonInteractive");
        start.ArgumentList.Add("-EncodedCommand");
        start.ArgumentList.Add(Convert.ToBase64String(Encoding.Unicode.GetBytes("$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; " + script)));
        using var process = Process.Start(start) ?? throw new IOException("Could not start Windows inventory.");
        var output = process.StandardOutput.ReadToEndAsync(); var error = process.StandardError.ReadToEndAsync();
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(30));
        try { await process.WaitForExitAsync(timeout.Token); }
        catch (OperationCanceledException) { process.Kill(true); throw new IOException("Windows drive inventory timed out. Try Refresh."); }
        var result = await output;
        if (process.ExitCode != 0) throw new IOException("Windows inventory failed: " + await error);
        return result.Trim('\uFEFF', ' ', '\r', '\n');
    }
    public static async Task<List<UsbDrive>> DrivesAsync()
    {
        string json = await PowerShell("""
            $rows = @(foreach ($d in (Get-Disk | Where-Object BusType -eq USB)) {
              $parts = @(Get-Partition -DiskNumber $d.Number)
              foreach ($p in $parts) {
                if (!$p.DriveLetter) { continue }
                $v = $p | Get-Volume
                [pscustomobject]@{
                  DiskNumber=[int]$d.Number; UniqueId=[string]$d.UniqueId; Root=([string]$p.DriveLetter + ':\');
                  Label=[string]$v.FileSystemLabel; Bus=[string]$d.BusType; PartitionStyle=[string]$d.PartitionStyle;
                  FileSystem=[string]$v.FileSystem; PartitionCount=$parts.Count;
                  IsBoot=[bool]($d.IsBoot -or $p.IsBoot); IsSystem=[bool]($d.IsSystem -or $p.IsSystem);
                  Size=[long]$d.Size; Free=[long]$v.SizeRemaining; Offset=[long]$p.Offset; VolumeId=[string]$v.UniqueId
                }
              }
            })
            ConvertTo-Json -InputObject $rows -Compress
            """);
        return JsonSerializer.Deserialize<List<UsbDrive>>(json) ?? [];
    }
    public static async Task<string> SystemAsync()
    {
        var data = await PowerShell("$b=Get-CimInstance Win32_BaseBoard; $f=Get-CimInstance Win32_BIOS; $c=Get-CimInstance Win32_Processor | Select-Object -First 1; [pscustomobject]@{Board=$b.Product;Bios=$f.SMBIOSBIOSVersion;Cpu=$c.Name} | ConvertTo-Json -Compress");
        using var doc = JsonDocument.Parse(data);
        var e = doc.RootElement;
        string mc = "unavailable";
        using var key = Registry.LocalMachine.OpenSubKey(@"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        if (key?.GetValue("Update Revision") is byte[] bytes && bytes.Length >= 8)
            mc = $"0x{BitConverter.ToUInt32(bytes, 4):X}";
        string cpuid = System.Runtime.Intrinsics.X86.X86Base.IsSupported ? $" • CPUID {System.Runtime.Intrinsics.X86.X86Base.CpuId(1, 0).Eax:X}" : "";
        string board = e.GetProperty("Board").GetString() ?? "Unknown board";
        return $"{board} • BIOS {e.GetProperty("Bios").GetString()}\n{e.GetProperty("Cpu").GetString()}{cpuid}\nWindows-reported microcode: {mc}\nThe build target is determined by the BIOS you supply, not by this computer.";
    }
    public static async Task<string> PrepareAsync(UsbDrive selected, TransferPackage package, bool recovery, IProgress<string>? progress, CancellationToken ct)
    {
        // Rebuild from package bytes and rerun both verifiers; saved success text is not trusted.
        var result = await Task.Run(package.Revalidate, ct);
        string work = Path.Combine(AppPaths.DataDirectory, "verification-jobs", Guid.NewGuid().ToString("N"));
        await IndependentVerification.VerifyAsync(result, work, progress, ct);
        byte[] image = recovery ? result.Original : result.Image;
        string hash = Firmware.Hash(image), flashName = package.Manifest.Profile.FileName;
        async Task CheckIdentity(bool requireSpace = true)
        {
            var fresh = (await DrivesAsync()).SingleOrDefault(d => d.Identity == selected.Identity)
                ?? throw new IOException("USB was removed or changed. Refresh drives.");
            selected.RequireSameEligible(fresh, false);
            if (requireSpace && fresh.Free < image.Length + 1048576L) throw new IOException($"USB needs at least {(image.Length + 1048576L) / 1048576.0:0.#} MB free for this image.");
        }
        ct.ThrowIfCancellationRequested();
        progress?.Report("Checking USB identity, backing up the existing file, then copying and verifying…");
        await CheckIdentity();
        string target = Path.Combine(selected.Root, flashName);
        string staged = Path.Combine(selected.Root, $"ROCH-{Guid.NewGuid():N}.tmp");
        string? backup = null;
        string? priorHash = null;
        try
        {
            if (File.Exists(target))
            {
                if ((File.GetAttributes(target) & FileAttributes.ReparsePoint) != 0) throw new IOException("The existing CAP is a link. Remove it manually before preparing this drive.");
                var previous = Firmware.ReadImage(target);
                priorHash = Firmware.Hash(previous);
                string backups = Path.Combine(AppPaths.DataDirectory, "USB backups");
                Directory.CreateDirectory(backups);
                backup = Path.Combine(backups, $"{DateTime.Now:yyyyMMdd-HHmmss}-{Guid.NewGuid():N}-{flashName}");
                Builder.WriteNewVerified(backup, previous, priorHash);
            }
            Builder.WriteNewVerified(staged, image, hash);
            await CheckIdentity(false);
            if (File.Exists(target) && (File.GetAttributes(target) & FileAttributes.ReparsePoint) != 0) throw new IOException("Destination changed into a link. Copy stopped.");
            string? currentHash = File.Exists(target) ? Firmware.Hash(Firmware.ReadImage(target)) : null;
            if (currentHash != priorHash) throw new IOException("The existing USB CAP changed while preparing the drive. No replacement was committed. Try again.");
            File.Move(staged, target, true);
            if (Firmware.Hash(Firmware.ReadImage(target)) != hash) throw new IOException("USB read-back verification failed. Do not flash this file.");
            return $"Verified {target}\n{(recovery ? "Untouched new BIOS / recovery" : $"Modified BIOS / microcode 0x{result.NewRevision:X}")} is ready. Eject the drive in Windows before removing it." + (backup is null ? "" : $"\nPrevious BIOS file saved to {backup}");
        }
        finally
        {
            // Do not clean up through a drive letter that may now refer to another disk.
            try { await CheckIdentity(false); if (File.Exists(staged)) File.Delete(staged); } catch { }
        }
    }
}
