use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub fn with_file_lock(
    path: &Path,
    write: impl FnOnce(Option<&[u8]>) -> std::io::Result<Vec<u8>>,
) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    lock_exclusive(&file)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let existing = if buf.is_empty() {
        None
    } else {
        Some(buf.as_slice())
    };
    let out = write(existing)?;
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&out)?;
    file.flush()?;
    unlock(&file)?;
    Ok(())
}

#[cfg(windows)]
fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK};
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            u32::MAX,
            u32::MAX,
            &mut ov,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn unlock(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        UnlockFileEx(
            file.as_raw_handle() as HANDLE,
            0,
            u32::MAX,
            u32::MAX,
            &mut ov,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn lock_exclusive(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn unlock(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
