use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use deviceout_core::BridgeStats;

use crate::error::Fault;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Priming,
    Running,
    Reconnecting,
    Stopped,
    Failed,
}

impl EngineState {
    fn code(self) -> u8 {
        match self {
            Self::Priming => 0,
            Self::Running => 1,
            Self::Stopped => 2,
            Self::Failed => 3,
            Self::Reconnecting => 4,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Priming,
            1 => Self::Running,
            2 => Self::Stopped,
            4 => Self::Reconnecting,
            _ => Self::Failed,
        }
    }
}

fn store_f64(slot: &AtomicU64, v: f64) {
    slot.store(v.to_bits(), Ordering::Relaxed);
}

fn load_f64(slot: &AtomicU64) -> f64 {
    f64::from_bits(slot.load(Ordering::Relaxed))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StreamInfo {
    pub source_rate_hz: f64,
    pub sink_rate_hz: f64,
    pub period_frames: usize,
    pub channels: usize,
    pub capacity_frames: usize,
    pub resampler_delay_frames: usize,
    pub min_period_frames: usize,
    pub exclusive: bool,
}

#[derive(Debug)]
pub struct EngineMetrics {
    stats: Arc<BridgeStats>,

    state: AtomicU8,
    fill_frames: AtomicU64,
    capacity_frames: AtomicU64,
    target_frames: AtomicU64,
    min_target_frames: AtomicU64,
    floor_ms: AtomicU64,
    exclusive: AtomicBool,
    smoothed_fill: AtomicU64,
    drift_ppm: AtomicU64,
    ratio: AtomicU64,
    clamp_events: AtomicU64,
    source_rate_hz: AtomicU64,
    sink_rate_hz: AtomicU64,
    period_frames: AtomicU64,
    min_period_frames: AtomicU64,
    channels: AtomicU64,
    resampler_delay_frames: AtomicU64,
    device_queue_frames: AtomicU64,
    device_starvations: AtomicU64,
    device_queue_low: AtomicU64,
    host_silence: AtomicU64,
    running_seconds: AtomicU64,
    fill_low: AtomicU64,
    fill_high: AtomicU64,
    settled_after_s: AtomicU64,
    settled: AtomicBool,
    reconnects: AtomicU64,
    frames_discarded: AtomicU64,

    last_error: Mutex<Option<Fault>>,
}

pub fn latency_ms(
    fill_frames: f64,
    source_rate_hz: f64,
    device_queue_frames: f64,
    resampler_delay_frames: f64,
    sink_rate_hz: f64,
) -> f64 {
    let ring = if source_rate_hz > 0.0 {
        fill_frames / source_rate_hz
    } else {
        0.0
    };
    let device = if sink_rate_hz > 0.0 {
        (device_queue_frames + resampler_delay_frames) / sink_rate_hz
    } else {
        0.0
    };
    (ring + device) * 1000.0
}

impl EngineMetrics {
    pub(crate) fn new() -> Self {
        Self {
            stats: Arc::new(BridgeStats::default()),
            state: AtomicU8::new(EngineState::Priming.code()),
            fill_frames: AtomicU64::new(0),
            capacity_frames: AtomicU64::new(0),
            target_frames: AtomicU64::new(0),
            min_target_frames: AtomicU64::new(0),
            floor_ms: AtomicU64::new(0),
            exclusive: AtomicBool::new(false),
            smoothed_fill: AtomicU64::new(0),
            drift_ppm: AtomicU64::new(0),
            ratio: AtomicU64::new(1.0f64.to_bits()),
            clamp_events: AtomicU64::new(0),
            source_rate_hz: AtomicU64::new(0),
            sink_rate_hz: AtomicU64::new(0),
            period_frames: AtomicU64::new(0),
            min_period_frames: AtomicU64::new(0),
            channels: AtomicU64::new(1),
            resampler_delay_frames: AtomicU64::new(0),
            device_queue_frames: AtomicU64::new(0),
            device_starvations: AtomicU64::new(0),
            device_queue_low: AtomicU64::new(u64::MAX),
            host_silence: AtomicU64::new(0),
            running_seconds: AtomicU64::new(0),
            fill_low: AtomicU64::new(u64::MAX),
            fill_high: AtomicU64::new(0),
            settled_after_s: AtomicU64::new(f64::INFINITY.to_bits()),
            settled: AtomicBool::new(false),
            reconnects: AtomicU64::new(0),
            frames_discarded: AtomicU64::new(0),
            last_error: Mutex::new(None),
        }
    }

    pub(crate) fn set_running(&self) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = None;
        }
        self.set_state(EngineState::Running);
    }

    pub(crate) fn set_state(&self, state: EngineState) {
        self.state.store(state.code(), Ordering::Relaxed);
    }

    pub(crate) fn set_reconnecting(&self, fault: Fault) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(fault);
        }
        self.set_state(EngineState::Reconnecting);
    }

    pub(crate) fn record_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_discard(&self, frames: usize) {
        self.frames_discarded
            .fetch_add(frames as u64, Ordering::Relaxed);
    }

    pub(crate) fn set_failed(&self, fault: Fault) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(fault);
        }
        self.set_state(EngineState::Failed);
    }

    pub(crate) fn set_stream(&self, stream: StreamInfo) {
        store_f64(&self.source_rate_hz, stream.source_rate_hz);
        store_f64(&self.sink_rate_hz, stream.sink_rate_hz);
        self.period_frames
            .store(stream.period_frames as u64, Ordering::Relaxed);
        self.min_period_frames
            .store(stream.min_period_frames as u64, Ordering::Relaxed);
        self.channels
            .store(stream.channels as u64, Ordering::Relaxed);
        self.capacity_frames
            .store(stream.capacity_frames as u64, Ordering::Relaxed);
        self.resampler_delay_frames
            .store(stream.resampler_delay_frames as u64, Ordering::Relaxed);
        self.exclusive.store(stream.exclusive, Ordering::Relaxed);
    }

    pub(crate) fn set_target(
        &self,
        target_frames: f64,
        min_target_frames: usize,
        floor_ms: f64,
        settled_after_s: f64,
    ) {
        store_f64(&self.target_frames, target_frames);
        self.min_target_frames
            .store(min_target_frames as u64, Ordering::Relaxed);
        store_f64(&self.floor_ms, floor_ms);
        store_f64(&self.settled_after_s, settled_after_s);
    }

    pub(crate) fn publish_device(&self, queued_frames: usize, thinnest: usize, starved: bool) {
        self.device_queue_frames
            .store(queued_frames as u64, Ordering::Relaxed);
        self.device_queue_low
            .fetch_min(thinnest as u64, Ordering::Relaxed);
        if starved {
            self.device_starvations.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn take_device_queue_low(&self) -> u64 {
        let low = self.device_queue_low.swap(u64::MAX, Ordering::Relaxed);
        if low == u64::MAX {
            0
        } else {
            low
        }
    }

    pub(crate) fn publish(
        &self,
        fill_frames: usize,
        smoothed_fill: f64,
        drift_ppm: f64,
        ratio: f64,
        clamp_events: u64,
        running_seconds: f64,
    ) {
        self.fill_frames
            .store(fill_frames as u64, Ordering::Relaxed);
        self.fill_low
            .fetch_min(fill_frames as u64, Ordering::Relaxed);
        self.fill_high
            .fetch_max(fill_frames as u64, Ordering::Relaxed);
        store_f64(&self.smoothed_fill, smoothed_fill);
        store_f64(&self.drift_ppm, drift_ppm);
        store_f64(&self.ratio, ratio);
        self.clamp_events.store(clamp_events, Ordering::Relaxed);
        store_f64(&self.running_seconds, running_seconds);
    }

    pub fn stats(&self) -> &Arc<BridgeStats> {
        &self.stats
    }

    pub fn state(&self) -> EngineState {
        EngineState::from_code(self.state.load(Ordering::Relaxed))
    }

    pub fn last_error(&self) -> Option<Fault> {
        self.last_error.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn fill_frames(&self) -> u64 {
        self.fill_frames.load(Ordering::Relaxed)
    }

    pub fn take_fill_range(&self) -> (u64, u64) {
        let low = self.fill_low.swap(u64::MAX, Ordering::Relaxed);
        let high = self.fill_high.swap(0, Ordering::Relaxed);
        (low.min(high), high)
    }

    pub fn capacity_frames(&self) -> u64 {
        self.capacity_frames.load(Ordering::Relaxed)
    }

    pub fn target_frames(&self) -> f64 {
        load_f64(&self.target_frames)
    }

    pub fn exclusive(&self) -> bool {
        self.exclusive.load(Ordering::Relaxed)
    }

    pub fn min_target_frames(&self) -> u64 {
        self.min_target_frames.load(Ordering::Relaxed)
    }

    pub fn floor_ms(&self) -> f64 {
        load_f64(&self.floor_ms)
    }

    pub fn target_fraction(&self) -> f64 {
        let cap = self.capacity_frames();
        if cap == 0 {
            return 0.0;
        }
        self.target_frames() / cap as f64
    }

    pub fn smoothed_fill(&self) -> f64 {
        load_f64(&self.smoothed_fill)
    }

    pub fn fill_fraction(&self) -> f64 {
        let cap = self.capacity_frames();
        if cap == 0 {
            return 0.0;
        }
        self.smoothed_fill() / cap as f64
    }

    pub fn drift_ppm(&self) -> f64 {
        load_f64(&self.drift_ppm)
    }

    pub(crate) fn set_settled(&self, settled: bool) {
        self.settled.store(settled, Ordering::Relaxed);
    }

    pub fn running_seconds(&self) -> f64 {
        load_f64(&self.running_seconds)
    }

    pub fn is_settled(&self) -> bool {
        self.state() == EngineState::Running && self.settled.load(Ordering::Relaxed)
    }

    pub fn drift_ppm_settled(&self) -> Option<f64> {
        self.is_settled().then(|| self.drift_ppm())
    }

    pub fn ratio(&self) -> f64 {
        load_f64(&self.ratio)
    }

    pub fn clamp_events(&self) -> u64 {
        self.clamp_events.load(Ordering::Relaxed)
    }

    pub fn reconnects(&self) -> u64 {
        self.reconnects.load(Ordering::Relaxed)
    }

    pub fn frames_discarded(&self) -> u64 {
        self.frames_discarded.load(Ordering::Relaxed)
    }

    pub fn source_rate_hz(&self) -> f64 {
        load_f64(&self.source_rate_hz)
    }

    pub fn sink_rate_hz(&self) -> f64 {
        load_f64(&self.sink_rate_hz)
    }

    pub fn device_queue_frames(&self) -> u64 {
        self.device_queue_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn record_host_silence(&self) {
        self.host_silence.fetch_add(1, Ordering::Relaxed);
    }

    pub fn host_silence(&self) -> u64 {
        self.host_silence.load(Ordering::Relaxed)
    }

    pub fn device_starvations(&self) -> u64 {
        self.device_starvations.load(Ordering::Relaxed)
    }

    pub fn latency_ms(&self) -> f64 {
        latency_ms(
            self.smoothed_fill(),
            self.source_rate_hz(),
            self.device_queue_frames() as f64,
            self.resampler_delay_frames.load(Ordering::Relaxed) as f64,
            self.sink_rate_hz(),
        )
    }

    pub fn min_period_frames(&self) -> u64 {
        self.min_period_frames.load(Ordering::Relaxed)
    }

    pub fn period_frames(&self) -> u64 {
        self.period_frames.load(Ordering::Relaxed)
    }

    pub fn channels(&self) -> u64 {
        self.channels.load(Ordering::Relaxed)
    }

    pub fn dropout_seconds(&self) -> f64 {
        let rate = self.sink_rate_hz();
        let channels = self.channels().max(1);
        if rate <= 0.0 {
            return 0.0;
        }
        self.stats.samples_zero_filled() as f64 / channels as f64 / rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> EngineMetrics {
        EngineMetrics::new()
    }

    fn lost(detail: &str) -> Fault {
        Fault {
            kind: crate::FaultKind::DeviceLost,
            detail: detail.into(),
            attempt: 0,
        }
    }

    #[test]
    fn recovering_clears_the_stale_error() {
        let m = metrics();

        m.set_reconnecting(lost("设备已失效：被拔出"));
        assert_eq!(m.state(), EngineState::Reconnecting);
        assert!(m.last_error().is_some());

        m.set_running();
        assert_eq!(m.state(), EngineState::Running);
        assert!(
            m.last_error().is_none(),
            "重连成功后仍残留故障原因：{:?}",
            m.last_error()
        );
    }

    #[test]
    fn reconnect_count_survives_recovery() {
        let m = metrics();

        m.set_reconnecting(lost("设备失效"));
        m.record_reconnect();
        m.record_discard(1234);
        m.set_running();

        assert_eq!(m.reconnects(), 1);
        assert_eq!(m.frames_discarded(), 1234);
    }

    #[test]
    fn latency_uses_each_side_in_its_own_clock() {
        let ms = latency_ms(8192.0, 44_100.0, 1920.0, 128.0, 48_000.0);
        let expected = (8192.0 / 44_100.0 + (1920.0 + 128.0) / 48_000.0) * 1000.0;
        assert!((ms - expected).abs() < 1e-9);
        assert_eq!(latency_ms(1000.0, 0.0, 0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn device_starvation_is_counted_per_event() {
        let m = metrics();
        m.publish_device(1920, 960, false);
        m.publish_device(480, 0, true);
        m.publish_device(960, 0, true);
        assert_eq!(m.device_queue_frames(), 960);
        assert_eq!(m.device_starvations(), 2);
    }

    #[test]
    fn failure_keeps_its_reason() {
        let m = metrics();
        let fault = crate::EngineError::ChannelMismatch {
            device: 8,
            source: 2,
        }
        .fault();
        m.set_failed(fault.clone());

        assert_eq!(m.state(), EngineState::Failed);
        assert_eq!(m.last_error(), Some(fault));
        assert_eq!(
            m.last_error().unwrap().kind,
            crate::FaultKind::ChannelMismatch {
                device: 8,
                source: 2
            }
        );
    }
}
