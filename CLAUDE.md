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

No lint scripts are configured.

## Architecture

LazyWordsTauri is a Tauri v2 desktop app (Rust backend + Vanilla JS frontend) that periodically displays floating vocabulary word cards as an always-on-top overlay.

### Backend (src-tauri/src/)

**AppState** (`state.rs`) is the central Mutex-protected state shared across all Rust modules:
- `settings` – user config (intervals, font size, position, language, autostart, first_launch)
- `word_engine` – holds loaded dictionary + `HashSet<usize>` of learned word indices
- `stats_tracker` – daily/streak statistics
- `learned` – `HashMap<dict_name, Vec<usize>>` persisted across sessions
- `is_paused`, `is_card_visible`, `current_word`, `all_learned` – runtime state

**Timer loop** (`timer.rs`) — the core async loop:
- Respects `all_learned` flag (sleeps 30 s/cycle when set instead of running)
- Checks fullscreen app detection on Windows (skips word if foreground app is fullscreen)
- Records `shown` stat + emits `stats-updated` on every word shown
- When `get_random_word()` returns `None`: sets `all_learned = true`, emits `show-all-learned`, then scans all dictionaries for one with remaining words — if found emits `show-switched-dict` + `dict-auto-switched` and resets; if not found emits `show-all-dicts-learned`

**Global shortcuts** (`lib.rs`):
- `Ctrl+Shift+W` – toggle settings window (creates with `shadow(false)` if new)
- `Ctrl+Shift+P` – pause/resume
- `Ctrl+Shift+N` – implicit unpause + show next word immediately; adds `index` to payload
- `Ctrl+Shift+K` – emits `mark-known-shortcut`; card.js handles it
- All four shortcuts dismiss the onboarding window if it's open

**Window management** (`lib.rs`):
- Card window: `500×110px`, transparent, always-on-top, skip-taskbar, `shadow(false)`, ignore cursor events
- Settings window: created on-demand, `shadow(false)`
- Onboarding window: `380×270px`, transparent, decorations off, shown only when `first_launch = true`; flag cleared to `false` immediately on creation
- `reposition_window()` uses `window.cursor_position()` + `window.available_monitors()` to position the card on whichever monitor holds the cursor

**IPC commands** (`commands.rs`):
- `mark_learned(index)` — new; adds to learned, updates engine, records stat, saves both files, emits `stats-updated`, sets `all_learned` if no words remain
- `remove_learned` / `clear_learned` — now persist `learned.json` + `stats.json`, reset `all_learned`, emit `stats-updated`
- `delete_dictionary` — now also removes learned data for the deleted dict from `learned.json`
- `get_all_dict_ids` / `load_dictionary_by_id` — pub helpers used by timer for auto-switch
- `save_learned_to_disk` / `save_stats_to_disk` — pub helpers shared with timer

**Single-instance lock**: `tauri-plugin-single-instance` — second launch focuses the existing instance.

**OS language detection** (`i18n.rs`): `sys-locale::get_locale()` maps `ru-*` → `ru`, everything else → `en`.

**Fullscreen detection** (`timer.rs`): Windows-only via `winapi` — `GetForegroundWindow` + `GetMonitorInfoW`; always returns `false` on other platforms.

### Frontend (src/)

**`src/tauri-api.js`** — IPC bridge (`window.api`). Key additions: `markLearned(index)`, `onMarkKnown(cb)`.

**`src/windows/card/card.js`** — tracks `currentWordIndex` and `isCardVisible` from `show-word`/`hide-word` events; `onMarkKnown` listener guards with `isCardVisible && currentWordIndex !== null` before calling `markLearned`.

**`src/windows/settings/settings.js`** — unchanged; already handles `onStatsUpdated`, `onDictAutoSwitched`, `onLocaleUpdated`.

**`src/windows/onboarding/onboarding.html`** — calls `api.getLocale()` on init (no event-listener dependency).

**`src/modules/`** — JS stubs from the Electron era; not part of active code paths.

### Persistence

All data written to OS AppData directory via `tauri::path`:
- `data/settings.json`, `data/stats.json`, `data/learned.json`
- `dictionaries/` — user-imported dictionaries

### Key Design Notes

- `withGlobalTauri: true` in `tauri.conf.json` — Tauri API available as `window.__TAURI__` without a bundler.
- No frontend build step — raw HTML/CSS/JS served directly (`frontendDist: "../src"`).
- CSP disabled (`"csp": null`).
- `show-word` payload includes `index` field so `card.js` can pass it back to `mark_learned`.
- After marking a word as learned, the `all_learned` flag is set by `mark_learned`; the timer detects it on the next wake cycle and runs the auto-switch flow.
- To test onboarding: set `"firstLaunch": true` in `%APPDATA%\tauri-app\data\settings.json`.

## Unit Tests

Rust unit tests live as `#[cfg(test)]` blocks co-located in each module (no separate test files).
Run with `cd src-tauri && cargo test` (24 tests, ~0.01 s).

| Module | Coverage |
|--------|----------|
| `word_engine.rs` | Random selection returns dict words; learned words never returned; `None` when all learned; `None` for empty dict; words available again after `set_learned(vec![])` |
| `dictionary_loader.rs` | Valid CSV parse; valid JSON parse; unsupported extension error; missing `word` column error; Cyrillic characters preserved |
| `stats.rs` | `record_shown` increments today's count; `record_learned` increments today's count; `decrement_learned` does not go below zero; streak = 1 for single day; streak counts consecutive days; `get_last_7_days` returns exactly 7 entries |
| `i18n.rs` | `set_locale("en"/"ru")` switches locale; `t()` returns correct string; `t()` returns raw template string with `{placeholder}` intact (substitution is done in JS); `current_locale` reflects `set_locale`; unknown key returns the key itself as fallback |
| `settings.rs` | Default values match expected constants; round-trip JSON serialization/deserialization is lossless |

**No JS tests.** `src/modules/` contains legacy Electron stubs not used by the Tauri app; `src/windows/` and `src/tauri-api.js` are thin UI/IPC glue with no testable logic independent of the Tauri runtime.

## CI/CD

**Repository**: https://github.com/DemonazGH/LazyWords-Tauri

**Workflow**: [`.github/workflows/ci.yml`](.github/workflows/ci.yml) — runs on every push and pull_request to `main`.

- Runs on `ubuntu-latest`
- Installs stable Rust via `dtolnay/rust-toolchain@stable`
- Installs required Linux GTK/WebKit2 system dependencies for Tauri
- Runs `cargo test` in `src-tauri/` — executes all 24 unit tests across 5 modules (`word_engine`, `dictionary_loader`, `stats`, `i18n`, `settings`)
