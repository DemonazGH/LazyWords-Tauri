# Changelog

## 1.0.2 - 2026-10-16

Reliability updates for custom flashcards, settings, onboarding, and Windows
installation. Bring your own CSV, JSON, or XLSX dictionary with `term` and
`definition` columns, or use one of the three built-in starter packs.

### Fixed

- Long cards wrap and resize to fit the available monitor work area.
- Rapid settings changes are merged and saved in order; failed saves no longer
  show a misleading success indicator.
- Imported dictionary selection stays current when asynchronous requests finish
  out of order.
- Switching dictionaries invalidates the previous card so a shortcut cannot mark
  a card in the wrong dictionary as learned.
- Learning the final manually displayed card immediately starts completion and
  dictionary switching; stale timer notifications cannot hide a new card early.
- Hotkey conflicts produce a warning instead of preventing application startup.
- Interrupted first launch remains recoverable. Successful onboarding destroys
  its native window; import cancellation and errors leave selection retryable.
- MSI removal clears only the startup entry owned by that installation and
  preserves another LazyWords installation's entry and the user's data.
- Store MSIX startup uses Windows StartupTask APIs, respects externally disabled
  startup, and keeps WinRT calls on a process-lifetime worker.

### Packaging

- Clean Store MSIX packaging uses an explicit file allowlist instead of capturing
  an installation, excluding user profiles, imported decks, progress, and caches.
- GitHub release builds run JavaScript and Rust regressions before producing
  Windows x64 MSI and NSIS installers, with SHA256 checksums.
- Release builds create a draft release so installer checks can finish before
  publication. Store and GitHub builds are separate artifacts.
