[CmdletBinding()]
param(
    [string]$Version = "1.0.0",
    [string]$OutputDirectory = "dist\nsis",
    [string]$MakeNsis = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repository = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$buildRoot = Join-Path $repository "target\nsis"
$stageRoot = Join-Path $buildRoot "stage"
$boltsnapStage = Join-Path $stageRoot "Boltsnap"
$output = [System.IO.Path]::GetFullPath((Join-Path $repository $OutputDirectory))
$source = Join-Path $PSScriptRoot "Boltsnap.nsi"
$license = Join-Path $PSScriptRoot "License.rtf"
$icon = Join-Path $repository "assets\windows\boltsnap.ico"
$boltsnapExecutable = Join-Path $repository "target\release\boltsnap.exe"
$boltsnapBackgroundExecutable = Join-Path $repository "target\release\boltsnap-background.exe"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must use MAJOR.MINOR.PATCH: $Version"
}
if (-not $MakeNsis) {
    $command = Get-Command makensis -ErrorAction SilentlyContinue
    $MakeNsis = if ($command) {
        $command.Source
    } else {
        Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe"
    }
}
foreach ($required in @($source, $license, $icon, $MakeNsis, (Join-Path $repository "Cargo.toml"))) {
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
    throw "Refusing to clean staging directory outside the NSIS build tree: $stageRoot"
}
if (Test-Path -LiteralPath $stageRoot) {
    Remove-Item -LiteralPath $stageRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $boltsnapStage, $output -Force | Out-Null
Copy-Item -LiteralPath $boltsnapExecutable -Destination (Join-Path $boltsnapStage "boltsnap.exe")
Copy-Item -LiteralPath $boltsnapBackgroundExecutable `
    -Destination (Join-Path $boltsnapStage "boltsnap-background.exe")

$installer = Join-Path $output "Boltsnap-$Version-windows-x64-setup.exe"
Get-ChildItem -LiteralPath $output -Filter "Boltsnap-*-windows-x64-setup.exe" -File |
    Where-Object { $_.FullName -ne $installer } |
    Remove-Item -Force
& $MakeNsis "/V4" "/WX" "/DPRODUCT_VERSION=$Version" "/DOUTPUT_FILE=$installer" `
    "/DBOLTSNAP_SOURCE_DIR=$boltsnapStage" "/DLICENSE_FILE=$license" "/DAPP_ICON=$icon" $source
if ($LASTEXITCODE -ne 0) {
    throw "NSIS failed with exit code $LASTEXITCODE"
}

Write-Output $installer
