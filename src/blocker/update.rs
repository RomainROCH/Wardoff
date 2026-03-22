#![allow(dead_code)]

use crate::logger::{self, EventSource};
use log::{error, info, warn};
use std::mem::size_of;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{Error as WindowsError, Result as WindowsResult, BSTR, HRESULT};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    E_ACCESSDENIED, HANDLE, VARIANT_FALSE, VARIANT_TRUE,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::TaskScheduler::{
    IRegisteredTask, ITaskFolder, ITaskService, TaskScheduler as TASK_SCHEDULER_CLSID,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::VARIANT;

const UPDATE_ORCHESTRATOR_FOLDER_PATH: &str = "\\Microsoft\\Windows\\UpdateOrchestrator";
const UPDATE_ORCHESTRATOR_REBOOT_TASK_NAME: &str = "Reboot";
const UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH: &str =
    "\\Microsoft\\Windows\\UpdateOrchestrator\\Reboot";
const LAYER3_THREAD_NAME: &str = "wardoff-layer3-update-reboot";

/// Coordinates Layer 3 protection for the UpdateOrchestrator reboot task.
pub struct UpdateRebootBlocker {
    worker: Option<UpdateRebootWorker>,
    active_state: Arc<AtomicBool>,
}

impl UpdateRebootBlocker {
    /// Starts Layer 3 protection while Wardoff remains in Block mode.
    pub fn start_blocking() -> Self {
        let mut blocker = Self::default();
        if let Err(error) = blocker.activate() {
            warn!("{error}");
        }
        blocker
    }

    /// Starts Layer 3 protection if it is not already active.
    pub fn activate(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Ok(());
        }

        match is_process_elevated() {
            Ok(true) => {}
            Ok(false) => {
                warn!(
                    "Layer 3 requires administrator rights; skipping UpdateOrchestrator reboot-task protection."
                );
                logger::log_event(
                    "update_monitoring_started",
                    EventSource::UpdateOrchestrator,
                    format!(
                        "Layer 3 skipped scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} because administrator rights are required."
                    ),
                    false,
                );
                return Ok(());
            }
            Err(error) => {
                warn!(
                    "Layer 3 could not determine whether the process is elevated: {error}. Skipping UpdateOrchestrator reboot-task protection."
                );
                logger::log_event(
                    "update_monitoring_started",
                    EventSource::UpdateOrchestrator,
                    format!(
                        "Layer 3 could not determine whether administrator rights are available before monitoring {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                    ),
                    false,
                );
                return Ok(());
            }
        }

        let (stop_tx, stop_rx) = mpsc::channel();
        let active_state = Arc::clone(&self.active_state);

        match thread::Builder::new()
            .name(LAYER3_THREAD_NAME.to_string())
            .spawn(move || run_worker(active_state, stop_rx))
        {
            Ok(join_handle) => {
                self.worker = Some(UpdateRebootWorker {
                    stop_tx,
                    join_handle: Some(join_handle),
                });
                Ok(())
            }
            Err(error) => Err(format!(
                "Layer 3 could not start its polling thread: {error}"
            )),
        }
    }

    /// Stops Layer 3 polling and restores the reboot task when this process changed it.
    pub fn deactivate(&mut self) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };

        self.active_state.store(false, Ordering::Release);
        let _ = worker.stop_tx.send(());

        if let Some(join_handle) = worker.join_handle.take() {
            if join_handle.join().is_err() {
                error!("Layer 3 worker thread panicked while stopping");
                logger::log_event(
                    "update_monitoring_stopped",
                    EventSource::UpdateOrchestrator,
                    "Layer 3 worker thread panicked while stopping.",
                    false,
                );
            }
        }
    }

    /// Returns whether Layer 3 is currently monitoring UpdateOrchestrator.
    pub fn is_active(&self) -> bool {
        self.active_state.load(Ordering::Acquire)
    }
}

