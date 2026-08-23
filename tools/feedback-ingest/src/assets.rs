use std::path::PathBuf;
use std::sync::OnceLock;

const EMBED_CSS: &str = include_str!("../static/dash.css");
const EMBED_JS: &str = include_str!("../static/dash.js");

static CSS: OnceLock<String> = OnceLock::new();
static JS: OnceLock<String> = OnceLock::new();

pub fn warmup() {
    let _ = css();
    let _ = js();
}

pub fn css() -> &'static str {
    CSS.get_or_init(|| load("dash.css", EMBED_CSS))
}

pub fn js() -> &'static str {
    JS.get_or_init(|| load("dash.js", EMBED_JS))
}

fn load(name: &str, embedded: &str) -> String {
    for dir in candidate_dirs() {
        let path = dir.join("static").join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if !text.trim().is_empty() {
                eprintln!("loaded asset {}", path.display());
                return text;
            }
        }
    }
    embedded.to_string()
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    dirs
}
