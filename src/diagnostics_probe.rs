//! The Doctor's local Windows queries run in one owned, non-elevated child.
//! COM and filesystem calls can stall; closing this job cannot affect the runtime.

use crate::diagnostics::{Elevation, HistoricalFailure, Inspection, LocalContext, SessionKind};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, SetHandleInformation, ERROR_BROKEN_PIPE, HANDLE, HANDLE_FLAGS,
    HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, GetLengthSid, GetTokenInformation,
    SetTokenInformation, TokenElevation, TokenIntegrityLevel, WinBuiltinAdministratorsSid,
    DISABLE_MAX_PRIVILEGE, LUA_TOKEN, PSID, SECURITY_ATTRIBUTES, SID_AND_ATTRIBUTES,
    TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
    GetExitCodeProcess, InitializeProcThreadAttributeList, OpenProcessToken, ResumeThread,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

const MAX_OUTPUT: usize = 16 * 1024;
const CLEANUP_RESERVE: Duration = Duration::from_millis(150);

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ProbeResult {
    pub(crate) context: LocalContext,
    pub(crate) recent_failures: Vec<HistoricalFailure>,
}

/// All raw errors stay local. Partial/unavailable inspection never changes protection.
pub(crate) fn inspect(deadline: Instant) -> ProbeResult {
    let mut fallback = ProbeResult::default();
    fallback.context.client_elevation = match crate::windows_util::is_process_elevated() {
        Ok(true) => Elevation::Elevated,
        Ok(false) => Elevation::Standard,
        Err(_) => Elevation::Unknown,
    };
    fallback.context.session_kind = match crate::session_scope::current_session_id() {
        Ok(0) => SessionKind::Service,
        Ok(_) => SessionKind::UserSession,
        Err(_) => SessionKind::Unknown,
    };
    if fallback.context.session_kind != SessionKind::UserSession
        || fallback.context.client_elevation == Elevation::Unknown
    {
        return fallback;
    }
    let Some(work_deadline) = deadline.checked_sub(CLEANUP_RESERVE) else {
        return fallback;
    };
    if Instant::now() >= work_deadline {
        fallback.context.inspection = Inspection::TimedOut;
        return fallback;
    }
    let work_deadline = work_deadline.min(Instant::now() + Duration::from_secs(1));
    let outcome = (|| {
        ensure_time(work_deadline)?;
        let executable = std::env::current_exe().map_err(|_| ())?;
        ensure_time(work_deadline)?;
        let token = if fallback.context.client_elevation == Elevation::Elevated {
            Some(restricted_primary_token()?)
        } else {
            None
        };
        ensure_time(work_deadline)?;
        let child = OwnedProbe::spawn(
            &executable,
            "wardoff-diagnostics-probe",
            token.as_ref(),
            work_deadline,
        )?;
        let bytes = child.read(work_deadline)?;
        decode_result(&bytes)
    })();
    match outcome {
        Ok(mut result) => {
            result.context.client_elevation = fallback.context.client_elevation;
            result.context.session_kind = fallback.context.session_kind;
            result
        }
        Err(()) => {
            if Instant::now() >= work_deadline {
                fallback.context.inspection = Inspection::TimedOut;
            }
            fallback
        }
    }
}

fn decode_result(bytes: &[u8]) -> Result<ProbeResult, ()> {
    if bytes.len() > MAX_OUTPUT {
        return Err(());
    }
    serde_json::from_slice(bytes).map_err(|_| ())
}

