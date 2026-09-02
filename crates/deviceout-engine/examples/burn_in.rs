use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use deviceout_core::ring;
use deviceout_engine::{ring_capacity_frames, start, EngineConfig, EngineState};
use deviceout_sink::wasapi::{find_device_by_name, ComGuard};

const DAW_BLOCK: usize = 512;
const TONE_HZ: f64 = 220.0;
const TONE_AMP: f64 = 0.1;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_REPORT_SECS: f64 = 15.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let needle = args.next().unwrap_or_else(|| "CABLE Input".to_string());
    let minutes: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let report_interval = Duration::from_secs_f64(
        args.next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_REPORT_SECS),
    );

    let _com = ComGuard::new().expect("COM 初始化失败");

    let device = match find_device_by_name(&needle) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            eprintln!("先运行 `cargo run -p deviceout-sink --example list_devices`");
            std::process::exit(1);
        }
    };

    println!("目标设备: {device}");
    let channels = device.mix_format.channels as usize;
    let source_rate = f64::from(device.mix_format.sample_rate);

    let config = EngineConfig {
        device_id: device.id.clone(),
        source_rate_hz: source_rate,
        channels,
        ..Default::default()
    };

    let capacity = ring_capacity_frames(source_rate, 400.0);
    let (mut producer, consumer) = ring(capacity, channels);
    let handle = start(consumer, config);

    let metrics = Arc::clone(handle.metrics());
    if let Some(reason) = metrics.last_error() {
        eprintln!("启动失败: {reason}");
        std::process::exit(1);
    }

    println!(
        "已开流: {} Hz / {} 声道    设备周期 {} 帧    环形缓冲 {} 帧（目标水位 {:.0}）",
        metrics.sink_rate_hz(),
        metrics.channels(),
        metrics.period_frames(),
        metrics.capacity_frames(),
        metrics.target_frames(),
    );
    println!("DAW 侧模拟: 每 {DAW_BLOCK} 帧推一次，速率由高精度计数器定");
    println!();

    let stop_producer = Arc::new(AtomicBool::new(false));
    let producer_thread = {
        let stop = Arc::clone(&stop_producer);
        thread::spawn(move || {
            let mut block = vec![0.0f32; DAW_BLOCK * channels];
            let step = std::f64::consts::TAU * TONE_HZ / source_rate;
            let mut phase = 0.0f64;
            let t0 = Instant::now();
            let mut pushed = 0u64;

            while !stop.load(Ordering::Relaxed) {
                let due = (t0.elapsed().as_secs_f64() * source_rate) as u64;
                while pushed + DAW_BLOCK as u64 <= due {
                    for frame in block.chunks_exact_mut(channels) {
                        let s = (phase.sin() * TONE_AMP) as f32;
                        phase += step;
                        frame.fill(s);
                    }
                    producer.push(&block);
                    pushed += DAW_BLOCK as u64;
                }
                thread::sleep(Duration::from_millis(1));
            }
        })
    };

    let total = Duration::from_secs_f64(minutes * 60.0);
    let t0 = Instant::now();
    let mut next_report = t0 + report_interval;

    let warmup = Duration::from_secs(5);
    let mut lowest_fill = u64::MAX;
    let mut highest_fill = 0u64;

    let settled_after = total.mul_f64(2.0 / 3.0);
    let mut ppm_lo = f64::MAX;
    let mut ppm_hi = f64::MIN;
    let mut failed = false;

    let mut window_lo = u64::MAX;
    let mut window_hi = 0u64;

    println!(
        "{:>7}  {:>6}  {:>7}  {:>6}  {:>9}  {:>11}  {:>5}  {:>5}  {:>6}",
        "时间", "状态", "水位", "占比", "ppm", "本窗纹波", "欠载", "溢出", "限幅"
    );
    println!("（ppm 带 * 表示尚未收敛）");

    while t0.elapsed() < total {
        thread::sleep(SAMPLE_INTERVAL);
        let elapsed = t0.elapsed();

        let state = metrics.state();
        if state == EngineState::Failed {
            println!();
            eprintln!(
                "链路在 {:.0} 秒处出错: {}",
                elapsed.as_secs_f64(),
                metrics
                    .last_error()
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "原因未记录".into())
            );
            failed = true;
            break;
        }

        if state == EngineState::Running {
            let fill = metrics.fill_frames();
            window_lo = window_lo.min(fill);
            window_hi = window_hi.max(fill);
        }

        if state == EngineState::Running && elapsed > warmup {
            let fill = metrics.fill_frames();
            lowest_fill = lowest_fill.min(fill);
            highest_fill = highest_fill.max(fill);

            if elapsed >= settled_after {
                if let Some(ppm) = metrics.drift_ppm_settled() {
                    ppm_lo = ppm_lo.min(ppm);
                    ppm_hi = ppm_hi.max(ppm);
                }
            }
        }

        if Instant::now() >= next_report {
            next_report += report_interval;
            let stats = metrics.stats();
            let target = metrics.target_frames() as i64;
            let ripple = if window_hi > window_lo {
                format!(
                    "{:+}/{:+}",
                    window_lo as i64 - target,
                    window_hi as i64 - target
                )
            } else {
                "-".to_string()
            };
            let ppm = format!(
                "{:+.2}{}",
                metrics.drift_ppm(),
                if metrics.is_settled() { "" } else { "*" }
            );
            println!(
                "{:>6.0}s  {:>6}  {:>7.0}  {:>5.1}%  {:>9}  {:>11}  {:>5}  {:>5}  {:>6}",
                elapsed.as_secs_f64(),
                format!("{state:?}"),
                metrics.smoothed_fill(),
                metrics.fill_fraction() * 100.0,
                ppm,
                ripple,
                stats.underrun_events(),
                stats.overrun_events(),
                metrics.clamp_events(),
            );
            window_lo = u64::MAX;
            window_hi = 0;
        }
    }

    stop_producer.store(true, Ordering::Relaxed);
    let _ = producer_thread.join();
    drop(handle);

    let stats = metrics.stats();
    let capacity = metrics.capacity_frames();
    let ppm_spread = if ppm_hi > ppm_lo {
        ppm_hi - ppm_lo
    } else {
        0.0
    };

    println!();
    println!(
        "=== 烧机报告（{:.1} 分钟）===",
        t0.elapsed().as_secs_f64() / 60.0
    );
    println!("推入          {} 帧", stats.frames_pushed());
    println!("取出          {} 帧", stats.frames_pulled());
    println!(
        "欠载          {} 次，累计补零 {:.3} 秒",
        stats.underrun_events(),
        metrics.dropout_seconds()
    );
    println!("溢出          {} 次", stats.overrun_events());
    println!("比例限幅      {} 次", metrics.clamp_events());
    println!(
        "水位区间      {}~{} 帧（容量 {}，即 {:.1}%~{:.1}%）",
        lowest_fill,
        highest_fill,
        capacity,
        lowest_fill as f64 / capacity as f64 * 100.0,
        highest_fill as f64 / capacity as f64 * 100.0,
    );
    match metrics.drift_ppm_settled() {
        Some(ppm) => println!("时钟偏差      {ppm:+.2} ppm（已收敛）"),
        None => println!(
            "时钟偏差      {:+.2} ppm（未收敛，只跑了 {:.0} 秒）",
            metrics.drift_ppm(),
            metrics.running_seconds()
        ),
    }
    println!("后段 ppm 极差 {ppm_spread:.2}");
    println!("重采样比例    {:.9}（标称 1.0）", metrics.ratio());

    println!();
    let mut verdict_ok = !failed;
    let mut check = |ok: bool, label: &str| {
        println!("{} {label}", if ok { "[通过]" } else { "[失败]" });
        verdict_ok &= ok;
    };

    check(stats.is_clean(), "零欠载零溢出");
    check(metrics.clamp_events() == 0, "重采样比例从未顶到限幅");
    check(
        lowest_fill > capacity / 10 && highest_fill < capacity * 9 / 10,
        "水位始终在容量的 10%~90% 之间",
    );

    if minutes >= 10.0 {
        check(ppm_spread < 5.0, "收敛后 ppm 波动小于 5");
    } else {
        println!("[跳过] 收敛后 ppm 波动 -- 至少要跑 10 分钟，本次只有 {minutes:.0} 分钟");
    }

    println!();
    if verdict_ok {
        println!("烧机通过。");
    } else {
        println!("烧机未通过。");
        std::process::exit(1);
    }
}
