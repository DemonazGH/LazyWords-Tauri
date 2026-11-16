use crate::settings::Settings;
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

#[cfg(any(windows, test))]
pub const STARTUP_TASK_ID: &str = "LazyWordsStartup";

pub fn synchronize(app: &AppHandle, settings: &mut Settings) -> Result<(), String> {
    #[cfg(windows)]
    if packaged::has_identity()? {
        let request = initial_task_request(settings.msix_startup_initialized, settings.auto_start);
        match packaged::access_task(request) {
            Ok(actual) => {
                settings.auto_start = actual;
                settings.msix_startup_initialized = true;
                return Ok(());
            }
            Err(error) => {
                if let Ok(actual) = packaged::access_task(None) {
                    settings.auto_start = actual;
                    settings.msix_startup_initialized = true;
                }
                return Err(error);
            }
        }
    }
    set_enabled(app, settings.auto_start)
}

#[cfg(any(windows, test))]
fn initial_task_request(initialized: bool, preference: bool) -> Option<bool> {
    // Migrate old preferences once; Windows owns later external enable/disable choices.
    if initialized {
        None
    } else {
        Some(preference)
    }
}

pub fn is_enabled(app: &AppHandle) -> Result<bool, String> {
    #[cfg(windows)]
    if packaged::has_identity()? {
        return packaged::access_task(None);
    }
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

pub fn set_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    if packaged::has_identity()? {
        return packaged::access_task(Some(enabled)).map(|_| ());
    }
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|error| error.to_string())?;
    if launcher.is_enabled().map_err(|error| error.to_string())? != enabled {
        return Err("Autostart state did not change".into());
    }
    Ok(())
}

pub fn save_change(
    requested: Option<bool>,
    read: impl FnOnce() -> Result<bool, String>,
    mut apply: impl FnMut(bool) -> Result<(), String>,
    save: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let previous = match requested {
        Some(enabled) => {
            let previous = read()?;
            if previous != enabled {
                if let Err(error) = apply(enabled) {
                    return Err(rollback(error, apply(previous)));
                }
                Some(previous)
            } else {
                None
            }
        }
        None => None,
    };
    if let Err(error) = save() {
        return Err(match previous {
            Some(previous) => rollback(error, apply(previous)),
            None => error,
        });
    }
    Ok(())
}

fn rollback(error: String, result: Result<(), String>) -> String {
    match result {
        Ok(()) => error,
        Err(rollback) => format!("{error}; autostart rollback failed: {rollback}"),
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum TaskState {
    Disabled,
    DisabledByUser,
    DisabledByPolicy,
    Enabled,
    EnabledByPolicy,
}

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq)]
enum TaskAction {
    None,
    Enable,
    Disable,
}

#[cfg(any(windows, test))]
fn task_action(state: TaskState, enabled: bool) -> Result<TaskAction, String> {
    match (state, enabled) {
        (TaskState::DisabledByUser, true) => {
            Err("Enable LazyWords in Windows Startup apps first".into())
        }
        (TaskState::DisabledByPolicy, true) | (TaskState::EnabledByPolicy, false) => {
            Err("Autostart is controlled by Windows policy".into())
        }
        (TaskState::Disabled, true) => Ok(TaskAction::Enable),
        (TaskState::Enabled, false) => Ok(TaskAction::Disable),
        _ => Ok(TaskAction::None),
    }
}

