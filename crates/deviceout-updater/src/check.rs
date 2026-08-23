use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use deviceout_update::paths;
use deviceout_update::{Cmp, Feed, State};

use crate::{logutil, unix_now};

const BACKOFF_SECS: i64 = 24 * 60 * 60;

pub fn run(bundle: Option<&Path>, force: bool) -> Result<()> {
    let mut state = deviceout_update::load_state();
    if !force {
        if let Some(last) = state.last_check {
            if unix_now().saturating_sub(last) < BACKOFF_SECS {
                logutil::log("check: backoff");
                return Ok(());
            }
        }
    }

    state.last_attempt = Some(unix_now());
    let _ = deviceout_update::save_state(&state);

    let feed_url = paths::BUILTIN_FEED_URL;
    logutil::log(&format!("check: feed {feed_url}"));

    let etag = if force { None } else { state.feed_etag.as_deref() };
    let (bytes, etag) = match fetch_feed(feed_url, etag) {
        Ok(Fetch::NotModified) => {
            finish_ok(&mut state, None);
            logutil::log("check: 304");
            return Ok(());
        }
        Ok(Fetch::Body { bytes, etag }) => (bytes, etag),
        Err(e) => return Err(e),
    };

    let sig_url = format!("{feed_url}.sig");
    let sig = http_get_bytes(&sig_url).with_context(|| format!("sig {sig_url}"))?;
    if !deviceout_update::verify(&bytes, &sig) {
        bail!("feed signature rejected");
    }

    let feed = deviceout_update::parse_feed(&bytes).map_err(|e| anyhow::anyhow!(e))?;
    state.feed_etag = etag;

    let current = current_version(bundle);
    match deviceout_update::cmp_latest(&feed.version, &current) {
        Cmp::Newer => {}
        Cmp::EqualOrOlder => {
            logutil::log(&format!("check: idle latest={} current={current}", feed.version));
            finish_ok(&mut state, Some(&feed.version));
            return Ok(());
        }
        Cmp::Invalid => bail!("invalid semver latest={} current={current}", feed.version),
    }

    let writable = bundle.map(deviceout_update::bundle_writable).unwrap_or(false);
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

fn finish_ok(state: &mut State, latest: Option<&str>) {
    state.last_check = Some(unix_now());
    state.last_error = None;
    if let Some(v) = latest {
        state.last_latest = Some(v.to_string());
    }
    let _ = deviceout_update::save_state(state);
}

fn current_version(bundle: Option<&Path>) -> String {
    if let Some(bundle) = bundle {
        let dll = deviceout_update::bundle_dll(bundle);
        if let Some(v) = deviceout_update::file_version_string(&dll) {
            return v;
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

enum Fetch {
    NotModified,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

fn fetch_feed(url: &str, etag: Option<&str>) -> Result<Fetch> {
    let agent = crate::http::agent(Duration::from_secs(20));
    let mut req = agent.get(url);
    if let Some(etag) = etag {
        req = req.header("If-None-Match", etag);
    }
    let mut resp = req.call()?;
    if resp.status().as_u16() == 304 {
        return Ok(Fetch::NotModified);
    }
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .context("read feed")?;
    Ok(Fetch::Body { bytes, etag })
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let agent = crate::http::agent(Duration::from_secs(30));
    let mut resp = agent.get(url).call()?;
    let mut bytes = Vec::new();
    resp.body_mut().as_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn download_pending(feed: &Feed, bundle: Option<&Path>) -> Result<()> {
    std::fs::create_dir_all(paths::pending_dir())?;
    let part = paths::pending_dir().join("setup.exe.part");
    let agent = crate::http::agent(Duration::from_secs(120));
    let mut resp = agent.get(&feed.url).call().with_context(|| feed.url.clone())?;
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
        bundle_path: bundle
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    };
    deviceout_update::write_manifest(&manifest)?;
    Ok(())
}
