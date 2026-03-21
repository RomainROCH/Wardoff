use crate::blocker::update::{connect_task_service, initialize_com_apartment};
use crate::logger::{self, EventSource};
use chrono::Local;
use log::{info, warn};
use std::env;
use std::mem::size_of;
use std::path::Path;
use std::thread;
use windows::core::{Error as WindowsError, Interface, Result as WindowsResult, BSTR, HRESULT};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    E_ACCESSDENIED, HANDLE, VARIANT_FALSE, VARIANT_TRUE,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::CoUninitialize;
use windows::Win32::System::TaskScheduler::{
    IExecAction, ILogonTrigger, IRegisteredTask, ITaskFolder, ITaskService, TASK_ACTION_EXEC,
    TASK_CREATE_OR_UPDATE, TASK_INSTANCES_IGNORE_NEW, TASK_LOGON_INTERACTIVE_TOKEN,
    TASK_RUNLEVEL_HIGHEST, TASK_TRIGGER_LOGON,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::VARIANT;

const AUTOSTART_TASK_NAME: &str = "Wardoff";
const AUTOSTART_TASK_PATH: &str = "\\Wardoff";
const AUTOSTART_THREAD_NAME: &str = "wardoff-autostart";
const ROOT_TASK_FOLDER_PATH: &str = "\\";
const EXECUTION_TIME_LIMIT: &str = "PT0S";

/// Returns whether the Wardoff current-user logon task exists and is enabled.
pub(crate) fn is_enabled() -> Result<bool, String> {
    run_task_scheduler(|| {
        let task_service = connect_task_service().map_err(task_service_error)?;
        task_enabled(&task_service).map_err(task_service_error)
    })
}

/// Creates, updates, or removes the Wardoff current-user logon task.
pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    if let Err(message) = ensure_process_is_elevated() {
        warn!("{message}");
        logger::log_event(
            "autostart_task_updated",
            EventSource::Autostart,
            message.clone(),
            false,
        );
        return Err(message);
    }

    let executable_path = current_executable_path()?;
    let working_directory = current_working_directory(&executable_path);
    let user_id = current_user_id()?;

    let result = if enabled {
        let executable_path = executable_path.clone();
        let working_directory = working_directory.clone();
        let user_id = user_id.clone();
        run_task_scheduler(move || {
            let task_service = connect_task_service().map_err(task_service_error)?;
            register_autostart_task(
                &task_service,
                &user_id,
                &executable_path,
                &working_directory,
            )
            .map_err(task_service_error)?;
            Ok(format!(
                "Wardoff registered or updated scheduled task {AUTOSTART_TASK_PATH} to launch {} at current-user logon with the highest available privileges.",
                executable_path
            ))
        })
    } else {
        run_task_scheduler(|| {
            let task_service = connect_task_service().map_err(task_service_error)?;
            let removed = remove_autostart_task(&task_service).map_err(task_service_error)?;
            if removed {
                Ok(format!(
                    "Wardoff removed scheduled task {AUTOSTART_TASK_PATH}."
                ))
            } else {
                Ok(format!(
                    "Wardoff found no scheduled task {AUTOSTART_TASK_PATH}; Start with Windows was already off."
                ))
            }
        })
    };

    match result {
        Ok(message) => {
            info!("{message}");
            logger::log_event(
                "autostart_task_updated",
                EventSource::Autostart,
                message,
                true,
            );
            Ok(())
        }
        Err(message) => {
            warn!("{message}");
            logger::log_event(
                "autostart_task_updated",
                EventSource::Autostart,
                message.clone(),
                false,
            );
            Err(message)
        }
    }
}

struct ComApartmentGuard;

