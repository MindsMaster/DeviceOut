use std::ffi::OsStr;
use std::path::Path;

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Exited(u32),
    TimedOut,
}

pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '\n' | '"')) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

pub fn command_line(exe: &Path, args: &[String]) -> String {
    let mut line = quote_arg(&exe.display().to_string());
    for a in args {
        line.push(' ');
        line.push_str(&quote_arg(a));
    }
    line
}

#[cfg(windows)]
pub fn run_tree(exe: &Path, args: &[String], timeout_ms: u32) -> Result<Outcome> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, WaitForSingleObject, CREATE_NO_WINDOW,
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    let job = Handle(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
    if job.0.is_null() {
        bail!("CreateJobObject failed: {}", unsafe { GetLastError() });
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let ok = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        bail!("SetInformationJobObject failed: {}", unsafe {
            GetLastError()
        });
    }

    let mut cmd: Vec<u16> = OsStr::new(&command_line(exe, args))
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if created == 0 {
        bail!("CreateProcess failed: {}", unsafe { GetLastError() });
    }
    let process = Handle(pi.hProcess);
    let thread = Handle(pi.hThread);

    if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
        let err = unsafe { GetLastError() };
        unsafe { TerminateJobObject(job.0, 1) };
        bail!("AssignProcessToJobObject failed: {err}");
    }
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        let err = unsafe { GetLastError() };
        unsafe { TerminateJobObject(job.0, 1) };
        bail!("ResumeThread failed: {err}");
    }

    match unsafe { WaitForSingleObject(process.0, timeout_ms) } {
        WAIT_OBJECT_0 => {
            let mut code = 0u32;
            if unsafe { GetExitCodeProcess(process.0, &mut code) } == 0 {
                bail!("GetExitCodeProcess failed: {}", unsafe { GetLastError() });
            }
            Ok(Outcome::Exited(code))
        }
        WAIT_TIMEOUT => {
            unsafe { TerminateJobObject(job.0, 1) };
            Ok(Outcome::TimedOut)
        }
        _ => bail!("WaitForSingleObject failed: {}", unsafe { GetLastError() }),
    }
}

#[cfg(not(windows))]
pub fn run_tree(exe: &Path, args: &[String], _timeout_ms: u32) -> Result<Outcome> {
    let status = std::process::Command::new(exe).args(args).status()?;
    Ok(Outcome::Exited(status.code().unwrap_or(1) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_args_pass_through() {
        assert_eq!(quote_arg("/VERYSILENT"), "/VERYSILENT");
        assert_eq!(quote_arg(r"/DIR=C:\a\b.vst3"), r"/DIR=C:\a\b.vst3");
    }

    #[test]
    fn spaces_are_quoted_and_trailing_backslashes_doubled() {
        assert_eq!(
            quote_arg(r"/DIR=C:\Users\John Doe\DeviceOut.vst3"),
            r#""/DIR=C:\Users\John Doe\DeviceOut.vst3""#
        );
        assert_eq!(quote_arg(r"C:\space dir\"), r#""C:\space dir\\""#);
        assert_eq!(quote_arg(""), r#""""#);
    }

    #[test]
    fn embedded_quotes_are_escaped() {
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }

    #[test]
    fn command_line_joins_exe_and_args() {
        let line = command_line(
            Path::new(r"C:\Temp\setup exe\DeviceOut-Setup.exe"),
            &["/VERYSILENT".into(), "/LOG=C:\\a b\\x.log".into()],
        );
        assert_eq!(
            line,
            r#""C:\Temp\setup exe\DeviceOut-Setup.exe" /VERYSILENT "/LOG=C:\a b\x.log""#
        );
    }

    #[cfg(windows)]
    fn shell() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("ComSpec").expect("ComSpec"))
    }

    #[cfg(windows)]
    #[test]
    fn job_kills_a_hung_process_tree() {
        let started = std::time::Instant::now();
        let outcome = run_tree(
            &shell(),
            &["/C".into(), "ping 127.0.0.1 -n 30 >nul".into()],
            500,
        )
        .unwrap();
        assert_eq!(outcome, Outcome::TimedOut);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let pings = std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq ping.exe", "/NH"])
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&pings.stdout);
        assert!(
            !listing.to_ascii_lowercase().contains("ping.exe"),
            "{listing}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn job_reports_exit_code() {
        let outcome = run_tree(&shell(), &["/C".into(), "exit 7".into()], 10_000).unwrap();
        assert_eq!(outcome, Outcome::Exited(7));
    }
}
