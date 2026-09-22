use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use parking_lot::{Mutex, RwLock};

use deviceout_core::{ring, RingProducer};
use deviceout_engine::{
    min_total_ms, ring_capacity_for, start, EngineConfig, EngineHandle, EngineMetrics, SourceBus,
};

pub(crate) const DEFAULT_TARGET_MS: u32 = 80;
pub(crate) const MAX_TARGET_MS: u32 = 1_000;
pub(crate) const TARGET_MS_STEPS: &[u32] = &[10, 20, 30, 40, 50, 60, 80, 120, 200];
pub(crate) const QUEUE_PERIOD_STEPS: &[u32] = &[1, 2, 3, 4];
pub(crate) const DEFAULT_QUEUE_PERIODS: u32 = 4;

const UNATTACHED: u64 = 0;

static HOST_DROPS: AtomicU64 = AtomicU64::new(0);

fn hub() -> &'static Mutex<Vec<Weak<Device>>> {
    static HUB: OnceLock<Mutex<Vec<Weak<Device>>>> = OnceLock::new();
    HUB.get_or_init(|| Mutex::new(Vec::new()))
}

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

pub(crate) fn clamp_queue_periods(periods: u32) -> u32 {
    periods.clamp(
        QUEUE_PERIOD_STEPS[0],
        QUEUE_PERIOD_STEPS[QUEUE_PERIOD_STEPS.len() - 1],
    )
}

#[derive(Debug, Default)]
struct Member {
    producer: Mutex<Option<RingProducer>>,
    source: AtomicU64,
}

#[derive(Debug)]
struct Wiring {
    config: EngineConfig,
    bus: Arc<SourceBus>,
    engine: Option<EngineHandle>,
    members: Vec<Arc<Member>>,
}

#[derive(Debug)]
struct Device {
    id: String,
    wiring: Mutex<Wiring>,
    metrics: RwLock<Option<Arc<EngineMetrics>>>,
    target_ms: AtomicU32,
    floor_ms: AtomicU32,
    queue_periods: AtomicU32,
    exclusive: AtomicBool,
}

fn adopt(device_id: &str) -> Arc<Device> {
    let mut hub = hub().lock();
    hub.retain(|entry| entry.strong_count() > 0);
    let existing = hub
        .iter()
        .filter_map(Weak::upgrade)
        .find(|d| d.id == device_id);
    if let Some(device) = existing {
        return device;
    }
    let device = Arc::new(Device {
        id: device_id.to_string(),
        wiring: Mutex::new(Wiring {
            config: EngineConfig::default(),
            bus: Arc::new(SourceBus::default()),
            engine: None,
            members: Vec::new(),
        }),
        metrics: RwLock::new(None),
        target_ms: AtomicU32::new(DEFAULT_TARGET_MS),
        floor_ms: AtomicU32::new(DEFAULT_TARGET_MS),
        queue_periods: AtomicU32::new(DEFAULT_QUEUE_PERIODS),
        exclusive: AtomicBool::new(false),
    });
    hub.push(Arc::downgrade(&device));
    device
}

impl Device {
    fn admit(&self, member: &Arc<Member>, wanted: &EngineConfig, floor_ms: u32) -> u32 {
        let mut wiring = self.wiring.lock();
        let first = wiring.members.is_empty();

        if first {
            wiring.config = wanted.clone();
            self.floor_ms.store(floor_ms, Ordering::Relaxed);
            self.target_ms.store(
                clamp_target_ms(wanted.target_ms as u32, floor_ms),
                Ordering::Relaxed,
            );
            self.queue_periods.store(
                clamp_queue_periods(wanted.device_queue_periods),
                Ordering::Relaxed,
            );
            self.exclusive.store(wanted.exclusive, Ordering::Relaxed);
        }

        let roomier = wanted.max_block_frames > wiring.config.max_block_frames;
        if roomier {
            wiring.config.max_block_frames = wanted.max_block_frames;
            self.raise_floor(floor_ms);
        }

        wiring.members.push(Arc::clone(member));
        if first || roomier {
            self.restart(&mut wiring);
        } else {
            self.wire(&wiring, member);
        }
        self.target_ms.load(Ordering::Relaxed)
    }

    fn release(&self, member: &Arc<Member>) {
        let mut wiring = self.wiring.lock();
        let source = member.source.swap(UNATTACHED, Ordering::Relaxed);
        if source != UNATTACHED {
            wiring.bus.leave(source);
        }
        *member.producer.lock() = None;
        wiring.members.retain(|m| !Arc::ptr_eq(m, member));

        if wiring.members.is_empty() {
            if let Some(mut engine) = wiring.engine.take() {
                engine.stop();
            }
            *self.metrics.write() = None;
        }
    }

