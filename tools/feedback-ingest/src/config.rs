use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Hit {
    pub at: Instant,
}

pub struct Limits {
    pub by_ip: HashMap<String, Vec<Hit>>,
    pub by_id: HashMap<String, Vec<Hit>>,
    pub admin_fail_ip: HashMap<String, Vec<Hit>>,
    pub ping_by_ip: HashMap<String, Vec<Hit>>,
    pub last_maintenance: Instant,
}

pub struct Config {
    pub token: Option<String>,
    pub admin_password: Option<String>,
    pub admin_path: String,
    pub dir: PathBuf,
    pub kill: bool,
    pub rate_ip_hour: u32,
    pub rate_id_day: u32,
    pub admin_fail_hour: u32,
    pub ping_rate_ip_hour: u32,
    pub max_files: u32,
}

pub fn load_dotenv() {
    let mut files = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        files.push(cwd.join(".env"));
        files.push(cwd.join("env"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            files.push(dir.join(".env"));
            files.push(dir.join("env"));
        }
    }
    let mut seen = HashSet::new();
    for path in files {
        if !seen.insert(path.clone()) {
            continue;
        }
        apply_env_file(&path);
    }
}

fn apply_env_file(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    eprintln!("loaded env {}", path.display());
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        if key.is_empty() || std::env::var_os(key).is_some() {
            continue;
        }
        let mut val = v.trim().to_string();
        if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            val = val[1..val.len() - 1].to_string();
        }
        unsafe { std::env::set_var(key, val) };
    }
}

pub fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

pub fn load_or_create_admin_path(dir: &Path) -> String {
    if let Some(v) = env_nonempty("FEEDBACK_ADMIN_PATH") {
        if valid_admin_path(&v) {
            return v;
        }
        eprintln!("warning: FEEDBACK_ADMIN_PATH ignored (use 16-64 ascii letters/digits/hyphens)");
    }
    let file = dir.join(".admin-path");
    if let Ok(text) = std::fs::read_to_string(&file) {
        let v = text.trim();
        if valid_admin_path(v) {
            return v.to_string();
        }
    }
    let path = random_path();
    let _ = std::fs::write(&file, format!("{path}\n"));
    path
}

fn valid_admin_path(s: &str) -> bool {
    (16..=64).contains(&s.len())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && s != "admin"
}

fn random_path() -> String {
    let mut out = String::with_capacity(32);
    for _ in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(0);
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_path_is_valid_and_unpredictable() {
        let a = random_path();
        let b = random_path();
        assert!(valid_admin_path(&a), "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn env_file_quotes_are_stripped_safely() {
        let dir = std::env::temp_dir().join(format!("fb-env-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("env");
        std::fs::write(
            &file,
            "# c\nexport FB_TEST_A=\"quoted value\"\nFB_TEST_B='x'\nFB_TEST_C=\"\nFB_TEST_D=plain\n",
        )
        .unwrap();
        apply_env_file(&file);
        assert_eq!(std::env::var("FB_TEST_A").unwrap(), "quoted value");
        assert_eq!(std::env::var("FB_TEST_B").unwrap(), "x");
        assert_eq!(std::env::var("FB_TEST_C").unwrap(), "\"");
        assert_eq!(std::env::var("FB_TEST_D").unwrap(), "plain");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
