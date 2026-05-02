use crate::blocker::update::{connect_task_service, initialize_com_apartment};
use crate::logger::{self, EventSource};
use crate::windows_util::is_process_elevated;
use chrono::Local;
use log::{info, warn};
use std::env;
use std::fs;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::thread;
use windows::core::{
    Error as WindowsError, Interface, Result as WindowsResult, BSTR, HRESULT, PCWSTR, PWSTR,
};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER,
    ERROR_NONE_MAPPED, ERROR_NOT_FOUND, ERROR_PATH_NOT_FOUND, E_ACCESSDENIED, HANDLE, HLOCAL,
    VARIANT_FALSE, VARIANT_TRUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    AccessCheck, DuplicateTokenEx, GetTokenInformation, LookupAccountSidW, SecurityImpersonation,
    TokenImpersonation, TokenLinkedToken, TokenUser, DACL_SECURITY_INFORMATION, GENERIC_MAPPING,
    GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PRIVILEGE_SET, PSECURITY_DESCRIPTOR,
    PSID, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_LINKED_TOKEN, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_DELETE_CHILD,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES,
    FILE_WRITE_DATA, FILE_WRITE_EA,
};
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
    if enabled {
        ensure_trusted_autostart_location(
            Path::new(&executable_path),
            Path::new(&working_directory),
        )?;
    }

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
    let executable_path = env::current_exe()
        .map_err(|error| {
            format!(
                "Wardoff could not resolve its current executable path before updating {AUTOSTART_TASK_PATH}: {error}"
            )
        })
        .and_then(|path| {
            fs::canonicalize(&path).map_err(|error| {
                format!(
                    "Wardoff could not canonicalize its current executable path before updating {AUTOSTART_TASK_PATH}: {error}"
                )
            })
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
    unsafe {
        let token = open_current_process_token(TOKEN_QUERY, "resolve the current task principal")?;
        current_token_user_id(token.0, "resolve the current task principal")
    }
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

fn ensure_trusted_autostart_location(
    executable_path: &Path,
    working_directory: &Path,
) -> Result<(), String> {
    let medium_token = current_interactive_medium_token_for_access_check()?;
    let executable_writable = token_can_write_executable_path(medium_token.0, executable_path)?;
    let working_directory_writable = token_can_write_directory(medium_token.0, working_directory)?;

    if executable_writable || working_directory_writable {
        return Err(format!(
            "Wardoff refused to register scheduled task {AUTOSTART_TASK_PATH} from untrusted location {} because the current interactive user's non-elevated token can modify the executable path or its parent directory. Move Wardoff to an admin-writable install location and try again.",
            executable_path.display()
        ));
    }

    Ok(())
}

fn current_interactive_medium_token_for_access_check() -> Result<HandleGuard, String> {
    unsafe {
        let process_token = open_current_process_token(
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE,
            "validate the autostart install location",
        )?;
        let mut linked_token = TOKEN_LINKED_TOKEN::default();
        let mut returned_size = 0u32;
        GetTokenInformation(
            process_token.0,
            TokenLinkedToken,
            Some(&mut linked_token as *mut _ as *mut _),
            size_of::<TOKEN_LINKED_TOKEN>() as u32,
            &mut returned_size,
        )
        .map_err(|error| {
            format!(
                "Wardoff could not resolve the current interactive user's non-elevated token before updating {AUTOSTART_TASK_PATH}: {error}"
            )
        })?;

        let linked_token = HandleGuard(linked_token.LinkedToken);
        let mut impersonation_token = HANDLE::default();
        DuplicateTokenEx(
            linked_token.0,
            TOKEN_QUERY,
            None,
            SecurityImpersonation,
            TokenImpersonation,
            &mut impersonation_token,
        )
        .map_err(|error| {
            format!(
                "Wardoff could not duplicate the current interactive user's non-elevated token before updating {AUTOSTART_TASK_PATH}: {error}"
            )
        })?;

        Ok(HandleGuard(impersonation_token))
    }
}

fn token_can_write_executable_path(token: HANDLE, executable_path: &Path) -> Result<bool, String> {
    token_has_path_access(
        token,
        executable_path,
        (FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_ATTRIBUTES | FILE_WRITE_EA).0,
        "validate the current executable path",
    )
}

fn token_can_write_directory(token: HANDLE, directory_path: &Path) -> Result<bool, String> {
    token_has_path_access(
        token,
        directory_path,
        (FILE_ADD_FILE
            | FILE_ADD_SUBDIRECTORY
            | FILE_WRITE_ATTRIBUTES
            | FILE_WRITE_EA
            | FILE_DELETE_CHILD)
            .0,
        "validate the current executable directory",
    )
}

fn token_has_path_access(
    token: HANDLE,
    path: &Path,
    desired_access: u32,
    context: &str,
) -> Result<bool, String> {
    let security_descriptor = load_path_security_descriptor(path, context)?;
    let mut privileges = vec![0u8; size_of::<PRIVILEGE_SET>() + 256];
    let mut privilege_set_length = privileges.len() as u32;
    let mut granted_access = 0u32;
    let mut access_status = false.into();
    let generic_mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ.0,
        GenericWrite: FILE_GENERIC_WRITE.0,
        GenericExecute: FILE_GENERIC_EXECUTE.0,
        GenericAll: FILE_ALL_ACCESS.0,
    };

    unsafe {
        AccessCheck(
            security_descriptor.0,
            token,
            desired_access,
            &generic_mapping,
            Some(privileges.as_mut_ptr() as *mut PRIVILEGE_SET),
            &mut privilege_set_length,
            &mut granted_access,
            &mut access_status,
        )
        .map_err(|error| {
            format!(
                "Wardoff could not {context} for {} before updating {AUTOSTART_TASK_PATH}: {error}",
                path.display()
            )
        })?;
    }

    Ok(access_status.as_bool())
}

fn load_path_security_descriptor(
    path: &Path,
    context: &str,
) -> Result<LocalSecurityDescriptor, String> {
    let path_wide = wide_null(path);
    let mut security_descriptor = PSECURITY_DESCRIPTOR::default();
    let result = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut security_descriptor,
        )
    };

    if result.0 != 0 {
        return Err(format!(
            "Wardoff could not {context} for {} before updating {AUTOSTART_TASK_PATH} (Win32 error {}).",
            path.display(),
            result.0
        ));
    }

    Ok(LocalSecurityDescriptor(security_descriptor))
}

