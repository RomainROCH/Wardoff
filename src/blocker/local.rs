use crate::blocker::abort::{
    abort_pending_shutdown, preflight_shutdown_abort_capability, ShutdownAbortCapability,
    ShutdownAbortOutcome,
};
use crate::logger::{self, EventSource};
use log::{error, info, warn};
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{Error as WindowsError, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_NOT_SUPPORTED, ERROR_PRIVILEGE_NOT_HELD,
    ERROR_SUCCESS, ERROR_WMI_INSTANCE_NOT_FOUND, WIN32_ERROR,
};
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW,
    TdhGetProperty, TdhGetPropertySize, CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_ENABLE_PROVIDER,
    EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD,
    PROCESS_TRACE_MODE_REAL_TIME, PROPERTY_DATA_DESCRIPTOR, WNODE_FLAG_TRACED_GUID,
};

const LAYER2_THREAD_NAME: &str = "wardoff-layer2-local-shutdown";
const LAYER2_PROCESS_PROVIDER_KEYWORD: u64 = 0x10;
const LAYER2_PROCESS_START_EVENT_ID: u16 = 1;
const LAYER2_PROCESS_START_TASK: u16 = 1;
const LAYER2_PROCESS_START_OPCODE: u8 = 1;
const SHUTDOWN_IMAGE_NAME: &str = "shutdown.exe";
const KERNEL_PROCESS_PROVIDER_GUID: GUID = GUID::from_u128(0x22fb2cd6_0e7b_422b_a0c7_2fad1fd0e716);

/// Coordinates Layer 2 local `shutdown.exe` ETW monitoring.
pub struct LocalShutdownBlocker {
    worker: Option<LocalShutdownWorker>,
    active_state: Arc<AtomicBool>,
}

impl LocalShutdownBlocker {
    /// Starts Layer 2 ETW monitoring if it is not already active.
    pub fn activate(&mut self) -> Result<(), String> {
        self.cleanup_finished_worker();
        if self.worker.is_some() {
            return Ok(());
        }

        match preflight_shutdown_abort_capability() {
            ShutdownAbortCapability::Available => {}
            ShutdownAbortCapability::AccessDenied { details } => {
                let message = format!(
                    "Layer 2 local shutdown protection will stay inactive because this process lacks the shutdown-abort privilege required to cancel shutdown.exe launches: {details}"
                );
                warn!("{message}");
                logger::log_event(
                    "local_layer_unavailable",
                    EventSource::Local,
                    message,
                    false,
                );
                return Ok(());
            }
            ShutdownAbortCapability::Failed { details } => {
                let message = format!(
                    "Layer 2 local shutdown protection will stay inactive because Wardoff could not verify the shutdown-abort capability required before starting ETW monitoring: {details}"
                );
                warn!("{message}");
                logger::log_event(
                    "local_layer_unavailable",
                    EventSource::Local,
                    message,
                    false,
                );
                return Ok(());
            }
        }

        let session_name = build_session_name();
        let subscription = match LocalTraceSubscription::start(session_name.clone()) {
            Ok(subscription) => subscription,
            Err(message) => {
                warn!("{message}");
                logger::log_event(
                    "local_layer_unavailable",
                    EventSource::Local,
                    message,
                    false,
                );
                return Ok(());
            }
        };

        let active_state = Arc::clone(&self.active_state);
        let controller_process_handle = subscription.process_handle.clone();
        let subscription = Arc::new(Mutex::new(Some(subscription)));
        let worker_subscription = Arc::clone(&subscription);
        match thread::Builder::new()
            .name(LAYER2_THREAD_NAME.to_string())
            .spawn(move || {
                let subscription = worker_subscription
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take());

                if let Some(subscription) = subscription {
                    run_worker(subscription, active_state);
                }
            }) {
            Ok(join_handle) => {
                self.worker = Some(LocalShutdownWorker {
                    session_name,
                    process_handle: controller_process_handle,
                    join_handle: Some(join_handle),
                });
                Ok(())
            }
            Err(error) => {
                if let Ok(mut guard) = subscription.lock() {
                    if let Some(subscription) = guard.take() {
                        if let Err(cleanup_error) = subscription.shutdown() {
                            warn!(
                                "Layer 2 could not clean up its ETW session after thread startup failed: {cleanup_error}"
                            );
                        }
                    }
                }
                Err(format!(
                    "Layer 2 could not start its ETW worker thread: {error}"
                ))
            }
        }
    }

    /// Stops Layer 2 ETW monitoring when Block mode ends.
    pub fn deactivate(&mut self) {
        self.cleanup_finished_worker();

        let Some(mut worker) = self.worker.take() else {
            return;
        };

        self.active_state.store(false, Ordering::Release);
        if let Err(error) = stop_trace_session(&worker.session_name) {
            warn!("Layer 2 could not stop its ETW session cleanly: {error}");
            logger::log_event(
                "local_layer_disabled",
                EventSource::Local,
                format!("Layer 2 could not stop its ETW session cleanly: {error}"),
                false,
            );
        }
        if let Err(error) = worker.cancel_trace_processing() {
            warn!("Layer 2 could not close its ETW consumer handle while stopping: {error}");
            logger::log_event(
                "local_layer_disabled",
                EventSource::Local,
                format!("Layer 2 could not close its ETW consumer handle while stopping: {error}"),
                false,
            );
        }

        if let Some(join_handle) = worker.join_handle.take() {
            if join_handle.join().is_err() {
                error!("Layer 2 worker thread panicked while stopping");
                logger::log_event(
                    "local_layer_disabled",
                    EventSource::Local,
                    "Layer 2 worker thread panicked while stopping.",
                    false,
                );
            }
        }
    }

    /// Returns whether Layer 2 is currently monitoring local process-start ETW events.
    pub fn is_active(&self) -> bool {
        self.active_state.load(Ordering::Acquire)
    }

    fn cleanup_finished_worker(&mut self) {
        let should_join = self
            .worker
            .as_ref()
            .and_then(|worker| worker.join_handle.as_ref())
            .is_some_and(JoinHandle::is_finished);

        if !should_join {
            return;
        }

        if let Some(mut worker) = self.worker.take() {
            if let Some(join_handle) = worker.join_handle.take() {
                let _ = join_handle.join();
            }
        }
    }
}

