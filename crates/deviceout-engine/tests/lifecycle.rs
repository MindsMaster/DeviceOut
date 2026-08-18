use deviceout_core::ring;
use deviceout_engine::{ring_capacity_frames, start, EngineConfig, EngineState};

const CHANNELS: usize = 2;
const RATE: f64 = 48_000.0;

fn config() -> EngineConfig {
    EngineConfig {
        device_id: "{00000000-0000-0000-0000-000000000000}".into(),
        source_rate_hz: RATE,
        channels: CHANNELS,
        ..Default::default()
    }
}

#[test]
fn ring_capacity_follows_rate_and_duration() {
    assert_eq!(ring_capacity_frames(48_000.0, 400.0), 19_200);
    assert_eq!(ring_capacity_frames(44_100.0, 400.0), 17_640);
    assert!(ring_capacity_frames(48_000.0, 0.0) >= 2);
}

#[test]
fn a_missing_device_fails_visibly_instead_of_silently() {
    let (_tx, rx) = ring(ring_capacity_frames(RATE, 400.0), CHANNELS);
    let handle = start(rx, config());

    assert_eq!(handle.metrics().state(), EngineState::Failed);
    let reason = handle.metrics().last_error().expect("失败但未记录原因");
    assert!(!reason.is_empty());
}

#[test]
fn the_consumer_comes_back_even_when_startup_fails() {
    let capacity = ring_capacity_frames(RATE, 400.0);
    let (_tx, rx) = ring(capacity, CHANNELS);

    let mut handle = start(rx, config());
    let returned = handle.stop().expect("启动失败后读取端没有交回");

    assert_eq!(returned.channels(), CHANNELS);
    assert_eq!(returned.capacity_frames(), capacity);

    let mut handle = start(returned, config());
    assert_eq!(handle.metrics().state(), EngineState::Failed);
    assert!(handle.stop().is_some());
}

#[test]
fn stopping_twice_is_harmless() {
    let (_tx, rx) = ring(ring_capacity_frames(RATE, 400.0), CHANNELS);
    let mut handle = start(rx, config());

    assert!(handle.stop().is_some());
    assert!(handle.stop().is_none());
}

#[test]
fn a_channel_count_mismatch_is_caught_before_touching_the_device() {
    let (_tx, rx) = ring(1024, 1);
    let mut handle = start(rx, config());

    assert_eq!(handle.metrics().state(), EngineState::Failed);
    let reason = handle.metrics().last_error().unwrap_or_default();
    assert!(reason.contains("声道"), "错误信息没提声道数: {reason}");
    assert!(handle.stop().is_some());
}
