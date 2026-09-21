use std::ptr;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    IAudioClient, IAudioRenderClient, IMMDevice, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED,
    AUDCLNT_SHAREMODE, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::device::{SampleFormat, StreamFormat};
use crate::error::SinkError;
use crate::wasapi::{find_device_by_id, parse_format, ComGuard};
use crate::{AudioSink, WriteReport};

const WAIT_TIMEOUT_MS: u32 = 2000;

pub const MIN_QUEUE_PERIODS: u32 = 1;
pub const MAX_QUEUE_PERIODS: u32 = 4;
pub const DEFAULT_QUEUE_PERIODS: u32 = 2;

const HNS_PER_SECOND: f64 = 1.0e7;

#[derive(Debug, Clone, Copy)]
pub struct SinkOptions {
    pub buffer_ms: u32,
    pub queue_periods: u32,
    pub exclusive: bool,
}

impl Default for SinkOptions {
    fn default() -> Self {
        Self {
            buffer_ms: 40,
            queue_periods: DEFAULT_QUEUE_PERIODS,
            exclusive: false,
        }
    }
}

pub fn frames_of(duration_hns: i64, sample_rate: u32) -> usize {
    ((duration_hns as f64 / HNS_PER_SECOND * f64::from(sample_rate)).round() as usize).max(1)
}

pub fn duration_of(frames: u32, sample_rate: u32) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    (HNS_PER_SECOND * f64::from(frames) / f64::from(sample_rate)).round() as i64
}

pub fn device_queue_limit(period_frames: usize, buffer_frames: usize, periods: u32) -> usize {
    let periods = periods.clamp(MIN_QUEUE_PERIODS, MAX_QUEUE_PERIODS) as usize;
    period_frames
        .saturating_mul(periods)
        .clamp(1, buffer_frames.max(1))
}

#[derive(Debug)]
struct EventHandle(HANDLE);

impl EventHandle {
    fn new() -> Result<Self, SinkError> {
        let handle = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| SinkError::init_from_hresult("创建同步事件失败", e))?;
        Ok(Self(handle))
    }
}

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[derive(Debug)]
pub struct WasapiSink {
    render: IAudioRenderClient,
    client: IAudioClient,
    event: EventHandle,
    format: StreamFormat,
    buffer_frames: u32,
    period_frames: usize,
    queue_limit: usize,
    exclusive: bool,
    running: bool,
    _com: ComGuard,
}

unsafe impl Send for WasapiSink {}

struct Stream {
    render: IAudioRenderClient,
    client: IAudioClient,
    event: EventHandle,
    format: StreamFormat,
    buffer_frames: u32,
    period_frames: usize,
    exclusive: bool,
}

struct OwnedFormat(Vec<u32>);

impl OwnedFormat {
    unsafe fn copy_of(wfx: *const WAVEFORMATEX) -> Self {
        let bytes = std::mem::size_of::<WAVEFORMATEX>() + usize::from(unsafe { (*wfx).cbSize });
        let mut words = vec![0u32; bytes.div_ceil(std::mem::size_of::<u32>())];
        unsafe {
            ptr::copy_nonoverlapping(wfx.cast::<u8>(), words.as_mut_ptr().cast::<u8>(), bytes);
        }
        Self(words)
    }

    fn as_ptr(&self) -> *const WAVEFORMATEX {
        self.0.as_ptr().cast()
    }
}

unsafe fn activate(device: &IMMDevice) -> Result<IAudioClient, SinkError> {
    unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|e| SinkError::init_from_hresult("激活音频客户端失败", e))
}

