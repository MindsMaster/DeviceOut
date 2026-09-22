use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deviceout_core::{ring, RingProducer};
use deviceout_engine::{
    min_target_frames, start_with, EngineConfig, EngineHandle, EngineState, FaultKind, SourceBus,
    DEFAULT_BLOCK_FRAMES,
};
use deviceout_sink::{AudioSink, SampleFormat, SinkError, StreamFormat, WriteReport};

const RATE: u32 = 48_000;
const PERIOD: usize = 480;
const CHANNELS: usize = 2;
const CAPACITY: usize = 4096;

struct FakeSink {
    channels: u16,
    writes: usize,
    fail_at: Option<(usize, fn() -> SinkError)>,
    written: Arc<AtomicUsize>,
    loudest: Arc<AtomicUsize>,
}

impl AudioSink for FakeSink {
    fn format(&self) -> StreamFormat {
        StreamFormat {
            sample_rate: RATE,
            channels: self.channels,
            sample_format: SampleFormat::F32,
        }
    }

    fn period_frames(&self) -> usize {
        PERIOD
    }

    fn buffer_frames(&self) -> usize {
        PERIOD * 4
    }

    fn prefill_silence(&mut self) -> Result<usize, SinkError> {
        Ok(PERIOD * 4)
    }

    fn start(&mut self) -> Result<(), SinkError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SinkError> {
        Ok(())
    }

    fn write(&mut self, interleaved: &[f32]) -> Result<WriteReport, SinkError> {
        self.writes += 1;
        if let Some((at, make)) = self.fail_at {
            if self.writes >= at {
                return Err(make());
            }
        }
        std::thread::sleep(Duration::from_millis(5));
        self.written
            .fetch_add(interleaved.len() / CHANNELS, Ordering::SeqCst);
        let peak = interleaved
            .iter()
            .fold(0.0f32, |acc, s| acc.max(s.abs()))
            .min(8.0);
        self.loudest
            .fetch_max((peak * 1000.0) as usize, Ordering::SeqCst);
        Ok(WriteReport {
            queued_frames: PERIOD * 3,
            thinnest_frames: PERIOD * 2,
            starved: false,
        })
    }
}

struct Harness {
    opens: Arc<AtomicUsize>,
    written: Arc<AtomicUsize>,
    loudest: Arc<AtomicUsize>,
    bus: Arc<SourceBus>,
}

impl Harness {
    fn new() -> Self {
        Self {
            opens: Arc::new(AtomicUsize::new(0)),
            written: Arc::new(AtomicUsize::new(0)),
            loudest: Arc::new(AtomicUsize::new(0)),
            bus: Arc::new(SourceBus::default()),
        }
    }

    fn feed(&self, frames: usize, value: f32) -> RingProducer {
        let (mut tx, rx) = ring(CAPACITY, CHANNELS);
        if frames > 0 {
            tx.push(&vec![value; frames * CHANNELS]);
        }
        self.bus.join(rx);
        tx
    }

    fn pump(&self, value: f32) -> Pump {
        let (mut tx, rx) = ring(CAPACITY, CHANNELS);
        self.bus.join(rx);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let block = vec![value; PERIOD * CHANNELS];
            while !flag.load(Ordering::SeqCst) {
                tx.push(&block);
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        Pump {
            stop,
            thread: Some(thread),
        }
    }

    fn peak(&self) -> f32 {
        self.loudest.load(Ordering::SeqCst) as f32 / 1000.0
    }

    fn opener(
        &self,
        channels: u16,
        plan: impl Fn(usize) -> Result<Option<(usize, fn() -> SinkError)>, SinkError> + Send + 'static,
    ) -> impl FnMut(&EngineConfig) -> Result<FakeSink, SinkError> + Send + 'static {
        let opens = Arc::clone(&self.opens);
        let written = Arc::clone(&self.written);
        let loudest = Arc::clone(&self.loudest);
        move |_cfg: &EngineConfig| {
            let n = opens.fetch_add(1, Ordering::SeqCst) + 1;
            let fail_at = plan(n)?;
            Ok(FakeSink {
                channels,
                writes: 0,
                fail_at,
                written: Arc::clone(&written),
                loudest: Arc::clone(&loudest),
            })
        }
    }

    fn start(
        &self,
        open: impl FnMut(&EngineConfig) -> Result<FakeSink, SinkError> + Send + 'static,
    ) -> EngineHandle {
        start_with(Arc::clone(&self.bus), config(), open)
    }
}