    fn raise_floor(&self, floor_ms: u32) {
        let floor = self.floor_ms.load(Ordering::Relaxed).max(floor_ms);
        self.floor_ms.store(floor, Ordering::Relaxed);
        let target = clamp_target_ms(self.target_ms.load(Ordering::Relaxed), floor);
        self.target_ms.store(target, Ordering::Relaxed);
    }

    fn reshape(&self) {
        let mut wiring = self.wiring.lock();
        if wiring.members.is_empty() {
            return;
        }
        self.restart(&mut wiring);
    }

    fn wire(&self, wiring: &Wiring, member: &Arc<Member>) {
        let (tx, rx) = ring(ring_capacity_for(&wiring.config), wiring.config.channels);
        let source = wiring.bus.join(rx);
        let stale = member.source.swap(source, Ordering::Relaxed);
        *member.producer.lock() = Some(tx);
        if stale != UNATTACHED {
            wiring.bus.leave(stale);
        }
    }

    fn restart(&self, wiring: &mut Wiring) {
        if let Some(mut engine) = wiring.engine.take() {
            engine.stop();
        }
        wiring.config.device_id = self.id.clone();
        wiring.config.target_ms = f64::from(self.target_ms.load(Ordering::Relaxed));
        wiring.config.device_queue_periods = self.queue_periods.load(Ordering::Relaxed);
        wiring.config.exclusive = self.exclusive.load(Ordering::Relaxed);

        for member in wiring.members.clone() {
            self.wire(wiring, &member);
        }

        let engine = start(Arc::clone(&wiring.bus), wiring.config.clone());
        *self.metrics.write() = Some(Arc::clone(engine.metrics()));
        wiring.engine = Some(engine);
    }
}

#[derive(Debug, Default)]
pub(crate) struct EngineController {
    member: Arc<Member>,
    device: RwLock<Option<Arc<Device>>>,
    wanted: Mutex<Option<EngineConfig>>,
    target_ms: AtomicU32,
    floor_ms: AtomicU32,
    queue_periods: AtomicU32,
    exclusive: AtomicBool,
}

impl EngineController {
    pub(crate) fn producer(&self) -> &Mutex<Option<RingProducer>> {
        &self.member.producer
    }

    pub(crate) fn metrics(&self) -> Option<Arc<EngineMetrics>> {
        let device = self.device.read().clone()?;
        let metrics = device.metrics.read().clone();
        metrics
    }

