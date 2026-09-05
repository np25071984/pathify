# Pathify

A local-first command-line toolkit for GPS trace data — inspect, clean, merge,
convert, and view GPX, FIT, KML, GeoJSON, and CSV traces without a desktop GIS
application and without uploading anything anywhere.

> **Status: early.** `info`, `convert`, `merge`, and `clean` work across GPX,
> GeoJSON, and CSV today. The remaining commands and format adapters are in progress — see
> [Roadmap](#roadmap).

## Format support

| Format | Read | Write | Notes |
| --- | :---: | :---: | --- |
| GPX | ✅ | ✅ | Tracks, routes, and loose waypoints |
| GeoJSON | ✅ | ✅ | Timestamps via the `coordTimes` convention |
| CSV | ✅ | ✅ | [Documented column schema](#csv-schema) |
| KML | — | — | Planned |
| FIT | — | — | Planned |

## Why

Existing options ask you to pick two of three: a modern interface, real speed,
or keeping your location history on your own machine. Pathify is built so you
don't have to choose. It has no network code at all — not disabled by a setting,
[absent from the dependency graph](scripts/check-no-network-deps.sh), and
enforced in CI.

## Install

Requires Rust 1.88 or newer.

```sh
git clone <repository-url> && cd pathify
cargo install --path crates/pathify-cli
```

Or run it from the repository without installing:

```sh
cargo run -- info tests/fixtures/ride.gpx
```

## Usage

```console
$ pathify info tests/fixtures/ride.gpx
Burke-Gilman morning
  format      gpx
  tracks      1
  segments    2
  points      10
  distance    2.11 km
  duration    6:30
  avg speed   19.5 km/h
  elevation   +19 m / -11 m  (12–31 m)
  time        2024-05-01 15:00:00 → 2024-05-01 15:06:30 UTC
  bounds      47.6535, -122.3056 → 47.6651, -122.2745
```

Every command reads a file or `-` for stdin, writes results to stdout, and sends
diagnostics to stderr, so they compose:

```sh
cat ride.gpx | pathify info                    # format detected by sniffing content
pathify info ride.gpx --json | jq .distance_m  # machine-readable output
```

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Bad input — unparseable, or a format with no adapter yet |
| `2` | I/O failure — the file could not be read or the output written |

### `pathify info`

| Flag | Effect |
| --- | --- |
| `--json` | Emit JSON instead of the human-readable table |
| `--from <FORMAT>` | Override format detection (`gpx`, `fit`, `kml`, `geojson`, `csv`) |
| `--elevation-threshold <METERS>` | Ignore elevation changes below this floor (default `3`) |

The elevation threshold is not cosmetic. Consumer GPS elevation jitters by a
couple of meters at rest, so summing raw deltas reports hundreds of meters of
phantom climb on a flat ride. Gain and loss only accumulate once the elevation
departs from a running reference by more than the threshold.

### `pathify convert`

```sh
pathify convert ride.gpx --to csv            # to stdout
pathify convert ride.gpx -o ride.geojson     # target inferred from the extension
cat ride.gpx | pathify convert --to geojson | pathify info -
```

| Flag | Effect |
| --- | --- |
| `--to <FORMAT>` | Target format. Optional when `--output` implies one |
| `--output, -o <FILE>` | Write to a file instead of stdout |
| `--from <FORMAT>` | Override input format detection |

Conversion goes through the shared model, so it is N readers plus M writers
rather than a matrix of special cases. Metrics survive the trip: converting
`gpx → geojson → csv` and running `info` reports the same distance, duration,
elevation, and segment count as the original file.

What conversion does *not* promise is byte-for-byte fidelity. Format-specific
extensions are not preserved, and a format with nowhere to put a value loses it.
Where a convention exists, Pathify uses it rather than dropping data — GeoJSON
timestamps ride in `coordTimes`, and CSV keeps track/segment indexes.

### `pathify merge`

```sh
pathify merge phone.gpx watch.gpx > ride.gpx      # two devices, one ride
pathify merge monday.gpx tuesday.gpx --to geojson # two days, stitched in order
```

| Flag | Effect |
| --- | --- |
| `--primary <FILE>` | Input whose position, elevation, and time win on conflict (default: the first) |
| `--no-dedup` | Concatenate without matching, even where inputs overlap |
| `--dedup-window <SECONDS>` | Override the derived matching window |
| `--dedup-radius <METERS>` | Override the speed-derived matching distance |
| `--segment-gap <SECONDS>` | Gap that starts a new segment in reconciled output (default `120`) |
| `--verbose, -v` | Report what the merge did, on stderr |
| `--to`, `--from`, `--output` | As for `convert`; output defaults to the first input's format |

Merging picks one of two behaviors from the inputs themselves rather than
asking you which you meant:

**Concatenation**, when the inputs don't overlap in time — a paused and resumed
watch, or two consecutive days. The recordings are ordered by start time and
stitched together, each keeping its own segment boundaries. Nothing is compared.

**Reconciliation**, when they do overlap — a phone and a watch recording the
same ride. The same physical moment appears in both inputs and has to be
collapsed, or the merged trace reports roughly double the distance:

```console
$ pathify merge phone.gpx watch.gpx -v | pathify info -
  points      12
  distance    2.67 km
reconciled 2 inputs within 15.0s: 22 points in, 10 matched as duplicates, 12 out

$ pathify merge phone.gpx watch.gpx --no-dedup | pathify info -
  points      22
  distance    4.77 km        ← the same ride, counted twice
```

Two fixes are the same moment when they are close in **both** time and space:

- **Time** — within half the coarser input's sampling interval, which is exactly
  where "the nearest sample" tips over to the neighbouring one. A 30-second
  recording paired against a 1 Hz one gets a 15-second window. Among everything
  in the window, the closest in time wins, and each point can absorb at most one
  point per source: two fixes from the same device are two moments, however
  close together.
- **Space** — within `max plausible speed × time apart + GPS noise margin`. The
  subject genuinely moved between the two samples, so the allowance has to grow
  with the gap.

Matched points take position, elevation, and time from the primary — picking a
winner beats averaging, which would invent a location neither device recorded.
Optional extras go the other way and are unioned, so a watch's heart rate and a
phone's better fix both survive. Unmatched points pass through, so a device with
weaker signal still contributes wherever the other had none.

Reconciled output is re-segmented on time gaps, because two devices disagree
about where the pauses were and their own boundaries cannot both be kept.

### `pathify clean`

```sh
pathify clean ride.gpx > tidy.gpx                          # drift filtering only
pathify clean ride.gpx --trim-ends 300 > shareable.gpx     # hide both endpoints
pathify clean ride.gpx --redact-around 47.6535,-122.3056,300
```

| Flag | Effect |
| --- | --- |
| `--trim-ends <METERS>` | Remove everything near where the trace starts and ends |
| `--redact-around <LAT,LON,RADIUS>` | Remove everything inside a named circle. Repeatable |
| `--no-drift-filter` | Keep fixes that imply an impossible speed |
| `--max-speed <M/S>` | Speed above which a step is a bad fix (default `60`, about 216 km/h) |
| `--verbose, -v` | Report what was removed, on stderr |
| `--to`, `--from`, `--output` | As for `convert`; output defaults to the input's format |

`clean` does two unrelated things, and they have deliberately different
defaults.

**Drift filtering runs by default.** It discards fixes that could not have been
reached from the previous one at any plausible speed — readings the receiver got
wrong, not places anyone went. One bad fix wrecks every metric downstream:

```console
$ pathify info ride.gpx              → 11 points, 51.75 km
$ pathify clean ride.gpx | pathify info -   → 10 points, 2.11 km
```

Removing an outlier joins its neighbours back together, because the subject
really did travel through there. Filtering also gives up rather than deleting a
recording: if several fixes in a row are rejected, the *anchor* is treated as
the bad one, so a single bad first fix cannot cascade into an empty file.

**Redaction never runs unless you ask.** It deletes real locations, and which
ones is yours to decide — there is no default radius quietly cutting your ride.
Two ways to ask:

- `--trim-ends <METERS>` hides where a journey began and finished without you
  typing your home address into a shell command. It fences *both* endpoints and
  removes matching points wherever they appear, so a loop that passes the front
  door halfway round does not leak what the trim was meant to hide.
- `--redact-around <LAT,LON,RADIUS>` fences a location you can name, and can be
  given more than once.

Redaction **splits** the segment it cuts instead of closing the gap. Joining the
survivors would draw a straight line through the hidden area — inventing travel
that never happened, and leaving a chord pointing at exactly what was hidden.

#### CSV schema

Writing emits a stable header:

```text
track,segment,lat,lon,ele,time
```

`track` and `segment` are zero-based indexes, present so the hierarchy survives
a round trip — without them a paused recording returns as one unbroken segment,
and its gap silently becomes distance. Absent values are empty cells, never `0`.

Reading matches columns by name and accepts the aliases other tools emit
(`latitude`, `longitude`, `altitude`, `alt`, `timestamp`, `x`/`y`, and more).
Only latitude and longitude are required; a file with no `track`/`segment`
columns reads as a single segment. Timestamps parse as RFC 3339 or as the
space-separated form spreadsheets produce.

## Architecture

Three crates, so the spatial logic stays free of interface concerns:

| Crate | Responsibility |
| --- | --- |
| `pathify-core` | Trace model, format adapters, spatial math. No CLI, TUI, or network dependencies. |
| `pathify-cli` | Argument parsing, stdin/stdout plumbing, exit codes. Thin. |
| `pathify-tui` | The interactive terminal map. Isolated so `ratatui` never reaches the pipeline commands. |

Every adapter reads into and writes out of one `Trace` model
(`Trace` → `Track` → `Segment` → `Point`), which is why `convert` is N readers
plus M writers rather than N×M conversions. The hierarchy mirrors GPX, the most
expressive of the supported formats, so reads widen and writes narrow.

Segment boundaries carry meaning: they mark where a recording paused or lost
signal. Distance is never summed across one, and elevation gain never counts a
jump across one as climb.

## Roadmap

- [x] Workspace, trace model, GPX adapter, `info`
- [x] `convert`, with the GeoJSON and CSV adapters
- [x] `merge` — overlap-aware concatenation and reconciliation
- [x] `clean` — drift filtering and location redaction
- [ ] KML adapter
- [ ] `view` — interactive braille-canvas terminal map
- [ ] FIT adapter
- [ ] Distribution via Homebrew and crates.io

## Development

```sh
cargo test                             # unit, integration, and doc tests
cargo clippy --all-targets             # lints
cargo fmt --all                        # formatting
./scripts/check-no-network-deps.sh     # the local-first guarantee
```

See [CONTRIBUTING.md](CONTRIBUTING.md) before adding a dependency.

## License

MIT — see [LICENSE](LICENSE).
