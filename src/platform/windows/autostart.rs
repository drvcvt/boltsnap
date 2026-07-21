use std::env;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use crate::DynResult;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TASK_NAME: &str = "Boltsnap Daemon";

pub fn install() -> DynResult<()> {
    let controller = env::current_exe()?;
    let working_directory = controller
        .parent()
        .ok_or("Boltsnap executable has no parent directory")?;
    let executable = working_directory.join("boltsnap-background.exe");
    if !executable.is_file() {
        return Err(format!(
            "Boltsnap background launcher is missing: {}",
            executable.display()
        )
        .into());
    }
    let script = r#"
$ErrorActionPreference = 'Stop'
$taskName = $env:BOLTSNAP_TASK_NAME
$executable = $env:BOLTSNAP_TASK_EXECUTABLE
$workingDirectory = $env:BOLTSNAP_TASK_WORKING_DIRECTORY
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
$action = New-ScheduledTaskAction -Execute $executable -WorkingDirectory $workingDirectory
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$trigger.Delay = 'PT3S'
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero)
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'Starts the Boltsnap screenshot daemon for the current user at logon.' -Force | Out-Null
Start-ScheduledTask -TaskName $taskName
"#;
    let output = run_powershell(script, Some((&executable, working_directory)))?;
    ensure_success("register Boltsnap autostart task", output)?;

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if crate::ipc::daemon_alive() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("Boltsnap autostart task was registered but the daemon did not start".into())
}

pub fn remove() -> DynResult<()> {
    let script = r#"
$ErrorActionPreference = 'Stop'
$taskName = $env:BOLTSNAP_TASK_NAME
Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
"#;
    let output = run_powershell(script, None)?;
    ensure_success("remove Boltsnap autostart task", output)
}

fn run_powershell(script: &str, paths: Option<(&Path, &Path)>) -> DynResult<Output> {
    let windows = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = Path::new(&windows)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("BOLTSNAP_TASK_NAME", TASK_NAME)
        .creation_flags(CREATE_NO_WINDOW);
    if let Some((executable, working_directory)) = paths {
        command
            .env("BOLTSNAP_TASK_EXECUTABLE", executable)
            .env("BOLTSNAP_TASK_WORKING_DIRECTORY", working_directory);
    }
    Ok(command.output()?)
}

fn ensure_success(action: &str, output: Output) -> DynResult<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!("could not {action}: {detail}").into())
}