#[cfg(windows)]
mod packaged {
    use super::{task_action, TaskAction, TaskState, STARTUP_TASK_ID};
    use std::sync::{mpsc, OnceLock};
    use windows::{
        core::{Error, HSTRING},
        ApplicationModel::{StartupTask, StartupTaskState},
        Win32::{
            Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, E_FAIL},
            Storage::Packaging::Appx::GetCurrentPackageFullName,
            System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
        },
    };

    pub fn has_identity() -> Result<bool, String> {
        let mut length = 0;
        match unsafe { GetCurrentPackageFullName(&mut length, None) } {
            ERROR_INSUFFICIENT_BUFFER => Ok(true),
            APPMODEL_ERROR_NO_PACKAGE => Ok(false),
            error => Err(format!(
                "Cannot query Windows package identity: {}",
                error.0
            )),
        }
    }

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() }
        }
    }

    fn state(task: &StartupTask) -> windows::core::Result<TaskState> {
        match task.State()? {
            StartupTaskState::Disabled => Ok(TaskState::Disabled),
            StartupTaskState::DisabledByUser => Ok(TaskState::DisabledByUser),
            StartupTaskState::DisabledByPolicy => Ok(TaskState::DisabledByPolicy),
            StartupTaskState::Enabled => Ok(TaskState::Enabled),
            StartupTaskState::EnabledByPolicy => Ok(TaskState::EnabledByPolicy),
            _ => Err(Error::new(E_FAIL, "Unknown Windows startup task state")),
        }
    }

    struct Request {
        enabled: Option<bool>,
        reply: mpsc::SyncSender<Result<bool, String>>,
    }

    pub fn access_task(requested: Option<bool>) -> Result<bool, String> {
        static WORKER: OnceLock<Result<mpsc::Sender<Request>, String>> = OnceLock::new();
        let worker = WORKER
            .get_or_init(|| {
                let (sender, receiver) = mpsc::channel::<Request>();
                std::thread::Builder::new()
                    .name("startup-task".into())
                    .spawn(move || {
                        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                            .map_err(|error| error.to_string());
                        // windows-rs caches activation factories for the process lifetime.
                        // Keep their MTA alive, and never send COM objects/errors to the UI thread.
                        let _apartment = initialized.as_ref().ok().map(|_| Apartment);
                        for request in receiver {
                            let result = match &initialized {
                                Ok(()) => {
                                    operate_task(request.enabled).map_err(|error| error.to_string())
                                }
                                Err(error) => Err(error.clone()),
                            };
                            let _ = request.reply.send(result);
                        }
                    })
                    .map_err(|error| error.to_string())?;
                Ok(sender)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let (reply, response) = mpsc::sync_channel(1);
        worker
            .send(Request {
                enabled: requested,
                reply,
            })
            .map_err(|_| "Startup task worker stopped".to_string())?;
        response
            .recv_timeout(std::time::Duration::from_secs(15))
            .map_err(|error| format!("Startup task response unavailable: {error}"))?
    }

    fn operate_task(requested: Option<bool>) -> windows::core::Result<bool> {
        let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))?.get()?;
        if let Some(enabled) = requested {
            match task_action(state(&task)?, enabled).map_err(|error| Error::new(E_FAIL, error))? {
                TaskAction::Enable => {
                    task.RequestEnableAsync()?.get()?;
                }
                TaskAction::Disable => task.Disable()?,
                TaskAction::None => {}
            }
        }
        let enabled = matches!(
            state(&task)?,
            TaskState::Enabled | TaskState::EnabledByPolicy
        );
        if requested.is_some_and(|requested| requested != enabled) {
            return Err(Error::new(
                E_FAIL,
                "Windows rejected the startup task change",
            ));
        }
        Ok(enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[cfg(windows)]
    #[test]
    fn repeated_winrt_reads_keep_the_activation_factory_valid() {
        assert!(!packaged::has_identity().unwrap());
        // Reads from an unpackaged test host fail, but must not invalidate COM.
        for _ in 0..2 {
            let error = packaged::access_task(None).unwrap_err();
            assert!(
                !error.contains("Startup task response unavailable"),
                "{error}"
            );
        }
    }

    #[test]
    fn packaged_startup_preserves_preferences_then_respects_external_changes() {
        assert_eq!(initial_task_request(false, false), Some(false));
        assert_eq!(initial_task_request(false, true), Some(true));
        assert_eq!(initial_task_request(true, false), None);
        assert_eq!(initial_task_request(true, true), None);
    }

    #[test]
    fn startup_task_obeys_user_and_policy_blocks() {
        assert!(task_action(TaskState::DisabledByUser, true).is_err());
        assert!(task_action(TaskState::DisabledByPolicy, true).is_err());
        assert!(task_action(TaskState::EnabledByPolicy, false).is_err());
        assert_eq!(
            task_action(TaskState::DisabledByUser, false).unwrap(),
            TaskAction::None
        );
        assert_eq!(
            task_action(TaskState::DisabledByPolicy, false).unwrap(),
            TaskAction::None
        );
        assert_eq!(
            task_action(TaskState::EnabledByPolicy, true).unwrap(),
            TaskAction::None
        );
    }

    #[test]
    fn startup_task_enables_disables_and_is_idempotent() {
        assert_eq!(
            task_action(TaskState::Disabled, true).unwrap(),
            TaskAction::Enable
        );
        assert_eq!(
            task_action(TaskState::Enabled, false).unwrap(),
            TaskAction::Disable
        );
        assert_eq!(
            task_action(TaskState::Disabled, false).unwrap(),
            TaskAction::None
        );
        assert_eq!(
            task_action(TaskState::Enabled, true).unwrap(),
            TaskAction::None
        );
        assert_eq!(STARTUP_TASK_ID, "LazyWordsStartup");
    }

    #[test]
    fn unrelated_settings_do_not_access_autostart() {
        let saved = Cell::new(false);
        save_change(
            None,
            || panic!("unexpected read"),
            |_| panic!("unexpected change"),
            || {
                saved.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(saved.get());
    }

    #[test]
    fn registration_failure_never_saves_settings() {
        let calls = RefCell::new(Vec::new());
        let result = save_change(
            Some(true),
            || Ok(false),
            |value| {
                calls.borrow_mut().push(value);
                if value {
                    Err("blocked".into())
                } else {
                    Ok(())
                }
            },
            || panic!("must not save"),
        );
        assert_eq!(result.unwrap_err(), "blocked");
        assert_eq!(*calls.borrow(), vec![true, false]);
    }

    #[test]
    fn failed_persistence_restores_actual_previous_os_state() {
        let calls = RefCell::new(Vec::new());
        let result = save_change(
            Some(false),
            || Ok(true),
            |value| {
                calls.borrow_mut().push(value);
                Ok(())
            },
            || Err("disk full".into()),
        );
        assert_eq!(result.unwrap_err(), "disk full");
        assert_eq!(*calls.borrow(), vec![false, true]);
    }

    #[test]
    fn failed_read_prevents_registration_and_persistence() {
        assert!(save_change(
            Some(true),
            || Err("unavailable".into()),
            |_| panic!("must not change"),
            || panic!("must not save")
        )
        .is_err());
    }

    #[test]
    fn matching_os_state_is_saved_without_re_registration() {
        save_change(
            Some(true),
            || Ok(true),
            |_| panic!("already enabled"),
            || Ok(()),
        )
        .unwrap();
    }

    #[test]
    fn rollback_failure_is_not_silenced() {
        let result = save_change(
            Some(true),
            || Ok(false),
            |value| {
                if value {
                    Ok(())
                } else {
                    Err("rollback denied".into())
                }
            },
            || Err("disk full".into()),
        );
        assert!(result
            .unwrap_err()
            .contains("disk full; autostart rollback failed: rollback denied"));
    }
}
