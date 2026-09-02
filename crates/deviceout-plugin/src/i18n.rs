use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    fn code(self) -> u8 {
        match self {
            Self::Zh => 1,
            Self::En => 2,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Zh),
            2 => Some(Self::En),
            _ => None,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    if let Some(l) = Lang::from_code(LANG.load(Ordering::Relaxed)) {
        return l;
    }
    let resolved = override_lang().unwrap_or_else(detect);
    LANG.store(resolved.code(), Ordering::Relaxed);
    resolved
}

pub fn set_lang(l: Lang) {
    LANG.store(l.code(), Ordering::Relaxed);
    let path = deviceout_update::paths::appdata_dir().join("lang.txt");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tag = match l {
        Lang::Zh => "zh",
        Lang::En => "en",
    };
    let _ = std::fs::write(path, tag);
}

pub fn pick<'a>(zh: &'a str, en: &'a str) -> &'a str {
    match lang() {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

fn override_lang() -> Option<Lang> {
    let text =
        std::fs::read_to_string(deviceout_update::paths::appdata_dir().join("lang.txt")).ok()?;
    match text.trim() {
        "zh" => Some(Lang::Zh),
        "en" => Some(Lang::En),
        _ => None,
    }
}

#[cfg(windows)]
fn detect() -> Lang {
    match user_locale_name() {
        Some(name) if name.starts_with("zh") => Lang::Zh,
        _ => Lang::En,
    }
}

#[cfg(not(windows))]
fn detect() -> Lang {
    match std::env::var("LANG") {
        Ok(lang) if lang.to_lowercase().starts_with("zh") => Lang::Zh,
        _ => Lang::En,
    }
}

#[cfg(windows)]
fn user_locale_name() -> Option<String> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    type FnGetUserDefaultLocaleName = unsafe extern "system" fn(*mut u16, i32) -> i32;

    let module_name: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let module = GetModuleHandleW(module_name.as_ptr());
        if module.is_null() {
            return None;
        }
        let p = GetProcAddress(module, c"GetUserDefaultLocaleName".as_ptr().cast())?;
        let f: FnGetUserDefaultLocaleName = std::mem::transmute(p);
        let mut buf = [0u16; 85];
        let len = f(buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..(len - 1) as usize]).to_lowercase())
    }
}
