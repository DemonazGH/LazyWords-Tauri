use std::{collections::BTreeMap, sync::Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Pause,
    Next,
    Learned,
    Settings,
    QuickAdd,
}

const ACTIONS: [Action; 5] = [
    Action::Pause,
    Action::Next,
    Action::Learned,
    Action::Settings,
    Action::QuickAdd,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    enabled: bool,
    keys: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Hotkeys {
    #[serde(flatten)]
    known: BTreeMap<Action, Binding>,
    // Round-trip newer actions on downgrade, but never register or dispatch them.
    #[serde(flatten)]
    future: BTreeMap<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for Hotkeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Key {
            Known(Action),
            Future(String),
        }

        struct HotkeyMap;
        impl<'de> serde::de::Visitor<'de> for HotkeyMap {
            type Value = Hotkeys;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a map of shortcut action names to bindings")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Hotkeys, M::Error> {
                let mut prefs = Hotkeys::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Known(action) => {
                            prefs.known.insert(action, map.next_value()?);
                        }
                        Key::Future(name) => {
                            prefs.future.insert(name, map.next_value()?);
                        }
                    }
                }
                Ok(prefs)
            }
        }
        deserializer.deserialize_map(HotkeyMap)
    }
}

impl Action {
    fn default_binding(self) -> Binding {
        let keys = match self {
            Self::Pause => "Ctrl+Shift+P",
            Self::Next => "Ctrl+Shift+N",
            Self::Learned => "Ctrl+Shift+K",
            Self::Settings => "Ctrl+Shift+W",
            Self::QuickAdd => "",
        };
        Binding {
            enabled: !keys.is_empty(),
            keys: keys.into(),
        }
    }
}

pub fn defaults() -> Hotkeys {
    Hotkeys {
        known: ACTIONS
            .into_iter()
            .map(|a| (a, a.default_binding()))
            .collect(),
        future: BTreeMap::new(),
    }
}

fn binding(prefs: &Hotkeys, action: Action) -> Binding {
    prefs
        .known
        .get(&action)
        .cloned()
        .unwrap_or_else(|| action.default_binding())
}

const INVALID: &str = "hotkeys.invalid";
const DUPLICATE: &str = "hotkeys.duplicate";
const UNAVAILABLE: &str = "hotkeys.unavailable";
const STORAGE: &str = "hotkeys.saveError";
const RECOVERY: &str = "hotkeys.restart";

fn normalize(mut b: Binding) -> Result<(Binding, Option<Shortcut>), &'static str> {
    if b.keys.trim().is_empty() && !b.enabled {
        b.keys.clear();
        return Ok((b, None));
    }
    let upper = b.keys.to_ascii_uppercase();
    let mut parts: Vec<_> = upper.split('+').map(str::trim).collect();
    let key = parts.pop().ok_or(INVALID)?;
    let function = key
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n) && n != 12 && key == format!("F{n}"));
    if !(function || key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric()) {
        return Err(INVALID);
    }
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for part in parts {
        let flag = match part {
            "CTRL" | "CONTROL" => &mut ctrl,
            "ALT" => &mut alt,
            "SHIFT" => &mut shift,
            _ => return Err(INVALID),
        };
        if *flag {
            return Err(INVALID);
        }
        *flag = true;
    }
    if !ctrl && !alt || alt && !ctrl && !shift && key == "F4" {
        return Err(INVALID);
    }
    b.keys = format!(
        "{}{}{}{key}",
        if ctrl { "Ctrl+" } else { "" },
        if alt { "Alt+" } else { "" },
        if shift { "Shift+" } else { "" }
    );
    let shortcut = b.keys.parse::<Shortcut>().map_err(|_| INVALID)?;
    let enabled = b.enabled;
    Ok((b, enabled.then_some(shortcut)))
}

#[derive(Default)]
pub struct Runtime {
    edits: Mutex<()>,
    live: Mutex<BTreeMap<Action, Shortcut>>,
}

trait Reservations {
    fn acquire(&self, key: Shortcut) -> bool;
    fn release(&self, key: Shortcut) -> bool;
}

impl Reservations for AppHandle {
    fn acquire(&self, key: Shortcut) -> bool {
        self.global_shortcut().register(key).is_ok()
    }
    fn release(&self, key: Shortcut) -> bool {
        self.global_shortcut().unregister(key).is_ok()
    }
}

