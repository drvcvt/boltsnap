[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$NoAutostart
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:OS -ne 'Windows_NT') {
    throw 'Dieses Skript kann nur unter Windows ausgefuehrt werden.'
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$sourceExe = Join-Path $repoRoot 'target\release\boltsnap.exe'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Boltsnap'
$installExe = Join-Path $installDir 'boltsnap.exe'
$stateDir = Join-Path $env:LOCALAPPDATA 'boltsnap'
$statePath = Join-Path $stateDir 'windows-integration.json'
$uninstallSource = Join-Path $PSScriptRoot 'uninstall-windows-snipping-replacement.ps1'
$uninstallTarget = Join-Path $installDir 'uninstall-windows-snipping-replacement.ps1'
$keyboardPath = 'HKCU:\Control Panel\Keyboard'
$keyboardName = 'PrintScreenKeyForSnippingEnabled'
$runPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'Boltsnap'
$taskName = 'Boltsnap Daemon'

if (-not $SkipBuild) {
    Write-Host 'Erstelle Boltsnap Release-Binary ...'
    & cargo build --release --manifest-path (Join-Path $repoRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release ist mit Exitcode $LASTEXITCODE fehlgeschlagen."
    }
}
if (-not (Test-Path -LiteralPath $sourceExe -PathType Leaf)) {
    throw "Release-Binary fehlt: $sourceExe"
}

$state = if (Test-Path -LiteralPath $statePath -PathType Leaf) {
    Get-Content -LiteralPath $statePath -Raw -Encoding utf8 | ConvertFrom-Json
} else {
    $keyboardProperty = Get-ItemProperty -LiteralPath $keyboardPath -Name $keyboardName -ErrorAction SilentlyContinue
    $runProperty = Get-ItemProperty -LiteralPath $runPath -Name $runName -ErrorAction SilentlyContinue
    [ordered]@{
        version = 1
        print_screen_present = $null -ne $keyboardProperty
        print_screen_value = if ($null -ne $keyboardProperty) { $keyboardProperty.$keyboardName } else { $null }
        run_present = $null -ne $runProperty
        run_value = if ($null -ne $runProperty) { $runProperty.$runName } else { $null }
    }
}

Get-CimInstance Win32_Process |
    Where-Object { $_.Name -eq 'boltsnap.exe' -and $_.CommandLine -like '* daemon*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force -Path $installDir, $stateDir | Out-Null
$state | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding utf8
Copy-Item -LiteralPath $sourceExe -Destination $installExe -Force
Copy-Item -LiteralPath $uninstallSource -Destination $uninstallTarget -Force

New-ItemProperty `
    -LiteralPath $keyboardPath `
    -Name $keyboardName `
    -PropertyType DWord `
    -Value 0 `
    -Force | Out-Null

if ($NoAutostart) {
    Remove-ItemProperty -LiteralPath $runPath -Name $runName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
} else {
    $runCommand = '"{0}" daemon' -f $installExe
    New-ItemProperty `
        -LiteralPath $runPath `
        -Name $runName `
        -PropertyType String `
        -Value $runCommand `
        -Force | Out-Null

    $taskAction = New-ScheduledTaskAction `
        -Execute $installExe `
        -Argument 'daemon' `
        -WorkingDirectory $installDir
    $taskTrigger = New-ScheduledTaskTrigger `
        -AtLogOn `
        -User ([System.Security.Principal.WindowsIdentity]::GetCurrent().Name)
    $taskTrigger.Delay = 'PT3S'
    $taskPrincipal = New-ScheduledTaskPrincipal `
        -UserId ([System.Security.Principal.WindowsIdentity]::GetCurrent().Name) `
        -LogonType Interactive `
        -RunLevel Limited
    $taskSettings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -StartWhenAvailable `
        -MultipleInstances IgnoreNew `
        -RestartCount 3 `
        -RestartInterval (New-TimeSpan -Minutes 1) `
        -ExecutionTimeLimit ([TimeSpan]::Zero)
    Register-ScheduledTask `
        -TaskName $taskName `
        -Action $taskAction `
        -Trigger $taskTrigger `
        -Principal $taskPrincipal `
        -Settings $taskSettings `
        -Description 'Starts the Boltsnap screenshot daemon for the current user at logon.' `
        -Force | Out-Null
}

if ($NoAutostart) {
    Start-Process -FilePath $installExe -ArgumentList 'daemon' -WindowStyle Hidden
} else {
    Start-ScheduledTask -TaskName $taskName
}

$daemon = $null
for ($attempt = 0; $attempt -lt 50 -and $null -eq $daemon; $attempt++) {
    Start-Sleep -Milliseconds 100
    $daemon = Get-CimInstance Win32_Process |
        Where-Object { $_.ExecutablePath -eq $installExe -and $_.CommandLine -like '* daemon*' } |
        Select-Object -First 1
}
if ($null -eq $daemon) {
    throw 'Boltsnap wurde installiert, aber der Daemon konnte nicht gestartet werden.'
}

Write-Host ''
Write-Host 'Boltsnap ersetzt jetzt die Windows-Snipping-Hotkeys.' -ForegroundColor Green
Write-Host "Installiert: $installExe"
Write-Host 'Druck          -> Boltsnap-Bereichsauswahl'
Write-Host 'Win+Shift+S    -> Boltsnap-Bereichsauswahl'
Write-Host 'Alt+Shift+S    -> Boltsnap-Bereichsaufnahme'
if (-not $NoAutostart) {
    Write-Host "Autostart      -> Windows-Logon-Task '$taskName' plus Run-Fallback"
}
Write-Host "Rueckgaengig: powershell -ExecutionPolicy Bypass -File `"$uninstallTarget`""
Write-Host 'Falls Windows einmalig noch Snipping Tool oeffnet: ab- und wieder anmelden.'
