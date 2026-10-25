//! Process dispatch and effectful runtime orchestration.
//!
//! Start at `run_main` for CLI dispatch, `bootstrap`/`run` for lifecycle, and
//! `Application` for IPC/tray effects. Pure decisions live in `runtime_policy`.
//! See docs/DEVELOPMENT.md for task-specific entrypoints and validation.

mod autostart;
mod blocker;
mod cli;
mod diagnostics;
mod diagnostics_probe;
mod instance;
mod ipc;
mod logger;
mod power_intent;
mod runtime_policy;
mod session_owner;
mod session_scope;
mod tray;
mod windows_util;

use crate::blocker::{
    create_blocker_coordinator, execute_power_action, BlockerCoordinator, BlockerMode, PowerAction,
};
use crate::cli::{parse_cli, RequestedAction, StatusOutput};
use crate::instance::{
    claim_primary_instance, claim_primary_instance_with_retry, InstanceClaim, InstanceGuard,
};
use crate::ipc::{
    read_status, send_request, send_status_request_with_connect_deadline, ClientError, IpcRequest,
    IpcResponse, IpcServer, PendingRequest, IPC_WAKE_MESSAGE,
};
use crate::logger::EventSource;
use crate::power_intent::PowerIntent;
use crate::runtime_policy::sessions::{OwnerKey, SessionLedger};
use crate::runtime_policy::{
    control_error_message, ipc_request_for, pending_wake_restore_for_tray_power_action,
    pending_wake_restore_ready_for_resume, resolved_autostart_state_for_tray, runtime_options_for,
    session_maintenance_wait_ms, session_protection_target, PendingWakeRestore, RuntimeOptions,
    TraySurface,
};
use crate::session_owner::{elapsed_millis, SessionOwner};
use crate::tray::{
    spawn_tray_service, TrayAction, TrayServiceHandle, TrayVisibility, TRAY_ACTION_WAKE_MESSAGE,
};
use crate::windows_util::{
    show_fatal_error_dialog, ConsoleLaunchContext, ElevationLaunchResult,
    INTERNAL_DETACHED_RUNTIME_ARG, INTERNAL_ELEVATED_RELAUNCH_ARG,
};
use log::{error, info, warn};
use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsStr;
use std::io;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::WAIT_FAILED;
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, TranslateMessage, WaitMessage,
    MSG, MWMO_INPUTAVAILABLE, PM_NOREMOVE, PM_REMOVE, QS_ALLINPUT, WM_QUIT,
};

const ELEVATED_RELAUNCH_MUTEX_RETRY_ATTEMPTS: usize = 20;
const ELEVATED_RELAUNCH_MUTEX_RETRY_DELAY: Duration = Duration::from_millis(100);
const STATUS_PIPE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

// Callback bridge for the Win32 message loop on the application thread. `run`
// registers it only while its local Application is alive and clears callbacks
// before ordinary cleanup. Never dereference it from a background worker.
static ACTIVE_APPLICATION: AtomicPtr<Application> = AtomicPtr::new(std::ptr::null_mut());

/// Coordinates CLI, tray, and blocker scaffolding for the Wardoff binary.
struct Application {
    blocker_coordinator: BlockerCoordinator,
    tray_service: Option<TrayServiceHandle>,
    ipc_requests: Receiver<PendingRequest>,
    ipc_server: IpcServer,
    started_at: Instant,
    forced_shutdown_cleanup_completed: bool,
    pending_block_restore_after_wake: Option<PendingWakeRestore>,
    sessions: SessionLedger,
    session_owners: BTreeMap<OwnerKey, SessionOwner>,
    reconciliation_pending: bool,
    last_reconciliation_failure: Option<String>,
    next_session_maintenance_ms: u64,
    _primary_instance: InstanceGuard,
}

/// Bootstraps logging, the central state manager, and the tray surface.
fn bootstrap(
    primary_instance: InstanceGuard,
    options: RuntimeOptions,
) -> Result<Application, Box<dyn Error>> {
    let runtime_id = unsafe { CoCreateGuid()? }.to_u128().to_le_bytes();
    let sessions = SessionLedger::new(
        runtime_id,
        (options.initial_mode == BlockerMode::Block).then_some(options.power_intent),
    );
    logger::initialize_structured_logging().map_err(other_error)?;

    let blocker_coordinator =
        create_blocker_coordinator(options.initial_mode, options.power_intent)
            .map_err(other_error)?;
    ensure_message_queue();
    let ui_thread_id = unsafe { GetCurrentThreadId() };
    let tray_service = match options.tray_surface {
        TraySurface::Headless => None,
        TraySurface::Visible => Some(
            spawn_tray_service(
                TrayVisibility::Visible,
                blocker_coordinator.mode(),
                blocker_coordinator.power_intent(),
                ui_thread_id,
            )
            .map_err(other_error)?,
        ),
        TraySurface::Hidden => Some(
            spawn_tray_service(
                TrayVisibility::Hidden,
                blocker_coordinator.mode(),
                blocker_coordinator.power_intent(),
                ui_thread_id,
            )
            .map_err(other_error)?,
        ),
    };

    let (ipc_request_tx, ipc_requests) = mpsc::channel();
    let ipc_server = IpcServer::start(ipc_request_tx, ui_thread_id).map_err(other_error)?;

    let start_message = application_start_message(options);
    info!("{start_message}");
    logger::log_event(
        "application_started",
        EventSource::Application,
        start_message,
        true,
    );
    logger::log_event(
        "state_changed",
        EventSource::Application,
        format!(
            "Wardoff transitioned from inactive to {} mode during startup.",
            mode_label(options.initial_mode)
        ),
        true,
    );

    let mut application = Application {
        blocker_coordinator,
        tray_service,
        ipc_requests,
        ipc_server,
        started_at: Instant::now(),
        forced_shutdown_cleanup_completed: false,
        pending_block_restore_after_wake: None,
        sessions,
        session_owners: BTreeMap::new(),
        reconciliation_pending: false,
        last_reconciliation_failure: None,
        next_session_maintenance_ms: 0,
        _primary_instance: primary_instance,
    };
    application.refresh_autostart_tray_state();

    Ok(application)
}

/// Runs an operation and always runs its cleanup, preserving both failures when needed.
fn run_with_cleanup<T, Loop, Cleanup>(
    state: &mut T,
    loop_fn: Loop,
    cleanup_fn: Cleanup,
) -> Result<(), Box<dyn Error>>
where
    Loop: FnOnce(&mut T) -> Result<(), Box<dyn Error>>,
    Cleanup: FnOnce(&mut T) -> Result<(), Box<dyn Error>>,
{
    let loop_result = loop_fn(state);
    let cleanup_result = cleanup_fn(state);

    match (loop_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(loop_error), Ok(())) => Err(loop_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(loop_error), Err(cleanup_error)) => Err(Box::new(other_error(format!(
            "runtime loop failed: {loop_error}; cleanup failed: {cleanup_error}"
        )))),
    }
}

