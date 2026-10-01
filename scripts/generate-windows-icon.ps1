[CmdletBinding()]
param(
    [string]$SourcePath = (Join-Path $PSScriptRoot '..\TreeMapicon.png'),
    [string]$OutputPath = (Join-Path $PSScriptRoot '..\TreeMapicon.ico')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$source = [System.Drawing.Image]::FromFile([System.IO.Path]::GetFullPath($SourcePath))
$bitmap = [System.Drawing.Bitmap]::new(256, 256, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$stream = [System.IO.MemoryStream]::new()

try {
    $graphics.Clear([System.Drawing.Color]::Transparent)
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality

    $scale = [Math]::Min(232.0 / $source.Width, 232.0 / $source.Height)
    $width = [int][Math]::Round($source.Width * $scale)
    $height = [int][Math]::Round($source.Height * $scale)
    $bounds = [System.Drawing.Rectangle]::new(
        [int](($bitmap.Width - $width) / 2),
        [int](($bitmap.Height - $height) / 2),
        $width,
        $height
    )

    $graphics.DrawImage($source, $bounds)
    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $png = $stream.ToArray()

    $icon = [byte[]]::new(22 + $png.Length)
    [Array]::Copy([BitConverter]::GetBytes([UInt16]0), 0, $icon, 0, 2)
    [Array]::Copy([BitConverter]::GetBytes([UInt16]1), 0, $icon, 2, 2)
    [Array]::Copy([BitConverter]::GetBytes([UInt16]1), 0, $icon, 4, 2)
    $icon[6] = 0
    $icon[7] = 0
    $icon[8] = 0
    $icon[9] = 0
    [Array]::Copy([BitConverter]::GetBytes([UInt16]1), 0, $icon, 10, 2)
    [Array]::Copy([BitConverter]::GetBytes([UInt16]32), 0, $icon, 12, 2)
    [Array]::Copy([BitConverter]::GetBytes([UInt32]$png.Length), 0, $icon, 14, 4)
    [Array]::Copy([BitConverter]::GetBytes([UInt32]22), 0, $icon, 18, 4)
    [Array]::Copy($png, 0, $icon, 22, $png.Length)

    [System.IO.File]::WriteAllBytes([System.IO.Path]::GetFullPath($OutputPath), $icon)
}
finally {
    $stream.Dispose()
    $graphics.Dispose()
    $bitmap.Dispose()
    $source.Dispose()
}

Write-Output "Created $OutputPath"