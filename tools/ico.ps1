<#
Build assets/noiseman.ico (the Windows icon that build.rs embeds in noiseman.exe) from the PNG icons: 48-256 px as
they are, 16-32 px scaled down from the 256 px one. Each image is stored PNG-compressed (Windows Vista+).
Run after changing the PNGs:  powershell -NoProfile -File tools/ico.ps1
#>
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assets = Join-Path $PSScriptRoot '..\assets'

function Scaled([int]$size) {
    $src = [Drawing.Image]::FromFile((Join-Path $assets 'noiseman-256.png'))
    $bmp = New-Object Drawing.Bitmap $size, $size
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($src, 0, 0, $size, $size)
    $ms = New-Object IO.MemoryStream
    $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose(); $src.Dispose()
    , $ms.ToArray()
}

$sizes = 16, 24, 32, 48, 64, 128, 256
$images = @(foreach ($s in $sizes) {
    if ($s -lt 48) { , (Scaled $s) } else { , [IO.File]::ReadAllBytes((Join-Path $assets "noiseman-$s.png")) }
})

$out = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter $out
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)  # reserved, type 1 (icon), count
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $b = [byte]($sizes[$i] % 256)  # 256 is written as 0
    $w.Write($b); $w.Write($b); $w.Write([byte]0); $w.Write([byte]0)  # width, height, palette, reserved
    $w.Write([uint16]1); $w.Write([uint16]32)  # planes, bits per pixel
    $w.Write([uint32]$images[$i].Length); $w.Write([uint32]$offset)
    $offset += $images[$i].Length
}
foreach ($img in $images) { $w.Write($img) }
$w.Flush()
[IO.File]::WriteAllBytes((Join-Path $assets 'noiseman.ico'), $out.ToArray())
"wrote assets/noiseman.ico ($($sizes.Count) sizes, $($out.Length) bytes)"
