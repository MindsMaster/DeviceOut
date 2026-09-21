use deviceout_i18n::Script;
use nih_plug_egui::egui::{FontData, FontFamily};

pub(crate) const NOTO_SANS: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
pub(crate) const NOTO_THAI: &[u8] = include_bytes!("../../assets/fonts/NotoSansThai-Regular.ttf");
pub(crate) const NOTO_CJK: &[u8] = include_bytes!("../../assets/fonts/NotoSansCJK-Regular.ttc");

pub(crate) const NAME_SANS: &str = "noto-sans";
pub(crate) const NAME_THAI: &str = "noto-thai";
pub(crate) const NAME_CJK_JP: &str = "noto-cjk-jp";
pub(crate) const NAME_CJK_KR: &str = "noto-cjk-kr";
pub(crate) const NAME_CJK_SC: &str = "noto-cjk-sc";
pub(crate) const NAME_CJK_TC: &str = "noto-cjk-tc";

const CJK_JP: u32 = 0;
const CJK_KR: u32 = 1;
const CJK_SC: u32 = 2;
const CJK_TC: u32 = 3;

pub(crate) fn script_family(script: Script) -> FontFamily {
    FontFamily::Name(match script {
        Script::Latin => return FontFamily::Proportional,
        Script::Hans => "deviceout-hans".into(),
        Script::Hant => "deviceout-hant".into(),
        Script::Japanese => "deviceout-ja".into(),
        Script::Korean => "deviceout-ko".into(),
        Script::Thai => "deviceout-th".into(),
    })
}

pub(crate) fn cjk_data(index: u32) -> FontData {
    let mut data = FontData::from_static(NOTO_CJK);
    data.index = index;
    data
}

pub(crate) fn script_font_name(script: Script) -> &'static str {
    match script {
        Script::Latin => NAME_SANS,
        Script::Hans => NAME_CJK_SC,
        Script::Hant => NAME_CJK_TC,
        Script::Japanese => NAME_CJK_JP,
        Script::Korean => NAME_CJK_KR,
        Script::Thai => NAME_THAI,
    }
}

pub(crate) fn gdi_face(script: Script) -> &'static str {
    match script {
        Script::Latin => "Noto Sans",
        Script::Hans => "Noto Sans CJK SC",
        Script::Hant => "Noto Sans CJK TC",
        Script::Japanese => "Noto Sans CJK JP",
        Script::Korean => "Noto Sans CJK KR",
        Script::Thai => "Noto Sans Thai",
    }
}

pub(crate) fn named_family_chain(script: Script) -> Vec<String> {
    match script {
        Script::Latin => vec![NAME_SANS.into()],
        Script::Thai => vec![NAME_SANS.into(), NAME_THAI.into()],
        _ => vec![NAME_SANS.into(), script_font_name(script).into()],
    }
}

pub(crate) fn ui_chain(script: Script) -> Vec<String> {
    let mut chain = named_family_chain(script);
    let extras = [NAME_CJK_SC, NAME_CJK_JP, NAME_THAI];
    for name in extras {
        if !chain.iter().any(|n| n == name) {
            chain.push(name.into());
        }
    }
    chain
}

pub(crate) fn cjk_faces() -> [(&'static str, u32); 4] {
    [
        (NAME_CJK_JP, CJK_JP),
        (NAME_CJK_KR, CJK_KR),
        (NAME_CJK_SC, CJK_SC),
        (NAME_CJK_TC, CJK_TC),
    ]
}
