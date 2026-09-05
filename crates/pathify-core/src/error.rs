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
}
