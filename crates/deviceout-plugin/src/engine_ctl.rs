use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use deviceout_core::{ring, RingConsumer, RingProducer};
use deviceout_engine::{
    frames_for_ms, min_total_ms, ring_capacity_for_target, start, EngineConfig, EngineHandle,
    EngineMetrics,
};

pub(crate) const DEFAULT_TARGET_MS: u32 = 30;
pub(crate) const MAX_TARGET_MS: u32 = 1_000;
pub(crate) const TARGET_MS_STEPS: &[u32] = &[10, 20, 30, 50, 80, 120, 200];
pub(crate) const QUEUE_PERIOD_STEPS: &[u32] = &[1, 2, 3, 4];
pub(crate) const DEFAULT_QUEUE_PERIODS: u32 = 2;
const DRIFT_CARRY_SANITY_PPM: f64 = 20_000.0;

pub(crate) fn min_target_ms(max_block_frames: u32, source_rate_hz: f64, queue_periods: u32) -> u32 {
    let ms = min_total_ms(max_block_frames as usize, source_rate_hz, queue_periods);
    if !ms.is_finite() || ms <= 0.0 {
        return DEFAULT_TARGET_MS;
    }
    ms.ceil() as u32
}

pub(crate) fn clamp_target_ms(ms: u32, floor: u32) -> u32 {
    let floor = floor.max(1);
    ms.clamp(floor, MAX_TARGET_MS.max(floor))
}

#[derive(Debug, Default)]
struct Slot {
    consumer: Option<RingConsumer>,
    handle: Option<EngineHandle>,
    config: Option<EngineConfig>,
    running_device: String,
    carried_drift: Option<(String, f64)>,
}

#[derive(Debug, Default)]
pub(crate) struct EngineController {
    producer: Mutex<Option<RingProducer>>,
    slot: Mutex<Slot>,
    metrics: RwLock<Option<Arc<EngineMetrics>>>,
    target_ms: AtomicU32,
    target_floor_ms: AtomicU32,
    queue_periods: AtomicU32,
    exclusive: AtomicBool,
}

impl EngineController {
    pub(crate) fn producer(&self) -> &Mutex<Option<RingProducer>> {
        &self.producer
    }

    pub(crate) fn metrics(&self) -> Option<Arc<EngineMetrics>> {
        self.metrics.read().clone()
    }

    pub(crate) fn target_floor_ms(&self) -> u32 {
        let live = self.metrics().map_or(0.0, |m| m.floor_ms());
        if live.is_finite() && live > 0.0 {
            return live.ceil() as u32;
        }
        self.target_floor_ms.load(Ordering::Relaxed)
    }

    pub(crate) fn target_ms(&self) -> u32 {
        self.target_ms.load(Ordering::Relaxed)
    }

    pub(crate) fn queue_periods(&self) -> u32 {
        self.queue_periods.load(Ordering::Relaxed)
    }

    pub(crate) fn exclusive(&self) -> bool {
        self.exclusive.load(Ordering::Relaxed)
    }

    pub(crate) fn initialize(&self, config: EngineConfig, target_ms: u32, floor_ms: u32) -> u32 {
        let ms = clamp_target_ms(target_ms, floor_ms);
        self.target_floor_ms.store(floor_ms, Ordering::Relaxed);
        self.target_ms.store(ms, Ordering::Relaxed);
        self.queue_periods.store(
            clamp_queue_periods(config.device_queue_periods),
            Ordering::Relaxed,
        );
        self.exclusive.store(config.exclusive, Ordering::Relaxed);

        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        if let Some(mut handle) = self.retire(&mut slot) {
            handle.stop();
        }
        let config = EngineConfig {
            target_ms: f64::from(ms),
            ..config
        };
        let (tx, rx) = ring(capacity_for(&config), config.channels);
        *producer = Some(tx);
        let carried = slot.carried_drift.take();
        *slot = Slot {
            consumer: Some(rx),
            handle: None,
            config: Some(config),
            running_device: String::new(),
            carried_drift: carried,
        };
        self.restart(&mut producer, &mut slot);
        ms
    }

    pub(crate) fn deactivate(&self) {
        let mut slot = self.slot.lock();
        if let Some(mut handle) = self.retire(&mut slot) {
            slot.consumer = handle.stop();
        }
        *self.metrics.write() = None;
    }

