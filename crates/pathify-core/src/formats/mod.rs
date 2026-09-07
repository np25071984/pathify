//! Format adapters.
//!
//! Every adapter converts to and from the one [`Trace`] model, so `convert` is
//! N readers plus M writers rather than N×M special cases.

pub mod csv;
pub mod geojson;
pub mod gpx;
pub mod tcx;

use std::io::Write;
use std::path::Path;
use std::str::FromStr;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::Trace;

/// The formats Pathify knows about.
///
/// Variants exist for formats that are not implemented yet so that detection,
/// the CLI's `--from`/`--to` parsing, and error messages can all name them
/// before the adapter lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Gpx,
    Tcx,
    Fit,
    Kml,
    GeoJson,
    Csv,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Gpx,
        Format::Tcx,
        Format::Fit,
        Format::Kml,
        Format::GeoJson,
        Format::Csv,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Format::Gpx => "gpx",
            Format::Tcx => "tcx",
            Format::Fit => "fit",
            Format::Kml => "kml",
            Format::GeoJson => "geojson",
            Format::Csv => "csv",
        }
    }

    /// Whether reading and writing this format is implemented.
    pub const fn is_implemented(self) -> bool {
        matches!(
            self,
            Format::Gpx | Format::Tcx | Format::GeoJson | Format::Csv
        )
    }

    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "gpx" => Some(Format::Gpx),
            "tcx" => Some(Format::Tcx),
            "fit" => Some(Format::Fit),
            "kml" => Some(Format::Kml),
            "geojson" | "json" => Some(Format::GeoJson),
            "csv" => Some(Format::Csv),
            _ => None,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Format::from_extension)
    }

    /// Guess a format from leading bytes.
    ///
    /// This is what makes `pathify info -` work: piped input has no filename to
    /// take an extension from.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        // A FIT file carries the ASCII tag ".FIT" at bytes 8..12 of its header.
        if bytes.len() >= 12 && &bytes[8..12] == b".FIT" {
            return Some(Format::Fit);
        }

        let head = &bytes[..bytes.len().min(4096)];
        let text = String::from_utf8_lossy(head);
        let trimmed = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);

        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return Some(Format::GeoJson);
        }

        if trimmed.starts_with('<') {
            let lowered = trimmed.to_ascii_lowercase();
            if lowered.contains("<gpx") {
                return Some(Format::Gpx);
            }
            if lowered.contains("<trainingcenterdatabase") {
                return Some(Format::Tcx);
            }
            if lowered.contains("<kml") {
                return Some(Format::Kml);
            }
            // An XML document we can't identify is not worth guessing at.
            return None;
        }

        // Fall back to CSV only on something that actually looks tabular: a
        // first line with commas and a plausible coordinate header.
        let first_line = trimmed.lines().next().unwrap_or_default();
        let lowered = first_line.to_ascii_lowercase();
        if first_line.contains(',') && (lowered.contains("lat") || lowered.contains("lon")) {
            return Some(Format::Csv);
        }

        None
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Format {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Format::from_extension(s).ok_or_else(|| Error::UnknownFormatName(s.to_string()))
    }
}

/// Work out which format to read, preferring an explicit choice, then the file
/// extension, then content sniffing.
pub fn detect(explicit: Option<Format>, path: Option<&Path>, bytes: &[u8]) -> Result<Format> {
    if let Some(format) = explicit {
        return Ok(format);
    }
    if let Some(format) = path.and_then(Format::from_path) {
        return Ok(format);
    }
    Format::sniff(bytes).ok_or_else(|| Error::UnknownFormat(path.map(|p| p.display().to_string())))
}

/// Parse bytes in `format` into the trace model.
pub fn read(format: Format, bytes: &[u8]) -> Result<Trace> {
    match format {
        Format::Gpx => gpx::read(bytes),
        Format::Tcx => tcx::read(bytes),
        Format::GeoJson => geojson::read(bytes),
        Format::Csv => csv::read(bytes),
        Format::Fit => Err(Error::UnsupportedFormat("FIT")),
        Format::Kml => Err(Error::UnsupportedFormat("KML")),
    }
}

/// Serialize a trace as `format`.
pub fn write(format: Format, trace: &Trace, out: &mut dyn Write) -> Result<()> {
    match format {
        Format::Gpx => gpx::write(trace, out),
        Format::Tcx => tcx::write(trace, out),
        Format::GeoJson => geojson::write(trace, out),
        Format::Csv => csv::write(trace, out),
        Format::Fit => Err(Error::UnsupportedFormat("FIT")),
        Format::Kml => Err(Error::UnsupportedFormat("KML")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping_round_trips_through_name() {
        for format in Format::ALL {
            assert_eq!(Format::from_extension(format.name()), Some(format));
            assert_eq!(format.name().parse::<Format>().unwrap(), format);
        }
    }

    #[test]
    fn extensions_are_case_insensitive() {
        assert_eq!(Format::from_extension("GPX"), Some(Format::Gpx));
        assert_eq!(
            Format::from_path(Path::new("/tmp/Ride.GPX")),
            Some(Format::Gpx)
        );
        assert_eq!(Format::from_extension("kmz"), None);
    }

    #[test]
    fn sniffs_gpx_tcx_and_kml_apart() {
        let gpx = br#"<?xml version="1.0"?><gpx version="1.1"><trk></trk></gpx>"#;
        let tcx =
            br#"<?xml version="1.0"?><TrainingCenterDatabase xmlns="x"></TrainingCenterDatabase>"#;
        let kml = br#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2"></kml>"#;
        assert_eq!(Format::sniff(gpx), Some(Format::Gpx));
        assert_eq!(Format::sniff(tcx), Some(Format::Tcx));
        assert_eq!(Format::sniff(kml), Some(Format::Kml));
    }

    #[test]
    fn sniffs_fit_by_header_tag() {
        let mut bytes = vec![0u8; 12];
        bytes[8..12].copy_from_slice(b".FIT");
        assert_eq!(Format::sniff(&bytes), Some(Format::Fit));
    }

    #[test]
    fn sniffs_json_and_csv() {
        assert_eq!(
            Format::sniff(br#"{"type":"FeatureCollection"}"#),
            Some(Format::GeoJson)
        );
        assert_eq!(
            Format::sniff(b"lat,lon,ele\n47.6,-122.3,20\n"),
            Some(Format::Csv)
        );
    }

    #[test]
    fn declines_to_guess_at_unknown_content() {
        assert_eq!(Format::sniff(b"hello world"), None);
        assert_eq!(Format::sniff(b"<html><body></body></html>"), None);
        assert_eq!(Format::sniff(b""), None);
    }

    #[test]
    fn detection_prefers_explicit_then_extension_then_content() {
        let gpx_bytes = br#"<?xml version="1.0"?><gpx></gpx>"#;
        // Explicit wins even when it contradicts everything else.
        assert_eq!(
            detect(Some(Format::Csv), Some(Path::new("a.gpx")), gpx_bytes).unwrap(),
            Format::Csv
        );
        // Extension wins over content.
        assert_eq!(
            detect(None, Some(Path::new("a.kml")), gpx_bytes).unwrap(),
            Format::Kml
        );
        // Content is the stdin fallback.
        assert_eq!(detect(None, None, gpx_bytes).unwrap(), Format::Gpx);
        assert!(detect(None, None, b"???").is_err());
    }
}