pub fn initialize(app: &AppHandle) -> Vec<String> {
    let prefs = app
        .state::<crate::state::AppState>()
        .settings
        .lock()
        .unwrap()
        .hotkeys
        .clone();
    let (live, failures) =
        reserve_startup(&prefs, app, app.get_webview_window("quick-add").is_some());
    *app.state::<Runtime>().live.lock().unwrap() = live;
    failures
}

fn reserve_startup(
    prefs: &Hotkeys,
    os: &impl Reservations,
    quick_add_available: bool,
) -> (BTreeMap<Action, Shortcut>, Vec<String>) {
    let mut live = BTreeMap::new();
    let mut failures = vec![];
    for action in ACTIONS {
        let b = binding(prefs, action);
        if !b.enabled {
            continue;
        }
        if let Ok((_, Some(key))) = normalize(b.clone()) {
            if !live.values().any(|v| *v == key)
                && (action != Action::QuickAdd || quick_add_available)
                && os.acquire(key)
            {
                live.insert(action, key);
                continue;
            }
        }
        failures.push(b.keys);
    }
    (live, failures)
}

pub fn handle(app: &AppHandle, key: &Shortcut, event: ShortcutEvent) {
    if event.state != ShortcutState::Pressed {
        return;
    }
    let action = app
        .state::<Runtime>()
        .live
        .lock()
        .unwrap()
        .iter()
        .find_map(|(&a, k)| (key == k).then_some(a));
    let Some(action) = action else {
        return;
    };
    if let Some(window) = app.get_webview_window("onboarding") {
        let _ = window.close();
    }
    match action {
        Action::QuickAdd => crate::quick_add::open(app),
        Action::Learned => {
            let _ = app.emit("mark-known-shortcut", ());
        }
        Action::Next => crate::tray::next_card(app),
        Action::Pause => {
            let state = app.state::<crate::state::AppState>();
            {
                let mut paused = state.is_paused.lock().unwrap();
                *paused = !*paused;
            }
            crate::tray::refresh_tray(app);
        }
        Action::Settings => {
            if let Some(w) = app.get_webview_window("settings") {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.hide();
                    let _ = w.set_skip_taskbar(true);
                } else {
                    crate::center_on_cursor_monitor(&w);
                    let _ = w.set_skip_taskbar(false);
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        }
    }
}

#[derive(Serialize)]
pub struct Row {
    #[serde(flatten)]
    binding: Binding,
    active: bool,
}

#[tauri::command]
pub fn get_hotkeys(window: WebviewWindow) -> Result<BTreeMap<Action, Row>, &'static str> {
    if window.label() != "settings" {
        return Err("Settings window required");
    }
    let app = window.app_handle();
    let state = app.state::<crate::state::AppState>();
    let prefs = state.settings.lock().unwrap();
    let rt = app.state::<Runtime>();
    let live = rt.live.lock().unwrap();
    Ok(ACTIONS
        .into_iter()
        .map(|a| {
            (
                a,
                Row {
                    binding: binding(&prefs.hotkeys, a),
                    active: live.contains_key(&a),
                },
            )
        })
        .collect())
}

fn requested(
    prefs: &Hotkeys,
    action: Action,
    input: Binding,
) -> Result<(Binding, Option<Shortcut>), &'static str> {
    let (b, key) = if !input.enabled {
        // A typo in the text box must never prevent turning a shortcut off.
        normalize(input).unwrap_or_else(|_| {
            let mut previous = binding(prefs, action);
            previous.enabled = false;
            normalize(previous).unwrap_or((
                Binding {
                    enabled: false,
                    keys: String::new(),
                },
                None,
            ))
        })
    } else {
        normalize(input)?
    };
    if let Some(key) = key {
        if ACTIONS
            .iter()
            .copied()
            .filter(|a| *a != action)
            .any(|a| normalize(binding(prefs, a)).is_ok_and(|(_, k)| k == Some(key)))
        {
            return Err(DUPLICATE);
        }
    }
    Ok((b, key))
}

// Reserve before saving; keep the old reservation until the atomic settings write succeeds.
// The event handler only dispatches committed routes, never provisional reservations.
fn transition(
    os: &impl Reservations,
    old: Option<Shortcut>,
    new: Option<Shortcut>,
    mut commit: impl FnMut(bool) -> bool,
) -> Result<(), &'static str> {
    if old == new {
        return commit(true).then_some(()).ok_or(STORAGE);
    }
    if new.is_some_and(|key| !os.acquire(key)) {
        return Err(UNAVAILABLE);
    }
    if !commit(true) {
        return Err(if new.is_some_and(|key| !os.release(key)) {
            RECOVERY
        } else {
            STORAGE
        });
    }
    if old.is_some_and(|key| !os.release(key)) {
        if !commit(false) || new.is_some_and(|key| !os.release(key)) {
            return Err(RECOVERY);
        }
        return Err(UNAVAILABLE);
    }
    Ok(())
}

