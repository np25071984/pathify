//! Opening a Google Takeout export without unpacking it.
//!
//! An export is a Zip — often several, for a large account — and the parts a
//! GPS tool wants are a few hundred kilobytes of a two-gigabyte file. So this
//! reads the Zip's central directory to learn what is in there, and
//! decompresses only the individual entries a command actually asks for.
//!
//! A directory works as well as a Zip, because "unzip it first and point
//! Pathify at the folder" is what people try, and there is no reason for it to
//! fail.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Which Google product's data an export holds.
///
/// Takeout is not one format: what is inside depends on which products were
/// ticked at export time, and the layout of each has changed repeatedly. So
/// the backend is detected from the entry names rather than assumed — an
/// implementation written against Timeline finds nothing at all in a Health
/// export, and would otherwise exit reporting zero segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Google Health / Fitbit: per-day GPS CSVs plus exercise logs.
    Fitbit,
    /// Location History / Timeline, in any of its eras.
    Timeline,
}

impl Backend {
    pub const fn name(self) -> &'static str {
        match self {
            Backend::Fitbit => "Google Health",
            Backend::Timeline => "Location History",
        }
    }
}

/// One file inside an export.
///
/// Cloned out of the archive before reading, so a caller can pick the handful
/// of entries it wants and then read them without holding a borrow on the
/// archive it is about to read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path as the export spells it, e.g.
    /// `Takeout/Google Health/Global Export Data/exercise-0.json`.
    pub name: String,
    /// Which of the opened parts holds it.
    part: usize,
}

impl Entry {
    /// The last path component, which is where every name pattern that
    /// matters lives.
    pub fn file_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

/// One or more Takeout exports, opened together.
///
/// A multi-part export splits one account's data across `…-001.zip`,
/// `…-002.zip` and so on, and the split falls wherever the size limit landed —
/// the exercise logs can easily be in a different part from the GPS traces. So
/// the parts are treated as one namespace rather than searched separately.
pub struct Archive {
    parts: Vec<Part>,
    entries: Vec<Entry>,
}

enum Part {
    Zip {
        path: PathBuf,
        zip: zip::ZipArchive<BufReader<File>>,
    },
    Directory {
        root: PathBuf,
    },
}

impl Part {
    fn path(&self) -> &Path {
        match self {
            Part::Zip { path, .. } => path,
            Part::Directory { root } => root,
        }
    }
}

impl Archive {
    /// Open every named archive or directory and index what is inside.
    ///
    /// Nothing is decompressed here: for a Zip this reads the central
    /// directory, which is a few tens of kilobytes at the end of the file.
    pub fn open(paths: &[PathBuf]) -> Result<Self> {
        let mut parts = Vec::with_capacity(paths.len());
        let mut entries = Vec::new();

        for path in paths {
            let index = parts.len();
            let metadata = std::fs::metadata(path).map_err(|error| Error::Archive {
                path: path.display().to_string(),
                source: Box::new(error),
            })?;

            if metadata.is_dir() {
                collect_directory(path, path, index, &mut entries)?;
                parts.push(Part::Directory { root: path.clone() });
            } else {
                let file = File::open(path).map_err(|error| Error::Archive {
                    path: path.display().to_string(),
                    source: Box::new(error),
                })?;
                let zip =
                    zip::ZipArchive::new(BufReader::new(file)).map_err(|error| Error::Archive {
                        path: path.display().to_string(),
                        source: Box::new(error),
                    })?;
                entries.extend(
                    zip.file_names()
                        .filter(|name| !name.ends_with('/'))
                        .map(|name| Entry {
                            name: name.to_string(),
                            part: index,
                        }),
                );
                parts.push(Part::Zip {
                    path: path.clone(),
                    zip,
                });
            }
        }

        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { parts, entries })
    }

