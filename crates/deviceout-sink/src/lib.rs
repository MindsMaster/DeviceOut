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
pub use render::WasapiSink;

pub trait AudioSink: Send {
    fn format(&self) -> StreamFormat;
    fn period_frames(&self) -> usize;
    fn start(&mut self) -> Result<(), SinkError>;
    fn stop(&mut self) -> Result<(), SinkError>;
    fn write(&mut self, interleaved: &[f32]) -> Result<(), SinkError>;
}