/// Runs the shared Win32 event loop for the hidden Layer 1 windows and the tray icon.
fn run(mut application: Application) -> Result<(), Box<dyn Error>> {
    ACTIVE_APPLICATION.store(&mut application as *mut Application, Ordering::Release);
    blocker::shutdown::set_end_session_cleanup_callback(forced_shutdown_cleanup_callback);
    blocker::shutdown::set_power_broadcast_callback(power_broadcast_callback);

    run_with_cleanup(&mut application, run_message_loop, |application| {
        blocker::shutdown::clear_end_session_cleanup_callback();
        blocker::shutdown::clear_power_broadcast_callback();
        ACTIVE_APPLICATION.store(std::ptr::null_mut(), Ordering::Release);
        application.shutdown()
    })
}

/// Completes a logical release independently of a recoverable cleanup failure.
fn finish_session_release(
    logical: Result<bool, String>,
    reconcile: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if logical? {
        // The caller retains cleanup failures for diagnostics/retry; a
        // duplicate release performs no additional effects.
        let _ = reconcile();
    }
    Ok(())
}

fn run_message_loop(application: &mut Application) -> Result<(), Box<dyn Error>> {
    let mut message = MSG::default();

    'message_loop: loop {
        application.poll_session_maintenance();
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            // A continuous stream of messages must not starve lease expiry.
            application.poll_session_maintenance();
            if message.message == WM_QUIT {
                break 'message_loop;
            }

            if message.message == IPC_WAKE_MESSAGE || message.message == TRAY_ACTION_WAKE_MESSAGE {
                application.process_ipc_requests();
                if application.process_tray_actions() {
                    break 'message_loop;
                }
                continue;
            }

            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            application.process_ipc_requests();
            if application.process_tray_actions() {
                break 'message_loop;
            }
        }

        application.process_ipc_requests();
        if application.process_tray_actions() {
            break;
        }

        if let Some(timeout_ms) = session_maintenance_wait_ms(
            application.sessions.has_entries() || application.reconciliation_pending,
            application.next_session_maintenance_ms,
            elapsed_millis(),
        ) {
            // The wait is only a wake-up mechanism. Expiry uses suspend-inclusive
            // absolute GetTickCount64 deadlines and is swept on resume as well.
            let result = unsafe {
                MsgWaitForMultipleObjectsEx(None, timeout_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
            };
            if result == WAIT_FAILED {
                return Err(io::Error::last_os_error().into());
            }
        } else {
            unsafe { WaitMessage()? };
        }
    }
    Ok(())
}

fn main() {
    match run_main() {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            if should_show_graphical_startup_error() {
                show_fatal_error_dialog(
                    "Wardoff failed to start",
                    &format!("Wardoff failed to start:\n\n{error}"),
                );
            }
            if logger::structured_logging_initialized() {
                logger::log_event(
                    "application_start_failed",
                    EventSource::Application,
                    format!("Wardoff failed to start: {error}"),
                    false,
                );
                logger::shutdown_structured_logging();
            }
            error!("Wardoff failed to start: {error}");
            eprintln!("wardoff failed: {error}");
            std::process::exit(1);
        }
    }
}

fn run_main() -> Result<i32, Box<dyn Error>> {
    // An Owned console means Windows allocated a new console for this process
    // (typically a double-click or an autostart spawn). Hide and free it before
    // anything else so the console window does not flash before the runtime
    // bootstrap path reaches its existing late hide in prepare_runtime_console_launch.
    // Inherited consoles are left intact so read-only CLI commands still write
    // to the user's shell.
    if matches!(
        windows_util::console_launch_context(),
        Ok(ConsoleLaunchContext::Owned)
    ) {
        let _ = windows_util::hide_and_free_console();
    }

    logger::initialize_human_logging()?;
    let cli = parse_cli();
    let action = cli.requested_action();

    if action.is_read_only() {
        return handle_read_only_request(action);
    }

    match action {
        RequestedAction::Autostart { enabled } => handle_autostart_request(enabled),
        action @ (RequestedAction::Default
        | RequestedAction::Block
        | RequestedAction::BlockWithPowerIntent { .. }
        | RequestedAction::Allow
        | RequestedAction::Hide) => handle_runtime_request(
            action,
            cli.is_internal_elevated_relaunch(),
            cli.is_internal_detached_runtime(),
        ),
        RequestedAction::Status
        | RequestedAction::Log { .. }
        | RequestedAction::Doctor { .. }
        | RequestedAction::DiagnosticsProbe => {
            unreachable!("read-only CLI actions are handled before runtime and elevation dispatch")
        }
    }
}

fn handle_read_only_request(action: RequestedAction) -> Result<i32, Box<dyn Error>> {
    match action {
        RequestedAction::Status => handle_status_request(),
        RequestedAction::Log { tail } => handle_log_request(tail),
        RequestedAction::Doctor { json } => handle_doctor_request(json),
        RequestedAction::DiagnosticsProbe => {
            diagnostics_probe::emit().map_err(|error| other_error(error).into())
        }
        _ => {
            unreachable!("only read-only CLI actions should reach the non-elevating dispatch path")
        }
    }
}

fn handle_doctor_request(json: bool) -> Result<i32, Box<dyn Error>> {
    use diagnostics::{build_report, ReportInput, RuntimeMode};
    let deadline = Instant::now() + STATUS_PIPE_DISCOVERY_TIMEOUT;
    let (mode, legacy_api, snapshot) = match instance::primary_instance_exists() {
        Ok(false) => (RuntimeMode::Inactive, None, None),
        Ok(true) => match ipc::read_diagnostic_status(deadline) {
            Ok(status) => status.diagnostic_input(),
            Err(_) => (RuntimeMode::Unreachable, None, None),
        },
        Err(_) => (RuntimeMode::Unreachable, None, None),
    };
    let local = diagnostics_probe::inspect(deadline);
    let report = build_report(ReportInput {
        mode,
        legacy_api,
        snapshot,
        context: local.context,
        recent_failures: local.recent_failures,
    });
    if json {
        println!("{}", report.to_json()?);
    } else {
        println!("{}", report.human_text());
    }
    Ok(report.exit_code())
}

fn handle_runtime_request(
    action: RequestedAction,
    internal_elevated_relaunch: bool,
    internal_detached_runtime: bool,
) -> Result<i32, Box<dyn Error>> {
    let instance_claim = if action == RequestedAction::Default && internal_elevated_relaunch {
        claim_primary_instance_with_retry(
            ELEVATED_RELAUNCH_MUTEX_RETRY_ATTEMPTS,
            ELEVATED_RELAUNCH_MUTEX_RETRY_DELAY,
        )
        .map_err(other_error)?
    } else {
        claim_primary_instance().map_err(other_error)?
    };

    match instance_claim {
        InstanceClaim::Primary(primary_instance) => handle_primary_runtime_request(
            action,
            internal_elevated_relaunch,
            internal_detached_runtime,
            primary_instance,
        ),
        InstanceClaim::Secondary => {
            forward_request_to_primary(ipc_request_for(action), |primary_instance| {
                handle_primary_runtime_request(
                    action,
                    internal_elevated_relaunch,
                    internal_detached_runtime,
                    primary_instance,
                )
            })
        }
    }
}