/// Hidden child entrypoint. Even a direct invocation never inspects with a high token.
pub(crate) fn emit() -> Result<i32, String> {
    let mut result = ProbeResult::default();
    if matches!(crate::windows_util::is_process_elevated(), Ok(false)) {
        result.context.client_elevation = Elevation::Standard;
        result.context.inspection = Inspection::Complete;
        result.context.update_task = crate::blocker::update::reboot_task_presence();
        result.context.autostart = crate::autostart::inspect_read_only();
        result.recent_failures = crate::logger::read_diagnostic_history();
    }
    let json = serde_json::to_string(&result)
        .map_err(|_| "Diagnostic serialization failed.".to_string())?;
    println!("{json}");
    Ok(0)
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// LUA filtering, explicit admin denial, disabled privileges and medium integrity.
/// Failure deliberately has no unrestricted/elevated launch fallback.
fn restricted_primary_token() -> Result<Handle, ()> {
    unsafe {
        let mut source = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
            &mut source,
        )
        .map_err(|_| ())?;
        let source = Handle(source);
        let mut admin_storage = [0u32; 32];
        let mut admin_size = size_of_val(&admin_storage) as u32;
        let admin = PSID(admin_storage.as_mut_ptr().cast());
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            None,
            Some(admin),
            &mut admin_size,
        )
        .map_err(|_| ())?;
        let disabled = [SID_AND_ATTRIBUTES {
            Sid: admin,
            Attributes: 0,
        }];
        let mut restricted = HANDLE::default();
        CreateRestrictedToken(
            source.0,
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
            Some(&disabled),
            None,
            None,
            &mut restricted,
        )
        .map_err(|_| ())?;
        let restricted = Handle(restricted);
        let mut medium = PSID::default();
        ConvertStringSidToSidW(PCWSTR(wide("S-1-16-8192").as_ptr()), &mut medium)
            .map_err(|_| ())?;
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: medium,
                Attributes: SE_GROUP_INTEGRITY as u32,
            },
        };
        let set = SetTokenInformation(
            restricted.0,
            TokenIntegrityLevel,
            &label as *const _ as _,
            size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(medium),
        );
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            medium.0,
        )));
        set.map_err(|_| ())?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0;
        GetTokenInformation(
            restricted.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .map_err(|_| ())?;
        if elevation.TokenIsElevated != 0 {
            return Err(());
        }
        Ok(restricted)
    }
}

struct Attributes {
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn handles(handles: &[HANDLE]) -> Result<Self, ()> {
        unsafe {
            let mut bytes = 0;
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut bytes);
            if bytes == 0 || bytes > MAX_OUTPUT {
                return Err(());
            }
            let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let pointer = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
            InitializeProcThreadAttributeList(Some(pointer), 1, None, &mut bytes)
                .map_err(|_| ())?;
            let attributes = Self {
                _storage: storage,
                pointer,
            };
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                size_of_val(handles),
                None,
                None,
            )
            .map_err(|_| ())?;
            Ok(attributes)
        }
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

