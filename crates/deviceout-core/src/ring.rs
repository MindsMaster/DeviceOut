use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Ok,
    Overrun { dropped: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullOutcome {
    Ok,
    Underrun { filled: usize },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub frames_pushed: u64,
    pub push_events: u64,
    pub frames_pulled: u64,
    pub overrun_events: u64,
    pub underrun_events: u64,
    pub samples_dropped: u64,
    pub samples_zero_filled: u64,
}

impl Counters {
    pub fn since(&self, earlier: &Counters) -> Counters {
        Counters {
            frames_pushed: self.frames_pushed.saturating_sub(earlier.frames_pushed),
            push_events: self.push_events.saturating_sub(earlier.push_events),
            frames_pulled: self.frames_pulled.saturating_sub(earlier.frames_pulled),
            overrun_events: self.overrun_events.saturating_sub(earlier.overrun_events),
            underrun_events: self.underrun_events.saturating_sub(earlier.underrun_events),
            samples_dropped: self.samples_dropped.saturating_sub(earlier.samples_dropped),
            samples_zero_filled: self
                .samples_zero_filled
                .saturating_sub(earlier.samples_zero_filled),
        }
    }

    pub fn peak(&self, other: &Counters) -> Counters {
        Counters {
            frames_pushed: self.frames_pushed.max(other.frames_pushed),
            push_events: self.push_events.max(other.push_events),
            frames_pulled: self.frames_pulled.max(other.frames_pulled),
            overrun_events: self.overrun_events.max(other.overrun_events),
            underrun_events: self.underrun_events.max(other.underrun_events),
            samples_dropped: self.samples_dropped.max(other.samples_dropped),
            samples_zero_filled: self.samples_zero_filled.max(other.samples_zero_filled),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Counters::default()
    }
}

#[derive(Debug, Default)]
pub struct BridgeStats {
    frames_pushed: AtomicU64,
    push_events: AtomicU64,
    frames_pulled: AtomicU64,
    overrun_events: AtomicU64,
    underrun_events: AtomicU64,
    samples_dropped: AtomicU64,
    samples_zero_filled: AtomicU64,
}

impl BridgeStats {
    pub fn counters(&self) -> Counters {
        Counters {
            frames_pushed: self.frames_pushed(),
            push_events: self.push_events(),
            frames_pulled: self.frames_pulled(),
            overrun_events: self.overrun_events(),
            underrun_events: self.underrun_events(),
            samples_dropped: self.samples_dropped(),
            samples_zero_filled: self.samples_zero_filled(),
        }
    }

    pub fn add(&self, delta: &Counters) {
        let bump = |slot: &AtomicU64, by: u64| {
            if by > 0 {
                slot.fetch_add(by, Ordering::Relaxed);
            }
        };
        bump(&self.frames_pushed, delta.frames_pushed);
        bump(&self.push_events, delta.push_events);
        bump(&self.frames_pulled, delta.frames_pulled);
        bump(&self.overrun_events, delta.overrun_events);
        bump(&self.underrun_events, delta.underrun_events);
        bump(&self.samples_dropped, delta.samples_dropped);
        bump(&self.samples_zero_filled, delta.samples_zero_filled);
    }

    pub fn frames_pushed(&self) -> u64 {
        self.frames_pushed.load(Ordering::Relaxed)
    }

    pub fn push_events(&self) -> u64 {
        self.push_events.load(Ordering::Relaxed)
    }

    pub fn frames_pulled(&self) -> u64 {
        self.frames_pulled.load(Ordering::Relaxed)
    }

    pub fn overrun_events(&self) -> u64 {
        self.overrun_events.load(Ordering::Relaxed)
    }

    pub fn underrun_events(&self) -> u64 {
        self.underrun_events.load(Ordering::Relaxed)
    }

    pub fn samples_dropped(&self) -> u64 {
        self.samples_dropped.load(Ordering::Relaxed)
    }

    pub fn samples_zero_filled(&self) -> u64 {
        self.samples_zero_filled.load(Ordering::Relaxed)
    }

    pub fn is_clean(&self) -> bool {
        self.overrun_events() == 0 && self.underrun_events() == 0
    }

    fn record_push(&self, frames: usize, dropped: usize) {
        self.frames_pushed
            .fetch_add(frames as u64, Ordering::Relaxed);
        self.push_events.fetch_add(1, Ordering::Relaxed);
        if dropped > 0 {
            self.overrun_events.fetch_add(1, Ordering::Relaxed);
            self.samples_dropped
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
    }

    fn record_pull(&self, frames: usize, zero_filled: usize) {
        self.frames_pulled
            .fetch_add(frames as u64, Ordering::Relaxed);
        if zero_filled > 0 {
            self.underrun_events.fetch_add(1, Ordering::Relaxed);
            self.samples_zero_filled
                .fetch_add(zero_filled as u64, Ordering::Relaxed);
        }
    }
}

#[derive(Debug)]
pub struct RingProducer {
    inner: rtrb::Producer<f32>,
    stats: Arc<BridgeStats>,
    channels: usize,
}

fn whole_frames(samples: usize, channels: usize) -> usize {
    samples - samples % channels
}

impl RingProducer {
    pub fn push(&mut self, interleaved: &[f32]) -> PushOutcome {
        let mut written = 0;
        let room = whole_frames(interleaved.len().min(self.inner.slots()), self.channels);
        if let Ok(chunk) = self.inner.write_chunk_uninit(room) {
            written = chunk.fill_from_iter(interleaved.iter().copied());
        }

        let dropped = interleaved.len() - written;
        self.stats.record_push(written / self.channels, dropped);

        if dropped == 0 {
            PushOutcome::Ok
        } else {
            PushOutcome::Overrun { dropped }
        }
    }

    pub fn stats(&self) -> &Arc<BridgeStats> {
        &self.stats
    }
}

#[derive(Debug)]
pub struct RingConsumer {
    inner: rtrb::Consumer<f32>,
    stats: Arc<BridgeStats>,
    channels: usize,
    capacity: usize,
}

impl RingConsumer {
    pub fn pull(&mut self, out: &mut [f32]) -> PullOutcome {
        let available = self.inner.slots();
        let take = whole_frames(available.min(out.len()), self.channels);

        if take > 0 {
            if let Ok(chunk) = self.inner.read_chunk(take) {
                let (first, second) = chunk.as_slices();
                out[..first.len()].copy_from_slice(first);
                out[first.len()..first.len() + second.len()].copy_from_slice(second);
                chunk.commit_all();
            }
        }

        let filled = out.len() - take;
        if filled > 0 {
            out[take..].fill(0.0);
        }
        self.stats.record_pull(take / self.channels, filled);

        if filled == 0 {
            PullOutcome::Ok
        } else {
            PullOutcome::Underrun { filled }
        }
    }

    pub fn discard(&mut self, frames: usize) -> usize {
        let take = (frames * self.channels).min(self.inner.slots());
        if take == 0 {
            return 0;
        }
        match self.inner.read_chunk(take) {
            Ok(chunk) => {
                chunk.commit_all();
                take / self.channels
            }
            Err(_) => 0,
        }
    }

    pub fn available_frames(&self) -> usize {
        self.inner.slots() / self.channels
    }

    pub fn capacity_frames(&self) -> usize {
        self.capacity / self.channels
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn stats(&self) -> &Arc<BridgeStats> {
        &self.stats
    }
}

pub fn ring(capacity_frames: usize, channels: usize) -> (RingProducer, RingConsumer) {
    assert!(capacity_frames > 0, "容量必须为正");
    assert!(channels > 0, "声道数必须为正");

    let capacity = capacity_frames * channels;
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let stats = Arc::new(BridgeStats::default());

    (
        RingProducer {
            inner: producer,
            stats: Arc::clone(&stats),
            channels,
        },
        RingConsumer {
            inner: consumer,
            stats,
            channels,
            capacity,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_samples() {
        let (mut tx, mut rx) = ring(64, 2);
        let input: Vec<f32> = (0..32).map(|i| i as f32).collect();
        assert_eq!(tx.push(&input), PushOutcome::Ok);

        let mut out = vec![-1.0f32; 32];
        assert_eq!(rx.pull(&mut out), PullOutcome::Ok);
        assert_eq!(out, input);
    }

    #[test]
    fn overrun_drops_and_reports() {
        let (mut tx, _rx) = ring(4, 2);
        let input = vec![1.0f32; 16];

        match tx.push(&input) {
            PushOutcome::Overrun { dropped } => assert_eq!(dropped, 8),
            other => panic!("应当溢出，实际: {other:?}"),
        }
        assert_eq!(tx.stats().overrun_events(), 1);
        assert_eq!(tx.stats().samples_dropped(), 8);
    }

    #[test]
    fn underrun_fills_zeros_and_reports() {
        let (mut tx, mut rx) = ring(64, 2);
        tx.push(&[1.0, 2.0, 3.0, 4.0]);

        let mut out = vec![-1.0f32; 10];
        match rx.pull(&mut out) {
            PullOutcome::Underrun { filled } => assert_eq!(filled, 6),
            other => panic!("应当欠载，实际: {other:?}"),
        }
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
        assert!(out[4..].iter().all(|&s| s == 0.0));
        assert_eq!(rx.stats().underrun_events(), 1);
        assert_eq!(rx.stats().samples_zero_filled(), 6);
    }

    #[test]
    fn survives_wraparound() {
        let (mut tx, mut rx) = ring(8, 1);
        let mut expected = 0.0f32;
        let mut out = [0.0f32; 5];

        for _ in 0..100 {
            let batch: Vec<f32> = (0..5).map(|i| expected + i as f32).collect();
            assert_eq!(tx.push(&batch), PushOutcome::Ok);
            assert_eq!(rx.pull(&mut out), PullOutcome::Ok);
            assert_eq!(out.to_vec(), batch, "环绕处数据错位");
            expected += 5.0;
        }
    }

    #[test]
    fn stats_track_frames_not_samples() {
        let (mut tx, mut rx) = ring(64, 2);
        tx.push(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(tx.stats().frames_pushed(), 2);

        let mut out = [0.0f32; 4];
        rx.pull(&mut out);
        assert_eq!(rx.stats().frames_pulled(), 2);
        assert!(rx.stats().is_clean());
    }

    #[test]
    fn available_frames_reflects_channel_count() {
        let (mut tx, rx) = ring(64, 2);
        tx.push(&[0.0; 20]);
        assert_eq!(rx.available_frames(), 10);
        assert_eq!(rx.capacity_frames(), 64);
    }

    #[test]
    fn discard_drops_the_oldest_frames() {
        let (mut tx, mut rx) = ring(8, 2);
        tx.push(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0]);
        assert_eq!(rx.available_frames(), 4);

        assert_eq!(rx.discard(2), 2);
        assert_eq!(rx.available_frames(), 2);

        let mut out = [0.0f32; 4];
        rx.pull(&mut out);
        assert_eq!(out, [3.0, 3.0, 4.0, 4.0], "丢弃的不是最旧的两帧");
    }

    #[test]
    fn discard_is_not_recorded_as_a_fault() {
        let (mut tx, mut rx) = ring(8, 2);
        tx.push(&[1.0; 8]);

        let before_pulled = rx.stats().frames_pulled();
        rx.discard(3);

        assert!(rx.stats().is_clean(), "丢弃被记成了欠载或溢出");
        assert_eq!(
            rx.stats().frames_pulled(),
            before_pulled,
            "丢弃被计入正常读出的帧数，吞吐统计会偏高"
        );
    }

    #[test]
    fn partial_frames_never_enter_or_leave_the_ring() {
        let (mut tx, mut rx) = ring(8, 2);
        match tx.push(&[1.0, 2.0, 3.0]) {
            PushOutcome::Overrun { dropped } => assert_eq!(dropped, 1),
            other => panic!("尾部半帧应被丢弃并记账，实际: {other:?}"),
        }
        assert_eq!(rx.available_frames(), 1);
        tx.push(&[4.0, 5.0]);

        let mut out = [9.0f32; 3];
        match rx.pull(&mut out) {
            PullOutcome::Underrun { filled } => assert_eq!(filled, 1),
            other => panic!("半帧输出应补零并记账，实际: {other:?}"),
        }
        assert_eq!(out, [1.0, 2.0, 0.0]);
        let mut rest = [0.0f32; 2];
        assert_eq!(rx.pull(&mut rest), PullOutcome::Ok);
        assert_eq!(rest, [4.0, 5.0], "左右声道错位");
    }

    #[test]
    fn a_nearly_full_ring_only_accepts_whole_frames() {
        let (mut tx, rx) = ring(4, 2);
        tx.push(&[1.0; 6]);
        match tx.push(&[7.0, 8.0, 9.0, 10.0]) {
            PushOutcome::Overrun { dropped } => assert_eq!(dropped, 2),
            other => panic!("{other:?}"),
        }
        assert_eq!(rx.available_frames(), 4);
    }

    #[test]
    fn a_delta_is_what_happened_between_two_readings() {
        let (mut tx, mut rx) = ring(8, 2);
        tx.push(&[1.0; 4]);
        let before = tx.stats().counters();

        tx.push(&[1.0; 16]);
        let mut out = [0.0f32; 16];
        rx.pull(&mut out);

        let delta = tx.stats().counters().since(&before);
        assert_eq!(delta.frames_pushed, 6);
        assert_eq!(delta.push_events, 1);
        assert_eq!(delta.overrun_events, 1);
        assert_eq!(delta.samples_dropped, 4);
        assert_eq!(delta.frames_pulled, 8);
        assert_eq!(delta.underrun_events, 0);
        assert!(!delta.is_empty());

        let now = tx.stats().counters();
        assert!(now.since(&now).is_empty());
    }

    #[test]
    fn the_peak_of_two_readings_takes_the_larger_of_each_field() {
        let busy = Counters {
            frames_pushed: 900,
            push_events: 3,
            samples_dropped: 8,
            ..Counters::default()
        };
        let idle = Counters {
            frames_pushed: 400,
            push_events: 7,
            overrun_events: 1,
            ..Counters::default()
        };

        let peak = busy.peak(&idle);
        assert_eq!(peak.frames_pushed, 900);
        assert_eq!(peak.push_events, 7);
        assert_eq!(peak.overrun_events, 1);
        assert_eq!(peak.samples_dropped, 8);
        assert_eq!(peak, idle.peak(&busy));
    }

    #[test]
    fn several_rings_add_up_into_one_ledger() {
        let total = BridgeStats::default();
        let (mut a, _ra) = ring(64, 2);
        let (mut b, _rb) = ring(64, 2);
        a.push(&[0.0; 8]);
        b.push(&[0.0; 4]);

        total.add(&a.stats().counters());
        total.add(&b.stats().counters());

        assert_eq!(total.frames_pushed(), 6);
        assert_eq!(total.push_events(), 2);
        assert!(total.is_clean());
    }

    #[test]
    fn discard_saturates_at_whats_available() {
        let (mut tx, mut rx) = ring(8, 2);
        tx.push(&[1.0; 4]);

        assert_eq!(rx.discard(100), 2, "丢弃量应当截到实际可读的帧数");
        assert_eq!(rx.available_frames(), 0);
        assert_eq!(rx.discard(1), 0, "空缓冲上丢弃应当返回 0 而不是 panic");
    }
}
