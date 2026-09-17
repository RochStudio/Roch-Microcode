$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assetsPath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\assets'))
[IO.Directory]::CreateDirectory($assetsPath) | Out-Null
$images = @()
foreach ($size in @(16, 32, 48, 256)) {
    $bitmap = [Drawing.Bitmap]::new($size, $size)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.Clear([Drawing.Color]::FromArgb(20, 20, 20))
    $red = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(255, 77, 77))
    $white = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(245, 245, 245))
    $pen = [Drawing.Pen]::new($red, [single]($size * 0.04))
    $graphics.DrawRectangle($pen, [single]($size * 0.19), [single]($size * 0.19), [single]($size * 0.62), [single]($size * 0.62))
    foreach ($f in @(0.32, 0.50, 0.68)) {
        $graphics.DrawLine($pen, [single]($size * $f), [single]($size * 0.07), [single]($size * $f), [single]($size * 0.19))
        $graphics.DrawLine($pen, [single]($size * $f), [single]($size * 0.81), [single]($size * $f), [single]($size * 0.93))
        $graphics.DrawLine($pen, [single]($size * 0.07), [single]($size * $f), [single]($size * 0.19), [single]($size * $f))
        $graphics.DrawLine($pen, [single]($size * 0.81), [single]($size * $f), [single]($size * 0.93), [single]($size * $f))
    }
    $font = [Drawing.Font]::new('Segoe UI', [single]($size * 0.37), [Drawing.FontStyle]::Bold, [Drawing.GraphicsUnit]::Pixel)
    $format = [Drawing.StringFormat]::new()
    $format.Alignment = [Drawing.StringAlignment]::Center
    $format.LineAlignment = [Drawing.StringAlignment]::Center
    $graphics.DrawString('R', $font, $white, [Drawing.RectangleF]::new(0, 0, $size, [single]($size * 0.97)), $format)
    $memory = [IO.MemoryStream]::new()
    $bitmap.Save($memory, [Drawing.Imaging.ImageFormat]::Png)
    $images += ,$memory.ToArray()
    $memory.Dispose(); $format.Dispose(); $font.Dispose(); $pen.Dispose(); $red.Dispose(); $white.Dispose(); $graphics.Dispose(); $bitmap.Dispose()
}
$stream = [IO.File]::Create((Join-Path $assetsPath 'RochMicrocode.ico'))
$writer = [IO.BinaryWriter]::new($stream)
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]4)
$offset = 6 + 4 * 16
$sizes = @(16, 32, 48, 0)
for ($i = 0; $i -lt 4; $i++) {
    $writer.Write([byte]$sizes[$i]); $writer.Write([byte]$sizes[$i]); $writer.Write([byte]0); $writer.Write([byte]0)
    $writer.Write([uint16]1); $writer.Write([uint16]32); $writer.Write([uint32]$images[$i].Length); $writer.Write([uint32]$offset)
    $offset += $images[$i].Length
}
foreach ($imageBytes in $images) { $writer.Write([byte[]]$imageBytes) }
$writer.Dispose(); $stream.Dispose()
