use deviceout_core::{ring, DriftController, DriftResampler, DriftTuning};

const SAMPLE_RATE: f64 = 6_000.0;
const CHANNELS: usize = 1;
const SINK_BLOCK: usize = 60;
const DAW_BLOCK: usize = 64;
const CAPACITY: usize = 2_400;

#[derive(Debug)]
struct LinkReport {
    underruns: u64,
    overruns: u64,
    settled_ppm: f64,
    ppm_spread: f64,
    lowest_fill: usize,
    highest_fill: usize,
    clamp_events: u64,
}

fn run_link(source_ppm: f64, seconds: f64) -> LinkReport {
    let (mut tx, mut rx) = ring(CAPACITY, CHANNELS);
    let target = (CAPACITY / 2) as f64;
    let tuning = DriftTuning::default();
    let mut ctrl = DriftController::new(target, SAMPLE_RATE, 1.0, tuning);
    let mut resampler = DriftResampler::new(CHANNELS, SINK_BLOCK, 1.0, &tuning).unwrap();

    let mut pull_buf = vec![0.0f32; resampler.input_frames_max() * CHANNELS];
    let mut out_buf = vec![0.0f32; resampler.output_frames() * CHANNELS];
    let daw_block: Vec<f32> = (0..DAW_BLOCK * CHANNELS)
        .map(|i| (i as f32 * 0.05).sin() * 0.5)
        .collect();

    let prime = vec![0.0f32; (target as usize) * CHANNELS];
    tx.push(&prime);

    let source_rate = SAMPLE_RATE * (1.0 + source_ppm * 1.0e-6);
    let sink_period = SINK_BLOCK as f64 / SAMPLE_RATE;
    let steps = (seconds / sink_period) as usize;
    let settled_after = seconds * 2.0 / 3.0;

    let mut daw_credit = 0.0f64;
    let mut lowest_fill = usize::MAX;
    let mut highest_fill = 0usize;
    let mut ppm_lo = f64::MAX;
    let mut ppm_hi = f64::MIN;

    for step in 0..steps {
        daw_credit += source_rate * sink_period;
        while daw_credit >= DAW_BLOCK as f64 {
            daw_credit -= DAW_BLOCK as f64;
            tx.push(&daw_block);
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

        let t = step as f64 * sink_period;

        if t > 2.0 {
            lowest_fill = lowest_fill.min(fill);
            highest_fill = highest_fill.max(fill);
        }

        if t >= settled_after {
            let ppm = ctrl.drift_ppm();
            ppm_lo = ppm_lo.min(ppm);
            ppm_hi = ppm_hi.max(ppm);
        }
    }

    LinkReport {
        underruns: rx.stats().underrun_events(),
        overruns: rx.stats().overrun_events(),
        settled_ppm: ctrl.drift_ppm(),
        ppm_spread: ppm_hi - ppm_lo,
        lowest_fill,
        highest_fill,
        clamp_events: resampler.clamp_events(),
    }
}

const RUN_SECONDS: f64 = 600.0;
const SHORT_RUN_SECONDS: f64 = 200.0;

#[test]
fn real_resampler_closes_the_loop() {
    let r = run_link(50.0, RUN_SECONDS);

    assert_eq!(r.underruns, 0, "出现断流: {r:?}");
    assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
    assert_eq!(r.clamp_events, 0, "比例被限幅，说明控制器饱和了: {r:?}");

    assert!(
        (r.settled_ppm - 50.0).abs() < 5.0,
        "时钟偏差估计偏离真值 50 ppm 过多: {r:?}"
    );

    assert!(
        r.lowest_fill > CAPACITY / 10,
        "水位过低: {r:?}（容量 {CAPACITY} 帧）"
    );
    assert!(
        r.highest_fill < CAPACITY * 9 / 10,
        "水位过高: {r:?}（容量 {CAPACITY} 帧）"
    );

    assert!(
        r.ppm_spread < 5.0,
        "ppm 估计的振荡超出预期，收敛后极差 {:.2} ppm（稳态实测约 3.0 ppm）: {r:?}",
        r.ppm_spread
    );
}

#[test]
fn real_resampler_handles_negative_drift() {
    let r = run_link(-50.0, SHORT_RUN_SECONDS);

    assert_eq!(r.underruns, 0, "出现断流: {r:?}");
    assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
    assert_eq!(r.clamp_events, 0, "比例被限幅: {r:?}");
    assert!(
        (r.settled_ppm + 50.0).abs() < 5.0,
        "时钟偏差估计偏离真值 -50 ppm 过多: {r:?}"
    );
}

#[test]
fn stays_quiet_when_clocks_match() {
    let r = run_link(0.0, SHORT_RUN_SECONDS);

    assert_eq!(r.underruns, 0, "出现断流: {r:?}");
    assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
    assert!(
        r.settled_ppm.abs() < 2.0,
        "时钟一致却产生了 {:.2} ppm 的修正: {r:?}",
        r.settled_ppm
    );
}