    fn retire(&self, slot: &mut Slot) -> Option<EngineHandle> {
        let handle = slot.handle.take()?;
        self.remember_drift(&slot.running_device);
        if !slot.running_device.is_empty() {
            if let Some(ppm) = self.live_drift_ppm() {
                slot.carried_drift = Some((slot.running_device.clone(), ppm));
            }
        }
        slot.running_device.clear();
        Some(handle)
    }

    fn live_drift_ppm(&self) -> Option<f64> {
        let ppm = self.metrics()?.drift_ppm();
        (ppm.is_finite() && ppm.abs() <= DRIFT_CARRY_SANITY_PPM).then_some(ppm)
    }

    fn remember_drift(&self, device_id: &str) {
        let Some(metrics) = self.metrics() else {
            return;
        };
        let Some(ppm) = metrics.drift_ppm_settled() else {
            return;
        };
        deviceout_update::remember_drift(device_id, ppm, now_unix());
    }

    pub(crate) fn set_device_async(self: &Arc<Self>, device_id: String) {
        let ctl = Arc::clone(self);
        std::thread::Builder::new()
            .name("deviceout-switch".into())
            .spawn(move || ctl.set_device(device_id))
            .ok();
    }

    pub(crate) fn request_target_ms(self: &Arc<Self>, ms: u32) -> u32 {
        let ms = clamp_target_ms(ms, self.target_floor_ms());
        if ms == self.target_ms() {
            return ms;
        }
        self.target_ms.store(ms, Ordering::Relaxed);
        self.spawn("deviceout-resize", move |ctl| ctl.apply_target_ms(ms));
        ms
    }

    pub(crate) fn request_queue_periods(self: &Arc<Self>, periods: u32) -> u32 {
        let periods = clamp_queue_periods(periods);
        if periods == self.queue_periods() {
            return periods;
        }
        self.queue_periods.store(periods, Ordering::Relaxed);
        self.spawn("deviceout-queue", move |ctl| {
            ctl.apply_queue_periods(periods)
        });
        periods
    }

    pub(crate) fn clear_exclusive(&self) {
        self.exclusive.store(false, Ordering::Relaxed);
    }

    pub(crate) fn request_exclusive(self: &Arc<Self>, exclusive: bool) {
        if exclusive == self.exclusive() {
            return;
        }
        self.exclusive.store(exclusive, Ordering::Relaxed);
        self.spawn("deviceout-mode", move |ctl| ctl.apply_exclusive(exclusive));
    }

    fn spawn(self: &Arc<Self>, name: &str, job: impl FnOnce(&Self) + Send + 'static) {
        let ctl = Arc::clone(self);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || job(&ctl))
            .ok();
    }

    fn apply_exclusive(&self, exclusive: bool) {
        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        let Some(config) = slot.config.as_mut() else {
            return;
        };
        config.exclusive = exclusive;
        self.restart(&mut producer, &mut slot);
    }

    fn apply_queue_periods(&self, periods: u32) {
        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        let Some(config) = slot.config.as_mut() else {
            return;
        };
        config.device_queue_periods = periods;
        self.restart(&mut producer, &mut slot);
    }

    fn set_device(&self, device_id: String) {
        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        let Some(config) = slot.config.as_mut() else {
            return;
        };
        config.device_id = device_id;
        self.restart(&mut producer, &mut slot);
    }

    fn apply_target_ms(&self, ms: u32) {
        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        if let Some(mut handle) = self.retire(&mut slot) {
            handle.stop();
        }
        let Some(config) = slot.config.as_mut() else {
            return;
        };
        config.target_ms = f64::from(ms);
        let (tx, rx) = ring(capacity_for(config), config.channels);
        *producer = Some(tx);
        slot.consumer = Some(rx);
        self.restart(&mut producer, &mut slot);
    }

    fn restart(&self, producer: &mut Option<RingProducer>, slot: &mut Slot) {
        if let Some(mut handle) = self.retire(slot) {
            if let Some(consumer) = handle.stop() {
                slot.consumer = Some(consumer);
            }
        }
        let Some(mut config) = slot.config.clone() else {
            *self.metrics.write() = None;
            return;
        };
        config.initial_drift_ppm = carried_drift_ppm(slot, &config.device_id)
            .unwrap_or_else(|| deviceout_update::recall_drift(&config.device_id));
        let consumer = match slot.consumer.take() {
            Some(c) => c,
            None => {
                let (tx, rx) = ring(capacity_for(&config), config.channels);
                *producer = Some(tx);
                rx
            }
        };
        slot.running_device = config.device_id.clone();
        let handle = start(consumer, config);
        *self.metrics.write() = Some(Arc::clone(handle.metrics()));
        slot.handle = Some(handle);
    }
}

