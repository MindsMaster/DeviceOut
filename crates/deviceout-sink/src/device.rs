use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    F32,
    I16,
    I24In32,
    I32,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::F32 | Self::I24In32 | Self::I32 => 4,
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::F32 => write!(f, "32 位浮点"),
            Self::I16 => write!(f, "16 位整型"),
            Self::I24In32 => write!(f, "24 位整型(32 位容器)"),
            Self::I32 => write!(f, "32 位整型"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

impl StreamFormat {
    pub fn frame_bytes(&self) -> usize {
        self.channels as usize * self.sample_format.bytes()
    }
}

impl fmt::Display for StreamFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Hz / {} 声道 / {}",
            self.sample_rate, self.channels, self.sample_format
        )
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub mix_format: StreamFormat,
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}  [{}]",
            self.name,
            if self.is_default { " (默认)" } else { "" },
            self.mix_format
        )
    }
}
