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
