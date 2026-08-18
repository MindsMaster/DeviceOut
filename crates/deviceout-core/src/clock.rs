#[derive(Debug, Clone, Copy)]
pub struct DriftTuning {
    pub settle_time_s: f64,
    pub damping: f64,
    pub lowpass_tau_s: f64,
    pub max_correction: f64,
}

impl Default for DriftTuning {
    fn default() -> Self {
        Self {
            settle_time_s: 60.0,
            damping: 1.0,
            lowpass_tau_s: 3.0,
            max_correction: 0.02,
        }
    }
}

#[derive(Debug)]
pub struct DriftController {
    target_frames: f64,
    kp: f64,
    ki: f64,
    lowpass_tau_s: f64,
    max_correction: f64,
    nominal_ratio: f64,

    filtered_fill: f64,
    integral: f64,
    correction: f64,
    primed: bool,
}

impl DriftController {
    pub fn new(
        target_frames: f64,
        sink_rate_hz: f64,
        nominal_ratio: f64,
        tuning: DriftTuning,
    ) -> Self {
        assert!(target_frames > 0.0, "目标水位必须为正");
        assert!(sink_rate_hz > 0.0, "采样率必须为正");
        assert!(nominal_ratio > 0.0, "标称比例必须为正");
        assert!(tuning.settle_time_s > 0.0 && tuning.damping > 0.0);

        let g = sink_rate_hz / target_frames;
        let wn = 4.0 / (tuning.damping * tuning.settle_time_s);

        debug_assert!(
            1.0 / tuning.lowpass_tau_s >= 5.0 * wn,
            "低通截止 {:.3} rad/s 相对控制带宽 {:.3} rad/s 太低 \
             请增大 settle_time_s 或减小 lowpass_tau_s",
            1.0 / tuning.lowpass_tau_s,
            wn
        );

        Self {
            target_frames,
            ki: wn * wn / g,
            kp: 2.0 * tuning.damping * wn / g,
            lowpass_tau_s: tuning.lowpass_tau_s,
            max_correction: tuning.max_correction,
            nominal_ratio,
            filtered_fill: target_frames,
            integral: 0.0,
            correction: 0.0,
            primed: false,
        }
    }

    pub fn update(&mut self, fill_frames: f64, dt_s: f64) -> f64 {
        debug_assert!(dt_s > 0.0);

        if !self.primed {
            self.filtered_fill = fill_frames;
            self.primed = true;
        }

        let alpha = dt_s / (self.lowpass_tau_s + dt_s);
        self.filtered_fill += alpha * (fill_frames - self.filtered_fill);

        let err_norm = (self.filtered_fill - self.target_frames) / self.target_frames;

        let integral_candidate = self.integral + err_norm * dt_s;
        let raw = self.kp * err_norm + self.ki * integral_candidate;

        if raw.abs() < self.max_correction {
            self.integral = integral_candidate;
        }

        self.correction = raw.clamp(-self.max_correction, self.max_correction);
        self.ratio()
    }

    #[inline]
    pub fn ratio(&self) -> f64 {
        self.nominal_ratio * (1.0 - self.correction)
    }

    #[inline]
    pub fn drift_ppm(&self) -> f64 {
        self.ki * self.integral * 1.0e6
    }

    #[inline]
    pub fn correction_ppm(&self) -> f64 {
        self.correction * 1.0e6
    }

    #[inline]
    pub fn smoothed_fill(&self) -> f64 {
        self.filtered_fill
    }

    #[inline]
    pub fn target_frames(&self) -> f64 {
        self.target_frames
    }