fn handle_status_request() -> Result<i32, Box<dyn Error>> {
    match claim_primary_instance().map_err(other_error)? {
        InstanceClaim::Primary(primary_instance) => {
            drop(primary_instance);
            println!("{}", StatusOutput::inactive().to_json()?);
            Ok(1)
        }
        InstanceClaim::Secondary => handle_secondary_status_request(),
    }
}

fn handle_secondary_status_request() -> Result<i32, Box<dyn Error>> {
    // Give the read-only pipe its startup grace, then share the remaining discovery
    // budget with the legacy control fallback. Connected reply reads are unchanged.
    let connect_deadline = Instant::now() + STATUS_PIPE_DISCOVERY_TIMEOUT;
    match read_status(Some(connect_deadline)) {
        Ok(status) => {
            println!("{}", status.to_json()?);
            Ok(0)
        }
        Err(ClientError::Unavailable) => {
            handle_secondary_status_request_via_control_pipe(connect_deadline)
        }
        Err(ClientError::Transport(message)) => Err(Box::new(other_error(message))),
    }
}

fn handle_secondary_status_request_via_control_pipe(
    connect_deadline: Instant,
) -> Result<i32, Box<dyn Error>> {
    match send_status_request_with_connect_deadline(connect_deadline) {
        Ok(IpcResponse::Status { status }) => {
            println!("{}", status.to_json()?);
            Ok(0)
        }
        Ok(IpcResponse::Error { message }) => Err(Box::new(other_error(message))),
        Ok(
            IpcResponse::Ok | IpcResponse::SessionReady { .. } | IpcResponse::SessionActive { .. },
        ) => Err(Box::new(other_error(
            "Wardoff received an unexpected empty response for --status.".to_string(),
        ))),
        Err(ClientError::Unavailable) => Err(Box::new(other_error(
            active_primary_pipe_unavailable_message(),
        ))),
        Err(ClientError::Transport(message)) => Err(Box::new(other_error(message))),
    }
}

fn handle_log_request(tail: usize) -> Result<i32, Box<dyn Error>> {
    for line in logger::read_recent_lines(tail).map_err(other_error)? {
        println!("{line}");
    }

    Ok(0)
}

fn handle_autostart_request(enabled: bool) -> Result<i32, Box<dyn Error>> {
    match claim_primary_instance().map_err(other_error)? {
        InstanceClaim::Primary(primary_instance) => {
            handle_local_autostart_request(enabled, primary_instance)
        }
        InstanceClaim::Secondary => {
            forward_request_to_primary(IpcRequest::SetAutostart { enabled }, |primary_instance| {
                handle_local_autostart_request(enabled, primary_instance)
            })
        }
    }
}

fn handle_primary_runtime_request(
    action: RequestedAction,
    internal_elevated_relaunch: bool,
    internal_detached_runtime: bool,
    primary_instance: InstanceGuard,
) -> Result<i32, Box<dyn Error>> {
    let primary_instance = match action {
        RequestedAction::Default => {
            match prepare_default_launch(primary_instance, internal_elevated_relaunch)? {
                DefaultLaunchDisposition::Bootstrap(primary_instance) => primary_instance,
                DefaultLaunchDisposition::Exit(exit_code) => return Ok(exit_code),
            }
        }
        _ => primary_instance,
    };

    match prepare_runtime_console_launch(action, internal_detached_runtime)? {
        RuntimeConsoleDisposition::Bootstrap => {}
        RuntimeConsoleDisposition::Exit(exit_code) => return Ok(exit_code),
    }

    let application = bootstrap(primary_instance, runtime_options_for(action))?;
    run(application)?;
    Ok(0)
}

fn handle_local_autostart_request(
    enabled: bool,
    primary_instance: InstanceGuard,
) -> Result<i32, Box<dyn Error>> {
    drop(primary_instance);

    let mut structured_logging_started = false;
    match logger::initialize_structured_logging() {
        Ok(()) => structured_logging_started = true,
        Err(error) => {
            error!("Wardoff could not initialize structured logging for --autostart: {error}");
        }
    }

    let result = autostart::set_enabled(enabled)
        .map_err(other_error)
        .map(|_| 0);
    if structured_logging_started {
        logger::shutdown_structured_logging();
    }

    Ok(result?)
}

enum DefaultLaunchDisposition {
    Bootstrap(InstanceGuard),
    Exit(i32),
}

enum RuntimeConsoleDisposition {
    Bootstrap,
    Exit(i32),
}

fn prepare_default_launch(
    primary_instance: InstanceGuard,
    internal_elevated_relaunch: bool,
) -> Result<DefaultLaunchDisposition, Box<dyn Error>> {
    let is_elevated = windows_util::is_process_elevated().map_err(|error| {
        other_error(format!(
            "Wardoff could not determine whether administrator rights are available before default startup: {error}"
        ))
    })?;

    if internal_elevated_relaunch {
        if is_elevated {
            return Ok(DefaultLaunchDisposition::Bootstrap(primary_instance));
        }

        return Err(Box::new(other_error(
            "Wardoff relaunched its default startup path without administrator rights.".to_string(),
        )));
    }

    if is_elevated {
        return Ok(DefaultLaunchDisposition::Bootstrap(primary_instance));
    }

    match windows_util::relaunch_self_elevated().map_err(other_error)? {
        ElevationLaunchResult::Launched => {
            info!("Wardoff requested administrator rights for its default startup path.");
            Ok(DefaultLaunchDisposition::Exit(0))
        }
        ElevationLaunchResult::Cancelled => {
            info!("Wardoff default startup was canceled at the UAC prompt.");
            Ok(DefaultLaunchDisposition::Exit(1))
        }
    }
}

fn prepare_runtime_console_launch(
    action: RequestedAction,
    internal_detached_runtime: bool,
) -> Result<RuntimeConsoleDisposition, Box<dyn Error>> {
    match windows_util::console_launch_context().map_err(other_error)? {
        ConsoleLaunchContext::None => Ok(RuntimeConsoleDisposition::Bootstrap),
        ConsoleLaunchContext::Owned => {
            windows_util::hide_and_free_console().map_err(other_error)?;
            Ok(RuntimeConsoleDisposition::Bootstrap)
        }
        ConsoleLaunchContext::Inherited if internal_detached_runtime => {
            Ok(RuntimeConsoleDisposition::Bootstrap)
        }
        ConsoleLaunchContext::Inherited => {
            windows_util::relaunch_self_detached().map_err(other_error)?;
            info!(
                "Wardoff detached its {} runtime launch from the current shell console.",
                runtime_request_label(action)
            );
            Ok(RuntimeConsoleDisposition::Exit(0))
        }
    }
}

