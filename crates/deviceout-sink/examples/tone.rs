use std::time::Instant;

use deviceout_sink::wasapi::{find_device_by_name, ComGuard};
use deviceout_sink::{AudioSink, WasapiSink};

const FREQ: f64 = 440.0;
const CHUNK_FRAMES: usize = 480;

fn main() {
    let mut args = std::env::args().skip(1);
    let needle = args.next().unwrap_or_else(|| "CABLE Input".to_string());
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let amp: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.25);

    let _com = ComGuard::new().expect("COM 初始化失败");

    let info = match find_device_by_name(&needle) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            eprintln!("可用设备见：cargo run -p deviceout-sink --example list_devices");
            std::process::exit(1);
        }
    };
    println!("目标设备: {info}");

    let mut sink = WasapiSink::open(&info.id, deviceout_sink::SinkOptions::default()).expect("打开输出流失败");
    let fmt = sink.format();
    let channels = fmt.channels as usize;
    println!(
        "设备已打开: {fmt}    缓冲 {} 帧    周期 {} 帧",
        sink.buffer_frames(),
        sink.period_frames()
    );

    sink.start().expect("启动失败");

    let step = std::f64::consts::TAU * FREQ / f64::from(fmt.sample_rate);
    let total_frames = (seconds * f64::from(fmt.sample_rate)) as usize;
    let mut phase = 0.0f64;
    let mut buf = vec![0.0f32; CHUNK_FRAMES * channels];

    println!(
        "播放 {seconds} 秒 {FREQ} Hz，幅度 {amp}（{:.1} dBFS）",
        20.0 * amp.log10()
    );
    let mut written = 0usize;

    let warmup_frames = fmt.sample_rate as usize;
    let mut mark: Option<(Instant, usize)> = None;
    let mut measured: Option<(f64, usize)> = None;

    while written < total_frames {
        let n = CHUNK_FRAMES.min(total_frames - written);
        for f in 0..n {
            let s = (phase.sin() * amp) as f32;
            phase += step;
            for c in 0..channels {
                buf[f * channels + c] = s;
            }
        }
        sink.write(&buf[..n * channels]).expect("写入失败");
        written += n;

        if mark.is_none() && written >= warmup_frames {
            mark = Some((Instant::now(), written));
        }
    }
    if let Some((t0, n0)) = mark {
        measured = Some((t0.elapsed().as_secs_f64(), written - n0));
    }

    sink.stop().expect("停止失败");

    println!();
    match measured {
        Some((dt, frames)) if dt > 0.5 => {
            let rate = frames as f64 / dt;
            println!("稳态: {frames} 帧 / {dt:.3} 秒");
            println!("设备速率 {rate:.2} Hz（标称 {}）", fmt.sample_rate);
            println!(
                "相对系统时钟: {:+.1} ppm",
                (rate / f64::from(fmt.sample_rate) - 1.0) * 1.0e6
            );
            if dt < 20.0 {
                println!("窗口 {dt:.1} s，ppm 仅供参考；测时钟至少 30 秒。");
            }
        }
        _ => println!("播放过短，无法测稳态速率。"),
    }
}
