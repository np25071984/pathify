# Contributing to Pathify

## The one hard rule: no network code

Pathify's central promise is that your location history never leaves your
machine. That is only credible if the program is *incapable* of making a
request, so:

- **No dependency may perform network I/O**, directly or transitively. This
  includes HTTP clients, async runtimes that bundle networking, TLS stacks, and
  DNS resolvers.
- `scripts/check-no-network-deps.sh` enforces this against `Cargo.lock` and runs
  in CI. A pull request that trips it will not pass.
- If you believe a flagged crate is a false positive, remove it from the list in
  that script **in the same commit**, with a comment explaining why it cannot
  reach the network.

There is no "offline mode" flag and there should never be one. A setting can be
changed; an absent dependency cannot.

## Where code belongs

- `pathify-core` — the trace model, format adapters, and spatial math. This
  crate must not depend on `clap`, `ratatui`, or anything terminal-related. If
  you find yourself wanting to print or exit here, return an error instead.
- `pathify-cli` — argument parsing, input/output plumbing, and turning errors
  into messages and exit codes. Keep it thin; logic that could be unit-tested
  belongs in core.
- `pathify-tui` — the interactive map. Nothing else may depend on it except the
  `view` command.
- `pathify-render` — Web Mercator projection and PNG compositing. Depends on
  core and `png` and nothing else; nothing may depend on it except the `render`
  command. Note that this is a *second* projection, deliberately: the terminal
  map projects equirectangularly, which is right for a picture of a trace on
  its own, and wrong over a downloaded map, which is Mercator. Drawing one with
  the other shears the track away from the roads it followed.

## Conventions that carry meaning

A few model invariants are load-bearing, and breaking them produces numbers that
look plausible and are wrong:

- **Segment boundaries are gaps.** A segment break means the recording paused or
  lost signal. Never sum distance across one, and never count the elevation step
  across one as climb.
- **Absent is not zero.** `elevation: None` means the source did not record an
  elevation. Do not substitute `0.0` — it silently becomes sea level in every
  downstream calculation.
- **Coordinates are validated at construction.** Build points with
  `Point::new`, which rejects non-finite values and wraps longitude, rather than
  assembling the struct literal from parsed input.
- **Reads widen, writes narrow.** Every adapter reads into the full model; the
  write path decides what the target format cannot represent. Where a write
  loses data, say so in the adapter's documentation and pin it with a test.

## Testing

```sh
cargo test                             # everything
cargo clippy --all-targets             # lints must be clean
cargo fmt --all                        # before committing
./scripts/check-no-network-deps.sh
```

Two areas want more rigor than ordinary code:

- **Spatial math.** Assert against known real-world reference values, not just
  self-consistency — a symmetric round-trip test passes happily with latitude
  and longitude transposed. See `spatial::distance` for the pattern, and
  `pathify-render`'s `mercator` module, which checks published EPSG:3857
  northings rather than its own output fed back in.
- **Redaction.** When location redaction lands, a false negative is a real
  privacy leak rather than a cosmetic bug. It gets fixture-based tests asserting
  that sensitive coordinates never appear in output.

New format adapters need a fixture in `tests/fixtures/` and, at minimum, tests
for reading, writing, a round trip through the model, and a malformed input that
must produce an error rather than a panic.

## Releasing

The four crates are versioned in lockstep via `workspace.package.version`, and
published to crates.io in dependency order: `pathify-core` first, then
`pathify-tui` and `pathify-render` in either order, then `pathify-cli`. Every
crate depends on the others by path *and* version
(`workspace.dependencies`), so each `cargo publish` step needs the ones before
it to have finished indexing — that usually takes well under a minute, but a
publish that starts too soon fails with "no matching package found" rather than
silently using the old version.

`pathify-cli`'s own integration tests are excluded from its published package
(`exclude = ["tests/*"]`): they reach for fixtures in the workspace-root
`tests/fixtures/`, which sits outside the crate and cannot be packaged
alongside it. This only affects `cargo test` on a downloaded source
distribution — `cargo install pathify-cli` never runs tests.
