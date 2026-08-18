use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

const ISS_RELATIVE: &str = "packaging/deviceout.iss";
const PACKAGE: &str = "deviceout-plugin";
const BUNDLE_NAME: &str = "DeviceOut.vst3";

fn find_iscc() -> Option<PathBuf> {
    if let Ok(output) = Command::new("ISCC.exe").arg("/?").output() {
        if output.status.success() || !output.stdout.is_empty() {
            return Some(PathBuf::from("ISCC.exe"));
        }
    }

    let roots = [
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs")),
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        std::env::var_os("ProgramFiles").map(PathBuf::from),
    ];

    for root in roots.into_iter().flatten() {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with("Inno Setup")
            {
                continue;
            }
            let candidate = entry.path().join("ISCC.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
}

pub fn package(args: &[String]) -> Result<()> {
    nih_plug_xtask::chdir_workspace_root()?;

    let version = env!("CARGO_PKG_VERSION");
    let skip_build = args.iter().any(|a| a == "--no-build");

    println!("DeviceOut {version} - 安装包");

    let iss = Path::new(ISS_RELATIVE);
    if !iss.is_file() {
        bail!("找不到 Inno 脚本：{}", iss.display());
    }

    if skip_build {
        println!("  跳过构建（--no-build）");
    } else {
        println!("  构建 {PACKAGE}（release）…");
        let build_args = vec!["--release".to_string()];
        let packages = vec![PACKAGE.to_string()];
        nih_plug_xtask::build(&packages, &build_args).context("构建失败")?;
        nih_plug_xtask::bundle(&target_dir(), PACKAGE, &build_args, false)
            .context("打包 .vst3 失败")?;
    }

    let bundle = target_dir().join("bundled").join(BUNDLE_NAME);
    if !bundle.is_dir() {
        bail!(
            "找不到 {}，先跑：cargo xtask bundle {PACKAGE} --release",
            bundle.display()
        );
    }

    let Some(iscc) = find_iscc() else {
        bail!(
            "找不到 ISCC.exe。安装：winget install JRSoftware.InnoSetup\n\
             或 https://jrsoftware.org/isdl.php（需 6.3+）"
        );
    };
    println!("  Inno Setup: {}", iscc.display());

    let status = Command::new(&iscc)
        .arg(format!("/DMyAppVersion={version}"))
        .arg(iss)
        .status()
        .with_context(|| format!("启动 {} 失败", iscc.display()))?;

    if !status.success() {
        bail!("Inno Setup 编译失败（退出码 {:?}）", status.code());
    }

    let out = Path::new("packaging")
        .join("out")
        .join(format!("DeviceOut-{version}-setup.exe"));
    println!();
    if out.is_file() {
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        println!("安装包：{}（{} 字节）", out.display(), size);
    } else {
        println!("Inno 成功，产物未在预期路径，请查看 packaging\\out\\");
    }

    Ok(())
}
