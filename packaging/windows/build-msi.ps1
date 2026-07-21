[CmdletBinding()]
param(
    [string]$Version = "1.0.0",
    [string]$OutputDirectory = "dist\msi",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repository = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$buildRoot = Join-Path $repository "target\msi"
$stageRoot = Join-Path $buildRoot "stage"
$boltsnapStage = Join-Path $stageRoot "Boltsnap"
$output = [System.IO.Path]::GetFullPath((Join-Path $repository $OutputDirectory))
$source = Join-Path $PSScriptRoot "Boltsnap.wxs"
$license = Join-Path $PSScriptRoot "License.rtf"
$boltsnapExecutable = Join-Path $repository "target\release\boltsnap.exe"
$boltsnapBackgroundExecutable = Join-Path $repository "target\release\boltsnap-background.exe"
$wix = Join-Path $buildRoot "wix\wix.exe"

foreach ($required in @($source, $license, (Join-Path $repository "Cargo.toml"))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required file not found: $required"
    }
}

if (-not $SkipBuild) {
    & cargo build --release --manifest-path (Join-Path $repository "Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
        throw "Boltsnap release build failed with exit code $LASTEXITCODE"
    }
}
foreach ($required in @($boltsnapExecutable, $boltsnapBackgroundExecutable)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Release executable not found: $required"
    }
}

if (-not $stageRoot.StartsWith($buildRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to clean staging directory outside the MSI build tree: $stageRoot"
}
if (Test-Path -LiteralPath $stageRoot) {
    Remove-Item -LiteralPath $stageRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $boltsnapStage, $output -Force | Out-Null
Copy-Item -LiteralPath $boltsnapExecutable -Destination (Join-Path $boltsnapStage "boltsnap.exe")
Copy-Item -LiteralPath $boltsnapBackgroundExecutable `
    -Destination (Join-Path $boltsnapStage "boltsnap-background.exe")

if (-not (Test-Path -LiteralPath $wix -PathType Leaf)) {
    New-Item -ItemType Directory -Path (Split-Path $wix) -Force | Out-Null
    & dotnet tool install --tool-path (Split-Path $wix) wix --version 5.0.2
    if ($LASTEXITCODE -ne 0) {
        throw "Could not install the local WiX build tool"
    }
}
foreach ($extension in @("WixToolset.UI.wixext/5.0.2", "WixToolset.Util.wixext/5.0.2")) {
    & $wix extension add --global $extension
    if ($LASTEXITCODE -ne 0) {
        throw "Could not install WiX extension $extension"
    }
}

$msi = Join-Path $output "Boltsnap-$Version-windows-x64.msi"
& $wix build $source -arch x64 -culture en-US `
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -d "BoltsnapSourceDir=$boltsnapStage" `
    -d "ProductVersion=$Version" `
    -d "LicenseRtf=$license" `
    -o $msi
if ($LASTEXITCODE -ne 0) {
    throw "WiX failed with exit code $LASTEXITCODE"
}

Write-Output $msi