fn active_primary_pipe_unavailable_message() -> String {
    "Wardoff found an active primary instance, but its control pipe is unavailable. The primary instance may still be starting up or shutting down.".to_string()
}

fn forward_request_to_primary<F>(
    request: IpcRequest,
    local_fallback: F,
) -> Result<i32, Box<dyn Error>>
where
    F: FnOnce(InstanceGuard) -> Result<i32, Box<dyn Error>>,
{
    match send_request(&request) {
        Ok(IpcResponse::Ok) => Ok(0),
        Ok(IpcResponse::Error { message }) => Err(Box::new(other_error(control_error_message(
            &request, message,
        )))),
        Ok(
            IpcResponse::Status { .. }
            | IpcResponse::SessionReady { .. }
            | IpcResponse::SessionActive { .. },
        ) => Err(Box::new(other_error(
            "Wardoff received a status payload for a control command.".to_string(),
        ))),
        Err(ClientError::Unavailable) => match claim_primary_instance().map_err(other_error)? {
            InstanceClaim::Primary(primary_instance) => local_fallback(primary_instance),
            InstanceClaim::Secondary => Err(Box::new(other_error(
                active_primary_pipe_unavailable_message(),
            ))),
        },
        Err(ClientError::Transport(message)) => Err(Box::new(other_error(message))),
    }
}

fn ensure_message_queue() {
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
}

fn application_start_message(options: RuntimeOptions) -> String {
    format!(
        "Wardoff started in {} with {}.",
        configuration_label(options.initial_mode, options.power_intent),
        tray_surface_label(options.tray_surface)
    )
}

fn tray_surface_label(surface: TraySurface) -> &'static str {
    match surface {
        TraySurface::Visible => "a visible tray icon",
        TraySurface::Hidden => "a hidden tray icon",
        TraySurface::Headless => "no tray icon",
    }
}

impl Application {
    fn process_ipc_requests(&mut self) {
        while let Ok(pending_request) = self.ipc_requests.try_recv() {
            self.maintain_sessions();
            let response = self.handle_ipc_request(pending_request.request, pending_request.owner);
            let _ = pending_request.response_tx.send(response);
        }
    }

    fn handle_ipc_request(
        &mut self,
        request: IpcRequest,
        owner: Option<SessionOwner>,
    ) -> IpcResponse {
        if request.is_session_command() {
            return match self.handle_session_request(request, owner) {
                Ok(response) => response,
                Err(message) => IpcResponse::Error { message },
            };
        }
        match request {
            IpcRequest::SetPowerIntent { intent } => {
                self.clear_pending_block_restore_after_wake();
                match self.set_configuration(BlockerMode::Block, intent, EventSource::Ipc) {
                    Ok(()) => {
                        logger::log_event(
                            "power_intent_request_processed",
                            EventSource::Ipc,
                            format!(
                                "Wardoff applied {}.",
                                configuration_label(BlockerMode::Block, intent)
                            ),
                            true,
                        );
                        IpcResponse::Ok
                    }
                    Err(message) => {
                        logger::log_event(
                            "power_intent_request_processed",
                            EventSource::Ipc,
                            &message,
                            false,
                        );
                        IpcResponse::Error { message }
                    }
                }
            }
            IpcRequest::SetMode { mode } => {
                let requested_mode: BlockerMode = mode.into();
                self.clear_pending_block_restore_after_wake();

                match self.set_mode(requested_mode, EventSource::Ipc) {
                    Ok(()) => {
                        logger::log_event(
                            "mode_request_processed",
                            EventSource::Ipc,
                            format!(
                                "Wardoff applied a named-pipe request for {} mode.",
                                mode_label(requested_mode)
                            ),
                            true,
                        );
                        IpcResponse::Ok
                    }
                    Err(message) => {
                        logger::log_event(
                            "mode_request_processed",
                            EventSource::Ipc,
                            format!(
                                "Wardoff could not apply a named-pipe request for {} mode: {message}",
                                mode_label(requested_mode)
                            ),
                            false,
                        );
                        IpcResponse::Error { message }
                    }
                }
            }
            IpcRequest::SetAutostart { enabled } => {
                match self.set_autostart_enabled(enabled, EventSource::Ipc) {
                    Ok(()) => {
                        logger::log_event(
                            "autostart_request_processed",
                            EventSource::Ipc,
                            format!(
                            "Wardoff applied a named-pipe request to turn Start with Windows {}.",
                            on_off_label(enabled)
                        ),
                            true,
                        );
                        IpcResponse::Ok
                    }
                    Err(message) => {
                        logger::log_event(
                        "autostart_request_processed",
                        EventSource::Ipc,
                        format!(
                            "Wardoff could not apply a named-pipe request to turn Start with Windows {}: {message}",
                            on_off_label(enabled)
                        ),
                        false,
                    );
                        IpcResponse::Error { message }
                    }
                }
            }
            IpcRequest::Status => IpcResponse::Status {
                status: self.status_output(),
            },
            IpcRequest::BeginSession
            | IpcRequest::AcquireSession { .. }
            | IpcRequest::ExtendSession { .. }
            | IpcRequest::ReleaseSession { .. } => IpcResponse::Error {
                message: "Wardoff could not route a session command.".to_string(),
            },
            IpcRequest::ShutdownServer => IpcResponse::Error {
                message:
                    "Wardoff cannot route the internal shutdown request through the UI thread."
                        .to_string(),
            },
        }
    }

    fn status_output(&self) -> StatusOutput {
        let (layers, mut snapshot) = self.blocker_coordinator.capture_status(
            windows_util::is_process_elevated().ok(),
            self.pending_block_restore_after_wake.is_some(),
        );
        snapshot.manual_indefinite = self.sessions.manual().is_some();
        snapshot.sessions = Some(self.sessions.views(elapsed_millis()));
        snapshot.session_cleanup_pending = Some(self.reconciliation_pending);
        let target = session_protection_target(
            &self.sessions,
            self.pending_block_restore_after_wake.is_some(),
        );
        snapshot.power_requests.system.requested = Some(target.is_some());
        snapshot.power_requests.display.requested =
            Some(target == Some(PowerIntent::SystemAndDisplay));
        StatusOutput::active(
            self.blocker_coordinator.mode(),
            layers,
            self.started_at.elapsed().as_secs(),
            self.blocker_coordinator.blocked_count(),
        )
        .with_diagnostics(snapshot)
    }

