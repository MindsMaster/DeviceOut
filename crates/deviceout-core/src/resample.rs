use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

use crate::clock::DriftTuning;

const SINC_LEN: usize = 256;
const OVERSAMPLING: usize = 256;
const RATIO_RANGE_MARGIN: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResampleError {
    InvalidChannels,
    InvalidOutputFrames,
    InvalidNominalRatio,
    InputTooShort { needed: usize, got: usize },
    OutputTooShort { needed: usize, got: usize },
    Backend,
}

impl std::fmt::Display for ResampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidChannels => write!(f, "声道数必须为正"),
            Self::InvalidOutputFrames => write!(f, "输出帧数必须为正"),
            Self::InvalidNominalRatio => write!(f, "标称重采样比例必须是有限正数"),
            Self::InputTooShort { needed, got } => {
                write!(f, "输入缓冲太短：需要 {needed} 帧，实际 {got} 帧")
            }
            Self::OutputTooShort { needed, got } => {
                write!(f, "输出缓冲太短：需要 {needed} 帧，实际 {got} 帧")
            }
            Self::Backend => write!(f, "重采样后端报错"),
        }
    }
}

impl std::error::Error for ResampleError {}

#[derive(Debug)]
pub struct DriftResampler {
    inner: Async<f32>,
    channels: usize,
    output_frames: usize,
    nominal_ratio: f64,

    min_ratio: f64,
    max_ratio: f64,

    ratio: f64,
    clamp_events: u64,
}

impl DriftResampler {
    pub fn new(
        channels: usize,
        output_frames: usize,
        nominal_ratio: f64,
        tuning: &DriftTuning,
    ) -> Result<Self, ResampleError> {
        if channels == 0 {
            return Err(ResampleError::InvalidChannels);
        }
        if output_frames == 0 {
            return Err(ResampleError::InvalidOutputFrames);
        }
        if !nominal_ratio.is_finite() || nominal_ratio <= 0.0 {
            return Err(ResampleError::InvalidNominalRatio);
        }

        let max_relative = 1.0 + tuning.max_correction + RATIO_RANGE_MARGIN;

        let params = SincInterpolationParameters::new(SINC_LEN, WindowFunction::BlackmanHarris2)
            .oversampling_factor(OVERSAMPLING)
            .interpolation(SincInterpolationType::Cubic);

        let inner = Async::<f32>::new_sinc(
            nominal_ratio,
            max_relative,
            &params,
            output_frames,
            channels,
            FixedAsync::Output,
        )
            .map_err(|_| ResampleError::Backend)?;

        Ok(Self {
            inner,
            channels,
            output_frames,
            nominal_ratio,
            min_ratio: nominal_ratio * (1.0 - tuning.max_correction),
            max_ratio: nominal_ratio * (1.0 + tuning.max_correction),
            ratio: nominal_ratio,
            clamp_events: 0,
        })
    }

    pub fn set_ratio(&mut self, requested: f64) -> f64 {
        let applied = if requested.is_finite() {
            requested.clamp(self.min_ratio, self.max_ratio)
        } else {
            self.nominal_ratio
        };

        if applied != requested {
            self.clamp_events += 1;
        }

        if applied != self.ratio {
            let accepted = self.inner.set_resample_ratio(applied, true).is_ok();
            debug_assert!(accepted, "rubato 拒绝了已限幅的比例 {applied}");
            if accepted {
                self.ratio = applied;
            }
        }

        self.ratio
    }

    #[inline]
    pub fn input_frames_next(&self) -> usize {
        self.inner.input_frames_next()
    }

    #[inline]
    pub fn input_frames_max(&self) -> usize {
        self.inner.input_frames_max()
    }

    #[inline]
    pub fn output_frames(&self) -> usize {
        self.output_frames
    }

    #[inline]
    pub fn channels(&self) -> usize {
        self.channels
    }

    #[inline]
    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    #[inline]
    pub fn output_delay(&self) -> usize {
        self.inner.output_delay()
    }

