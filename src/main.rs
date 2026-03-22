mod autostart;
mod blocker;
mod cli;
mod config;
mod instance;
mod ipc;
mod logger;
mod tray;

use crate::blocker::{
    create_blocker_coordinator, execute_power_action, BlockerCoordinator, BlockerMode, PowerAction,
};
use crate::cli::{parse_cli, RequestedAction, StatusOutput};
use crate::instance::{claim_primary_instance, InstanceClaim, InstanceGuard};
use crate::ipc::{
    send_request, ClientError, IpcRequest, IpcResponse, IpcServer, PendingRequest, PipeMode,
    IPC_WAKE_MESSAGE,
};
use crate::logger::EventSource;
use crate::tray::{create_tray_controller, TrayAction, TrayController, TrayVisibility};
use log::{error, info, warn};
use std::error::Error;
use std::io;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, WaitMessage, MSG, PM_NOREMOVE, PM_REMOVE,
    WM_QUIT,
};

/// Coordinates CLI, tray, and blocker scaffolding for the Wardoff binary.
struct Application {
    blocker_coordinator: BlockerCoordinator,
    tray_controller: Option<TrayController>,
    ipc_requests: Receiver<PendingRequest>,
    ipc_server: IpcServer,
    started_at: Instant,
    _primary_instance: InstanceGuard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TraySurface {
    Visible,
    Hidden,
    Headless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeOptions {
    initial_mode: BlockerMode,
    tray_surface: TraySurface,
}

/// Bootstraps logging, the central state manager, and the tray surface.
fn bootstrap(
    primary_instance: InstanceGuard,
    options: RuntimeOptions,
) -> Result<Application, Box<dyn Error>> {
    logger::initialize_structured_logging().map_err(other_error)?;

    let blocker_coordinator =
        create_blocker_coordinator(options.initial_mode).map_err(other_error)?;
    let mut tray_controller = match options.tray_surface {
        TraySurface::Headless => None,
        TraySurface::Visible => Some(
            create_tray_controller(TrayVisibility::Visible)
                .map_err(|message| other_error(message.to_string()))?,
        ),
        TraySurface::Hidden => Some(
            create_tray_controller(TrayVisibility::Hidden)
                .map_err(|message| other_error(message.to_string()))?,
        ),
    };

    if let Some(tray_controller) = tray_controller.as_mut() {
        tray_controller
            .set_mode(blocker_coordinator.mode())
            .map_err(other_error)?;
    }

    ensure_message_queue();
    let (ipc_request_tx, ipc_requests) = mpsc::channel();
    let ipc_server =
        IpcServer::start(ipc_request_tx, unsafe { GetCurrentThreadId() }).map_err(other_error)?;

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
        tray_controller,
        ipc_requests,
        ipc_server,
        started_at: Instant::now(),
        _primary_instance: primary_instance,
    };
    application.refresh_autostart_tray_state();

    Ok(application)
}

/// Runs the shared Win32 event loop for the hidden Layer 1 windows and the tray icon.
fn run(mut application: Application) -> Result<(), Box<dyn Error>> {
    let mut message = MSG::default();

    'message_loop: loop {
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                break 'message_loop;
            }

            if message.message == IPC_WAKE_MESSAGE {
                application.process_ipc_requests();
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

        unsafe { WaitMessage()? };
    }

    application.shutdown()?;
    Ok(())
}

