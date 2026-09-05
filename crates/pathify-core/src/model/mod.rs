//! The unified trace model every format adapter reads into and writes out of.

mod metadata;
mod point;
mod trace;

pub use metadata::Metadata;
pub use point::{Extras, Point};
pub use trace::{Bounds, Segment, Trace, Track};
