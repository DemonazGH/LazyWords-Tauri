# Changelog

## 1.1.0 - Release candidate (unreleased)

- Quick Add opens from the tray and can prefill a term from the local clipboard. Save a card to My Words without switching decks, or choose **Save and practice My Words** to practice it immediately and return to your previous deck when finished.
- Edit or delete cards in My Words. Editing a card preserves its learned mark and practice history.
- Each global shortcut can be changed or turned off independently. The four 1.0.2 bindings carry over; Quick Add has no shortcut until you assign one.
- Optionally delay an answer by 1–5 seconds or show new and less-practiced cards more often. Both options start off; learned cards stay out of rotation.
- Upgrades preserve the original learned file. Marks for verified bundled decks are linked to stable card IDs; imported or ambiguous marks remain pending for review rather than silently moving to a different card. Unreadable settings, learned data, or statistics are not silently replaced with defaults.

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
