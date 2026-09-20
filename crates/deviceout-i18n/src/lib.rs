mod lang;
mod strings;

use std::sync::atomic::{AtomicU8, Ordering};

pub use strings::{fill, Strings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    ZhHans,
    ZhHant,
    Ja,
    Ko,
    De,
    Fr,
    Es,
    PtBr,
    Ru,
    Th,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Latin,
    Hans,
    Hant,
    Japanese,
    Korean,
    Thai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    System,
    Fixed(Lang),
}

impl Lang {
    pub const ALL: [Lang; 11] = [
        Lang::En,
        Lang::ZhHans,
        Lang::ZhHant,
        Lang::Ja,
        Lang::Ko,
        Lang::De,
        Lang::Fr,
        Lang::Es,
        Lang::PtBr,
        Lang::Ru,
        Lang::Th,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::ZhHans => "zh-Hans",
            Lang::ZhHant => "zh-Hant",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::PtBr => "pt-BR",
            Lang::Ru => "ru",
            Lang::Th => "th",
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::ZhHans => "简体中文",
            Lang::ZhHant => "繁體中文",
            Lang::Ja => "日本語",
            Lang::Ko => "한국어",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::PtBr => "Português (Brasil)",
            Lang::Ru => "Русский",
            Lang::Th => "ไทย",
        }
    }

    pub fn script(self) -> Script {
        match self {
            Lang::ZhHans => Script::Hans,
            Lang::ZhHant => Script::Hant,
            Lang::Ja => Script::Japanese,
            Lang::Ko => Script::Korean,
            Lang::Th => Script::Thai,
            _ => Script::Latin,
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &lang::en::STRINGS,
            Lang::ZhHans => &lang::zh_hans::STRINGS,
            Lang::ZhHant => &lang::zh_hant::STRINGS,
            Lang::Ja => &lang::ja::STRINGS,
            Lang::Ko => &lang::ko::STRINGS,
            Lang::De => &lang::de::STRINGS,
            Lang::Fr => &lang::fr::STRINGS,
            Lang::Es => &lang::es::STRINGS,
            Lang::PtBr => &lang::pt_br::STRINGS,
            Lang::Ru => &lang::ru::STRINGS,
            Lang::Th => &lang::th::STRINGS,
        }
    }

    pub fn from_tag(tag: &str) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|l| l.tag().eq_ignore_ascii_case(tag.trim()))
    }

    pub fn from_locale(locale: &str) -> Option<Lang> {
        let lower = locale.trim().to_ascii_lowercase().replace('_', "-");
        let mut parts = lower.split('-');
        let language = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        match language {
            "en" => Some(Lang::En),
            "zh" => {
                let traditional = rest
                    .iter()
                    .any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"));
                Some(if traditional {
                    Lang::ZhHant
                } else {
                    Lang::ZhHans
                })
            }
            "ja" => Some(Lang::Ja),
            "ko" => Some(Lang::Ko),
            "de" => Some(Lang::De),
            "fr" => Some(Lang::Fr),
            "es" => Some(Lang::Es),
            "pt" => Some(Lang::PtBr),
            "ru" => Some(Lang::Ru),
            "th" => Some(Lang::Th),
            _ => None,
        }
    }

    fn code(self) -> u8 {
        Lang::ALL
            .iter()
            .position(|l| *l == self)
            .map_or(0, |i| i as u8 + 1)
    }

    fn from_code(code: u8) -> Option<Lang> {
        Lang::ALL.get(usize::from(code).checked_sub(1)?).copied()
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    if let Some(l) = Lang::from_code(CURRENT.load(Ordering::Relaxed)) {
        return l;
    }
    let resolved = resolve(preference());
    CURRENT.store(resolved.code(), Ordering::Relaxed);
    resolved
}

pub fn t() -> &'static Strings {
    current().strings()
}

pub fn preference() -> Preference {
    let Ok(text) = std::fs::read_to_string(preference_path()) else {
        return Preference::System;
    };
    parse_preference(&text)
}

pub fn set_preference(pref: Preference) {
    let path = preference_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = match pref {
        Preference::System => "system".to_string(),
        Preference::Fixed(l) => l.tag().to_string(),
    };
    let _ = std::fs::write(path, text);
    CURRENT.store(resolve(pref).code(), Ordering::Relaxed);
}

pub fn system_lang() -> Lang {
    system_locale()
        .and_then(|l| Lang::from_locale(&l))
        .unwrap_or(Lang::En)
}

fn resolve(pref: Preference) -> Lang {
    match pref {
        Preference::System => system_lang(),
        Preference::Fixed(l) => l,
    }
}

