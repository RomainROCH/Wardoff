use std::mem::size_of;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{GetTokenInformation, TokenSessionId, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const MUTEX_NAME_PREFIX: &str = r"Local\WardoffInstance-Session-";
const CONTROL_PIPE_PATH_PREFIX: &str = r"\\.\pipe\WardoffControl-Session-";
const STATUS_PIPE_PATH_PREFIX: &str = r"\\.\pipe\WardoffStatus-Session-";

pub(crate) fn current_session_id() -> Result<u32, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|error| {
            format!("Wardoff could not query its process token for the current session ID: {error}")
        })?;
        let token = HandleGuard(token);

        let mut session_id = 0u32;
        let mut returned_size = 0u32;
        GetTokenInformation(
            token.0,
            TokenSessionId,
            Some(&mut session_id as *mut _ as *mut _),
            size_of::<u32>() as u32,
            &mut returned_size,
        )
        .map_err(|error| {
            format!("Wardoff could not read its current session ID from the process token: {error}")
        })?;

        if returned_size != size_of::<u32>() as u32 {
            return Err(format!(
                "Wardoff received an unexpected session-ID size ({returned_size}) from the process token."
            ));
        }

        Ok(session_id)
    }
}

pub(crate) fn current_mutex_name() -> Result<String, String> {
    Ok(mutex_name_for_session(current_session_id()?))
}

pub(crate) fn current_control_pipe_path() -> Result<String, String> {
    Ok(control_pipe_path_for_session(current_session_id()?))
}

pub(crate) fn current_status_pipe_path() -> Result<String, String> {
    Ok(status_pipe_path_for_session(current_session_id()?))
}

pub(crate) fn mutex_name_for_session(session_id: u32) -> String {
    format!("{MUTEX_NAME_PREFIX}{session_id}")
}

pub(crate) fn control_pipe_path_for_session(session_id: u32) -> String {
    format!("{CONTROL_PIPE_PATH_PREFIX}{session_id}")
}

pub(crate) fn status_pipe_path_for_session(session_id: u32) -> String {
    format!("{STATUS_PIPE_PATH_PREFIX}{session_id}")
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

#[cfg(test)]
mod tests {
    use super::{
        control_pipe_path_for_session, mutex_name_for_session, status_pipe_path_for_session,
    };

    #[test]
    fn mutex_name_is_local_and_session_scoped() {
        assert_eq!(
            mutex_name_for_session(42),
            r"Local\WardoffInstance-Session-42"
        );
    }

    #[test]
    fn control_and_status_pipes_share_the_same_session_suffix() {
        assert_eq!(
            control_pipe_path_for_session(42),
            r"\\.\pipe\WardoffControl-Session-42"
        );
        assert_eq!(
            status_pipe_path_for_session(42),
            r"\\.\pipe\WardoffStatus-Session-42"
        );
    }
}