    /// Every file across every opened part, in name order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Entries whose name satisfies `predicate`, cloned so the caller can read
    /// them back out of the same archive.
    pub fn find(&self, predicate: impl Fn(&Entry) -> bool) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| predicate(e))
            .cloned()
            .collect()
    }

    /// Decompress one entry.
    pub fn read(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        let part = self
            .parts
            .get_mut(entry.part)
            .expect("an entry names a part of the archive it came from");
        let where_from = format!("{}: {}", part.path().display(), entry.name);

        let mut bytes = Vec::new();
        match part {
            Part::Zip { zip, .. } => {
                let mut file = zip.by_name(&entry.name).map_err(|error| Error::Archive {
                    path: where_from.clone(),
                    source: Box::new(error),
                })?;
                file.read_to_end(&mut bytes)
                    .map_err(|error| Error::Archive {
                        path: where_from,
                        source: Box::new(error),
                    })?;
            }
            Part::Directory { root } => {
                bytes = std::fs::read(root.join(&entry.name)).map_err(|error| Error::Archive {
                    path: where_from,
                    source: Box::new(error),
                })?;
            }
        }
        Ok(bytes)
    }

    /// Which backend's data this export holds, preferring Health over Timeline
    /// where an export happens to carry both.
    pub fn backend(&self) -> Option<Backend> {
        let has = |test: fn(&str) -> bool| self.entries.iter().any(|e| test(e.file_name()));
        if has(super::fitbit::is_gps_day) || has(super::fitbit::is_exercise_log) {
            return Some(Backend::Fitbit);
        }
        if self.entries.iter().any(looks_like_timeline) {
            return Some(Backend::Timeline);
        }
        None
    }

    /// The product folders the export actually contains, for the error a user
    /// gets when they exported the wrong things.
    ///
    /// Reported instead of "0 activities found", because the real problem is
    /// upstream at takeout.google.com and no amount of re-running the command
    /// will fix it.
    pub fn products(&self) -> Vec<String> {
        let mut products: Vec<String> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let mut parts = entry.name.split('/');
                let first = parts.next()?;
                match parts.next() {
                    // `Takeout/Google Health/…` — the product is the second
                    // component; the first is just the export's own wrapper.
                    Some(second) if parts.next().is_some() => Some(format!("{first}/{second}")),
                    _ => Some(first.to_string()),
                }
            })
            .collect();
        products.sort();
        products.dedup();
        products
    }
}

/// Whether an entry looks like any era of Location History.
///
/// Deliberately loose. The legacy folder name is localized to the account
/// language — `Standortverlauf`, `Historique des positions` — so matching an
/// English glob would miss most of the world. The file names inside are not
/// localized, and that is what this looks at.
fn looks_like_timeline(entry: &Entry) -> bool {
    let file = entry.file_name();
    if file.eq_ignore_ascii_case("Timeline.json")
        || file.eq_ignore_ascii_case("location-history.json")
        || file.eq_ignore_ascii_case("Records.json")
    {
        return true;
    }
    // Legacy Semantic Location History: `2021/2021_JANUARY.json`, whose month
    // name is in English even in a localized export.
    file.len() > 5
        && file.ends_with(".json")
        && file
            .split_once('_')
            .is_some_and(|(year, _)| year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()))
}

fn collect_directory(
    root: &Path,
    current: &Path,
    part: usize,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    let listing = std::fs::read_dir(current).map_err(|error| Error::Archive {
        path: current.display().to_string(),
        source: Box::new(error),
    })?;

    for item in listing {
        let item = item.map_err(|error| Error::Archive {
            path: current.display().to_string(),
            source: Box::new(error),
        })?;
        let path = item.path();
        // Symlinks are not followed: an unpacked export has no use for them,
        // and following one is how a walk ends up in a cycle or outside the
        // directory the user actually named.
        let kind = item.file_type().map_err(|error| Error::Archive {
            path: path.display().to_string(),
            source: Box::new(error),
        })?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_directory(root, &path, part, entries)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            entries.push(Entry {
                // Zip entries always use forward slashes, so a directory read
                // on Windows has to spell its names the same way or the two
                // inputs would need two sets of patterns.
                name: relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
                part,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            part: 0,
        }
    }

    #[test]
    fn an_entrys_file_name_is_its_last_component() {
        assert_eq!(entry("a/b/c.json").file_name(), "c.json");
        assert_eq!(entry("c.json").file_name(), "c.json");
    }

    /// The localized-folder trap: a German export's Location History folder is
    /// called `Standortverlauf`, so detection has to key on the file names,
    /// which are not translated.
    #[test]
    fn timeline_is_recognized_through_a_localized_folder_name() {
        assert!(looks_like_timeline(&entry(
            "Takeout/Standortverlauf/Semantic Location History/2021/2021_JANUARY.json"
        )));
        assert!(looks_like_timeline(&entry(
            "Takeout/Historique des positions/Timeline.json"
        )));
        assert!(!looks_like_timeline(&entry(
            "Takeout/Google Health/Global Export Data/exercise-0.json"
        )));
    }
}