struct OwnedProbe {
    process: Handle,
    _thread: Handle,
    _job: Handle,
    stdout: Handle,
    cleanup_deadline: Instant,
}
impl OwnedProbe {
    fn spawn(
        executable: &std::path::Path,
        arguments: &str,
        token: Option<&Handle>,
        deadline: Instant,
    ) -> Result<Self, ()> {
        ensure_time(deadline)?;
        unsafe {
            let job = Handle(CreateJobObjectW(None, PCWSTR::null()).map_err(|_| ())?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                size_of_val(&limits) as u32,
            )
            .map_err(|_| ())?;
            let security = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                bInheritHandle: true.into(),
                ..Default::default()
            };
            let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
            CreatePipe(&mut read, &mut write, Some(&security), 0).map_err(|_| ())?;
            let read = Handle(read);
            let write = Handle(write);
            SetHandleInformation(read.0, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)).map_err(|_| ())?;
            let null = OpenOptions::new()
                .read(true)
                .write(true)
                .open("NUL")
                .map_err(|_| ())?;
            let null_handle = HANDLE(null.as_raw_handle());
            SetHandleInformation(null_handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT)
                .map_err(|_| ())?;
            let inherited = [write.0, null_handle];
            let attributes = Attributes::handles(&inherited)?;
            let mut startup = STARTUPINFOEXW {
                lpAttributeList: attributes.pointer,
                ..Default::default()
            };
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = null_handle;
            startup.StartupInfo.hStdError = null_handle;
            startup.StartupInfo.hStdOutput = write.0;
            let application = wide(executable.as_os_str());
            let mut command = wide(format!("\"{}\" {arguments}", executable.display()));
            let environment = minimal_environment();
            let directory = executable
                .parent()
                .map(|path| wide(path.as_os_str()))
                .ok_or(())?;
            let flags = CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT;
            let mut process = PROCESS_INFORMATION::default();
            ensure_time(deadline)?;
            match token {
                Some(token) => CreateProcessAsUserW(
                    Some(token.0),
                    PCWSTR(application.as_ptr()),
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    true,
                    flags,
                    Some(environment.as_ptr().cast()),
                    PCWSTR(directory.as_ptr()),
                    &startup.StartupInfo,
                    &mut process,
                ),
                None => CreateProcessW(
                    PCWSTR(application.as_ptr()),
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    true,
                    flags,
                    Some(environment.as_ptr().cast()),
                    PCWSTR(directory.as_ptr()),
                    &startup.StartupInfo,
                    &mut process,
                ),
            }
            .map_err(|_| ())?;
            let child = Self {
                process: Handle(process.hProcess),
                _thread: Handle(process.hThread),
                _job: job,
                stdout: read,
                cleanup_deadline: deadline + CLEANUP_RESERVE,
            };
            ensure_time(deadline)?;
            AssignProcessToJobObject(child._job.0, child.process.0).map_err(|_| ())?;
            ensure_time(deadline)?;
            if ResumeThread(child._thread.0) == u32::MAX {
                return Err(());
            }
            Ok(child)
        }
    }

    fn read(self, deadline: Instant) -> Result<Vec<u8>, ()> {
        let mut output = Vec::new();
        let mut stdout_closed = false;
        loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            let mut available = 0;
            if !stdout_closed {
                match unsafe {
                    PeekNamedPipe(self.stdout.0, None, 0, None, Some(&mut available), None)
                } {
                    Ok(()) => {}
                    Err(error)
                        if error.code()
                            == windows::core::HRESULT::from_win32(ERROR_BROKEN_PIPE.0) =>
                    {
                        // Closing stdout can precede the process exit signal. Keep
                        // the same deadline while waiting for that owned process.
                        stdout_closed = true;
                    }
                    Err(_) => return Err(()),
                }
            }
            if available > 0 {
                if output.len() == MAX_OUTPUT {
                    return Err(());
                }
                let mut buffer = [0u8; 1024];
                let count = buffer
                    .len()
                    .min(available as usize)
                    .min(MAX_OUTPUT - output.len());
                let mut read = 0;
                unsafe {
                    ReadFile(
                        self.stdout.0,
                        Some(&mut buffer[..count]),
                        Some(&mut read),
                        None,
                    )
                }
                .map_err(|_| ())?;
                if read == 0 {
                    return Err(());
                }
                output.extend_from_slice(&buffer[..read as usize]);
                if output.len() == MAX_OUTPUT && output.last() != Some(&b'\n') {
                    return Err(());
                }
            } else {
                let exited = unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_OBJECT_0;
                if exited {
                    let mut code = 1;
                    unsafe { GetExitCodeProcess(self.process.0, &mut code) }.map_err(|_| ())?;
                    return if code == 0 && output.last() == Some(&b'\n') {
                        Ok(output)
                    } else {
                        Err(())
                    };
                }
                std::thread::sleep(
                    Duration::from_millis(5)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    }
}
impl Drop for OwnedProbe {
    fn drop(&mut self) {
        unsafe {
            if WaitForSingleObject(self.process.0, 0) != WAIT_OBJECT_0 {
                let _ = TerminateProcess(self.process.0, 1);
                let remaining = self
                    .cleanup_deadline
                    .saturating_duration_since(Instant::now())
                    .min(CLEANUP_RESERVE);
                let _ = WaitForSingleObject(self.process.0, remaining.as_millis() as u32);
            }
        }
    }
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn ensure_time(deadline: Instant) -> Result<(), ()> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(())
    }
}