    fn handle_session_request(
        &mut self,
        request: IpcRequest,
        owner: Option<SessionOwner>,
    ) -> Result<IpcResponse, String> {
        let owner = owner.ok_or("Wardoff session commands require an inspected process owner.")?;
        if !owner.is_alive() {
            return Err("The Wardoff session owner has exited.".to_string());
        }
        let owner_key = owner.key();
        let now = elapsed_millis();
        match request {
            IpcRequest::BeginSession => {
                let key = self.sessions.begin(owner_key, now)?;
                self.session_owners.entry(owner_key).or_insert(owner);
                // No Power Request is acquired by a pending ticket. Keep Allow
                // available in the tray so it can cancel even an unclaimed ticket.
                self.reconciliation_pending = true;
                let _ = self.reconcile_current_intent(EventSource::Ipc);
                Ok(IpcResponse::SessionReady {
                    session_protocol: 1,
                    key,
                })
            }
            IpcRequest::AcquireSession { key, spec } => {
                let mut draft = self.sessions.clone();
                draft.acquire(owner_key, key, spec, now)?;
                self.clear_pending_block_restore_after_wake();
                self.commit_intent_draft(draft, EventSource::Ipc)?;
                self.maintain_sessions();
                let session = self
                    .sessions
                    .active_view(key, elapsed_millis())
                    .ok_or("The session ended before protection could be acknowledged.")?;
                Ok(IpcResponse::SessionActive { session })
            }
            IpcRequest::ExtendSession {
                key,
                expected_revision,
                duration_ms,
            } => {
                let result =
                    self.sessions
                        .extend(owner_key, key, expected_revision, duration_ms, now);
                self.reconciliation_pending = true;
                self.maintain_sessions();
                result?;
                let session = self
                    .sessions
                    .active_view(key, elapsed_millis())
                    .ok_or("The session ended before its extension could be acknowledged.")?;
                Ok(IpcResponse::SessionActive { session })
            }
            IpcRequest::ReleaseSession { key } => {
                let logical = self.sessions.release(owner_key, key);
                finish_session_release(logical, || {
                    self.reconciliation_pending = true;
                    self.prune_sessions();
                    self.reconcile_current_intent(EventSource::Ipc)
                })?;
                Ok(IpcResponse::Ok)
            }
            _ => Err("Wardoff received a non-session command on the session handler.".to_string()),
        }
    }

    fn prune_sessions(&mut self) {
        let owners = &self.session_owners;
        if self.sessions.expire(elapsed_millis(), |key| {
            owners.get(&key).is_some_and(SessionOwner::is_alive)
        }) {
            self.reconciliation_pending = true;
        }
        let retained = self.sessions.owners();
        self.session_owners.retain(|key, _| retained.contains(key));
    }

    fn maintain_sessions(&mut self) {
        self.next_session_maintenance_ms = elapsed_millis().saturating_add(1000);
        self.prune_sessions();
        if self.reconciliation_pending {
            let _ = self.reconcile_current_intent(EventSource::Application);
        }
    }

    fn poll_session_maintenance(&mut self) {
        if (self.sessions.has_entries() || self.reconciliation_pending)
            && elapsed_millis() >= self.next_session_maintenance_ms
        {
            self.maintain_sessions();
        }
    }

    fn commit_intent_draft(
        &mut self,
        draft: SessionLedger,
        source: EventSource,
    ) -> Result<(), String> {
        let intent = session_protection_target(&draft, false);
        let mode = if intent.is_some() {
            BlockerMode::Block
        } else {
            BlockerMode::Allow
        };
        let result = self.apply_configuration(
            mode,
            intent.unwrap_or(self.blocker_coordinator.power_intent()),
            source,
            &draft,
        );
        match result {
            Ok(()) => {
                self.sessions = draft;
                self.reconciliation_pending = false;
                self.last_reconciliation_failure = None;
                Ok(())
            }
            Err(message) => {
                // A failed draft is never the committed source of truth. Retry
                // against the current ledger, including when it contains no intent.
                self.reconciliation_pending = true;
                Err(message)
            }
        }
    }

    fn reconcile_current_intent(&mut self, source: EventSource) -> Result<(), String> {
        // Recompute on every attempt. Never cache an old cleanup target across
        // a newer manual Block or newly acquired session.
        let target = session_protection_target(
            &self.sessions,
            self.pending_block_restore_after_wake.is_some(),
        );
        let mode = if target.is_some() {
            BlockerMode::Block
        } else {
            BlockerMode::Allow
        };
        let intent = target.unwrap_or(self.blocker_coordinator.power_intent());
        let effects = self.blocker_coordinator.set_configuration(mode, intent);
        let changed = matches!(&effects, Ok(true));
        let tray = self.sync_tray(self.blocker_coordinator.mode());
        let failure = match (effects, tray) {
            (Ok(_), Ok(())) => None,
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Some(error),
            (Err(error), Err(tray_error)) => Some(format!("{error}; tray: {tray_error}")),
        };
        self.reconciliation_pending = failure.is_some();
        if let Some(message) = failure {
            if self.last_reconciliation_failure.as_ref() != Some(&message) {
                logger::log_event("session_reconciliation_failed", source, &message, false);
            }
            self.last_reconciliation_failure = Some(message.clone());
            Err(message)
        } else {
            self.last_reconciliation_failure = None;
            if changed {
                logger::log_event(
                    "state_changed",
                    source,
                    format!(
                        "Wardoff reconciled {} from its current protection choices.",
                        configuration_label(mode, intent)
                    ),
                    true,
                );
            }
            Ok(())
        }
    }

    fn process_tray_actions(&mut self) -> bool {
        let actions = match self.tray_service.as_ref() {
            Some(tray_service) => tray_service.drain_actions(),
            None => return false,
        };

        for action in actions {
            match self.handle_tray_action(action) {
                Ok(true) => return true,
                Ok(false) => {}
                Err(message) => {
                    logger::log_event(
                        "tray_action",
                        EventSource::Tray,
                        format!(
                            "Wardoff could not process the {:?} tray action: {message}",
                            action
                        ),
                        false,
                    );
                    error!(
                        "Wardoff could not process tray action {:?}: {message}",
                        action
                    );
                }
            }
        }

        false
    }