unsafe fn open_stream(
    device: &IMMDevice,
    options: &SinkOptions,
    exclusive: bool,
) -> Result<Stream, SinkError> {
    let mut client = unsafe { activate(device) }?;

    let wfx = {
        let raw = unsafe { client.GetMixFormat() }
            .map_err(|e| SinkError::init_from_hresult("读取混音格式失败", e))?;
        let owned = unsafe { OwnedFormat::copy_of(raw) };
        unsafe { CoTaskMemFree(Some(raw as *const _)) };
        owned
    };
    let format = unsafe { parse_format(wfx.as_ptr()) }?;

    let mut default_period = 0i64;
    let mut min_period = 0i64;
    unsafe { client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period)) }
        .map_err(|e| SinkError::init_from_hresult("读取设备周期失败", e))?;

    let mode: AUDCLNT_SHAREMODE = if exclusive {
        unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wfx.as_ptr(), None) }
            .ok()
            .map_err(|e| SinkError::init_from_hresult("设备不接受当前格式的独占流", e))?;
        AUDCLNT_SHAREMODE_EXCLUSIVE
    } else {
        AUDCLNT_SHAREMODE_SHARED
    };

    let (mut duration, mut periodicity) = if exclusive {
        let period = if min_period > 0 {
            min_period
        } else {
            default_period
        };
        (period, period)
    } else {
        (i64::from(options.buffer_ms) * 10_000, 0)
    };

    let mut init = unsafe {
        client.Initialize(
            mode,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            duration,
            periodicity,
            wfx.as_ptr(),
            None,
        )
    };

    if exclusive
        && init
            .as_ref()
            .is_err_and(|e| e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
    {
        let aligned = unsafe { client.GetBufferSize() }
            .map_err(|e| SinkError::init_from_hresult("读取对齐后的缓冲大小失败", e))?;
        duration = duration_of(aligned, format.sample_rate);
        periodicity = duration;
        client = unsafe { activate(device) }?;
        init = unsafe {
            client.Initialize(
                mode,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                duration,
                periodicity,
                wfx.as_ptr(),
                None,
            )
        };
    }
    init.map_err(|e| SinkError::init_from_hresult("初始化音频流失败", e))?;

    let event = EventHandle::new()?;
    unsafe { client.SetEventHandle(event.0) }
        .map_err(|e| SinkError::init_from_hresult("绑定同步事件失败", e))?;

    let buffer_frames = unsafe { client.GetBufferSize() }
        .map_err(|e| SinkError::init_from_hresult("读取缓冲大小失败", e))?;

    let period_frames = if exclusive {
        frames_of(periodicity, format.sample_rate)
    } else {
        frames_of(default_period, format.sample_rate)
    };

    let render: IAudioRenderClient = unsafe { client.GetService() }
        .map_err(|e| SinkError::init_from_hresult("获取渲染服务失败", e))?;

    Ok(Stream {
        render,
        client,
        event,
        format,
        buffer_frames,
        period_frames,
        exclusive,
    })
}

impl WasapiSink {
    pub fn open(device_id: &str, options: SinkOptions) -> Result<Self, SinkError> {
        let com = ComGuard::new()?;
        let device = find_device_by_id(device_id)?;

        let stream = if options.exclusive {
            match unsafe { open_stream(&device, &options, true) } {
                Ok(stream) => Ok(stream),
                Err(e) => {
                    eprintln!("独占模式不可用，回落共享模式: {e}");
                    unsafe { open_stream(&device, &options, false) }
                }
            }
        } else {
            unsafe { open_stream(&device, &options, false) }
        }?;

        let queue_limit = device_queue_limit(
            stream.period_frames,
            stream.buffer_frames as usize,
            options.queue_periods,
        );
        Ok(Self {
            render: stream.render,
            client: stream.client,
            event: stream.event,
            format: stream.format,
            buffer_frames: stream.buffer_frames,
            period_frames: stream.period_frames,
            queue_limit,
            exclusive: stream.exclusive,
            running: false,
            _com: com,
        })
    }

    pub fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    pub fn queue_limit_frames(&self) -> usize {
        self.queue_limit
    }

    fn padding(&self) -> Result<u32, SinkError> {
        unsafe { self.client.GetCurrentPadding() }
            .map_err(|e| SinkError::from_hresult("查询缓冲余量失败", e))
    }

    fn wait_for_device(&self) -> Result<(), SinkError> {
        let state = unsafe { WaitForSingleObject(self.event.0, WAIT_TIMEOUT_MS) };
        if state == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(SinkError::DeviceLost(format!(
                "等待设备超时（{state:?}），已 {WAIT_TIMEOUT_MS} ms 没有推进"
            )))
        }
    }
}

impl AudioSink for WasapiSink {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn period_frames(&self) -> usize {
        self.period_frames
    }

    fn buffer_frames(&self) -> usize {
        self.buffer_frames as usize
    }

    fn queue_limit_frames(&self) -> usize {
        self.queue_limit
    }

    fn exclusive(&self) -> bool {
        self.exclusive
    }

    fn prefill_silence(&mut self) -> Result<usize, SinkError> {
        let room = self.queue_limit.saturating_sub(self.padding()? as usize) as u32;
        if room == 0 {
            return Ok(0);
        }
        unsafe {
            let dst = self
                .render
                .GetBuffer(room)
                .map_err(|e| SinkError::from_hresult("获取设备缓冲失败", e))?;
            ptr::write_bytes(dst, 0, room as usize * self.format.frame_bytes());
            self.render
                .ReleaseBuffer(room, 0)
                .map_err(|e| SinkError::from_hresult("提交设备缓冲失败", e))?;
        }
        Ok(room as usize)
    }