impl Drop for LocalShutdownBlocker {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl Default for LocalShutdownBlocker {
    fn default() -> Self {
        Self {
            worker: None,
            active_state: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct LocalShutdownWorker {
    session_name: String,
    process_handle: SharedProcessTraceHandle,
    join_handle: Option<JoinHandle<()>>,
}

impl LocalShutdownWorker {
    fn cancel_trace_processing(&self) -> Result<(), String> {
        let _ = self.process_handle.close()?;
        Ok(())
    }
}

struct ActiveStateGuard(Arc<AtomicBool>);

impl ActiveStateGuard {
    fn activate(active_state: Arc<AtomicBool>) -> Self {
        active_state.store(true, Ordering::Release);
        Self(active_state)
    }
}

impl Drop for ActiveStateGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct LocalTraceSubscription {
    session_name: String,
    _session_name_wide: Vec<u16>,
    process_handle: SharedProcessTraceHandle,
    _logfile: EVENT_TRACE_LOGFILEW,
}

unsafe impl Send for LocalTraceSubscription {}

impl LocalTraceSubscription {
    fn start(session_name: String) -> Result<Self, String> {
        let mut properties = TracePropertiesBuffer::new(&session_name);
        let mut session_handle = CONTROLTRACE_HANDLE::default();
        let session_name_wide = session_name_to_wide(&session_name);
        let start_status = unsafe {
            StartTraceW(
                &mut session_handle,
                PCWSTR(session_name_wide.as_ptr()),
                properties.as_mut_ptr(),
            )
        };
        if start_status != ERROR_SUCCESS {
            return Err(format_start_trace_error(start_status));
        }

        let enable_status = unsafe {
            EnableTraceEx2(
                session_handle,
                &KERNEL_PROCESS_PROVIDER_GUID,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                4,
                LAYER2_PROCESS_PROVIDER_KEYWORD,
                0,
                0,
                None,
            )
        };
        if enable_status != ERROR_SUCCESS {
            let _ = stop_trace_session(&session_name);
            return Err(format!(
                "Layer 2 could not enable the Microsoft-Windows-Kernel-Process provider for ETW monitoring: {}",
                win32_error_details(enable_status)
            ));
        }

        let mut logfile = EVENT_TRACE_LOGFILEW::default();
        let mut open_session_name_wide = session_name_to_wide(&session_name);
        logfile.LoggerName = PWSTR(open_session_name_wide.as_mut_ptr());
        logfile.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(handle_event_record);
        let process_handle = unsafe { OpenTraceW(&mut logfile) };
        if process_handle.Value == u64::MAX {
            let open_error = WindowsError::from_thread();
            let _ = stop_trace_session(&session_name);
            return Err(format!(
                "Layer 2 could not open its real-time ETW consumer: {open_error}"
            ));
        }

        Ok(Self {
            session_name,
            _session_name_wide: open_session_name_wide,
            process_handle: SharedProcessTraceHandle::new(process_handle),
            _logfile: logfile,
        })
    }

    fn shutdown(self) -> Result<(), String> {
        let close_result = self.process_handle.close();
        let stop_result = stop_trace_session(&self.session_name);

        if let Err(error) = close_result {
            warn!("Layer 2 could not close its ETW consumer handle during cleanup: {error}");
        }

        stop_result
    }
}

fn run_worker(subscription: LocalTraceSubscription, active_state: Arc<AtomicBool>) {
    let process_handle = match subscription.process_handle.raw_handle() {
        Ok(process_handle) => process_handle,
        Err(message) => {
            warn!("{message}");
            logger::log_event("local_layer_disabled", EventSource::Local, message, false);
            let _ = stop_trace_session(&subscription.session_name);
            return;
        }
    };
    let _active_guard = ActiveStateGuard::activate(active_state);

    info!(
        "Layer 2 local shutdown monitoring is active, confirmed shutdown-abort capability, and is watching Microsoft-Windows-Kernel-Process ETW events for shutdown.exe."
    );
    logger::log_event(
        "local_layer_enabled",
        EventSource::Local,
        "Layer 2 confirmed shutdown-abort capability, started ETW process-start monitoring for local shutdown.exe launches, and will attempt AbortSystemShutdownW(None) when it detects one.",
        true,
    );

    let process_status = unsafe { ProcessTrace(&[process_handle], None, None) };
    let close_result = subscription.process_handle.close();
    let stop_result = stop_trace_session(&subscription.session_name);

    if process_status == ERROR_SUCCESS || process_status == ERROR_WMI_INSTANCE_NOT_FOUND {
        logger::log_event(
            "local_layer_disabled",
            EventSource::Local,
            "Layer 2 stopped ETW monitoring as Block mode ended.",
            true,
        );
    } else {
        let message = format!(
            "Layer 2 ETW monitoring stopped unexpectedly after ProcessTrace returned {}.",
            win32_error_details(process_status)
        );
        warn!("{message}");
        logger::log_event("local_layer_disabled", EventSource::Local, message, false);
    }

    if let Err(error) = close_result {
        warn!("Layer 2 could not close its ETW consumer handle cleanly: {error}");
    }
    if let Err(error) = stop_result {
        warn!("Layer 2 could not stop its ETW session after ProcessTrace ended: {error}");
    }
}

unsafe extern "system" fn handle_event_record(event_record: *mut EVENT_RECORD) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(event_record) = unsafe { event_record.as_ref() } {
            process_event_record(event_record);
        }
    }));
}

