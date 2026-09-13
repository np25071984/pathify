# Changelog

All notable changes to Pathify are documented here. This project follows
[Semantic Versioning](https://semver.org/): once released, a breaking change
to CLI flags, output formats, or exit codes requires a major version bump.

## 1.4.0

### Added

- `takeout`, which pulls GPS traces out of a Google Takeout export without
  unzipping two gigabytes by hand and without uploading anything. It reads the
  Google Health / Fitbit exports — the ones with a `Physical
  Activity_GoogleData` folder — by joining the two file families that are no
  use apart: the per-day GPS CSVs, and the exercise logs that say which stretch
  of a day was a bike ride. The Zip is read in place, and only those two
  families are ever opened, which matters more here than anywhere else in
  Pathify: an export carries sleep, heart rate and menstrual health beside the
  locations, and the logs' `tcxLink` points at `fitbit.com`, which nothing in
  Pathify can follow.
  - The unit of selection is the activity type, not the file, because a day
    file is a day: it runs midnight to midnight, holds unrelated outings, and
    interleaves every device that was recording. `--type "walk,outdoor bike"`
    names types on the command line, `--list` prints the ones an archive holds
    with how many of each carry coordinates, and with a terminal at both ends
    you get a menu instead.
  - Where a phone and a watch both recorded one journey, one is kept — the one
    that saw the most of it, unless `--source` names another. Concatenating the
    two would report roughly twice the distance along a plausible-looking
    zigzag.
  - The exercise logs' start times carry no UTC offset, so the offset is
    measured rather than assumed: the shift that lands the most activity
    windows over recorded points wins. Assuming UTC would silently emit empty
    tracks for everyone it is not true for, and `--verbose` says so when no
    shift lands on anything.
- `takeout --list --json` emits that listing as JSON — each activity type with
  its log count and how many carry GPS, plus the totals — shaped like
  `info --json`, for a program deciding what to ask for rather than a person
  reading a table.
- `takeout --per-activity` writes one file per activity into the `--output`
  directory instead of welding every match into a single trace, for importers
  that take one activity per record and would otherwise have to work out where
  one outing ends and the next begins. Names come from the activity's own UTC
  start and its type — `20260711T113000Z-outdoor-bike.gpx` — so a listing is
  chronological, two outings of one type on one day are two files, and running
  the same command twice rewrites its own output rather than accumulating
  copies. `-o` without the flag still writes the one combined file it always
  did.

### Internal

- New `pathify_core::takeout` module. `Archive` reads a Zip and an unpacked
  directory through one interface, so a multi-part export and a folder someone
  already unzipped take the same path; `Backend` names what an export turned
  out to hold, so a Location History archive is reported rather than misread as
  an empty one.
- `pathify-tui` gained the multi-select menu `takeout` asks with, beside the
  map, so `ratatui` still never reaches the pipeline commands.
- `split_on_gaps` and the CSV column-alias table are now shared rather than
  copied: the Takeout reader segments on the same rule `merge` does, and
  recognizes the same coordinate columns the CSV adapter does, with one of its
  own (`data source`) on top.
- `AGENTS.md` now states the rules an agent working in this repository has to
  know before it writes anything — the no-network rule, the crate boundaries,
  and the invariants that produce plausible wrong numbers when broken — and the
  release process itself is written down as a skill rather than recalled a step
  at a time.

## 1.3.0

### Added

- `render`, which draws a trace onto a map image and writes a PNG. Pathify
  still fetches nothing: `render` takes a basemap PNG you already have, plus
  the geographic box that image covers, so the network half of the job stays in
  your shell. Two modes — the track stroked over the map, or `--fog`, which
  darkens the whole map and clears it again along the route so the only map you
  can read is the ground you covered.
  - Either input may be the piped one, decided by content rather than
    position: a PNG signature is the basemap, anything else is the trace. So
    `cat map.png | pathify render ride.gpx` and
    `pathify clean ride.gpx | pathify render --basemap map.png` both work.
  - `--bbox` defaults to the trace's own bounds, which is correct when the
    image was fetched for the bbox `info` reports. A basemap whose proportions
    do not match the box it is said to cover is reported on stderr, since that
    is the signature of an image fetched for a different area.
  - `--reveal`, the width of the corridor fog lifts along, takes its unit
    inline like every other measured flag: `--reveal 20ft` is feet, a bare
    `--reveal 20` is meters. The host locale chooses only the default, so it
    can no longer change what a radius you wrote down means.
- `info` now reports a `bbox` row, and a `bbox` array in `--json`: the same box
  as `bounds` but longitude first, in the order GeoJSON and map services'
  `bbox=` parameters use, and to six decimal places so it can be pasted into a
  download without a visible offset.

### Internal

- New crate `pathify-render`, isolated the way `pathify-tui` is, so the `png`
  dependency never enters the pipeline commands' dependency graph. It carries
  its own Web Mercator projection: the terminal map's equirectangular one is
  right for a trace drawn on its own and wrong over a downloaded map.
- `Units` moved from `pathify-tui` to `pathify-core`, so `render` can read the
  locale for `--reveal` without depending on the terminal crate. Still
  re-exported from `pathify-tui`.
- `render --fog` no longer slows down as the corridor widens. It was flooding
  outward from every point in turn, so the cost went with the points times the
  square of the reveal radius while the image it was drawing on stayed the same
  size: a lifetime-scale trace of 168,000 points on a 4000×3000 basemap took
  5.5 s at a 100 m reveal, 29 s at 250 m, and seven minutes at 1 km, doing the
  same pixels over and over. The corridor is now measured once and swept across
  the image, which takes about 0.8 s at every one of those radii. Narrow
  corridors, where flooding is exact and cheap, still flood — output below the
  switch is byte-identical, and above it the two agree to well under a pixel.

## 1.2.0

### Added

- Every numeric flag that measures something now takes its unit inline:
  `--trim-ends 200ft`, `--elevation-threshold 3m`, `--max-speed 216km/h`,
  `--dedup-window 2min`, and the radius of `--redact-around 47.65,-122.31,500ft`.
  Distances accept `m`, `km`, `cm`, `ft`, `yd`, `mi`, and `nmi`; durations `s`,
  `ms`, `min`, and `h`; speeds `m/s`, `km/h`, `mph`, `ft/s`, and `kn`. A bare
  number still means the metric base unit each flag already counted in — meters,
  seconds, or meters per second — so existing commands and scripts are
  unaffected. A unit from the wrong kind is refused rather than guessed at, as
  is a bare `m` on a duration, which reads as minutes to one person and meters
  to another. `--help` now spells the defaults with their units (`3m`, `120s`,
  `60m/s`), and a negative value is refused for every measure, not just a fence
  radius.

### Fixed

- `clean --redact-around` accepted no latitude south of the equator. A leading
  minus sign made the argument parser read the whole value as an unknown flag,
  so every southern-hemisphere fence failed with `unexpected argument '-3'` —
  a message naming neither the flag nor the problem — and the fence's own
  validation never ran to explain it. Redaction by coordinate was unusable for
  half the planet unless you knew to write `--redact-around=<value>`. It failed
  closed, so nothing leaked.

## 1.1.0

- `view` now reports distance and elevation in feet and miles when the host's
  locale prefers imperial units (as `LC_ALL`, `LC_MEASUREMENT`, or `LANG`
  indicates), and in metres and kilometres otherwise.
- Added a TCX adapter (read and write): `Activity` maps to a track and each
  `Track` element — a lap can hold more than one, marking a GPS gap — to a
  segment. Heart rate, cadence, and speed carry over where the source records
  them; a trackpoint with no `<Position>` (an indoor activity, say) is dropped
  rather than invented as `(0, 0)`.

## 1.0.0

First stable release.

### Commands

- `info` — summarize a trace: points, distance, duration, elevation gain, segment count.
- `convert` — read and write between supported formats through a shared trace model.
- `merge` — combine traces, automatically choosing overlap-aware reconciliation
  (matching the same moment recorded by two devices) or plain concatenation
  (recordings that don't overlap in time).
- `clean` — drift filtering (implausible GPS jumps) and location redaction.
- `view` — interactive braille terminal map, with panning and zoom.

### Formats

- GPX — tracks, routes, and loose waypoints (read and write).
- GeoJSON — timestamps via the `coordTimes` convention (read and write).
- CSV — a documented column schema (read and write).

KML and FIT adapters are planned but not yet implemented — see the
[Roadmap](README.md#roadmap).

### Guarantees

- Local-first: no network I/O anywhere in the dependency graph, enforced in
  CI (`scripts/check-no-network-deps.sh`), not just disabled by a setting.
- Pipeline-friendly: `-` means stdin, results go to stdout, diagnostics never
  do, and a missing argument at an interactive terminal is reported rather
  than silently waited on.
