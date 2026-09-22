use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use deviceout_core::{BridgeStats, Counters, PullOutcome, RingConsumer, RingLimit};

const QUIET_S: f64 = 0.1;

pub type SourceId = u64;

#[derive(Debug)]
enum Order {
    Join(SourceId, RingConsumer),
    Leave(SourceId),
}

#[derive(Debug, Default)]
pub struct SourceBus {
    inbox: Mutex<Vec<Order>>,
    next_id: AtomicU64,
    joined: AtomicUsize,
    live: AtomicUsize,
}

impl SourceBus {
    pub fn join(&self, consumer: RingConsumer) -> SourceId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(Order::Join(id, consumer));
        }
        id
    }

    pub fn leave(&self, id: SourceId) {
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(Order::Leave(id));
        }
    }

    pub fn joined(&self) -> usize {
        self.joined.load(Ordering::Relaxed)
    }

    pub fn live(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
struct Source {
    id: SourceId,
    consumer: RingConsumer,
    live: bool,
    quiet_s: f64,
    seen: Counters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MixReport {
    pub fill: usize,
    pub live: usize,
    pub joined: usize,
    pub limit: RingLimit,
}

#[derive(Debug)]
pub(crate) struct Mixer {
    bus: Arc<SourceBus>,
    stats: Arc<BridgeStats>,
    sources: Vec<Source>,
    scratch: Vec<f32>,
    channels: usize,
    reported: Counters,
}

impl Mixer {
    pub(crate) fn new(bus: Arc<SourceBus>, stats: Arc<BridgeStats>, channels: usize) -> Self {
        Self {
            bus,
            stats,
            sources: Vec::new(),
            scratch: Vec::new(),
            channels,
            reported: Counters::default(),
        }
    }

    pub(crate) fn reserve(&mut self, samples: usize) {
        if self.scratch.len() < samples {
            self.scratch.resize(samples, 0.0);
        }
    }

    pub(crate) fn sync(&mut self) {
        let Ok(mut inbox) = self.bus.inbox.try_lock() else {
            return;
        };
        for order in inbox.drain(..) {
            match order {
                Order::Join(id, consumer) if consumer.channels() == self.channels => {
                    let seen = consumer.stats().counters();
                    self.reported = self.reported.peak(&seen);
                    self.sources.push(Source {
                        id,
                        consumer,
                        live: false,
                        quiet_s: 0.0,
                        seen,
                    });
                }
                Order::Join(..) => {}
                Order::Leave(id) => self.sources.retain(|s| s.id != id),
            }
        }
        self.bus.joined.store(self.sources.len(), Ordering::Relaxed);
    }

    pub(crate) fn fill(&self) -> usize {
        self.sources
            .iter()
            .map(|s| s.consumer.available_frames())
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn trim(&mut self, target: usize) -> usize {
        let mut discarded = 0;
        for source in &mut self.sources {
            let fill = source.consumer.available_frames();
            if fill > target {
                discarded += source.consumer.discard(fill - target);
            }
        }
        discarded
    }

    pub(crate) fn pull(&mut self, out: &mut [f32], dt_s: f64, target: usize) -> MixReport {
        self.sync();

        let Self {
            sources,
            scratch,
            stats,
            channels,
            bus,
            reported,
        } = self;

        let span = out.len();
        let mut busiest = Counters::default();
        let mut mixed = 0usize;
        let mut starved = 0usize;
        let mut widest_gap = 0u64;
        let mut fill = 0usize;

        for source in sources.iter_mut() {
            let now = source.consumer.stats().counters();
            let delta = now.since(&source.seen);
            source.seen = now;

            if delta.frames_pushed > 0 {
                source.quiet_s = 0.0;
            } else {
                source.quiet_s += dt_s;
            }

            busiest = busiest.peak(&now);

            if !source.live && source.quiet_s < QUIET_S {
                source.live = source.consumer.available_frames() >= target;
            }
            if !source.live {
                continue;
            }

            let outcome = if mixed == 0 {
                source.consumer.pull(out)
            } else {
                let outcome = source.consumer.pull(&mut scratch[..span]);
                for (sum, add) in out.iter_mut().zip(&scratch[..span]) {
                    *sum += *add;
                }
                outcome
            };
            mixed += 1;
            fill = fill.max(source.consumer.available_frames());

            if let PullOutcome::Underrun { filled } = outcome {
                starved += 1;
                widest_gap = widest_gap.max(filled as u64);
                if source.quiet_s >= QUIET_S {
                    source.live = false;
                }
            }
        }

        if mixed == 0 {
            out.fill(0.0);
        }

        let mut ledger = busiest.since(reported);
        *reported = reported.peak(&busiest);
        ledger.frames_pulled = 0;
        ledger.underrun_events = 0;
        ledger.samples_zero_filled = 0;

        let limit = if mixed == 0 {
            RingLimit::Dry
        } else if ledger.overrun_events > 0 {
            RingLimit::Full
        } else if starved == mixed {
            RingLimit::Empty
        } else {
            RingLimit::Free
        };

        if mixed > 0 {
            ledger.frames_pulled = (span / *channels) as u64;
            if limit == RingLimit::Empty {
                ledger.underrun_events = 1;
                ledger.samples_zero_filled = widest_gap;
            }
        }
        stats.add(&ledger);
        bus.live.store(mixed, Ordering::Relaxed);

        MixReport {
            fill,
            live: mixed,
            joined: sources.len(),
            limit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deviceout_core::{ring, RingProducer};

    const CHANNELS: usize = 2;
    const TARGET: usize = 8;
    const BLOCK: usize = 4;
    const DT: f64 = 0.01;

    fn bus() -> Arc<SourceBus> {
        Arc::new(SourceBus::default())
    }

    fn mixer(bus: &Arc<SourceBus>) -> (Mixer, Arc<BridgeStats>) {
        let stats = Arc::new(BridgeStats::default());
        let mut mixer = Mixer::new(Arc::clone(bus), Arc::clone(&stats), CHANNELS);
        mixer.reserve(BLOCK * CHANNELS);
        (mixer, stats)
    }

    fn feed(tx: &mut RingProducer, frames: usize, value: f32) {
        tx.push(&vec![value; frames * CHANNELS]);
    }

    fn join(bus: &SourceBus, value: f32, frames: usize) -> (RingProducer, SourceId) {
        let (mut tx, rx) = ring(64, CHANNELS);
        feed(&mut tx, frames, value);
        let id = bus.join(rx);
        (tx, id)
    }

    #[test]
    fn a_source_stays_out_of_the_mix_until_it_has_a_full_target() {
        let bus = bus();
        let (mut mixer, _) = mixer(&bus);
        let (mut tx, _) = join(&bus, 1.0, TARGET - 1);

        let mut out = vec![-1.0f32; BLOCK * CHANNELS];
        let report = mixer.pull(&mut out, DT, TARGET);
        assert_eq!(report.limit, RingLimit::Dry);
        assert_eq!(report.live, 0);
        assert_eq!(report.joined, 1);
        assert!(out.iter().all(|&s| s == 0.0), "还没攒够就出声了");

        feed(&mut tx, 4, 1.0);
        let report = mixer.pull(&mut out, DT, TARGET);
        assert_eq!(report.live, 1);
        assert_eq!(report.limit, RingLimit::Free);
        assert!(out.iter().all(|&s| s == 1.0));
    }

    #[test]
    fn two_sources_are_summed_not_picked() {
        let bus = bus();
        let (mut mixer, _) = mixer(&bus);
        let (_a, _) = join(&bus, 0.25, TARGET);
        let (_b, _) = join(&bus, 0.5, TARGET);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        let report = mixer.pull(&mut out, DT, TARGET);

        assert_eq!(report.live, 2);
        assert!(
            out.iter().all(|&s| (s - 0.75).abs() < 1e-6),
            "两路没有相加: {out:?}"
        );
    }

    #[test]
    fn one_silent_track_never_drags_the_others_down() {
        let bus = bus();
        let (mut mixer, stats) = mixer(&bus);
        let (mut alive, _) = join(&bus, 1.0, TARGET);
        let (_dead, _) = join(&bus, 1.0, TARGET);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        let mut report = mixer.pull(&mut out, DT, TARGET);
        assert_eq!(report.live, 2);

        for _ in 0..60 {
            feed(&mut alive, BLOCK, 1.0);
            report = mixer.pull(&mut out, DT, TARGET);
            assert_ne!(report.limit, RingLimit::Empty, "一路停供不该算作整体欠载");
        }

        assert_eq!(report.live, 1, "停供的那一路应当退出混音");
        assert_eq!(stats.underrun_events(), 0, "停供被记成了断流");
        assert!(out.iter().all(|&s| (s - 1.0).abs() < 1e-6));
    }

    #[test]
    fn a_track_that_comes_back_rejoins_after_it_refills() {
        let bus = bus();
        let (mut mixer, _) = mixer(&bus);
        let (mut tx, _) = join(&bus, 1.0, TARGET);
        let mut out = vec![0.0f32; BLOCK * CHANNELS];

        for _ in 0..60 {
            mixer.pull(&mut out, DT, TARGET);
        }
        assert_eq!(mixer.pull(&mut out, DT, TARGET).live, 0);

        feed(&mut tx, TARGET, 1.0);
        let report = mixer.pull(&mut out, DT, TARGET);
        assert_eq!(report.live, 1);
        assert_eq!(report.limit, RingLimit::Free);
    }

    #[test]
    fn everyone_running_dry_together_is_a_real_underrun() {
        let bus = bus();
        let (mut mixer, stats) = mixer(&bus);
        let (_a, _) = join(&bus, 1.0, TARGET);
        let (_b, _) = join(&bus, 1.0, TARGET);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        assert_eq!(mixer.pull(&mut out, DT, TARGET).live, 2);
        assert_eq!(mixer.pull(&mut out, DT, TARGET).limit, RingLimit::Free);
        let report = mixer.pull(&mut out, DT, TARGET);

        assert_eq!(report.limit, RingLimit::Empty);
        assert_eq!(stats.underrun_events(), 1, "整体欠载只该记一次");
    }

    #[test]
    fn the_ledger_counts_the_bridge_once_not_once_per_track() {
        let bus = bus();
        let (mut mixer, stats) = mixer(&bus);
        let (mut a, _) = join(&bus, 1.0, TARGET);
        let (mut b, _) = join(&bus, 1.0, TARGET);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        mixer.pull(&mut out, DT, TARGET);
        feed(&mut a, BLOCK, 1.0);
        feed(&mut b, BLOCK, 1.0);
        mixer.pull(&mut out, DT, TARGET);

        assert_eq!(stats.frames_pushed(), BLOCK as u64, "推入帧数按路数翻倍了");
        assert_eq!(stats.push_events(), 1);
        assert_eq!(stats.frames_pulled(), (BLOCK * 2) as u64);
    }

    #[test]
    fn two_tracks_pushing_on_alternate_periods_are_not_counted_twice() {
        let bus = bus();
        let (mut mixer, stats) = mixer(&bus);
        let (mut a, _) = join(&bus, 1.0, TARGET);
        let (mut b, _) = join(&bus, 1.0, TARGET);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        mixer.pull(&mut out, DT, TARGET);

        for round in 0..8 {
            if round % 2 == 0 {
                feed(&mut a, BLOCK * 2, 1.0);
            } else {
                feed(&mut b, BLOCK * 2, 1.0);
            }
            mixer.pull(&mut out, DT, TARGET);
        }

        assert_eq!(
            stats.frames_pushed(),
            (BLOCK * 2 * 4) as u64,
            "两路错相推送，推入帧数被逐周期取最大值累加而虚高"
        );
    }

    #[test]
    fn leaving_takes_the_track_out_of_the_mix() {
        let bus = bus();
        let (mut mixer, _) = mixer(&bus);
        let (_a, _) = join(&bus, 0.25, TARGET);
        let (_b, id) = join(&bus, 0.5, TARGET);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        assert_eq!(mixer.pull(&mut out, DT, TARGET).live, 2);

        bus.leave(id);
        let report = mixer.pull(&mut out, DT, TARGET);
        assert_eq!(report.joined, 1);
        assert_eq!(report.live, 1);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn a_ring_with_the_wrong_channel_count_is_refused() {
        let bus = bus();
        let (mut mixer, _) = mixer(&bus);
        let (_tx, rx) = ring(64, 1);
        bus.join(rx);

        let mut out = vec![0.0f32; BLOCK * CHANNELS];
        assert_eq!(mixer.pull(&mut out, DT, TARGET).joined, 0);
    }
}