impl Drop for UpdateRebootBlocker {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl Default for UpdateRebootBlocker {
    fn default() -> Self {
        Self {
            worker: None,
            active_state: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Initializes the COM apartment required for Task Scheduler access.
pub fn initialize_com_apartment() -> WindowsResult<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
}

/// Connects to the Task Scheduler service used for the UpdateOrchestrator reboot task.
pub fn connect_task_service() -> WindowsResult<ITaskService> {
    let task_service: ITaskService =
        unsafe { CoCreateInstance(&TASK_SCHEDULER_CLSID, None, CLSCTX_INPROC_SERVER)? };
    let empty = VARIANT::default();
    unsafe {
        task_service.Connect(&empty, &empty, &empty, &empty)?;
    }
    Ok(task_service)
}

/// Opens the UpdateOrchestrator task folder that contains the reboot task.
pub fn open_update_orchestrator_folder() -> WindowsResult<ITaskFolder> {
    let task_service = connect_task_service()?;
    let folder_path = BSTR::from(UPDATE_ORCHESTRATOR_FOLDER_PATH);
    unsafe { task_service.GetFolder(&folder_path) }
}

/// Returns the interval used to re-check the reboot task state while blocking is active.
pub fn recheck_interval() -> Duration {
    Duration::from_secs(5 * 60)
}

struct UpdateRebootWorker {
    stop_tx: Sender<()>,
    join_handle: Option<JoinHandle<()>>,
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

#[derive(Default)]
struct RestoreState {
    original_state_recorded: bool,
    restore_enabled_on_exit: bool,
}

enum RebootTaskLookup {
    Task(IRegisteredTask),
    MissingFolder,
    MissingTask,
}

struct ComApartmentGuard;

impl ComApartmentGuard {
    fn initialize() -> WindowsResult<Self> {
        initialize_com_apartment()?;
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

fn run_worker(active_state: Arc<AtomicBool>, stop_rx: Receiver<()>) {
    let _com_apartment = match ComApartmentGuard::initialize() {
        Ok(guard) => guard,
        Err(error) => {
            warn!(
                "Layer 3 could not initialize the COM apartment for Task Scheduler access: {error}. Skipping UpdateOrchestrator reboot-task protection."
            );
            logger::log_event(
                "update_monitoring_started",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 could not initialize the COM apartment before monitoring {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                ),
                false,
            );
            return;
        }
    };
    let mut restore_state = RestoreState::default();

    match ensure_reboot_task_disabled(&mut restore_state) {
        Ok(true) => {
            info!(
                "Layer 3 will re-check scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} every 5 minutes while Block mode is active."
            );
            logger::log_event(
                "update_monitoring_started",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 started monitoring scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} every {} seconds while Block mode is active.",
                    recheck_interval().as_secs()
                ),
                true,
            );
        }
        Ok(false) => return,
        Err(error) if is_access_denied_error(&error) => {
            warn!(
                "Layer 3 could not access scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}. Skipping UpdateOrchestrator reboot-task protection."
            );
            logger::log_event(
                "update_monitoring_started",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 could not access scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                ),
                false,
            );
            return;
        }
        Err(error) => {
            warn!(
                "Layer 3 could not query scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}. Skipping UpdateOrchestrator reboot-task protection."
            );
            logger::log_event(
                "update_monitoring_started",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 could not query scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                ),
                false,
            );
            return;
        }
    }

    let active_guard = ActiveStateGuard::activate(active_state);
    let mut stopped_cleanly = true;
    loop {
        match stop_rx.recv_timeout(recheck_interval()) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                match ensure_reboot_task_disabled(&mut restore_state) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) if is_access_denied_error(&error) => {
                        warn!(
                            "Layer 3 lost access to scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}. Stopping Layer 3 polling."
                        );
                        logger::log_event(
                            "update_monitoring_stopped",
                            EventSource::UpdateOrchestrator,
                            format!(
                                "Layer 3 lost access to scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                            ),
                            false,
                        );
                        stopped_cleanly = false;
                        break;
                    }
                    Err(error) => {
                        warn!(
                            "Layer 3 polling stopped after a Task Scheduler error for {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                        );
                        logger::log_event(
                            "update_monitoring_stopped",
                            EventSource::UpdateOrchestrator,
                            format!(
                                "Layer 3 polling stopped after a Task Scheduler error for {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                            ),
                            false,
                        );
                        stopped_cleanly = false;
                        break;
                    }
                }
            }
        }
    }