fn process_event_record(event_record: &EVENT_RECORD) {
    if event_record.EventHeader.ProviderId != KERNEL_PROCESS_PROVIDER_GUID {
        return;
    }

    let descriptor = event_record.EventHeader.EventDescriptor;
    if descriptor.Id != LAYER2_PROCESS_START_EVENT_ID
        || descriptor.Task != LAYER2_PROCESS_START_TASK
        || descriptor.Opcode != LAYER2_PROCESS_START_OPCODE
    {
        return;
    }

    let Some(image_name) = read_unicode_property(event_record, "ImageName") else {
        return;
    };
    let image_basename = basename_or_original(&image_name);
    if !image_basename.eq_ignore_ascii_case(SHUTDOWN_IMAGE_NAME) {
        return;
    }

    let process_id =
        read_u32_property(event_record, "ProcessID").unwrap_or(event_record.EventHeader.ProcessId);
    let parent_process_id = read_u32_property(event_record, "ParentProcessID").unwrap_or_default();

    logger::log_event(
        "local_shutdown_process_detected",
        EventSource::Local,
        format!(
            "Layer 2 detected a local shutdown.exe process start (pid={process_id}, parent_pid={parent_process_id}, image={image_basename}) and is attempting AbortSystemShutdownW(None)."
        ),
        true,
    );

    match abort_pending_shutdown() {
        ShutdownAbortOutcome::Aborted => {
            super::record_blocked_event();
            info!(
                "Layer 2 intercepted a local shutdown.exe launch and aborted a pending shutdown."
            );
            logger::log_event(
                "local_shutdown_abort_succeeded",
                EventSource::Local,
                format!(
                    "Layer 2 detected shutdown.exe (pid={process_id}, parent_pid={parent_process_id}, image={image_basename}) and AbortSystemShutdownW(None) aborted a pending shutdown."
                ),
                true,
            );
        }
        ShutdownAbortOutcome::NoShutdownPending => {
            logger::log_event(
                "local_shutdown_abort_not_needed",
                EventSource::Local,
                format!(
                    "Layer 2 detected shutdown.exe (pid={process_id}, parent_pid={parent_process_id}, image={image_basename}) but AbortSystemShutdownW(None) reported that no shutdown was pending. This can happen when the local shutdown path was already past the abortable window, including fast paths such as shutdown /t 0 /f."
                ),
                false,
            );
        }
        ShutdownAbortOutcome::AccessDenied { details } => {
            warn!(
                "Layer 2 detected shutdown.exe but could not call AbortSystemShutdownW(None) because the required privilege is missing: {details}"
            );
            logger::log_event(
                "local_shutdown_abort_failed",
                EventSource::Local,
                format!(
                    "Layer 2 detected shutdown.exe (pid={process_id}, parent_pid={parent_process_id}, image={image_basename}) but AbortSystemShutdownW(None) was denied: {details}"
                ),
                false,
            );
        }
        ShutdownAbortOutcome::Failed { details } => {
            warn!(
                "Layer 2 detected shutdown.exe but AbortSystemShutdownW(None) failed unexpectedly: {details}"
            );
            logger::log_event(
                "local_shutdown_abort_failed",
                EventSource::Local,
                format!(
                    "Layer 2 detected shutdown.exe (pid={process_id}, parent_pid={parent_process_id}, image={image_basename}) but AbortSystemShutdownW(None) failed unexpectedly: {details}"
                ),
                false,
            );
        }
    }
}

