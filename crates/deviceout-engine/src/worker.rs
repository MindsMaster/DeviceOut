use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use deviceout_core::{DriftController, DriftResampler, DriftTuning, RingConsumer};
use deviceout_sink::{AudioSink, SinkError, StreamFormat};

use crate::error::{EngineError, Fault};
use crate::metrics::{EngineMetrics, EngineState, StreamInfo};

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
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            source_rate_hz: 48_000.0,
            channels: 2,
            max_block_frames: DEFAULT_BLOCK_FRAMES,
            target_ms: DEFAULT_TARGET_MS,
            device_buffer_ms: 40,
            device_queue_periods: deviceout_sink::MIN_QUEUE_PERIODS,
            exclusive: false,
            initial_drift_ppm: 0.0,
            prime_timeout_s: 5.0,
            tuning: DriftTuning::default(),
        }
    }
}

pub const DEFAULT_TARGET_MS: f64 = 30.0;
pub const DEFAULT_BLOCK_FRAMES: usize = 512;

const CAPACITY_FACTOR: usize = 4;
const MIN_CAPACITY_FRAMES: usize = 2_048;
const PERIOD_MARGIN: usize = 2;
const SETTLE_COLD: f64 = 3.0;
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

pub fn ring_capacity_for_target(target_frames: usize) -> usize {
    target_frames
        .saturating_mul(CAPACITY_FACTOR)
        .max(MIN_CAPACITY_FRAMES)
        .next_power_of_two()
}

fn settle_wait(config: &EngineConfig) -> f64 {
    let scale = if config.initial_drift_ppm == 0.0 {
        SETTLE_COLD
    } else {
        SETTLE_WARM
    };
    config.tuning.settle_time_s * scale
}

fn target_level(config: &EngineConfig, period_frames: usize, capacity_frames: usize) -> (f64, usize) {
    let ceiling = (capacity_frames / 2).max(1);
    let floor = min_target_frames(config.max_block_frames, period_frames).min(ceiling);
    let target = frames_for_ms(config.target_ms, config.source_rate_hz).clamp(floor, ceiling);
    (target as f64, floor)
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
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<RingConsumer>>,
}

impl EngineHandle {
    pub fn metrics(&self) -> &Arc<EngineMetrics> {
        &self.metrics
    }

