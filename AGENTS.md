# AGENTS.md

Guidance for AI coding agents working in this repository. Human-facing docs live
in [README.md](README.md) (usage) and [CONTRIBUTING.md](CONTRIBUTING.md)
(conventions and rationale) — read CONTRIBUTING.md before adding a dependency or
touching the trace model.

## What this is

Pathify is a local-first Rust CLI and TUI for GPS trace data: inspect, clean,
merge, convert, view, and render GPX, TCX, GeoJSON, and CSV traces. Cargo
workspace, edition 2024, MSRV 1.88. The binary is `pathify`, built from
`crates/pathify-cli`.

## Commands

```sh
cargo test --all-features              # unit, integration, and doc tests
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all                        # run before committing
./scripts/check-no-network-deps.sh     # the local-first guarantee
cargo run -- info tests/fixtures/ride.gpx   # exercise the binary in-tree
```

CI runs all four on Linux and macOS, with `cargo fmt --all -- --check` and
clippy warnings as errors. Match that locally before proposing a change.

## The one hard rule: no network code

Pathify must be *incapable* of making a request, not merely configured not to.
No dependency may perform network I/O, directly or transitively — no HTTP
clients, async runtimes that bundle networking, TLS stacks, or DNS resolvers.
`scripts/check-no-network-deps.sh` enforces this against `Cargo.lock` and gates
CI.

Do not add an "offline mode" flag, and do not suggest one: a setting can be
changed, an absent dependency cannot. If the script flags a crate you believe is
a false positive, remove it from the list in that script *in the same commit*,
with a comment explaining why it cannot reach the network.

## Layout

| Crate | Responsibility |
| --- | --- |
| `pathify-core` | Trace model, format adapters, spatial math. Must not depend on `clap`, `ratatui`, or anything terminal-related. |
| `pathify-cli` | Argument parsing, stdin/stdout plumbing, exit codes. Thin. |
| `pathify-render` | Web Mercator projection and PNG compositing for `render`. Depends on core and `png` only; only the `render` command depends on it. |
| `pathify-tui` | The interactive terminal map. Only the `view` command depends on it. |

Logic that could be unit-tested belongs in `pathify-core`. Never print or exit
from core — return an error instead. Each `pathify-cli` subcommand lives in
`crates/pathify-cli/src/commands/`; shared I/O rules are in `io.rs` and unit
parsing in `units.rs`.

The two projections are deliberate, not duplication: the terminal map projects
equirectangularly (right for a trace on its own), and `render` projects Web
Mercator (right over a downloaded map). Using one where the other belongs shears
the track off the roads it followed.

## Invariants that produce plausible wrong numbers when broken

- **Segment boundaries are gaps.** A break means the recording paused or lost
  signal. Never sum distance across one; never count the elevation step across
  one as climb.
- **Absent is not zero.** `elevation: None` means unrecorded. Substituting `0.0`
  silently means sea level in every downstream calculation. CSV writes absent
  values as empty cells.
- **Coordinates are validated at construction.** Build points with `Point::new`
  (rejects non-finite values, wraps longitude) rather than assembling the struct
  literal from parsed input.
- **Reads widen, writes narrow.** Every adapter reads into the full `Trace` →
  `Track` → `Segment` → `Point` model; the write path decides what the target
  format cannot represent. Document losses in the adapter and pin them with a
  test.
- **Redaction splits, never joins.** Closing the gap draws a straight line
  through the hidden area and leaves a chord pointing at it.

## CLI conventions

- Results to stdout, diagnostics to stderr, so commands compose. `-` means
  stdin; a missing filename at an interactive terminal is an error, not a wait.
- Exit codes are contractual: `0` success, `1` bad input, `2` I/O failure.
- Every numeric flag that measures something accepts its unit inline
  (`--trim-ends 200ft`, `--dedup-window 2min`, `--max-speed 25km/h`). A bare
  number is the metric base unit — meters, seconds, meters per second. Parsers
  in `units.rs` return base units so the rest of the program never asks what a
  value is in. Add a new measured flag the same way; do not introduce a
  unitless-with-separate-`--unit`-flag style.
- Units are not interchangeable across kinds, and a bare `m` on a duration is
  refused as ambiguous rather than guessed at.
- `info`, `convert`, `merge`, `clean`, and `view` are stable as of 1.x: flags,
  output formats, and exit codes do not break without a major bump. `render`
  flags may still settle.

## Testing expectations

- **Spatial math** asserts against known real-world reference values, not
  self-consistency — a symmetric round-trip test passes happily with latitude
  and longitude transposed. See `spatial::distance`, and `pathify-render`'s
  `mercator`, which checks published EPSG:3857 northings.
- **Redaction** gets fixture-based tests asserting sensitive coordinates never
  appear in output; a false negative is a privacy leak, not a cosmetic bug.
- **New format adapters** need a fixture in `tests/fixtures/` plus tests for
  reading, writing, a round trip through the model, and a malformed input that
  errors rather than panics.
- Fixtures live at the workspace root (`tests/fixtures/`), which is why
  `pathify-cli` sets `exclude = ["tests/*"]` — its integration tests reach
  outside the crate and cannot be packaged. Do not "fix" that exclude.

## Releasing

The four crates are versioned in lockstep via `workspace.package.version` and
published in dependency order: `pathify-core`, then `pathify-tui` and
`pathify-render` in either order, then `pathify-cli`. Each step needs the
previous one indexed on crates.io, since crates depend on each other by path
*and* version. Homebrew formula: `Formula/pathify.rb`. Changelog:
`CHANGELOG.md`.
