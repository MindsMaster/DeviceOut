use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::IMMDevice;
use windows::Win32::System::Com::CLSCTX_ALL;

use deviceout_sink::wasapi::{find_device_by_id, list_devices, ComGuard, Direction};

fn main() {
    let mut args = std::env::args().skip(1);
    let needle = args.next().unwrap_or_else(|| "CABLE".to_string());
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);

    let _com = ComGuard::new().expect("COM 初始化失败");

    let mut targets: Vec<(String, String, IMMDevice)> = Vec::new();
    for dir in [Direction::Output, Direction::Input] {
        let label = if dir == Direction::Output {
            "播放"
        } else {
            "录制"
        };
        for info in list_devices(dir).expect("枚举失败") {
            if info.name.contains(&needle) {
                match find_device_by_id(&info.id) {
                    Ok(dev) => targets.push((label.to_string(), info.name.clone(), dev)),
                    Err(e) => eprintln!("跳过 {}: {e}", info.name),
                }
            }
        }
    }

    if targets.is_empty() {
        eprintln!("没有名字包含「{needle}」的设备");
        std::process::exit(1);
    }

    let meters: Vec<(String, String, IAudioMeterInformation)> = targets
        .into_iter()
        .filter_map(|(dir, name, dev)| {
            let m: Result<IAudioMeterInformation, _> = unsafe { dev.Activate(CLSCTX_ALL, None) };
            match m {
                Ok(m) => Some((dir, name, m)),
                Err(e) => {
                    eprintln!("跳过 {name}: 无法读取计量器 {e}");
                    None
                }
            }
        })
        .collect();

    println!("监视 {} 个端点，{seconds} 秒：\n", meters.len());
    for (dir, name, _) in &meters {
        println!("  [{dir}] {name}");
    }
    println!();

    let started = Instant::now();
    let mut peaks = vec![0.0f32; meters.len()];

    while started.elapsed().as_secs() < seconds {
        let mut line = String::new();
        for (i, (dir, name, m)) in meters.iter().enumerate() {
            let v = unsafe { m.GetPeakValue() }.unwrap_or(0.0);
            peaks[i] = peaks[i].max(v);
            let db = if v > 0.0 {
                format!("{:>7.1} dB", 20.0 * v.log10())
            } else {
                "   -inf dB".to_string()
            };
            let short: String = name.chars().take(14).collect();
            line.push_str(&format!("[{dir}]{short:<15}{db}   "));
        }
        print!("\r{line}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_millis(200));
    }

    println!("\n");
    println!("本次峰值：");
    for (i, (dir, name, _)) in meters.iter().enumerate() {
        let db = if peaks[i] > 0.0 {
            format!("{:.1} dB", 20.0 * peaks[i].log10())
        } else {
            "-inf dB（全程静音）".to_string()
        };
        println!("  [{dir}] {name}: {db}");
    }
}
