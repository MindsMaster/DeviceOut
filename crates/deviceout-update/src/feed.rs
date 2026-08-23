use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Feed {
    pub version: String,
    pub sha256: String,
    pub url: String,
}

pub fn parse_feed(bytes: &[u8]) -> Result<Feed, String> {
    let mut feed: Feed = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if feed.version.trim().is_empty() {
        return Err("missing version".into());
    }
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
}
