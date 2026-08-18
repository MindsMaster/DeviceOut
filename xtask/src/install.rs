use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const BUNDLE_NAME: &str = "DeviceOut.vst3";
const BINARY_RELATIVE: &str = "Contents/x86_64-win/DeviceOut.vst3";
const MARKER_RELATIVE: &str = "Contents/Resources/deviceout-install.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    User,
    Machine,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::User => "用户级",
            Self::Machine => "机器级",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Machine => "machine",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::User => Self::Machine,
            Self::Machine => Self::User,
        }
    }

    fn vst3_root(self) -> Result<PathBuf> {
        match self {
            Self::User => {
                let base = std::env::var_os("LOCALAPPDATA")
                    .context("读不到 LOCALAPPDATA，无法确定用户级 VST3 目录")?;
                Ok(PathBuf::from(base)
                    .join("Programs")
                    .join("Common")
                    .join("VST3"))
            }
            Self::Machine => {
                let base = std::env::var_os("CommonProgramFiles")
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("ProgramFiles")
                            .map(|p| PathBuf::from(p).join("Common Files"))
                    })
                    .context("读不到 CommonProgramFiles，无法确定机器级 VST3 目录")?;
                Ok(base.join("VST3"))
            }
        }
    }

    fn bundle_path(self) -> Result<PathBuf> {
        Ok(self.vst3_root()?.join(BUNDLE_NAME))
    }
}

fn installed_version(bundle: &Path) -> Option<String> {
    let text = fs::read_to_string(bundle.join(MARKER_RELATIVE)).ok()?;
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "version")
        .map(|(_, value)| value.trim().to_string())
}

fn write_marker(bundle: &Path, scope: Scope, source: &Path) -> Result<()> {
    let path = bundle.join(MARKER_RELATIVE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let body = format!(
        "product = DeviceOut\nversion = {}\nscope = {}\nsource = {}\n",
        env!("CARGO_PKG_VERSION"),
        scope.key(),
        source.display()
    );
    fs::write(&path, body).with_context(|| format!("写入 {} 失败", path.display()))
}

fn is_locked(binary: &Path) -> bool {
    if !binary.exists() {
        return false;
    }
    fs::OpenOptions::new().write(true).open(binary).is_err()
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("创建 {} 失败", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("读取 {} 失败", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)
                .with_context(|| format!("复制 {} -> {} 失败", from.display(), to.display()))?;
        }
    }
    Ok(())
}

fn explain_permission(e: anyhow::Error, scope: Scope) -> anyhow::Error {
    let denied = e
        .chain()
        .filter_map(|c| c.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == std::io::ErrorKind::PermissionDenied);

    if denied && scope == Scope::Machine {
        e.context(
            "机器级目录需要管理员权限。\n\
             用管理员终端重跑，或改用：cargo xtask install",
        )
    } else {
        e
    }
}

fn default_source() -> PathBuf {
    PathBuf::from("target").join("bundled").join(BUNDLE_NAME)
}

fn parse_scope(args: &[String]) -> Result<Scope> {
    let machine = args.iter().any(|a| a == "--machine");
    let user = args.iter().any(|a| a == "--user");
    if machine && user {
        bail!("--machine 和 --user 只能给一个");
    }
    Ok(if machine { Scope::Machine } else { Scope::User })
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let pos = args.iter().position(|a| a == name)?;
    args.get(pos + 1).cloned()
}

fn warn_about_duplicate(scope: Scope) {
    let other = scope.other();
    let Ok(path) = other.bundle_path() else {
        return;
    };
    if !path.exists() {
        return;
    }

    let flag = if other == Scope::Machine {
        "--machine"
    } else {
        "--user"
    };
    eprintln!();
    eprintln!("警告：{}也已安装：{}", other.label(), path.display());
    eprintln!("      类 ID 相同，继续可能看不到本次更新。");
    eprintln!("      删除：cargo xtask uninstall {flag}");
}

pub fn install(args: &[String]) -> Result<()> {
    nih_plug_xtask::chdir_workspace_root()?;

    let scope = parse_scope(args)?;
    let source = flag_value(args, "--source")
        .map(PathBuf::from)
        .unwrap_or_else(default_source);

    println!(
        "DeviceOut {} - 安装（{}）",
        env!("CARGO_PKG_VERSION"),
        scope.label()
    );

    if !source.exists() {
        bail!(
            "找不到插件包：{}\n先构建：cargo xtask bundle deviceout-plugin --release",
            source.display()
        );
    }
    let source_binary = source.join(BINARY_RELATIVE);
    if !source_binary.exists() {
        bail!(
            "{} 不是有效的 VST3 包，缺少 {}",
            source.display(),
            BINARY_RELATIVE
        );
    }

    let target = scope.bundle_path()?;

    match installed_version(&target) {
        Some(old) => println!("  已装 {old} -> {}", env!("CARGO_PKG_VERSION")),
        None if target.exists() => println!("  该位置已有一份，将被覆盖"),
        None => println!("  全新安装"),
    }

    let target_binary = target.join(BINARY_RELATIVE);
    if is_locked(&target_binary) {
        bail!(
            "插件正被宿主占用，无法覆盖：{}\n请关闭 DAW / OBS 后重试。",
            target_binary.display()
        );
    }

    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("删除旧版本 {} 失败", target.display()))
            .map_err(|e| explain_permission(e, scope))?;
    }
    copy_dir_all(&source, &target).map_err(|e| explain_permission(e, scope))?;
    write_marker(&target, scope, &source)?;

    println!("  已装到 {}", target.display());
    warn_about_duplicate(scope);

    println!();
    println!("重启 DAW -> 总线挂 DeviceOut -> 选输出设备 -> 语音软件麦克风选对应 CABLE Output");

    Ok(())
}

pub fn uninstall(args: &[String]) -> Result<()> {
    nih_plug_xtask::chdir_workspace_root()?;

    let all = args.iter().any(|a| a == "--all");
    let scopes: Vec<Scope> = if all {
        vec![Scope::User, Scope::Machine]
    } else {
        vec![parse_scope(args)?]
    };

    println!("DeviceOut - 卸载");

    let mut removed = 0usize;
    let mut blocked = Vec::new();

    for scope in scopes {
        let bundle = scope.bundle_path()?;
        if !bundle.exists() {
            println!("  {}：未安装", scope.label());
            continue;
        }

        let version = installed_version(&bundle).unwrap_or_else(|| "未知版本".to_string());

        if is_locked(&bundle.join(BINARY_RELATIVE)) {
            println!("  {}：跳过，插件正被宿主占用", scope.label());
            blocked.push(bundle);
            continue;
        }

        match fs::remove_dir_all(&bundle)
            .with_context(|| format!("删除 {} 失败", bundle.display()))
            .map_err(|e| explain_permission(e, scope))
        {
            Ok(()) => {
                println!(
                    "  {}：已删除 {}（{version}）",
                    scope.label(),
                    bundle.display()
                );
                removed += 1;
            }
            Err(e) if all => {
                println!("  {}：{e:#}", scope.label());
                blocked.push(bundle);
            }
            Err(e) => return Err(e),
        }
    }

    println!();
    if blocked.is_empty() {
        if removed > 0 {
            println!("卸载完成。");
        }
    } else {
        println!("未删除：");
        for path in &blocked {
            println!("  {}", path.display());
        }
        println!("关闭宿主后重试（机器级需管理员）");
    }

    Ok(())
}
