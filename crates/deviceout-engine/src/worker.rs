use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use deviceout_core::{DriftController, DriftResampler, DriftTuning, RingConsumer, RingLimit};
use deviceout_sink::{AudioSink, SinkError, StreamFormat};

use crate::error::{EngineError, Fault};
use crate::metrics::{EngineMetrics, EngineState, StreamInfo};
use crate::mixer::{Mixer, SourceBus, SourceId};
use crate::trace::{Trace, TraceHeader};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub device_id: String,
    pub source_rate_hz: f64,
    pub channels: usize,
    pub max_block_frames: usize,
    pub target_ms: f64,
    pub device_buffer_ms: u32,
    pub device_queue_periods: u32,
    pub exclusive: bool,
    pub initial_drift_ppm: f64,
    pub prime_timeout_s: f64,
    pub tuning: DriftTuning,
    pub trace_path: Option<std::path::PathBuf>,
    pub host_drops: Option<&'static AtomicU64>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            source_rate_hz: 48_000.0,
            channels: 2,
            max_block_frames: DEFAULT_BLOCK_FRAMES,
            target_ms: DEFAULT_TARGET_MS,
            device_buffer_ms: 80,
            device_queue_periods: deviceout_sink::DEFAULT_QUEUE_PERIODS,
            exclusive: false,
            initial_drift_ppm: 0.0,
            trace_path: None,
            host_drops: None,
            prime_timeout_s: 5.0,
            tuning: DriftTuning::default(),
        }
    }
}

pub const DEFAULT_TARGET_MS: f64 = 80.0;
pub const DEFAULT_BLOCK_FRAMES: usize = 512;

pub const ASSUMED_PERIOD_MS: f64 = 10.0;
pub const ASSUMED_RESAMPLER_MS: f64 = 3.0;

const CAPACITY_FACTOR: usize = 4;
const MIN_CAPACITY_FRAMES: usize = 2_048;
const PERIOD_MARGIN: usize = 1;
const SETTLE_COLD: f64 = 2.0;
const SETTLE_CHECK_S: f64 = 2.0;
const SETTLE_TOL_PPM: f64 = 2.0;
const SETTLE_STABLE_CHECKS: u32 = 3;
const SETTLE_MIN_S: f64 = 10.0;
const SETTLE_WARM: f64 = 0.5;

pub fn ring_capacity_frames(source_rate_hz: f64, ring_ms: f64) -> usize {
    ((source_rate_hz * ring_ms * 1.0e-3).round() as usize).max(2)
}

pub fn frames_for_ms(ms: f64, rate_hz: f64) -> usize {
    if !ms.is_finite() || ms <= 0.0 || !rate_hz.is_finite() || rate_hz <= 0.0 {
        return 1;
    }
    ((ms * rate_hz * 1.0e-3).round() as usize).max(1)
}

pub fn min_target_frames(max_block_frames: usize, period_frames: usize) -> usize {
    (max_block_frames + PERIOD_MARGIN * period_frames).max(1)
}

pub fn min_total_ms(max_block_frames: usize, source_rate_hz: f64, queue_periods: u32) -> f64 {
    let periods = queue_periods.clamp(
        deviceout_sink::MIN_QUEUE_PERIODS,
        deviceout_sink::MAX_QUEUE_PERIODS,
    );
    let period = frames_for_ms(ASSUMED_PERIOD_MS, source_rate_hz);
    let ring = ms_of(min_target_frames(max_block_frames, period), source_rate_hz);
    ring + ASSUMED_PERIOD_MS * f64::from(periods) + ASSUMED_RESAMPLER_MS
}

pub fn ring_capacity_for_target(target_frames: usize) -> usize {
    target_frames
        .saturating_mul(CAPACITY_FACTOR)
        .max(MIN_CAPACITY_FRAMES)
        .next_power_of_two()
}

pub fn ring_capacity_for(config: &EngineConfig) -> usize {
    let target = frames_for_ms(config.target_ms, config.source_rate_hz);
    ring_capacity_for_target(target.max(config.max_block_frames))
}

fn settle_wait(config: &EngineConfig) -> f64 {
    let scale = if config.initial_drift_ppm == 0.0 {
        SETTLE_COLD
    } else {
        SETTLE_WARM
    };
    config.tuning.settle_time_s * scale
}

