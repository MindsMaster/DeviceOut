use std::fmt;

use deviceout_core::ResampleError;
use deviceout_sink::SinkError;

#[derive(Debug)]
pub enum EngineError {
    Config(String),
    ChannelMismatch { device: usize, source: usize },
    Sink(SinkError),
    Resample(ResampleError),
}

impl EngineError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::Sink(e) => e.is_recoverable(),
            _ => false,
        }
    }

    pub fn fault(&self) -> Fault {
        let kind = match self {
            Self::Config(_) => FaultKind::Config,
            Self::ChannelMismatch { device, source } => FaultKind::ChannelMismatch {
                device: *device,
                source: *source,
            },
            Self::Resample(_) => FaultKind::Resample,
            Self::Sink(e) => match e {
                SinkError::ComInit(_) => FaultKind::ComInit,
                SinkError::DeviceNotFound(_) => FaultKind::DeviceNotFound,
                SinkError::Enumeration(_) => FaultKind::Enumeration,
                SinkError::UnsupportedFormat { .. } => FaultKind::UnsupportedFormat,
                SinkError::StreamInit(_) => FaultKind::StreamInit,
                SinkError::Stream(_) | SinkError::MisalignedBuffer { .. } => FaultKind::Stream,
                SinkError::DeviceLost(_) => FaultKind::DeviceLost,
            },
        };
        Fault {
            kind,
            detail: self.to_string(),
            attempt: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    Config,
    ChannelMismatch { device: usize, source: usize },
    Resample,
    ComInit,
    DeviceNotFound,
    Enumeration,
    UnsupportedFormat,
    StreamInit,
    Stream,
    DeviceLost,
    Thread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    pub detail: String,
    pub attempt: u32,
}

impl Fault {
    pub fn thread(detail: String) -> Self {
        Self {
            kind: FaultKind::Thread,
            detail,
            attempt: 0,
        }
    }

    pub fn with_attempt(mut self, attempt: u32) -> Self {
        self.attempt = attempt;
        self
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.attempt > 0 {
            write!(f, "{}（第 {} 次）", self.detail, self.attempt)
        } else {
            f.write_str(&self.detail)
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(m) => write!(f, "配置不合法: {m}"),
            Self::ChannelMismatch { device, source } => {
                write!(f, "设备 {device} 声道，DAW {source} 声道，不一致")
            }
            Self::Sink(e) => write!(f, "{e}"),
            Self::Resample(e) => write!(f, "重采样出错: {e}"),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sink(e) => Some(e),
            Self::Resample(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SinkError> for EngineError {
    fn from(e: SinkError) -> Self {
        Self::Sink(e)
    }
}

impl From<ResampleError> for EngineError {
    fn from(e: ResampleError) -> Self {
        Self::Resample(e)
    }
}
