<#
.SYNOPSIS
    Builds the Candela MSI from a staged payload directory.

.DESCRIPTION
    Point -StageDir at a directory whose contents are exactly what should land
    in the install folder: candela.exe, candela-vm.exe, the receipt, the libs/
    tree, and the license files. Everything in it is harvested into the package.

    Requires the WiX command line tool on PATH:
        dotnet tool install --global wix --version 6.0.1

.EXAMPLE
    ./build-msi.ps1 -Version 0.0.2 -StageDir ../msi-stage -OutFile ../candela-x86_64-windows.msi
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$StageDir,
    [Parameter(Mandatory = $true)][string]$OutFile
)

$ErrorActionPreference = 'Stop'

# MSI compares only the first three numeric fields of ProductVersion, so the
# version has to be plain x.y.z. A tag suffix would silently not upgrade.
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must be x.y.z, got '$Version'."
}

if (-not (Test-Path -LiteralPath $StageDir -PathType Container)) {
    throw "StageDir '$StageDir' does not exist."
}

# WiX resolves the harvest glob relative to its own working directory, so give
# it a path that does not depend on where this script was started from.
$stage = (Resolve-Path -LiteralPath $StageDir).ProviderPath.TrimEnd('\')

if (-not (Get-ChildItem -LiteralPath $stage)) {
    throw "StageDir '$stage' is empty."
}

foreach ($required in @('candela.exe', 'candela-vm.exe', 'receipt', 'libs')) {
    if (-not (Test-Path -LiteralPath (Join-Path $stage $required))) {
        throw "StageDir '$stage' is missing '$required'."
    }
}

$outDir = Split-Path -Parent $OutFile
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}
$out = if ($outDir) {
    Join-Path (Resolve-Path -LiteralPath $outDir).ProviderPath (Split-Path -Leaf $OutFile)
} else {
    Join-Path (Get-Location).ProviderPath $OutFile
}

$source = Join-Path $PSScriptRoot 'candela.wxs'

Write-Host "Building $out from $stage at version $Version"

# No ICE suppressions. Add one only when a check fires on something a per-user
# package is meant to do, with a comment naming the check and the reason.
& wix build `
    -arch x64 `
    -d "Version=$Version" `
    -d "StageDir=$stage" `
    -o $out `
    $source

if ($LASTEXITCODE -ne 0) {
    throw "wix build failed with exit code $LASTEXITCODE."
}

Write-Host "Built $out"
