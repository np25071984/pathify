//! Unit-bearing values for numeric flags.
//!
//! Every flag that measures something accepts its unit inline —
//! `--elevation-threshold 3m`, `--trim-ends 200ft`, `--dedup-window 90s`,
//! `--max-speed 25km/h` — so the command says what it means without the
//! reader having to remember which flag counts in what.
//!
//! A bare number keeps the metric base unit the flag has always used: meters
//! for a distance, seconds for a duration, meters per second for a speed.
//! That is both the documented default and what keeps existing scripts
//! working.
//!
//! Parsers here return the base unit, so the rest of the program still deals
//! only in meters and seconds and never has to ask what a value is in.

/// Meters in one foot, and the other conversions that follow from it.
const METERS_PER_FOOT: f64 = 0.3048;
const METERS_PER_YARD: f64 = METERS_PER_FOOT * 3.0;
const METERS_PER_MILE: f64 = METERS_PER_FOOT * 5280.0;
const METERS_PER_NAUTICAL_MILE: f64 = 1852.0;

/// Parse a distance into meters. A bare number is already meters.
pub fn parse_distance(value: &str) -> Result<f64, String> {
    let (magnitude, unit) = split(value)?;
    let meters = match unit.as_str() {
        "" | "m" | "meter" | "meters" | "metre" | "metres" => magnitude,
        "km" | "kilometer" | "kilometers" | "kilometre" | "kilometres" => magnitude * 1000.0,
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => magnitude / 100.0,
        "ft" | "foot" | "feet" => magnitude * METERS_PER_FOOT,
        "yd" | "yard" | "yards" => magnitude * METERS_PER_YARD,
        "mi" | "mile" | "miles" => magnitude * METERS_PER_MILE,
        "nmi" => magnitude * METERS_PER_NAUTICAL_MILE,
        other => {
            return Err(unknown(
                other,
                value,
                "distance",
                "m, km, cm, ft, yd, mi, nmi",
            ));
        }
    };
    // A negative distance would fence, trim, or match nothing while looking
    // like it had been asked for — for `--redact-around` that quietly
    // publishes the location the user meant to hide.
    non_negative(meters, value, "distance")
}

/// Parse a duration into seconds. A bare number is already seconds.
pub fn parse_duration(value: &str) -> Result<f64, String> {
    let (magnitude, unit) = split(value)?;
    let seconds = match unit.as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => magnitude,
        "ms" | "msec" | "msecs" | "millisecond" | "milliseconds" => magnitude / 1000.0,
        "min" | "mins" | "minute" | "minutes" => magnitude * 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => magnitude * 3600.0,
        // `m` is minutes to one reader and meters to another, and both are
        // plausible on a GPS tool. Neither guess is worth the risk of
        // silently applying a window sixty times the intended one.
        "m" => {
            return Err(format!(
                "`m` is ambiguous in `{value}`: write `min` for minutes or `s` for seconds"
            ));
        }
        other => return Err(unknown(other, value, "duration", "s, ms, min, h")),
    };
    non_negative(seconds, value, "duration")
}

/// Parse a speed into meters per second. A bare number is already m/s.
pub fn parse_speed(value: &str) -> Result<f64, String> {
    let (magnitude, unit) = split(value)?;
    let mps = match unit.as_str() {
        "" | "m/s" | "mps" => magnitude,
        "km/h" | "kmh" | "kph" | "kmph" => magnitude / 3.6,
        "mi/h" | "mph" => magnitude * METERS_PER_MILE / 3600.0,
        "ft/s" | "fps" => magnitude * METERS_PER_FOOT,
        "kn" | "kt" | "knot" | "knots" => magnitude * METERS_PER_NAUTICAL_MILE / 3600.0,
        other => return Err(unknown(other, value, "speed", "m/s, km/h, mph, ft/s, kn")),
    };
    non_negative(mps, value, "speed")
}

/// Split a value into its number and its unit suffix, lowercased.
///
/// The unit starts at the first character that cannot belong to the number,
/// so `200ft`, `200 ft`, and `200FT` are the same value. An `e` is part of the
/// number only when an exponent follows it, which keeps `1e3m` working
/// without swallowing the `e` of a unit.
fn split(value: &str) -> Result<(f64, String), String> {
    let text = value.trim();
    let bytes = text.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        let byte = bytes[end];
        let is_exponent = matches!(byte, b'e' | b'E')
            && bytes
                .get(end + 1)
                .is_some_and(|next| next.is_ascii_digit() || matches!(next, b'+' | b'-'));
        if byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-') || is_exponent {
            end += 1;
        } else {
            break;
        }
    }

    let (number, unit) = text.split_at(end);
    let magnitude = number
        .parse::<f64>()
        .map_err(|_| format!("`{value}` does not start with a number"))?;
    if !magnitude.is_finite() {
        return Err(format!("`{value}` is not a finite number"));
    }
    Ok((magnitude, unit.trim().to_ascii_lowercase()))
}