pub fn ms_of(frames: usize, rate_hz: f64) -> f64 {
    if !rate_hz.is_finite() || rate_hz <= 0.0 {
        return 0.0;
    }
    frames as f64 * 1.0e3 / rate_hz
}

fn target_level(
    config: &EngineConfig,
    period_frames: usize,
    capacity_frames: usize,
    downstream_ms: f64,
) -> TargetLevel {
    let ceiling = (capacity_frames / 2).max(1);
    let floor = min_target_frames(config.max_block_frames, period_frames).min(ceiling);
    let ring_ms = (config.target_ms - downstream_ms).max(0.0);
    let frames = frames_for_ms(ring_ms, config.source_rate_hz).clamp(floor, ceiling);
    TargetLevel {
        frames: frames as f64,
        floor,
        total_floor_ms: ms_of(floor, config.source_rate_hz) + downstream_ms,
    }
}

#[derive(Debug, Clone, Copy)]
struct TargetLevel {
    frames: f64,
    floor: usize,
    total_floor_ms: f64,
}

#[derive(Debug)]
struct SettleWatch {
    deadline_s: f64,
    last_ppm: f64,
    next_check_s: f64,
    stable: u32,
    settled: bool,
}

impl SettleWatch {
    fn new(deadline_s: f64, seed_ppm: f64) -> Self {
        Self {
            deadline_s,
            last_ppm: seed_ppm,
            next_check_s: SETTLE_CHECK_S,
            stable: 0,
            settled: false,
        }
    }

    fn update(&mut self, ppm: f64, elapsed_s: f64) -> bool {
        if self.settled {
            return true;
        }
        if elapsed_s >= self.deadline_s {
            self.settled = true;
            return true;
        }
        if elapsed_s < self.next_check_s {
            return false;
        }
        if (ppm - self.last_ppm).abs() <= SETTLE_TOL_PPM {
            self.stable += 1;
        } else {
            self.stable = 0;
        }
        self.last_ppm = ppm;
        self.next_check_s = elapsed_s + SETTLE_CHECK_S;
        self.settled = self.stable >= SETTLE_STABLE_CHECKS && elapsed_s >= SETTLE_MIN_S;
        self.settled
    }
}

pub trait OpenSink: Send + 'static {
    type Sink: AudioSink + 'static;
    fn open(&mut self, config: &EngineConfig) -> Result<Self::Sink, SinkError>;
}

impl<S, F> OpenSink for F
where
    S: AudioSink + 'static,
    F: FnMut(&EngineConfig) -> Result<S, SinkError> + Send + 'static,
{
    type Sink = S;
    fn open(&mut self, config: &EngineConfig) -> Result<S, SinkError> {
        self(config)
    }
}

#[derive(Debug)]
pub struct EngineHandle {
    metrics: Arc<EngineMetrics>,
    sources: Arc<SourceBus>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn metrics(&self) -> &Arc<EngineMetrics> {
        &self.metrics
    }

    pub fn sources(&self) -> &Arc<SourceBus> {
        &self.sources
    }

    pub fn attach(&self, consumer: RingConsumer) -> SourceId {
        self.sources.join(consumer)
    }

    pub fn detach(&self, id: SourceId) {
        self.sources.leave(id);
    }

