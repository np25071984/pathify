//! Pure spatial math over the trace model.
//!
//! Everything here is a plain function on model types: no I/O, no formats, no
//! terminal. That keeps the numerically fiddly parts — where the real bugs
//! live — testable in isolation.

pub mod dedup;
pub mod distance;
pub mod drift;
pub mod elevation;
pub mod privacy;

pub use distance::{distance, segment_length, trace_length};
pub use drift::{
    GPS_NOISE_MARGIN_M, MAX_PLAUSIBLE_SPEED_MPS, filter_drift, implied_speed, is_plausible_step,
};
pub use elevation::{DEFAULT_NOISE_THRESHOLD_M, ElevationStats, elevation_stats};
pub use privacy::{Geofence, redact, trim_ends};
