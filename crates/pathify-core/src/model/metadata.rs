use chrono::{DateTime, Utc};

use crate::formats::Format;

/// File-level information about where a trace came from.
///
/// Pathify preserves what maps cleanly onto every format; it deliberately does
/// not promise byte-for-byte round-tripping of arbitrary format extensions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub name: Option<String>,
    pub description: Option<String>,
    /// The format this trace was read from, when it was read from a file.
    pub source_format: Option<Format>,
    /// Recording device or producing application, where the source names one.
    pub source_device: Option<String>,
    pub created: Option<DateTime<Utc>>,
}

impl Metadata {
    pub fn with_source_format(mut self, format: Format) -> Self {
        self.source_format = Some(format);
        self
    }
}
