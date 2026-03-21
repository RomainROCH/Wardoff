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
use crate::tray::{create_tray_controller, TrayAction, TrayController, TrayVisibility};
use env_logger::{Builder, Env};
use log::{error, info, LevelFilter};
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
    initialize_logging()?;

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

    info!("{}", application_start_message(options));

    Ok(Application {
        blocker_coordinator,
        tray_controller,
        ipc_requests,
        ipc_server,
        started_at: Instant::now(),
        _primary_instance: primary_instance,
    })
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
            error!("Wardoff failed to start: {error}");
            eprintln!("wardoff failed: {error}");
            std::process::exit(1);
        }
    }
}

fn run_main() -> Result<i32, Box<dyn Error>> {
    let cli = parse_cli();

    match cli.requested_action() {
        RequestedAction::Status => handle_status_request(),
        action => match claim_primary_instance().map_err(other_error)? {
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
        RequestedAction::Status => RuntimeOptions {
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
        RequestedAction::Status => IpcRequest::Status,
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

fn initialize_logging() -> Result<(), Box<dyn Error>> {
    let mut builder = Builder::from_env(Env::default().default_filter_or("info"));
    builder
        .format_timestamp_secs()
        .filter_level(LevelFilter::Info);
    builder.try_init()?;
    Ok(())
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
            IpcRequest::SetMode { mode } => match self.set_mode(mode.into()) {
                Ok(()) => IpcResponse::Ok,
                Err(message) => IpcResponse::Error { message },
            },
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
                Err(message) => error!(
                    "Wardoff could not process tray action {:?}: {message}",
                    action
                ),
            }
        }

        false
    }

    fn handle_tray_action(&mut self, action: TrayAction) -> Result<bool, String> {
        match action {
            TrayAction::Block => {
                self.set_mode(BlockerMode::Block)?;
                Ok(false)
            }
            TrayAction::Allow => {
                self.set_mode(BlockerMode::Allow)?;
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
            TrayAction::Quit => Ok(true),
        }
    }

    fn run_power_action(&mut self, action: PowerAction) -> Result<(), String> {
        let previous_mode = self.blocker_coordinator.mode();
        self.set_mode(BlockerMode::Allow)?;

        match execute_power_action(action) {
            Ok(()) => {
                info!(
                    "Wardoff switched to Allow mode and issued the requested {:?} power action.",
                    action
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
                if previous_mode == BlockerMode::Block {
                    return match self.set_mode(BlockerMode::Block) {
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

    fn set_mode(&mut self, mode: BlockerMode) -> Result<(), String> {
        let previous_mode = self.blocker_coordinator.mode();

        if previous_mode == mode {
            return self.sync_tray(previous_mode);
        }

        self.blocker_coordinator.set_mode(mode)?;

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

            return Err(error_message);
        }

        Ok(())
    }

    fn sync_tray(&mut self, mode: BlockerMode) -> Result<(), String> {
        match self.tray_controller.as_mut() {
            Some(tray_controller) => tray_controller.set_mode(mode),
            None => Ok(()),
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

        ipc_shutdown?;
        blocker_shutdown.map_err(|error| Box::new(error) as Box<dyn Error>)
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