    fn handle_tray_action(&mut self, action: TrayAction) -> Result<bool, String> {
        match action {
            TrayAction::Block => {
                self.clear_pending_block_restore_after_wake();
                self.set_mode(BlockerMode::Block, EventSource::Tray)?;
                logger::log_event(
                    "tray_action",
                    EventSource::Tray,
                    "Wardoff processed a Block request from the tray menu.",
                    true,
                );
                Ok(false)
            }
            TrayAction::Allow => {
                self.clear_pending_block_restore_after_wake();
                self.set_mode(BlockerMode::Allow, EventSource::Tray)?;
                logger::log_event(
                    "tray_action",
                    EventSource::Tray,
                    "Wardoff processed an Allow request from the tray menu.",
                    true,
                );
                Ok(false)
            }
            TrayAction::SetAutostart(enabled) => {
                self.set_autostart_enabled(enabled, EventSource::Tray)?;
                logger::log_event(
                    "tray_action",
                    EventSource::Tray,
                    format!(
                        "Wardoff processed a Start with Windows request to turn autostart {}.",
                        on_off_label(enabled)
                    ),
                    true,
                );
                Ok(false)
            }
            TrayAction::Shutdown => {
                self.run_power_action(PowerAction::Shutdown)?;
                Ok(false)
            }
            TrayAction::Reboot => {
                self.run_power_action(PowerAction::Reboot)?;
                Ok(false)
            }
            TrayAction::Sleep => {
                self.run_power_action(PowerAction::Sleep)?;
                Ok(false)
            }
            TrayAction::Hibernate => {
                self.run_power_action(PowerAction::Hibernate)?;
                Ok(false)
            }
            TrayAction::Quit => {
                logger::log_event(
                    "quit_requested",
                    EventSource::Tray,
                    "Wardoff received a Quit request from the tray menu.",
                    true,
                );
                Ok(true)
            }
        }
    }

    fn run_power_action(&mut self, action: PowerAction) -> Result<(), String> {
        let previous_intent = self.blocker_coordinator.power_intent();
        let restore_mode = if self.sessions.effective_intent().is_some() {
            BlockerMode::Block
        } else {
            BlockerMode::Allow
        };
        self.pending_block_restore_after_wake =
            pending_wake_restore_for_tray_power_action(restore_mode, previous_intent, action);
        // Suspend pauses effects, not ownership. Explicit Allow has different,
        // irrevocable global-cancellation semantics.
        let presentation = self.sessions.clone();
        if let Err(error) = self.apply_configuration(
            BlockerMode::Allow,
            previous_intent,
            EventSource::Tray,
            &presentation,
        ) {
            self.clear_pending_block_restore_after_wake();
            self.reconciliation_pending = true;
            return Err(error);
        }
        match execute_power_action(action) {
            Ok(()) => {
                if matches!(action, PowerAction::Shutdown | PowerAction::Reboot) {
                    self.sessions.global_stop();
                    self.session_owners.clear();
                    self.clear_pending_block_restore_after_wake();
                    self.reconciliation_pending = true;
                    let _ = self.reconcile_current_intent(EventSource::Tray);
                }
                logger::log_event(
                    "power_action_issued",
                    EventSource::Tray,
                    format!(
                        "Wardoff issued the requested {} power action after releasing protection.",
                        power_action_label(action)
                    ),
                    true,
                );
                Ok(())
            }
            Err(message) => {
                self.clear_pending_block_restore_after_wake();
                self.prune_sessions();
                self.reconciliation_pending = true;
                let restored = self.reconcile_current_intent(EventSource::Tray);
                logger::log_event(
                    "power_action_issued",
                    EventSource::Tray,
                    format!(
                        "Wardoff could not issue the requested {} power action: {message}",
                        power_action_label(action)
                    ),
                    false,
                );
                match restored {
                    Ok(()) => Err(format!(
                        "{message} Wardoff reapplied its still-valid protection choices."
                    )),
                    Err(error) => Err(format!(
                        "{message} Wardoff could not reconcile its current protection: {error}"
                    )),
                }
            }
        }
    }

    fn set_mode(&mut self, mode: BlockerMode, source: EventSource) -> Result<(), String> {
        let intent = if mode == BlockerMode::Block {
            PowerIntent::default()
        } else {
            self.blocker_coordinator.power_intent()
        };
        self.set_configuration(mode, intent, source)
    }

    fn set_configuration(
        &mut self,
        mode: BlockerMode,
        intent: PowerIntent,
        source: EventSource,
    ) -> Result<(), String> {
        self.clear_pending_block_restore_after_wake();
        if mode == BlockerMode::Allow {
            // Logical cancellation precedes cleanup and cannot be rolled back by
            // a failed API or tray update. Work processes remain untouched.
            self.sessions.global_stop();
            self.session_owners.clear();
            self.reconciliation_pending = true;
            self.reconcile_current_intent(source)
        } else {
            let mut draft = self.sessions.clone();
            draft.set_manual(Some(intent));
            self.commit_intent_draft(draft, source)
        }
    }

    fn apply_configuration(
        &mut self,
        mode: BlockerMode,
        intent: PowerIntent,
        source: EventSource,
        presentation_ledger: &SessionLedger,
    ) -> Result<(), String> {
        let previous_mode = self.blocker_coordinator.mode();
        let previous_intent = self.blocker_coordinator.power_intent();
        let changed = match self.blocker_coordinator.set_configuration(mode, intent) {
            Ok(changed) => changed,
            Err(error) => {
                let error_message = format!(
                    "Wardoff could not change from {} to {}: {error}",
                    configuration_label(previous_mode, previous_intent),
                    configuration_label(mode, intent)
                );
                logger::log_event("state_changed", source, error_message.clone(), false);
                return Err(error_message);
            }
        };

        if let Err(tray_error) =
            self.sync_tray_for_ledger(self.blocker_coordinator.mode(), presentation_ledger)
        {
            let rollback_result = self
                .blocker_coordinator
                .set_configuration(previous_mode, previous_intent);
            let final_mode = self.blocker_coordinator.mode();
            let final_intent = self.blocker_coordinator.power_intent();
            let tray_resync_result = self.sync_tray(final_mode);

            let mut error_message = format!(
                "Wardoff could not update the tray UI after applying {}: {tray_error}",
                configuration_label(mode, intent)
            );

            match rollback_result {
                Ok(_) => error_message.push_str(&format!(
                    " Wardoff restored {}.",
                    configuration_label(previous_mode, previous_intent)
                )),
                Err(rollback_error) => error_message.push_str(&format!(
                    " Wardoff could not restore {} and currently requests {}: {rollback_error}",
                    configuration_label(previous_mode, previous_intent),
                    configuration_label(final_mode, final_intent)
                )),
            }

            if let Err(tray_resync_error) = tray_resync_result {
                error_message.push_str(&format!(
                    " Wardoff also could not resync the tray UI to {} mode: {tray_resync_error}",
                    mode_label(final_mode)
                ));
            }

            logger::log_event("state_changed", source, error_message.clone(), false);
            return Err(error_message);
        }

        if changed {
            logger::log_event(
                "state_changed",
                source,
                format!(
                    "Wardoff applied {} (previous choice: {}).",
                    configuration_label(mode, intent),
                    configuration_label(previous_mode, previous_intent)
                ),
                true,
            );
        }

        Ok(())
    }

