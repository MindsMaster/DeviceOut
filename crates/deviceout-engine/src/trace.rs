use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::metrics::EngineMetrics;
use crate::mixer::SourceBus;

const TICK: Duration = Duration::from_millis(1_000);
const ROTATE_BYTES: u64 = 4 * 1024 * 1024;

static NEXT_ENGINE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct TraceHeader {
    pub device_id: String,
    pub source_rate_hz: f64,
    pub sink_rate_hz: f64,
    pub channels: usize,
    pub max_block_frames: usize,
    pub period_frames: usize,
    pub min_period_frames: usize,
    pub queue_periods: u32,
    pub exclusive: bool,
    pub target_ms: f64,
    pub target_frames: f64,
    pub floor_frames: usize,
    pub capacity_frames: usize,
    pub resampler_delay_frames: usize,
    pub device_buffer_frames: usize,
    pub queue_limit_frames: usize,
}

#[derive(Debug)]
pub(crate) struct Trace {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Trace {
    pub(crate) fn spawn(
        path: PathBuf,
        header: TraceHeader,
        metrics: Arc<EngineMetrics>,
        sources: Arc<SourceBus>,
        host_drops: Option<&'static AtomicU64>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let engine = NEXT_ENGINE.fetch_add(1, Ordering::Relaxed);
        let thread = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("deviceout-trace".into())
                .spawn(move || {
                    run(Site {
                        path: &path,
                        engine,
                        header: &header,
                        metrics: &metrics,
                        sources: &sources,
                        host_drops,
                        stop: &stop,
                    })
                })
                .ok()
        };
        Self { stop, thread }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Site<'a> {
    path: &'a Path,
    engine: u64,
    header: &'a TraceHeader,
    metrics: &'a EngineMetrics,
    sources: &'a SourceBus,
    host_drops: Option<&'static AtomicU64>,
    stop: &'a AtomicBool,
}

fn run(site: Site<'_>) {
    let Site {
        path,
        engine,
        header,
        metrics,
        sources,
        host_drops,
        stop,
    } = site;
    let Some(mut file) = open(path) else {
        return;
    };
    let _ = writeln!(file, "{}", describe(engine, header));

    let stats = Arc::clone(metrics.stats());
    let started = Instant::now();
    let mut last = started;
    let mut pushed = stats.frames_pushed();
    let mut pulled = stats.frames_pulled();
    let mut pushes = stats.push_events();

    while wait(TICK, stop) {
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f64();
        last = now;
        if dt <= 0.0 {
            continue;
        }

        let pushed_now = stats.frames_pushed();
        let pulled_now = stats.frames_pulled();
        let pushes_now = stats.push_events();
        let push_rate = (pushed_now - pushed) as f64 / dt;
        let pull_rate = (pulled_now - pulled) as f64 / dt;
        let calls = pushes_now - pushes;
        let block = if calls > 0 {
            (pushed_now - pushed) as f64 / calls as f64
        } else {
            0.0
        };
        pushed = pushed_now;
        pulled = pulled_now;
        pushes = pushes_now;
        let (low, high) = metrics.take_fill_range();

        let line = format!(
            "e{engine} +{:.1}s push/s={push_rate:.1} pull/s={pull_rate:.1} calls={calls} \
             block={block:.0} src={}/{} fill={} lo={low} hi={high} smoothed={:.0} \
             drift={:+.1}ppm ratio={:.8} under={} over={} starve={} mute={:.2}s \
             clamp={} queued={} padlo={} hostdrop={} hostquiet={} state={:?}",
            started.elapsed().as_secs_f64(),
            sources.live(),
            sources.joined(),
            metrics.fill_frames(),
            metrics.smoothed_fill(),
            metrics.drift_ppm(),
            metrics.ratio(),
            stats.underrun_events(),
            stats.overrun_events(),
            metrics.device_starvations(),
            metrics.dropout_seconds(),
            metrics.clamp_events(),
            metrics.device_queue_frames(),
            metrics.take_device_queue_low(),
            host_drops.map_or(0, |d| d.load(Ordering::Relaxed)),
            metrics.host_silence(),
            metrics.state(),
        );
        if writeln!(file, "{line}").is_err() {
            return;
        }
        let _ = file.flush();
    }
}

fn wait(total: Duration, stop: &AtomicBool) -> bool {
    const SLICE: Duration = Duration::from_millis(100);
    let deadline = Instant::now() + total;
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        thread::sleep(left.min(SLICE));
    }
}

fn describe(engine: u64, h: &TraceHeader) -> String {
    format!(
        "=== e{engine} run {} device={} ===\n\
         source_hz={:.0} sink_hz={:.0} channels={} block={} period={} min_period={} \
         queue={} exclusive={}\n\
         target_ms={:.1} target_frames={:.0} floor_frames={} capacity={} resampler_delay={}\n\
         device_buffer={} queue_limit={}",
        env!("CARGO_PKG_VERSION"),
        h.device_id,
        h.source_rate_hz,
        h.sink_rate_hz,
        h.channels,
        h.max_block_frames,
        h.period_frames,
        h.min_period_frames,
        h.queue_periods,
        h.exclusive,
        h.target_ms,
        h.target_frames,
        h.floor_frames,
        h.capacity_frames,
        h.resampler_delay_frames,
        h.device_buffer_frames,
        h.queue_limit_frames,
    )
}

fn open(path: &Path) -> Option<std::fs::File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) >= ROTATE_BYTES {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}
