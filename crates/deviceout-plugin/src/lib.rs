use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::Once;

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;

use deviceout_engine::{EngineConfig, EngineState, Fault};
use deviceout_sink::wasapi::{list_output_devices, ComGuard};
use deviceout_sink::DeviceInfo;

mod editor;
mod engine_ctl;
mod heartbeat;

pub(crate) use engine_ctl::{
    min_target_ms, EngineController, DEFAULT_QUEUE_PERIODS, DEFAULT_TARGET_MS, MAX_TARGET_MS,
    QUEUE_PERIOD_STEPS, TARGET_MS_STEPS,
};
pub(crate) use heartbeat::Heartbeat;

const EDITOR_WIDTH: u32 = 440;
const EDITOR_HEIGHT: u32 = 560;

pub struct DeviceOut {
    params: Arc<DeviceOutParams>,
    engine: Arc<EngineController>,
    scratch: Vec<f32>,
    devices: Arc<RwLock<Vec<DeviceInfo>>>,
    heartbeat: Option<Heartbeat>,
}

impl std::fmt::Debug for DeviceOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceOut")
            .field("device_id", &*self.params.device_id.read())
            .field("scratch_samples", &self.scratch.len())
            .finish_non_exhaustive()
    }
}

#[derive(Params)]
struct DeviceOutParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[persist = "device-id"]
    device_id: Arc<RwLock<String>>,

    #[persist = "target-ms"]
    target_ms: Arc<RwLock<u32>>,

    #[persist = "queue-periods"]
    queue_periods: Arc<RwLock<u32>>,

    #[persist = "exclusive"]
    exclusive: Arc<RwLock<bool>>,
}

impl Default for DeviceOutParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(EDITOR_WIDTH, EDITOR_HEIGHT),
            device_id: Arc::new(RwLock::new(String::new())),
            target_ms: Arc::new(RwLock::new(DEFAULT_TARGET_MS)),
            queue_periods: Arc::new(RwLock::new(DEFAULT_QUEUE_PERIODS)),
            exclusive: Arc::new(RwLock::new(false)),
        }
    }
}

impl Default for DeviceOut {
    fn default() -> Self {
        Self {
            params: Arc::new(DeviceOutParams::default()),
            engine: Arc::new(EngineController::default()),
            scratch: Vec::new(),
            devices: Arc::new(RwLock::new(Vec::new())),
            heartbeat: None,
        }
    }
}

fn enumerate_devices() -> Vec<DeviceInfo> {
    let Ok(_com) = ComGuard::new() else {
        return Vec::new();
    };
    list_output_devices().unwrap_or_default()
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
            engine: Arc::clone(&self.engine),
            devices: Arc::clone(&self.devices),
        })
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        kick_updater();
        if self.heartbeat.is_none() {
            self.heartbeat = Some(Heartbeat::start(env!("CARGO_PKG_VERSION")));
        }

        let channels = audio_io_layout
            .main_output_channels
            .map_or(2, |c| c.get() as usize);
        let source_rate = f64::from(buffer_config.sample_rate);

        self.scratch = vec![0.0; buffer_config.max_buffer_size as usize * channels];

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

        let queue_periods = *self.params.queue_periods.read();
        let floor = min_target_ms(buffer_config.max_buffer_size, source_rate, queue_periods);
        let requested = *self.params.target_ms.read();
        let target_ms = self.engine.initialize(
            EngineConfig {
                device_id,
                source_rate_hz: source_rate,
                channels,
                max_block_frames: buffer_config.max_buffer_size as usize,
                device_queue_periods: queue_periods,
                exclusive: *self.params.exclusive.read(),
                trace_path: Some(deviceout_update::paths::engine_log_path()),
                ..Default::default()
            },
            requested,
            floor,
        );
        *self.params.target_ms.write() = target_ms;
        *self.params.queue_periods.write() = self.engine.queue_periods();

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(mut guard) = self.engine.producer().try_lock() else {
            self.engine.note_host_drop();
            return ProcessStatus::Normal;
        };
        if let Some(producer) = guard.as_mut() {
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
        if let Some(mut heartbeat) = self.heartbeat.take() {
            heartbeat.stop();
        }
        self.engine.deactivate();
    }
}

impl Vst3Plugin for DeviceOut {
    const VST3_CLASS_ID: [u8; 16] = *b"DeviceOutVST3_01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nih_export_vst3!(DeviceOut);

fn kick_updater() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = deviceout_update::write_panic(info);
            prev(info);
        }));

        if let Some(bundle) = deviceout_update::loaded_bundle_path() {
            deviceout_update::spawn_updater(&[bundle.as_os_str()]);
        }
        if !deviceout_update::list_json(&deviceout_update::outbox_dir()).is_empty() {
            deviceout_update::spawn_updater(&[std::ffi::OsStr::new("--send-outbox")]);
        }
    });
}

pub(crate) struct UiState {
    pub state: EngineState,
    pub error: Option<Fault>,
    pub fill_fraction: f64,
    pub target_fraction: f64,
    pub capacity_frames: u64,
    pub target_frames: f64,
    pub min_target_frames: u64,
    pub exclusive: bool,
    pub drift_ppm: Option<f64>,
    pub raw_drift_ppm: f64,
    pub underruns: u64,
    pub overruns: u64,
    pub dropout_seconds: f64,
    pub clamp_events: u64,
    pub sink_rate_hz: f64,
    pub period_frames: u64,
    pub min_period_frames: u64,
    pub latency_ms: f64,
    pub device_starvations: u64,
    pub reconnects: u64,
    pub frames_discarded: u64,
}
