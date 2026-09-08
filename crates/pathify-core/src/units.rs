//! Whether distances are reported in metres or in feet and miles.
//!
//! In core rather than in an interface crate because more than one asks:
//! the terminal map's status line formats what it shows, and `render`
//! interprets the reveal radius it is given.

/// Distance units to format numbers in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Metric,
    Imperial,
}

impl Default for Units {
    /// Metric until something says otherwise — see [`Units::detect`].
    fn default() -> Self {
        Units::Metric
    }
}

/// Countries that measure everyday distance in miles rather than kilometres.
/// Short enough that checking a locale's region against it is more reliable
/// than trying to enumerate every metric one.
const IMPERIAL_REGIONS: [&str; 3] = ["US", "LR", "MM"];

impl Units {
    /// Guess the host's preferred units from its locale.
    ///
    /// POSIX systems carry a `LC_MEASUREMENT` category for exactly this, and
    /// even a terminal that never set it still leaves a region behind in
    /// `LANG` (e.g. `en_US.UTF-8`). `LC_ALL` overrides both when set, per the
    /// usual POSIX precedence. Nothing recognized — including no locale
    /// variable at all — defaults to metric.
    pub fn detect() -> Self {
        ["LC_ALL", "LC_MEASUREMENT", "LANG"]
            .into_iter()
            .find_map(|var| std::env::var(var).ok().and_then(|v| Self::from_locale(&v)))
            .unwrap_or_default()
    }

    /// Parse a POSIX locale string, e.g. `en_US.UTF-8`, `en-US`, or `C`.
    fn from_locale(locale: &str) -> Option<Self> {
        let region = region_of(locale)?;
        Some(
            if IMPERIAL_REGIONS.contains(&region.to_ascii_uppercase().as_str()) {
                Units::Imperial
            } else {
                Units::Metric
            },
        )
    }
}

/// The region subtag of a POSIX locale name, e.g. `US` from `en_US.UTF-8`.
///
/// `None` for a locale with no region at all — `C`, `POSIX`, a bare language
/// code, or an empty string — since there is nothing to check against the
/// imperial list.
fn region_of(locale: &str) -> Option<&str> {
    let name = locale.split(['.', '@']).next().unwrap_or("");
    let mut parts = name.split(['_', '-']);
    parts.next()?;
    parts.next().filter(|region| !region.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_imperial_regions() {
        assert_eq!(Units::from_locale("en_US.UTF-8"), Some(Units::Imperial));
        assert_eq!(Units::from_locale("en-US"), Some(Units::Imperial));
        assert_eq!(Units::from_locale("my_MM"), Some(Units::Imperial));
        assert_eq!(Units::from_locale("en_LR"), Some(Units::Imperial));
    }

    #[test]
    fn defaults_other_regions_to_metric() {
        assert_eq!(Units::from_locale("en_GB.UTF-8"), Some(Units::Metric));
        assert_eq!(Units::from_locale("de_DE.UTF-8"), Some(Units::Metric));
        assert_eq!(Units::from_locale("fr-CA"), Some(Units::Metric));
    }

    #[test]
    fn region_matching_ignores_case() {
        assert_eq!(Units::from_locale("en_us"), Some(Units::Imperial));
    }

    #[test]
    fn locales_with_no_region_are_unrecognized() {
        assert_eq!(Units::from_locale("C"), None);
        assert_eq!(Units::from_locale("POSIX"), None);
        assert_eq!(Units::from_locale("en"), None);
        assert_eq!(Units::from_locale(""), None);
        assert_eq!(Units::from_locale("en_.UTF-8"), None);
    }
}
