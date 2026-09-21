use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use deviceout_update::paths;
use deviceout_update::{schedule, Cmp, Feed, PendingManifest, State};

use crate::{logutil, unix_now};

const FEED_MAX_BYTES: u64 = 64 * 1024;

pub fn run(bundle: &Path, force: bool) -> Result<()> {
    let dll = deviceout_update::bundle_dll(bundle);
    if !dll.is_file() {
        bail!("bundle has no plugin binary: {}", dll.display());
    }
    let mut state = deviceout_update::load_state();
    if !schedule::should_check(&state, unix_now(), force) {
        logutil::log("check: backoff");
        return Ok(());
    }

    state.last_attempt = Some(unix_now());
    state.last_error = None;
    let _ = deviceout_update::save_state(&state);

    let feed_url = paths::BUILTIN_FEED_URL;
    logutil::log(&format!("check: feed {feed_url}"));

    let current = current_version(bundle);
    let stale = needs_full_fetch(&state, &current, deviceout_update::pending_ready().as_ref());
    if stale {
        logutil::log("check: known update has no installer, ignoring the cached feed");
    }
    let etag = if force || stale {
        None
    } else {
        state.feed_etag.as_deref()
    };
    let (bytes, etag) = match fetch_feed(feed_url, etag)? {
        Fetch::NotModified => {
            finish_ok(&mut state, None);
            logutil::log("check: 304");
            return Ok(());
        }
        Fetch::NothingPublished => {
            finish_ok(&mut state, None);
            logutil::log("check: no feed published");
            return Ok(());
        }
        Fetch::Body { bytes, etag } => (bytes, etag),
    };

    let sig_url = format!("{feed_url}.sig");
    let sig = http_get_bytes(&sig_url).with_context(|| format!("sig {sig_url}"))?;
    if !deviceout_update::verify(&bytes, &sig) {
        bail!("feed signature rejected");
    }

    let feed = deviceout_update::parse_feed(&bytes).map_err(|e| anyhow::anyhow!(e))?;
    state.feed_etag = etag;

    match deviceout_update::cmp_latest(&feed.version, &current) {
        Cmp::Newer => {}
        Cmp::EqualOrOlder => {
            logutil::log(&format!(
                "check: idle latest={} current={current}",
                feed.version
            ));
            finish_ok(&mut state, Some(&feed.version));
            return Ok(());
        }
        Cmp::Invalid => bail!("invalid semver latest={} current={current}", feed.version),
    }

    let writable = deviceout_update::bundle_writable(bundle);
    download_pending(&feed, bundle)?;
    finish_ok(&mut state, Some(&feed.version));
    if writable {
        logutil::log(&format!("check: pending {} ready", feed.version));
    } else {
        logutil::log(&format!(
            "check: pending {} downloaded, bundle not writable (manual)",
            feed.version
        ));
    }
    Ok(())
}

fn needs_full_fetch(state: &State, current: &str, pending: Option<&PendingManifest>) -> bool {
    let Some(latest) = state.last_latest.as_deref() else {
        return false;
    };
    if !matches!(deviceout_update::cmp_latest(latest, current), Cmp::Newer) {
        return false;
    }
    !pending.is_some_and(|p| p.version == latest)
}

fn finish_ok(state: &mut State, latest: Option<&str>) {
    state.last_check = Some(unix_now());
    state.last_error = None;
    if let Some(v) = latest {
        state.last_latest = Some(v.to_string());
    }
    let _ = deviceout_update::save_state(state);
}

fn current_version(bundle: &Path) -> String {
    let dll = deviceout_update::bundle_dll(bundle);
    deviceout_update::file_version_string(&dll)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

enum Fetch {
    NotModified,
    NothingPublished,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
}

fn fetch_feed(url: &str, etag: Option<&str>) -> Result<Fetch> {
    let agent = crate::http::agent_lenient(Duration::from_secs(20));
    let mut req = agent.get(url);
    if let Some(etag) = etag {
        req = req.header("If-None-Match", etag);
    }
    let mut resp = req.call()?;
    match resp.status().as_u16() {
        304 => return Ok(Fetch::NotModified),
        404 => return Ok(Fetch::NothingPublished),
        200 => {}
        code => bail!("feed http status {code}"),
    }
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(FEED_MAX_BYTES)
        .read_to_end(&mut bytes)
        .context("read feed")?;
    Ok(Fetch::Body { bytes, etag })
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let agent = crate::http::agent(Duration::from_secs(30));
    let mut resp = agent.get(url).call()?;
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(FEED_MAX_BYTES)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn download_pending(feed: &Feed, bundle: &Path) -> Result<()> {
    std::fs::create_dir_all(paths::pending_dir())?;
    let part = paths::pending_dir().join("setup.exe.part");
    let agent = crate::http::agent(Duration::from_secs(120));
    let mut resp = agent
        .get(&feed.url)
        .call()
        .with_context(|| feed.url.clone())?;
    {
        let mut file = std::fs::File::create(&part).context("create part")?;
        let mut reader = resp.body_mut().as_reader();
        std::io::copy(&mut reader, &mut file).context("download setup")?;
        file.flush()?;
    }
    let actual = deviceout_update::sha256_file(&part)?;
    if actual != feed.sha256.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&part);
        bail!("sha256 mismatch");
    }
    let dest = paths::pending_setup_path();
    deviceout_update::replace_file(&part, &dest).context("rename setup")?;
    let manifest = deviceout_update::PendingManifest {
        version: feed.version.clone(),
        sha256: actual,
        bundle_path: bundle.display().to_string(),
    };
    deviceout_update::write_manifest(&manifest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(latest: &str) -> State {
        State {
            last_latest: Some(latest.into()),
            feed_etag: Some("\"abc\"".into()),
            ..State::default()
        }
    }

    fn pending(version: &str) -> PendingManifest {
        PendingManifest {
            version: version.into(),
            sha256: "a".repeat(64),
            bundle_path: "C:\\DeviceOut.vst3".into(),
        }
    }

    #[test]
    fn a_ready_installer_lets_the_cached_feed_stand() {
        assert!(!needs_full_fetch(
            &state("1.1.2"),
            "1.1.1",
            Some(&pending("1.1.2"))
        ));
    }

    #[test]
    fn a_missing_installer_forces_a_full_fetch() {
        assert!(needs_full_fetch(&state("1.1.2"), "1.1.1", None));
        assert!(needs_full_fetch(
            &state("1.1.2"),
            "1.1.1",
            Some(&pending("1.1.0"))
        ));
    }

    #[test]
    fn nothing_to_install_means_nothing_to_refetch() {
        assert!(!needs_full_fetch(&State::default(), "1.1.1", None));
        assert!(!needs_full_fetch(&state("1.1.1"), "1.1.1", None));
        assert!(!needs_full_fetch(&state("1.0.0"), "1.1.1", None));
        assert!(!needs_full_fetch(&state("nonsense"), "1.1.1", None));
    }
}