    pub fn reset(&mut self) {
        self.filtered_fill = self.target_frames;
        self.integral = 0.0;
        self.correction = 0.0;
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINK_RATE: f64 = 48_000.0;
    const BLOCK: f64 = 480.0;
    const CAPACITY: f64 = 19_200.0;
    const TARGET: f64 = CAPACITY / 2.0;

    #[derive(Debug)]
    struct SimResult {
        final_fill: f64,
        settled_ppm: f64,
        underruns: u32,
        overruns: u32,
        max_fill: f64,
        min_fill: f64,
    }

    fn simulate(ppm: f64, secs: f64, controlled: bool) -> SimResult {
        let source_rate = SINK_RATE * (1.0 + ppm * 1.0e-6);
        let dt = BLOCK / SINK_RATE;
        let mut ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, DriftTuning::default());

        let mut fill = TARGET;
        let (mut underruns, mut overruns) = (0u32, 0u32);
        let (mut max_fill, mut min_fill) = (fill, fill);
        let warmup = (2.0 / dt) as usize;

        for step in 0..(secs / dt) as usize {
            fill += source_rate * dt;
            if fill > CAPACITY {
                overruns += 1;
                fill = CAPACITY;
            }

            let ratio = if controlled { ctrl.ratio() } else { 1.0 };
            let needed = BLOCK / ratio;
            if fill < needed {
                underruns += 1;
                fill = 0.0;
            } else {
                fill -= needed;
            }

            if controlled {
                ctrl.update(fill, dt);
            }

            if step > warmup {
                max_fill = max_fill.max(fill);
                min_fill = min_fill.min(fill);
            }
        }

        SimResult {
            final_fill: fill,
            settled_ppm: ctrl.drift_ppm(),
            underruns,
            overruns,
            max_fill,
            min_fill,
        }
    }

    #[test]
    fn uncontrolled_drift_destroys_the_buffer() {
        let r = simulate(100.0, 2400.0, false);
        assert!(r.overruns > 0, "100 ppm 运行四十分钟应出现溢出: {r:?}");

        let ok = simulate(100.0, 2400.0, true);
        assert_eq!(ok.overruns, 0, "控制器未能阻止溢出: {ok:?}");
        assert_eq!(ok.underruns, 0, "控制器未能阻止欠载: {ok:?}");
    }

    #[test]
    fn converges_with_positive_drift() {
        let r = simulate(100.0, 600.0, true);
        let err = (r.final_fill - TARGET).abs() / TARGET;

        assert_eq!(r.underruns, 0, "出现欠载: {r:?}");
        assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
        assert!(err < 0.02, "水位未收敛到中点 2% 以内: {r:?}");
        assert!(
            (r.settled_ppm - 100.0).abs() < 5.0,
            "时钟偏差估计不准，应接近 100 ppm: {r:?}"
        );
    }

    #[test]
    fn converges_with_negative_drift() {
        let r = simulate(-100.0, 600.0, true);
        let err = (r.final_fill - TARGET).abs() / TARGET;

        assert_eq!(r.underruns, 0, "出现欠载: {r:?}");
        assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
        assert!(err < 0.02, "水位未收敛到中点 2% 以内: {r:?}");
        assert!(
            (r.settled_ppm + 100.0).abs() < 5.0,
            "时钟偏差估计不准，应接近 -100 ppm: {r:?}"
        );
    }

    #[test]
    fn survives_extreme_drift() {
        let r = simulate(1000.0, 600.0, true);
        assert_eq!(r.underruns, 0, "出现欠载: {r:?}");
        assert_eq!(r.overruns, 0, "出现溢出: {r:?}");
    }

    #[test]
    fn stays_quiet_when_clocks_match() {
        let r = simulate(0.0, 600.0, true);
        assert_eq!(r.underruns, 0);
        assert_eq!(r.overruns, 0);
        assert!(r.settled_ppm.abs() < 2.0, "时钟一致却产生了修正: {r:?}");
    }

    #[test]
    fn does_not_overshoot() {
        let r = simulate(100.0, 600.0, true);
        assert!(
            r.min_fill > TARGET * 0.85,
            "临界阻尼下出现了明显过冲: {r:?}"
        );
        assert!(r.max_fill < TARGET * 1.15, "水位向上偏离过多: {r:?}");
    }

    #[test]
    fn correction_is_always_clamped() {
        let tuning = DriftTuning::default();
        let mut ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, tuning);
        for _ in 0..100_000 {
            ctrl.update(CAPACITY, BLOCK / SINK_RATE);
        }
        let r = ctrl.ratio();
        assert!(
            (1.0 - r).abs() <= tuning.max_correction + 1e-12,
            "比例超出限幅: {r}"
        );

        for _ in 0..100_000 {
            ctrl.update(TARGET, BLOCK / SINK_RATE);
        }
        assert!(
            (1.0 - ctrl.ratio()).abs() < tuning.max_correction,
            "积分饱和后未能退出: {}",
            ctrl.ratio()
        );
    }
}