    pub fn stop(&mut self) -> bool {
        self.stop.store(true, Ordering::Relaxed);
        match self.thread.take() {
            Some(thread) => {
                let _ = thread.join();
                true
            }
            None => false,
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
pub fn start(sources: Arc<SourceBus>, config: EngineConfig) -> EngineHandle {
    start_with(sources, config, |cfg: &EngineConfig| {
        deviceout_sink::WasapiSink::open(
            &cfg.device_id,
            deviceout_sink::SinkOptions {
                buffer_ms: cfg.device_buffer_ms,
                queue_periods: cfg.device_queue_periods,
                exclusive: cfg.exclusive,
            },
        )
    })
}

pub fn start_with<O: OpenSink>(
    sources: Arc<SourceBus>,
    config: EngineConfig,
    open: O,
) -> EngineHandle {
    let metrics = Arc::new(EngineMetrics::new());
    let stop = Arc::new(AtomicBool::new(false));

    if let Err(e) = validate(&config) {
        metrics.set_failed(e.fault());
        return EngineHandle {
            metrics,
            sources,
            stop,
            thread: None,
        };
    }

    let (ready_tx, ready_rx) = mpsc::channel::<()>();

    let thread = {
        let metrics = Arc::clone(&metrics);
        let sources = Arc::clone(&sources);
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("deviceout-output".into())
            .spawn(move || run(config, open, sources, metrics, stop, ready_tx))
    };

    match thread {
        Ok(thread) => {
            let _ = ready_rx.recv();
            EngineHandle {
                metrics,
                sources,
                stop,
                thread: Some(thread),
            }
        }
        Err(e) => {
            metrics.set_failed(Fault::thread(format!("无法创建输出线程: {e}")));
            EngineHandle {
                metrics,
                sources,
                stop,
                thread: None,
            }
        }
    }
}

fn validate(config: &EngineConfig) -> Result<(), EngineError> {
    if config.channels == 0 {
        return Err(EngineError::Config("声道数必须为正".into()));
    }
    if !config.source_rate_hz.is_finite() || config.source_rate_hz <= 0.0 {
        return Err(EngineError::Config("DAW 侧采样率必须是有限正数".into()));
    }
    if !config.target_ms.is_finite() || config.target_ms <= 0.0 {
        return Err(EngineError::Config("目标延迟必须是有限正数".into()));
    }
    Ok(())
}

fn backoff_delay(attempt: u32) -> Duration {
    const BASE_MS: u64 = 250;
    const CAP_MS: u64 = 5_000;
    let shift = attempt.saturating_sub(1).min(5);
    Duration::from_millis((BASE_MS << shift).min(CAP_MS))
}

fn sleep_interruptible(total: Duration, stop: &AtomicBool) {
    const SLICE: Duration = Duration::from_millis(50);
    let deadline = Instant::now() + total;
    while !stop.load(Ordering::Relaxed) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        thread::sleep(SLICE.min(left));
    }
}

fn signal_ready(ready: &mut Option<mpsc::Sender<()>>) {
    if let Some(tx) = ready.take() {
        let _ = tx.send(());
    }
}

fn run<O: OpenSink>(
    config: EngineConfig,
    mut open: O,
    sources: Arc<SourceBus>,
    metrics: Arc<EngineMetrics>,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<()>,
) {
    #[cfg(windows)]
    let _priority = deviceout_sink::AudioPriority::raise_current_thread().ok();

    let mut mixer = Mixer::new(
        Arc::clone(&sources),
        Arc::clone(metrics.stats()),
        config.channels,
    );
    let mut ready = Some(ready);
    let mut saw_device_loss = false;
    let mut attempt = 0u32;
    let mut trace: Option<Trace> = None;

    while !stop.load(Ordering::Relaxed) {
        let opened = open
            .open(&config)
            .map_err(EngineError::from)
            .and_then(|sink| Worker::new(sink, &config, &metrics));
        match opened {
            Ok(mut worker) => {
                if saw_device_loss {
                    metrics.record_reconnect();
                    attempt = 0;
                }
                if trace.is_none() {
                    if let Some(path) = config.trace_path.clone() {
                        trace = Some(Trace::spawn(
                            path,
                            worker.trace.clone(),
                            Arc::clone(&metrics),
                            Arc::clone(&sources),
                            config.host_drops,
                        ));
                    }
                }
                mixer.reserve(worker.pull_buf.len());
                signal_ready(&mut ready);

                match worker.run(&mut mixer, &metrics, &stop) {
                    Ok(()) => {
                        metrics.set_state(EngineState::Stopped);
                        return;
                    }
                    Err(e) if e.is_recoverable() => {
                        metrics.set_reconnecting(e.fault());
                        saw_device_loss = true;
                    }
                    Err(e) => {
                        metrics.set_failed(e.fault());
                        return;
                    }
                }
            }

            Err(e) if saw_device_loss && e.is_recoverable() => {
                attempt = attempt.saturating_add(1);
                metrics.set_reconnecting(e.fault().with_attempt(attempt));
            }

            Err(e) => {
                metrics.set_failed(e.fault());
                signal_ready(&mut ready);
                return;
            }
        }

        if saw_device_loss {
            signal_ready(&mut ready);
            sleep_interruptible(backoff_delay(attempt), &stop);
        }
    }

    metrics.set_state(EngineState::Stopped);
}

struct Worker<S: AudioSink> {
    sink: S,
    ctrl: DriftController,
    resampler: DriftResampler,
    pull_buf: Vec<f32>,
    out_buf: Vec<f32>,
    format: StreamFormat,
    period_frames: usize,
    channels: usize,
    dt_s: f64,
    warmup_periods: usize,
    prime_timeout_s: f64,
    target_frames: f64,
    settle: SettleWatch,
    trace: TraceHeader,
}

impl<S: AudioSink> Worker<S> {
    fn new(sink: S, config: &EngineConfig, metrics: &EngineMetrics) -> Result<Self, EngineError> {
        let format = sink.format();
        let channels = format.channels as usize;
        if channels != config.channels {
            return Err(EngineError::ChannelMismatch {
                device: channels,
                source: config.channels,
            });
        }

        let sink_rate = f64::from(format.sample_rate);
        let period_frames = sink.period_frames();
        let nominal_ratio = sink_rate / config.source_rate_hz;
        let period_source_frames = ((period_frames as f64 / nominal_ratio).ceil() as usize).max(1);

        let resampler =
            DriftResampler::new(channels, period_frames, nominal_ratio, &config.tuning)?;
        let downstream_frames = sink.queue_limit_frames() + resampler.output_delay();
        let capacity_frames = ring_capacity_for(config);
        let level = target_level(
            config,
            period_source_frames,
            capacity_frames,
            ms_of(downstream_frames, sink_rate),
        );
        let target_frames = level.frames;

        let mut ctrl = DriftController::new(target_frames, sink_rate, nominal_ratio, config.tuning);
        ctrl.seed_drift_ppm(config.initial_drift_ppm);

        let pull_buf = vec![0.0f32; resampler.input_frames_max() * channels];
        let out_buf = vec![0.0f32; resampler.output_frames() * channels];

        metrics.set_stream(StreamInfo {
            source_rate_hz: config.source_rate_hz,
            sink_rate_hz: sink_rate,
            period_frames,
            channels,
            capacity_frames,
            resampler_delay_frames: resampler.output_delay(),
            min_period_frames: sink.min_period_frames(),
            exclusive: sink.exclusive(),
        });
        metrics.set_target(
            target_frames,
            level.floor,
            level.total_floor_ms,
            settle_wait(config),
        );

        let trace = TraceHeader {
            device_id: config.device_id.clone(),
            source_rate_hz: config.source_rate_hz,
            sink_rate_hz: sink_rate,
            channels,
            max_block_frames: config.max_block_frames,
            period_frames,
            min_period_frames: sink.min_period_frames(),
            queue_periods: config.device_queue_periods,
            exclusive: sink.exclusive(),
            target_ms: config.target_ms,
            target_frames,
            floor_frames: level.floor,
            capacity_frames,
            resampler_delay_frames: resampler.output_delay(),
            device_buffer_frames: sink.buffer_frames(),
            queue_limit_frames: sink.queue_limit_frames(),
        };

        Ok(Self {
            sink,
            ctrl,
            resampler,
            pull_buf,
            out_buf,
            format,
            period_frames,
            channels,
            dt_s: period_frames as f64 / sink_rate,
            warmup_periods: 4,
            prime_timeout_s: config.prime_timeout_s,
            target_frames,
            settle: SettleWatch::new(settle_wait(config), config.initial_drift_ppm),
            trace,
        })
    }

    fn prime(&self, mixer: &mut Mixer, metrics: &EngineMetrics, stop: &AtomicBool) {
        metrics.set_state(EngineState::Priming);
        let deadline = Instant::now() + Duration::from_secs_f64(self.prime_timeout_s);

        let mut step = 0usize;
        let mut last = 0usize;

        loop {
            mixer.sync();
            let fill = mixer.fill();
            step = step.max(fill.saturating_sub(last));
            last = fill;

            if fill as f64 >= self.target_frames + step as f64 / 2.0 {
                break;
            }
            if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                break;
            }

            metrics.publish(fill, fill as f64, 0.0, self.resampler.ratio(), 0, 0.0);
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn run(
        &mut self,
        mixer: &mut Mixer,
        metrics: &EngineMetrics,
        stop: &AtomicBool,
    ) -> Result<(), EngineError> {
        self.prime(mixer, metrics, stop);
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let discarded = mixer.trim(self.target_frames as usize);
        if discarded > 0 {
            metrics.record_discard(discarded);
        }

        self.sink.prefill_silence()?;
        self.sink.start()?;
        metrics.set_running();

        let mut period = 0usize;
        while !stop.load(Ordering::Relaxed) {
            let need = self.resampler.input_frames_next();
            let samples = need * self.channels;
            if samples > self.pull_buf.len() {
                return Err(EngineError::Config(format!(
                    "重采样器索取 {need} 帧，超出预分配的 {} 帧",
                    self.pull_buf.len() / self.channels
                )));
            }

            let mix = mixer.pull(
                &mut self.pull_buf[..samples],
                self.dt_s,
                self.target_frames as usize,
            );
            self.resampler
                .process(&self.pull_buf[..samples], &mut self.out_buf)?;

            let report = self.sink.write(&self.out_buf)?;

            if mix.limit == RingLimit::Dry {
                metrics.record_host_silence();
            }
            period += 1;
            if period > self.warmup_periods {
                self.ctrl.update(mix.fill as f64, self.dt_s, mix.limit);
                self.resampler.set_ratio(self.ctrl.ratio());
            }

            let elapsed = period as f64 * self.dt_s;
            let drift = self.ctrl.drift_ppm();
            metrics.publish(
                mix.fill,
                self.ctrl.smoothed_fill(),
                drift,
                self.resampler.ratio(),
                self.resampler.clamp_events(),
                elapsed,
            );
            metrics.set_settled(self.settle.update(drift, elapsed));
            metrics.publish_device(report.queued_frames, report.thinnest_frames, report.starved);
        }

        self.sink.stop()?;
        Ok(())
    }
}

impl<S: AudioSink> std::fmt::Debug for Worker<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("format", &self.format)
            .field("period_frames", &self.period_frames)
            .field("target_frames", &self.target_frames)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(target_ms: f64, max_block_frames: usize) -> EngineConfig {
        EngineConfig {
            source_rate_hz: 48_000.0,
            max_block_frames,
            target_ms,
            ..Default::default()
        }
    }

    fn level(
        target_ms: f64,
        block: usize,
        period: usize,
        capacity: usize,
        downstream_ms: f64,
    ) -> TargetLevel {
        target_level(&cfg(target_ms, block), period, capacity, downstream_ms)
    }

    #[test]
    fn the_target_follows_the_requested_milliseconds() {
        assert_eq!(level(30.0, 128, 480, 16_384, 0.0).frames, 1_440.0);
        assert_eq!(level(50.0, 128, 480, 16_384, 0.0).frames, 2_400.0);
        assert_eq!(level(120.0, 128, 480, 32_768, 0.0).frames, 5_760.0);
    }

    #[test]
    fn the_device_and_resampler_come_out_of_the_ring() {
        assert_eq!(level(50.0, 128, 480, 16_384, 20.0).frames, 1_440.0);
        assert_eq!(level(80.0, 128, 480, 16_384, 20.0).frames, 2_880.0);
        assert_eq!(level(120.0, 128, 480, 32_768, 22.5).frames, 4_680.0);
    }

    #[test]
    fn the_floor_it_reports_is_what_the_user_can_actually_ask_for() {
        let l = level(30.0, 512, 480, 16_384, 20.0);
        assert_eq!(l.floor, 992);
        assert!((l.total_floor_ms - (992.0 * 1.0e3 / 48_000.0 + 20.0)).abs() < 1.0e-9);
    }

    #[test]
    fn the_target_never_dips_below_one_block_plus_a_period() {
        assert_eq!(min_target_frames(512, 480), 992);
        assert_eq!(level(30.0, 512, 480, 16_384, 0.0).frames, 1_440.0);
        assert_eq!(level(10.0, 512, 480, 16_384, 0.0).frames, 992.0);
        assert_eq!(level(10.0, 128, 480, 16_384, 0.0).frames, 608.0);
        assert_eq!(level(1.0, 64, 64, 16_384, 0.0).frames, 128.0);
        assert_eq!(level(50.0, 512, 480, 16_384, 40.0).frames, 992.0);
    }

    #[test]
    fn the_target_never_eats_the_overrun_headroom() {
        assert_eq!(level(500.0, 512, 480, 16_384, 0.0).frames, 8_192.0);
        let l = level(30.0, 65_536, 480, 2_048, 0.0);
        assert_eq!((l.frames, l.floor), (1_024.0, 1_024));
    }

    #[test]
    fn a_steady_reading_settles_long_before_the_deadline() {
        let mut watch = SettleWatch::new(120.0, 0.0);
        let mut elapsed = 0.0;
        let mut settled_at = None;
        while elapsed < 120.0 {
            elapsed += 0.01;
            if watch.update(-133.5, elapsed) {
                settled_at = Some(elapsed);
                break;
            }
        }
        let at = settled_at.expect("稳定的读数应当在截止前判定收敛");
        assert!(at < 20.0, "收敛判定太晚: {at}");
    }

    #[test]
    fn a_reading_that_keeps_moving_waits_for_the_deadline() {
        let mut watch = SettleWatch::new(30.0, 0.0);
        let mut elapsed = 0.0;
        let mut ppm = 0.0;
        while elapsed < 29.0 {
            elapsed += 0.01;
            ppm += 0.5;
            assert!(!watch.update(ppm, elapsed), "还在移动就不该判定收敛");
        }
        assert!(watch.update(ppm, 30.0), "过了截止时间仍应放行");
    }

    #[test]
    fn it_never_calls_it_settled_in_the_opening_seconds() {
        let mut watch = SettleWatch::new(120.0, -100.0);
        let mut elapsed = 0.0;
        while elapsed < 9.5 {
            elapsed += 0.01;
            assert!(!watch.update(-100.0, elapsed), "{elapsed} s 就判定收敛太早");
        }
    }

    #[test]
    fn capacity_leaves_room_above_the_target() {
        assert_eq!(ring_capacity_for_target(1_440), 8_192);
        assert_eq!(ring_capacity_for_target(240), 2_048);
        assert_eq!(ring_capacity_for_target(5_760), 32_768);
        for target in [96usize, 480, 1_440, 2_400, 5_760] {
            assert!(ring_capacity_for_target(target) >= 2 * target);
        }
    }

    #[test]
    fn a_remembered_drift_shortens_the_settling_wait() {
        let cold = cfg(30.0, 512);
        let warm = EngineConfig {
            initial_drift_ppm: -152.0,
            ..cfg(30.0, 512)
        };
        assert_eq!(settle_wait(&cold), cold.tuning.settle_time_s * 2.0);
        assert!(settle_wait(&warm) < settle_wait(&cold) / 3.0);
    }

    #[test]
    fn a_broken_target_is_refused_not_clamped() {
        assert!(validate(&cfg(0.0, 512)).is_err());
        assert!(validate(&cfg(f64::NAN, 512)).is_err());
        assert!(validate(&cfg(30.0, 512)).is_ok());
        assert!(validate(&EngineConfig {
            channels: 0,
            ..cfg(30.0, 512)
        })
        .is_err());
    }

    #[test]
    fn the_ring_is_sized_from_the_config_alone() {
        assert_eq!(ring_capacity_for(&cfg(30.0, 512)), 8_192);
        assert_eq!(ring_capacity_for(&cfg(120.0, 512)), 32_768);
        assert_eq!(ring_capacity_for(&cfg(1.0, 16_384)), 65_536);
    }

    #[test]
    fn backoff_grows_then_caps() {
        let delays: Vec<u64> = (1..=10)
            .map(|n| backoff_delay(n).as_millis() as u64)
            .collect();

        assert_eq!(delays[0], 250);
        for pair in delays.windows(2) {
            assert!(pair[1] >= pair[0], "退避出现了回退: {delays:?}");
        }
        assert!(
            delays.iter().all(|&d| d <= 5_000),
            "退避超过了 5 秒上限: {delays:?}"
        );
        assert_eq!(*delays.last().unwrap(), 5_000);
    }

    #[test]
    fn backoff_handles_zero_attempt() {
        assert_eq!(backoff_delay(0).as_millis(), 250);
    }

    #[test]
    fn sleep_returns_at_once_when_stopped() {
        let stop = AtomicBool::new(true);
        let started = Instant::now();
        sleep_interruptible(Duration::from_secs(5), &stop);
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "已请求停止，睡眠仍持续了 {:?}",
            started.elapsed()
        );
    }
}
