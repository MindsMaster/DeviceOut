use std::ptr;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    IAudioClient, IAudioRenderClient, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::device::{SampleFormat, StreamFormat};
use crate::error::SinkError;
use crate::wasapi::{find_device_by_id, parse_format, ComGuard};
use crate::AudioSink;

const WAIT_TIMEOUT_MS: u32 = 2000;

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
    stream_latency_ms: Option<f64>,
    running: bool,
    _com: ComGuard,
}

unsafe impl Send for WasapiSink {}

impl WasapiSink {
    pub fn open(device_id: &str, buffer_ms: u32) -> Result<Self, SinkError> {
        let com = ComGuard::new()?;
        let device = find_device_by_id(device_id)?;

        unsafe {
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| SinkError::init_from_hresult("激活音频客户端失败", e))?;

            let wfx = client
                .GetMixFormat()
                .map_err(|e| SinkError::init_from_hresult("读取混音格式失败", e))?;

            let format = parse_format(wfx);

            let duration_100ns = i64::from(buffer_ms) * 10_000;
            let init = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                duration_100ns,
                0,
                wfx,
                None,
            );
            CoTaskMemFree(Some(wfx as *const _));
            init.map_err(|e| SinkError::init_from_hresult("初始化音频流失败", e))?;
            let format = format?;

            let event = EventHandle::new()?;
            client
                .SetEventHandle(event.0)
                .map_err(|e| SinkError::init_from_hresult("绑定同步事件失败", e))?;

            let buffer_frames = client
                .GetBufferSize()
                .map_err(|e| SinkError::init_from_hresult("读取缓冲大小失败", e))?;

            let mut default_period = 0i64;
            client
                .GetDevicePeriod(Some(&mut default_period), None)
                .map_err(|e| SinkError::init_from_hresult("读取设备周期失败", e))?;
            let period_frames =
                (default_period as f64 * 1.0e-7 * f64::from(format.sample_rate)).round() as usize;

            let render: IAudioRenderClient = client
                .GetService()
                .map_err(|e| SinkError::init_from_hresult("获取渲染服务失败", e))?;

            let stream_latency_ms = stream_latency_ms(&client);

            Ok(Self {
                render,
                client,
                event,
                format,
                buffer_frames,
                period_frames: period_frames.max(1),
                stream_latency_ms,
                running: false,
                _com: com,
            })
        }
    }

    pub fn buffer_frames(&self) -> u32 {
        self.buffer_frames
    }

    pub fn stream_latency_ms(&self) -> Option<f64> {
        self.stream_latency_ms
    }

    pub fn prefill_silence(&mut self) -> Result<usize, SinkError> {
        unsafe {
            let padding = self
                .client
                .GetCurrentPadding()
                .map_err(|e| SinkError::from_hresult("查询缓冲余量失败", e))?;

            let free = self.buffer_frames.saturating_sub(padding);
            if free == 0 {
                return Ok(0);
            }

            let dst = self
                .render
                .GetBuffer(free)
                .map_err(|e| SinkError::from_hresult("获取设备缓冲失败", e))?;

            ptr::write_bytes(dst, 0, free as usize * self.format.frame_bytes());

            self.render
                .ReleaseBuffer(free, 0)
                .map_err(|e| SinkError::from_hresult("提交设备缓冲失败", e))?;

            Ok(free as usize)
        }
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

    fn write(&mut self, interleaved: &[f32]) -> Result<(), SinkError> {
        let channels = self.format.channels as usize;
        if !interleaved.len().is_multiple_of(channels) {
            return Err(SinkError::MisalignedBuffer {
                samples: interleaved.len(),
                channels,
            });
        }

        let total_frames = interleaved.len() / channels;
        let mut done = 0usize;

        while done < total_frames {
            self.wait_for_device()?;

            unsafe {
                let padding = self
                    .client
                    .GetCurrentPadding()
                    .map_err(|e| SinkError::from_hresult("查询缓冲余量失败", e))?;

                let free = self.buffer_frames.saturating_sub(padding) as usize;
                if free == 0 {
                    continue;
                }

                let n = free.min(total_frames - done);
                let dst = self
                    .render
                    .GetBuffer(n as u32)
                    .map_err(|e| SinkError::from_hresult("获取设备缓冲失败", e))?;

                let src = &interleaved[done * channels..(done + n) * channels];
                write_samples(dst, src, self.format.sample_format);

                self.render
                    .ReleaseBuffer(n as u32, 0)
                    .map_err(|e| SinkError::from_hresult("提交设备缓冲失败", e))?;

                done += n;
            }
        }
        Ok(())
    }
}

impl Drop for WasapiSink {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn stream_latency_ms(client: &IAudioClient) -> Option<f64> {
    let hns = unsafe { client.GetStreamLatency() }.ok()?;
    (hns > 0).then(|| hns_to_ms(hns))
}

fn hns_to_ms(hns: i64) -> f64 {
    hns as f64 / 10_000.0
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
    fn hns_converts_to_ms() {
        assert_eq!(hns_to_ms(10_000), 1.0);
        assert_eq!(hns_to_ms(132_000), 13.2);
        assert_eq!(hns_to_ms(0), 0.0);
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