fn non_negative(measure: f64, value: &str, what: &str) -> Result<f64, String> {
    if measure < 0.0 {
        return Err(format!("{what} `{value}` must be zero or more"));
    }
    Ok(measure)
}

fn unknown(unit: &str, value: &str, what: &str, expected: &str) -> String {
    format!("unknown {what} unit `{unit}` in `{value}` (expected one of: {expected})")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule that keeps every existing script and README example working.
    #[test]
    fn a_bare_number_is_the_metric_base_unit() {
        assert_eq!(parse_distance("300").unwrap(), 300.0);
        assert_eq!(parse_duration("120").unwrap(), 120.0);
        assert_eq!(parse_speed("60").unwrap(), 60.0);
    }

    #[test]
    fn converts_metric_distances() {
        assert_eq!(parse_distance("3m").unwrap(), 3.0);
        assert_eq!(parse_distance("1.5km").unwrap(), 1500.0);
        assert_eq!(parse_distance("50cm").unwrap(), 0.5);
        assert_eq!(parse_distance("2 metres").unwrap(), 2.0);
    }

    #[test]
    fn converts_imperial_distances() {
        assert!((parse_distance("200ft").unwrap() - 60.96).abs() < 1e-9);
        assert!((parse_distance("1mi").unwrap() - 1609.344).abs() < 1e-9);
        assert!((parse_distance("100yd").unwrap() - 91.44).abs() < 1e-9);
        assert!((parse_distance("1nmi").unwrap() - 1852.0).abs() < 1e-9);
    }

    #[test]
    fn converts_durations() {
        assert_eq!(parse_duration("90s").unwrap(), 90.0);
        assert_eq!(parse_duration("2min").unwrap(), 120.0);
        assert_eq!(parse_duration("1h").unwrap(), 3600.0);
        assert_eq!(parse_duration("500ms").unwrap(), 0.5);
    }

    #[test]
    fn converts_speeds() {
        assert_eq!(parse_speed("36km/h").unwrap(), 10.0);
        assert_eq!(parse_speed("36kph").unwrap(), 10.0);
        assert!((parse_speed("60mph").unwrap() - 26.8224).abs() < 1e-9);
        assert!((parse_speed("10kn").unwrap() - 5.144_444_444).abs() < 1e-6);
    }

    #[test]
    fn unit_spelling_is_forgiving_about_case_and_spacing() {
        assert_eq!(
            parse_distance("200 FT").unwrap(),
            parse_distance("200ft").unwrap()
        );
        assert_eq!(
            parse_speed("25 KM/H").unwrap(),
            parse_speed("25km/h").unwrap()
        );
        assert_eq!(parse_duration(" 2 Minutes ").unwrap(), 120.0);
    }

    /// `m` means minutes to one reader and meters to another, and a duration
    /// flag cannot tell which was meant — sixty times the intended window is
    /// too big a mistake to guess at.
    #[test]
    fn an_ambiguous_m_on_a_duration_is_refused() {
        let error = parse_duration("5m").unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
        assert!(error.contains("min"), "{error}");
    }

    #[test]
    fn an_unknown_unit_names_the_ones_that_work() {
        let error = parse_distance("10furlongs").unwrap_err();
        assert!(error.contains("furlongs"), "{error}");
        assert!(error.contains("km"), "{error}");
    }

    #[test]
    fn a_value_without_a_number_is_refused() {
        assert!(parse_distance("m").is_err());
        assert!(parse_distance("").is_err());
        assert!(parse_speed("fast").is_err());
    }

    /// A negative fence radius would publish the location it was asked to
    /// hide, so no measure is allowed to go below zero.
    #[test]
    fn negative_measures_are_refused() {
        for error in [
            parse_distance("-100").unwrap_err(),
            parse_distance("-100ft").unwrap_err(),
            parse_duration("-5s").unwrap_err(),
            parse_speed("-1mph").unwrap_err(),
        ] {
            assert!(error.contains("zero or more"), "{error}");
        }
    }

    #[test]
    fn exponents_still_parse() {
        assert_eq!(parse_distance("1e3").unwrap(), 1000.0);
        assert_eq!(parse_distance("1e3m").unwrap(), 1000.0);
        assert_eq!(parse_distance("1e-1km").unwrap(), 100.0);
    }
}
