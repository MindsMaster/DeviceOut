use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use deviceout_core::BridgeStats;

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

impl std::fmt::Display for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Priming => write!(f, "预填充"),
            Self::Running => write!(f, "运行中"),
            Self::Stopped => write!(f, "已停止"),
            Self::Failed => write!(f, "出错"),
            Self::Reconnecting => write!(f, "重连中"),
        }
    }
}

fn store_f64(slot: &AtomicU64, v: f64) {
    slot.store(v.to_bits(), Ordering::Relaxed);
}

fn load_f64(slot: &AtomicU64) -> f64 {
    f64::from_bits(slot.load(Ordering::Relaxed))
}

#[derive(Debug)]
pub struct EngineMetrics {
    stats: Arc<BridgeStats>,

    state: AtomicU8,
    fill_frames: AtomicU64,
    capacity_frames: AtomicU64,
    target_frames: AtomicU64,
    smoothed_fill: AtomicU64,
    drift_ppm: AtomicU64,
    ratio: AtomicU64,
    clamp_events: AtomicU64,
    sink_rate_hz: AtomicU64,
    period_frames: AtomicU64,
    channels: AtomicU64,
    running_seconds: AtomicU64,
    settled_after_s: AtomicU64,
    reconnects: AtomicU64,
    frames_discarded: AtomicU64,

    last_error: Mutex<Option<String>>,
}

impl EngineMetrics {
    pub(crate) fn new(stats: Arc<BridgeStats>) -> Self {
        Self {
            stats,
            state: AtomicU8::new(EngineState::Priming.code()),
            fill_frames: AtomicU64::new(0),
            capacity_frames: AtomicU64::new(0),
            target_frames: AtomicU64::new(0),
            smoothed_fill: AtomicU64::new(0),
            drift_ppm: AtomicU64::new(0),
            ratio: AtomicU64::new(1.0f64.to_bits()),
            clamp_events: AtomicU64::new(0),
            sink_rate_hz: AtomicU64::new(0),
            period_frames: AtomicU64::new(0),
            channels: AtomicU64::new(1),
            running_seconds: AtomicU64::new(0),
            settled_after_s: AtomicU64::new(f64::INFINITY.to_bits()),
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

    pub(crate) fn set_reconnecting(&self, message: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(message);
        }
        self.set_state(EngineState::Reconnecting);
    }

    pub(crate) fn record_reconnect(&self, discarded_frames: usize) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
        self.frames_discarded
            .fetch_add(discarded_frames as u64, Ordering::Relaxed);
    }

    pub(crate) fn set_failed(&self, message: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(message);
        }
        self.set_state(EngineState::Failed);
    }

    pub(crate) fn set_stream(
        &self,
        sink_rate_hz: f64,
        period_frames: usize,
        channels: usize,
        capacity: usize,
    ) {
        store_f64(&self.sink_rate_hz, sink_rate_hz);
        self.period_frames
            .store(period_frames as u64, Ordering::Relaxed);
        self.channels.store(channels as u64, Ordering::Relaxed);
        self.capacity_frames
            .store(capacity as u64, Ordering::Relaxed);
    }

    pub(crate) fn set_target(&self, target_frames: f64, settled_after_s: f64) {
        store_f64(&self.target_frames, target_frames);
        store_f64(&self.settled_after_s, settled_after_s);
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

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn fill_frames(&self) -> u64 {
        self.fill_frames.load(Ordering::Relaxed)
    }

    pub fn capacity_frames(&self) -> u64 {
        self.capacity_frames.load(Ordering::Relaxed)
    }

    pub fn target_frames(&self) -> f64 {
        load_f64(&self.target_frames)
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

    pub fn running_seconds(&self) -> f64 {
        load_f64(&self.running_seconds)
    }

    pub fn is_settled(&self) -> bool {
        self.state() == EngineState::Running
            && self.running_seconds() >= load_f64(&self.settled_after_s)
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

    pub fn sink_rate_hz(&self) -> f64 {
        load_f64(&self.sink_rate_hz)
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
        EngineMetrics::new(Arc::new(BridgeStats::default()))
    }

    #[test]
    fn recovering_clears_the_stale_error() {
        let m = metrics();

        m.set_reconnecting("设备已失效：被拔出".into());
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

        m.set_reconnecting("设备失效".into());
        m.record_reconnect(1234);
        m.set_running();

        assert_eq!(m.reconnects(), 1);
        assert_eq!(m.frames_discarded(), 1234);
    }

    #[test]
    fn failure_keeps_its_reason() {
        let m = metrics();
        m.set_failed("声道数不一致".into());

        assert_eq!(m.state(), EngineState::Failed);
        assert_eq!(m.last_error().as_deref(), Some("声道数不一致"));
    }
}
