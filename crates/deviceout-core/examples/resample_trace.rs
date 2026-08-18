use deviceout_core::{ring, DriftController, DriftResampler, DriftTuning};

const SAMPLE_RATE: f64 = 48_000.0;
const CHANNELS: usize = 2;
const DAW_BLOCK: usize = 512;
const SINK_BLOCK: usize = 480;
const CAPACITY: usize = 19_200;

const WINDOW_SECONDS: f64 = 120.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let ppm: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(50.0);
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1800.0);
    let tau: Option<f64> = args.next().and_then(|s| s.parse().ok());

    let (mut tx, mut rx) = ring(CAPACITY, CHANNELS);
    let target = (CAPACITY / 2) as f64;
    let tuning = DriftTuning {
        lowpass_tau_s: tau.unwrap_or(DriftTuning::default().lowpass_tau_s),
        ..DriftTuning::default()
    };
    let mut ctrl = DriftController::new(target, SAMPLE_RATE, 1.0, tuning);
    let mut resampler = DriftResampler::new(CHANNELS, SINK_BLOCK, 1.0, &tuning).unwrap();

    let mut pull_buf = vec![0.0f32; resampler.input_frames_max() * CHANNELS];
    let mut out_buf = vec![0.0f32; resampler.output_frames() * CHANNELS];
    let wave: Vec<f32> = (0..DAW_BLOCK * CHANNELS)
        .map(|i| (i as f32 * 0.01).sin() * 0.5)
        .collect();

    let prime = vec![0.0f32; (target as usize) * CHANNELS];
    tx.push(&prime);

    let source_rate = SAMPLE_RATE * (1.0 + ppm * 1.0e-6);
    let sink_period = SINK_BLOCK as f64 / SAMPLE_RATE;
    let steps = (seconds / sink_period) as usize;
    let window_steps = (WINDOW_SECONDS / sink_period) as usize;

    let wn = 4.0 / (tuning.damping * tuning.settle_time_s);
    println!("源时钟偏差 {ppm:+.1} ppm 模拟 {seconds:.0} 秒");
    println!(
        "低通 tau={:.2}s（截止 {:.3} rad/s），控制带宽 wn={:.3} rad/s，比值 {:.1}（需 >= 5）",
        tuning.lowpass_tau_s,
        1.0 / tuning.lowpass_tau_s,
        wn,
        (1.0 / tuning.lowpass_tau_s) / wn
    );
    println!(
        "滤波器延迟 {} 帧，输入帧上界 {} 帧",
        resampler.output_delay(),
        resampler.input_frames_max()
    );
    println!();
    println!(
        "{:>8} {:>8} {:>10} {:>12} {:>12} {:>12}",
        "时刻", "水位", "平滑水位", "漂移(ppm)", "总修正(ppm)", "输入(帧)"
    );

    let mut daw_credit = 0.0f64;
    let mut window_lo = f64::MAX;
    let mut window_hi = f64::MIN;
    let mut spreads: Vec<(f64, f64)> = Vec::new();

    for step in 0..steps {
        daw_credit += source_rate * sink_period;
        while daw_credit >= DAW_BLOCK as f64 {
            daw_credit -= DAW_BLOCK as f64;
            tx.push(&wave);
        }

        let need = resampler.input_frames_next();
        let samples = need * CHANNELS;
        rx.pull(&mut pull_buf[..samples]);
        resampler
            .process(&pull_buf[..samples], &mut out_buf)
            .expect("重采样失败");

        let fill = rx.available_frames();
        ctrl.update(fill as f64, sink_period);
        resampler.set_ratio(ctrl.ratio());

        let drift = ctrl.drift_ppm();
        window_lo = window_lo.min(drift);
        window_hi = window_hi.max(drift);

        let t = step as f64 * sink_period;
        let hit = if t < 30.0 {
            step % (5.0 / sink_period) as usize == 0
        } else {
            step % (60.0 / sink_period) as usize == 0
        };
        if hit {
            println!(
                "{:>7.0}s {:>8} {:>10.1} {:>12.2} {:>12.2} {:>12}",
                t,
                fill,
                ctrl.smoothed_fill(),
                drift,
                ctrl.correction_ppm(),
                need
            );
        }

        if step > 0 && step % window_steps == 0 {
            spreads.push((t, window_hi - window_lo));
            window_lo = f64::MAX;
            window_hi = f64::MIN;
        }
    }

    println!();
    println!("每 {WINDOW_SECONDS:.0} 秒窗口内 ppm 估计的极差：");
    for (end_t, spread) in &spreads {
        let bar = "#".repeat((spread * 4.0).round().min(60.0) as usize);
        println!("  截至 {:>6.0}s  {:>8.3} ppm  {}", end_t, spread, bar);
    }

    println!();
    println!(
        "最终漂移估计 {:+.2} ppm（真值 {ppm:+.1}），最终水位 {} / {CAPACITY} 帧",
        ctrl.drift_ppm(),
        rx.available_frames()
    );
    println!(
        "断流 {} 次，溢出 {} 次，比例限幅 {} 次",
        rx.stats().underrun_events(),
        rx.stats().overrun_events(),
        resampler.clamp_events()
    );
}
