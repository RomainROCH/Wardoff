use chrono::{SecondsFormat, Utc};
use env_logger::{Builder, Env};
use log::{error, LevelFilter};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    mpsc::{self, Receiver, Sender},
    Mutex, OnceLock,
};
use std::thread::{self, JoinHandle};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

const DEFAULT_LOG_FILE_NAME: &str = "wardoff.jsonl";
const WRITER_THREAD_NAME: &str = "wardoff-structured-logger";
const DEFAULT_TAIL_LINE_COUNT: usize = 20;
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const MAX_LOG_FILES: usize = 3;

static LOG_COMMANDS: OnceLock<Sender<LogCommand>> = OnceLock::new();
static WRITER_HANDLE: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

/// Represents the subsystem that emitted a structured Wardoff log event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    /// The Task Scheduler autostart layer emitted the event.
    Autostart,
    /// The main application runtime emitted the event.
    Application,
    /// The command-line interface emitted the event.
    Cli,
    /// The named-pipe IPC layer emitted the event.
    Ipc,
    /// The Layer 2 local-shutdown ETW blocker emitted the event.
    Local,
    /// The Layer 4 remote-shutdown blocker emitted the event.
    Remote,
    /// The Layer 1 shutdown blocker emitted the event.
    Shutdown,
    /// The sleep and hibernate blocker emitted the event.
    Sleep,
    /// The tray surface emitted the event.
    Tray,
    /// The Layer 3 UpdateOrchestrator blocker emitted the event.
    UpdateOrchestrator,
}

/// Describes one line written to Wardoff's rotating JSON lines log.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogRecord {
    timestamp: String,
    event: String,
    source: String,
    action: String,
    success: bool,
}

enum LogCommand {
    Record(LogRecord),
    Shutdown,
}

/// Creates the configured `env_logger` builder used for human-readable stderr output.
pub fn logger_builder() -> Builder {
    let mut builder = Builder::from_env(Env::default().default_filter_or("info"));
    builder
        .format_timestamp_secs()
        .filter_level(default_level());
    builder
}

/// Returns the default filter level for Wardoff operational logs.
pub fn default_level() -> LevelFilter {
    LevelFilter::Info
}

/// Returns the filesystem path used for Wardoff's primary JSON lines log file.
pub fn default_log_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Wardoff")
        .join("logs")
        .join(DEFAULT_LOG_FILE_NAME)
}

/// Initializes human-readable stderr logging exactly once for the current process.
pub(crate) fn initialize_human_logging() -> Result<(), log::SetLoggerError> {
    logger_builder().try_init()
}

/// Starts the background writer thread used for the structured JSON lines sink.
pub(crate) fn initialize_structured_logging() -> Result<(), String> {
    if LOG_COMMANDS.get().is_some() {
        return Ok(());
    }

    let log_path = default_log_path();
    let (sender, receiver) = mpsc::channel();
    let writer_handle = thread::Builder::new()
        .name(WRITER_THREAD_NAME.to_string())
        .spawn(move || run_writer_loop(receiver, log_path))
        .map_err(|error| {
            format!("Wardoff could not start its structured log writer thread: {error}")
        })?;

    let _ = WRITER_HANDLE.get_or_init(|| Mutex::new(Some(writer_handle)));

    LOG_COMMANDS
        .set(sender)
        .map_err(|_| "Wardoff initialized its structured logger more than once.".to_string())?;

    Ok(())
}

/// Returns whether the structured JSON lines sink has been initialized.
pub(crate) fn structured_logging_initialized() -> bool {
    LOG_COMMANDS.get().is_some()
}

/// Stops the structured log writer thread after all queued records have been flushed.
pub(crate) fn shutdown_structured_logging() {
    if let Some(sender) = LOG_COMMANDS.get() {
        let _ = sender.send(LogCommand::Shutdown);
    }

    if let Some(writer_handle) = WRITER_HANDLE.get() {
        match writer_handle.lock() {
            Ok(mut guard) => {
                if let Some(join_handle) = guard.take() {
                    if join_handle.join().is_err() {
                        error!("Wardoff structured log writer thread panicked during shutdown.");
                    }
                }
            }
            Err(_) => error!("Wardoff could not lock its structured log writer handle."),
        }
    }
}

/// Queues a structured log record for the background writer thread.
pub(crate) fn log_event(
    event: impl Into<String>,
    source: EventSource,
    action: impl Into<String>,
    success: bool,
) {
    let Some(sender) = LOG_COMMANDS.get() else {
        return;
    };

    let record = LogRecord::new(event.into(), source, action.into(), success);
    let _ = sender.send(LogCommand::Record(record));
}

