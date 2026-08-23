mod install;
mod package;
mod publish;

fn main() -> nih_plug_xtask::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("install") => install::install(&args[1..]),
        Some("uninstall") => install::uninstall(&args[1..]),
        Some("package") => package::package(&args[1..]),
        Some("help" | "--help" | "-h") => {
            print_usage();
            Ok(())
        }
        _ => nih_plug_xtask::main(),
    }
}

fn print_usage() {
    println!(
        "\
用法：cargo xtask <命令> [参数]

构建
  bundle deviceout-plugin --release   生成 target/bundled/DeviceOut.vst3

安装
  install                    当前用户（无需提权）
  install --machine          所有用户（需管理员）
  install --source <包路径>  指定 .vst3 包

发布
  package                    构建并编出 setup.exe + latest.json/.sig
  package --no-build         用现有产物
  package --upload           按序 PUT 到 Nexus raw-releases

卸载
  uninstall                  用户级
  uninstall --machine        机器级
  uninstall --all            两处都卸

日常：
  cargo xtask bundle deviceout-plugin --release
  cargo xtask install"
    );
}
