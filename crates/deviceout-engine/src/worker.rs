use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use deviceout_core::{DriftController, DriftResampler, DriftTuning, RingConsumer};
use deviceout_sink::wasapi::{list_output_devices, ComGuard};
use deviceout_sink::{AudioPriority, AudioSink, StreamFormat, WasapiSink};

use crate::error::EngineError;
use crate::metrics::{EngineMetrics, EngineState};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub device_id: String,
    pub source_rate_hz: f64,
    pub channels: usize,
    pub device_buffer_ms: u32,
    pub prime_timeout_s: f64,
    pub tuning: DriftTuning,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            source_rate_hz: 48_000.0,
            channels: 2,
            device_buffer_ms: 40,
            prime_timeout_s: 5.0,
            tuning: DriftTuning::default(),
        }
    }
}

pub fn ring_capacity_frames(source_rate_hz: f64, ring_ms: f64) -> usize {
    ((source_rate_hz * ring_ms * 1.0e-3).round() as usize).max(2)
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

pub fn start(consumer: RingConsumer, config: EngineConfig) -> EngineHandle {
    let metrics = Arc::new(EngineMetrics::new(Arc::clone(consumer.stats())));
    let stop = Arc::new(AtomicBool::new(false));

    if let Err(e) = preflight(&consumer, &config) {
        metrics.set_failed(e.to_string());
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
            .spawn(move || run(config, consumer, metrics, stop, ready_tx))
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
            metrics.set_failed(format!("无法创建输出线程: {e}"));
            EngineHandle {
                metrics,
                stop,
                thread: None,
            }
        }
    }
}

fn preflight(consumer: &RingConsumer, config: &EngineConfig) -> Result<(), EngineError> {
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

    let _com = ComGuard::new()?;
    let device = list_output_devices()?
        .into_iter()
        .find(|d| d.id == config.device_id)
        .ok_or_else(|| {
            EngineError::Sink(deviceout_sink::SinkError::DeviceNotFound(
                config.device_id.clone(),
            ))
        })?;

    let device_channels = device.mix_format.channels as usize;
    if device_channels != config.channels {
        return Err(EngineError::ChannelMismatch {
            device: device_channels,
            source: config.channels,
        });
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

fn run(
    config: EngineConfig,
    mut rx: RingConsumer,
    metrics: Arc<EngineMetrics>,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<()>,
) -> RingConsumer {
    let _priority = AudioPriority::raise_current_thread().ok();

    let mut ready = Some(ready);
    let mut saw_device_loss = false;
    let mut attempt = 0u32;

    while !stop.load(Ordering::Relaxed) {
        match Worker::open(&config, &mut rx, &metrics) {
            Ok(mut worker) => {
                if saw_device_loss {
                    let discarded = worker.trim_backlog(&mut rx);
                    metrics.record_reconnect(discarded);
                    attempt = 0;
                }
                signal_ready(&mut ready);

                match worker.run(&mut rx, &metrics, &stop) {
                    Ok(()) => {
                        metrics.set_state(EngineState::Stopped);
                        return rx;
                    }
                    Err(e) if e.is_recoverable() => {
                        metrics.set_reconnecting(e.to_string());
                        saw_device_loss = true;
                    }
                    Err(e) => {
                        metrics.set_failed(e.to_string());
                        return rx;
                    }
                }
            }

            Err(e) if saw_device_loss && e.is_recoverable() => {
                attempt = attempt.saturating_add(1);
                metrics.set_reconnecting(format!("{e}（第 {attempt} 次）"));
            }

            Err(e) => {
                metrics.set_failed(e.to_string());
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

struct Worker {
    sink: WasapiSink,
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

impl Worker {
    fn open(
        config: &EngineConfig,
        rx: &mut RingConsumer,
        metrics: &EngineMetrics,
    ) -> Result<Self, EngineError> {
        let sink = WasapiSink::open(&config.device_id, config.device_buffer_ms)?;
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
        let target_frames = (rx.capacity_frames() / 2) as f64;
        let nominal_ratio = sink_rate / config.source_rate_hz;

        let ctrl = DriftController::new(target_frames, sink_rate, nominal_ratio, config.tuning);
        let resampler =
            DriftResampler::new(channels, period_frames, nominal_ratio, &config.tuning)?;

        let pull_buf = vec![0.0f32; resampler.input_frames_max() * channels];
        let out_buf = vec![0.0f32; resampler.output_frames() * channels];

        metrics.set_stream(sink_rate, period_frames, channels, rx.capacity_frames());
        metrics.set_target(target_frames, config.tuning.settle_time_s * 3.0);

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

        self.sink.prefill_silence()?;
        self.sink.start()?;
        metrics.set_running();
        metrics.set_sink_latency(self.sink.stream_latency_ms().unwrap_or(0.0));

        let mut period = 0usize;
        while !stop.load(Ordering::Relaxed) {
            let need = self.resampler.input_frames_next();
            let samples = need * self.channels;
            debug_assert!(samples <= self.pull_buf.len());

            rx.pull(&mut self.pull_buf[..samples]);
            self.resampler
                .process(&self.pull_buf[..samples], &mut self.out_buf)?;

            self.sink.write(&self.out_buf)?;

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
        }

        self.sink.stop()?;
        Ok(())
    }
}

impl std::fmt::Debug for Worker {
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
