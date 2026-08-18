use std::fmt;

#[derive(Debug)]
pub enum SinkError {
    ComInit(String),
    DeviceNotFound(String),
    Enumeration(String),
    UnsupportedFormat {
        requested: String,
        supported: String,
    },
    StreamInit(String),
    Stream(String),
    DeviceLost(String),
    MisalignedBuffer {
        samples: usize,
        channels: usize,
    },
}

impl SinkError {
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::DeviceLost(_) | Self::DeviceNotFound(_))
    }
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ComInit(m) => write!(f, "COM 初始化失败: {m}"),
            Self::DeviceNotFound(m) => write!(f, "找不到输出设备: {m}"),
            Self::Enumeration(m) => write!(f, "枚举音频设备失败: {m}"),
            Self::UnsupportedFormat {
                requested,
                supported,
            } => write!(f, "设备不支持请求的格式 {requested}，它只接受 {supported}"),
            Self::StreamInit(m) => write!(f, "音频流初始化失败: {m}"),
            Self::Stream(m) => write!(f, "音频流出错: {m}"),
            Self::DeviceLost(m) => write!(f, "输出设备已失效: {m}"),
            Self::MisalignedBuffer { samples, channels } => write!(
                f,
                "样本数 {samples} 不是声道数 {channels} 的整数倍，缓冲会错位"
            ),
        }
    }
}

impl std::error::Error for SinkError {}

#[cfg(windows)]
mod hresult {
    pub const AUDCLNT_E_DEVICE_INVALIDATED: i32 = -2004287484;
    pub const AUDCLNT_E_SERVICE_NOT_RUNNING: i32 = -2004287472;
    pub const AUDCLNT_E_RESOURCES_INVALIDATED: i32 = -2004287450;
}

#[cfg(windows)]
impl From<windows::core::Error> for SinkError {
    fn from(e: windows::core::Error) -> Self {
        Self::from_hresult("", e)
    }
}

#[cfg(windows)]
impl SinkError {
    pub(crate) fn from_hresult(context: &str, e: windows::core::Error) -> Self {
        Self::classify(context, e, Self::Stream)
    }

    pub(crate) fn init_from_hresult(context: &str, e: windows::core::Error) -> Self {
        Self::classify(context, e, Self::StreamInit)
    }

    fn classify(context: &str, e: windows::core::Error, fallback: fn(String) -> Self) -> Self {
        let code = e.code().0;
        let detail = if context.is_empty() {
            format!("{e}")
        } else {
            format!("{context}: {e}")
        };

        if matches!(
            code,
            hresult::AUDCLNT_E_DEVICE_INVALIDATED
                | hresult::AUDCLNT_E_SERVICE_NOT_RUNNING
                | hresult::AUDCLNT_E_RESOURCES_INVALIDATED
        ) {
            Self::DeviceLost(detail)
        } else {
            fallback(detail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_device_loss_is_worth_retrying() {
        assert!(SinkError::DeviceLost("x".into()).is_recoverable());
        assert!(SinkError::DeviceNotFound("x".into()).is_recoverable());

        assert!(!SinkError::Stream("x".into()).is_recoverable());
        assert!(!SinkError::StreamInit("x".into()).is_recoverable());
        assert!(!SinkError::ComInit("x".into()).is_recoverable());
        assert!(!SinkError::Enumeration("x".into()).is_recoverable());
        assert!(!SinkError::MisalignedBuffer {
            samples: 3,
            channels: 2
        }
            .is_recoverable());
        assert!(!SinkError::UnsupportedFormat {
            requested: "a".into(),
            supported: "b".into()
        }
            .is_recoverable());
    }

    #[cfg(windows)]
    #[test]
    fn device_invalidated_hresults_are_classified_as_loss() {
        for code in [
            hresult::AUDCLNT_E_DEVICE_INVALIDATED,
            hresult::AUDCLNT_E_SERVICE_NOT_RUNNING,
            hresult::AUDCLNT_E_RESOURCES_INVALIDATED,
        ] {
            let e = windows::core::Error::from_hresult(windows::core::HRESULT(code));
            let mapped = SinkError::from_hresult("写入时", e);
            assert!(
                mapped.is_recoverable(),
                "HRESULT {code:#x} 没被认成设备失效: {mapped}"
            );
            assert!(
                format!("{mapped}").contains("写入时"),
                "上下文丢失，无法判断失效发生在哪一步"
            );
        }

        let e = windows::core::Error::from_hresult(windows::core::HRESULT(-2147024809));
        assert!(!SinkError::from_hresult("", e).is_recoverable());
    }
}