struct Pump {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn config() -> EngineConfig {
    EngineConfig {
        device_id: "fake".into(),
        source_rate_hz: f64::from(RATE),
        channels: CHANNELS,
        prime_timeout_s: 0.2,
        ..Default::default()
    }
}

fn wait_for(
    handle: &EngineHandle,
    timeout: Duration,
    pred: impl Fn(&EngineHandle) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred(handle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    pred(handle)
}

#[test]
fn a_full_ring_is_trimmed_to_target_before_streaming() {
    let h = Harness::new();
    let _tx = h.feed(CAPACITY, 0.25);
    let mut handle = h.start(h.opener(2, |_| Ok(None)));

    assert!(wait_for(&handle, Duration::from_secs(2), |h| {
        h.metrics().state() == EngineState::Running
    }));
    let target = handle.metrics().target_frames() as u64;
    assert!(
        target >= min_target_frames(DEFAULT_BLOCK_FRAMES, PERIOD) as u64,
        "目标水位低于下限: {target}"
    );
    assert_eq!(
        handle.metrics().frames_discarded(),
        CAPACITY as u64 - target
    );
    assert_eq!(handle.metrics().reconnects(), 0);
    assert!(handle.stop());
}

#[test]
fn priming_times_out_and_the_stream_still_starts() {
    let h = Harness::new();
    let _tx = h.feed(0, 0.0);
    let started = Instant::now();
    let handle = h.start(h.opener(2, |_| Ok(None)));

    assert!(wait_for(&handle, Duration::from_secs(2), |h| {
        h.metrics().state() == EngineState::Running
    }));
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(wait_for(&handle, Duration::from_secs(1), |h| {
        h.metrics().host_silence() > 0
    }));
    assert_eq!(
        handle.metrics().stats().underrun_events(),
        0,
        "宿主一帧都没给，不该记成断流"
    );
}

#[test]
fn two_tracks_reach_the_device_as_one_summed_stream() {
    let h = Harness::new();
    let _a = h.pump(0.5);
    let _b = h.pump(0.5);
    let handle = h.start(h.opener(2, |_| Ok(None)));

    assert!(wait_for(&handle, Duration::from_secs(3), |_| h.peak() > 0.8));
    assert_eq!(h.bus.joined(), 2);
    assert_eq!(h.bus.live(), 2);
    assert_eq!(h.opens.load(Ordering::SeqCst), 1, "两路各开了一次设备");
}

#[test]
fn a_track_can_join_a_stream_that_is_already_running() {
    let h = Harness::new();
    let _a = h.pump(0.5);
    let handle = h.start(h.opener(2, |_| Ok(None)));
    assert!(wait_for(&handle, Duration::from_secs(3), |_| h.peak() > 0.4));

    let _b = h.pump(0.5);
    assert!(wait_for(&handle, Duration::from_secs(3), |_| h.peak() > 0.8));
    assert_eq!(h.opens.load(Ordering::SeqCst), 1, "加一条轨重开了设备");
}

#[test]
fn device_loss_reconnects_and_keeps_the_tracks() {
    let h = Harness::new();
    let _tx = h.feed(CAPACITY, 0.0);
    let opens = Arc::clone(&h.opens);
    let mut handle = h.start(h.opener(2, |n| {
        Ok(if n == 1 {
            Some((3, || SinkError::DeviceLost("pulled".into())))
        } else {
            None
        })
    }));

    assert!(wait_for(&handle, Duration::from_secs(3), |h| {
        h.metrics().reconnects() == 1 && h.metrics().state() == EngineState::Running
    }));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(h.bus.joined(), 1, "重连丢掉了已挂载的轨道");
    assert!(handle.metrics().last_error().is_none());
    assert!(handle.stop());
}

#[test]
fn a_non_recoverable_stream_error_fails_the_engine() {
    let h = Harness::new();
    let _tx = h.feed(CAPACITY, 0.0);
    let mut handle = h.start(h.opener(2, |_| Ok(Some((2, || SinkError::Stream("bad".into()))))));

    assert!(wait_for(&handle, Duration::from_secs(2), |h| {
        h.metrics().state() == EngineState::Failed
    }));
    let fault = handle.metrics().last_error().unwrap();
    assert_eq!(fault.kind, FaultKind::Stream);
    assert!(fault.detail.contains("bad"), "{}", fault.detail);
    assert!(handle.stop());
}

#[test]
fn a_channel_mismatch_at_open_fails_without_streaming() {
    let h = Harness::new();
    let _tx = h.feed(0, 0.0);
    let written = Arc::clone(&h.written);
    let handle = h.start(h.opener(1, |_| Ok(None)));

    assert_eq!(handle.metrics().state(), EngineState::Failed);
    assert_eq!(
        handle.metrics().last_error().unwrap().kind,
        FaultKind::ChannelMismatch {
            device: 1,
            source: 2
        }
    );
    assert_eq!(written.load(Ordering::SeqCst), 0);
}

#[test]
fn an_open_failure_before_any_success_is_final() {
    let h = Harness::new();
    let handle = h.start(h.opener(2, |_| Err(SinkError::DeviceNotFound("nope".into()))));

    assert_eq!(handle.metrics().state(), EngineState::Failed);
    assert_eq!(
        handle.metrics().last_error().unwrap().kind,
        FaultKind::DeviceNotFound
    );
}

#[test]
fn stop_returns_promptly_while_streaming() {
    let h = Harness::new();
    let _tx = h.feed(CAPACITY, 0.0);
    let mut handle = h.start(h.opener(2, |_| Ok(None)));
    assert!(wait_for(&handle, Duration::from_secs(2), |h| {
        h.metrics().state() == EngineState::Running
    }));

    let started = Instant::now();
    assert!(handle.stop());
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(handle.metrics().state(), EngineState::Stopped);
}

#[test]
fn stream_metrics_reflect_the_sink_report() {
    let h = Harness::new();
    let _tx = h.feed(CAPACITY, 0.0);
    let mut handle = h.start(h.opener(2, |_| Ok(None)));
    assert!(wait_for(&handle, Duration::from_secs(2), |h| {
        h.metrics().device_queue_frames() == (PERIOD * 3) as u64
    }));
    let m = handle.metrics();
    assert_eq!(m.source_rate_hz(), f64::from(RATE));
    assert_eq!(m.sink_rate_hz(), f64::from(RATE));
    assert_eq!(m.period_frames(), PERIOD as u64);
    assert!(m.latency_ms() > 30.0, "{}", m.latency_ms());
    assert!(handle.stop());
}