    pub fn stop(&mut self) -> Option<RingConsumer> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().and_then(|h| h.join().ok())
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
pub fn start(consumer: RingConsumer, config: EngineConfig) -> EngineHandle {
    start_with(consumer, config, |cfg: &EngineConfig| {
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

pub fn start_with<O: OpenSink>(consumer: RingConsumer, config: EngineConfig, open: O) -> EngineHandle {
    let metrics = Arc::new(EngineMetrics::new(Arc::clone(consumer.stats())));
    let stop = Arc::new(AtomicBool::new(false));

    if let Err(e) = validate(&consumer, &config) {
        metrics.set_failed(e.fault());
        return EngineHandle {
            metrics,
            stop,
            thread: Some(spawn_idle(consumer)),
        };
    }

    let (ready_tx, ready_rx) = mpsc::channel::<()>();

    let thread = {
        let metrics = Arc::clone(&metrics);
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("deviceout-output".into())
            .spawn(move || run(config, open, consumer, metrics, stop, ready_tx))
    };

    match thread {
        Ok(thread) => {
            let _ = ready_rx.recv();
            EngineHandle {
                metrics,
                stop,
                thread: Some(thread),
            }
        }
        Err(e) => {
            metrics.set_failed(Fault::thread(format!("无法创建输出线程: {e}")));
            EngineHandle {
                metrics,
                stop,
                thread: None,
            }
        }
    }
}

fn validate(consumer: &RingConsumer, config: &EngineConfig) -> Result<(), EngineError> {
    if config.channels == 0 {
        return Err(EngineError::Config("声道数必须为正".into()));
    }
    if consumer.channels() != config.channels {
        return Err(EngineError::Config(format!(
            "环形缓冲是 {} 声道，配置写的是 {} 声道",
            consumer.channels(),
            config.channels
        )));
    }
    if !config.source_rate_hz.is_finite() || config.source_rate_hz <= 0.0 {
        return Err(EngineError::Config("DAW 侧采样率必须是有限正数".into()));
    }
    if !config.target_ms.is_finite() || config.target_ms <= 0.0 {
        return Err(EngineError::Config("目标延迟必须是有限正数".into()));
    }
    Ok(())
}

fn spawn_idle(consumer: RingConsumer) -> JoinHandle<RingConsumer> {
    thread::spawn(move || consumer)
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
    mut rx: RingConsumer,
    metrics: Arc<EngineMetrics>,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<()>,
) -> RingConsumer {
    #[cfg(windows)]
    let _priority = deviceout_sink::AudioPriority::raise_current_thread().ok();

    let mut ready = Some(ready);
    let mut saw_device_loss = false;
    let mut attempt = 0u32;

    while !stop.load(Ordering::Relaxed) {
        let opened = open
            .open(&config)
            .map_err(EngineError::from)
            .and_then(|sink| Worker::new(sink, &config, &rx, &metrics));
        match opened {
            Ok(mut worker) => {
                if saw_device_loss {
                    metrics.record_reconnect();
                    attempt = 0;
                }
                signal_ready(&mut ready);

                match worker.run(&mut rx, &metrics, &stop) {
                    Ok(()) => {
                        metrics.set_state(EngineState::Stopped);
                        return rx;
                    }
                    Err(e) if e.is_recoverable() => {
                        metrics.set_reconnecting(e.fault());
                        saw_device_loss = true;
                    }
                    Err(e) => {
                        metrics.set_failed(e.fault());
                        return rx;
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
                return rx;
            }
        }

        if saw_device_loss {
            signal_ready(&mut ready);
            sleep_interruptible(backoff_delay(attempt), &stop);
        }
    }

    metrics.set_state(EngineState::Stopped);
    rx
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
}

impl<S: AudioSink> Worker<S> {
    fn new(
        sink: S,
        config: &EngineConfig,
        rx: &RingConsumer,
        metrics: &EngineMetrics,
    ) -> Result<Self, EngineError> {
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
        let (target_frames, min_target) =
            target_level(config, period_source_frames, rx.capacity_frames());

        let mut ctrl = DriftController::new(target_frames, sink_rate, nominal_ratio, config.tuning);
        ctrl.seed_drift_ppm(config.initial_drift_ppm);
        let resampler =
            DriftResampler::new(channels, period_frames, nominal_ratio, &config.tuning)?;

        let pull_buf = vec![0.0f32; resampler.input_frames_max() * channels];
        let out_buf = vec![0.0f32; resampler.output_frames() * channels];

        metrics.set_stream(StreamInfo {
            source_rate_hz: config.source_rate_hz,
            sink_rate_hz: sink_rate,
            period_frames,
            channels,
            capacity_frames: rx.capacity_frames(),
            resampler_delay_frames: resampler.output_delay(),
            exclusive: sink.exclusive(),
        });
        metrics.set_target(target_frames, min_target, settle_wait(config));

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
        })
    }

    fn trim_backlog(&self, rx: &mut RingConsumer) -> usize {
        let fill = rx.available_frames();
        let target = self.target_frames as usize;
        if fill <= target {
            return 0;
        }
        rx.discard(fill - target)
    }

    fn prime(&self, rx: &RingConsumer, metrics: &EngineMetrics, stop: &AtomicBool) {
        metrics.set_state(EngineState::Priming);
        let deadline = Instant::now() + Duration::from_secs_f64(self.prime_timeout_s);

        let mut step = 0usize;
        let mut last = rx.available_frames();

        loop {
            let fill = rx.available_frames();
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
        rx: &mut RingConsumer,
        metrics: &EngineMetrics,
        stop: &AtomicBool,
    ) -> Result<(), EngineError> {
        self.prime(rx, metrics, stop);
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let discarded = self.trim_backlog(rx);
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

            rx.pull(&mut self.pull_buf[..samples]);
            self.resampler
                .process(&self.pull_buf[..samples], &mut self.out_buf)?;

            let report = self.sink.write(&self.out_buf)?;

            let fill = rx.available_frames();
            period += 1;
            if period > self.warmup_periods {
                self.ctrl.update(fill as f64, self.dt_s);
                self.resampler.set_ratio(self.ctrl.ratio());
            }

            metrics.publish(
                fill,
                self.ctrl.smoothed_fill(),
                self.ctrl.drift_ppm(),
                self.resampler.ratio(),
                self.resampler.clamp_events(),
                period as f64 * self.dt_s,
            );
            metrics.publish_device(report.queued_frames, report.starved);
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

    #[test]
    fn the_target_follows_the_requested_milliseconds() {
        assert_eq!(target_level(&cfg(30.0, 128), 480, 16_384).0, 1_440.0);
        assert_eq!(target_level(&cfg(50.0, 128), 480, 16_384).0, 2_400.0);
        assert_eq!(target_level(&cfg(120.0, 128), 480, 32_768).0, 5_760.0);
    }

    #[test]
    fn the_target_never_dips_below_one_block_plus_two_periods() {
        assert_eq!(min_target_frames(512, 480), 1_472);
        assert_eq!(target_level(&cfg(30.0, 512), 480, 16_384), (1_472.0, 1_472));
        assert_eq!(target_level(&cfg(10.0, 512), 480, 16_384), (1_472.0, 1_472));
        assert_eq!(target_level(&cfg(10.0, 128), 480, 16_384), (1_088.0, 1_088));
        assert_eq!(target_level(&cfg(1.0, 64), 64, 16_384), (192.0, 192));
    }

    #[test]
    fn the_target_never_eats_the_overrun_headroom() {
        assert_eq!(target_level(&cfg(500.0, 512), 480, 16_384).0, 8_192.0);
        assert_eq!(target_level(&cfg(30.0, 65_536), 480, 2_048), (1_024.0, 1_024));
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
        assert_eq!(settle_wait(&cold), cold.tuning.settle_time_s * 3.0);
        assert!(settle_wait(&warm) < settle_wait(&cold) / 4.0);
    }

    #[test]
    fn a_broken_target_is_refused_not_clamped() {
        let (_tx, rx) = deviceout_core::ring(4_096, 2);
        assert!(validate(&rx, &cfg(0.0, 512)).is_err());
        assert!(validate(&rx, &cfg(f64::NAN, 512)).is_err());
        assert!(validate(&rx, &cfg(30.0, 512)).is_ok());
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
