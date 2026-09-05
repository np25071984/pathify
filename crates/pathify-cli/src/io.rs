//! Input and output plumbing shared by the commands.
//!
//! The rules that keep Pathify pipeline-friendly live here: `-` means stdin,
//! results go to stdout, and diagnostics never do.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Whether a path argument refers to stdin rather than a file on disk.
pub fn is_stdin(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Read an entire input into memory.
///
/// Returns the bytes plus the path they came from, which is `None` for stdin —
/// that distinction is what tells format detection whether it has a filename
/// extension to work with or has to sniff the content.
///
/// Traces are read whole rather than streamed because the XML and JSON formats
/// have to be anyway, and a GPS trace is small: a dense all-day ride is a few
/// megabytes.
pub fn read_input(path: &Path) -> Result<(Vec<u8>, Option<PathBuf>)> {
    if is_stdin(path) {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .context("failed to read from stdin")?;
        Ok((bytes, None))
    } else {
        let bytes =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        Ok((bytes, Some(path.to_path_buf())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn recognizes_the_stdin_sentinel() {
        assert!(is_stdin(Path::new("-")));
        assert!(!is_stdin(Path::new("./-")));
        assert!(!is_stdin(Path::new("ride.gpx")));
    }

    #[test]
    fn reads_a_file_and_reports_its_path() {
        let dir = std::env::temp_dir().join(format!("pathify-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.gpx");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"hello").unwrap();

        let (bytes, from) = read_input(&path).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(from.as_deref(), Some(path.as_path()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_names_itself_in_the_error() {
        let err = read_input(Path::new("/nonexistent/pathify/ride.gpx")).unwrap_err();
        assert!(err.to_string().contains("ride.gpx"), "{err}");
    }
}