    fn set_autostart_enabled(&mut self, enabled: bool, source: EventSource) -> Result<(), String> {
        let previous_state = autostart::is_enabled().ok();

        match autostart::set_enabled(enabled) {
            Ok(()) => {
                self.sync_tray_autostart_with_logging(
                    enabled,
                    source,
                    format!(
                        "Wardoff turned Start with Windows {} but could not update the tray checkbox",
                        on_off_label(enabled)
                    ),
                );
                logger::log_event(
                    "autostart_changed",
                    source,
                    format!(
                        "Wardoff turned Start with Windows {}.",
                        on_off_label(enabled)
                    ),
                    true,
                );
                Ok(())
            }
            Err(error) => {
                match resolved_autostart_state_for_tray(previous_state, autostart::is_enabled()) {
                    Ok(restored_state) => self.sync_tray_autostart_with_logging(
                        restored_state,
                        source,
                        format!(
                            "Wardoff could not restore the tray checkbox to Start with Windows {} after a failed scheduled-task update",
                            on_off_label(restored_state)
                        ),
                    ),
                    Err(read_error) => self.log_autostart_tray_sync_failure(
                        source,
                        format!(
                            "Wardoff could not restore the tray checkbox after failing to turn Start with Windows {} because the scheduled task state could not be read before or after the change attempt: {read_error}",
                            on_off_label(enabled)
                        ),
                    ),
                }

                let error_message = format!(
                    "Wardoff could not turn Start with Windows {}: {error}",
                    on_off_label(enabled)
                );
                logger::log_event("autostart_changed", source, error_message.clone(), false);
                Err(error_message)
            }
        }
    }

    fn sync_tray(&self, mode: BlockerMode) -> Result<(), String> {
        self.sync_tray_for_ledger(mode, &self.sessions)
    }

    fn sync_tray_for_ledger(
        &self,
        mode: BlockerMode,
        ledger: &SessionLedger,
    ) -> Result<(), String> {
        match self.tray_service.as_ref() {
            Some(tray_service) => tray_service.set_mode(
                mode,
                self.blocker_coordinator.power_intent(),
                ledger.manual(),
                ledger.manual().is_some()
                    || ledger.has_entries()
                    || self.pending_block_restore_after_wake.is_some(),
            ),
            None => Ok(()),
        }
    }

    fn refresh_autostart_tray_state(&mut self) {
        match autostart::is_enabled() {
            Ok(enabled) => self.sync_tray_autostart_with_logging(
                enabled,
                EventSource::Application,
                format!(
                    "Wardoff could not update the tray checkbox while refreshing Start with Windows {}",
                    on_off_label(enabled)
                ),
            ),
            Err(message) => {
                self.sync_tray_autostart_with_logging(
                    false,
                    EventSource::Application,
                    "Wardoff could not update the tray checkbox while falling back to Start with Windows off during initialization"
                        .to_string(),
                );
                warn!("Wardoff could not query its Start with Windows state: {message}");
                logger::log_event(
                    "autostart_state_read",
                    EventSource::Application,
                    format!(
                        "Wardoff could not query scheduled task \\Wardoff while initializing the tray checkbox: {message}"
                    ),
                    false,
                );
            }
        }
    }

    fn sync_tray_autostart(&self, enabled: bool) -> Result<(), String> {
        match self.tray_service.as_ref() {
            Some(tray_service) => tray_service.set_autostart_enabled(enabled),
            None => Ok(()),
        }
    }

    fn sync_tray_autostart_with_logging(
        &self,
        enabled: bool,
        source: EventSource,
        context: String,
    ) {
        if let Err(tray_error) = self.sync_tray_autostart(enabled) {
            self.log_autostart_tray_sync_failure(source, format!("{context}: {tray_error}"));
        }
    }

    fn log_autostart_tray_sync_failure(&self, source: EventSource, message: String) {
        warn!("{message}");
        logger::log_event("autostart_tray_sync", source, message, false);
    }

    fn reject_pending_ipc_requests(&mut self) {
        while let Ok(pending_request) = self.ipc_requests.try_recv() {
            let _ = pending_request.response_tx.send(IpcResponse::Error {
                message: "Wardoff is shutting down.".to_string(),
            });
        }
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        self.sessions.global_stop();
        self.session_owners.clear();
        self.clear_pending_block_restore_after_wake();
        self.ipc_server.begin_shutdown();
        self.reject_pending_ipc_requests();
        let ipc_shutdown = self.ipc_server.shutdown().map_err(other_error);
        let blocker_shutdown = self.blocker_coordinator.shutdown().map_err(other_error);

        if let Some(tray_service) = self.tray_service.as_mut() {
            tray_service.shutdown();
        }

        let shutdown_result: Result<(), Box<dyn Error>> = match (ipc_shutdown, blocker_shutdown) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(ipc_error), Ok(())) => Err(Box::new(ipc_error) as Box<dyn Error>),
            (Ok(()), Err(blocker_error)) => Err(Box::new(blocker_error) as Box<dyn Error>),
            (Err(ipc_error), Err(blocker_error)) => Err(Box::new(other_error(format!(
                "Wardoff encountered shutdown errors in both the IPC server and blocker coordinator: {ipc_error}; {blocker_error}"
            ))) as Box<dyn Error>),
        };

        match &shutdown_result {
            Ok(()) => logger::log_event(
                "application_stopped",
                EventSource::Application,
                "Wardoff shut down cleanly.",
                true,
            ),
            Err(error) => logger::log_event(
                "application_stopped",
                EventSource::Application,
                format!("Wardoff shut down with an error: {error}"),
                false,
            ),
        }

        logger::shutdown_structured_logging();
        shutdown_result
    }

    fn handle_forced_shutdown_cleanup(&mut self) {
        if self.forced_shutdown_cleanup_completed {
            return;
        }

        self.forced_shutdown_cleanup_completed = true;
        self.sessions.global_stop();
        self.session_owners.clear();
        self.clear_pending_block_restore_after_wake();
        self.blocker_coordinator.forced_shutdown_cleanup();
        info!("Wardoff shutting down due to user-forced shutdown");
        logger::log_event(
            "application_forced_shutdown",
            EventSource::Application,
            "Wardoff shutting down due to user-forced shutdown",
            true,
        );
    }

    fn clear_pending_block_restore_after_wake(&mut self) {
        self.pending_block_restore_after_wake = None;
    }

    fn handle_power_broadcast(&mut self, event: blocker::shutdown::PowerBroadcastEvent) {
        match event {
            blocker::shutdown::PowerBroadcastEvent::Suspend => {
                if let Some(pending_restore) = self.pending_block_restore_after_wake {
                    self.pending_block_restore_after_wake =
                        Some(pending_restore.mark_system_suspended());
                }
            }
            blocker::shutdown::PowerBroadcastEvent::Resume => {
                // Prune before any acquisition: elapsed deadlines include sleep.
                self.prune_sessions();
                if self.pending_block_restore_after_wake.is_some() {
                    self.restore_block_after_wake_if_needed();
                } else {
                    if self.reconciliation_pending {
                        let _ = self.reconcile_current_intent(EventSource::Sleep);
                    }
                    if session_protection_target(&self.sessions, false).is_some()
                        && self.blocker_coordinator.mode() == BlockerMode::Block
                    {
                        if let Err(error) =
                            self.blocker_coordinator.renew_sleep_requests_after_resume()
                        {
                            warn!("Could not renew idle-power requests after resume: {error}");
                            logger::log_event(
                                "sleep_resume_failed",
                                EventSource::Sleep,
                                error,
                                false,
                            );
                        }
                    }
                }
            }
        }
    }

    fn restore_block_after_wake_if_needed(&mut self) {
        let Some((action, _)) =
            pending_wake_restore_ready_for_resume(self.pending_block_restore_after_wake)
        else {
            return;
        };
        self.clear_pending_block_restore_after_wake();
        self.reconciliation_pending = true;
        let result = self.reconcile_current_intent(EventSource::Sleep);
        logger::log_event(
            "block_restored_after_wake",
            EventSource::Tray,
            match &result {
                Ok(()) => format!(
                    "Wardoff reconciled still-valid protection after tray {} resume.",
                    power_action_label(action)
                ),
                Err(error) => format!(
                    "Wardoff could not reconcile protection after tray {} resume: {error}",
                    power_action_label(action)
                ),
            },
            result.is_ok(),
        );
    }
}

