[CmdletBinding()]
param(
    [string]$EddyRepository = "",
    [string]$QtDirectory = "C:\Qt\6.9.3\msvc2022_64",
    [string]$Version = "1.0.0",
    [string]$OutputDirectory = "dist\nsis",
    [string]$MakeNsis = "",
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
$buildRoot = Join-Path $repository "target\nsis"
$stageRoot = Join-Path $buildRoot "stage"
$boltsnapStage = Join-Path $stageRoot "Boltsnap"
$eddyStage = Join-Path $stageRoot "Eddy"
$output = [System.IO.Path]::GetFullPath((Join-Path $repository $OutputDirectory))
$source = Join-Path $PSScriptRoot "Boltsnap.nsi"
$license = Join-Path $PSScriptRoot "License.rtf"
$boltsnapExecutable = Join-Path $repository "target\release\boltsnap.exe"
$boltsnapBackgroundExecutable = Join-Path $repository "target\release\boltsnap-background.exe"
$eddyBuild = Join-Path $EddyRepository "build-win"
$eddyExecutable = Join-Path $eddyBuild "Release\eddy.exe"
$windeployqt = Join-Path $QtDirectory "bin\windeployqt.exe"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must use MAJOR.MINOR.PATCH: $Version"
}
if (-not $MakeNsis) {
    $command = Get-Command makensis -ErrorAction SilentlyContinue
    if ($command) {
        $MakeNsis = $command.Source
    } else {
        $MakeNsis = Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe"
    }
}

foreach ($required in @($source, $license, $windeployqt, $MakeNsis,
        (Join-Path $repository "Cargo.toml"), (Join-Path $EddyRepository "CMakeLists.txt"))) {
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
foreach ($required in @($boltsnapExecutable, $boltsnapBackgroundExecutable, $eddyExecutable)) {
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
New-Item -ItemType Directory -Path $boltsnapStage, $eddyStage, $output -Force | Out-Null
Copy-Item -LiteralPath $boltsnapExecutable -Destination (Join-Path $boltsnapStage "boltsnap.exe")
Copy-Item -LiteralPath $boltsnapBackgroundExecutable `
    -Destination (Join-Path $boltsnapStage "boltsnap-background.exe")
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

$installer = Join-Path $output "Boltsnap-$Version-windows-x64-setup.exe"
& $MakeNsis "/V4" "/WX" "/DPRODUCT_VERSION=$Version" "/DOUTPUT_FILE=$installer" `
    "/DBOLTSNAP_SOURCE_DIR=$boltsnapStage" "/DEDDY_SOURCE_DIR=$eddyStage" `
    "/DLICENSE_FILE=$license" $source
if ($LASTEXITCODE -ne 0) {
    throw "NSIS failed with exit code $LASTEXITCODE"
}

Write-Output $installer