fn carried_drift_ppm(slot: &Slot, device_id: &str) -> Option<f64> {
    match &slot.carried_drift {
        Some((id, ppm)) if id.as_str() == device_id => Some(*ppm),
        _ => None,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn clamp_queue_periods(periods: u32) -> u32 {
    periods.clamp(
        QUEUE_PERIOD_STEPS[0],
        QUEUE_PERIOD_STEPS[QUEUE_PERIOD_STEPS.len() - 1],
    )
}

fn capacity_for(config: &EngineConfig) -> usize {
    let target = frames_for_ms(config.target_ms, config.source_rate_hz);
    ring_capacity_for_target(target.max(config.max_block_frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_counts_the_device_queue_and_the_resampler_too() {
        assert_eq!(min_target_ms(512, 48_000.0, 2), 54);
        assert_eq!(min_target_ms(2048, 48_000.0, 2), 86);
        assert_eq!(min_target_ms(128, 44_100.0, 2), 46);
        assert_eq!(min_target_ms(512, 48_000.0, 4), 74);
        assert_eq!(min_target_ms(65_536, 48_000.0, 2), 1_409);
    }

    #[test]
    fn a_typed_value_is_kept_unless_it_falls_below_the_floor() {
        assert_eq!(clamp_target_ms(112, 54), 112);
        assert_eq!(clamp_target_ms(70, 54), 70);
        assert_eq!(clamp_target_ms(10, 54), 54);
        assert_eq!(clamp_target_ms(0, 54), 54);
        assert_eq!(clamp_target_ms(u32::MAX, 54), MAX_TARGET_MS);
        assert_eq!(clamp_target_ms(10, 1_409), 1_409);
        assert_eq!(clamp_target_ms(u32::MAX, 1_409), 1_409);
    }

    #[test]
    fn the_device_queue_stays_within_the_offered_steps() {
        assert_eq!(clamp_queue_periods(0), 1);
        assert_eq!(clamp_queue_periods(3), 3);
        assert_eq!(clamp_queue_periods(99), 4);
        assert_eq!(clamp_queue_periods(DEFAULT_QUEUE_PERIODS), 2);
    }

    fn carrying(device_id: &str, ppm: f64) -> Slot {
        Slot {
            carried_drift: Some((device_id.to_string(), ppm)),
            ..Slot::default()
        }
    }

    #[test]
    fn a_restart_keeps_what_the_last_run_learned_about_the_same_device() {
        assert_eq!(carried_drift_ppm(&Slot::default(), "cable"), None);

        let slot = carrying("cable", -152.0);
        assert_eq!(carried_drift_ppm(&slot, "cable"), Some(-152.0));
        assert_eq!(carried_drift_ppm(&slot, "speakers"), None);
    }

    #[test]
    fn a_reading_that_never_settled_is_still_worth_more_than_nothing() {
        let seed = carried_drift_ppm(&carrying("cable", -41.5), "cable").unwrap_or(0.0);
        assert!(seed.abs() > 0.0);
        assert!(seed.abs() < DRIFT_CARRY_SANITY_PPM);
    }

    #[test]
    fn a_block_larger_than_the_target_still_fits_in_the_ring() {
        let config = EngineConfig {
            source_rate_hz: 48_000.0,
            max_block_frames: 16_384,
            target_ms: 10.0,
            ..Default::default()
        };
        assert!(capacity_for(&config) >= 2 * 16_384);
    }

    #[test]
    fn capacity_tracks_the_target_not_the_other_way_round() {
        let at = |ms: f64| {
            capacity_for(&EngineConfig {
                source_rate_hz: 48_000.0,
                max_block_frames: 512,
                target_ms: ms,
                ..Default::default()
            })
        };
        assert_eq!(at(10.0), 2_048);
        assert_eq!(at(30.0), 8_192);
        assert_eq!(at(120.0), 32_768);
    }
}
