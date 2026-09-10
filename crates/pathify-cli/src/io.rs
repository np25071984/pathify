//! Input and output plumbing shared by the commands.
//!
//! The rules that keep Pathify pipeline-friendly live here: `-` means stdin,
//! results go to stdout, and diagnostics never do.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Whether a path argument refers to stdin rather than a file on disk.
pub fn is_stdin(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Where a command should read its trace from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Stdin,
    /// No file was named and nothing is being piped in — the user has almost
    /// certainly forgotten the filename.
    Missing,
}

/// Work out where the input is coming from.
///
/// Defaulting to stdin is the right convention, but blocking on it when nobody
/// is piping anything is not: the program just sits there having drawn nothing,
/// with no clue that it is waiting on a keyboard nobody is typing at. So a
/// missing argument on an interactive terminal is treated as the mistake it
/// almost always is.
///
/// An explicit `-` still means stdin even at a terminal. Asking for it in so
/// many words is a deliberate choice, and refusing it would break the one case
/// where someone really does mean to paste a trace in.
pub fn choose_source(input: Option<&Path>, stdin_is_terminal: bool) -> Source {
    match input {
        Some(path) if !is_stdin(path) => Source::File(path.to_path_buf()),
        Some(_) => Source::Stdin,
        None if stdin_is_terminal => Source::Missing,
        None => Source::Stdin,
    }
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
pub fn read_input(input: Option<&Path>) -> Result<(Vec<u8>, Option<PathBuf>)> {
    let stdin_is_terminal = std::io::stdin().is_terminal();

    match choose_source(input, stdin_is_terminal) {
        Source::Missing => bail!(
            "no input: name a file, or pipe a trace in.\n\
             For example `pathify … ride.gpx`, or `cat ride.gpx | pathify …`.\n\
             To type one in by hand, ask for stdin explicitly with `-`."
        ),
        Source::Stdin => {
            if stdin_is_terminal {
                // Explicitly asked for, so honour it — but say so, or this
                // looks identical to the program having hung.
                eprintln!("pathify: reading from stdin; press Ctrl-D when finished");
            }
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .context("failed to read from stdin")?;
            Ok((bytes, None))
        }
        Source::File(path) => {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            Ok((bytes, Some(path)))
        }
    }
}

/// Read a trace and a basemap, either of which may be the piped input.
///
/// Which one arrived on stdin is decided by its content rather than by
/// position: a PNG signature is a basemap, anything else is a trace. That is
/// what lets both of these mean what they look like —
///
/// ```text
/// cat map.png | pathify render ride.gpx
/// cat ride.gpx | pathify render --basemap map.png
/// ```
///
/// — without the second one quietly trying to decode a GPX file as an image.
/// Stdin is read at most once, and only when one of the two is missing.
pub fn read_render_inputs(
    trace: Option<&Path>,
    basemap: Option<&Path>,
) -> Result<(Vec<u8>, Option<PathBuf>, Vec<u8>)> {
    let trace_is_stdin = trace.is_some_and(is_stdin);
    let basemap_is_stdin = basemap.is_some_and(is_stdin);

    if trace_is_stdin && basemap_is_stdin {
        bail!(
            "the trace and the basemap cannot both come from stdin.\n\
             Name one of them as a file."
        );
    }

    // An explicit `-` on either side settles which is which, so there is
    // nothing to sniff.
    if trace_is_stdin {
        let (basemap_bytes, _) = read_input(Some(require_basemap(basemap)?))?;
        let (trace_bytes, _) = read_input(trace)?;
        return Ok((trace_bytes, None, basemap_bytes));
    }
    if basemap_is_stdin {
        let (trace_bytes, path) = read_input(Some(require_trace(trace)?))?;
        let (basemap_bytes, _) = read_input(basemap)?;
        return Ok((trace_bytes, path, basemap_bytes));
    }

    if let (Some(trace), Some(basemap)) = (trace, basemap) {
        let (trace_bytes, path) = read_input(Some(trace))?;
        let (basemap_bytes, _) = read_input(Some(basemap))?;
        return Ok((trace_bytes, path, basemap_bytes));
    }

    // One of the two is missing, so it has to be the piped input. `read_input`
    // with no path applies the usual rules, including refusing to sit and wait
    // at a terminal for a filename someone forgot.
    let (piped, _) = read_input(None)?;

    if pathify_render::is_png(&piped) {
        if basemap.is_some() {
            bail!(
                "the basemap was given twice: a PNG was piped in, and --basemap \
                 names another one.\nPipe the trace instead, or drop --basemap."
            );
        }
        let (trace_bytes, path) = read_input(Some(require_trace(trace)?))?;
        Ok((trace_bytes, path, piped))
    } else {
        // Not a PNG, so the pipe is the trace — unless no basemap was named
        // either, in which case the missing basemap is the real problem and
        // saying anything else sends the reader looking in the wrong place.
        let basemap = require_basemap(basemap)?;
        let (basemap_bytes, _) = read_input(Some(basemap))?;
        Ok((piped, None, basemap_bytes))
    }
}

/// The basemap is the one input Pathify cannot produce for itself, so the
/// error says where to get one rather than just naming the missing flag.
fn require_basemap(basemap: Option<&Path>) -> Result<&Path> {
    basemap.ok_or_else(|| {
        anyhow::anyhow!(
            "no basemap: `render` draws onto a map image, and Pathify does not \
             download one.\n\
             Take the area from `pathify info`'s bbox row, fetch a PNG of it \
             yourself, then pass it with --basemap FILE or pipe it in."
        )
    })
}

fn require_trace(trace: Option<&Path>) -> Result<&Path> {
    trace.ok_or_else(|| {
        anyhow::anyhow!(
            "no trace: name a trace file, or pipe one in.\n\
             For example `pathify render ride.gpx --basemap map.png`."
        )
    })
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

        let (bytes, from) = read_input(Some(&path)).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(from.as_deref(), Some(path.as_path()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_names_itself_in_the_error() {
        let err = read_input(Some(Path::new("/nonexistent/pathify/ride.gpx"))).unwrap_err();
        assert!(err.to_string().contains("ride.gpx"), "{err}");
    }

    /// The reported bug: `pathify view` with no arguments sat there having
    /// drawn nothing, because it was blocking on a terminal nobody was typing
    /// at. A forgotten filename has to be reported, not waited on.
    #[test]
    fn a_missing_argument_at_a_terminal_is_a_mistake_not_an_invitation() {
        assert_eq!(choose_source(None, true), Source::Missing);
    }

    /// With something actually piped in, no argument means stdin — that is the
    /// whole point of the convention.
    #[test]
    fn a_missing_argument_with_piped_input_reads_the_pipe() {
        assert_eq!(choose_source(None, false), Source::Stdin);
    }

    /// Asking for stdin in so many words is a deliberate choice, so it is
    /// honoured even at a terminal; that is how you paste a trace in by hand.
    #[test]
    fn an_explicit_dash_always_means_stdin() {
        let dash = Path::new("-");
        assert_eq!(choose_source(Some(dash), true), Source::Stdin);
        assert_eq!(choose_source(Some(dash), false), Source::Stdin);
    }

    #[test]
    fn a_named_file_is_read_whatever_stdin_is_doing() {
        let path = Path::new("ride.gpx");
        for terminal in [true, false] {
            assert_eq!(
                choose_source(Some(path), terminal),
                Source::File(path.to_path_buf())
            );
        }
    }

    /// Only a bare `-` means stdin. A path that merely contains one — the way
    /// you refer to a file actually named `-` — is a file.
    #[test]
    fn a_path_that_merely_contains_a_dash_is_a_file() {
        let path = Path::new("./-");
        assert_eq!(
            choose_source(Some(path), true),
            Source::File(path.to_path_buf())
        );
    }
}
