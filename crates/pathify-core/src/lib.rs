//! Core trace model, format adapters, and spatial math for Pathify.
//!
//! This crate deliberately has no CLI or TUI dependencies, and performs no
//! network I/O of any kind — the local-first guarantee is enforced by the
//! dependency tree, not by a runtime setting.
//!
//! ```
//! use pathify_core::{formats, summary::Summary};
//!
//! let gpx = br#"<?xml version="1.0"?>
//! <gpx version="1.1" creator="demo" xmlns="http://www.topografix.com/GPX/1/1">
//!   <trk><trkseg>
//!     <trkpt lat="47.6062" lon="-122.3321"><ele>56.0</ele></trkpt>
//!     <trkpt lat="47.6070" lon="-122.3330"><ele>61.0</ele></trkpt>
//!   </trkseg></trk>
//! </gpx>"#;
//!
//! let trace = formats::read(formats::Format::Gpx, gpx).unwrap();
//! let summary = Summary::of(&trace, pathify_core::spatial::DEFAULT_NOISE_THRESHOLD_M);
//! assert_eq!(summary.points, 2);
//! ```

pub mod clean;
pub mod error;
pub mod formats;
pub mod merge;
pub mod model;
pub mod spatial;
pub mod summary;

pub use clean::{CleanOptions, CleanReport, clean};
pub use error::{Error, Result};
pub use formats::Format;
pub use merge::{MergeMode, MergeOptions, MergeReport, merge};
pub use model::{Bounds, Extras, Metadata, Point, Segment, Trace, Track};
pub use summary::Summary;
