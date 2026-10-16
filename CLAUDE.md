# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Development (hot-reload)
npm run tauri dev

# Production build
npm run tauri build

# Rust-only checks (faster than full dev)
cd src-tauri && cargo check
cd src-tauri && cargo build
```

```bash
# Run Rust unit tests
cd src-tauri && cargo test
```

Run frontend and packaging regressions with `npm test`.

No lint scripts are configured.

## Architecture

LazyWordsTauri is a Tauri v2 desktop app (Rust backend + Vanilla JS frontend) that periodically displays floating flashcard overlays for any subject (vocabulary, verbs, definitions, custom content).

### Data Format

**Standard column names** (CSV/JSON/XLSX):
- `term` — front of the card
- `definition` — back of the card

**Backward-compatible aliases** accepted by `dictionary_loader.rs` (case-insensitive):
- `word` / `translation` — legacy format
- `front` / `back` — alternative

If no recognised column names are found, the first column is treated as `term` and the second as `definition`.

JSON files support both `{"term": ..., "definition": ...}` and legacy `{"word": ..., "translation": ...}` via serde `#[serde(alias)]`.

### Bundled Dictionaries (`src/dictionaries/`)

| File | Contents |
|---|---|
| `en-ru.csv` | ~2809 NGSL English words → Russian translations |
| `en-definitions.csv` | ~2809 NGSL words with short English definitions |
| `en-irregular-verbs.csv` | ~200 complete English irregular verbs (base / past / past participle) |

### Backend (src-tauri/src/)

**AppState** (`state.rs`) is the central Mutex-protected state shared across all Rust modules:
- `settings` – user config (intervals, font size, position, language, autostart, first_launch)
- `word_engine` – holds loaded dictionary + `HashSet<usize>` of learned card indices
- `stats_tracker` – daily/streak statistics
- `learned` – `HashMap<dict_name, Vec<usize>>` persisted across sessions
- `is_paused`, `is_card_visible`, `current_word`, `all_learned`, `all_dicts_done` – runtime state
- `all_learned_notify` – wakes card-duration and interval waits when completion becomes pending
- `manual_trigger` – wakes timer interval when `Ctrl+Shift+N` / tray "Next" shows a card manually
- `tray` – `Option<TrayIcon>` handle for updating icon and menu from anywhere

**Timer loop** (`timer.rs`) — the core async loop:
- Sleeps 5 s/cycle when `active_dictionary` is empty (no dictionary selected yet)
- Handles pending `all_learned` immediately; sleeps 30 s/cycle only when `all_dicts_done` is set
- Checks fullscreen app detection on Windows (skips card if foreground app is fullscreen)
- Records `shown` stat + emits `stats-updated` on every card shown
- Interval wait prioritizes completion over manual Next and the one-second tick. Both waits register before checking pending completion and ignore stale notifications, including late wakeups after Restore.
- When `get_random_word()` returns `None`: sets `all_learned = true`, emits `show-all-learned`, then scans all dictionaries for one with remaining cards — if found emits `show-switched-dict` + `dict-auto-switched` and resets; if not found emits `show-all-dicts-learned`

**System tray** (`tray.rs`):
- Tray icon with normal/dimmed (paused) states; tooltip "LazyWords"
- Menu: Pause/Resume, Next word, Dictionary submenu (active dict has checkmark), Settings, Quit
- All labels localized via `t()` — menu rebuilt on language change
- Quit saves learned/stats/settings before `app.exit(0)`

**Global shortcuts** (`lib.rs`):
- `Ctrl+Shift+W` – toggle settings window (show/hide pre-created window)
- `Ctrl+Shift+P` – pause/resume; updates tray icon and menu
- `Ctrl+Shift+N` – implicit unpause + show next card immediately; records `shown` stat; fires `manual_trigger` to reset timer interval
- `Ctrl+Shift+K` – emits `mark-known-shortcut`; card.js handles it
- All four shortcuts dismiss the onboarding window if it's open
- Registration errors are logged per shortcut and shown in a localized warning; the tray and settings remain usable.

**Window management** (`lib.rs`):
- Card window: transparent, always-on-top, skip-taskbar, `shadow(false)`, ignore cursor events; renderer measurement resizes it within the monitor work area and DPI limits.
- Settings window: pre-created hidden at startup with `skip_taskbar(true)`; `CloseRequested` intercepted with `prevent_close()` + `hide()` so it is never destroyed; `center_on_cursor_monitor()` positions it on the active monitor before showing
- Onboarding window: transparent, decorations off, skip-taskbar. Recover interrupted/empty-dictionary startup; clear `first_launch` only after validated, persisted completion and destroy the native window.
- `reposition_window()` uses `window.cursor_position()` + `window.available_monitors()` to position the card on whichever monitor holds the cursor
- `center_on_cursor_monitor()` centers a window on the monitor under the cursor (used for settings window)