    fn start(&mut self) -> Result<(), SinkError> {
        if self.running {
            return Ok(());
        }
        unsafe { self.client.Start() }.map_err(|e| SinkError::from_hresult("启动音频流失败", e))?;
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SinkError> {
        if !self.running {
            return Ok(());
        }
        unsafe { self.client.Stop() }.map_err(|e| SinkError::from_hresult("停止音频流失败", e))?;
        self.running = false;
        Ok(())
    }

    fn write(&mut self, interleaved: &[f32]) -> Result<WriteReport, SinkError> {
        let channels = self.format.channels as usize;
        if !interleaved.len().is_multiple_of(channels) {
            return Err(SinkError::MisalignedBuffer {
                samples: interleaved.len(),
                channels,
            });
        }

        let total_frames = interleaved.len() / channels;
        let mut done = 0usize;
        let mut report = WriteReport::default();

        while done < total_frames {
            let padding = self.padding()? as usize;
            let room = self.queue_limit.saturating_sub(padding);
            if room == 0 {
                self.wait_for_device()?;
                continue;
            }
            if padding == 0 && self.running {
                report.starved = true;
            }

            let n = room.min(total_frames - done);
            unsafe {
                let dst = self
                    .render
                    .GetBuffer(n as u32)
                    .map_err(|e| SinkError::from_hresult("获取设备缓冲失败", e))?;

                let src = &interleaved[done * channels..(done + n) * channels];
                write_samples(dst, src, self.format.sample_format);

                self.render
                    .ReleaseBuffer(n as u32, 0)
                    .map_err(|e| SinkError::from_hresult("提交设备缓冲失败", e))?;
            }

            done += n;
            report.queued_frames = padding + n;
        }
        Ok(report)
    }
}

impl Drop for WasapiSink {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

unsafe fn write_samples(dst: *mut u8, src: &[f32], fmt: SampleFormat) {
    match fmt {
        SampleFormat::F32 => {
            unsafe { ptr::copy_nonoverlapping(src.as_ptr(), dst.cast::<f32>(), src.len()) };
        }
        SampleFormat::I16 => {
            let out = dst.cast::<i16>();
            for (i, &s) in src.iter().enumerate() {
                let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                unsafe { out.add(i).write_unaligned(v) };
            }
        }
        SampleFormat::I32 => {
            let out = dst.cast::<i32>();
            for (i, &s) in src.iter().enumerate() {
                let v = (f64::from(s.clamp(-1.0, 1.0)) * f64::from(i32::MAX)) as i32;
                unsafe { out.add(i).write_unaligned(v) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_and_durations_round_trip() {
        assert_eq!(frames_of(100_000, 48_000), 480);
        assert_eq!(frames_of(30_000, 48_000), 144);
        assert_eq!(duration_of(480, 48_000), 100_000);
        assert_eq!(duration_of(144, 48_000), 30_000);
        assert_eq!(frames_of(0, 48_000), 1);
        assert_eq!(duration_of(480, 0), 0);
    }

    #[test]
    fn the_device_queue_holds_the_requested_periods_not_the_whole_buffer() {
        assert_eq!(device_queue_limit(480, 4_800, 2), 960);
        assert_eq!(device_queue_limit(480, 4_800, 3), 1_440);
        assert_eq!(device_queue_limit(480, 4_800, 4), 1_920);
        assert_eq!(device_queue_limit(128, 4_800, 2), 256);
    }

    #[test]
    fn a_silly_period_count_is_pulled_back_into_range() {
        assert_eq!(device_queue_limit(480, 4_800, 0), 480);
        assert_eq!(device_queue_limit(480, 4_800, 1), 480);
        assert_eq!(device_queue_limit(480, 4_800, u32::MAX), 1_920);
    }

    #[test]
    fn the_device_queue_never_exceeds_the_allocated_buffer() {
        assert_eq!(device_queue_limit(480, 480, 2), 480);
        assert_eq!(device_queue_limit(480, 0, 2), 1);
        assert_eq!(device_queue_limit(usize::MAX, 1_920, 2), 1_920);
    }

    fn rendered(fmt: SampleFormat, src: &[f32]) -> Vec<u8> {
        let mut buf = vec![0xAAu8; src.len() * fmt.bytes() + 8];
        unsafe { write_samples(buf.as_mut_ptr(), src, fmt) };
        assert!(buf[src.len() * fmt.bytes()..].iter().all(|&b| b == 0xAA));
        buf.truncate(src.len() * fmt.bytes());
        buf
    }

    #[test]
    fn every_format_writes_exactly_bytes_per_sample() {
        let src = [1.0f32, -1.0, 0.0, 2.0];
        let i16s: Vec<i16> = rendered(SampleFormat::I16, &src)
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(i16s, [i16::MAX, -i16::MAX, 0, i16::MAX]);

        let i32s: Vec<i32> = rendered(SampleFormat::I32, &src)
            .chunks(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(i32s, [i32::MAX, -i32::MAX, 0, i32::MAX]);

        let f32s: Vec<f32> = rendered(SampleFormat::F32, &src)
            .chunks(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(f32s, src);
    }
}