    pub(crate) fn note_host_drop(&self) {
        HOST_DROPS.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn target_floor_ms(&self) -> u32 {
        let live = self.metrics().map_or(0.0, |m| m.floor_ms());
        if live.is_finite() && live > 0.0 {
            return live.ceil() as u32;
        }
        self.read(|d| d.floor_ms.load(Ordering::Relaxed), &self.floor_ms)
    }

    pub(crate) fn target_ms(&self) -> u32 {
        self.read(|d| d.target_ms.load(Ordering::Relaxed), &self.target_ms)
    }

    pub(crate) fn queue_periods(&self) -> u32 {
        self.read(
            |d| d.queue_periods.load(Ordering::Relaxed),
            &self.queue_periods,
        )
    }

    pub(crate) fn exclusive(&self) -> bool {
        match self.device.read().as_ref() {
            Some(device) => device.exclusive.load(Ordering::Relaxed),
            None => self.exclusive.load(Ordering::Relaxed),
        }
    }

    fn read(&self, from: impl Fn(&Device) -> u32, fallback: &AtomicU32) -> u32 {
        match self.device.read().as_ref() {
            Some(device) => from(device),
            None => fallback.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn initialize(&self, config: EngineConfig, target_ms: u32, floor_ms: u32) -> u32 {
        let ms = clamp_target_ms(target_ms, floor_ms);
        let wanted = EngineConfig {
            target_ms: f64::from(ms),
            host_drops: Some(&HOST_DROPS),
            ..config
        };
        self.target_ms.store(ms, Ordering::Relaxed);
        self.floor_ms.store(floor_ms, Ordering::Relaxed);
        self.queue_periods.store(
            clamp_queue_periods(wanted.device_queue_periods),
            Ordering::Relaxed,
        );
        self.exclusive.store(wanted.exclusive, Ordering::Relaxed);
        self.join(wanted, floor_ms)
    }

    pub(crate) fn deactivate(&self) {
        self.leave();
    }

    pub(crate) fn set_device_async(self: &Arc<Self>, device_id: String) {
        self.spawn("deviceout-switch", move |ctl| ctl.set_device(device_id));
    }

    pub(crate) fn request_target_ms(&self, ms: u32) -> u32 {
        let ms = clamp_target_ms(ms, self.target_floor_ms());
        if ms == self.target_ms() {
            return ms;
        }
        self.target_ms.store(ms, Ordering::Relaxed);
        self.retune("deviceout-resize", move |device| {
            device.target_ms.store(ms, Ordering::Relaxed);
        });
        ms
    }

    pub(crate) fn request_queue_periods(&self, periods: u32) -> u32 {
        let periods = clamp_queue_periods(periods);
        if periods == self.queue_periods() {
            return periods;
        }
        self.queue_periods.store(periods, Ordering::Relaxed);
        self.retune("deviceout-queue", move |device| {
            device.queue_periods.store(periods, Ordering::Relaxed);
        });
        periods
    }

    pub(crate) fn clear_exclusive(&self) {
        self.exclusive.store(false, Ordering::Relaxed);
        if let Some(device) = self.device.read().as_ref() {
            device.exclusive.store(false, Ordering::Relaxed);
        }
    }

    pub(crate) fn request_exclusive(&self, exclusive: bool) {
        if exclusive == self.exclusive() {
            return;
        }
        self.exclusive.store(exclusive, Ordering::Relaxed);
        self.retune("deviceout-mode", move |device| {
            device.exclusive.store(exclusive, Ordering::Relaxed);
        });
    }

    fn retune(&self, name: &str, set: impl FnOnce(&Device) + Send + 'static) {
        let Some(device) = self.device.read().clone() else {
            return;
        };
        crate::spawn::detach(name, move || {
            set(&device);
            device.reshape();
        });
    }

    fn spawn(self: &Arc<Self>, name: &str, job: impl FnOnce(&Self) + Send + 'static) {
        let ctl = Arc::clone(self);
        crate::spawn::detach(name, move || job(&ctl));
    }

    fn set_device(&self, device_id: String) {
        let Some(mut wanted) = self.wanted.lock().clone() else {
            return;
        };
        if wanted.device_id == device_id {
            return;
        }
        wanted.device_id = device_id;
        wanted.target_ms = f64::from(self.target_ms());
        wanted.device_queue_periods = self.queue_periods();
        wanted.exclusive = self.exclusive();
        let floor = self.floor_ms.load(Ordering::Relaxed);
        self.join(wanted, floor);
    }

    fn join(&self, wanted: EngineConfig, floor_ms: u32) -> u32 {
        self.leave();
        let device = adopt(&wanted.device_id);
        let ms = device.admit(&self.member, &wanted, floor_ms);
        *self.wanted.lock() = Some(wanted);
        *self.device.write() = Some(device);
        self.target_ms.store(ms, Ordering::Relaxed);
        ms
    }

    fn leave(&self) {
        let device = self.device.write().take();
        if let Some(device) = device {
            device.release(&self.member);
        }
    }
}

impl Drop for EngineController {
    fn drop(&mut self) {
        self.leave();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_counts_the_device_queue_and_the_resampler_too() {
        assert_eq!(min_target_ms(512, 48_000.0, 2), 44);
        assert_eq!(min_target_ms(2048, 48_000.0, 2), 76);
        assert_eq!(min_target_ms(128, 44_100.0, 2), 36);
        assert_eq!(min_target_ms(512, 48_000.0, 4), 64);
        assert_eq!(min_target_ms(65_536, 48_000.0, 2), 1_399);
    }

    #[test]
    fn a_typed_value_is_kept_unless_it_falls_below_the_floor() {
        assert_eq!(clamp_target_ms(112, 44), 112);
        assert_eq!(clamp_target_ms(70, 44), 70);
        assert_eq!(clamp_target_ms(10, 44), 44);
        assert_eq!(clamp_target_ms(0, 44), 44);
        assert_eq!(clamp_target_ms(u32::MAX, 44), MAX_TARGET_MS);
        assert_eq!(clamp_target_ms(10, 1_409), 1_409);
        assert_eq!(clamp_target_ms(u32::MAX, 1_409), 1_409);
    }

    #[test]
    fn the_device_queue_stays_within_the_offered_steps() {
        assert_eq!(clamp_queue_periods(0), 1);
        assert_eq!(clamp_queue_periods(3), 3);
        assert_eq!(clamp_queue_periods(99), 4);
        assert_eq!(clamp_queue_periods(DEFAULT_QUEUE_PERIODS), 4);
    }

    #[test]
    fn the_shipped_default_clears_the_floor_it_ships_with() {
        let floor = min_target_ms(512, 48_000.0, DEFAULT_QUEUE_PERIODS);
        assert_eq!(floor, 64);
        assert_eq!(clamp_target_ms(DEFAULT_TARGET_MS, floor), DEFAULT_TARGET_MS);
        assert!(TARGET_MS_STEPS.contains(&DEFAULT_TARGET_MS));
        assert!(QUEUE_PERIOD_STEPS.contains(&DEFAULT_QUEUE_PERIODS));
    }
}