fn main() {
    match run_main() {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
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
    logger::initialize_human_logging()?;
    let cli = parse_cli();

    match cli.requested_action() {
        RequestedAction::Autostart { enabled } => handle_autostart_request(enabled),
        RequestedAction::Status => handle_status_request(),
        RequestedAction::Log { tail } => handle_log_request(tail),
        action @ (RequestedAction::Default
        | RequestedAction::Block
        | RequestedAction::Allow
        | RequestedAction::Hide) => match claim_primary_instance().map_err(other_error)? {
            InstanceClaim::Primary(primary_instance) => {
                let application = bootstrap(primary_instance, runtime_options_for(action))?;
                run(application)?;
                Ok(0)
            }
            InstanceClaim::Secondary => forward_request_to_primary(ipc_request_for(action)),
        },
    }
}

fn handle_status_request() -> Result<i32, Box<dyn Error>> {
    match send_request(&IpcRequest::Status) {
        Ok(IpcResponse::Status { status }) => {
            println!("{}", status.to_json()?);
            Ok(0)
        }
        Ok(IpcResponse::Error { message }) => Err(Box::new(other_error(message))),
        Ok(IpcResponse::Ok) => Err(Box::new(other_error(
            "Wardoff received an unexpected empty response for --status.".to_string(),
        ))),
        Err(ClientError::Unavailable) => {
            println!("{}", StatusOutput::inactive().to_json()?);
            Ok(1)
        }
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
            drop(primary_instance);

            let mut structured_logging_started = false;
            match logger::initialize_structured_logging() {
                Ok(()) => structured_logging_started = true,
                Err(error) => {
                    error!(
                        "Wardoff could not initialize structured logging for --autostart: {error}"
                    );
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
        InstanceClaim::Secondary => {
            forward_request_to_primary(IpcRequest::SetAutostart { enabled })
        }
    }
}

fn forward_request_to_primary(request: IpcRequest) -> Result<i32, Box<dyn Error>> {
    match send_request(&request) {
        Ok(IpcResponse::Ok) => Ok(0),
        Ok(IpcResponse::Error { message }) => Err(Box::new(other_error(message))),
        Ok(IpcResponse::Status { .. }) => Err(Box::new(other_error(
            "Wardoff received a status payload for a control command.".to_string(),
        ))),
        Err(ClientError::Unavailable) => Err(Box::new(other_error(
            "Wardoff found another instance, but its control pipe was unavailable.".to_string(),
        ))),
        Err(ClientError::Transport(message)) => Err(Box::new(other_error(message))),
    }
}

fn runtime_options_for(action: RequestedAction) -> RuntimeOptions {
    match action {
        RequestedAction::Default => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Visible,
        },
        RequestedAction::Block => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Headless,
        },
        RequestedAction::Allow => RuntimeOptions {
            initial_mode: BlockerMode::Allow,
            tray_surface: TraySurface::Visible,
        },
        RequestedAction::Hide => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Hidden,
        },
        RequestedAction::Autostart { .. } => {
            unreachable!("autostart changes do not start the primary runtime")
        }
        RequestedAction::Status | RequestedAction::Log { .. } => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Visible,
        },
    }
}

fn ipc_request_for(action: RequestedAction) -> IpcRequest {
    match action {
        RequestedAction::Default | RequestedAction::Block | RequestedAction::Hide => {
            IpcRequest::SetMode {
                mode: PipeMode::Block,
            }
        }
        RequestedAction::Allow => IpcRequest::SetMode {
            mode: PipeMode::Allow,
        },
        RequestedAction::Autostart { enabled } => IpcRequest::SetAutostart { enabled },
        RequestedAction::Status | RequestedAction::Log { .. } => IpcRequest::Status,
    }
}

fn ensure_message_queue() {
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
}