fn read_unicode_property(event_record: &EVENT_RECORD, property_name: &str) -> Option<String> {
    let mut property_buffer = read_property_bytes(event_record, property_name)?;
    if property_buffer.len() < 2 {
        return None;
    }

    if property_buffer.len() % 2 != 0 {
        let _ = property_buffer.pop();
    }

    let utf16 = property_buffer
        .chunks_exact(2)
        .map(|chunk| u16::from_ne_bytes([chunk[0], chunk[1]]))
        .take_while(|value| *value != 0)
        .collect::<Vec<_>>();

    if utf16.is_empty() {
        return None;
    }

    String::from_utf16(&utf16).ok()
}

fn read_u32_property(event_record: &EVENT_RECORD, property_name: &str) -> Option<u32> {
    let property_buffer = read_property_bytes(event_record, property_name)?;
    if property_buffer.len() < size_of::<u32>() {
        return None;
    }

    Some(u32::from_ne_bytes([
        property_buffer[0],
        property_buffer[1],
        property_buffer[2],
        property_buffer[3],
    ]))
}

fn read_property_bytes(event_record: &EVENT_RECORD, property_name: &str) -> Option<Vec<u8>> {
    let property_name_wide = session_name_to_wide(property_name);
    let descriptor = PROPERTY_DATA_DESCRIPTOR {
        PropertyName: property_name_wide.as_ptr() as usize as u64,
        ArrayIndex: 0,
        Reserved: 0,
    };

    let mut property_size = 0;
    let size_status = unsafe {
        TdhGetPropertySize(
            event_record as *const EVENT_RECORD,
            None,
            &[descriptor],
            &mut property_size,
        )
    };
    if size_status != ERROR_SUCCESS.0 {
        return None;
    }

    let mut property_buffer = vec![0u8; property_size as usize];
    let read_status = unsafe {
        TdhGetProperty(
            event_record as *const EVENT_RECORD,
            None,
            &[descriptor],
            &mut property_buffer,
        )
    };
    if read_status != ERROR_SUCCESS.0 {
        return None;
    }

    Some(property_buffer)
}

fn basename_or_original(image_name: &str) -> String {
    Path::new(image_name)
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| image_name.to_string())
}

fn build_session_name() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("Wardoff-Layer2-Local-{}-{timestamp}", std::process::id())
}