/// Returns the default number of JSON lines shown by `wardoff --log`.
pub(crate) fn default_tail_line_count() -> usize {
    DEFAULT_TAIL_LINE_COUNT
}

/// Reads the newest structured log lines across the retained rotation set.
pub(crate) fn read_recent_lines(limit: usize) -> Result<Vec<String>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut recent_lines = VecDeque::with_capacity(limit);

    for path in log_paths_oldest_to_newest(default_log_path()) {
        if log_file_metadata_if_exists(&path)?.is_none() {
            continue;
        }

        let file = open_plain_log_file_for_read(&path)?;
        let reader = BufReader::new(file);

        for line_result in reader.lines() {
            let line = line_result
                .map_err(|error| format!("Wardoff could not read {}: {error}", path.display()))?;

            if recent_lines.len() == limit {
                let _ = recent_lines.pop_front();
            }
            recent_lines.push_back(line);
        }
    }

    Ok(recent_lines.into_iter().collect())
}

impl EventSource {
    fn label(self) -> &'static str {
        match self {
            EventSource::Autostart => "autostart",
            EventSource::Application => "application",
            EventSource::Cli => "cli",
            EventSource::Ipc => "ipc",
            EventSource::Local => "local",
            EventSource::Remote => "remote",
            EventSource::Shutdown => "shutdown",
            EventSource::Sleep => "sleep",
            EventSource::Tray => "tray",
            EventSource::UpdateOrchestrator => "update_orchestrator",
        }
    }
}

impl LogRecord {
    fn new(event: String, source: EventSource, action: String, success: bool) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            event,
            source: source.label().to_string(),
            action,
            success,
        }
    }
}

fn run_writer_loop(receiver: Receiver<LogCommand>, log_path: PathBuf) {
    while let Ok(command) = receiver.recv() {
        match command {
            LogCommand::Record(record) => {
                if let Err(error) = write_record(&log_path, &record) {
                    error!("Wardoff could not append a structured log record: {error}");
                }
            }
            LogCommand::Shutdown => break,
        }
    }
}

fn write_record(path: &Path, record: &LogRecord) -> Result<(), String> {
    ensure_managed_log_directory(path)?;

    let line = serde_json::to_string(record)
        .map_err(|error| format!("Wardoff could not serialize a log record: {error}"))?;
    rotate_if_needed(path, line.len() as u64 + 1)?;

    let mut file = open_plain_log_file_for_append(path)?;

    file.write_all(line.as_bytes())
        .map_err(|error| format!("Wardoff could not write {}: {error}", path.display()))?;
    file.write_all(b"\n").map_err(|error| {
        format!(
            "Wardoff could not finish writing {}: {error}",
            path.display()
        )
    })?;
    file.flush()
        .map_err(|error| format!("Wardoff could not flush {}: {error}", path.display()))?;

    Ok(())
}

fn rotate_if_needed(path: &Path, incoming_bytes: u64) -> Result<(), String> {
    let current_size = match log_file_metadata_if_exists(path)? {
        Some(metadata) => metadata.len(),
        None => 0,
    };

    if current_size + incoming_bytes <= MAX_LOG_BYTES {
        return Ok(());
    }

    rotate_logs(path)
}

fn rotate_logs(path: &Path) -> Result<(), String> {
    let highest_rotated_index = MAX_LOG_FILES.saturating_sub(1);
    if highest_rotated_index == 0 {
        remove_file_if_exists(path)?;
        return Ok(());
    }

    remove_file_if_exists(&rotated_log_path(path, highest_rotated_index))?;

    for index in (1..highest_rotated_index).rev() {
        rename_if_exists(
            &rotated_log_path(path, index),
            &rotated_log_path(path, index + 1),
        )?;
    }

    rename_if_exists(path, &rotated_log_path(path, 1))
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if log_file_metadata_if_exists(path)?.is_none() {
        return Ok(());
    }

    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Wardoff could not remove {}: {error}",
            path.display()
        )),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> Result<(), String> {
    if log_file_metadata_if_exists(from)?.is_none() {
        return Ok(());
    }
    let _ = log_file_metadata_if_exists(to)?;

    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Wardoff could not rotate {} to {}: {error}",
            from.display(),
            to.display()
        )),
    }
}

