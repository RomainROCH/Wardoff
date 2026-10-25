//! Process-instance ownership for optional protection, with no termination rights.

use crate::runtime_policy::sessions::OwnerKey;
use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::Foundation::{FILETIME, HANDLE, WAIT_TIMEOUT};
use windows::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeClientSessionId};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE,
};

/// Suspend-inclusive elapsed milliseconds, independent of the wall clock.
pub(crate) fn elapsed_millis() -> u64 {
    // Win32 GetTickCount64 includes low-power time; an unbiased clock does not.
    unsafe { GetTickCount64() }
}

fn process_session(pid: u32) -> Result<u32, String> {
    let mut session = 0;
    unsafe { ProcessIdToSessionId(pid, &mut session) }.map_err(|error| {
        format!("Wardoff could not inspect the session owner's Windows session: {error}")
    })?;
    Ok(session)
}

/// Keeps the kernel process object alive so a reused PID cannot adopt a lease.
#[derive(Debug)]
pub(crate) struct SessionOwner {
    key: OwnerKey,
    handle: OwnedHandle,
}

impl SessionOwner {
    /// Captures the authenticated connected pipe client's process instance.
    pub(crate) fn from_pipe(pipe: &File) -> Result<Self, String> {
        let handle = HANDLE(pipe.as_raw_handle());
        let mut pid = 0;
        let mut session = 0;
        unsafe {
            GetNamedPipeClientProcessId(handle, &mut pid)
                .and_then(|_| GetNamedPipeClientSessionId(handle, &mut session))
        }
        .map_err(|error| {
            format!("Wardoff could not inspect the connected session owner: {error}")
        })?;
        let current_session = process_session(std::process::id())?;
        if session != current_session {
            return Err(
                "Wardoff sessions require an owner in the same Windows session.".to_string(),
            );
        }
        if pid == std::process::id() {
            Self::current()
        } else {
            Self::open(pid, current_session)
        }
    }

    /// Captures this process for sessions created by the native panel.
    pub(crate) fn current() -> Result<Self, String> {
        let pid = std::process::id();
        Self::open(pid, process_session(pid)?)
    }

    fn open(pid: u32, expected_session: u32) -> Result<Self, String> {
        if pid == 0 || process_session(pid)? != expected_session {
            return Err("Wardoff cannot use this process as a session owner.".to_string());
        }
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .map_err(|error| format!("Wardoff could not retain the session owner: {error}"))?;
        // OpenProcess returned a real owned handle. No termination/modify right is requested.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                HANDLE(handle.as_raw_handle()),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(|error| {
            format!("Wardoff could not inspect the session owner's creation time: {error}")
        })?;
        let owner = Self {
            key: OwnerKey {
                pid,
                creation_time: (u64::from(creation.dwHighDateTime) << 32)
                    | u64::from(creation.dwLowDateTime),
            },
            handle,
        };
        if owner.key.creation_time == 0 || !owner.is_alive() {
            return Err(
                "Wardoff cannot acquire a session for an exited or uninspectable owner."
                    .to_string(),
            );
        }
        Ok(owner)
    }

    /// Returns a process-instance key without command-line or account details.
    pub(crate) fn key(&self) -> OwnerKey {
        self.key
    }

    /// Observes exit through the retained process object, never by PID lookup.
    pub(crate) fn is_alive(&self) -> bool {
        // An error is conservatively treated as owner loss, releasing its intent.
        unsafe { WaitForSingleObject(HANDLE(self.handle.as_raw_handle()), 0) == WAIT_TIMEOUT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;

    fn current_session() -> u32 {
        let mut session = 0;
        unsafe { ProcessIdToSessionId(std::process::id(), &mut session) }.unwrap();
        session
    }

    #[test]
    fn current_owner_has_creation_identity_and_requires_the_same_session() {
        let owner = SessionOwner::current().expect("inspect the current process");
        assert_eq!(owner.key().pid, std::process::id());
        assert_ne!(owner.key().creation_time, 0);
        assert!(owner.is_alive());
        assert!(SessionOwner::open(std::process::id(), current_session() ^ 1).is_err());
        assert!(SessionOwner::open(u32::MAX, current_session()).is_err());
        assert!(SessionOwner::open(0, current_session()).is_err());
    }

    #[test]
    fn retained_owner_observes_actual_exit_without_terminating_the_process() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$null = [Console]::In.ReadLine()",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start inert owner fixture");
        let owner = SessionOwner::open(child.id(), current_session());
        let observed_alive = owner.as_ref().is_ok_and(|owner| owner.is_alive());
        // Always close stdin and reap our own child before asserting, even on RED.
        drop(child.stdin.take());
        let status = child.wait().expect("reap inert owner fixture");
        assert!(status.success());
        let owner = owner.expect("retain the live fixture process object");
        assert!(observed_alive);
        assert!(!owner.is_alive());
        assert_ne!(owner.key().creation_time, 0);
    }
}
