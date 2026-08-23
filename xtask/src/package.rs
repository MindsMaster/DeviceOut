use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::publish;

const ISS_RELATIVE: &str = "packaging/deviceout.iss";
const PACKAGE: &str = "deviceout-plugin";
const BUNDLE_NAME: &str = "DeviceOut.vst3";
const UPDATER: &str = "deviceout-updater";

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
    let upload = args.iter().any(|a| a == "--upload");

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

        println!("  构建 {UPDATER}（release）…");
        let status = Command::new("cargo")
            .args([
                "build",
                "-p",
                UPDATER,
                "--release",
                "--bin",
                UPDATER,
            ])
            .status()
            .context("启动 cargo build updater")?;
        if !status.success() {
            bail!("构建 {UPDATER} 失败");
        }
    }

    let bundle = target_dir().join("bundled").join(BUNDLE_NAME);
    if !bundle.is_dir() {
        bail!(
            "找不到 {}，先跑：cargo xtask bundle {PACKAGE} --release",
            bundle.display()
        );
    }

    let updater_exe = target_dir().join("release").join(format!("{UPDATER}.exe"));
    if !updater_exe.is_file() {
        bail!("找不到 {}，先构建 updater", updater_exe.display());
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

    let setup_name = format!("DeviceOut-Setup-{version}.exe");
    let out = Path::new("packaging").join("out").join(&setup_name);
    println!();
    if !out.is_file() {
        println!("Inno 成功，产物未在预期路径，请查看 packaging\\out\\");
        return Ok(());
    }
    let size = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!("安装包：{}（{} 字节）", out.display(), size);

    let (json_path, sig_path) = write_feed(&out, version)?;
    println!("feed：{}", json_path.display());
    println!("sig：{}", sig_path.display());

    if upload {
        publish::upload(&out, &sig_path, &json_path)?;
        println!("已上传 latest.json -> raw-releases");
    }

    Ok(())
}

fn write_feed(setup: &Path, version: &str) -> Result<(PathBuf, PathBuf)> {
    let sha = deviceout_update::sha256_file(setup).context("sha256 setup")?;
    let file_name = setup
        .file_name()
        .and_then(|n| n.to_str())
        .context("setup filename")?;
    let url = format!(
        "https://repo.azuramc.cc/repository/raw-public/deviceout/{file_name}"
    );
    let feed = serde_json::json!({
        "version": version,
        "sha256": sha,
        "url": url,
    });
    let bytes = serde_json::to_vec_pretty(&feed)?;
    let key = load_sign_key()?;
    let sig = deviceout_update::sign(&key, &bytes);

    let json_path = Path::new("packaging").join("out").join("latest.json");
    let sig_path = Path::new("packaging").join("out").join("latest.json.sig");
    fs::write(&json_path, &bytes)?;
    fs::write(&sig_path, sig)?;
    Ok((json_path, sig_path))
}

fn load_sign_key() -> Result<[u8; 32]> {
    let raw = if let Ok(v) = std::env::var("DEVICEOUT_SIGN_KEY") {
        v
    } else {
        fs::read_to_string("packaging/keys/primary.hex")
            .context("缺少 DEVICEOUT_SIGN_KEY 或 packaging/keys/primary.hex")?
    };
    let bytes = deviceout_update::parse_hex(raw.trim()).context("sign key 不是 hex")?;
    if bytes.len() != 32 {
        bail!("sign key 必须是 32 字节");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}