fn parse_preference(text: &str) -> Preference {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("system") {
        return Preference::System;
    }
    Lang::from_tag(text)
        .or_else(|| Lang::from_locale(text))
        .map_or(Preference::System, Preference::Fixed)
}

fn preference_path() -> std::path::PathBuf {
    deviceout_update::paths::appdata_dir().join("lang.txt")
}

#[cfg(windows)]
pub fn system_locale() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 1 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..(len - 1) as usize]))
}

#[cfg(not(windows))]
pub fn system_locale() -> Option<String> {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .ok()
        .map(|s| s.split('.').next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_map_to_the_nearest_supported_language() {
        assert_eq!(Lang::from_locale("en-US"), Some(Lang::En));
        assert_eq!(Lang::from_locale("en-GB"), Some(Lang::En));
        assert_eq!(Lang::from_locale("zh-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_locale("zh-Hans-SG"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_locale("zh-TW"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_locale("zh-Hant-HK"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_locale("zh-MO"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_locale("pt-BR"), Some(Lang::PtBr));
        assert_eq!(Lang::from_locale("pt-PT"), Some(Lang::PtBr));
        assert_eq!(Lang::from_locale("es-MX"), Some(Lang::Es));
        assert_eq!(Lang::from_locale("ru_RU"), Some(Lang::Ru));
        assert_eq!(Lang::from_locale("th-TH"), Some(Lang::Th));
        assert_eq!(Lang::from_locale("ja-JP"), Some(Lang::Ja));
        assert_eq!(Lang::from_locale("ko-KR"), Some(Lang::Ko));
        assert_eq!(Lang::from_locale("de-AT"), Some(Lang::De));
        assert_eq!(Lang::from_locale("fr-CA"), Some(Lang::Fr));
        assert_eq!(Lang::from_locale("vi-VN"), None);
        assert_eq!(Lang::from_locale(""), None);
    }

    #[test]
    fn old_preference_files_still_parse() {
        assert_eq!(parse_preference("zh"), Preference::Fixed(Lang::ZhHans));
        assert_eq!(parse_preference("en\n"), Preference::Fixed(Lang::En));
        assert_eq!(parse_preference("pt-BR"), Preference::Fixed(Lang::PtBr));
        assert_eq!(parse_preference("system"), Preference::System);
        assert_eq!(parse_preference(""), Preference::System);
        assert_eq!(parse_preference("klingon"), Preference::System);
    }

    #[test]
    fn tags_round_trip_and_codes_are_unique() {
        for l in Lang::ALL {
            assert_eq!(Lang::from_tag(l.tag()), Some(l));
            assert_eq!(Lang::from_code(l.code()), Some(l));
            assert!(!l.native_name().is_empty());
        }
        assert_eq!(Lang::from_code(0), None);
    }

    type Field = fn(&Strings) -> &'static str;

    #[test]
    fn every_language_fills_every_placeholder_the_english_table_uses() {
        let en = Lang::En.strings();
        let templates: [(&str, Field); 10] = [
            ("fault_channel_mismatch", |s| s.fault_channel_mismatch),
            ("fault_retry", |s| s.fault_retry),
            ("alert_dropouts", |s| s.alert_dropouts),
            ("alert_reconnects", |s| s.alert_reconnects),
            ("alert_clamps", |s| s.alert_clamps),
            ("mix_format", |s| s.mix_format),
            ("update_ready", |s| s.update_ready),
            ("update_available", |s| s.update_available),
            ("up_to_date", |s| s.up_to_date),
            ("updater_failed", |s| s.updater_failed),
        ];
        for lang in Lang::ALL {
            let s = lang.strings();
            for (name, get) in templates {
                for key in placeholders(get(en)) {
                    assert!(get(s).contains(&key), "{} {name} lacks {key}", lang.tag());
                }
            }
        }
    }

    #[test]
    fn sample_formats_are_translated_in_every_non_english_language() {
        let english = Lang::En.strings();
        for lang in Lang::ALL {
            if lang == Lang::En {
                continue;
            }
            let strings = lang.strings();
            for (translated, original) in [
                (strings.sample_f32, english.sample_f32),
                (strings.sample_i16, english.sample_i16),
                (strings.sample_i32, english.sample_i32),
            ] {
                assert!(!translated.trim().is_empty(), "{}", lang.tag());
                assert_ne!(translated, original, "{}: {original}", lang.tag());
            }
        }
    }

    fn placeholders(template: &str) -> Vec<String> {
        template
            .split('{')
            .skip(1)
            .filter_map(|rest| rest.split('}').next())
            .map(|k| format!("{{{k}}}"))
            .collect()
    }
}
