# Changelog

## Unreleased

### Added
- `/feedcheck`: probes every subscribed feed concurrently and reports the dead
  ones (HTTP error, unreachable, no longer valid RSS/Atom, empty, abandoned),
  alongside the failure history the scheduler recorded. Strictly read-only — it
  never writes `contents`, sends an article, or changes a source's state.
- `[fetch] max_item_age_days` (default 30, `0` disables): items published longer
  ago than this are recorded as seen but never pushed. Fixes `/check` and the
  scheduler blasting a feed's entire back catalogue after a GUID change or after
  `prune_contents` aged the ledger rows out.
- Migration `0005_source_health.sql` adds `sources.last_error`,
  `last_error_at`, `last_success_at`. The fetch error text previously only ever
  reached the server log.
- `/list` marks each source ✅/⚠️/⏸, and `/set` shows last-success and
  last-error.

### Fixed
- A single malformed feed no longer aborts the whole scheduler pass, starving
  every later due source; it is recorded as that source's error instead.
- `/check`'s parse failures are now recorded against the source, matching the
  scheduler.

## [Unreleased] 2019-10-29

### Added
- source update fetch control by `/set` command (usually used when source updating paused by reached 100 error when 
getting update data)
- export feeds

### Changed
- merge `/set` command's response message template to a function for more struct