    #[inline]
    pub fn clamp_events(&self) -> u64 {
        self.clamp_events
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<usize, ResampleError> {
        let needed_in = self.input_frames_next();
        let in_frames = input.len() / self.channels;
        if in_frames < needed_in {
            return Err(ResampleError::InputTooShort {
                needed: needed_in,
                got: in_frames,
            });
        }

        let out_frames = output.len() / self.channels;
        if out_frames < self.output_frames {
            return Err(ResampleError::OutputTooShort {
                needed: self.output_frames,
                got: out_frames,
            });
        }

        let source = InterleavedSlice::new(input, self.channels, in_frames)
            .map_err(|_| ResampleError::Backend)?;
        let mut sink = InterleavedSlice::new_mut(output, self.channels, out_frames)
            .map_err(|_| ResampleError::Backend)?;

        let (consumed, produced) = self
            .inner
            .process_into_buffer(&source, &mut sink, None)
            .map_err(|_| ResampleError::Backend)?;

        debug_assert_eq!(consumed, needed_in);
        debug_assert_eq!(produced, self.output_frames);
        Ok(consumed)
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.ratio = self.nominal_ratio;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const CHANNELS: usize = 2;
    const OUTPUT_FRAMES: usize = 480;

    fn resampler() -> DriftResampler {
        DriftResampler::new(CHANNELS, OUTPUT_FRAMES, 1.0, &DriftTuning::default()).unwrap()
    }

    fn consume_periods(r: &mut DriftResampler, periods: usize) -> usize {
        let mut input = vec![0.0f32; r.input_frames_max() * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];
        let mut total_in = 0usize;

        for _ in 0..periods {
            let need = r.input_frames_next();
            total_in += r.process(&input[..need * CHANNELS], &mut output).unwrap();
            input[0] = 0.0;
        }
        total_in
    }

    fn steady_state_frame_error(target_ratio: f64, periods: usize) -> f64 {
        let mut r = resampler();
        r.set_ratio(target_ratio);
        consume_periods(&mut r, 100);

        let measured = consume_periods(&mut r, periods) as f64;
        let ideal = periods as f64 * OUTPUT_FRAMES as f64 / target_ratio;
        measured - ideal
    }

    #[test]
    fn fractional_phase_survives_frame_quantisation() {
        let periods = 600;
        let error = steady_state_frame_error(1.0 + 100.0e-6, periods);
        assert!(
            error.abs() < 1.0,
            "{periods} 个周期后输入帧数偏离理想值 {error:.3} 帧，\
             小数相位很可能被整帧量化消除"
        );
    }

    #[test]
    fn fractional_phase_works_for_negative_drift() {
        let periods = 600;
        let error = steady_state_frame_error(1.0 - 100.0e-6, periods);
        assert!(
            error.abs() < 1.0,
            "{periods} 个周期后输入帧数偏离理想值 {error:.3} 帧"
        );
    }

    #[test]
    fn frame_quantisation_would_be_caught() {
        let periods = 600;
        let target = 1.0 + 100.0e-6;

        let quantised: usize = (0..periods)
            .map(|_| (OUTPUT_FRAMES as f64 / target).round() as usize)
            .sum();
        let ideal = periods as f64 * OUTPUT_FRAMES as f64 / target;
        let error = quantised as f64 - ideal;

        assert!(
            error.abs() > 10.0,
            "对照模型只差了 {error:.3} 帧，判别距离不够，\
             说明 fractional_phase_survives_frame_quantisation 的断言无法区分两种实现"
        );
    }

    #[test]
    fn input_request_varies_around_the_average() {
        let mut r = resampler();
        r.set_ratio(1.0 + 100.0e-6);

        let mut input = vec![0.0f32; r.input_frames_max() * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];
        let mut seen = std::collections::BTreeSet::new();

        for _ in 0..200 {
            let need = r.input_frames_next();
            seen.insert(need);
            r.process(&input[..need * CHANNELS], &mut output).unwrap();
            input[0] = 0.0;
        }

        assert!(seen.len() >= 2, "所需输入帧始终是 {seen:?}，小数相位未累积");
    }

    #[test]
    fn input_request_never_exceeds_max() {
        let mut r = resampler();
        let max = r.input_frames_max();
        let mut input = vec![0.0f32; max * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];

        for step in 0..400 {
            let phase = (step as f64 * 0.25).sin();
            r.set_ratio(1.0 + 0.02 * phase);
            let need = r.input_frames_next();
            assert!(need <= max, "所需输入 {need} 帧超过上界 {max} 帧");
            r.process(&input[..need * CHANNELS], &mut output).unwrap();
            input[0] = 0.0;
        }
    }

    #[test]
    fn unity_ratio_preserves_amplitude_and_waveform() {
        const CYCLE: usize = 50;
        let freq = 1.0 / CYCLE as f64;

        let mut r = resampler();
        let mut input = vec![0.0f32; r.input_frames_max() * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];
        let mut collected: Vec<f32> = Vec::new();
        let mut phase = 0usize;

        for block in 0..6 {
            let need = r.input_frames_next();
            for frame in 0..need {
                let v = ((phase + frame) as f64 * freq * TAU).sin() as f32;
                for ch in 0..CHANNELS {
                    input[frame * CHANNELS + ch] = v;
                }
            }
            phase += need;
            r.process(&input[..need * CHANNELS], &mut output).unwrap();

            if block >= 2 {
                collected.extend((0..r.output_frames()).map(|f| output[f * CHANNELS]));
            }
        }

        let n = collected.len() / CYCLE * CYCLE;
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for (k, &v) in collected[..n].iter().enumerate() {
            let w = k as f64 * freq * TAU;
            a += v as f64 * w.sin();
            b += v as f64 * w.cos();
        }
        a *= 2.0 / n as f64;
        b *= 2.0 / n as f64;

        let amplitude = (a * a + b * b).sqrt();
        assert!(
            (amplitude - 1.0).abs() < 0.02,
            "通带幅度变了：{amplitude:.4}，应当接近 1.0"
        );

        let mut peak_residual = 0.0f64;
        for (k, &v) in collected[..n].iter().enumerate() {
            let w = k as f64 * freq * TAU;
            peak_residual = peak_residual.max((v as f64 - (a * w.sin() + b * w.cos())).abs());
        }
        assert!(
            peak_residual < 0.02,
            "输出偏离纯正弦，峰值残差 {peak_residual:.4}"
        );
    }

    #[test]
    fn channels_do_not_bleed_into_each_other() {
        let mut r = resampler();
        let mut input = vec![0.0f32; r.input_frames_max() * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];

        for _ in 0..8 {
            let need = r.input_frames_next();
            for frame in 0..need {
                input[frame * CHANNELS] = 0.5;
                input[frame * CHANNELS + 1] = 0.0;
            }
            r.process(&input[..need * CHANNELS], &mut output).unwrap();
        }

        let left_peak = (0..r.output_frames())
            .map(|f| output[f * CHANNELS].abs())
            .fold(0.0f32, f32::max);
        let right_peak = (0..r.output_frames())
            .map(|f| output[f * CHANNELS + 1].abs())
            .fold(0.0f32, f32::max);

        assert!(left_peak > 0.4, "左声道信号丢了，峰值 {left_peak}");
        assert_eq!(right_peak, 0.0, "右声道被串入了 {right_peak}");
    }

    #[test]
    fn ratio_is_always_clamped() {
        let mut r = resampler();
        let tuning = DriftTuning::default();
        let (lo, hi) = (1.0 - tuning.max_correction, 1.0 + tuning.max_correction);

        for absurd in [1.0e6, -1.0, 0.0, 1.0e-9, f64::INFINITY, f64::NEG_INFINITY] {
            let applied = r.set_ratio(absurd);
            assert!(
                (lo..=hi).contains(&applied),
                "请求 {absurd} 后比例跑到了 {applied}，超出 [{lo}, {hi}]"
            );
        }

        let applied = r.set_ratio(f64::NAN);
        assert_eq!(applied, 1.0, "NaN 没退回标称比例，而是 {applied}");

        assert!(r.clamp_events() >= 7, "限幅发生了却没计数");
    }

    #[test]
    fn saturated_ratio_still_reaches_the_backend() {
        let mut r = resampler();
        let tuning = DriftTuning::default();
        let mut input = vec![0.0f32; r.input_frames_max() * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];

        for target in [1.0 + tuning.max_correction, 1.0 - tuning.max_correction] {
            r.set_ratio(target);
            assert_eq!(r.ratio(), target, "限幅边界上的比例没能生效");

            let need = r.input_frames_next();
            r.process(&input[..need * CHANNELS], &mut output).unwrap();

            let expected = OUTPUT_FRAMES as f64 / target;
            let settled = r.input_frames_next() as f64;
            assert!(
                (settled - expected).abs() < 2.0,
                "所需输入 {settled} 帧与比例 {target} 不符，预期约 {expected:.1}"
            );
            input[0] = 0.0;
        }
    }

    #[test]
    fn output_length_is_fixed() {
        let mut r = resampler();
        let mut input = vec![0.0f32; r.input_frames_max() * CHANNELS];
        let mut output = vec![-1.0f32; (r.output_frames() + 16) * CHANNELS];

        r.set_ratio(1.0 - 0.01);
        for _ in 0..50 {
            let need = r.input_frames_next();
            r.process(&input[..need * CHANNELS], &mut output).unwrap();
            input[0] = 0.0;
        }

        assert!(
            output[r.output_frames() * CHANNELS..]
                .iter()
                .all(|&s| s == -1.0),
            "写出了超过 output_frames() 的数据"
        );
    }

    #[test]
    fn short_buffers_are_rejected() {
        let mut r = resampler();
        let need = r.input_frames_next();
        let input = vec![0.0f32; need * CHANNELS];
        let mut output = vec![0.0f32; r.output_frames() * CHANNELS];

        match r.process(&input[..(need - 1) * CHANNELS], &mut output) {
            Err(ResampleError::InputTooShort { needed, got }) => {
                assert_eq!(needed, need);
                assert_eq!(got, need - 1);
            }
            other => panic!("输入过短应当报错，实际: {other:?}"),
        }

        let short = r.output_frames() - 1;
        match r.process(&input, &mut output[..short * CHANNELS]) {
            Err(ResampleError::OutputTooShort { needed, got }) => {
                assert_eq!(needed, r.output_frames());
                assert_eq!(got, short);
            }
            other => panic!("输出过短应当报错，实际: {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_construction() {
        let t = DriftTuning::default();
        assert_eq!(
            DriftResampler::new(0, 480, 1.0, &t).unwrap_err(),
            ResampleError::InvalidChannels
        );
        assert_eq!(
            DriftResampler::new(2, 0, 1.0, &t).unwrap_err(),
            ResampleError::InvalidOutputFrames
        );
        assert_eq!(
            DriftResampler::new(2, 480, 0.0, &t).unwrap_err(),
            ResampleError::InvalidNominalRatio
        );
        assert_eq!(
            DriftResampler::new(2, 480, f64::NAN, &t).unwrap_err(),
            ResampleError::InvalidNominalRatio
        );
    }

    #[test]
    fn non_unity_nominal_ratio_scales_the_limits() {
        let tuning = DriftTuning::default();
        let nominal = 48_000.0 / 44_100.0;
        let mut r = DriftResampler::new(CHANNELS, OUTPUT_FRAMES, nominal, &tuning).unwrap();

        assert_eq!(r.ratio(), nominal);
        let applied = r.set_ratio(1.0e6);
        assert!(
            (applied - nominal * (1.0 + tuning.max_correction)).abs() < 1.0e-12,
            "限幅没按标称比例缩放：{applied}"
        );

        let need = r.input_frames_next() as f64;
        assert!(
            need < OUTPUT_FRAMES as f64,
            "升采样时所需输入帧应少于输出帧，实际 {need}"
        );
    }

    #[test]
    fn reset_returns_to_nominal() {
        let mut r = resampler();
        r.set_ratio(1.0 + 0.01);
        assert_ne!(r.ratio(), 1.0);

        r.reset();
        assert_eq!(r.ratio(), 1.0);
        assert_eq!(r.input_frames_next(), resampler().input_frames_next());
    }
}
