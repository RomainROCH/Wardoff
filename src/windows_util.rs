use std::env;
use std::ffi::OsStr;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use windows::core::{Result as WindowsResult, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_CANCELLED, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK, SW_SHOWNORMAL};

pub(crate) const INTERNAL_ELEVATED_RELAUNCH_ARG: &str = "--wardoff-elevated-relaunch";

pub(crate) enum ElevationLaunchResult {
    Launched,
    Cancelled,
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