**IPC commands** (`commands.rs`):
- `mark_learned(index, dictionary_id)` validates the visible card's source and rejects duplicates/stale indices, persists progress, emits statistics and wakes pending completion.
- `remove_learned` / `clear_learned` — persist `learned.json` + `stats.json`, reset `all_learned`, emit `stats-updated`
- `delete_dictionary` — removes learned data for the deleted dict; falls back to first available dictionary (not hardcoded)
- `get_all_dict_ids` / `load_dictionary_by_id` — pub helpers used by timer for auto-switch
- `save_learned_to_disk` / `save_stats_to_disk` — pub helpers shared with timer
- `complete_onboarding` validates/persists selection before destroying the native window; import uses a parented picker outside the UI thread.

**Single-instance lock**: Windows named mutex — second launch exits immediately.

**OS language detection** (`i18n.rs`): `sys-locale::get_locale()` maps `ru-*` → `ru`, everything else → `en`.

**Fullscreen detection** (`timer.rs`): Windows-only via `winapi` — `GetForegroundWindow` + `GetMonitorInfoW`; always returns `false` on other platforms.

### Frontend (src/)

**`src/tauri-api.js`** — IPC bridge (`window.api`). Key entries: `markLearned(index)`, `onMarkKnown(cb)`.

**`src/windows/card/card.js`** — tracks `currentWordIndex` and `isCardVisible` from `show-word`/`hide-word` events; `onMarkKnown` listener guards with `isCardVisible && currentWordIndex !== null` before calling `markLearned`. Card elements use IDs `#term` and `#definition`.

**`src/windows/settings/settings.js`** — handles `onStatsUpdated`, `onDictAutoSwitched`, `onLocaleUpdated`.

**`src/windows/onboarding/`** - HTML and `onboarding.js` offer three starter packs and "Import my own". Completion goes through `complete_onboarding`, never DOM `window.close()`. Pending operations prevent duplicate clicks; cancelled/failed imports restore usable controls.

Settings merge pending slider patches and serialize persistence. Discrete changes
flush pending sliders; failed saves do not show success. Dictionary selection
discards stale async responses. Card state is invalidated on dictionary switches.

**`src/modules/`** — JS stubs from the Electron era; not part of active code paths.

### Persistence

All data written to OS AppData directory via `tauri::path`:
- `data/settings.json`, `data/stats.json`, `data/learned.json`
- `dictionaries/` — user-imported dictionaries

### Key Design Notes

- `withGlobalTauri: true` in `tauri.conf.json` — Tauri API available as `window.__TAURI__` without a bundler.
- No frontend build step — raw HTML/CSS/JS served directly (`frontendDist: "../src"`).
- CSP disabled (`"csp": null`).
- `show-word` includes the source dictionary ID with term/definition/index; mark-known passes the source back for backend validation.
- Default `active_dictionary` is `""` (empty) — timer sleeps until a dictionary is chosen.
- Completion wakes the timer immediately, including when the last card was shown manually.
- Use an isolated, backed-up profile for onboarding tests; never delete a real user's settings/progress to prepare QA.

### Windows Installation

`src-tauri/src/autostart.rs` selects Windows StartupTask APIs when package identity
is present; MSI/NSIS keep the existing autostart plugin. One process-lifetime MTA
worker owns WinRT calls. `msixStartupInitialized` migrates a saved preference once,
then reads actual Windows state, respecting external enable/disable and policy.

`build.rs` compiles `windows/autostart_cleanup.rs` into a native MSI action DLL.
The hidden WiX feature runs its commit action only after successful full removal,
not upgrade or rollback. Delete the initiating user's Run value only when its
complete executable path belongs to the removed installation; never delete data
or another installation's startup entry.

`scripts/build-msix.cjs` is an opt-in clean packaging path with an exact executable
hash and eleven-file payload allowlist. See `packaging/msix/README.md`. Store MSIX
and GitHub native installers are distinct artifacts and need their own checks.

## Unit Tests

Rust unit tests live as `#[cfg(test)]` blocks co-located in each module (no separate test files).
Run with `cd src-tauri && cargo test --locked`. Windows-specific tests include
autostart and native MSI cleanup behavior; counts differ on other platforms.

