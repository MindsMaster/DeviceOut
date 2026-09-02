const UNKNOWN: &str = "未知";

const NAMES: &[(&str, &str)] = &[
    ("CN", "中国"),
    ("TW", "中国台湾"),
    ("HK", "中国香港"),
    ("MO", "中国澳门"),
    ("US", "美国"),
    ("CA", "加拿大"),
    ("MX", "墨西哥"),
    ("BR", "巴西"),
    ("AR", "阿根廷"),
    ("CL", "智利"),
    ("CO", "哥伦比亚"),
    ("PE", "秘鲁"),
    ("VE", "委内瑞拉"),
    ("UY", "乌拉圭"),
    ("CR", "哥斯达黎加"),
    ("GB", "英国"),
    ("IE", "爱尔兰"),
    ("DE", "德国"),
    ("FR", "法国"),
    ("ES", "西班牙"),
    ("PT", "葡萄牙"),
    ("IT", "意大利"),
    ("NL", "荷兰"),
    ("BE", "比利时"),
    ("CH", "瑞士"),
    ("AT", "奥地利"),
    ("SE", "瑞典"),
    ("NO", "挪威"),
    ("DK", "丹麦"),
    ("FI", "芬兰"),
    ("PL", "波兰"),
    ("CZ", "捷克"),
    ("HU", "匈牙利"),
    ("RO", "罗马尼亚"),
    ("GR", "希腊"),
    ("UA", "乌克兰"),
    ("RU", "俄罗斯"),
    ("KZ", "哈萨克斯坦"),
    ("TR", "土耳其"),
    ("IL", "以色列"),
    ("SA", "沙特阿拉伯"),
    ("AE", "阿联酋"),
    ("EG", "埃及"),
    ("MA", "摩洛哥"),
    ("ZA", "南非"),
    ("NG", "尼日利亚"),
    ("KE", "肯尼亚"),
    ("IN", "印度"),
    ("PK", "巴基斯坦"),
    ("BD", "孟加拉国"),
    ("JP", "日本"),
    ("KR", "韩国"),
    ("TH", "泰国"),
    ("VN", "越南"),
    ("ID", "印度尼西亚"),
    ("MY", "马来西亚"),
    ("SG", "新加坡"),
    ("PH", "菲律宾"),
    ("AU", "澳大利亚"),
    ("NZ", "新西兰"),
    ("419", "拉丁美洲"),
    ("001", "国际"),
];

const TZ_COUNTRIES: &[(&str, &str)] = &[
    ("China Standard Time", "CN"),
    ("Asia/Shanghai", "CN"),
    ("Asia/Urumqi", "CN"),
    ("Taipei Standard Time", "TW"),
    ("Asia/Taipei", "TW"),
    ("Asia/Hong_Kong", "HK"),
    ("Asia/Macau", "MO"),
    ("Tokyo Standard Time", "JP"),
    ("Asia/Tokyo", "JP"),
    ("Korea Standard Time", "KR"),
    ("Asia/Seoul", "KR"),
    ("Russian Standard Time", "RU"),
    ("Europe/Moscow", "RU"),
    ("E. South America Standard Time", "BR"),
    ("America/Sao_Paulo", "BR"),
    ("Turkey Standard Time", "TR"),
    ("Israel Standard Time", "IL"),
    ("India Standard Time", "IN"),
    ("Egypt Standard Time", "EG"),
    ("South Africa Standard Time", "ZA"),
    ("New Zealand Standard Time", "NZ"),
];

pub fn country_code(locale: &str, tz: &str) -> Option<String> {
    let from_locale = locale
        .trim()
        .split(['-', '_'])
        .skip(1)
        .find(|part| {
            (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
                || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_ascii_uppercase);
    if from_locale.is_some() {
        return from_locale;
    }
    let tz = tz.trim();
    TZ_COUNTRIES
        .iter()
        .find(|(name, _)| tz.eq_ignore_ascii_case(name))
        .map(|(_, code)| (*code).to_string())
}

pub fn country_name(code: &str) -> String {
    NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| code.to_string())
}

pub fn country_of(locale: &str, tz: &str) -> String {
    country_code(locale, tz)
        .map(|code| country_name(&code))
        .unwrap_or_else(|| UNKNOWN.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_region_wins() {
        assert_eq!(country_of("en-US", "China Standard Time"), "美国");
        assert_eq!(country_of("es-MX", ""), "墨西哥");
        assert_eq!(country_of("pt-BR", ""), "巴西");
        assert_eq!(country_of("zh-CN", ""), "中国");
        assert_eq!(country_of("zh-Hant-TW", ""), "中国台湾");
        assert_eq!(country_of("ru_RU", ""), "俄罗斯");
        assert_eq!(country_of("th-TH", ""), "泰国");
        assert_eq!(country_of("es-419", ""), "拉丁美洲");
    }

    #[test]
    fn time_zone_fills_in_when_the_locale_has_no_region() {
        assert_eq!(country_of("zh", "China Standard Time"), "中国");
        assert_eq!(country_of("", "Asia/Taipei"), "中国台湾");
        assert_eq!(country_of("ja", "Tokyo Standard Time"), "日本");
        assert_eq!(country_of("en", "Pacific Standard Time"), "未知");
        assert_eq!(country_of("", ""), "未知");
    }

    #[test]
    fn unlisted_regions_show_their_code() {
        assert_eq!(country_of("xx-ZZ", ""), "ZZ");
        assert_eq!(country_code("de-de", ""), Some("DE".to_string()));
    }
}
