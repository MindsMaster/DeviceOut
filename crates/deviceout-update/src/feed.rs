use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Feed {
    pub version: String,
    pub sha256: String,
    pub url: String,
}

pub fn parse_feed(bytes: &[u8]) -> Result<Feed, String> {
    let mut feed: Feed = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let Some(version) = crate::version::release_version(&feed.version) else {
        return Err(format!("version must be plain major.minor.patch, got {:?}", feed.version));
    };
    feed.version = version.to_string();
    feed.sha256 = feed.sha256.trim().to_ascii_lowercase();
    if feed.sha256.len() != 64 || !feed.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("sha256 must be 64 hex chars".into());
    }
    if !feed.url.starts_with("https://") {
        return Err("url must be https".into());
    }
    Ok(feed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_unknown_keys() {
        let json = br#"{"version":"0.1.1","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","url":"https://example.com/x.exe","channel":"stable","extra":1}"#;
        let feed = parse_feed(json).unwrap();
        assert_eq!(feed.version, "0.1.1");
        assert_eq!(feed.url, "https://example.com/x.exe");
    }

    #[test]
    fn rejects_http_url() {
        let json = br#"{"version":"0.1.1","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","url":"http://example.com/x.exe"}"#;
        assert!(parse_feed(json).is_err());
    }

    #[test]
    fn rejects_prerelease_build_and_malformed_hashes() {
        let sha = "a".repeat(64);
        for version in ["1.0.0-rc.1", "1.0.0+build", "1.0", "", "abc"] {
            let json = format!(
                r#"{{"version":"{version}","sha256":"{sha}","url":"https://example.com/x.exe"}}"#
            );
            assert!(parse_feed(json.as_bytes()).is_err(), "{version}");
        }
        for sha in ["a".repeat(63), "g".repeat(64)] {
            let json = format!(
                r#"{{"version":"1.0.0","sha256":"{sha}","url":"https://example.com/x.exe"}}"#
            );
            assert!(parse_feed(json.as_bytes()).is_err());
        }
        let upper = format!(
            r#"{{"version":"1.0.0","sha256":"{}","url":"https://example.com/x.exe"}}"#,
            "A".repeat(64)
        );
        assert_eq!(parse_feed(upper.as_bytes()).unwrap().sha256, "a".repeat(64));
    }
}
