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
| `en-definitions.csv` | ~50 common English words with English definitions |
| `en-irregular-verbs.csv` | ~50 common English irregular verbs (past / past participle) |

### Backend (src-tauri/src/)

**AppState** (`state.rs`) is the central Mutex-protected state shared across all Rust modules:
- `settings` – user config (intervals, font size, position, language, autostart, first_launch)
- `word_engine` – holds loaded dictionary + `HashSet<usize>` of learned card indices
- `stats_tracker` – daily/streak statistics
- `learned` – `HashMap<dict_name, Vec<usize>>` persisted across sessions
- `is_paused`, `is_card_visible`, `current_word`, `all_learned` – runtime state

**Timer loop** (`timer.rs`) — the core async loop:
- Sleeps 5 s/cycle when `active_dictionary` is empty (no dictionary selected yet)
- Respects `all_learned` flag (sleeps 30 s/cycle when set instead of running)
- Checks fullscreen app detection on Windows (skips card if foreground app is fullscreen)
- Records `shown` stat + emits `stats-updated` on every card shown
- When `get_random_word()` returns `None`: sets `all_learned = true`, emits `show-all-learned`, then scans all dictionaries for one with remaining cards — if found emits `show-switched-dict` + `dict-auto-switched` and resets; if not found emits `show-all-dicts-learned`

**Global shortcuts** (`lib.rs`):
- `Ctrl+Shift+W` – toggle settings window (creates with `shadow(false)` if new)
- `Ctrl+Shift+P` – pause/resume
- `Ctrl+Shift+N` – implicit unpause + show next card immediately; adds `index` to payload
- `Ctrl+Shift+K` – emits `mark-known-shortcut`; card.js handles it
- All four shortcuts dismiss the onboarding window if it's open

**Window management** (`lib.rs`):
- Card window: `500×110px`, transparent, always-on-top, skip-taskbar, `shadow(false)`, ignore cursor events
- Settings window: created on-demand, `shadow(false)`
- Onboarding window: `420×320px`, transparent, decorations off, shown only when `first_launch = true`; flag cleared to `false` immediately on creation
- `reposition_window()` uses `window.cursor_position()` + `window.available_monitors()` to position the card on whichever monitor holds the cursor

**IPC commands** (`commands.rs`):
- `mark_learned(index)` — adds to learned, updates engine, records stat, saves both files, emits `stats-updated`, sets `all_learned` if no cards remain
- `remove_learned` / `clear_learned` — persist `learned.json` + `stats.json`, reset `all_learned`, emit `stats-updated`
- `delete_dictionary` — removes learned data for the deleted dict; falls back to first available dictionary (not hardcoded)
- `get_all_dict_ids` / `load_dictionary_by_id` — pub helpers used by timer for auto-switch
- `save_learned_to_disk` / `save_stats_to_disk` — pub helpers shared with timer

**Single-instance lock**: Windows named mutex — second launch exits immediately.

**OS language detection** (`i18n.rs`): `sys-locale::get_locale()` maps `ru-*` → `ru`, everything else → `en`.

**Fullscreen detection** (`timer.rs`): Windows-only via `winapi` — `GetForegroundWindow` + `GetMonitorInfoW`; always returns `false` on other platforms.

### Frontend (src/)

**`src/tauri-api.js`** — IPC bridge (`window.api`). Key entries: `markLearned(index)`, `onMarkKnown(cb)`.

**`src/windows/card/card.js`** — tracks `currentWordIndex` and `isCardVisible` from `show-word`/`hide-word` events; `onMarkKnown` listener guards with `isCardVisible && currentWordIndex !== null` before calling `markLearned`. Card elements use IDs `#term` and `#definition`.

**`src/windows/settings/settings.js`** — handles `onStatsUpdated`, `onDictAutoSwitched`, `onLocaleUpdated`.

**`src/windows/onboarding/onboarding.html`** — dictionary selection screen shown on first launch. Offers 4 choices: three bundled starter packs (en-ru, en-definitions, en-irregular-verbs) and "Import my own". Calls `saveSettings({ activeDictionary })` then `window.close()` on selection. No close button — user must choose.

**`src/modules/`** — JS stubs from the Electron era; not part of active code paths.

### Persistence

All data written to OS AppData directory via `tauri::path`:
- `data/settings.json`, `data/stats.json`, `data/learned.json`
- `dictionaries/` — user-imported dictionaries

### Key Design Notes

- `withGlobalTauri: true` in `tauri.conf.json` — Tauri API available as `window.__TAURI__` without a bundler.
- No frontend build step — raw HTML/CSS/JS served directly (`frontendDist: "../src"`).
- CSP disabled (`"csp": null`).
- `show-word` payload: `{term, definition, index}` — `card.js` passes `index` back to `mark_learned`.
- Default `active_dictionary` is `""` (empty) — timer sleeps until a dictionary is chosen.
- After marking a card as learned, the `all_learned` flag is set by `mark_learned`; the timer detects it on the next wake cycle and runs the auto-switch flow.
- To test onboarding: delete `%APPDATA%\tauri-app\data\settings.json` or set `"firstLaunch": true` in it.

## Unit Tests

Rust unit tests live as `#[cfg(test)]` blocks co-located in each module (no separate test files).
Run with `cd src-tauri && cargo test` (27 tests, ~0.01 s).

| Module | Coverage |
|--------|----------|
| `word_engine.rs` | Random selection returns dict words; learned words never returned; `None` when all learned; `None` for empty dict; words available again after `set_learned(vec![])` |
| `dictionary_loader.rs` | Valid CSV with `term`/`definition`; legacy CSV with `word`/`translation`; `front`/`back` CSV; valid JSON with `term`/`definition`; legacy JSON with `word`/`translation`; unsupported extension error; single-column CSV error; Cyrillic characters preserved |
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
- Runs `cargo test` in `src-tauri/` — executes all 27 unit tests across 5 modules (`word_engine`, `dictionary_loader`, `stats`, `i18n`, `settings`)
