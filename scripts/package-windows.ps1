[CmdletBinding()]
param(
    [string]$TargetDir = (Join-Path $PSScriptRoot '..\target\release'),
    [string]$OutputDir = (Join-Path $PSScriptRoot '..\dist')
)

$ErrorActionPreference = 'Stop'

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$targetPath = [System.IO.Path]::GetFullPath($TargetDir)
$outputPath = [System.IO.Path]::GetFullPath($OutputDir)
$binaryPath = Join-Path $targetPath 'DirMap.exe'

if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "Release executable not found: $binaryPath. Build the app first."
}

$stagePath = Join-Path $outputPath ".treemap-package-$([guid]::NewGuid().ToString('N'))"
$archivePath = Join-Path $outputPath 'TreeMap-windows-x86_64.zip'
$checksumPath = "$archivePath.sha256"

New-Item -ItemType Directory -Path $outputPath -Force | Out-Null

try {
    New-Item -ItemType Directory -Path $stagePath | Out-Null
    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stagePath 'TreeMap.exe')
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'README.md') -Destination $stagePath
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -Destination (Join-Path $stagePath 'LICENSE.txt')
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'open-source-core\LICENSE') -Destination (Join-Path $stagePath 'LICENSE-open-source-core.txt')

    Compress-Archive -Path (Join-Path $stagePath '*') -DestinationPath $archivePath -Force
    $hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $(Split-Path -Leaf $archivePath)" | Set-Content -LiteralPath $checksumPath -Encoding ascii
}
finally {
    Remove-Item -LiteralPath $stagePath -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Output "Created $archivePath"
Write-Output "Created $checksumPath"