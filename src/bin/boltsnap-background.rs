#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
fn main() {
    use std::env;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::core::PCWSTR;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let result = env::current_exe()
        .ok()
        .and_then(|launcher| {
            launcher
                .parent()
                .map(|directory| directory.join("boltsnap.exe"))
        })
        .and_then(|boltsnap| unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_err()
            {
                let _ = CloseHandle(job);
                return None;
            }

            let mut child = Command::new(boltsnap)
                .arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .ok()?;
            let process = HANDLE(child.as_raw_handle().cast::<c_void>());
            if AssignProcessToJobObject(job, process).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = CloseHandle(job);
                return None;
            }
            let status = child.wait().ok();
            let _ = CloseHandle(job);
            status
        });

    let exit_code = result.and_then(|status| status.code()).unwrap_or(1);
    std::process::exit(exit_code);
}

#[cfg(not(target_os = "windows"))]
fn main() {}