fn forced_shutdown_cleanup_callback() {
    let application_ptr = ACTIVE_APPLICATION.load(Ordering::Acquire);
    if application_ptr.is_null() {
        return;
    }

    unsafe {
        (*application_ptr).handle_forced_shutdown_cleanup();
    }
}

fn power_broadcast_callback(event: blocker::shutdown::PowerBroadcastEvent) {
    let application_ptr = ACTIVE_APPLICATION.load(Ordering::Acquire);
    if application_ptr.is_null() {
        return;
    }

    unsafe {
        (*application_ptr).handle_power_broadcast(event);
    }
}

fn other_error(message: String) -> io::Error {
    io::Error::other(message)
}

fn should_show_graphical_startup_error() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let startup_context = args.is_empty()
        || args.iter().all(|arg| {
            arg == OsStr::new(INTERNAL_ELEVATED_RELAUNCH_ARG)
                || arg == OsStr::new(INTERNAL_DETACHED_RUNTIME_ARG)
        });
    if !startup_context {
        return false;
    }

    !matches!(
        windows_util::console_launch_context(),
        Ok(ConsoleLaunchContext::Inherited)
    )
}

fn mode_label(mode: BlockerMode) -> &'static str {
    match mode {
        BlockerMode::Block => "Block",
        BlockerMode::Allow => "Allow",
    }
}

fn configuration_label(mode: BlockerMode, intent: PowerIntent) -> &'static str {
    match (mode, intent) {
        (BlockerMode::Allow, _) => "Allow",
        (BlockerMode::Block, PowerIntent::SystemOnly) => "Block with display timeout allowed",
        (BlockerMode::Block, PowerIntent::SystemAndDisplay) => {
            "Block with display timeout prevented"
        }
    }
}

fn power_action_label(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Shutdown => "shutdown",
        PowerAction::Reboot => "reboot",
        PowerAction::Sleep => "sleep",
        PowerAction::Hibernate => "hibernate",
    }
}

fn on_off_label(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

fn runtime_request_label(action: RequestedAction) -> &'static str {
    match action {
        RequestedAction::Default => "default",
        RequestedAction::Block => "block",
        RequestedAction::BlockWithPowerIntent { .. } => "block with a display-timeout choice",
        RequestedAction::Allow => "allow",
        RequestedAction::Hide => "hidden block",
        RequestedAction::Autostart { .. } => "autostart",
        RequestedAction::Status
        | RequestedAction::Log { .. }
        | RequestedAction::Doctor { .. }
        | RequestedAction::DiagnosticsProbe => "read-only",
    }
}

#[cfg(test)]
mod tests {
    use super::run_with_cleanup;
    use std::error::Error;
    use std::io;

    #[test]
    fn run_with_cleanup_returns_success_after_normal_loop_and_cleanup() {
        let mut events = Vec::new();

        let result = run_with_cleanup(
            &mut events,
            |events| {
                events.push("loop");
                Ok::<(), Box<dyn Error>>(())
            },
            |events| {
                events.push("cleanup");
                Ok::<(), Box<dyn Error>>(())
            },
        );

        assert!(result.is_ok());
        assert_eq!(events, ["loop", "cleanup"]);
    }

    #[test]
    fn run_with_cleanup_runs_cleanup_after_loop_error() {
        let mut events = Vec::new();

        let result = run_with_cleanup(
            &mut events,
            |events| {
                events.push("loop");
                Err::<(), Box<dyn Error>>(Box::new(io::Error::other("loop error")))
            },
            |events| {
                events.push("cleanup");
                Ok::<(), Box<dyn Error>>(())
            },
        );

        assert_eq!(result.unwrap_err().to_string(), "loop error");
        assert_eq!(events, ["loop", "cleanup"]);
    }

    #[test]
    fn run_with_cleanup_propagates_shutdown_error() {
        let mut events = Vec::new();

        let result = run_with_cleanup(
            &mut events,
            |events| {
                events.push("loop");
                Ok::<(), Box<dyn Error>>(())
            },
            |events| {
                events.push("shutdown");
                Err::<(), Box<dyn Error>>(Box::new(io::Error::other("shutdown error")))
            },
        );

        assert_eq!(result.unwrap_err().to_string(), "shutdown error");
        assert_eq!(events, ["loop", "shutdown"]);
    }

    #[test]
    fn run_with_cleanup_preserves_both_errors_in_order() {
        let mut events = Vec::new();

        let result = run_with_cleanup(
            &mut events,
            |events| {
                events.push("loop");
                Err::<(), Box<dyn Error>>(Box::new(io::Error::other("loop error")))
            },
            |events| {
                events.push("shutdown");
                Err::<(), Box<dyn Error>>(Box::new(io::Error::other("shutdown error")))
            },
        );

        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime loop failed: loop error; cleanup failed: shutdown error"
        );
        assert_eq!(events, ["loop", "shutdown"]);
    }
}

#[cfg(test)]
mod session_release_tests {
    use super::finish_session_release;

    #[test]
    fn session_release_acknowledges_removal_despite_pending_cleanup() {
        assert!(finish_session_release(Ok(true), || Err("Fixture tray failure".into())).is_ok());
    }

    #[test]
    fn session_duplicate_release_is_an_acknowledged_noop() {
        let mut called = false;
        let result = finish_session_release(Ok(false), || {
            called = true;
            Err("Unrelated cleanup failure".into())
        });
        assert!(result.is_ok());
        assert!(!called);
    }

    #[test]
    fn session_foreign_release_never_reconciles_an_unowned_mutation() {
        let mut called = false;
        let result = finish_session_release(Err("Owner mismatch".into()), || {
            called = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!called);
    }
}
