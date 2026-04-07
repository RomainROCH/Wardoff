use std::env;
use std::ffi::{OsStr, OsString};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use windows::core::{Result as WindowsResult, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_CANCELLED, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList, GetConsoleWindow};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcessToken, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, ShowWindow, MB_ICONERROR, MB_OK, SW_HIDE, SW_SHOWNORMAL,
};

pub(crate) const INTERNAL_DETACHED_RUNTIME_ARG: &str = "--wardoff-detached-runtime";
pub(crate) const INTERNAL_ELEVATED_RELAUNCH_ARG: &str = "--wardoff-elevated-relaunch";

pub(crate) enum ElevationLaunchResult {
    Launched,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConsoleLaunchContext {
    None,
    Owned,
    Inherited,
}

pub(crate) fn is_process_elevated() -> WindowsResult<bool> {
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

pub(crate) fn relaunch_self_elevated() -> Result<ElevationLaunchResult, String> {
    let executable_path = env::current_exe().map_err(|error| {
        format!("Wardoff could not resolve its executable path for elevation: {error}")
    })?;
    let operation = wide_null(OsStr::new("runas"));
    let executable_path_wide = wide_null(executable_path.as_os_str());
    let parameters = wide_null(OsStr::new(INTERNAL_ELEVATED_RELAUNCH_ARG));
    let working_directory = executable_path
        .parent()
        .map(|path| wide_null(path.as_os_str()));
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(executable_path_wide.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            working_directory
                .as_ref()
                .map_or(PCWSTR::null(), |path| PCWSTR(path.as_ptr())),
            SW_SHOWNORMAL,
        )
    };

    if result.0 as usize > 32 {
        return Ok(ElevationLaunchResult::Launched);
    }

    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_CANCELLED {
        return Ok(ElevationLaunchResult::Cancelled);
    }

    let last_error_suffix = if last_error.0 == 0 {
        String::new()
    } else {
        format!(" (Win32 error {})", last_error.0)
    };

    Err(format!(
        "Wardoff could not relaunch itself with administrator rights (ShellExecuteW code {}).{last_error_suffix}",
        result.0 as usize
    ))
}

pub(crate) fn console_launch_context() -> Result<ConsoleLaunchContext, String> {
    let console_window = unsafe { GetConsoleWindow() };
    if console_window.0.is_null() {
        return Ok(ConsoleLaunchContext::None);
    }

    let mut attached_processes = [0u32; 2];
    let process_count = unsafe { GetConsoleProcessList(&mut attached_processes) };

    if process_count == 0 {
        let last_error = unsafe { GetLastError() };
        return Err(format!(
            "Wardoff could not inspect its console attachment state (Win32 error {}).",
            last_error.0
        ));
    }

    if process_count == 1 {
        Ok(ConsoleLaunchContext::Owned)
    } else {
        Ok(ConsoleLaunchContext::Inherited)
    }
}

pub(crate) fn hide_and_free_console() -> Result<(), String> {
    let console_window = unsafe { GetConsoleWindow() };
    if console_window.0.is_null() {
        return Ok(());
    }

    unsafe {
        let _ = ShowWindow(console_window, SW_HIDE);
    }

    if let Err(error) = unsafe { FreeConsole() } {
        let last_error = unsafe { GetLastError() };
        Err(format!(
            "Wardoff could not detach from its standalone console window (Win32 error {}).",
            if last_error.0 == 0 {
                error.code().0 as u32
            } else {
                last_error.0
            }
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn relaunch_self_detached() -> Result<(), String> {
    let executable_path = env::current_exe().map_err(|error| {
        format!("Wardoff could not resolve its executable path for detached relaunch: {error}")
    })?;
    let existing_args: Vec<OsString> = env::args_os().skip(1).collect();

    let mut command = Command::new(&executable_path);
    command.args(&existing_args);
    if !existing_args
        .iter()
        .any(|arg| arg == OsStr::new(INTERNAL_DETACHED_RUNTIME_ARG))
    {
        command.arg(INTERNAL_DETACHED_RUNTIME_ARG);
    }

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS.0 | CREATE_NEW_PROCESS_GROUP.0)
        .spawn()
        .map_err(|error| {
            format!("Wardoff could not relaunch its long-lived runtime without a shell console: {error}")
        })?;

    Ok(())
}

pub(crate) fn show_fatal_error_dialog(title: &str, message: &str) {
    let title = wide_null(OsStr::new(title));
    let message = wide_null(OsStr::new(message));

    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}
