use std::sync::Arc;

use deviceout_core::ring;
use deviceout_engine::{ring_capacity_frames, start, EngineConfig, EngineState, SourceBus};

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
    let handle = start(Arc::new(SourceBus::default()), config());

    assert_eq!(handle.metrics().state(), EngineState::Failed);
    let reason = handle.metrics().last_error().expect("失败但未记录原因");
    assert!(!reason.detail.is_empty());
}

#[test]
fn the_bus_outlives_the_engine_it_was_handed_to() {
    let bus = Arc::new(SourceBus::default());
    let (_tx, rx) = ring(ring_capacity_frames(RATE, 400.0), CHANNELS);
    bus.join(rx);

    let mut handle = start(Arc::clone(&bus), config());
    handle.stop();

    let handle = start(Arc::clone(&bus), config());
    assert_eq!(handle.metrics().state(), EngineState::Failed);
    assert!(Arc::strong_count(&bus) >= 2, "总线没有被第二台引擎接手");
}

#[test]
fn stopping_twice_is_harmless() {
    let mut handle = start(Arc::new(SourceBus::default()), config());

    assert!(handle.stop());
    assert!(!handle.stop());
}

#[test]
fn a_nonsense_config_is_refused_before_any_thread_starts() {
    let handle = start(
        Arc::new(SourceBus::default()),
        EngineConfig {
            channels: 0,
            ..config()
        },
    );

    assert_eq!(handle.metrics().state(), EngineState::Failed);
    let reason = handle.metrics().last_error().expect("失败但未记录原因");
    assert_eq!(reason.kind, deviceout_engine::FaultKind::Config);
    assert!(
        reason.detail.contains("声道"),
        "错误信息没提声道数: {}",
        reason.detail
    );
}