    drop(active_guard);
    restore_task_if_needed(&restore_state);

    if stopped_cleanly {
        logger::log_event(
            "update_monitoring_stopped",
            EventSource::UpdateOrchestrator,
            format!(
                "Layer 3 stopped monitoring scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} as Block mode ended."
            ),
            true,
        );
    }
}

fn ensure_reboot_task_disabled(restore_state: &mut RestoreState) -> WindowsResult<bool> {
    match lookup_reboot_task()? {
        RebootTaskLookup::Task(task) => {
            let enabled = task_is_enabled(&task)?;

            if !restore_state.original_state_recorded {
                restore_state.original_state_recorded = true;

                if enabled {
                    set_task_enabled(&task, false)?;
                    restore_state.restore_enabled_on_exit = true;
                    info!(
                        "Layer 3 disabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} while Block mode is active."
                    );
                    logger::log_event(
                        "update_task_disabled",
                        EventSource::UpdateOrchestrator,
                        format!(
                            "Layer 3 disabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} while Block mode is active."
                        ),
                        true,
                    );
                } else {
                    info!(
                        "Layer 3 found scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} already disabled; monitoring it without changing its original state."
                    );
                    logger::log_event(
                        "update_task_disabled",
                        EventSource::UpdateOrchestrator,
                        format!(
                            "Layer 3 found scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} already disabled and is only monitoring it."
                        ),
                        true,
                    );
                }

                return Ok(true);
            }

            if enabled {
                set_task_enabled(&task, false)?;
                if restore_state.restore_enabled_on_exit {
                    info!(
                        "Layer 3 re-disabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} after Windows re-enabled it."
                    );
                    logger::log_event(
                        "update_task_disabled",
                        EventSource::UpdateOrchestrator,
                        format!(
                            "Layer 3 re-disabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} after Windows re-enabled it."
                        ),
                        true,
                    );
                } else {
                    info!(
                        "Layer 3 disabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} after it was re-enabled during Block mode; the task will remain disabled when this process exits because it was already disabled before Layer 3 started."
                    );
                    logger::log_event(
                        "update_task_disabled",
                        EventSource::UpdateOrchestrator,
                        format!(
                            "Layer 3 disabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} after it was re-enabled during Block mode; it will remain disabled on exit because it was already disabled before Layer 3 started."
                        ),
                        true,
                    );
                }
            }

            Ok(true)
        }
        RebootTaskLookup::MissingFolder => {
            info!(
                "Layer 3 skipped because the Task Scheduler folder {UPDATE_ORCHESTRATOR_FOLDER_PATH} does not exist on this machine."
            );
            logger::log_event(
                "update_monitoring_started",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 skipped because the Task Scheduler folder {UPDATE_ORCHESTRATOR_FOLDER_PATH} does not exist on this machine."
                ),
                false,
            );
            Ok(false)
        }
        RebootTaskLookup::MissingTask => {
            info!(
                "Layer 3 skipped because the scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} does not exist on this machine."
            );
            logger::log_event(
                "update_monitoring_started",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 skipped because the scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} does not exist on this machine."
                ),
                false,
            );
            Ok(false)
        }
    }
}