fn minimal_environment() -> Vec<u16> {
    let mut result = Vec::new();
    for name in [
        "APPDATA",
        "LOCALAPPDATA",
        "SystemDrive",
        "SystemRoot",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "WINDIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            result.extend(OsStr::new(name).encode_wide());
            result.push(b'=' as u16);
            result.extend(value.encode_wide());
            result.push(0);
        }
    }
    if result.is_empty() {
        result.push(0);
    }
    result.push(0);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
    use windows::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, TokenIntegrityLevel, TOKEN_ELEVATION,
        TOKEN_MANDATORY_LABEL,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    fn command_interpreter() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot is required"))
            .join("System32")
            .join("cmd.exe")
    }

    fn powershell() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot is required"))
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe")
    }

    fn duplicate_process(process: HANDLE) -> Handle {
        let mut duplicate = HANDLE::default();
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                process,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
            .expect("process handle should duplicate");
        }
        Handle(duplicate)
    }

    #[test]
    fn owned_probe_reads_constant_cmd_output_and_reaps_normally() {
        let child = OwnedProbe::spawn(
            &command_interpreter(),
            "/D /Q /C echo {}",
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("cmd child should start");
        let duplicate = duplicate_process(child.process.0);
        let output = child
            .read(Instant::now() + Duration::from_secs(5))
            .expect("cmd output should be read");
        assert_eq!(output, b"{}\r\n");
        assert_eq!(
            unsafe { WaitForSingleObject(duplicate.0, 0) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn decode_result_rejects_malformed_probe_output() {
        assert!(decode_result(b"not-json\n").is_err());
    }

    #[test]
    fn owned_probe_rejects_oversized_output_and_timeout() {
        let oversized = OwnedProbe::spawn(
            &command_interpreter(),
            "/D /Q /C for /L %i in (1,1,20000) do @echo x",
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("oversized-output child should start");
        let oversized_process = duplicate_process(oversized.process.0);
        assert!(oversized
            .read(Instant::now() + Duration::from_secs(5))
            .is_err());
        assert_eq!(
            unsafe { WaitForSingleObject(oversized_process.0, 5000) },
            WAIT_OBJECT_0
        );

        let silent = OwnedProbe::spawn(
            &powershell(),
            "-NoProfile -NonInteractive -Command Start-Sleep -Seconds 30",
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("silent child should start");
        let silent_process = duplicate_process(silent.process.0);
        assert!(silent
            .read(Instant::now() + Duration::from_millis(100))
            .is_err());
        assert_eq!(
            unsafe { WaitForSingleObject(silent_process.0, 5000) },
            WAIT_OBJECT_0
        );

        let exact = OwnedProbe::spawn(
            &powershell(),
            &format!(
                "-NoProfile -NonInteractive -Command \"[Console]::Write(('x' * {} ) + [Environment]::NewLine)\"",
                MAX_OUTPUT - 2
            ),
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("exact-boundary child should start");
        let exact_output = exact
            .read(Instant::now() + Duration::from_secs(5))
            .expect("exact MAX_OUTPUT output should be accepted");
        assert_eq!(exact_output.len(), MAX_OUTPUT);
    }

    #[test]
    fn owned_probe_cleanup_does_not_stop_another_child() {
        let first = OwnedProbe::spawn(
            &powershell(),
            "-NoProfile -NonInteractive -Command Start-Sleep -Seconds 30",
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("first child should start");
        let second = OwnedProbe::spawn(
            &powershell(),
            "-NoProfile -NonInteractive -Command Start-Sleep -Seconds 30",
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("second child should start");
        let first_process = duplicate_process(first.process.0);
        let second_process = duplicate_process(second.process.0);
        drop(first);
        assert_eq!(
            unsafe { WaitForSingleObject(first_process.0, 5000) },
            WAIT_OBJECT_0
        );
        assert_ne!(
            unsafe { WaitForSingleObject(second_process.0, 0) },
            WAIT_OBJECT_0
        );
        drop(second);
        assert_eq!(
            unsafe { WaitForSingleObject(second_process.0, 5000) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn minimal_environment_is_whitelisted_and_double_nul_terminated() {
        let environment = minimal_environment();
        assert!(environment.ends_with(&[0, 0]));
        let allowed = [
            "APPDATA",
            "LOCALAPPDATA",
            "SystemDrive",
            "SystemRoot",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "WINDIR",
        ];
        for entry in environment[..environment.len() - 1]
            .split(|value| *value == 0)
            .filter(|entry| !entry.is_empty())
        {
            let name = entry
                .iter()
                .position(|value| *value == b'=' as u16)
                .map(|index| String::from_utf16_lossy(&entry[..index]))
                .expect("environment entry should have a key");
            assert!(allowed.contains(&name.as_str()));
        }
    }

    #[test]
    fn nonexistent_executable_fails_without_a_child() {
        assert!(OwnedProbe::spawn(
            Path::new("C:\\Wardoff-test-no-such-executable.exe"),
            "/D /Q /C echo",
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .is_err());
        assert!(OwnedProbe::spawn(
            &command_interpreter(),
            "/D /Q /C echo {}",
            None,
            Instant::now() - Duration::from_secs(1),
        )
        .is_err());
    }

    #[test]
    fn restricted_token_is_non_elevated_and_medium_when_available() {
        let Ok(token) = restricted_primary_token() else {
            eprintln!("restricted token unavailable; skipping token-shape assertion");
            return;
        };
        unsafe {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut returned = 0;
            GetTokenInformation(
                token.0,
                TokenElevation,
                Some(&mut elevation as *mut _ as _),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
            .expect("restricted token elevation query should succeed");
            assert_eq!(elevation.TokenIsElevated, 0);

            let mut storage = [0u8; 256];
            GetTokenInformation(
                token.0,
                TokenIntegrityLevel,
                Some(storage.as_mut_ptr().cast()),
                storage.len() as u32,
                &mut returned,
            )
            .expect("restricted token integrity query should succeed");
            let label = &*(storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
            let count = *GetSidSubAuthorityCount(label.Label.Sid);
            let rid = *GetSidSubAuthority(label.Label.Sid, u32::from(count - 1));
            assert_eq!(rid, 8192);
        }
    }
}
