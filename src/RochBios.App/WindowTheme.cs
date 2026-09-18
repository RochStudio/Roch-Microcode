using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;

namespace RochBios.App;

internal static class WindowTheme
{
    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(nint hwnd, int attribute, ref int value, int size);

    public static void Apply(Window window, bool light)
    {
        var handle = new WindowInteropHelper(window).Handle;
        if (handle == 0) return;
        Set(handle, 20, light ? 0 : 1);
        // Caption/text/border colours are supported on Windows 11. Older Windows
        // versions reject these attributes and retain their native frame behaviour.
        Set(handle, 35, ColorRef(window, "Canvas"));
        Set(handle, 36, ColorRef(window, "Text"));
        Set(handle, 34, ColorRef(window, "Outline"));
    }

    private static int ColorRef(Window window, string resource)
    {
        var color = ((SolidColorBrush)window.FindResource(resource)).Color;
        return color.R | color.G << 8 | color.B << 16;
    }

    private static void Set(nint handle, int attribute, int value) =>
        _ = DwmSetWindowAttribute(handle, attribute, ref value, sizeof(int));
}
