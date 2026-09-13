# `takeout-sample`

A synthetic Google Takeout export, laid out exactly like a real Google Health
one but four kilobytes instead of two gigabytes. `crates/pathify-cli/tests/takeout.rs`
reads it both as a directory and zipped into a temporary file, because
`pathify takeout` accepts either.

Every awkward thing in it is deliberate, and each reproduces something the real
export does:

| In here | Why |
| --- | --- |
| Two `data source` values interleaved in `gps_location_2026-07-11.csv` | Most days carry a phone *and* a watch recording the same journey. Reading a day file as-is gives a doubled, zigzagging track. |
| `gps_location_2026-07-11.csv` holds three unrelated outings | A day file is a day, not an activity, so the points have to be sliced by the exercise log's window. |
| A six-minute hole inside log 1001's window | Segment splitting: a gap longer than `--segment-gap` starts a new segment rather than drawing a line across it. |
| Log 1003 runs from 23:55 to 00:05 | A window can straddle midnight UTC, so two day files have to be read and stitched. |
| Log 1004 (`Swim`) claims GPS, and there is no `gps_location_2026-07-20.csv` | The join is not total in real archives either. It has to be reported, not hidden. |
| `gps_location_readme.txt` | The real export ships one. A reader that globs `gps_location_*` tries to parse it as coordinates. |
| A fractional-second timestamp | The real day files mix `…:17Z` and `…:17.328Z`. |
| `tcxLink` on two logs | It points at `fitbit.com`. Pathify has no network code and must never follow it. |
| `Menstrual Health/` | A Takeout export carries health data far more sensitive than the locations. Nothing outside the two file families is ever read, and a test asserts the marker string in here never reaches the output. |
