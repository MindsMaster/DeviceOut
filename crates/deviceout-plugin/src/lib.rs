use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::{Mutex, RwLock};

use deviceout_core::{ring, RingProducer};
use deviceout_engine::{ring_capacity_frames, start, EngineConfig, EngineHandle, EngineState};
use deviceout_sink::wasapi::{list_output_devices, ComGuard};
use deviceout_sink::DeviceInfo;

mod editor;

const RING_MS: f64 = 400.0;

const EDITOR_WIDTH: u32 = 440;
const EDITOR_HEIGHT: u32 = 420;

pub struct DeviceOut {
    params: Arc<DeviceOutParams>,

    producer: Option<RingProducer>,

    scratch: Vec<f32>,
    slot: Arc<Mutex<EngineSlot>>,
    metrics: Arc<RwLock<Option<Arc<deviceout_engine::EngineMetrics>>>>,
    devices: Arc<RwLock<Vec<DeviceInfo>>>,
}

impl std::fmt::Debug for DeviceOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceOut")
            .field("device_id", &*self.params.device_id.read())
            .field("initialized", &self.producer.is_some())
            .field("scratch_samples", &self.scratch.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct EngineSlot {
    consumer: Option<deviceout_core::RingConsumer>,
    handle: Option<EngineHandle>,
    config: Option<EngineConfig>,
}

#[derive(Params)]
struct DeviceOutParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[persist = "device-id"]
    device_id: Arc<RwLock<String>>,
}

impl Default for DeviceOutParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(EDITOR_WIDTH, EDITOR_HEIGHT),
            device_id: Arc::new(RwLock::new(String::new())),
        }
    }
}

impl Default for DeviceOut {
    fn default() -> Self {
        Self {
            params: Arc::new(DeviceOutParams::default()),
            producer: None,
            scratch: Vec::new(),
            slot: Arc::new(Mutex::new(EngineSlot::default())),
            metrics: Arc::new(RwLock::new(None)),
            devices: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

fn enumerate_devices() -> Vec<DeviceInfo> {
    let Ok(_com) = ComGuard::new() else {
        return Vec::new();
    };
    list_output_devices().unwrap_or_default()
}

impl DeviceOut {
    fn restart_locked(slot: &mut EngineSlot, device_id: String) {
        if let Some(mut handle) = slot.handle.take() {
            if let Some(consumer) = handle.stop() {
                slot.consumer = Some(consumer);
            }
        }

        let (Some(consumer), Some(config)) = (slot.consumer.take(), slot.config.clone()) else {
            return;
        };

        let config = EngineConfig {
            device_id,
            ..config
        };
        slot.config = Some(config.clone());
        slot.handle = Some(start(consumer, config));
    }
}

impl Plugin for DeviceOut {
    const NAME: &'static str = "DeviceOut";
    const VENDOR: &'static str = "DeviceOut";
    const URL: &'static str = "https://github.com/MindsMaster/DeviceOut";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(editor::Wiring {
            params: Arc::clone(&self.params),
            slot: Arc::clone(&self.slot),
            metrics: Arc::clone(&self.metrics),
            devices: Arc::clone(&self.devices),
        })
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let channels = audio_io_layout
            .main_output_channels
            .map_or(2, |c| c.get() as usize);
        let source_rate = f64::from(buffer_config.sample_rate);

        self.scratch = vec![0.0; buffer_config.max_buffer_size as usize * channels];

        let capacity = ring_capacity_frames(source_rate, RING_MS);
        let (producer, consumer) = ring(capacity, channels);
        self.producer = Some(producer);

        *self.devices.write() = enumerate_devices();

        let device_id = {
            let stored = self.params.device_id.read().clone();
            if stored.is_empty() {
                let devices = self.devices.read();
                devices
                    .iter()
                    .find(|d| d.is_default)
                    .or_else(|| devices.first())
                    .map(|d| d.id.clone())
                    .unwrap_or_default()
            } else {
                stored
            }
        };
        *self.params.device_id.write() = device_id.clone();

        let mut slot = self.slot.lock();
        *slot = EngineSlot {
            consumer: Some(consumer),
            handle: None,
            config: Some(EngineConfig {
                device_id: device_id.clone(),
                source_rate_hz: source_rate,
                channels,
                ..Default::default()
            }),
        };
        Self::restart_locked(&mut slot, device_id);
        *self.metrics.write() = slot.handle.as_ref().map(|h| Arc::clone(h.metrics()));

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if let Some(producer) = self.producer.as_mut() {
            let mut n = 0;
            for channel_samples in buffer.iter_samples() {
                for sample in channel_samples {
                    if n >= self.scratch.len() {
                        break;
                    }
                    self.scratch[n] = *sample;
                    n += 1;
                }
            }

            producer.push(&self.scratch[..n]);
        }

        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        let mut slot = self.slot.lock();
        if let Some(mut handle) = slot.handle.take() {
            slot.consumer = handle.stop();
        }
    }
}

impl Vst3Plugin for DeviceOut {
    const VST3_CLASS_ID: [u8; 16] = *b"DeviceOutVST3_01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nih_export_vst3!(DeviceOut);

pub(crate) struct UiState {
    pub state: EngineState,
    pub error: Option<String>,
    pub fill_fraction: f64,
    pub capacity_frames: u64,
    pub drift_ppm: Option<f64>,
    pub raw_drift_ppm: f64,
    pub underruns: u64,
    pub overruns: u64,
    pub dropout_seconds: f64,
    pub clamp_events: u64,
    pub sink_rate_hz: f64,
    pub period_frames: u64,
    pub latency_ms: f64,
    pub reconnects: u64,
    pub frames_discarded: u64,
}
