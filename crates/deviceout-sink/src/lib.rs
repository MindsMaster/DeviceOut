pub mod device;
pub mod error;

#[cfg(windows)]
pub mod priority;
#[cfg(windows)]
pub mod render;
#[cfg(windows)]
pub mod wasapi;

pub use device::{DeviceInfo, SampleFormat, StreamFormat};
pub use error::SinkError;
#[cfg(windows)]
pub use priority::AudioPriority;
#[cfg(windows)]
pub use render::{device_queue_limit, WasapiSink, MAX_QUEUE_PERIODS, MIN_QUEUE_PERIODS};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteReport {
    pub queued_frames: usize,
    pub starved: bool,
}

pub trait AudioSink: Send {
    fn format(&self) -> StreamFormat;
    fn period_frames(&self) -> usize;
    fn buffer_frames(&self) -> usize;
    fn prefill_silence(&mut self) -> Result<usize, SinkError>;
    fn start(&mut self) -> Result<(), SinkError>;
    fn stop(&mut self) -> Result<(), SinkError>;
    fn write(&mut self, interleaved: &[f32]) -> Result<WriteReport, SinkError>;
}