fn ensure_managed_log_directory(path: &Path) -> Result<(), String> {
    let log_directory = path.parent().ok_or_else(|| {
        format!(
            "Wardoff could not resolve the parent directory for {}.",
            path.display()
        )
    })?;
    let wardoff_directory = log_directory.parent().ok_or_else(|| {
        format!(
            "Wardoff could not resolve the managed log root for {}.",
            path.display()
        )
    })?;
    let local_app_data = wardoff_directory.parent().ok_or_else(|| {
        format!(
            "Wardoff could not resolve the LOCALAPPDATA parent for {}.",
            path.display()
        )
    })?;

    ensure_managed_directory_component(local_app_data, wardoff_directory)?;
    ensure_managed_directory_component(wardoff_directory, log_directory)
}

fn ensure_managed_directory_component(parent: &Path, path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_plain_directory(path, &metadata),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(path)
                .or_else(|create_error| {
                    if create_error.kind() == ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(create_error)
                    }
                })
                .map_err(|error| {
                    format!(
                        "Wardoff could not create managed log directory {} under {}: {error}",
                        path.display(),
                        parent.display()
                    )
                })?;

            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "Wardoff could not inspect managed log directory {} after creation: {error}",
                    path.display()
                )
            })?;
            validate_plain_directory(path, &metadata)
        }
        Err(error) => Err(format!(
            "Wardoff could not inspect managed log directory {}: {error}",
            path.display()
        )),
    }
}

fn open_plain_log_file_for_read(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|error| format!("Wardoff could not open {}: {error}", path.display()))?;
    validate_open_plain_file(path, &file)?;
    Ok(file)
}

fn open_plain_log_file_for_append(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|error| format!("Wardoff could not open {}: {error}", path.display()))?;
    validate_open_plain_file(path, &file)?;
    Ok(file)
}

fn validate_open_plain_file(path: &Path, file: &File) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("Wardoff could not inspect {}: {error}", path.display()))?;
    validate_plain_file(path, &metadata)
}

fn log_file_metadata_if_exists(path: &Path) -> Result<Option<Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_plain_file(path, &metadata)?;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Wardoff could not inspect {} before rotation: {error}",
            path.display()
        )),
    }
}

fn validate_plain_directory(path: &Path, metadata: &Metadata) -> Result<(), String> {
    if has_reparse_point(metadata) {
        return Err(format!(
            "Wardoff refused to follow a reparse point at managed log directory {}.",
            path.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "Wardoff expected a directory at managed log path {}.",
            path.display()
        ));
    }

    Ok(())
}

fn validate_plain_file(path: &Path, metadata: &Metadata) -> Result<(), String> {
    if has_reparse_point(metadata) {
        return Err(format!(
            "Wardoff refused to follow a reparse point at managed log file {}.",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "Wardoff expected a plain file at managed log path {}.",
            path.display()
        ));
    }

    Ok(())
}

fn has_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

fn rotated_log_path(base_path: &Path, index: usize) -> PathBuf {
    let parent = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let stem = base_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("wardoff");
    let extension = base_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("jsonl");

    parent.join(format!("{stem}.{index}.{extension}"))
}

fn log_paths_oldest_to_newest(base_path: PathBuf) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(MAX_LOG_FILES);

    for index in (1..MAX_LOG_FILES).rev() {
        paths.push(rotated_log_path(&base_path, index));
    }

    paths.push(base_path);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ensure_managed_log_directory_creates_missing_components() {
        let base = unique_test_root("creates-managed-log-directory");
        fs::create_dir_all(&base).unwrap();

        let log_path = base
            .join("Wardoff")
            .join("logs")
            .join(DEFAULT_LOG_FILE_NAME);
        ensure_managed_log_directory(&log_path).unwrap();

        assert!(base.join("Wardoff").is_dir());
        assert!(base.join("Wardoff").join("logs").is_dir());

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn ensure_managed_log_directory_rejects_non_directory_components() {
        let base = unique_test_root("rejects-non-directory-component");
        fs::create_dir_all(&base).unwrap();
        File::create(base.join("Wardoff")).unwrap();

        let log_path = base
            .join("Wardoff")
            .join("logs")
            .join(DEFAULT_LOG_FILE_NAME);
        let error = ensure_managed_log_directory(&log_path).unwrap_err();

        assert!(error.contains("expected a directory"));

        fs::remove_file(base.join("Wardoff")).unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn log_file_metadata_if_exists_rejects_directories() {
        let base = unique_test_root("rejects-directory-log-file");
        let directory_path = base.join("Wardoff");
        fs::create_dir_all(&directory_path).unwrap();

        let error = log_file_metadata_if_exists(&directory_path).unwrap_err();
        assert!(error.contains("expected a plain file"));

        fs::remove_dir_all(base).unwrap();
    }

    fn unique_test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("wardoff-{name}-{unique}"))
    }
}
