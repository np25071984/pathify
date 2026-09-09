# Changelog

All notable changes to Pathify are documented here. This project follows
[Semantic Versioning](https://semver.org/): once released, a breaking change
to CLI flags, output formats, or exit codes requires a major version bump.

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