fn open_current_process_token(
    desired_access: windows::Win32::Security::TOKEN_ACCESS_MASK,
    context: &str,
) -> Result<HandleGuard, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), desired_access, &mut token).map_err(|error| {
            format!("Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: {error}")
        })?;
        Ok(HandleGuard(token))
    }
}

unsafe fn current_token_user_id(token: HANDLE, context: &str) -> Result<String, String> {
    let mut required_length = 0u32;
    let _ = GetTokenInformation(token, TokenUser, None, 0, &mut required_length);
    if required_length == 0 {
        let probe_error = WindowsError::from_thread();
        return Err(format!(
            "Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: {probe_error}"
        ));
    }

    let mut token_information = vec![0u8; required_length as usize];
    GetTokenInformation(
        token,
        TokenUser,
        Some(token_information.as_mut_ptr() as *mut _),
        required_length,
        &mut required_length,
    )
    .map_err(|error| {
        format!("Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: {error}")
    })?;

    let token_user = &*(token_information.as_ptr() as *const TOKEN_USER);
    if token_user.User.Sid.is_invalid() {
        return Err(format!(
            "Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: the process token had no user SID."
        ));
    }

    lookup_account_name_for_sid(token_user.User.Sid, context)
}

fn lookup_account_name_for_sid(sid: PSID, context: &str) -> Result<String, String> {
    let mut username_length = 0u32;
    let mut domain_length = 0u32;
    let mut sid_name_use = Default::default();
    let probe = unsafe {
        LookupAccountSidW(
            None,
            sid,
            None,
            &mut username_length,
            None,
            &mut domain_length,
            &mut sid_name_use,
        )
    };

    if let Err(error) = probe {
        let code = error.code();
        if code == HRESULT::from_win32(ERROR_NONE_MAPPED.0) {
            return sid_to_string(sid, context);
        }
        if code != HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) {
            return Err(format!(
                "Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: {error}"
            ));
        }
    }

    let mut username = vec![0u16; username_length as usize];
    let mut domain = vec![0u16; domain_length as usize];
    unsafe {
        LookupAccountSidW(
            None,
            sid,
            Some(PWSTR(username.as_mut_ptr())),
            &mut username_length,
            if domain.is_empty() {
                None
            } else {
                Some(PWSTR(domain.as_mut_ptr()))
            },
            &mut domain_length,
            &mut sid_name_use,
        )
        .map_err(|error| {
            format!("Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: {error}")
        })?;
    }

    let username = String::from_utf16_lossy(&username[..username_length as usize]);
    let domain = String::from_utf16_lossy(&domain[..domain_length as usize]);
    if username.trim().is_empty() {
        return sid_to_string(sid, context);
    }

    if domain.trim().is_empty() {
        Ok(username)
    } else {
        Ok(format!("{domain}\\{username}"))
    }
}

fn sid_to_string(sid: PSID, context: &str) -> Result<String, String> {
    let mut string_sid = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(sid, &mut string_sid).map_err(|error| {
            format!("Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: {error}")
        })?;
    }

    let string_sid = LocalAllocatedWideString(string_sid);
    let sid = string_sid.to_string();
    if sid.is_empty() {
        return Err(format!(
            "Wardoff could not {context} before updating {AUTOSTART_TASK_PATH}: the process token resolved to an empty SID string."
        ));
    }

    Ok(sid)
}

fn wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
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

struct LocalSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0 .0 as *mut _)));
            }
        }
    }
}

struct LocalAllocatedWideString(PWSTR);

impl LocalAllocatedWideString {
    fn to_string(&self) -> String {
        if self.0.is_null() {
            return String::new();
        }

        unsafe {
            let mut length = 0usize;
            while *self.0 .0.add(length) != 0 {
                length += 1;
            }

            String::from_utf16_lossy(std::slice::from_raw_parts(self.0 .0, length))
        }
    }
}

impl Drop for LocalAllocatedWideString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0 .0 as *mut _)));
            }
        }
    }
}
