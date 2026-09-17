using System.IO;
using System.Windows;
using RochBios.Core;
namespace RochBios.App;
public partial class App : Application
{
    public bool IsLightTheme { get; private set; }
    static string ThemePath => Path.Combine(AppPaths.DataDirectory, "theme.txt");
    protected override void OnStartup(StartupEventArgs e)
    {
        bool light = false;
        try { light = File.Exists(ThemePath) && File.ReadAllText(ThemePath).Trim() == "Light"; }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { }
        SetTheme(light, false);
        base.OnStartup(e);
    }
    public bool SetTheme(bool light, bool persist = true)
    {
        Resources.MergedDictionaries[0] = new ResourceDictionary { Source = new Uri($"Themes/{(light ? "Light" : "Dark")}.xaml", UriKind.Relative) };
        IsLightTheme = light;
        if (!persist) return true;
        try { Directory.CreateDirectory(Path.GetDirectoryName(ThemePath)!); File.WriteAllText(ThemePath, light ? "Light" : "Dark"); return true; }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { return false; }
    }
}
