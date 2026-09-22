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
            settle_time_s: 20.0,
            damping: 1.0,
            lowpass_tau_s: 3.0,
            max_correction: 0.02,
        }
    }
}

const LOWPASS_BANDWIDTH_RATIO: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingLimit {
    Free,
    Empty,
    Full,
    Dry,
}

fn max_lowpass_tau_s(wn: f64) -> f64 {
    1.0 / (LOWPASS_BANDWIDTH_RATIO * wn)
}

fn integral_limit(ki: f64, max_correction: f64) -> f64 {
    if ki > 0.0 {
        max_correction / ki
    } else {
        0.0
    }
}

#[derive(Debug)]
pub struct DriftController {
    target_frames: f64,
    kp: f64,
    ki: f64,
    integral_limit: f64,
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
        let ki = wn * wn / g;

        Self {
            target_frames,
            ki,
            kp: 2.0 * tuning.damping * wn / g,
            integral_limit: integral_limit(ki, tuning.max_correction),
            lowpass_tau_s: max_lowpass_tau_s(wn).min(tuning.lowpass_tau_s),
            max_correction: tuning.max_correction,
            nominal_ratio,
            filtered_fill: target_frames,
            integral: 0.0,
            correction: 0.0,
            primed: false,
        }
    }

    pub fn update(&mut self, fill_frames: f64, dt_s: f64, limit: RingLimit) -> f64 {
        debug_assert!(dt_s > 0.0);

        if limit == RingLimit::Dry {
            return self.ratio();
        }

        if !self.primed {
            self.filtered_fill = fill_frames;
            self.primed = true;
        }

        let alpha = dt_s / (self.lowpass_tau_s + dt_s);
        self.filtered_fill += alpha * (fill_frames - self.filtered_fill);

        let err_norm = (self.filtered_fill - self.target_frames) / self.target_frames;

        let integral_candidate =
            (self.integral + err_norm * dt_s).clamp(-self.integral_limit, self.integral_limit);
        let mut raw = self.kp * err_norm + self.ki * integral_candidate;

        if raw.abs() < self.max_correction {
            self.integral = integral_candidate;
        }

        if self.disproved_by(limit, raw) {
            self.integral =
                (-self.kp * err_norm / self.ki).clamp(-self.integral_limit, self.integral_limit);
            raw = self.kp * err_norm + self.ki * self.integral;
        }

        self.correction = raw.clamp(-self.max_correction, self.max_correction);
        self.ratio()
    }

    fn disproved_by(&self, limit: RingLimit, raw: f64) -> bool {
        if self.ki <= 0.0 {
            return false;
        }
        match limit {
            RingLimit::Free | RingLimit::Dry => false,
            RingLimit::Empty => raw > 0.0,
            RingLimit::Full => raw < 0.0,
        }
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

    #[inline]
    pub fn lowpass_tau_s(&self) -> f64 {
        self.lowpass_tau_s
    }

    pub fn seed_drift_ppm(&mut self, ppm: f64) {
        if !ppm.is_finite() || ppm == 0.0 || self.ki == 0.0 {
            return;
        }
        self.integral = (ppm * 1.0e-6 / self.ki).clamp(-self.integral_limit, self.integral_limit);
        self.correction =
            (self.ki * self.integral).clamp(-self.max_correction, self.max_correction);
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
    const SHALLOW_TARGET: f64 = 1_790.0;
    const SHALLOW_CAPACITY: f64 = 8_192.0;

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
        simulate_at(TARGET, CAPACITY, ppm, secs, controlled)
    }

    fn simulate_at(target: f64, capacity: f64, ppm: f64, secs: f64, controlled: bool) -> SimResult {
        simulate_seeded(target, capacity, ppm, 0.0, secs, controlled)
    }

    fn simulate_seeded(
        target: f64,
        capacity: f64,
        ppm: f64,
        seed_ppm: f64,
        secs: f64,
        controlled: bool,
    ) -> SimResult {
        let source_rate = SINK_RATE * (1.0 + ppm * 1.0e-6);
        let dt = BLOCK / SINK_RATE;
        let mut ctrl = DriftController::new(target, SINK_RATE, 1.0, DriftTuning::default());
        ctrl.seed_drift_ppm(seed_ppm);

        let mut fill = target;
        let (mut underruns, mut overruns) = (0u32, 0u32);
        let (mut max_fill, mut min_fill) = (fill, fill);
        let warmup = (2.0 / dt) as usize;

        for step in 0..(secs / dt) as usize {
            let mut limit = RingLimit::Free;

            fill += source_rate * dt;
            if fill > capacity {
                overruns += 1;
                fill = capacity;
                limit = RingLimit::Full;
            }

            let ratio = if controlled { ctrl.ratio() } else { 1.0 };
            let needed = BLOCK / ratio;
            if fill < needed {
                underruns += 1;
                fill = 0.0;
                limit = RingLimit::Empty;
            } else {
                fill -= needed;
            }

            if controlled {
                ctrl.update(fill, dt, limit);
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
    fn a_virtual_cable_off_by_most_of_a_percent_is_still_tracked() {
        for ppm in [1_000.0, 8_000.0, -8_000.0] {
            let r = simulate(ppm, 600.0, true);
            assert_eq!(r.underruns, 0, "{ppm} ppm 出现欠载: {r:?}");
            assert_eq!(r.overruns, 0, "{ppm} ppm 出现溢出: {r:?}");
            assert!(
                (r.settled_ppm - ppm).abs() < ppm.abs() * 0.05 + 20.0,
                "{ppm} ppm 未被跟踪上: {r:?}"
            );
        }
    }

    #[test]
    fn a_seed_the_run_disproves_is_dropped_within_seconds() {
        let r = simulate_seeded(SHALLOW_TARGET, SHALLOW_CAPACITY, 0.0, 12_911.0, 600.0, true);
        assert!(r.underruns <= 4, "错误的种子仍在持续掏空环形缓冲: {r:?}");
        assert!(r.settled_ppm.abs() < 100.0, "种子始终没有被纠正: {r:?}");
    }

    #[test]
    fn the_shallow_ring_the_plugin_uses_survives_a_seed_pointing_the_wrong_way() {
        for (real, seed) in [(0.0, 8_000.0), (100.0, 12_911.0), (-1_000.0, 8_000.0)] {
            let r = simulate_seeded(SHALLOW_TARGET, SHALLOW_CAPACITY, real, seed, 600.0, true);
            assert!(
                r.underruns < 200,
                "真实 {real} ppm 配上 {seed} ppm 的种子仍然崩了: {r:?}"
            );
            assert!(
                (r.settled_ppm - real).abs() < real.abs() * 0.05 + 100.0,
                "真实 {real} ppm 未被跟踪上: {r:?}"
            );
        }
    }

    #[test]
    fn a_silent_host_teaches_the_estimate_nothing() {
        let mut ctrl = DriftController::new(SHALLOW_TARGET, SINK_RATE, 1.0, DriftTuning::default());
        let dt = BLOCK / SINK_RATE;

        for _ in 0..600 {
            ctrl.update(SHALLOW_TARGET * 0.4, dt, RingLimit::Free);
        }
        let learned = ctrl.drift_ppm();
        assert!(learned < -1.0, "没有先学到东西，测不出冻结: {learned}");

        for _ in 0..400 {
            ctrl.update(0.0, dt, RingLimit::Dry);
        }
        assert!(
            (ctrl.drift_ppm() - learned).abs() < 1.0e-9,
            "源断供期间估计被改写了: {learned} -> {}",
            ctrl.drift_ppm()
        );
    }

    #[test]
    fn an_empty_ring_overrules_whatever_the_integral_believes() {
        let mut ctrl = DriftController::new(SHALLOW_TARGET, SINK_RATE, 1.0, DriftTuning::default());
        ctrl.seed_drift_ppm(12_911.0);
        let dt = BLOCK / SINK_RATE;
        let barely = SHALLOW_TARGET * 0.5;

        ctrl.update(barely, dt, RingLimit::Free);
        assert!(ctrl.correction_ppm() > 0.0, "尚未见底就不该清掉修正");

        ctrl.update(barely, dt, RingLimit::Empty);
        assert!(
            ctrl.correction_ppm() <= 1.0,
            "见底之后仍在加速消费: {} ppm",
            ctrl.correction_ppm()
        );
    }

    #[test]
    fn the_integral_never_outruns_the_correction_authority() {
        let tuning = DriftTuning::default();
        let ceiling = tuning.max_correction * 1.0e6;

        let mut ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, tuning);
        ctrl.seed_drift_ppm(50_000.0);
        assert!(ctrl.drift_ppm() <= ceiling + 1.0);

        let r = simulate(60_000.0, 600.0, true);
        assert!(
            r.settled_ppm.abs() <= ceiling + 1.0,
            "积分绕过了修正上限: {r:?}"
        );
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
    fn the_lowpass_can_never_outrun_the_control_bandwidth() {
        let slack = DriftTuning {
            lowpass_tau_s: 30.0,
            ..DriftTuning::default()
        };
        let ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, slack);
        assert!(
            ctrl.lowpass_tau_s() <= 3.0 + 1e-12,
            "{}",
            ctrl.lowpass_tau_s()
        );

        let tight = DriftTuning {
            lowpass_tau_s: 0.2,
            ..DriftTuning::default()
        };
        let ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, tight);
        assert_eq!(ctrl.lowpass_tau_s(), 0.2);
    }

    #[test]
    fn a_thirty_millisecond_target_still_holds() {
        const SMALL: f64 = 1_440.0;
        const ROOM: f64 = 8_192.0;

        for ppm in [-1_000.0, -152.0, 0.0, 152.0, 1_000.0] {
            let r = simulate_at(SMALL, ROOM, ppm, 900.0, true);
            assert_eq!(r.underruns, 0, "{ppm} ppm 下欠载: {r:?}");
            assert_eq!(r.overruns, 0, "{ppm} ppm 下溢出: {r:?}");
            assert!(
                (r.final_fill - SMALL).abs() / SMALL < 0.02,
                "{ppm} ppm 未收敛: {r:?}"
            );
        }
    }

    #[test]
    fn the_settling_excursion_does_not_grow_as_the_target_shrinks() {
        let wide = simulate_at(TARGET, CAPACITY, 152.0, 900.0, true);
        let tight = simulate_at(1_440.0, 8_192.0, 152.0, 900.0, true);

        let wide_swing = (wide.max_fill - TARGET).max(TARGET - wide.min_fill);
        let tight_swing = (tight.max_fill - 1_440.0).max(1_440.0 - tight.min_fill);
        assert!(
            tight_swing < wide_swing * 1.5,
            "小水位的偏移被放大了: {wide_swing:.0} -> {tight_swing:.0}"
        );
        assert!(tight_swing < 200.0, "偏移超出预期: {tight_swing:.0}");
    }

    #[test]
    fn a_seeded_controller_starts_where_it_would_have_settled() {
        let mut ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, DriftTuning::default());
        ctrl.seed_drift_ppm(-152.0);
        assert!(
            (ctrl.drift_ppm() + 152.0).abs() < 1.0e-6,
            "{}",
            ctrl.drift_ppm()
        );
        assert!((ctrl.correction_ppm() + 152.0).abs() < 1.0e-6);

        ctrl.seed_drift_ppm(f64::NAN);
        assert!((ctrl.drift_ppm() + 152.0).abs() < 1.0e-6);

        let mut wild = DriftController::new(TARGET, SINK_RATE, 1.0, DriftTuning::default());
        wild.seed_drift_ppm(1.0e9);
        assert!((1.0 - wild.ratio()).abs() <= DriftTuning::default().max_correction + 1e-12);
    }

    #[test]
    fn a_seeded_controller_holds_the_level_from_the_first_second() {
        let ppm = 400.0;
        let source_rate = SINK_RATE * (1.0 + ppm * 1.0e-6);
        let dt = BLOCK / SINK_RATE;

        let drop_of = |seed: f64| {
            let mut ctrl = DriftController::new(1_440.0, SINK_RATE, 1.0, DriftTuning::default());
            ctrl.seed_drift_ppm(seed);
            let mut fill = 1_440.0;
            let mut worst: f64 = 0.0;
            for _ in 0..(30.0 / dt) as usize {
                fill += source_rate * dt - BLOCK / ctrl.ratio();
                ctrl.update(fill, dt, RingLimit::Free);
                worst = worst.max((fill - 1_440.0).abs());
            }
            worst
        };

        let cold = drop_of(0.0);
        let warm = drop_of(ppm);
        assert!(
            warm < cold / 4.0,
            "种子没有帮助: 冷启 {cold:.0} 帧, 热启 {warm:.0} 帧"
        );
    }

    #[test]
    fn correction_is_always_clamped() {
        let tuning = DriftTuning::default();
        let mut ctrl = DriftController::new(TARGET, SINK_RATE, 1.0, tuning);
        for _ in 0..100_000 {
            ctrl.update(CAPACITY, BLOCK / SINK_RATE, RingLimit::Free);
        }
        let r = ctrl.ratio();
        assert!(
            (1.0 - r).abs() <= tuning.max_correction + 1e-12,
            "比例超出限幅: {r}"
        );

        for _ in 0..100_000 {
            ctrl.update(TARGET, BLOCK / SINK_RATE, RingLimit::Free);
        }
        assert!(
            (1.0 - ctrl.ratio()).abs() < tuning.max_correction,
            "积分饱和后未能退出: {}",
            ctrl.ratio()
        );
    }
}