fn restore_task_if_needed(restore_state: &RestoreState) {
    if !restore_state.restore_enabled_on_exit {
        return;
    }

    match lookup_reboot_task() {
        Ok(RebootTaskLookup::Task(task)) => match task_is_enabled(&task) {
            Ok(true) => {
                info!(
                    "Layer 3 left scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} enabled as Block mode ended."
                );
                logger::log_event(
                    "update_task_enabled",
                    EventSource::UpdateOrchestrator,
                    format!(
                        "Layer 3 left scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} enabled as Block mode ended."
                    ),
                    true,
                );
            }
            Ok(false) => {
                if let Err(error) = set_task_enabled(&task, true) {
                    warn!(
                        "Layer 3 could not re-enable scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                    );
                    logger::log_event(
                        "update_task_enabled",
                        EventSource::UpdateOrchestrator,
                        format!(
                            "Layer 3 could not re-enable scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                        ),
                        false,
                    );
                } else {
                    info!(
                        "Layer 3 re-enabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} as Block mode ended."
                    );
                    logger::log_event(
                        "update_task_enabled",
                        EventSource::UpdateOrchestrator,
                        format!(
                            "Layer 3 re-enabled scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} as Block mode ended."
                        ),
                        true,
                    );
                }
            }
            Err(error) => {
                warn!(
                    "Layer 3 could not query scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} during restore: {error}"
                );
                logger::log_event(
                    "update_task_enabled",
                    EventSource::UpdateOrchestrator,
                    format!(
                        "Layer 3 could not query scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} during restore: {error}"
                    ),
                    false,
                );
            }
        },
        Ok(RebootTaskLookup::MissingFolder) => {
            info!(
                "Layer 3 could not restore {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} because the folder {UPDATE_ORCHESTRATOR_FOLDER_PATH} is not present."
            );
            logger::log_event(
                "update_task_enabled",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 could not restore {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} because the folder {UPDATE_ORCHESTRATOR_FOLDER_PATH} is not present."
                ),
                false,
            );
        }
        Ok(RebootTaskLookup::MissingTask) => {
            info!(
                "Layer 3 could not restore {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} because the task is not present."
            );
            logger::log_event(
                "update_task_enabled",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 could not restore {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH} because the task is not present."
                ),
                false,
            );
        }
        Err(error) => {
            warn!(
                "Layer 3 could not restore scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
            );
            logger::log_event(
                "update_task_enabled",
                EventSource::UpdateOrchestrator,
                format!(
                    "Layer 3 could not restore scheduled task {UPDATE_ORCHESTRATOR_REBOOT_TASK_PATH}: {error}"
                ),
                false,
            );
        }
    }
}

fn lookup_reboot_task() -> WindowsResult<RebootTaskLookup> {
    let folder = match open_update_orchestrator_folder() {
        Ok(folder) => folder,
        Err(error) if is_not_found_error(&error) => return Ok(RebootTaskLookup::MissingFolder),
        Err(error) => return Err(error),
    };

    let task_name = BSTR::from(UPDATE_ORCHESTRATOR_REBOOT_TASK_NAME);
    match unsafe { folder.GetTask(&task_name) } {
        Ok(task) => Ok(RebootTaskLookup::Task(task)),
        Err(error) if is_not_found_error(&error) => Ok(RebootTaskLookup::MissingTask),
        Err(error) => Err(error),
    }
}

fn set_task_enabled(task: &IRegisteredTask, enabled: bool) -> WindowsResult<()> {
    let desired_state = if enabled { VARIANT_TRUE } else { VARIANT_FALSE };

    unsafe { task.SetEnabled(desired_state) }
}

fn task_is_enabled(task: &IRegisteredTask) -> WindowsResult<bool> {
    unsafe { Ok(task.Enabled()? != VARIANT_FALSE) }
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

fn is_not_found_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0)
        || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
        || code == HRESULT::from_win32(ERROR_NOT_FOUND.0)
}

#[allow(dead_code)]
fn is_access_denied_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == E_ACCESSDENIED || code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
}
