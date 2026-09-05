use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

/// A single GPS fix.
///
/// Elevation and time are optional because raw CSV and GeoJSON inputs often
/// carry neither — commands have to degrade gracefully rather than invent zeros.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Latitude in WGS84 degrees, `-90.0..=90.0`.
    pub lat: f64,
    /// Longitude in WGS84 degrees, normalized to `-180.0..=180.0`.
    pub lon: f64,
    /// Meters above sea level.
    pub elevation: Option<f64>,
    /// Fix time in UTC.
    pub time: Option<DateTime<Utc>>,
    /// Per-point extras that only some source formats record.
    pub extras: Extras,
}

/// Optional per-point fields. Modeled as named options rather than a
/// stringly-typed map: the set of extras GPX and FIT actually carry is small
/// and known, and named fields keep the merge/convert paths type-checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extras {
    pub heart_rate: Option<u16>,
    pub cadence: Option<u16>,
    /// Meters per second.
    pub speed: Option<f64>,
    /// Horizontal dilution of precision, where the source reports it.
    pub hdop: Option<f64>,
    pub satellites: Option<u16>,
    /// Degrees Celsius.
    pub temperature: Option<f64>,
}

impl Point {
    /// Build a point, rejecting non-finite or out-of-range latitudes and
    /// wrapping longitude into `-180..=180`, so distance math downstream never
    /// has to defend against a bare `181.0` or a NaN.
    pub fn new(lat: f64, lon: f64) -> Result<Self> {
        if !lat.is_finite() || !lon.is_finite() {
            return Err(Error::InvalidCoordinate(format!(
                "latitude and longitude must be finite, got ({lat}, {lon})"
            )));
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(Error::InvalidCoordinate(format!(
                "latitude {lat} is outside -90..=90"
            )));
        }
        Ok(Self {
            lat,
            lon: normalize_lon(lon),
            elevation: None,
            time: None,
            extras: Extras::default(),
        })
    }

    pub fn with_elevation(mut self, elevation: Option<f64>) -> Self {
        self.elevation = elevation.filter(|e| e.is_finite());
        self
    }

    pub fn with_time(mut self, time: Option<DateTime<Utc>>) -> Self {
        self.time = time;
        self
    }

    pub fn with_extras(mut self, extras: Extras) -> Self {
        self.extras = extras;
        self
    }
}

/// Wrap a longitude into `-180..=180`, keeping an exact eastern `180.0` on the
/// eastern side instead of flipping it to `-180.0`.
fn normalize_lon(lon: f64) -> f64 {
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 && lon > 0.0 {
        180.0
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_finite_coordinates() {
        assert!(Point::new(f64::NAN, 0.0).is_err());
        assert!(Point::new(0.0, f64::INFINITY).is_err());
    }

    #[test]
    fn rejects_out_of_range_latitude() {
        assert!(Point::new(90.5, 0.0).is_err());
        assert!(Point::new(-91.0, 0.0).is_err());
        assert!(Point::new(90.0, 0.0).is_ok());
    }

    #[test]
    fn wraps_longitude_into_range() {
        assert_eq!(Point::new(0.0, 190.0).unwrap().lon, -170.0);
        assert_eq!(Point::new(0.0, -190.0).unwrap().lon, 170.0);
        assert_eq!(Point::new(0.0, 180.0).unwrap().lon, 180.0);
        assert_eq!(Point::new(0.0, -180.0).unwrap().lon, -180.0);
        assert_eq!(Point::new(0.0, 12.5).unwrap().lon, 12.5);
    }

    #[test]
    fn drops_non_finite_elevation() {
        let p = Point::new(0.0, 0.0).unwrap().with_elevation(Some(f64::NAN));
        assert_eq!(p.elevation, None);
    }
}
