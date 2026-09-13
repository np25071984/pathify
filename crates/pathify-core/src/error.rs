use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the core crate.
///
/// Variants are structured rather than stringly-typed so the CLI can map them
/// onto distinct exit codes; core itself never prints and never exits.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("could not determine the input format{}; pass --from to set it explicitly", .0.as_ref().map(|s| format!(" of {s}")).unwrap_or_default())]
    UnknownFormat(Option<String>),

    #[error("{0} support is not implemented yet")]
    UnsupportedFormat(&'static str),

    #[error("not a known format name: {0}")]
    UnknownFormatName(String),

    #[error("failed to parse {format} input: {message}")]
    Parse {
        format: &'static str,
        message: String,
    },

    #[error("failed to write {format} output: {message}")]
    Write {
        format: &'static str,
        message: String,
    },

    #[error("invalid coordinate: {0}")]
    InvalidCoordinate(String),

    /// A Takeout archive could not be opened or an entry could not be read.
    ///
    /// The cause is kept as a source rather than flattened into a string so
    /// that an underlying [`std::io::Error`] stays visible in the chain — the
    /// CLI reads that to tell an unreadable file (exit 2) apart from a
    /// corrupt one (exit 1).
    #[error("failed to read {path}")]
    Archive {
        path: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A Takeout archive was readable but could not be used as asked. The
    /// message is composed in core because what is worth saying depends on
    /// what the archive turned out to contain, and core never prints.
    #[error("{message}")]
    Takeout { message: String },
}
