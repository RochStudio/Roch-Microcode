namespace RochBios.Core;

public static class AppPaths
{
    // Continue using an existing data folder so renaming the product does not
    // strand theme settings, verified tool caches, or USB-file backups.
    public static string DataDirectory
    {
        get
        {
            string local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            string current = Path.Combine(local, "Roch Microcode"), legacy = Path.Combine(local, "Roch BIOS");
            return Directory.Exists(legacy) && !Directory.Exists(current) ? legacy : current;
        }
    }
}
