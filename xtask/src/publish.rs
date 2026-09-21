use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

const HOST: &str = "https://repo.azuramc.cc/repository";

pub fn upload(setup: &Path, sig: &Path, json: &Path) -> Result<()> {
    let repo = "raw-releases";
    let (user, pass) = creds()?;
    let setup_name = setup
        .file_name()
        .and_then(|n| n.to_str())
        .context("setup name")?;
    let json_name = json
        .file_name()
        .and_then(|n| n.to_str())
        .context("json name")?;
    let sig_name = sig
        .file_name()
        .and_then(|n| n.to_str())
        .context("sig name")?;

    put(
        repo,
        setup_name,
        setup,
        "application/octet-stream",
        &user,
        &pass,
    )?;
    put(
        repo,
        sig_name,
        sig,
        "application/octet-stream",
        &user,
        &pass,
    )?;
    put(repo, json_name, json, "application/json", &user, &pass)?;
    Ok(())
}

fn put(repo: &str, name: &str, file: &Path, ct: &str, user: &str, pass: &str) -> Result<()> {
    let url = format!("{HOST}/{repo}/deviceout/{name}");
    let bytes = fs::read(file).with_context(|| format!("读 {}", file.display()))?;
    let resp = ureq::put(&url)
        .header("Content-Type", ct)
        .header("Authorization", basic_auth(user, pass))
        .send(&bytes);
    match resp {
        Ok(r) => {
            println!("  PUT {} {}", r.status(), url);
            Ok(())
        }
        Err(ureq::Error::StatusCode(code)) => bail!("PUT {code} {url}"),
        Err(e) => Err(e.into()),
    }
}

fn basic_auth(user: &str, pass: &str) -> String {
    format!("Basic {}", b64(&format!("{user}:{pass}")))
}

fn b64(input: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(b2 & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn creds() -> Result<(String, String)> {
    if let (Ok(u), Ok(p)) = (
        std::env::var("AZURA_REPO_USERNAME"),
        std::env::var("AZURA_REPO_PASSWORD"),
    ) {
        if !u.is_empty() && !p.is_empty() {
            return Ok((u, p));
        }
    }
    let gradle_home = std::env::var_os("GRADLE_USER_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs_home)
        .map(|h| h.join("gradle.properties"));
    if let Some(path) = gradle_home {
        if let Ok(text) = fs::read_to_string(&path) {
            let mut user = None;
            let mut pass = None;
            for line in text.lines() {
                let t = line.trim();
                if t.starts_with('#') || t.is_empty() {
                    continue;
                }
                if let Some((k, v)) = t.split_once('=') {
                    match k.trim() {
                        "azuraRepoUsername" => user = Some(v.trim().to_string()),
                        "azuraRepoPassword" => pass = Some(v.trim().to_string()),
                        _ => {}
                    }
                }
            }
            if let (Some(u), Some(p)) = (user, pass) {
                return Ok((u, p));
            }
        }
    }
    bail!(
        "缺少仓库凭证。设置 AZURA_REPO_USERNAME / AZURA_REPO_PASSWORD，\
         或在 GRADLE_USER_HOME/gradle.properties 写入 azuraRepoUsername / azuraRepoPassword"
    )
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".gradle"))
}
