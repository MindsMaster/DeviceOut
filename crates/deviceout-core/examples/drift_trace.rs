use deviceout_core::{ring, DriftController, DriftTuning, PullOutcome, RingLimit};

const SAMPLE_RATE: f64 = 48_000.0;
const CHANNELS: usize = 2;
const DAW_BLOCK: usize = 512;
const SINK_BLOCK: usize = 480;
const CAPACITY: usize = 19_200;

fn main() {
    let ppm: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50.0);

    let (mut tx, mut rx) = ring(CAPACITY, CHANNELS);
    let target = (CAPACITY / 2) as f64;
    let mut ctrl = DriftController::new(target, SAMPLE_RATE, 1.0, DriftTuning::default());

    let source_rate = SAMPLE_RATE * (1.0 + ppm * 1.0e-6);
    let sink_period = SINK_BLOCK as f64 / SAMPLE_RATE;

    let prime = vec![0.5f32; (target as usize) * CHANNELS];
    tx.push(&prime);

    let wave: Vec<f32> = (0..DAW_BLOCK * CHANNELS)
        .map(|i| (i as f32 * 0.01).sin())
        .collect();
    let mut out = vec![0.0f32; SINK_BLOCK * CHANNELS * 2];
    let mut daw_credit = 0.0f64;
    let mut frac_remainder = 0.0f64;

    println!(
        "源时钟偏差 {ppm:+.1} ppm    理论稳态修正应为 {:+.1} ppm",
        ppm
    );
    println!(
        "{:>8} {:>10} {:>12} {:>12} {:>10}",
        "时刻", "水位", "平滑水位", "修正(ppm)", "比例"
    );

    let steps = (1800.0 / sink_period) as usize;
    let mut pushed_total = 0u64;
    let mut consumed_total = 0u64;

    for step in 0..steps {
        daw_credit += source_rate * sink_period;
        while daw_credit >= DAW_BLOCK as f64 {
            daw_credit -= DAW_BLOCK as f64;
            tx.push(&wave);
            pushed_total += DAW_BLOCK as u64;
        }

        let ratio = ctrl.ratio();
        frac_remainder += SINK_BLOCK as f64 / ratio;
        let needed_frames = frac_remainder.floor() as usize;
        frac_remainder -= needed_frames as f64;
        let outcome = rx.pull(&mut out[..needed_frames * CHANNELS]);
        consumed_total += needed_frames as u64;

        let fill = rx.available_frames();
        let limit = match outcome {
            PullOutcome::Underrun { .. } => RingLimit::Empty,
            PullOutcome::Ok => RingLimit::Free,
        };
        ctrl.update(fill as f64, sink_period, limit);

        let t = step as f64 * sink_period;
        let hit = if t < 30.0 {
            step % (5.0 / sink_period) as usize == 0
        } else {
            step % (60.0 / sink_period) as usize == 0
        };
        if hit {
            println!(
                "{:>7.0}s {:>10} {:>12.1} {:>12.2} {:>10.7}",
                t,
                fill,
                ctrl.smoothed_fill(),
                ctrl.drift_ppm(),
                ctrl.ratio()
            );
        }
    }

    println!();
    println!("累计推入 {pushed_total} 帧，累计消耗 {consumed_total} 帧");
    println!(
        "实测收支比 {:.9}（理论应为 {:.9}）",
        consumed_total as f64 / pushed_total as f64,
        1.0
    );
    println!(
        "最终修正 {:+.2} ppm，最终水位 {}",
        ctrl.drift_ppm(),
        rx.available_frames()
    );
    println!(
        "断流 {} 次，溢出 {} 次",
        rx.stats().underrun_events(),
        rx.stats().overrun_events()
    );
}
