use std::path::Path;

use crate::paths;

pub fn sanitize_user_paths(input: &str) -> String {
    let mut s = input.to_string();
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if !profile.is_empty() {
            s = s.replace(&profile, "%USERPROFILE%");
        }
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some((skip, _drive)) = match_users_prefix(&s, i) {
            if let Some(name_end) = next_sep_or_end(&s, i + skip) {
                out.push_str("%USERPROFILE%");
                i = name_end;
                continue;
            }
        }
        out.push(s[i..].chars().next().unwrap());
        i += s[i..].chars().next().unwrap().len_utf8();
    }
    out
}

fn match_users_prefix(s: &str, i: usize) -> Option<(usize, char)> {
    let rest = &s[i..];
    let lower = rest.to_ascii_lowercase();
    const PATTERNS: &[&str] = &[
        r"c:\users\",
        r"d:\users\",
        r"e:\users\",
        r"f:\users\",
        r"c:/users/",
    ];
    for p in PATTERNS {
        if lower.starts_with(p) {
            return Some((p.len(), rest.chars().next().unwrap()));
        }
    }
    if lower.starts_with(r"\\?\") {
        let inner = match_users_prefix(&rest[4..], 0)?;
        return Some((inner.0 + 4, inner.1));
    }
    None
}

fn next_sep_or_end(s: &str, start: usize) -> Option<usize> {
    if start >= s.len() {
        return None;
    }
    match s[start..].find(['\\', '/']) {
        Some(rel) => Some(start + rel),
        None => Some(s.len()),
    }
}

pub fn write_panic(info: &dyn std::fmt::Display) -> std::io::Result<()> {
    let path = paths::panic_log_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ts = now_stamp();
    let mut line = format!("{ts} panic: {info}\n");
    line = sanitize_user_paths(&line);
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(line.as_bytes())
}

pub fn build_diagnostics(bundle: Option<&Path>, plugin_version: &str) -> String {
    let mut lines = Vec::new();
    lines.push(format!("plugin_version={plugin_version}"));
    lines.push(format!("os={}", os_summary()));
    lines.push(format!(
        "cpus={}",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    ));
    if let Some((total, avail)) = ram_mb() {
        lines.push(format!("ram_mb={total}/{avail}"));
    }
    if let Some(cpu) = std::env::var_os("PROCESSOR_IDENTIFIER") {
        lines.push(format!("cpu={}", cpu.to_string_lossy()));
    }
    if let Some(arch) = std::env::var_os("PROCESSOR_ARCHITECTURE") {
        lines.push(format!("arch={}", arch.to_string_lossy()));
    } else {
        lines.push(format!("arch={}", std::env::consts::ARCH));
    }
    if let Some(bundle) = bundle {
        lines.push(format!(
            "bundle_path={}",
            sanitize_user_paths(&bundle.display().to_string())
        ));
        lines.push(format!("install_scope={}", crate::scope_label(bundle)));
    }
    let updater = paths::updater_exe();
    if updater.is_file() {
        let ver = crate::file_version_string(&updater).unwrap_or_else(|| "-".into());
        lines.push(format!("updater={ver}"));
    }
    if let Ok(host) = std::env::current_exe() {
        let name = host
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ver = crate::file_version_string(&host).unwrap_or_else(|| "-".into());
        lines.push(format!("host={name} {ver}"));
    }
    if let Some(err) = crate::load_state().last_error {
        lines.push(format!("last_error={err}"));
    }
    let log_tail = tail_file(&paths::update_log_path(), 32 * 1024);
    if !log_tail.is_empty() {
        lines.push("--- update.log ---".into());
        lines.push(sanitize_user_paths(&log_tail));
    }
    let panic_tail = tail_file(&paths::panic_log_path(), 8 * 1024);
    if !panic_tail.is_empty() {
        lines.push("--- panic.log ---".into());
        lines.push(sanitize_user_paths(&panic_tail));
    }
    lines.join("\n")
}

fn tail_file(path: &Path, max: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let slice = if bytes.len() > max {
        &bytes[bytes.len() - max..]
    } else {
        &bytes
    };
    String::from_utf8_lossy(slice).into_owned()
}

pub fn os_short() -> String {
    let base = os_summary();
    #[cfg(windows)]
    {
        format!("{base} {}", std::env::consts::ARCH)
    }
    #[cfg(not(windows))]
    {
        base
    }
}

fn os_summary() -> String {
    #[cfg(windows)]
    {
        match rtl_os_version() {
            Some((major, minor, build)) => {
                let name = if major >= 10 && build >= 22000 {
                    "Windows 11"
                } else if major >= 10 {
                    "Windows 10"
                } else {
                    "Windows"
                };
                format!("{name} {major}.{minor}.{build}")
            }
            None => {
                let root = std::env::var_os("SystemRoot")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
                let ntdll = root.join("System32").join("ntdll.dll");
                let ver = crate::file_version_string(&ntdll).unwrap_or_else(|| "?".into());
                format!("Windows {ver}")
            }
        }
    }
    #[cfg(not(windows))]
    {
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

#[cfg(windows)]
fn rtl_os_version() -> Option<(u32, u32, u32)> {
    #[repr(C)]
    struct OsVersionInfoW {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }

    unsafe {
        let mut info = OsVersionInfoW {
            dw_os_version_info_size: size_of::<OsVersionInfoW>() as u32,
            dw_major_version: 0,
            dw_minor_version: 0,
            dw_build_number: 0,
            dw_platform_id: 0,
            sz_csd_version: [0; 128],
        };
        if RtlGetVersion(&mut info) != 0 {
            return None;
        }
        Some((
            info.dw_major_version,
            info.dw_minor_version,
            info.dw_build_number,
        ))
    }
}

fn ram_mb() -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        unsafe {
            let mut info: MEMORYSTATUSEX = std::mem::zeroed();
            info.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut info) == 0 {
                return None;
            }
            Some((
                info.ullTotalPhys / (1024 * 1024),
                info.ullAvailPhys / (1024 * 1024),
            ))
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_users_path() {
        let s = r"C:\Users\ZhangSan\AppData\Local\DeviceOut\update.log";
        let out = sanitize_user_paths(s);
        assert!(!out.to_ascii_lowercase().contains("zhangsan"), "{out}");
        assert!(out.contains("%USERPROFILE%"), "{out}");
        assert!(out.contains(r"\AppData\Local\DeviceOut\update.log"), "{out}");
    }
}
