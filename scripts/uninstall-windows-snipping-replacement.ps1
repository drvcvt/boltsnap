[CmdletBinding()]
param(
    [switch]$KeepProgramFiles
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Boltsnap'
$installExe = Join-Path $installDir 'boltsnap.exe'
$installedUninstaller = Join-Path $installDir 'uninstall-windows-snipping-replacement.ps1'
$statePath = Join-Path (Join-Path $env:LOCALAPPDATA 'boltsnap') 'windows-integration.json'
$keyboardPath = 'HKCU:\Control Panel\Keyboard'
$keyboardName = 'PrintScreenKeyForSnippingEnabled'
$runPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'Boltsnap'
$taskName = 'Boltsnap Daemon'

$state = if (Test-Path -LiteralPath $statePath -PathType Leaf) {
    Get-Content -LiteralPath $statePath -Raw -Encoding utf8 | ConvertFrom-Json
} else {
    $null
}

Get-CimInstance Win32_Process |
    Where-Object { $_.ExecutablePath -eq $installExe -and $_.CommandLine -like '* daemon*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

if ($null -ne $state -and $state.print_screen_present) {
    New-ItemProperty `
        -LiteralPath $keyboardPath `
        -Name $keyboardName `
        -PropertyType DWord `
        -Value ([int]$state.print_screen_value) `
        -Force | Out-Null
} else {
    Remove-ItemProperty -LiteralPath $keyboardPath -Name $keyboardName -ErrorAction SilentlyContinue
}

if ($null -ne $state -and $state.run_present) {
    New-ItemProperty `
        -LiteralPath $runPath `
        -Name $runName `
        -PropertyType String `
        -Value ([string]$state.run_value) `
        -Force | Out-Null
} else {
    Remove-ItemProperty -LiteralPath $runPath -Name $runName -ErrorAction SilentlyContinue
}

Remove-Item -LiteralPath $statePath -Force -ErrorAction SilentlyContinue
if (-not $KeepProgramFiles) {
    Remove-Item -LiteralPath $installExe -Force -ErrorAction SilentlyContinue
    if ($MyInvocation.MyCommand.Path -ne $installedUninstaller) {
        Remove-Item -LiteralPath $installedUninstaller -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $installDir -PathType Container) {
        $remaining = @(Get-ChildItem -LiteralPath $installDir -Force)
        if ($remaining.Count -eq 0) {
            Remove-Item -LiteralPath $installDir -Force
        }
    }
}

Write-Host 'Die Druck-Taste, der Run-Eintrag und der Logon-Task wurden zurueckgesetzt.' -ForegroundColor Green
if ($MyInvocation.MyCommand.Path -eq $installedUninstaller -and -not $KeepProgramFiles) {
    Write-Host "Der Uninstaller bleibt unter $installedUninstaller und kann jetzt manuell geloescht werden."
}
