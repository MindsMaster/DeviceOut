use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use deviceout_core::{ring, RingConsumer, RingProducer};
use deviceout_engine::{start, EngineConfig, EngineHandle, EngineMetrics};

pub(crate) const DEFAULT_RING_FRAMES: u32 = 16384;
pub(crate) const RING_FRAME_STEPS: &[u32] = &[1024, 2048, 4096, 8192, 16384, 32768];

pub(crate) fn nearest_ring_step(frames: u32) -> u32 {
    RING_FRAME_STEPS
        .iter()
        .copied()
        .min_by_key(|step| step.abs_diff(frames.max(1)))
        .unwrap_or(DEFAULT_RING_FRAMES)
}

pub(crate) fn min_ring_frames(max_block_frames: u32, source_rate_hz: f64) -> u32 {
    let period = (source_rate_hz * 0.01).ceil() as u32;
    let need = 2 * (max_block_frames + period);
    RING_FRAME_STEPS
        .iter()
        .copied()
        .find(|&step| step >= need)
        .unwrap_or(RING_FRAME_STEPS[RING_FRAME_STEPS.len() - 1])
}

pub(crate) fn clamp_ring_frames(frames: u32, floor: u32) -> u32 {
    nearest_ring_step(frames).max(floor)
}

#[derive(Debug, Default)]
struct Slot {
    consumer: Option<RingConsumer>,
    handle: Option<EngineHandle>,
    config: Option<EngineConfig>,
}

#[derive(Debug, Default)]
pub(crate) struct EngineController {
    producer: Mutex<Option<RingProducer>>,
    slot: Mutex<Slot>,
    metrics: RwLock<Option<Arc<EngineMetrics>>>,
    ring_frames: AtomicU32,
    ring_floor: AtomicU32,
}

impl EngineController {
    pub(crate) fn producer(&self) -> &Mutex<Option<RingProducer>> {
        &self.producer
    }

    pub(crate) fn metrics(&self) -> Option<Arc<EngineMetrics>> {
        self.metrics.read().clone()
    }

    pub(crate) fn ring_floor(&self) -> u32 {
        self.ring_floor.load(Ordering::Relaxed)
    }

    pub(crate) fn ring_frames(&self) -> u32 {
        self.ring_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn initialize(&self, config: EngineConfig, ring_frames: u32, ring_floor: u32) -> u32 {
        let frames = clamp_ring_frames(ring_frames, ring_floor);
        self.ring_floor.store(ring_floor, Ordering::Relaxed);
        self.ring_frames.store(frames, Ordering::Relaxed);

        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        if let Some(mut handle) = slot.handle.take() {
            handle.stop();
        }
        let (tx, rx) = ring(frames as usize, config.channels);
        *producer = Some(tx);
        *slot = Slot {
            consumer: Some(rx),
            handle: None,
            config: Some(config),
        };
        self.restart(&mut producer, &mut slot);
        frames
    }

    pub(crate) fn deactivate(&self) {
        let mut slot = self.slot.lock();
        if let Some(mut handle) = slot.handle.take() {
            slot.consumer = handle.stop();
        }
        *self.metrics.write() = None;
    }

    pub(crate) fn set_device_async(self: &Arc<Self>, device_id: String) {
        let ctl = Arc::clone(self);
        std::thread::Builder::new()
            .name("deviceout-switch".into())
            .spawn(move || ctl.set_device(device_id))
            .ok();
    }

    pub(crate) fn set_ring_frames_async(self: &Arc<Self>, frames: u32) {
        let ctl = Arc::clone(self);
        std::thread::Builder::new()
            .name("deviceout-resize".into())
            .spawn(move || ctl.set_ring_frames(frames))
            .ok();
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

    fn set_ring_frames(&self, frames: u32) {
        let frames = clamp_ring_frames(frames, self.ring_floor());
        self.ring_frames.store(frames, Ordering::Relaxed);
        let mut producer = self.producer.lock();
        let mut slot = self.slot.lock();
        if let Some(mut handle) = slot.handle.take() {
            handle.stop();
        }
        let Some(channels) = slot.config.as_ref().map(|c| c.channels) else {
            return;
        };
        let (tx, rx) = ring(frames as usize, channels);
        *producer = Some(tx);
        slot.consumer = Some(rx);
        self.restart(&mut producer, &mut slot);
    }

    fn restart(&self, producer: &mut Option<RingProducer>, slot: &mut Slot) {
        if let Some(mut handle) = slot.handle.take() {
            if let Some(consumer) = handle.stop() {
                slot.consumer = Some(consumer);
            }
        }
        let Some(config) = slot.config.clone() else {
            *self.metrics.write() = None;
            return;
        };
        let consumer = match slot.consumer.take() {
            Some(c) => c,
            None => {
                let (tx, rx) = ring(self.ring_frames() as usize, config.channels);
                *producer = Some(tx);
                rx
            }
        };
        let handle = start(consumer, config);
        *self.metrics.write() = Some(Arc::clone(handle.metrics()));
        slot.handle = Some(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_covers_two_blocks_plus_device_periods() {
        assert_eq!(min_ring_frames(512, 48_000.0), 2048);
        assert_eq!(min_ring_frames(1024, 48_000.0), 4096);
        assert_eq!(min_ring_frames(2048, 48_000.0), 8192);
        assert_eq!(min_ring_frames(128, 44_100.0), 2048);
        assert_eq!(min_ring_frames(65_536, 48_000.0), 32_768);
    }

    #[test]
    fn user_choice_snaps_to_a_step_but_never_below_the_floor() {
        assert_eq!(clamp_ring_frames(1024, 4096), 4096);
        assert_eq!(clamp_ring_frames(3000, 1024), 2048);
        assert_eq!(clamp_ring_frames(16384, 1024), 16384);
        assert_eq!(clamp_ring_frames(0, 1024), 1024);
        assert_eq!(clamp_ring_frames(u32::MAX, 1024), 32_768);
    }
}
