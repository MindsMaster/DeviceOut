use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(60);
const SLICE: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub(crate) struct Heartbeat {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Heartbeat {
    pub(crate) fn start(version: &'static str) -> Self {
        Self::spawn(TICK, move || {
            deviceout_update::telemetry::spawn_ping(version)
        })
    }

    fn spawn(tick: Duration, mut beat: impl FnMut() + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("deviceout-heartbeat".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    beat();
                    sleep_interruptible(tick, &flag);
                }
            })
            .ok();
        Self { stop, thread }
    }

    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.stop();
    }
}

fn sleep_interruptible(total: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + total;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stop.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(SLICE.min(left));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn the_beat_keeps_going_until_it_is_stopped() {
        let beats = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&beats);
        let mut heartbeat = Heartbeat::spawn(Duration::from_millis(20), move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let started = Instant::now();
        while beats.load(Ordering::SeqCst) < 2 && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(5));
        }
        heartbeat.stop();
        let settled = beats.load(Ordering::SeqCst);
        assert!(settled >= 2, "{settled}");
        thread::sleep(Duration::from_millis(80));
        assert_eq!(beats.load(Ordering::SeqCst), settled);
    }

    #[test]
    fn stopping_twice_is_harmless_and_joins_the_thread() {
        let mut heartbeat = Heartbeat::spawn(Duration::from_secs(3600), || {});
        heartbeat.stop();
        assert!(heartbeat.thread.is_none());
        heartbeat.stop();
    }
}
