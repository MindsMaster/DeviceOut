use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Newer,
    EqualOrOlder,
    Invalid,
}

pub fn release_version(s: &str) -> Option<semver::Version> {
    let v = semver::Version::parse(s.trim()).ok()?;
    (v.pre.is_empty() && v.build.is_empty()).then_some(v)
}

pub fn cmp_latest(latest: &str, current: &str) -> Cmp {
    let (Some(latest), Some(current)) = (release_version(latest), release_version(current)) else {
        return Cmp::Invalid;
    };
    if latest.cmp_precedence(&current) == std::cmp::Ordering::Greater {
        Cmp::Newer
    } else {
        Cmp::EqualOrOlder
    }
}

pub fn bundle_writable(bundle: &Path) -> bool {
    let dir = bundle.join("Contents").join("x86_64-win");
    let dir = if dir.is_dir() {
        dir
    } else if bundle.is_dir() {
        bundle.to_path_buf()
    } else {
        match bundle.parent() {
            Some(p) => p.to_path_buf(),
            None => return false,
        }
    };
    let probe = dir.join(".deviceout-write-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn bundle_dll_locked(bundle: &Path) -> bool {
    let dll = crate::bundle_dll(bundle);
    if !dll.exists() {
        return false;
    }
    std::fs::OpenOptions::new().write(true).open(&dll).is_err()
}

pub fn scope_label(bundle: &Path) -> &'static str {
    let text = bundle.to_string_lossy();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if text.starts_with(&local) {
            return "user";
        }
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("\\program files") || lower.contains("\\common files") {
        return "machine";
    }
    "unknown"
}

pub fn loaded_bundle_path() -> Option<PathBuf> {
    let dll = loaded_module_path()?;
    bundle_from_dll(&dll)
}

pub fn file_version_string(path: &Path) -> Option<String> {
    let (maj, min, pat, _build) = file_version_parts(path)?;
    Some(format!("{maj}.{min}.{pat}"))
}

fn name_is(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(expected))
}

pub fn valid_bundle_path(bundle: &Path) -> bool {
    bundle.is_absolute()
        && bundle
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("vst3"))
        && bundle.parent().is_some()
}

fn bundle_from_dll(dll: &Path) -> Option<PathBuf> {
    if !name_is(dll, crate::BUNDLE_NAME) {
        return None;
    }
    let win = dll.parent().filter(|p| name_is(p, "x86_64-win"))?;
    let contents = win.parent().filter(|p| name_is(p, "Contents"))?;
    let bundle = contents.parent()?;
    valid_bundle_path(bundle).then(|| bundle.to_path_buf())
}

#[cfg(windows)]
fn loaded_module_path() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::MAX_PATH;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    let mut handle = std::ptr::null_mut();
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let addr = loaded_module_path as *const u16;
    let ok = unsafe { GetModuleHandleExW(flags, addr, &mut handle) };
    if ok == 0 || handle.is_null() {
        return None;
    }
    let mut buf = vec![0u16; MAX_PATH as usize];
    let n = unsafe { GetModuleFileNameW(handle, buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buf[..n as usize],
    )))
}

#[cfg(not(windows))]
fn loaded_module_path() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn file_version_parts(path: &Path) -> Option<(u16, u16, u16, u16)> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    #[repr(C)]
    struct VsFixedFileInfo {
        signature: u32,
        struc_version: u32,
        file_vers_ms: u32,
        file_vers_ls: u32,
        product_vers_ms: u32,
        product_vers_ls: u32,
        flags_mask: u32,
        flags: u32,
        os: u32,
        file_type: u32,
        subtype: u32,
        date_ms: u32,
        date_ls: u32,
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut dummy = 0u32;
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), &mut dummy) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    let ok = unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr() as *mut _) };
    if ok == 0 {
        return None;
    }
    let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len = 0u32;
    let query: Vec<u16> = "\\\0".encode_utf16().collect();
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr() as *const _,
            query.as_ptr(),
            &mut ptr,
            &mut len,
        )
    };
    if ok == 0 || ptr.is_null() || (len as usize) < std::mem::size_of::<VsFixedFileInfo>() {
        return None;
    }
    let info = unsafe { &*(ptr as *const VsFixedFileInfo) };
    let maj = (info.file_vers_ms >> 16) as u16;
    let min = (info.file_vers_ms & 0xffff) as u16;
    let pat = (info.file_vers_ls >> 16) as u16;
    let build = (info.file_vers_ls & 0xffff) as u16;
    Some((maj, min, pat, build))
}

#[cfg(not(windows))]
fn file_version_parts(_path: &Path) -> Option<(u16, u16, u16, u16)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_is_not_newer() {
        assert_eq!(cmp_latest("0.1.0", "0.1.0"), Cmp::EqualOrOlder);
        assert_eq!(cmp_latest("0.1.0", "0.1.1"), Cmp::EqualOrOlder);
        assert_eq!(cmp_latest("0.1.1", "0.1.0"), Cmp::Newer);
        assert_eq!(cmp_latest("1.0.0", "0.9.9"), Cmp::Newer);
    }

    #[test]
    fn only_plain_release_versions_are_accepted() {
        assert_eq!(cmp_latest("1.0.0+hotfix", "1.0.0"), Cmp::Invalid);
        assert_eq!(cmp_latest("1.1.0-rc.1", "1.0.0"), Cmp::Invalid);
        assert_eq!(cmp_latest("1.0.0", "1.0.0-beta"), Cmp::Invalid);
        assert_eq!(cmp_latest("1.0.0.0", "1.0.0"), Cmp::Invalid);
        assert_eq!(cmp_latest("v1.0.0", "1.0.0"), Cmp::Invalid);
        assert!(release_version(" 1.2.3 ").is_some());
    }

    #[cfg(windows)]
    #[test]
    fn bundle_is_derived_only_from_a_real_bundle_layout() {
        let dll = Path::new(
            r"C:\Users\me\AppData\Local\Programs\Common\VST3\DeviceOut.vst3\Contents\x86_64-win\DeviceOut.vst3",
        );
        assert_eq!(
            bundle_from_dll(dll),
            Some(PathBuf::from(
                r"C:\Users\me\AppData\Local\Programs\Common\VST3\DeviceOut.vst3"
            ))
        );
        assert_eq!(
            bundle_from_dll(Path::new(
                r"C:\Users\me\AppData\Local\DeviceOut\deviceout-updater.exe"
            )),
            None
        );
        assert_eq!(bundle_from_dll(Path::new(r"C:\a\b\c\DeviceOut.vst3")), None);
        assert_eq!(
            bundle_from_dll(Path::new(r"C:\x.vst3\Contents\x86_64-win\Other.vst3")),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn bundle_paths_must_be_absolute_vst3_dirs() {
        assert!(valid_bundle_path(Path::new(
            r"C:\Program Files\Common Files\VST3\DeviceOut.vst3"
        )));
        assert!(!valid_bundle_path(Path::new(r"C:\Users\me\AppData")));
        assert!(!valid_bundle_path(Path::new("")));
        assert!(!valid_bundle_path(Path::new(r"DeviceOut.vst3")));
    }
}
