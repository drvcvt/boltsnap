[CmdletBinding()]
param(
    [string]$EddyRepository = "",
    [string]$QtDirectory = "C:\Qt\6.9.3\msvc2022_64",
    [string]$Version = "0.4.5",
    [string]$OutputDirectory = "dist\msi",
    [switch]$SkipBuild,
    [switch]$SkipEddyBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repository = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if (-not $EddyRepository) {
    $EddyRepository = [System.IO.Path]::GetFullPath((Join-Path $repository "..\..\eddy windows\eddy"))
} else {
    $EddyRepository = [System.IO.Path]::GetFullPath($EddyRepository)
}
$buildRoot = Join-Path $repository "target\msi"
$stageRoot = Join-Path $buildRoot "stage"
$boltsnapStage = Join-Path $stageRoot "Boltsnap"
$eddyStage = Join-Path $stageRoot "Eddy"
$output = [System.IO.Path]::GetFullPath((Join-Path $repository $OutputDirectory))
$source = Join-Path $PSScriptRoot "Boltsnap.wxs"
$license = Join-Path $PSScriptRoot "License.rtf"
$boltsnapExecutable = Join-Path $repository "target\release\boltsnap.exe"
$eddyBuild = Join-Path $EddyRepository "build-win"
$eddyExecutable = Join-Path $eddyBuild "Release\eddy.exe"
$windeployqt = Join-Path $QtDirectory "bin\windeployqt.exe"
$wix = Join-Path $buildRoot "wix\wix.exe"

foreach ($required in @($source, $license, $windeployqt, (Join-Path $repository "Cargo.toml"),
        (Join-Path $EddyRepository "CMakeLists.txt"))) {
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
if (-not $SkipEddyBuild) {
    $cmake = Get-Command cmake -ErrorAction SilentlyContinue
    if ($cmake) {
        if (-not (Test-Path -LiteralPath (Join-Path $eddyBuild "CMakeCache.txt") -PathType Leaf)) {
            & $cmake.Source -S $EddyRepository -B $eddyBuild -G "Visual Studio 17 2022" -A x64 `
                "-DCMAKE_PREFIX_PATH=$QtDirectory"
            if ($LASTEXITCODE -ne 0) {
                throw "Eddy CMake configure failed with exit code $LASTEXITCODE"
            }
        }
        & $cmake.Source --build $eddyBuild --config Release --target eddy
        if ($LASTEXITCODE -ne 0) {
            throw "Eddy release build failed with exit code $LASTEXITCODE"
        }
    } elseif (Test-Path -LiteralPath $eddyExecutable -PathType Leaf) {
        Write-Warning "cmake is unavailable; packaging the existing Eddy release binary."
    } else {
        throw "cmake is unavailable and no Eddy release executable exists: $eddyExecutable"
    }
}
foreach ($required in @($boltsnapExecutable, $eddyExecutable)) {
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
New-Item -ItemType Directory -Path $boltsnapStage, $eddyStage, $output -Force | Out-Null
Copy-Item -LiteralPath $boltsnapExecutable -Destination (Join-Path $boltsnapStage "boltsnap.exe")
Copy-Item -LiteralPath $eddyExecutable -Destination (Join-Path $eddyStage "eddy.exe")

& $windeployqt --release --dir $eddyStage $eddyExecutable
if ($LASTEXITCODE -ne 0) {
    throw "windeployqt failed with exit code $LASTEXITCODE"
}

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw "Visual Studio locator not found: $vswhere"
}
$visualStudio = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Redist.14.Latest -property installationPath
$redistRoot = Join-Path $visualStudio "VC\Redist\MSVC"
$visualStudioRedist = Get-ChildItem -LiteralPath $redistRoot -Directory |
    ForEach-Object { Join-Path $_.FullName "x64\Microsoft.VC143.CRT" } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Container } |
    Sort-Object -Descending |
    Select-Object -First 1
if (-not $visualStudioRedist) {
    throw "Visual C++ runtime directory not found below: $redistRoot"
}
Get-ChildItem -LiteralPath $visualStudioRedist -Filter "*.dll" -File |
    Copy-Item -Destination $eddyStage

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
    -d "EddySourceDir=$eddyStage" `
    -d "ProductVersion=$Version" `
    -d "LicenseRtf=$license" `
    -o $msi
if ($LASTEXITCODE -ne 0) {
    throw "WiX failed with exit code $LASTEXITCODE"
}

Write-Output $msi
