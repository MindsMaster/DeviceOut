use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use deviceout_core::{ring, DriftController, DriftResampler, DriftTuning};

struct CountingAllocator;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const SAMPLE_RATE: f64 = 48_000.0;
const CHANNELS: usize = 2;
const DAW_BLOCK: usize = 512;
const SINK_BLOCK: usize = 480;
const CAPACITY: usize = 19_200;

#[test]
fn the_hot_path_never_allocates() {
    ARMED.store(true, Ordering::Relaxed);
    let canary: Vec<u8> = Vec::with_capacity(4096);
    ARMED.store(false, Ordering::Relaxed);
    assert!(
        ALLOCATIONS.load(Ordering::Relaxed) > 0,
        "计数分配器未生效，本测试无效"
    );
    drop(canary);
    ALLOCATIONS.store(0, Ordering::Relaxed);

    let (mut tx, mut rx) = ring(CAPACITY, CHANNELS);
    let target = (CAPACITY / 2) as f64;
    let tuning = DriftTuning::default();
    let mut ctrl = DriftController::new(target, SAMPLE_RATE, 1.0, tuning);
    let mut resampler = DriftResampler::new(CHANNELS, SINK_BLOCK, 1.0, &tuning).unwrap();

    let mut pull_buf = vec![0.0f32; resampler.input_frames_max() * CHANNELS];
    let mut out_buf = vec![0.0f32; resampler.output_frames() * CHANNELS];
    let daw_block = vec![0.25f32; DAW_BLOCK * CHANNELS];

    let prime = vec![0.1f32; (target as usize) * CHANNELS];
    tx.push(&prime);
    drop(prime);

    let sink_period = SINK_BLOCK as f64 / SAMPLE_RATE;
    let source_rate = SAMPLE_RATE * (1.0 + 50.0e-6);
    let mut daw_credit = 0.0f64;

    ARMED.store(true, Ordering::Relaxed);
    for _ in 0..500 {
        daw_credit += source_rate * sink_period;
        while daw_credit >= DAW_BLOCK as f64 {
            daw_credit -= DAW_BLOCK as f64;
            tx.push(&daw_block);
        }

        let need = resampler.input_frames_next();
        let samples = need * CHANNELS;
        rx.pull(&mut pull_buf[..samples]);
        let consumed = resampler.process(&pull_buf[..samples], &mut out_buf);
        ctrl.update(rx.available_frames() as f64, sink_period);
        resampler.set_ratio(ctrl.ratio());

        let _ = rx.stats().underrun_events();
        let _ = ctrl.drift_ppm();

        if consumed.is_err() {
            ARMED.store(false, Ordering::Relaxed);
            panic!("重采样在热循环中失败: {consumed:?}");
        }
    }
    ARMED.store(false, Ordering::Relaxed);

    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    assert_eq!(
        allocations, 0,
        "输出线程的热路径里发生了 {allocations} 次堆分配"
    );

    assert!(
        rx.stats().is_clean(),
        "热循环期间出现了欠载或溢出，这轮数据不足以支撑结论：\
         欠载 {} 次，溢出 {} 次",
        rx.stats().underrun_events(),
        rx.stats().overrun_events()
    );
}
