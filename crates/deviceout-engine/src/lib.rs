pub mod error;
pub mod metrics;
pub mod worker;

pub use error::{EngineError, Fault, FaultKind};
pub use metrics::{latency_ms, EngineMetrics, EngineState};
#[cfg(windows)]
pub use worker::start;
pub use worker::{
    frames_for_ms, min_target_frames, ring_capacity_for_target, ring_capacity_frames, start_with,
    EngineConfig, EngineHandle, OpenSink, DEFAULT_BLOCK_FRAMES, DEFAULT_TARGET_MS,
};