| Module | Coverage |
|--------|----------|
| `word_engine.rs` | Random selection returns dict words; learned words never returned; `None` when all learned; `None` for empty dict; words available again after `set_learned(vec![])` |
| `dictionary_loader.rs` | Valid CSV with `term`/`definition`; legacy CSV with `word`/`translation`; `front`/`back` CSV; valid JSON with `term`/`definition`; legacy JSON with `word`/`translation`; unsupported extension error; single-column CSV error; Cyrillic characters preserved |
| `stats.rs` | `record_shown` increments today's count; `record_learned` increments today's count; `decrement_learned` does not go below zero; streak = 1 for single day; streak counts consecutive days; `get_last_7_days` returns exactly 7 entries |
| `i18n.rs` | `set_locale("en"/"ru")` switches locale; `t()` returns correct string; `t()` returns raw template string with `{placeholder}` intact (substitution is done in JS); `current_locale` reflects `set_locale`; unknown key returns the key itself as fallback |
| `settings.rs` | Default values match expected constants; round-trip JSON serialization/deserialization is lossless |

Additional Rust coverage includes card layout, completion timers, onboarding
recovery, persistence failures, and Windows startup/cleanup. `npm test` runs 29
Node.js regressions for renderer races, settings writes, onboarding and packaging
contracts. `src/modules/` remains legacy Electron code, not the active backend.

## CI/CD

**Repository**: https://github.com/DemonazGH/LazyWords-Tauri

**Workflow**: [`.github/workflows/ci.yml`](.github/workflows/ci.yml) — runs on every push and pull_request to `main`.

- Runs on `ubuntu-latest`
- Installs stable Rust via `dtolnay/rust-toolchain@stable`
- Installs required Linux GTK/WebKit2 system dependencies for Tauri
- Runs Rust unit tests in `src-tauri/`; Windows-only coverage runs in the release workflow.

**Release workflow**: [`.github/workflows/release.yml`](.github/workflows/release.yml) - runs on `codex/release-*` branch pushes and `v*` tags.

- Runs on `windows-latest`
- Validates app/tag versions, runs JS regressions and locked release-profile Rust tests, then builds Windows x64 MSI/NSIS.
- Retains both installers and `SHA256SUMS.txt` as a workflow artifact.
- Tag builds create a draft GitHub release; publish only after checking the exact artifacts. The workflow does not submit anything to Microsoft Store.

## Known Bugs

No currently reproduced open application bug is recorded for the R3 source.
This is not a claim of exhaustive platform coverage. A missing-WebView2 fault
injection is not proof of a clean machine without the runtime; previous installer
tests do not certify a newly rebuilt package. Review exact artifacts before release.

## Fixed Bugs

- Startup shortcut conflicts no longer abort the app.
- Long cards wrap and resize within the current monitor's work area.
- Imported selection rejects stale responses; autosave merges and serializes changes.
- Mark-known cannot apply a card from a previously selected dictionary.
- Final-card completion wakes immediately and stale notifications cannot hide a new card.
- Interrupted onboarding is recoverable; native completion closes its actual window and import errors remain retryable.
- MSI removal cleans only its owned startup entry, preserving another installation and user data.
- MSIX startup uses StartupTask instead of Run; persistent WinRT worker lifetime avoids activation-cache teardown crashes.
- Explicit MSIX payload mapping excludes user profiles, imported content, progress and WebView caches.

## Version History

| Version | Changes |
|---------|---------|
| **v1.0.2** | R3 reliability fixes, clean MSIX packaging, Windows StartupTask support, ownership-safe MSI cleanup, renderer/onboarding regressions and checked native release builds; see CHANGELOG.md |
| **v0.2.0** | System tray icon with localized menu (Pause/Resume, Next, Dictionary submenu, Settings, Quit); normal/dimmed icon states; CD workflow for auto-building installers on git tag |
| **v0.2.1** | Bundle `icons/icon.png`, `dictionaries/*`, `locales/*` as resources (fixes tray icon, bundled dicts, locale files missing in production); show first card immediately after onboarding; add missing `card.allDictsLearned` / `card.restoreInSettings` locale keys |
| **v0.2.2** | Wire `tauri-plugin-autostart` enable/disable; add `skip_taskbar(true)` to onboarding window; add `manual_trigger` Notify to prevent double card on `Ctrl+Shift+N`; record `shown` stat on manual next (shortcut + tray) |
| **v0.2.3** | Pre-create settings window hidden at startup (fixes deadlock on first `Ctrl+Shift+W`); fire `manual_trigger` after onboarding first-card (fixes double card after onboarding) |
| **v0.2.4** | Intercept `CloseRequested` on settings window with `prevent_close()` + `hide()` (fixes window only opening once); add `center_on_cursor_monitor()` to open settings on active monitor |

## Planned Features

- **Simplified spaced repetition (two levels)** — cards cycle through "new" and "seen" buckets with different intervals, without a full SRS algorithm
- **Transcription support** — display phonetic transcription (IPA) alongside term/definition on cards *(deferred)*
- **Export/import progress** — allow users to export and import their learned.json / stats.json for backup or device transfer *(deferred)*
