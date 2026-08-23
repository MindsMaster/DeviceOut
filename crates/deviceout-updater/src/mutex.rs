pub struct MutexGuard {
    handle: isize,
}

pub fn try_acquire(name: &str) -> Option<MutexGuard> {
    acquire(name, false, 0)
}

pub fn wait_acquire(name: &str, timeout_ms: u32) -> Option<MutexGuard> {
    acquire(name, true, timeout_ms)
}

fn acquire(name: &str, wait: bool, timeout_ms: u32) -> Option<MutexGuard> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        use windows_sys::Win32::Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
        };
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let handle: HANDLE = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already && !wait {
            unsafe { CloseHandle(handle) };
            return None;
        }
        let rc = unsafe { WaitForSingleObject(handle, if wait { timeout_ms } else { 0 }) };
        if rc != WAIT_OBJECT_0 {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(MutexGuard {
            handle: handle as isize,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (name, wait, timeout_ms);
        Some(MutexGuard { handle: 0 })
    }
}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
            use windows_sys::Win32::System::Threading::ReleaseMutex;
            if self.handle != 0 {
                let handle = self.handle as HANDLE;
                unsafe {
                    ReleaseMutex(handle);
                    CloseHandle(handle);
                }
            }
        }
    }
}
