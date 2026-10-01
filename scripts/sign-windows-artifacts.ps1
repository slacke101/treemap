[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string[]]$Files
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_CERTIFICATE_PFX)) {
    Write-Output 'No code-signing certificate configured; artifact will be unsigned.'
    exit 0
}
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD)) {
    throw 'WINDOWS_SIGNING_CERTIFICATE_PASSWORD is required when a certificate is configured.'
}

$certificatePath = Join-Path $env:RUNNER_TEMP 'treemap-signing.pfx'
try {
    [System.IO.File]::WriteAllBytes(
        $certificatePath,
        [Convert]::FromBase64String($env:WINDOWS_SIGNING_CERTIFICATE_PFX)
    )
    $signtool = Get-ChildItem -Path "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter signtool.exe -Recurse |
        Where-Object { $_.Directory.Name -eq 'x64' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $signtool) {
        throw 'Windows SDK signtool.exe was not found.'
    }

    foreach ($file in $Files) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "Artifact to sign not found: $file"
        }
        & $signtool.FullName sign /fd SHA256 /f $certificatePath /p $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD /tr http://timestamp.digicert.com /td SHA256 $file
        if ($LASTEXITCODE -ne 0) {
            throw "signtool.exe failed for '$file' with exit code $LASTEXITCODE."
        }
    }
}
finally {
    Remove-Item -LiteralPath $certificatePath -Force -ErrorAction SilentlyContinue
}