fn application_start_message(options: RuntimeOptions) -> String {
    format!(
        "Wardoff started in {} mode with {}.",
        mode_label(options.initial_mode),
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
            let response = self.handle_ipc_request(pending_request.request);
            let _ = pending_request.response_tx.send(response);
        }
    }

    fn handle_ipc_request(&mut self, request: IpcRequest) -> IpcResponse {
        match request {
            IpcRequest::SetMode { mode } => {
                let requested_mode: BlockerMode = mode.into();

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
            IpcRequest::ShutdownServer => IpcResponse::Error {
                message:
                    "Wardoff cannot route the internal shutdown request through the UI thread."
                        .to_string(),
            },
        }
    }

    fn status_output(&self) -> StatusOutput {
        StatusOutput::active(
            self.blocker_coordinator.mode(),
            self.blocker_coordinator.layer_status(),
            self.started_at.elapsed().as_secs(),
            self.blocker_coordinator.blocked_count(),
        )
    }

    fn process_tray_actions(&mut self) -> bool {
        let actions = match self.tray_controller.as_ref() {
            Some(tray_controller) => tray_controller.drain_actions(),
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
        let previous_mode = self.blocker_coordinator.mode();
        self.set_mode(BlockerMode::Allow, EventSource::Tray)?;

        match execute_power_action(action) {
            Ok(()) => {
                info!(
                    "Wardoff switched to Allow mode and issued the requested {:?} power action.",
                    action
                );
                logger::log_event(
                    "power_action_issued",
                    EventSource::Tray,
                    format!(
                        "Wardoff issued the requested {} power action after switching to Allow mode.",
                        power_action_label(action)
                    ),
                    true,
                );
                if matches!(action, PowerAction::Sleep | PowerAction::Hibernate)
                    && previous_mode == BlockerMode::Block
                {
                    info!(
                        "Wardoff will remain in Allow mode after wake. Use the tray menu to return to Block mode when you need shutdown protection again."
                    );
                }
                Ok(())
            }
            Err(message) => {
                logger::log_event(
                    "power_action_issued",
                    EventSource::Tray,
                    format!(
                        "Wardoff could not issue the requested {} power action: {message}",
                        power_action_label(action)
                    ),
                    false,
                );
                if previous_mode == BlockerMode::Block {
                    return match self.set_mode(BlockerMode::Block, EventSource::Tray) {
                        Ok(()) => Err(format!(
                            "{message} Wardoff restored Block mode after the failed {:?} request.",
                            action
                        )),
                        Err(restore_error) => Err(format!(
                            "{message} Wardoff could not restore Block mode after the failed {:?} request and remains in {} mode: {restore_error}",
                            action,
                            mode_label(self.blocker_coordinator.mode())
                        )),
                    };
                }

                Err(format!(
                    "{message} Wardoff remains in {} mode.",
                    mode_label(self.blocker_coordinator.mode())
                ))
            }
        }
    }

    fn set_mode(&mut self, mode: BlockerMode, source: EventSource) -> Result<(), String> {
        let previous_mode = self.blocker_coordinator.mode();

        if previous_mode == mode {
            return self.sync_tray(previous_mode);
        }

        if let Err(error) = self.blocker_coordinator.set_mode(mode) {
            let error_message = format!(
                "Wardoff could not switch from {} mode to {} mode: {error}",
                mode_label(previous_mode),
                mode_label(mode)
            );
            logger::log_event("state_changed", source, error_message.clone(), false);
            return Err(error_message);
        }

        if let Err(tray_error) = self.sync_tray(self.blocker_coordinator.mode()) {
            let rollback_result = self.blocker_coordinator.set_mode(previous_mode);
            let final_mode = self.blocker_coordinator.mode();
            let tray_resync_result = self.sync_tray(final_mode);

            let mut error_message = format!(
                "Wardoff could not update the tray UI after switching to {} mode: {tray_error}",
                mode_label(mode)
            );

            match rollback_result {
                Ok(_) => error_message.push_str(&format!(
                    " Wardoff rolled the blocker state back to {} mode.",
                    mode_label(previous_mode)
                )),
                Err(rollback_error) => error_message.push_str(&format!(
                    " Wardoff could not roll the blocker state back to {} mode and is currently in {} mode: {rollback_error}",
                    mode_label(previous_mode),
                    mode_label(final_mode)
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

        logger::log_event(
            "state_changed",
            source,
            format!(
                "Wardoff switched from {} mode to {} mode.",
                mode_label(previous_mode),
                mode_label(mode)
            ),
            true,
        );

        Ok(())
    }

    fn set_autostart_enabled(&mut self, enabled: bool, source: EventSource) -> Result<(), String> {
        let previous_state = self
            .tray_controller
            .as_ref()
            .map(TrayController::autostart_enabled)
            .unwrap_or(false);

        match autostart::set_enabled(enabled) {
            Ok(()) => {
                self.sync_tray_autostart(enabled);
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
                match autostart::is_enabled() {
                    Ok(actual_state) => self.sync_tray_autostart(actual_state),
                    Err(_) => self.sync_tray_autostart(previous_state),
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

    fn sync_tray(&mut self, mode: BlockerMode) -> Result<(), String> {
        match self.tray_controller.as_mut() {
            Some(tray_controller) => tray_controller.set_mode(mode),
            None => Ok(()),
        }
    }

    fn refresh_autostart_tray_state(&mut self) {
        match autostart::is_enabled() {
            Ok(enabled) => self.sync_tray_autostart(enabled),
            Err(message) => {
                self.sync_tray_autostart(false);
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

    fn sync_tray_autostart(&self, enabled: bool) {
        if let Some(tray_controller) = self.tray_controller.as_ref() {
            tray_controller.set_autostart_enabled(enabled);
        }
    }

    fn reject_pending_ipc_requests(&mut self) {
        while let Ok(pending_request) = self.ipc_requests.try_recv() {
            let _ = pending_request.response_tx.send(IpcResponse::Error {
                message: "Wardoff is shutting down.".to_string(),
            });
        }
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        self.reject_pending_ipc_requests();
        let ipc_shutdown = self.ipc_server.shutdown().map_err(other_error);
        let blocker_shutdown = self.blocker_coordinator.shutdown().map_err(other_error);

        if let Some(tray_controller) = self.tray_controller.as_mut() {
            tray_controller.shutdown();
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
}

fn other_error(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message)
}

fn mode_label(mode: BlockerMode) -> &'static str {
    match mode {
        BlockerMode::Block => "Block",
        BlockerMode::Allow => "Allow",
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
