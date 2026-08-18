use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

use crate::device::{DeviceInfo, SampleFormat, StreamFormat};
use crate::error::SinkError;

const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
    GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const WAVE_FORMAT_PCM_TAG: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;

#[derive(Debug)]
pub struct ComGuard {
    owns: bool,
}

impl ComGuard {
    pub fn new() -> Result<Self, SinkError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            Ok(Self { owns: true })
        } else if hr == windows::Win32::Foundation::RPC_E_CHANGED_MODE {
            Ok(Self { owns: false })
        } else {
            Err(SinkError::ComInit(format!("{hr:?}")))
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owns {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

pub(crate) unsafe fn parse_format(wfx: *const WAVEFORMATEX) -> Result<StreamFormat, SinkError> {
    let base = unsafe { &*wfx };
    let tag = base.wFormatTag;
    let bits = base.wBitsPerSample;
    let sample_rate = base.nSamplesPerSec;
    let channels = base.nChannels;

    let sample_format = if tag == WAVE_FORMAT_EXTENSIBLE_TAG {
        let ext = unsafe { &*(wfx as *const WAVEFORMATEXTENSIBLE) };
        let sub = ext.SubFormat;
        if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            SampleFormat::F32
        } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
            int_format(bits)?
        } else {
            return Err(SinkError::UnsupportedFormat {
                requested: "32 位浮点".into(),
                supported: format!("未知子格式 {sub:?}"),
            });
        }
    } else if tag == WAVE_FORMAT_IEEE_FLOAT_TAG {
        SampleFormat::F32
    } else if tag == WAVE_FORMAT_PCM_TAG {
        int_format(bits)?
    } else {
        return Err(SinkError::UnsupportedFormat {
            requested: "32 位浮点".into(),
            supported: format!("未知格式标签 {tag}"),
        });
    };

    Ok(StreamFormat {
        sample_rate,
        channels,
        sample_format,
    })
}

fn int_format(bits: u16) -> Result<SampleFormat, SinkError> {
    match bits {
        16 => Ok(SampleFormat::I16),
        24 => Ok(SampleFormat::I24In32),
        32 => Ok(SampleFormat::I32),
        other => Err(SinkError::UnsupportedFormat {
            requested: "16/24/32 位整型".into(),
            supported: format!("{other} 位整型"),
        }),
    }
}

fn describe(device: &IMMDevice, default_id: Option<&str>) -> Result<DeviceInfo, SinkError> {
    let id = unsafe {
        let raw = device.GetId()?;
        let s = raw
            .to_string()
            .map_err(|e| SinkError::Enumeration(format!("设备 ID 解码失败: {e}")))?;
        CoTaskMemFree(Some(raw.0 as *const _));
        s
    };

    let name = unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        let value = store.GetValue(&PKEY_Device_FriendlyName)?;
        value.to_string()
    };

    let mix_format = unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let wfx = client.GetMixFormat()?;
        let parsed = parse_format(wfx);
        CoTaskMemFree(Some(wfx as *const _));
        parsed?
    };

    Ok(DeviceInfo {
        is_default: default_id == Some(id.as_str()),
        id,
        name,
        mix_format,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Output,
    Input,
}

impl Direction {
    fn flow(self) -> EDataFlow {
        match self {
            Self::Output => eRender,
            Self::Input => eCapture,
        }
    }
}

pub fn list_output_devices() -> Result<Vec<DeviceInfo>, SinkError> {
    list_devices(Direction::Output)
}

pub fn list_devices(direction: Direction) -> Result<Vec<DeviceInfo>, SinkError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| SinkError::Enumeration(format!("创建设备枚举器失败: {e}")))?;

        let default_id = enumerator
            .GetDefaultAudioEndpoint(direction.flow(), eConsole)
            .ok()
            .and_then(|d| d.GetId().ok())
            .and_then(|raw| {
                let s = raw.to_string().ok();
                CoTaskMemFree(Some(raw.0 as *const _));
                s
            });

        let collection = enumerator
            .EnumAudioEndpoints(direction.flow(), DEVICE_STATE_ACTIVE)
            .map_err(|e| SinkError::Enumeration(format!("枚举端点失败: {e}")))?;

        let count = collection.GetCount()?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = collection.Item(i)?;
            match describe(&device, default_id.as_deref()) {
                Ok(info) => out.push(info),
                Err(_) => continue,
            }
        }
        Ok(out)
    }
}

pub fn find_device_by_id(id: &str) -> Result<IMMDevice, SinkError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| SinkError::Enumeration(format!("创建设备枚举器失败: {e}")))?;
        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        enumerator
            .GetDevice(PCWSTR(wide.as_ptr()))
            .map_err(|_| SinkError::DeviceNotFound(id.to_string()))
    }
}

pub fn find_device_by_name(needle: &str) -> Result<DeviceInfo, SinkError> {
    find_device_by_name_in(needle, Direction::Output)
}

pub fn find_device_by_name_in(needle: &str, direction: Direction) -> Result<DeviceInfo, SinkError> {
    list_devices(direction)?
        .into_iter()
        .find(|d| d.name.contains(needle))
        .ok_or_else(|| SinkError::DeviceNotFound(needle.to_string()))
}