#[tauri::command]
pub fn set_hotkey(
    window: WebviewWindow,
    action: Action,
    input: Binding,
) -> Result<Row, &'static str> {
    if window.label() != "settings" {
        return Err("Settings window required");
    }
    let app = window.app_handle();
    let state = app.state::<crate::state::AppState>();
    let rt = app.state::<Runtime>();
    let _edit = rt.edits.lock().unwrap();
    let prefs = state.settings.lock().unwrap().hotkeys.clone();
    let (next, new_key) = requested(&prefs, action, input)?;
    if new_key.is_some()
        && action == Action::QuickAdd
        && app.get_webview_window("quick-add").is_none()
    {
        return Err(UNAVAILABLE);
    }
    let old_key = rt.live.lock().unwrap().get(&action).copied();
    let path = app
        .path()
        .app_data_dir()
        .map_err(|_| STORAGE)?
        .join("data/settings.json");
    transition(app, old_key, new_key, |forward| {
        let mut current = state.settings.lock().unwrap();
        let mut patched = current.clone();
        let (value, key) = if forward {
            (next.clone(), new_key)
        } else {
            (binding(&prefs, action), old_key)
        };
        patched.hotkeys.known.insert(action, value);
        if let Err(error) = crate::settings::try_save_settings(&path, &patched) {
            eprintln!("Could not save hotkey: {error}");
            return false;
        }
        *current = patched;
        let mut live = rt.live.lock().unwrap();
        if let Some(key) = key {
            live.insert(action, key);
        } else {
            live.remove(&action);
        }
        true
    })?;
    Ok(Row {
        binding: next,
        active: new_key.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::HashSet,
    };

    fn input(keys: &str, enabled: bool) -> Binding {
        Binding {
            keys: keys.to_owned(),
            enabled,
        }
    }

    fn key(keys: &str) -> Shortcut {
        keys.parse().unwrap()
    }

    #[derive(Default)]
    struct FakeOs {
        held: RefCell<HashSet<Shortcut>>,
        stuck: Cell<Option<Shortcut>>,
    }

    impl Reservations for FakeOs {
        fn acquire(&self, key: Shortcut) -> bool {
            self.held.borrow_mut().insert(key)
        }
        fn release(&self, key: Shortcut) -> bool {
            self.stuck.get() != Some(key) && self.held.borrow_mut().remove(&key)
        }
    }

    #[test]
    fn upgrade_keeps_four_shipped_shortcuts_and_quick_add_starts_unbound() {
        let before = r#"{"firstLaunch":false,"language":"ru","activeDictionary":"my-deck","autoStart":false,"position":"center","fontSize":30,"showDuration":7,"interval":4,"fadeDuration":0.5}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, before).unwrap();
        let mut prefs = crate::settings::load_settings(&path).unwrap();
        assert!(!prefs.needs_onboarding());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(prefs.hotkeys, defaults());
        let os = FakeOs::default();
        let (live, failures) = reserve_startup(&prefs.hotkeys, &os, true);
        assert!(failures.is_empty());
        assert_eq!(
            live,
            BTreeMap::from([
                (Action::Pause, key("Ctrl+Shift+P")),
                (Action::Next, key("Ctrl+Shift+N")),
                (Action::Learned, key("Ctrl+Shift+K")),
                (Action::Settings, key("Ctrl+Shift+W")),
            ])
        );
        assert_eq!(prefs.hotkeys.known[&Action::QuickAdd], input("", false));
        prefs
            .hotkeys
            .known
            .insert(Action::Next, input("Alt+F8", false));
        prefs
            .hotkeys
            .known
            .insert(Action::QuickAdd, input("Ctrl+Alt+Q", true));
        crate::settings::try_save_settings(&path, &prefs).unwrap();
        let restored = crate::settings::load_settings(&path).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&prefs).unwrap()
        );
        let (restarted, failures) = reserve_startup(&restored.hotkeys, &FakeOs::default(), true);
        assert!(failures.is_empty());
        assert!(!restarted.contains_key(&Action::Next));
        assert_eq!(restarted[&Action::QuickAdd], key("Ctrl+Alt+Q"));
        for a in [Action::Learned, Action::Pause, Action::Settings] {
            assert_eq!(live[&a], restarted[&a]);
        }
    }

    #[test]
    fn future_hotkeys_are_not_registered_and_survive_settings_saves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut file = serde_json::to_value(crate::settings::Settings::default()).unwrap();
        file["firstLaunch"] = false.into();
        file["activeDictionary"] = "custom-deck".into();
        file["hotkeys"] = serde_json::json!({
            "next": {"enabled": true, "keys": "Alt+F7"},
            "settings": {"enabled": false, "keys": "Ctrl+Shift+W"},
            "futureAction": {"enabled": true, "keys": "Alt+F8", "options": [1, 2]},
            "anotherFutureAction": ["a different binding format"]
        });
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let mut prefs = crate::settings::load_settings(&path).unwrap();
        assert!(!prefs.needs_onboarding());
        let os = FakeOs::default();
        let (live, failures) = reserve_startup(&prefs.hotkeys, &os, true);
        assert!(failures.is_empty());
        assert_eq!(live.len(), 3);
        assert_eq!(live[&Action::Next], key("Alt+F7"));
        assert!(!live.contains_key(&Action::Settings));
        assert!(!os.held.borrow().contains(&key("Alt+F8")));

        prefs.font_size = 30.0;
        prefs
            .hotkeys
            .known
            .insert(Action::Next, input("Alt+F9", true));
        crate::settings::try_save_settings(&path, &prefs).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        file["hotkeys"]["next"]["keys"] = "Alt+F9".into();
        assert_eq!(saved["hotkeys"], file["hotkeys"]);
        let restored = crate::settings::load_settings(&path).unwrap();
        assert_eq!(restored.hotkeys, prefs.hotkeys);
        assert_eq!(restored.active_dictionary, "custom-deck");
    }

    #[test]
    fn invalid_known_hotkey_is_still_a_settings_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut file = serde_json::to_value(crate::settings::Settings::default()).unwrap();
        file["hotkeys"] = serde_json::json!({
            "futureAction": null, "quickAdd": {"enabled": "yes", "keys": "Alt+F8"}
        });
        let original = serde_json::to_vec(&file).unwrap();
        std::fs::write(&path, &original).unwrap();
        let error = crate::settings::load_settings(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn each_action_can_be_the_only_active_action_or_all_can_be_off() {
        let mut prefs = defaults();
        prefs.known.values_mut().for_each(|b| b.enabled = false);
        assert!(reserve_startup(&prefs, &FakeOs::default(), true)
            .0
            .is_empty());
        for a in ACTIONS {
            prefs
                .known
                .insert(a, requested(&prefs, a, input("alt+F9", true)).unwrap().0);
            let (live, failures) = reserve_startup(&prefs, &FakeOs::default(), true);
            assert!(failures.is_empty());
            assert_eq!(live, BTreeMap::from([(a, key("Alt+F9"))]));
            prefs.known.insert(a, input("Alt+F9", false));
        }
    }

    #[test]
    fn normalizes_modifiers_and_rejects_invalid_or_reserved_combinations() {
        for (raw, expected) in [
            (" shift + alt + control + z ", "Ctrl+Alt+Shift+Z"),
            ("alt+0", "Alt+0"),
            ("ctrl+f24", "Ctrl+F24"),
            ("ctrl+alt+a", "Ctrl+Alt+A"),
            ("ctrl+c", "Ctrl+C"),
            ("ctrl+v", "Ctrl+V"),
            ("alt+f", "Alt+F"),
        ] {
            assert_eq!(
                normalize(input(raw, true)),
                Ok((input(expected, true), Some(key(expected))))
            );
            assert_eq!(
                normalize(input(raw, false)),
                Ok((input(expected, false), None))
            );
        }
        assert_eq!(normalize(input(" ", false)), Ok((input("", false), None)));
        for raw in [
            "",
            "B",
            "Shift+N",
            "Ctrl++N",
            "Ctrl+Ctrl+N",
            "Win+L",
            "Alt+F4",
            "Ctrl+F12",
            "Ctrl+F25",
            "Ctrl+F02",
            "Ctrl+Alt+Delete",
            "Ctrl+Shift",
            "Ctrl+AB",
        ] {
            assert_eq!(normalize(input(raw, true)), Err(INVALID), "{raw}");
        }
    }

    #[test]
    fn rejects_duplicate_enabled_bindings_but_disabled_keys_can_be_reused() {
        let mut prefs = defaults();
        let candidate = input("shift+control+n", true);
        assert_eq!(
            requested(&prefs, Action::Pause, candidate.clone()),
            Err(DUPLICATE)
        );
        assert!(requested(&prefs, Action::Next, candidate.clone()).is_ok());
        prefs
            .known
            .insert(Action::Next, input("Ctrl+Shift+N", false));
        assert!(requested(&prefs, Action::Pause, candidate).is_ok());
    }

    #[test]
    fn disabling_with_an_invalid_draft_keeps_the_previous_valid_keys() {
        let mut prefs = defaults();
        assert_eq!(
            requested(&prefs, Action::Pause, input("Ctrl+", false)),
            Ok((input("Ctrl+Shift+P", false), None))
        );
        prefs.known.insert(Action::Pause, input("typo", true));
        assert_eq!(
            requested(&prefs, Action::Pause, input("typo", false)),
            Ok((input("", false), None))
        );
    }

    #[test]
    fn startup_conflict_leaves_preferences_intact_and_can_be_retried_without_restart() {
        let os = FakeOs::default();
        let mut prefs = defaults();
        prefs.known.insert(Action::QuickAdd, input("Alt+Q", true));
        prefs.known.remove(&Action::Settings);
        let blocked = key("Ctrl+Shift+P");
        os.acquire(blocked);
        let (live, failures) = reserve_startup(&prefs, &os, false);
        assert_eq!(failures, ["Ctrl+Shift+P", "Alt+Q"]);
        assert!(!live.contains_key(&Action::QuickAdd));
        assert_eq!(live[&Action::Settings], key("Ctrl+Shift+W"));
        assert!(!live.contains_key(&Action::Pause));
        assert_eq!(prefs.known[&Action::Pause], input("Ctrl+Shift+P", true));
        assert_eq!(
            transition(&os, None, Some(blocked), |_| panic!("must not persist")),
            Err(UNAVAILABLE)
        );
        os.release(blocked);
        assert_eq!(
            transition(&os, None, Some(blocked), |forward| forward),
            Ok(())
        );
    }

    #[test]
    fn rebind_holds_both_keys_until_commit_and_disable_only_releases_its_key() {
        let os = FakeOs::default();
        let (old, new, other) = (key("Alt+P"), key("Alt+Q"), key("Alt+W"));
        os.acquire(old);
        os.acquire(other);
        transition(&os, Some(old), Some(new), |forward| {
            assert_eq!(*os.held.borrow(), HashSet::from([old, new, other]));
            forward
        })
        .unwrap();
        assert_eq!(*os.held.borrow(), HashSet::from([other, new]));
        transition(&os, Some(new), Some(new), |f| f).unwrap();
        transition(&os, Some(new), None, |f| f).unwrap();
        assert_eq!(*os.held.borrow(), HashSet::from([other]));
    }

    #[test]
    fn conflict_and_failed_write_do_not_lose_the_old_shortcut() {
        let os = FakeOs::default();
        let (old, new) = (key("Alt+P"), key("Alt+Q"));
        os.acquire(old);
        os.acquire(new);
        assert_eq!(
            transition(&os, Some(old), Some(new), |_| panic!("must not persist")),
            Err(UNAVAILABLE)
        );
        assert_eq!(*os.held.borrow(), HashSet::from([old, new]));
        os.release(new);
        assert_eq!(
            transition(&os, Some(old), Some(new), |_| false),
            Err(STORAGE)
        );
        assert_eq!(transition(&os, Some(old), None, |_| false), Err(STORAGE));
        assert_eq!(*os.held.borrow(), HashSet::from([old]));
    }

    #[test]
    fn failed_release_restores_old_route_or_reports_recovery_required() {
        let os = FakeOs::default();
        let (old, new) = (key("Alt+P"), key("Alt+Q"));
        os.acquire(old);
        os.stuck.set(Some(old));
        let mut writes = vec![];
        assert_eq!(
            transition(&os, Some(old), Some(new), |f| {
                writes.push(f);
                true
            }),
            Err(UNAVAILABLE)
        );
        assert_eq!(writes, [true, false]);
        assert_eq!(*os.held.borrow(), HashSet::from([old]));
        assert_eq!(transition(&os, Some(old), Some(new), |f| f), Err(RECOVERY));
    }
}