impl ComApartmentGuard {
    fn initialize() -> Result<Self, String> {
        initialize_com_apartment().map_err(|error| {
            format!(
                "Wardoff could not initialize the COM apartment for Task Scheduler access: {error}"
            )
        })?;
        Ok(Self)
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn run_task_scheduler<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    thread::Builder::new()
        .name(AUTOSTART_THREAD_NAME.to_string())
        .spawn(move || {
            let _com_apartment = ComApartmentGuard::initialize()?;
            operation()
        })
        .map_err(|error| format!("Wardoff could not start its autostart worker thread: {error}"))?
        .join()
        .map_err(|_| "Wardoff autostart worker thread panicked.".to_string())?
}

fn task_enabled(task_service: &ITaskService) -> WindowsResult<bool> {
    match lookup_autostart_task(task_service)? {
        Some(task) => unsafe { Ok(task.Enabled()? != VARIANT_FALSE) },
        None => Ok(false),
    }
}

fn register_autostart_task(
    task_service: &ITaskService,
    user_id: &str,
    executable_path: &str,
    working_directory: &str,
) -> WindowsResult<()> {
    let task_definition = unsafe { task_service.NewTask(0)? };
    configure_registration_info(&task_definition)?;
    configure_principal(&task_definition, user_id)?;
    configure_settings(&task_definition)?;
    configure_logon_trigger(&task_definition, user_id)?;
    configure_exec_action(&task_definition, executable_path, working_directory)?;

    let root_folder = open_root_folder(task_service)?;
    let task_name = BSTR::from(AUTOSTART_TASK_NAME);
    let empty = VARIANT::default();

    unsafe {
        let _ = root_folder.RegisterTaskDefinition(
            &task_name,
            &task_definition,
            TASK_CREATE_OR_UPDATE.0,
            &empty,
            &empty,
            TASK_LOGON_INTERACTIVE_TOKEN,
            &empty,
        )?;
    }

    Ok(())
}

fn remove_autostart_task(task_service: &ITaskService) -> WindowsResult<bool> {
    let root_folder = open_root_folder(task_service)?;
    let task_name = BSTR::from(AUTOSTART_TASK_NAME);

    match unsafe { root_folder.DeleteTask(&task_name, 0) } {
        Ok(()) => Ok(true),
        Err(error) if is_not_found_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

fn lookup_autostart_task(task_service: &ITaskService) -> WindowsResult<Option<IRegisteredTask>> {
    let root_folder = open_root_folder(task_service)?;
    let task_name = BSTR::from(AUTOSTART_TASK_NAME);

    match unsafe { root_folder.GetTask(&task_name) } {
        Ok(task) => Ok(Some(task)),
        Err(error) if is_not_found_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn open_root_folder(task_service: &ITaskService) -> WindowsResult<ITaskFolder> {
    let folder_path = BSTR::from(ROOT_TASK_FOLDER_PATH);
    unsafe { task_service.GetFolder(&folder_path) }
}

fn configure_registration_info(
    task_definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
) -> WindowsResult<()> {
    let registration_info = unsafe { task_definition.RegistrationInfo()? };

    unsafe {
        registration_info.SetAuthor(&BSTR::from("Wardoff"))?;
        registration_info.SetDescription(&BSTR::from(
            "Starts Wardoff at current-user logon with the highest available privileges.",
        ))?;
    }

    Ok(())
}

fn configure_principal(
    task_definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
    user_id: &str,
) -> WindowsResult<()> {
    let principal = unsafe { task_definition.Principal()? };
    let user_id = BSTR::from(user_id);

    unsafe {
        principal.SetId(&BSTR::from("WardoffPrincipal"))?;
        principal.SetUserId(&user_id)?;
        principal.SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN)?;
        principal.SetRunLevel(TASK_RUNLEVEL_HIGHEST)?;
    }

    Ok(())
}

fn configure_settings(
    task_definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
) -> WindowsResult<()> {
    let settings = unsafe { task_definition.Settings()? };
    let execution_time_limit = BSTR::from(EXECUTION_TIME_LIMIT);

    unsafe {
        settings.SetAllowDemandStart(VARIANT_TRUE)?;
        settings.SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW)?;
        settings.SetStopIfGoingOnBatteries(VARIANT_FALSE)?;
        settings.SetDisallowStartIfOnBatteries(VARIANT_FALSE)?;
        settings.SetStartWhenAvailable(VARIANT_TRUE)?;
        settings.SetExecutionTimeLimit(&execution_time_limit)?;
        settings.SetEnabled(VARIANT_TRUE)?;
    }

    Ok(())
}

fn configure_logon_trigger(
    task_definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
    user_id: &str,
) -> WindowsResult<()> {
    let triggers = unsafe { task_definition.Triggers()? };
    let trigger = unsafe { triggers.Create(TASK_TRIGGER_LOGON)? };
    let logon_trigger: ILogonTrigger = trigger.cast()?;
    let start_boundary = BSTR::from(trigger_start_boundary());
    let user_id = BSTR::from(user_id);

    unsafe {
        trigger.SetId(&BSTR::from("WardoffLogonTrigger"))?;
        trigger.SetEnabled(VARIANT_TRUE)?;
        trigger.SetStartBoundary(&start_boundary)?;
        logon_trigger.SetUserId(&user_id)?;
    }

    Ok(())
}

fn configure_exec_action(
    task_definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
    executable_path: &str,
    working_directory: &str,
) -> WindowsResult<()> {
    let actions = unsafe { task_definition.Actions()? };
    let action = unsafe { actions.Create(TASK_ACTION_EXEC)? };
    let exec_action: IExecAction = action.cast()?;
    let executable_path = BSTR::from(executable_path);

    unsafe {
        exec_action.SetPath(&executable_path)?;

        if !working_directory.is_empty() {
            exec_action.SetWorkingDirectory(&BSTR::from(working_directory))?;
        }
    }

    Ok(())
}

fn trigger_start_boundary() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn current_executable_path() -> Result<String, String> {
    let executable_path = env::current_exe().map_err(|error| {
        format!("Wardoff could not resolve its current executable path before updating {AUTOSTART_TASK_PATH}: {error}")
    })?;

    Ok(executable_path.to_string_lossy().into_owned())
}

fn current_working_directory(executable_path: &str) -> String {
    Path::new(executable_path)
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn current_user_id() -> Result<String, String> {
    let username = env::var("USERNAME").map_err(|error| {
        format!("Wardoff could not determine the current user name before updating {AUTOSTART_TASK_PATH}: {error}")
    })?;

    Ok(match env::var("USERDOMAIN") {
        Ok(domain) if !domain.trim().is_empty() => format!("{domain}\\{username}"),
        _ => username,
    })
}

fn ensure_process_is_elevated() -> Result<(), String> {
    match is_process_elevated() {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "Wardoff requires administrator rights to change scheduled task {AUTOSTART_TASK_PATH}. Run Wardoff elevated and try again."
        )),
        Err(error) => Err(format!(
            "Wardoff could not determine whether administrator rights are available before changing scheduled task {AUTOSTART_TASK_PATH}: {error}"
        )),
    }
}

fn is_process_elevated() -> WindowsResult<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = HandleGuard(token);

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned_size = 0u32;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_size,
        )?;

        Ok(elevation.TokenIsElevated != 0)
    }
}

fn task_service_error(error: WindowsError) -> String {
    if is_access_denied_error(&error) {
        format!(
            "Wardoff could not access scheduled task {AUTOSTART_TASK_PATH}. Run Wardoff elevated and try again: {error}"
        )
    } else {
        format!("Wardoff could not update scheduled task {AUTOSTART_TASK_PATH}: {error}")
    }
}

fn is_not_found_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0)
        || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
        || code == HRESULT::from_win32(ERROR_NOT_FOUND.0)
}

fn is_access_denied_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == E_ACCESSDENIED || code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
}