fn session_name_to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn stop_trace_session(session_name: &str) -> Result<(), String> {
    let mut properties = TracePropertiesBuffer::new(session_name);
    let session_name_wide = session_name_to_wide(session_name);
    let status = unsafe {
        ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            PCWSTR(session_name_wide.as_ptr()),
            properties.as_mut_ptr(),
            EVENT_TRACE_CONTROL_STOP,
        )
    };

    if status == ERROR_SUCCESS || status == ERROR_WMI_INSTANCE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!(
            "Windows returned {} while stopping the Layer 2 ETW session.",
            win32_error_details(status)
        ))
    }
}

fn format_start_trace_error(status: WIN32_ERROR) -> String {
    if status == ERROR_ACCESS_DENIED || status == ERROR_PRIVILEGE_NOT_HELD {
        format!(
            "Layer 2 could not start ETW process monitoring because this process lacks the privileges Windows requires: {}",
            win32_error_details(status)
        )
    } else if status == ERROR_NOT_SUPPORTED {
        format!(
            "Layer 2 ETW process monitoring is not available on this Windows configuration: {}",
            win32_error_details(status)
        )
    } else if status == ERROR_ALREADY_EXISTS {
        format!(
            "Layer 2 found an unexpected existing ETW session with the same name and will stay inactive: {}",
            win32_error_details(status)
        )
    } else {
        format!(
            "Layer 2 could not start its ETW process monitoring session and will stay inactive: {}",
            win32_error_details(status)
        )
    }
}

fn win32_error_details(status: WIN32_ERROR) -> String {
    format!(
        "{} (code {})",
        WindowsError::from_hresult(HRESULT::from_win32(status.0)),
        status.0
    )
}

#[derive(Clone)]
struct SharedProcessTraceHandle(Arc<Mutex<Option<PROCESSTRACE_HANDLE>>>);

impl SharedProcessTraceHandle {
    fn new(process_handle: PROCESSTRACE_HANDLE) -> Self {
        Self(Arc::new(Mutex::new(Some(process_handle))))
    }

    fn raw_handle(&self) -> Result<PROCESSTRACE_HANDLE, String> {
        self.0
            .lock()
            .map_err(|_| "Layer 2 ETW consumer handle lock was poisoned.".to_string())?
            .as_ref()
            .copied()
            .ok_or_else(|| "Layer 2 ETW consumer handle was already closed.".to_string())
    }

    fn close(&self) -> Result<bool, String> {
        let process_handle = self
            .0
            .lock()
            .map_err(|_| "Layer 2 ETW consumer handle lock was poisoned.".to_string())?
            .take();

        let Some(process_handle) = process_handle else {
            return Ok(false);
        };

        let close_status = unsafe { CloseTrace(process_handle) };
        if close_status == ERROR_SUCCESS || close_status == ERROR_WMI_INSTANCE_NOT_FOUND {
            Ok(true)
        } else {
            Err(format!(
                "Windows returned {} while closing the Layer 2 ETW consumer handle.",
                win32_error_details(close_status)
            ))
        }
    }
}

struct TracePropertiesBuffer {
    buffer: Vec<u8>,
}

impl TracePropertiesBuffer {
    fn new(session_name: &str) -> Self {
        let session_name_wide = session_name_to_wide(session_name);
        let total_size =
            size_of::<EVENT_TRACE_PROPERTIES>() + session_name_wide.len() * size_of::<u16>();
        let mut buffer = vec![0u8; total_size];
        let properties = unsafe { &mut *(buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
        *properties = EVENT_TRACE_PROPERTIES::default();
        properties.Wnode.BufferSize = total_size as u32;
        properties.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        properties.Wnode.ClientContext = 1;
        properties.BufferSize = 64;
        properties.MinimumBuffers = 2;
        properties.MaximumBuffers = 4;
        properties.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        properties.FlushTimer = 1;
        properties.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;

        let name_offset = properties.LoggerNameOffset as usize;
        let name_bytes_len = session_name_wide.len() * size_of::<u16>();
        let name_bytes = unsafe {
            std::slice::from_raw_parts(session_name_wide.as_ptr() as *const u8, name_bytes_len)
        };
        buffer[name_offset..name_offset + name_bytes_len].copy_from_slice(name_bytes);

        Self { buffer }
    }

    fn as_mut_ptr(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES
    }
}
