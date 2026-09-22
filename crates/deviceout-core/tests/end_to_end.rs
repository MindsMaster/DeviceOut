use deviceout_core::{ring, DriftController, DriftTuning, PullOutcome, PushOutcome, RingLimit};

const SAMPLE_RATE: f64 = 48_000.0;
const CHANNELS: usize = 2;
const DAW_BLOCK: usize = 512;
const SINK_BLOCK: usize = 480;
const CAPACITY: usize = 19_200;

fn run_link(source_ppm: f64, minutes: f64, compensate: bool) -> LinkReport {
    let (mut tx, mut rx) = ring(CAPACITY, CHANNELS);
    let target = (CAPACITY / 2) as f64;
    let mut ctrl = DriftController::new(target, SAMPLE_RATE, 1.0, DriftTuning::default());

    let source_rate = SAMPLE_RATE * (1.0 + source_ppm * 1.0e-6);
    let sink_period = SINK_BLOCK as f64 / SAMPLE_RATE;
    let total_secs = minutes * 60.0;

    let prime = vec![0.5f32; (target as usize) * CHANNELS];
    assert_eq!(tx.push(&prime), PushOutcome::Ok, "预填不应失败");

    let mut daw_credit = 0.0f64;
    let mut worst_fill = usize::MAX;
    let mut best_fill = 0usize;
    let mut ppm_lo = f64::MAX;
    let mut ppm_hi = f64::MIN;
    let mut ppm_sum = 0.0f64;
    let mut ppm_samples = 0u64;
    let mut ratio_lo = f64::MAX;
    let mut ratio_hi = f64::MIN;

    let wave: Vec<f32> = (0..DAW_BLOCK * CHANNELS)
        .map(|i| (i as f32 * 0.01).sin())
        .collect();
    let mut out = vec![0.0f32; SINK_BLOCK * CHANNELS * 2];

    let mut frac_remainder = 0.0f64;

    let steps = (total_secs / sink_period) as usize;
    for step in 0..steps {
        daw_credit += source_rate * sink_period;
        while daw_credit >= DAW_BLOCK as f64 {
            daw_credit -= DAW_BLOCK as f64;
            tx.push(&wave);
        }

        let ratio = if compensate { ctrl.ratio() } else { 1.0 };
        frac_remainder += SINK_BLOCK as f64 / ratio;
        let needed_frames = frac_remainder.floor() as usize;
        frac_remainder -= needed_frames as f64;

        let needed = needed_frames * CHANNELS;
        let outcome = rx.pull(&mut out[..needed]);

        let fill = rx.available_frames();
        worst_fill = worst_fill.min(fill);
        best_fill = best_fill.max(fill);

        if compensate {
            let limit = match outcome {
                PullOutcome::Underrun { .. } => RingLimit::Empty,
                PullOutcome::Ok => RingLimit::Free,
            };
            ctrl.update(fill as f64, sink_period, limit);
        }

        if step * 2 > steps {
            let p = ctrl.drift_ppm();
            ppm_lo = ppm_lo.min(p);
            ppm_hi = ppm_hi.max(p);
            ppm_sum += p;
            ppm_samples += 1;
            ratio_lo = ratio_lo.min(ctrl.ratio());
            ratio_hi = ratio_hi.max(ctrl.ratio());
        }
    }

    LinkReport {
        underruns: rx.stats().underrun_events(),
        overruns: rx.stats().overrun_events(),
        zero_filled: rx.stats().samples_zero_filled(),
        settled_ppm: ppm_sum / ppm_samples.max(1) as f64,
        ppm_spread: ppm_hi - ppm_lo,
        ratio_swing_ppm: (ratio_hi - ratio_lo) * 1.0e6,
        worst_fill,
        best_fill,
    }
}

#[derive(Debug)]
struct LinkReport {
    underruns: u64,
    overruns: u64,
    zero_filled: u64,
    settled_ppm: f64,
    ppm_spread: f64,
    ratio_swing_ppm: f64,
    worst_fill: usize,
    best_fill: usize,
}

impl LinkReport {
    fn silence_seconds(&self) -> f64 {
        self.zero_filled as f64 / CHANNELS as f64 / SAMPLE_RATE
    }
}

#[test]
fn clean_run_with_matched_clocks() {
    let r = run_link(0.0, 60.0, true);
    assert_eq!(r.underruns, 0, "出现断流: {r:?}");
    assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
}

#[test]
fn survives_one_hour_of_positive_drift() {
    let r = run_link(50.0, 60.0, true);
    assert_eq!(r.underruns, 0, "出现断流: {r:?}");
    assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
    assert_eq!(r.silence_seconds(), 0.0);
    assert!(
        (r.settled_ppm - 50.0).abs() < 5.0,
        "时钟偏差估计偏离过大: {r:?}"
    );
}

#[test]
fn drift_estimate_does_not_oscillate() {
    let r = run_link(50.0, 60.0, true);
    assert!(
        r.ppm_spread < 60.0,
        "ppm 估计在振荡，极差 {:.1}: {r:?}",
        r.ppm_spread
    );
    assert!(
        r.ratio_swing_ppm < 400.0,
        "重采样比例抖动 {:.0} ppm，已经接近可闻: {r:?}",
        r.ratio_swing_ppm
    );
}

#[test]
fn survives_one_hour_of_negative_drift() {
    let r = run_link(-50.0, 60.0, true);
    assert_eq!(r.underruns, 0, "出现断流: {r:?}");
    assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
}

#[test]
fn without_compensation_the_link_breaks() {
    let r = run_link(200.0, 30.0, false);
    assert!(
        r.underruns > 0 || r.overruns > 0,
        "无补偿时应出现欠载或溢出，否则测试场景无效: {r:?}"
    );

    let ok = run_link(200.0, 30.0, true);
    assert_eq!(ok.underruns, 0, "补偿未能阻止断流: {ok:?}");
    assert_eq!(ok.overruns, 0, "补偿未能阻止溢出: {ok:?}");
}

#[test]
fn fill_level_stays_away_from_the_edges() {
    let r = run_link(50.0, 30.0, true);
    let capacity_frames = CAPACITY;
    assert!(r.worst_fill > capacity_frames / 10, "水位过低: {r:?}");
    assert!(r.best_fill < capacity_frames * 9 / 10, "水位过高: {r:?}");
}

#[test]
fn starved_link_reports_every_gap() {
    let (mut tx, mut rx) = ring(1024, CHANNELS);
    tx.push(&[0.25f32; 200]);

    let mut out = vec![0.0f32; 1000];
    match rx.pull(&mut out) {
        PullOutcome::Underrun { filled } => assert_eq!(filled, 800),
        other => panic!("应当报告欠载，实际: {other:?}"),
    }
    assert_eq!(rx.stats().underrun_events(), 1);
    assert!(!rx.stats().is_clean(), "出了故障却报告链路健康");
}
