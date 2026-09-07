# Changelog

All notable changes to Pathify are documented here. This project follows
[Semantic Versioning](https://semver.org/): once released, a breaking change
to CLI flags, output formats, or exit codes requires a major version bump.

## Unreleased

- `view` now reports distance and elevation in feet and miles when the host's
  locale prefers imperial units (as `LC_ALL`, `LC_MEASUREMENT`, or `LANG`
  indicates), and in metres and kilometres otherwise.

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
