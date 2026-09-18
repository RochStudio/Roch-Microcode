using System.Diagnostics;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Microsoft.Win32;
using RochBios.Core;

namespace RochBios.App;
public partial class MainWindow : Window
{
    FirmwareInput? older, newer;
    TransferPackage? package;
    bool busy, updating;
    CancellationTokenSource? cancellation;
    public MainWindow()
    {
        InitializeComponent(); VendorBox.ItemsSource = FlashProfile.Vendors;
        UpdateThemeButton();
        SourceInitialized += (_, _) => WindowTheme.Apply(this, ((App)Application.Current).IsLightTheme);
        // Fit the initial window to the available desktop in WPF device-independent units.
        Width = Math.Min(Width, SystemParameters.WorkArea.Width);
        Height = Math.Min(Height, SystemParameters.WorkArea.Height);
        Loaded += async (_, _) =>
        {
            try { SystemText.Text = await WindowsServices.SystemAsync(); } catch { SystemText.Text = "System information unavailable. Use the target board's exact model and revision."; }
            var args = Environment.GetCommandLineArgs();
            if (args.Length == 5 && args[1] == "--smoke-test") await SmokeTest(args[2], args[3], args[4]);
        };
        Closing += (_, e) => { if (busy) { e.Cancel = true; StatusText.Text = "Wait for the operation to finish or cancel verification before closing."; } };
    }
    void UpdateThemeButton() => ThemeButton.Content = ((App)Application.Current).IsLightTheme ? "Dark theme" : "Light theme";
    void Theme_Click(object s, RoutedEventArgs e)
    {
        var app = (App)Application.Current;
        bool saved = app.SetTheme(!app.IsLightTheme);
        UpdateThemeButton();
        if (!saved) StatusText.Text = "Theme changed for this session; the preference could not be saved.";
    }
    void RefreshControls()
    {
        if (BuildButton is null) return;
        foreach (var b in new[] { OldButton, NewButton, InspectButton, RefreshUsbButton, LoadPackageButton }) b.IsEnabled = !busy;
        OldDrop.AllowDrop = NewDrop.AllowDrop = !busy;
        OldCodes.IsEnabled = NewCodes.IsEnabled = VendorBox.IsEnabled = MethodBox.IsEnabled = BoardBox.IsEnabled = FlashNameBox.IsEnabled = !busy;
        BuildButton.IsEnabled = !busy && OldCodes.SelectedItem is Microcode d && NewCodes.SelectedItem is Microcode t && Transfer.CanReplace(d, t) && d.Revision != t.Revision && d.Size <= t.Size;
        OpenPackageButton.IsEnabled = GoUsbButton.IsEnabled = !busy && package is not null;
        Drives.IsEnabled = ModifiedChoice.IsEnabled = RecoveryChoice.IsEnabled = !busy;
        PrepareButton.IsEnabled = !busy && package is not null && Drives.SelectedItem is UsbDrive { Rejection: null };
        Progress.Visibility = busy ? Visibility.Visible : Visibility.Collapsed;
    }
    async Task Run(Func<CancellationToken, Task> action, bool cancellable = true)
    {
        if (busy) return;
        busy = true; cancellation = new(); RefreshControls(); CancelButton.Visibility = cancellable ? Visibility.Visible : Visibility.Collapsed;
        try { await action(cancellation.Token); }
        catch (OperationCanceledException) { StatusText.Text = "Cancelled. An incomplete build is not accepted for USB preparation."; if (Tabs.SelectedIndex == 1 && package is null) VerificationTitle.Text = "Verification cancelled"; }
        catch (Exception ex) { StatusText.Text = "Stopped • " + ex.Message; if (Tabs.SelectedIndex == 1 && package is null) { VerificationTitle.Text = "Verification did not complete"; VerificationDetail.Text = ex.Message; } MessageBox.Show(this, ex.Message, "Roch Microcode", MessageBoxButton.OK, MessageBoxImage.Warning); }
        finally { busy = false; cancellation.Dispose(); cancellation = null; RefreshControls(); CancelButton.Visibility = Visibility.Collapsed; }
    }
    void Invalidate()
    {
        package = null;
        if (Checks is null) return;
        Checks.ItemsSource = null; OutputHash.Text = ""; VerificationTitle.Text = "Verification results will appear here";
        VerificationDetail.Text = "The current selection needs a new build and verification.";
        PackageText.Text = "Build and verify the current selection, or open an existing version 2 package.";
        BoardGuideText.Text = "Confirm the selected flash method and filename in the board manual. Keep a separate recovery drive.";
        UsbResult.Text = ""; RefreshControls();
    }
    string? PickImage(string title)
    {
        var dialog = new OpenFileDialog { Title = title, Filter = "BIOS files and ZIP packages|*.*" };
        return dialog.ShowDialog(this) == true ? dialog.FileName : null;
    }
    async Task Load(string file, bool old) => await Run(async _ =>
    {
        StatusText.Text = "Reading BIOS and finding Intel microcodes…";
        var input = await Task.Run(() => FirmwareInput.Load(file)); SetInput(input, old);
        StatusText.Text = input.Info.Microcodes.Count == 0 ? "No uncompressed Intel microcodes found. AMD or compressed images cannot be edited by this version." : "Image loaded. Choose the microcode to transfer.";
    }, false);
    void SetInput(FirmwareInput input, bool old)
    {
        updating = true;
        if (old)
        {
            older = input; OldName.Text = input.FileName; OldName.ToolTip = input.FileName;
            OldDetail.Text = $"{input.Bytes.Length / 1048576.0:0.##} MB • {input.Info.Microcodes.Count} microcodes found";
            var valid = input.Info.Microcodes.Where(m => m.ChecksumValid).ToArray(); OldCodes.ItemsSource = valid;
            uint cpu = System.Runtime.Intrinsics.X86.X86Base.IsSupported ? (uint)System.Runtime.Intrinsics.X86.X86Base.CpuId(1, 0).Eax : 0;
            OldCodes.SelectedItem = valid.FirstOrDefault(m => m.Cpuid == cpu && m.Revision == 0x11f) ?? valid.Where(m => m.Cpuid == cpu).OrderByDescending(m => m.Revision).FirstOrDefault() ?? valid.FirstOrDefault();
        }
        else
        {
            newer = input; NewName.Text = input.FileName; NewName.ToolTip = input.FileName;
            NewDetail.Text = $"{input.Bytes.Length / 1048576.0:0.##} MB • {input.Info.Microcodes.Count} microcodes found";
            VendorBox.SelectedItem = input.SuggestedVendor;
            BoardBox.Text = input.Info.IsSupportedBase ? Recipe.Board : Path.GetFileNameWithoutExtension(input.FileName).Replace('_', ' ');
            SetMethods(input.SuggestedVendor);
        }
        updating = false; Invalidate(); UpdateTargets(); ShowInspection(input.Info);
    }
    void UpdateTargets()
    {
        updating = true;
        var current = NewCodes.SelectedItem as Microcode;
        var choices = OldCodes.SelectedItem is Microcode donor && newer is not null
            ? newer.Info.Microcodes.Where(t => Transfer.CanReplace(donor, t)).OrderByDescending(t => t.Revision).ToArray() : [];
        NewCodes.ItemsSource = choices; NewCodes.SelectedItem = choices.FirstOrDefault(t => t.Offset == current?.Offset) ?? choices.FirstOrDefault();
        updating = false; UpdatePlan();
    }
    void UpdatePlan()
    {
        if (OldCodes.SelectedItem is Microcode d && NewCodes.SelectedItem is Microcode t)
            PlanText.Text = d.Revision == t.Revision ? "These revisions already match. Choose a different microcode."
                : d.Size > t.Size ? $"The old microcode is {d.Size - t.Size:N0} bytes too large for this slot. Relocation is not supported."
                : $"CPUID {d.Cpuid:X}: replace {t.RevisionText} with {d.RevisionText} in the new BIOS.";
        else PlanText.Text = older is not null && newer is not null ? "No compatible target. The donor needs the same primary CPU signature and a platform mask equal to or contained in the new patch's mask." : "Choose the old and new BIOS to start.";
        CoverageText.Text = OldCodes.SelectedItem is Microcode source && NewCodes.SelectedItem is Microcode dest && source.Platforms != dest.Platforms
            ? Transfer.CompatibilityDetail(source, dest) : "";
        CoverageText.Visibility = CoverageText.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        RefreshControls();
    }
    async void Old_Click(object s, RoutedEventArgs e) { var p = PickImage("Choose OLD donor BIOS or ZIP"); if (p is not null) await Load(p, true); }
    async void New_Click(object s, RoutedEventArgs e) { var p = PickImage("Choose NEW BIOS to modify or ZIP"); if (p is not null) await Load(p, false); }
    void File_DragOver(object s, DragEventArgs e) { e.Effects = !busy && e.Data.GetData(DataFormats.FileDrop) is string[] { Length: 1 } ? DragDropEffects.Copy : DragDropEffects.None; e.Handled = true; }
    async void Old_Drop(object s, DragEventArgs e) { if (!busy && e.Data.GetData(DataFormats.FileDrop) is string[] { Length: 1 } paths) { e.Handled = true; await Load(paths[0], true); } }
    async void New_Drop(object s, DragEventArgs e) { if (!busy && e.Data.GetData(DataFormats.FileDrop) is string[] { Length: 1 } paths) { e.Handled = true; await Load(paths[0], false); } }
    void OldCode_Changed(object s, SelectionChangedEventArgs e) { if (!updating && IsLoaded) { Invalidate(); UpdateTargets(); } }
    void NewCode_Changed(object s, SelectionChangedEventArgs e) { if (!updating && IsLoaded) { Invalidate(); UpdatePlan(); } }
    void Profile_Changed(object s, TextChangedEventArgs e) { if (!updating && IsLoaded) Invalidate(); }
    void Vendor_Changed(object s, SelectionChangedEventArgs e)
    {
        if (updating || !IsLoaded) return;
        updating = true; string vendor = VendorBox.SelectedItem as string ?? "";
        SetMethods(vendor);
        updating = false; Invalidate();
    }
    void SetMethods(string vendor)
    {
        MethodBox.ItemsSource = FlashProfile.Methods(vendor);
        MethodBox.SelectedIndex = 0;
        SetFlashName();
    }
    void SetFlashName()
    {
        string vendor = VendorBox.SelectedItem as string ?? "";
        string method = MethodBox.SelectedItem as string ?? FlashProfile.FlashBack;
        FlashNameBox.Text = FlashProfile.SuggestedName(vendor, method, newer?.FileName, newer?.SuggestedAsusName);
        FlashNameBox.IsReadOnly = method == FlashProfile.MFlash;
        ProfileHint.Text = method == FlashProfile.MFlash
            ? "M-FLASH keeps the NEW BIOS filename. MSI.ROM is only for the Flash BIOS Button. Modified firmware may be rejected."
            : "Check button support and filename against the board manual. ASUS filenames are model-specific.";
    }
    void Method_Changed(object s, SelectionChangedEventArgs e)
    {
        if (updating || !IsLoaded) return;
        updating = true; SetFlashName(); updating = false; Invalidate();
    }
    FlashProfile Profile() => new(VendorBox.SelectedItem as string ?? "", BoardBox.Text.Trim(), FlashNameBox.Text.Trim())
    { Method = MethodBox.SelectedItem as string ?? FlashProfile.FlashBack, OriginalFileName = newer?.FileName };
    async void Build_Click(object s, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "Save the verified BIOS package in…" }; if (dialog.ShowDialog(this) != true) return;
        var d = (Microcode)OldCodes.SelectedItem; var t = (Microcode)NewCodes.SelectedItem; var profile = Profile();
        await Run(async ct =>
        {
            profile.Validate(); package = null; Checks.ItemsSource = null;
            VerificationTitle.Text = "Building and verifying…"; Tabs.SelectedIndex = 1;
            var result = await Task.Run(() => Transfer.Build(newer!.Bytes, older!.Bytes, d.Offset, t.Offset), ct);
            package = await TransferPackage.ExportAsync(result, profile, dialog.FolderName, new Progress<string>(text => { StatusText.Text = text; VerificationDetail.Text = text; }), ct);
            ShowPackage(package); ShowInspection(Firmware.Inspect(result.Image, profile.FileName));
            StatusText.Text = "All 8 verification checks passed. Modified and recovery images are ready.";
        });
    }
    void ShowPackage(TransferPackage p)
    {
        Checks.ItemsSource = p.Manifest.Checks.Select(c => new { c.Name,
            Detail = c.Name == "CPU compatibility" ? c.Detail.Split(". ")[0] + "." : c.Detail,
            FullDetail = c.Detail }).ToArray();
        VerificationTitle.Text = $"Verified • 0x{p.Manifest.FromRevision:X} → 0x{p.Manifest.ToRevision:X}";
        VerificationDetail.Text = $"{p.Manifest.Profile.Vendor} • {p.Manifest.Profile.Board}\nOriginal firmware preserved; donor microcode transferred and independently checked.";
        OutputHash.Text = $"SHA-256  {p.Manifest.CandidateHash}\n{p.Directory}";
        var scope = p.Manifest.Checks.FirstOrDefault(c => c.Name == "CPU compatibility")?.Detail ?? "";
        PackageText.Text = $"{p.Manifest.Profile.Vendor} • {p.Manifest.Profile.Board}\n{p.Manifest.Profile.Method}\nUSB filename: {p.Manifest.Profile.FileName}\n\n{scope}";
        BoardGuideText.Text = p.Manifest.Profile.BoardInstructions;
        PackageText.ToolTip = p.Directory + "\n\n" + scope;
    }
    async void Inspect_Click(object s, RoutedEventArgs e)
    {
        var file = PickImage("Inspect firmware or ZIP"); if (file is null) return;
        await Run(async _ => { var input = await Task.Run(() => FirmwareInput.Load(file)); ShowInspection(input.Info); StatusText.Text = "Inspection complete."; }, false);
    }
    void ShowInspection(FirmwareInfo info) { ImageIdentity.Text = info.FileName; ImageHash.Text = $"{info.Size:N0} bytes\nSHA-256  {info.Sha256}"; MicrocodeGrid.ItemsSource = info.Microcodes; }
    async Task RefreshDrives() => await Run(async _ => { var drives = await WindowsServices.DrivesAsync(); Drives.ItemsSource = drives; Drives.SelectedIndex = -1; DriveStatus.Text = drives.Count == 0 ? "No mounted USB drives found. Connect one and refresh." : "Select the USB destination."; StatusText.Text = $"Found {drives.Count} USB volume(s)."; }, false);
    async void RefreshUsb_Click(object s, RoutedEventArgs e) => await RefreshDrives();
    async void GoUsb_Click(object s, RoutedEventArgs e) { Tabs.SelectedIndex = 2; await RefreshDrives(); }
    void Drive_Changed(object s, SelectionChangedEventArgs e) { if (PrepareButton is null) return; DriveStatus.Text = Drives.SelectedItem is UsbDrive d ? d.Rejection ?? "Compatible layout. The destination will be confirmed before copying." : "Select a USB drive."; UsbResult.Text = ""; RefreshControls(); }
    async void LoadPackage_Click(object s, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "Open a Roch Microcode version 2 package" }; if (dialog.ShowDialog(this) != true) return;
        await Run(async ct =>
        {
            package = null;
            var opened = await Task.Run(() => TransferPackage.Open(dialog.FolderName), ct);
            var result = await Task.Run(opened.Revalidate, ct);
            string work = Path.Combine(AppPaths.DataDirectory, "verification-jobs", Guid.NewGuid().ToString("N"));
            var evidence = await IndependentVerification.VerifyAsync(result, work, new Progress<string>(text => StatusText.Text = text), ct);
            package = opened with { Manifest = opened.Manifest with { Checks = result.Checks.Concat(evidence.Checks).ToArray() } };
            ShowPackage(package); StatusText.Text = "Package independently reverified from its actual firmware files.";
        });
    }
    async void Prepare_Click(object s, RoutedEventArgs e)
    {
        if (package is null || Drives.SelectedItem is not UsbDrive drive) return;
        var profile = package.Manifest.Profile; bool recovery = RecoveryChoice.IsChecked == true;
        string message = $"Prepare {drive.Display}\n\n{profile.Vendor} • {profile.Board}\nMethod: {profile.Method}\nWrite {(recovery ? "UNTOUCHED NEW BIOS / RECOVERY" : "MODIFIED BIOS")} as {drive.Root}{profile.FileName}?\n\n{profile.BoardInstructions}\n\nAn existing file at this path will be backed up locally and replaced. Other files remain. The app will not flash the motherboard.";
        if (MessageBox.Show(this, message, "Confirm USB destination and board", MessageBoxButton.YesNo, MessageBoxImage.Question, MessageBoxResult.No) != MessageBoxResult.Yes) return;
        await Run(async ct => { UsbResult.Text = await WindowsServices.PrepareAsync(drive, package, recovery, new Progress<string>(text => StatusText.Text = text), ct); StatusText.Text = "USB prepared and read-back hash verified."; }, false);
    }
    void Cancel_Click(object s, RoutedEventArgs e) => cancellation?.Cancel();
    static void Open(string target) => Process.Start(new ProcessStartInfo(target) { UseShellExecute = true });
    void Social_RequestNavigate(object sender, System.Windows.Navigation.RequestNavigateEventArgs e)
    {
        e.Handled = true;
        try { Open(e.Uri.AbsoluteUri); }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or InvalidOperationException)
        { StatusText.Text = "Could not open the link in your default browser: " + ex.Message; }
    }
    void OpenPackage_Click(object s, RoutedEventArgs e) { if (package is not null) Open(package.Directory); }
    void Guide_Click(object s, RoutedEventArgs e) => Open(package?.Manifest.Profile.GuideUrl ?? Profile().GuideUrl);
    async Task SmokeTest(string basePath, string donorPath, string output)
    {
        try
        {
            Directory.CreateDirectory(output); SetInput(FirmwareInput.Load(donorPath), true); SetInput(FirmwareInput.Load(basePath), false);
            if (newer!.SuggestedVendor == "MSI")
            {
                if (Profile().Method != FlashProfile.MFlash || FlashNameBox.Text != newer.FileName) throw new Exception("M-FLASH default filename failed");
                MethodBox.SelectedItem = FlashProfile.FlashBack;
                if (FlashNameBox.Text != "MSI.ROM") throw new Exception("Button filename failed");
                MethodBox.SelectedItem = FlashProfile.MFlash;
                if (FlashNameBox.Text != newer.FileName || !FlashNameBox.IsReadOnly) throw new Exception("M-FLASH filename restoration failed");
            }
            var donor = (Microcode)OldCodes.SelectedItem; var target = (Microcode)NewCodes.SelectedItem;
            var result = await Task.Run(() => Transfer.Build(newer!.Bytes, older!.Bytes, donor.Offset, target.Offset));
            package = await TransferPackage.ExportAsync(result, Profile(), output, null, CancellationToken.None);
            var reopened = TransferPackage.Open(package.Directory);
            if (reopened.Manifest.Profile.Method != Profile().Method || Path.GetFileName(reopened.CandidatePath) != Profile().FileName || Path.GetFileName(reopened.RecoveryPath) != Profile().FileName)
                throw new Exception("Flash method or filenames did not survive package export/reload");
            ShowPackage(package); ShowInspection(Firmware.Inspect(result.Image, Profile().FileName));
            Drives.ItemsSource = await WindowsServices.DrivesAsync(); RefreshControls();
            StatusText.Text = "UI test • all verification passed. No USB or firmware writes.";
            var app = (App)Application.Current;
            foreach (var size in new[] { (Name: "normal", Width: 1180d, Height: 800d), (Name: "compact", Width: 1000d, Height: 720d) })
            foreach (bool light in new[] { false, true })
            {
                Width = size.Width; Height = size.Height;
                app.SetTheme(light, false); UpdateThemeButton();
                for (int i = 0; i < 4; i++)
                {
                    Tabs.SelectedIndex = i; await Dispatcher.InvokeAsync(() => { }, System.Windows.Threading.DispatcherPriority.ApplicationIdle); UpdateLayout();
                    FrameworkElement page = i switch { 0 => TransferPage, 1 => VerificationPage, 2 => UsbPage, _ => InspectPage };
                    FrameworkElement[] probes = i switch {
                        0 => [OldCodes, NewCodes, MethodBox, BoardBox, FlashNameBox, ProfileHint, PlanText, CoverageText, BuildButton],
                        1 => [VerificationTitle, VerificationDetail, Checks, OutputHash, OpenPackageButton, GoUsbButton],
                        2 => [Drives, DriveStatus, ModifiedChoice, RecoveryChoice, PackageText, PrepareButton],
                        _ => [SystemText, ImageIdentity, ImageHash, MicrocodeGrid, InspectButton] };
                    foreach (var probe in probes.Where(p => p.IsVisible))
                    {
                        var rect = probe.TransformToAncestor(page).TransformBounds(new Rect(probe.RenderSize));
                        if (rect.Left < -1 || rect.Top < -1 || rect.Right > page.ActualWidth + 1 || rect.Bottom > page.ActualHeight + 1)
                            throw new InvalidOperationException($"Layout overflow: {size.Name}, tab {i}, {probe.Name}");
                    }
                    var surface = (FrameworkElement)Content; var bitmap = new RenderTargetBitmap((int)surface.ActualWidth + 40, (int)surface.ActualHeight + 28, 96, 96, PixelFormats.Pbgra32);
                    var drawing = new DrawingVisual(); using (var dc = drawing.RenderOpen()) { dc.DrawRectangle(Background, null, new Rect(0, 0, bitmap.PixelWidth, bitmap.PixelHeight)); dc.DrawRectangle(new VisualBrush(surface), null, new Rect(20, 14, surface.ActualWidth, surface.ActualHeight)); }
                    bitmap.Render(drawing); var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(bitmap)); using var stream = File.Create(Path.Combine(output, $"{size.Name}-{(light ? "light" : "dark")}-tab-{i}.png")); png.Save(stream);
                }
            }
            File.WriteAllText(Path.Combine(output, "ui-smoke.txt"), "PASS: BIOS transfer, independent verification, package export; four tabs in both themes at 1180x800 and 1000x720; key controls fit without page scrolling. Theme preference was not changed. No USB writes."); Close();
        }
        catch (Exception ex) { File.WriteAllText(Path.Combine(output, "ui-smoke.txt"), ex.ToString()); Environment.ExitCode = 1; Close(); }
    }
}
