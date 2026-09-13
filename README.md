# Pathify

A local-first command-line toolkit for GPS trace data — inspect, clean, merge,
convert, view, and render GPX, TCX, GeoJSON, and CSV traces, and pull them out
of a Google Takeout archive, without a desktop GIS application and without
uploading anything anywhere.

> **Status: 1.4.** `info`, `convert`, `merge`, `clean`, and `view` are stable
> across GPX, TCX, GeoJSON, and CSV — flags, output formats, and exit codes
> won't break without a major version bump. `takeout` is new in 1.4 and
> `render` arrived in 1.3, so both sets of flags may settle further. KML and FIT
> adapters are still planned, as are Location History / Timeline exports for
> `takeout`, which today reads the Google Health / Fitbit ones — see
> [Roadmap](#roadmap).

## Format support

| Format | Read | Write | Notes |
| --- | :---: | :---: | --- |
| GPX | ✅ | ✅ | Tracks, routes, and loose waypoints |
| TCX | ✅ | ✅ | Heart rate, cadence, and speed carry over where present |
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

Via Homebrew:

```sh
brew tap np25071984/pathify https://github.com/np25071984/pathify
brew trust np25071984/pathify
brew install pathify
```

Via crates.io — requires Rust 1.88 or newer (note the `-cli`: the plain
`pathify` name on crates.io belongs to an unrelated crate):

```sh
cargo install pathify-cli
```

Or from source — requires Rust 1.88 or newer:

```sh
git clone https://github.com/np25071984/pathify.git && cd pathify
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
  bbox        -122.305600,47.653500,-122.274500,47.665100
```

Every command reads a file or piped input, writes results to stdout, and sends
diagnostics to stderr, so they compose:

```sh
cat ride.gpx | pathify info                    # format detected by sniffing content
pathify info ride.gpx --json | jq .distance_m  # machine-readable output
```

Omit the filename and Pathify reads whatever is piped in. Omit it with nothing
piped in and it says so rather than sitting there waiting on a keyboard nobody
is typing at — a forgotten filename is a mistake, not a request. To type a trace
in by hand, ask for stdin explicitly with `-`.

### Units

Every flag that measures something takes its unit inline, so a command says
what it means without you having to remember which flag counts in what:

```sh
pathify clean ride.gpx --trim-ends 200ft        # feet
pathify clean ride.gpx --max-speed 216km/h      # km/h
pathify info ride.gpx --elevation-threshold 3m  # meters
pathify merge a.gpx b.gpx --dedup-window 2min   # minutes
```

A bare number means the metric base unit — meters for a distance, seconds for
a duration, meters per second for a speed — which is what the flags have always
counted in, so existing scripts keep working unchanged.

| Kind | Units accepted | Bare number means |
| --- | --- | --- |
| `<DISTANCE>` | `m`, `km`, `cm`, `ft`, `yd`, `mi`, `nmi` | meters |
| `<DURATION>` | `s`, `ms`, `min`, `h` | seconds |
| `<SPEED>` | `m/s`, `km/h`, `mph`, `ft/s`, `kn` | meters per second |

Spelling is forgiving about case, spacing, and the long forms — `200FT`,
`200 ft`, and `200feet` are one value. Unit names are not interchangeable
across kinds: `--trim-ends 5min` is refused rather than guessed at, and on a
duration a bare `m` is refused as ambiguous, since it reads as minutes to one
person and meters to another.

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
| `--from <FORMAT>` | Override format detection (`gpx`, `tcx`, `fit`, `kml`, `geojson`, `csv`) |
| `--elevation-threshold <DISTANCE>` | Ignore elevation changes below this floor (default `3m`) |

`bounds` and `bbox` are the same box twice. `bounds` reads the way people say
coordinates — latitude first, corner to corner. `bbox` is longitude first, in
the order GeoJSON and every map service's `bbox=` parameter use, so it can be
pasted into a download without transposing the world. It carries two more
decimal places than `bounds` for the same reason: four is fine to read but is
about 11 m of error, which shows as a visible offset once a trace is drawn on
a map fetched with it. See [`pathify render`](#pathify-render).

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
| `--dedup-window <DURATION>` | Override the derived matching window |
| `--dedup-radius <DISTANCE>` | Override the speed-derived matching distance |
| `--segment-gap <DURATION>` | Gap that starts a new segment in reconciled output (default `120s`) |
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
pathify clean ride.gpx --trim-ends 300m > shareable.gpx    # hide both endpoints
pathify clean ride.gpx --trim-ends 1000ft > shareable.gpx  # same, in feet
pathify clean ride.gpx --redact-around 47.6535,-122.3056,300m
```

| Flag | Effect |
| --- | --- |
| `--trim-ends <DISTANCE>` | Remove everything near where the trace starts and ends |
| `--redact-around <LAT,LON,RADIUS>` | Remove everything inside a named circle. Repeatable |
| `--no-drift-filter` | Keep fixes that imply an impossible speed |
| `--max-speed <SPEED>` | Speed above which a step is a bad fix (default `60m/s`, about 216 km/h) |
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

- `--trim-ends <DISTANCE>` hides where a journey began and finished without you
  typing your home address into a shell command. It fences *both* endpoints and
  removes matching points wherever they appear, so a loop that passes the front
  door halfway round does not leak what the trim was meant to hide.
- `--redact-around <LAT,LON,RADIUS>` fences a location you can name, and can be
  given more than once.

Redaction **splits** the segment it cuts instead of closing the gap. Joining the
survivors would draw a straight line through the hidden area — inventing travel
that never happened, and leaving a chord pointing at exactly what was hidden.

### `pathify view`

```sh
pathify view ride.gpx
cat ride.gpx | pathify view      # piping the trace in is fine
```

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` or arrows | Pan |
| `+` `=` `i` / `-` `_` `o` | Zoom in / out |
| `f` `r` | Fit the whole trace |
| `?` | Show or hide the key list |
| `q` `Esc` `Ctrl-C` | Quit |

```text
┌ ride.gpx ────────────────────────────────────────────────────────────────────┐
│                                                  ⢀                           │
│                                                ⢠⠊⠁                           │
│                                              ⡠⠒⠁                             │
│                                            ⡠⠊                                │
│                                                                              │
│                                   ⡔                                          │
│                                 ⡠⠊                                           │
│                            ⢀⠔⠊                                               │
└──────────────────────────────────────────────────────────────────────────────┘
14 pts  ·  3.80 km     hjkl pan · +/- zoom · f fit · ? help · q quit
```

This is the one command that does not compose in a pipeline: it paints a screen
rather than emitting a document, so it needs a terminal and refuses a redirected
stdout with an explanation rather than failing somewhere deep inside a terminal
library. Piping the *trace* in still works — key presses are read from the
controlling terminal, not from stdin.

The status line shows metric distances by default, and switches to feet and
miles when the host's locale says it prefers imperial (checked via
`LC_ALL`, `LC_MEASUREMENT`, or `LANG`, in that order).

There are no map tiles and never will be: fetching them would mean network
access. What you get is the trace itself, drawn with braille dots at four times
the vertical resolution of the character grid, projected so a degree of
longitude is drawn shorter than a degree of latitude — without that correction
every route comes out stretched sideways.

If you want the trace over an actual map, that is
[`pathify render`](#pathify-render), which draws onto a map image you fetched
yourself. Pathify still makes no request either way.

Segments are drawn separately here too, so a pause or a dropout shows as a break
in the line rather than a stroke across ground nobody covered.

### `pathify takeout`

Pull GPS traces out of a Google Takeout export, locally, without unzipping two
gigabytes by hand and without uploading anything. If you do not have an export
yet, [start here](#getting-an-archive).

```sh
pathify takeout takeout-*.zip --list                      # what is in there
pathify takeout takeout.zip --type walk -o walks.gpx      # extract
pathify takeout takeout.zip                               # pick from a menu
```

| Flag | Effect |
| --- | --- |
| `--list` | Print the activity types and exit |
| `--json` | Print `--list` as JSON instead of a table |
| `--type <TYPES>` | Comma-separated types to extract, e.g. `"walk,outdoor bike"` |
| `--source <NAME>` | Keep this recording device where several logged the same journey |
| `--segment-gap <DURATION>` | Gap that starts a new segment (default `120s`) |
| `--to <FORMAT>` | Output format (default `gpx`, since a Zip implies none) |
| `-o, --output <FILE>` | Write to a file instead of stdout |
| `--per-activity` | Write one file per activity into the `--output` directory |
| `-v, --verbose` | Report what was found and skipped, on stderr |

Pass several archives at once for a [multi-part export](#getting-an-archive),
or name the directory you unzipped them into. Either works.

The unit of selection is the **activity type**, not the file, because an archive
holds hundreds of recordings spread across per-day files that do not correspond
to activities at all:

```console
$ pathify takeout takeout-20260909T182813Z-1-001.zip --list
activity type   logs  with GPS
Walk             144       117
Bike               1         1
Workout           29         0
Swim              12         0
Outdoor Bike      11         0
Rowing machine     6         0

203 logs, 118 with GPS, 79 days of recording
```

The `logs` column is every recording of that type; `with GPS` is how many of
them carry coordinates, which is how many tracks you get. The two differ
because a tracker logs plenty of activity it has no fix for — a pool swim, a
rowing machine, a walk the watch caught after the fact.

With no `--type` and a terminal at both ends, you get a menu instead. Types
with no GPS behind them are shown but cannot be ticked — a swim with no
coordinates is not something anyone can hand you as a track:

```text
5 activity logs, 3 with GPS, across 2 days of recording.

┌────────────────────────────────────────────────────────────────────────┐
│> [x] Walk          1 of 2 with GPS                                     │
│  [ ] Outdoor Bike  1 of 1 with GPS                                     │
│  [ ] Swim          1 of 1 with GPS                                     │
│  [ ] Workout       0 of 1 with GPS                                     │
└────────────────────────────────────────────────────────────────────────┘
↑/↓ move · space toggle · a all · enter confirm · q cancel
```

Piping needs `--type`, because the menu would paint over the screen and corrupt
the pipe:

```sh
pathify takeout takeout.zip --type walk | pathify view
pathify takeout takeout.zip --type "walk,outdoor bike" | pathify clean -o sports.gpx
pathify takeout takeout.zip --type walk | pathify render --fog --basemap map.png -o fog.png
```

#### Driving it from another program

Two flags exist for callers that are not a person at a terminal. `--list
--json` is the table above with the formatting taken off:

```console
$ pathify takeout takeout.zip --list --json
{
  "types": [
    { "name": "Walk", "logs": 144, "with_gps": 117 },
    { "name": "Bike", "logs": 1, "with_gps": 1 },
    { "name": "Workout", "logs": 29, "with_gps": 0 }
  ],
  "total_logs": 203,
  "total_with_gps": 118,
  "days_of_recording": 79
}
```

`--per-activity` writes one file per activity into a directory instead of
welding every match into one trace, which is what an importer that takes one
activity per record wants:

```console
$ pathify takeout takeout.zip --type walk --per-activity -o walks/
$ ls walks/
20260711T113000Z-walk.gpx  20260713T081500Z-walk.gpx  20260714T173000Z-walk.gpx
```

The directory is created if it is not there, and `--to` chooses the format of
every file in it. Names are the activity's own start in UTC and its type, so
they sort chronologically, two walks on one day are two files, and running the
same command twice rewrites the same files rather than accumulating copies.
(Two logs of one type that start in the same second — a phone and a watch that
each filed the same walk — take their Fitbit log ids as well.)

Without `--per-activity`, `-o` keeps naming a single file holding every
matched activity, as it always has.

#### Getting an archive

Exports are made at [takeout.google.com](https://takeout.google.com). The page
gets rearranged from time to time, but the shape of it does not:

1. **Deselect all**, then tick **Google Health** — the product carrying Fitbit's
   data, and the one `takeout` reads. Older exports list it as **Fitbit**.
   Ticking everything instead produces a far larger archive carrying far more
   about you than a GPS tool has any use for. (Ticking **Location History
   (Timeline)** as well does nothing for `takeout` yet — see the end of this
   section.)
2. On the next step choose **Send download link via email**, **Export once**,
   and **.zip**.
3. Set the maximum archive size. Anything smaller than the total splits the
   export into `…-001.zip`, `…-002.zip` and so on. Pass all of them at once —
   the exercise logs and the GPS days can land in different parts, and neither
   is any use alone.
4. **Create export**, and wait. Minutes for one product, longer for a big
   account. An email arrives with a link, which expires after a few days and
   allows only a handful of downloads, so fetch it when it turns up.
5. Leave the Zip zipped. `takeout` reads it in place, and unzipping two
   gigabytes gains you nothing — though if you already have, pointing it at the
   unpacked directory works just as well.

Then run `--list` against it before building anything on top of it. Ticking
the wrong product is the common mistake, and the fix is a new export rather
than a different command, so the error names what the archive turned out to
hold:

```console
$ pathify takeout takeout-20260909T182813Z-1-001.zip --list
pathify: no location data in takeout-20260909T182813Z-1-001.zip.
`takeout` reads Google Health / Fitbit exports. This archive holds: Takeout/YouTube and YouTube Music.
Re-export from takeout.google.com with Google Health (or Location History) selected.
```

None of this is automated, and it will not be: fetching an export means talking
to Google, and Pathify has no network code.

#### What it actually does

An export keeps the coordinates and the activities in different places, and
neither is any use alone. `Physical Activity_GoogleData/gps_location_*.csv` has
the fixes, one file per calendar day at roughly 1 Hz.
`Global Export Data/exercise-*.json` has the logs — the records saying that a
particular seventy minutes of one of those days was a bike ride. So `takeout`
joins them, and three things about that are worth knowing:

- **A day file is a day, not an activity.** One file can hold a morning errand,
  a commute and an evening ride. Points are sliced to the log's window, and a
  window that runs past midnight UTC is stitched from both day files.
- **Most days carry two devices.** A phone and a watch record the same journey a
  few meters apart, interleaved in the same file. Concatenating them gives a
  zigzag with roughly twice the real distance, so one is kept: the one that saw
  most of the activity, unless `--source` names the other.
- **The logs' clock carries no offset.** Google writes `07/11/26 14:55:42` with
  no timezone at all, and it has not always been UTC. Rather than assume,
  `takeout` measures it — the offset that lands the activity windows on
  actually-recorded points wins — and `--verbose` reports what it settled on.

`--verbose` also reports the shortfall, which is real: an archive can have more
logs claiming GPS than it has day files to back them.

Nothing is unzipped, and only the entries above are ever read — a Takeout export
carries sleep, heart rate, glucose and menstrual health alongside the locations,
and none of it is opened, copied, or emitted. The archive itself is never
modified. The exercise logs carry a `tcxLink` pointing at `fitbit.com`; Pathify
ignores it, because Pathify has no network code.

One thing worth doing before you share any of this: the first and last points of
nearly every one of these tracks are a home address. `pathify clean --trim-ends
300m` removes them without your having to name the place, and
[`--redact-around`](#pathify-clean) fences one you can name.

Location History / Timeline exports are recognized and reported, but not read
yet. `takeout` currently reads Google Health / Fitbit exports.

### `pathify render`

```sh
pathify render ride.gpx --basemap map.png -o ride.png     # track over the map
pathify render ride.gpx --basemap map.png --fog -o fog.png # fog of war
cat map.png | pathify render ride.gpx > ride.png           # the map, piped in
pathify clean ride.gpx | pathify render --basemap map.png > ride.png
```

| Flag | Effect |
| --- | --- |
| `--basemap <FILE>` | The map image, as a PNG. Omit it to read a piped image |
| `--bbox <MIN_LON,MIN_LAT,MAX_LON,MAX_LAT>` | Area the basemap covers. Defaults to the trace's own bounds |
| `--fog` | Darken the map and clear it again only along the route |
| `--reveal <DISTANCE>` | Width of the cleared corridor either side of the route (default `6m`, or `20ft` on an imperial locale) |
| `--fog-opacity <0..1>` | How dark the fog is (default `0.65`) |
| `--track-color <#RRGGBB>` | Colour of the drawn track |
| `--track-width <PX>` | Stroke width of the drawn track |
| `--from <FORMAT>` | Override input format detection for the trace |
| `--output, -o <FILE>` | Write the PNG to a file instead of stdout |

Pathify does not download the map, and cannot: there is no network code to do
it with. `render` takes a PNG you already have, plus the geographic box that
image covers, and composites the trace onto it. That split is the whole design
— the fetching happens in your shell, where you can see it, and the trace never
leaves the machine.

Three steps:

```sh
pathify info ride.gpx                 # read the bbox row
# fetch a PNG of that box, however you like
pathify render ride.gpx --basemap map.png -o ride.png
```

Omitting `--bbox` assumes the image spans exactly the trace's own bounds, which
is true when you fetched it for the bbox `info` printed. Pass `--bbox`
explicitly whenever it isn't — a screenshot, a wider area, a map you already
had. Pathify cannot check: a PNG carries no geographic metadata, so the box is
something it has to be told. What it can do is notice when the image is not the
*shape* its box implies, and it says so on stderr rather than handing back a
picture that looks right and puts the route in the wrong place.

#### Getting a basemap

Any PNG rendered in Web Mercator works — which is every general-purpose web
map. What matters is knowing the box it covers.

OpenStreetMap's own `cgi-bin/export` endpoint used to take a `bbox=` and now
requires a token, so the dependable route is tiles. A tile's box is exactly
computable from its `z/x/y`, which makes it the one basemap whose extent you
never have to guess:

```sh
curl -A 'my-tool/1.0 (contact@example.com)' \
  -o map.png https://tile.openstreetmap.org/15/5251/11437.png

pathify render tests/fixtures/ride.gpx --basemap map.png \
  --bbox -122.310791,47.650588,-122.299805,47.657988 -o ride.png
```

That box comes from the standard tile formulas — for tile `z/x/y`, longitude is
`x / 2^z × 360 - 180` and latitude is `atan(sinh(π × (1 - 2y / 2^z)))` in
degrees, taking `x`/`y` for one edge and `x+1`/`y+1` for the other. Stitch a
grid of tiles for a bigger area and the box is the union, computed the same
way.

A box that does not overlap the trace is caught rather than rendered as an
untouched copy of the map, which is the usual sign of a tile picked one row
off:

```console
$ pathify render ride.gpx --basemap map.png --bbox -122.310791,47.657988,-122.299805,47.665387
pathify: no part of the trace falls inside the basemap's bounding box
```

Any static-map service that accepts a bounding box works too, and gives you the
box for free. Whichever you use, it is *their* service: mind the usage policy,
and do not point a loop at a volunteer-run tile server.

#### The two modes

Plain mode strokes the track over the map, with a green mark where the trace
starts and a red one where it ends — the same colour meanings the terminal map
uses.

`--fog` does something different. It darkens the whole basemap and lifts the
darkness along the route, so the only map you can read is the ground you
actually covered. No line is drawn in this mode: the cleared corridor *is* the
track, and a stroke down the middle of it would only repeat what the shape
already says. That is also why `--track-color` and `--track-width` are refused
with `--fog` rather than silently ignored.

`--reveal` is a ground distance, not a pixel count, so the same command clears
a comparable corridor whatever the scale of the map. It takes its unit inline
like every other measured flag — `--reveal 20ft` is feet, and a bare
`--reveal 20` is meters wherever you are. The host locale only chooses the
default, so saying nothing gets you a round `20ft` where imperial is preferred
and `6m` otherwise, the same check the terminal map's status line makes.

Segments are kept apart here as everywhere else. A pause or a dropout leaves a
gap in the stroke, and in fog mode leaves the ground between two segments
fogged: nobody covered it, so it is not revealed.

Being a document rather than a screen, `render` composes like the other
commands — it writes a PNG to stdout, and refuses to do so when stdout is a
terminal, since that is a screenful of garbage and sometimes a wedged shell.

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

Four crates, so the spatial logic stays free of interface concerns:

| Crate | Responsibility |
| --- | --- |
| `pathify-core` | Trace model, format adapters, spatial math. No CLI, TUI, or network dependencies. |
| `pathify-cli` | Argument parsing, stdin/stdout plumbing, exit codes. Thin. |
| `pathify-render` | Web Mercator projection and PNG compositing for `render`. Isolated so `png` never reaches the pipeline commands. |
| `pathify-tui` | The interactive terminal map, and the multi-select menu `takeout` asks with. Isolated so `ratatui` never reaches the pipeline commands. |

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
- [x] `view` — interactive braille terminal map
- [x] `render` — trace drawn onto a bitmap basemap, plain or fog-of-war
- [x] `takeout` — GPS traces joined out of a Google Takeout archive
- [ ] `takeout`: Location History / Timeline exports
- [x] TCX adapter
- [ ] KML adapter
- [ ] FIT adapter
- [x] Distribution via Homebrew
- [x] Distribution via crates.io

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
