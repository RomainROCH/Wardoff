use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const REPORT_SCHEMA_VERSION: u32 = 1;
const MAX_HISTORY_BYTES: usize = 512 * 1024;
const MAX_HISTORY_RECORD_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayerId {
    Shutdown,
    LocalShutdown,
    Update,
    Remote,
    IdlePower,
    #[serde(other)]
    Unknown,
}

pub(crate) const LAYER_IDS: [LayerId; 5] = [
    LayerId::Shutdown,
    LayerId::LocalShutdown,
    LayerId::Update,
    LayerId::Remote,
    LayerId::IdlePower,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum Observation {
    Inactive = 0,
    Starting = 1,
    Active = 2,
    ElevationRequired = 3,
    ElevationUnknown = 4,
    TaskAbsent = 5,
    ActivationFailed = 6,
    MonitoringFailed = 7,
    #[serde(other)]
    Unknown = 255,
}

impl Observation {
    pub(crate) fn api_active(self) -> Option<bool> {
        match self {
            Self::Active => Some(true),
            Self::Unknown => None,
            _ => Some(false),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LayerSnapshot {
    pub(crate) id: LayerId,
    pub(crate) observation: Observation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RequestObservation {
    pub(crate) requested: Option<bool>,
    pub(crate) api_active: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PowerRequests {
    pub(crate) system: RequestObservation,
    pub(crate) display: RequestObservation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) schema_version: u32,
    pub(crate) elevated: Option<bool>,
    pub(crate) manual_indefinite: bool,
    pub(crate) pending_wake_restore: bool,
    pub(crate) last_transition_failure: Option<LayerId>,
    pub(crate) layers: Vec<LayerSnapshot>,
    pub(crate) power_requests: PowerRequests,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeMode {
    Inactive,
    Block,
    Allow,
    Unreachable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Elevation {
    Elevated,
    Standard,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionKind {
    Service,
    UserSession,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Inspection {
    Complete,
    Unavailable,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskPresence {
    Present,
    Absent,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskIdentity {
    Absent,
    Matched,
    Unverified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PathAccess {
    UserWritable,
    Restricted,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AutostartFacts {
    pub(crate) enabled: Option<bool>,
    pub(crate) task_identity: TaskIdentity,
    pub(crate) path_access: PathAccess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LocalContext {
    pub(crate) client_elevation: Elevation,
    pub(crate) session_kind: SessionKind,
    pub(crate) inspection: Inspection,
    pub(crate) update_task: TaskPresence,
    pub(crate) autostart: AutostartFacts,
}

impl Default for LocalContext {
    fn default() -> Self {
        Self {
            client_elevation: Elevation::Unknown,
            session_kind: SessionKind::Unknown,
            inspection: Inspection::Unavailable,
            update_task: TaskPresence::Unknown,
            autostart: AutostartFacts {
                enabled: None,
                task_identity: TaskIdentity::Unverified,
                path_access: PathAccess::Unknown,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HistoricalCode {
    #[serde(rename = "previous_start_failed")]
    StartupFailure,
    #[serde(rename = "previous_idle_power_failed")]
    IdlePowerAcquisition,
    #[serde(rename = "previous_idle_power_cleanup_failed")]
    IdlePowerCleanup,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HistoricalFailure {
    pub(crate) code: HistoricalCode,
    pub(crate) timestamp: Option<String>,
    pub(crate) layer: Option<LayerId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportInput {
    pub(crate) mode: RuntimeMode,
    pub(crate) legacy_api: Option<[bool; 5]>,
    pub(crate) snapshot: Option<RuntimeSnapshot>,
    pub(crate) context: LocalContext,
    pub(crate) recent_failures: Vec<HistoricalFailure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeState {
    Inactive,
    Block,
    Allow,
    Unreachable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeEvidence {
    Inactive,
    Live,
    Legacy,
    Unreachable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RuntimeReport {
    state: RuntimeState,
    evidence: RuntimeEvidence,
    manual_indefinite: Option<bool>,
    pending_wake_restore: Option<bool>,
    elevated: Option<bool>,
    last_transition_failure: Option<LayerId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ContextReport {
    client_elevation: Elevation,
    session_kind: SessionKind,
    inspection: Inspection,
    update_task: TaskPresence,
    autostart: AutostartFacts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayerState {
    NotRequested,
    Starting,
    ApiActive,
    Failed,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Effectiveness {
    Untested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Coverage {
    InteractiveShutdown,
    BestEffortAbort,
    UpdateTaskOnly,
    IdleTimeouts,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct LayerReport {
    id: LayerId,
    requested: Option<bool>,
    api_active: Option<bool>,
    state: LayerState,
    reason_code: ReasonCode,
    effectiveness: Effectiveness,
    coverage: Coverage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RequestReport {
    requested: Option<bool>,
    api_active: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PowerReport {
    system: RequestReport,
    display: RequestReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct LimitReport {
    code: LimitCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DiagnosticReport {
    schema_version: u32,
    version: String,
    runtime: RuntimeReport,
    context: ContextReport,
    layers: Vec<LayerReport>,
    power_requests: PowerReport,
    recent_failures: Vec<HistoricalFailure>,
    limits: Vec<LimitReport>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReasonCode {
    NotRequested,
    Starting,
    ApiActive,
    ElevationRequired,
    ElevationUnknown,
    UpdateTaskAbsent,
    ActivationFailed,
    MonitoringFailed,
    UnexpectedActiveRequest,
    LegacyRuntime,
    NoRuntime,
    IpcUnreachable,
    InspectionUnavailable,
    InspectionTimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LimitCode {
    ForcedShutdownNotGuaranteed,
    ExplicitSleepNotVetoed,
    PolicyCanOverride,
    PhysicalEffectivenessUntested,
}

impl DiagnosticReport {
    pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub(crate) fn exit_code(&self) -> i32 {
        if self.runtime.evidence == RuntimeEvidence::Unreachable {
            1
        } else {
            0
        }
    }

    pub(crate) fn human_text(&self) -> String {
        let choice = match self.runtime.state {
            RuntimeState::Inactive => "Current choice: inactive.",
            RuntimeState::Block => {
                if self.runtime.manual_indefinite == Some(true) {
                    "Current choice: indefinite manual Block."
                } else {
                    "Current choice: Block."
                }
            }
            RuntimeState::Allow => "Current choice: Allow.",
            RuntimeState::Unreachable => "Current choice: runtime unreachable.",
        };
        let mut text = String::from(choice);
        text.push_str("\nObserved layers:");
        for layer in &self.layers {
            text.push_str(&format!(
                "\n- {}: requested {}, API {}, {} ({})",
                layer_name(layer.id),
                bool_label(layer.requested),
                bool_label(layer.api_active),
                reason_label(layer.reason_code),
                reason_code_label(layer.reason_code),
            ));
        }
        text.push_str(&format!(
            "\nContext: client elevation {}, session {}, inspection {}, Update task {}, autostart {}, task identity {}, path access {}.",
            context_label(self.context.client_elevation),
            context_label(self.context.session_kind),
            match self.context.inspection {
                Inspection::Complete => "complete",
                Inspection::Unavailable => reason_label(ReasonCode::InspectionUnavailable),
                Inspection::TimedOut => reason_label(ReasonCode::InspectionTimedOut),
            },
            context_label(self.context.update_task),
            bool_label(self.context.autostart.enabled),
            context_label(self.context.autostart.task_identity),
            context_label(self.context.autostart.path_access),
        ));
        text.push_str(&format!(
            "\nPower Requests: system requested {}, API {}; display requested {}, API {}.",
            bool_label(self.power_requests.system.requested),
            bool_label(self.power_requests.system.api_active),
            bool_label(self.power_requests.display.requested),
            bool_label(self.power_requests.display.api_active),
        ));
        if !self.recent_failures.is_empty() {
            text.push_str("\nRecent failures:");
            for failure in &self.recent_failures {
                text.push_str(&format!(
                    "\n- {} at {}",
                    historical_label(failure.code),
                    failure.timestamp.as_deref().unwrap_or("unknown time")
                ));
            }
        }
        text.push_str("\nLimitations: shutdown protection is best effort, explicit Sleep/Hibernate is not vetoed, Windows policy can override requests, and physical effectiveness is untested.");
        text
    }
}

fn bool_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

fn layer_name(id: LayerId) -> &'static str {
    match id {
        LayerId::Shutdown => "Shutdown",
        LayerId::LocalShutdown => "Local shutdown",
        LayerId::Update => "Update",
        LayerId::Remote => "Remote",
        LayerId::IdlePower => "Idle power",
        LayerId::Unknown => "Unknown layer",
    }
}

fn reason_label(reason: ReasonCode) -> &'static str {
    match reason {
        ReasonCode::ElevationRequired => "elevation required",
        ReasonCode::ElevationUnknown => "elevation unknown",
        ReasonCode::UpdateTaskAbsent => "Update task absent",
        ReasonCode::ActivationFailed => "activation failed",
        ReasonCode::MonitoringFailed => "monitoring failed",
        ReasonCode::UnexpectedActiveRequest => "unexpected active request",
        ReasonCode::LegacyRuntime => "legacy runtime details unavailable",
        ReasonCode::NoRuntime => "no runtime",
        ReasonCode::IpcUnreachable => "IPC unreachable",
        ReasonCode::InspectionUnavailable => "inspection unavailable",
        ReasonCode::InspectionTimedOut => "inspection timed out",
        ReasonCode::NotRequested => "not requested",
        ReasonCode::Starting => "starting",
        ReasonCode::ApiActive => "API active",
    }
}

fn reason_code_label(reason: ReasonCode) -> &'static str {
    match reason {
        ReasonCode::NotRequested => "not_requested",
        ReasonCode::Starting => "starting",
        ReasonCode::ApiActive => "api_active",
        ReasonCode::ElevationRequired => "elevation_required",
        ReasonCode::ElevationUnknown => "elevation_unknown",
        ReasonCode::UpdateTaskAbsent => "update_task_absent",
        ReasonCode::ActivationFailed => "activation_failed",
        ReasonCode::MonitoringFailed => "monitoring_failed",
        ReasonCode::UnexpectedActiveRequest => "unexpected_active_request",
        ReasonCode::LegacyRuntime => "legacy_runtime",
        ReasonCode::NoRuntime => "no_runtime",
        ReasonCode::IpcUnreachable => "ipc_unreachable",
        ReasonCode::InspectionUnavailable => "inspection_unavailable",
        ReasonCode::InspectionTimedOut => "inspection_timed_out",
    }
}

fn context_label<T: std::fmt::Debug>(value: T) -> String {
    format!("{:?}", value)
        .to_ascii_lowercase()
        .replace('_', " ")
        .replace("timedout", "timed out")
        .replace("usersession", "user session")
}

fn historical_label(code: HistoricalCode) -> &'static str {
    match code {
        HistoricalCode::StartupFailure => "previous start failed",
        HistoricalCode::IdlePowerAcquisition => "previous idle power failed",
        HistoricalCode::IdlePowerCleanup => "previous idle power cleanup failed",
    }
}

pub(crate) fn build_report(input: ReportInput) -> DiagnosticReport {
    let context = ContextReport {
        client_elevation: input.context.client_elevation,
        session_kind: input.context.session_kind,
        inspection: input.context.inspection,
        update_task: input.context.update_task,
        autostart: input.context.autostart,
    };
    let (state, evidence, snapshot) = match input.mode {
        RuntimeMode::Inactive => (RuntimeState::Inactive, RuntimeEvidence::Inactive, None),
        RuntimeMode::Unreachable => (
            RuntimeState::Unreachable,
            RuntimeEvidence::Unreachable,
            None,
        ),
        RuntimeMode::Block => snapshot_for_report(RuntimeMode::Block, input.snapshot),
        RuntimeMode::Allow => snapshot_for_report(RuntimeMode::Allow, input.snapshot),
    };
    let valid_snapshot = evidence == RuntimeEvidence::Live;
    let legacy_runtime = evidence == RuntimeEvidence::Legacy;
    let mode_requested = match input.mode {
        RuntimeMode::Inactive => Some(false),
        RuntimeMode::Block | RuntimeMode::Allow if legacy_runtime => None,
        RuntimeMode::Block => Some(true),
        RuntimeMode::Allow => Some(false),
        _ => None,
    };
    let observations: Vec<LayerSnapshot> = if let Some(snapshot) = snapshot.as_ref() {
        snapshot.layers.clone()
    } else {
        LAYER_IDS
            .iter()
            .enumerate()
            .map(|(index, id)| LayerSnapshot {
                id: *id,
                observation: input
                    .legacy_api
                    .and_then(|values| values.get(index).copied())
                    .map_or(Observation::Unknown, |active| {
                        if active {
                            Observation::Active
                        } else {
                            Observation::Inactive
                        }
                    }),
            })
            .collect()
    };
    let mut layers = Vec::with_capacity(LAYER_IDS.len());
    for id in LAYER_IDS {
        let observation = observations
            .iter()
            .find(|layer| layer.id == id)
            .map(|layer| layer.observation)
            .unwrap_or(Observation::Unknown);
        let api_active = if input.mode == RuntimeMode::Inactive {
            Some(false)
        } else if input.mode == RuntimeMode::Unreachable {
            None
        } else {
            observation.api_active()
        };
        layers.push(layer_report(
            id,
            mode_requested,
            observation,
            api_active,
            input.mode,
            legacy_runtime,
        ));
    }
    let power_requests = if input.mode == RuntimeMode::Inactive {
        PowerReport {
            system: RequestReport {
                requested: Some(false),
                api_active: Some(false),
            },
            display: RequestReport {
                requested: Some(false),
                api_active: Some(false),
            },
        }
    } else if input.mode == RuntimeMode::Unreachable {
        PowerReport {
            system: RequestReport {
                requested: None,
                api_active: None,
            },
            display: RequestReport {
                requested: None,
                api_active: None,
            },
        }
    } else if valid_snapshot {
        let power = &snapshot
            .as_ref()
            .expect("live snapshot is present")
            .power_requests;
        PowerReport {
            system: RequestReport {
                requested: power.system.requested,
                api_active: power.system.api_active,
            },
            display: RequestReport {
                requested: power.display.requested,
                api_active: power.display.api_active,
            },
        }
    } else {
        let sleep_active = input.legacy_api.and_then(|values| values.get(4).copied());
        PowerReport {
            system: RequestReport {
                requested: None,
                api_active: sleep_active.filter(|value| *value),
            },
            display: RequestReport {
                requested: None,
                api_active: sleep_active.filter(|value| *value),
            },
        }
    };
    let (manual_indefinite, pending_wake_restore, elevated, last_transition_failure) =
        if valid_snapshot {
            let value = snapshot.as_ref().expect("live snapshot is present");
            (
                Some(value.manual_indefinite),
                Some(value.pending_wake_restore),
                value.elevated,
                value.last_transition_failure,
            )
        } else {
            (None, None, None, None)
        };
    DiagnosticReport {
        schema_version: REPORT_SCHEMA_VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        runtime: RuntimeReport {
            state,
            evidence,
            manual_indefinite,
            pending_wake_restore,
            elevated,
            last_transition_failure,
        },
        context,
        layers,
        power_requests,
        recent_failures: sanitize_failures(input.recent_failures),
        limits: vec![
            LimitReport {
                code: LimitCode::ForcedShutdownNotGuaranteed,
            },
            LimitReport {
                code: LimitCode::ExplicitSleepNotVetoed,
            },
            LimitReport {
                code: LimitCode::PolicyCanOverride,
            },
            LimitReport {
                code: LimitCode::PhysicalEffectivenessUntested,
            },
        ],
    }
}

fn snapshot_for_report(
    mode: RuntimeMode,
    snapshot: Option<RuntimeSnapshot>,
) -> (RuntimeState, RuntimeEvidence, Option<RuntimeSnapshot>) {
    let valid = snapshot.filter(valid_snapshot);
    if valid.is_some() {
        (
            match mode {
                RuntimeMode::Allow => RuntimeState::Allow,
                _ => RuntimeState::Block,
            },
            RuntimeEvidence::Live,
            valid,
        )
    } else {
        (
            match mode {
                RuntimeMode::Allow => RuntimeState::Allow,
                _ => RuntimeState::Block,
            },
            RuntimeEvidence::Legacy,
            None,
        )
    }
}

fn valid_snapshot(snapshot: &RuntimeSnapshot) -> bool {
    snapshot.schema_version == REPORT_SCHEMA_VERSION
        && snapshot.layers.len() == LAYER_IDS.len()
        && LAYER_IDS.iter().all(|id| {
            snapshot
                .layers
                .iter()
                .filter(|layer| layer.id == *id)
                .count()
                == 1
        })
        && snapshot
            .layers
            .iter()
            .all(|layer| layer.id != LayerId::Unknown)
}

fn layer_report(
    id: LayerId,
    requested: Option<bool>,
    observation: Observation,
    api_active: Option<bool>,
    mode: RuntimeMode,
    legacy_runtime: bool,
) -> LayerReport {
    let unexpected = mode == RuntimeMode::Allow && api_active == Some(true);
    let (state, reason_code) = if mode == RuntimeMode::Inactive {
        (LayerState::Unknown, ReasonCode::NoRuntime)
    } else if mode == RuntimeMode::Unreachable {
        (LayerState::Unknown, ReasonCode::IpcUnreachable)
    } else if legacy_runtime {
        (LayerState::Unknown, ReasonCode::LegacyRuntime)
    } else if unexpected {
        (LayerState::Failed, ReasonCode::UnexpectedActiveRequest)
    } else if mode == RuntimeMode::Block && observation == Observation::Inactive {
        (LayerState::Failed, ReasonCode::ActivationFailed)
    } else {
        match observation {
            Observation::Inactive => (LayerState::NotRequested, ReasonCode::NotRequested),
            Observation::Starting => (LayerState::Starting, ReasonCode::Starting),
            Observation::Active => (LayerState::ApiActive, ReasonCode::ApiActive),
            Observation::ElevationRequired => {
                (LayerState::Unavailable, ReasonCode::ElevationRequired)
            }
            Observation::ElevationUnknown => (LayerState::Unknown, ReasonCode::ElevationUnknown),
            Observation::TaskAbsent => (LayerState::Unavailable, ReasonCode::UpdateTaskAbsent),
            Observation::ActivationFailed => (LayerState::Failed, ReasonCode::ActivationFailed),
            Observation::MonitoringFailed => (LayerState::Failed, ReasonCode::MonitoringFailed),
            Observation::Unknown => (LayerState::Unknown, ReasonCode::InspectionUnavailable),
        }
    };
    LayerReport {
        id,
        requested,
        api_active,
        state,
        reason_code,
        effectiveness: Effectiveness::Untested,
        coverage: coverage_for(id),
    }
}

fn coverage_for(id: LayerId) -> Coverage {
    match id {
        LayerId::Shutdown => Coverage::InteractiveShutdown,
        LayerId::LocalShutdown => Coverage::BestEffortAbort,
        LayerId::Update => Coverage::UpdateTaskOnly,
        LayerId::Remote => Coverage::BestEffortAbort,
        LayerId::IdlePower => Coverage::IdleTimeouts,
        LayerId::Unknown => Coverage::BestEffortAbort,
    }
}

fn sanitize_failures(failures: Vec<HistoricalFailure>) -> Vec<HistoricalFailure> {
    let mut seen = HashSet::new();
    failures
        .into_iter()
        .filter_map(|mut failure| {
            failure.timestamp = normalize_timestamp(failure.timestamp.as_deref());
            let key = (failure.code, failure.layer, failure.timestamp.clone());
            if seen.insert(key) {
                Some(failure)
            } else {
                None
            }
        })
        .take(5)
        .collect()
}

fn normalize_timestamp(timestamp: Option<&str>) -> Option<String> {
    timestamp
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| {
            value
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::AutoSi, true)
        })
}

#[derive(Deserialize)]
struct HistoryRecord {
    event: String,
    success: Option<bool>,
    timestamp: Option<String>,
}

pub(crate) fn parse_history(bytes: &[u8], truncated_prefix: bool) -> Vec<HistoricalFailure> {
    let is_truncated = truncated_prefix || bytes.len() > MAX_HISTORY_BYTES;
    let input = if bytes.len() > MAX_HISTORY_BYTES {
        &bytes[bytes.len() - MAX_HISTORY_BYTES..]
    } else {
        bytes
    };
    let mut lines = input.split_inclusive(|byte| *byte == b'\n');
    if is_truncated {
        let _ = lines.next();
    }
    let mut records = Vec::new();
    for segment in lines {
        if !segment.ends_with(b"\n") {
            continue;
        }
        let line = &segment[..segment.len() - 1];
        if line.is_empty() || line.len() > MAX_HISTORY_RECORD_BYTES {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<HistoryRecord>(line) else {
            continue;
        };
        if record.success != Some(false) {
            continue;
        }
        let Some((code, layer)) = (match record.event.as_str() {
            "application_start_failed" => Some((HistoricalCode::StartupFailure, None)),
            "sleep_blocked" => Some((
                HistoricalCode::IdlePowerAcquisition,
                Some(LayerId::IdlePower),
            )),
            "sleep_unblocked" => Some((HistoricalCode::IdlePowerCleanup, Some(LayerId::IdlePower))),
            _ => None,
        }) else {
            continue;
        };
        let failure = HistoricalFailure {
            code,
            timestamp: normalize_timestamp(record.timestamp.as_deref()),
            layer,
        };
        let key = (failure.code, failure.layer, failure.timestamp.clone());
        if let Some(position) = records.iter().position(|item: &HistoricalFailure| {
            (item.code, item.layer, item.timestamp.clone()) == key
        }) {
            records.remove(position);
        }
        records.push(failure);
        if records.len() > 5 {
            records.remove(0);
        }
    }
    records.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> LocalContext {
        LocalContext::default()
    }

    fn snapshot(observation: Observation) -> RuntimeSnapshot {
        RuntimeSnapshot {
            schema_version: 1,
            elevated: Some(true),
            manual_indefinite: true,
            pending_wake_restore: false,
            last_transition_failure: None,
            layers: LAYER_IDS
                .iter()
                .map(|id| LayerSnapshot {
                    id: *id,
                    observation,
                })
                .collect(),
            power_requests: PowerRequests {
                system: RequestObservation {
                    requested: Some(true),
                    api_active: Some(true),
                },
                display: RequestObservation {
                    requested: Some(true),
                    api_active: Some(true),
                },
            },
        }
    }

    #[test]
    fn inactive_and_unreachable_have_expected_exit_codes_and_null_details() {
        let inactive = build_report(ReportInput {
            mode: RuntimeMode::Inactive,
            legacy_api: None,
            snapshot: Some(snapshot(Observation::Active)),
            context: context(),
            recent_failures: Vec::new(),
        });
        let unreachable = build_report(ReportInput {
            mode: RuntimeMode::Unreachable,
            legacy_api: None,
            snapshot: Some(snapshot(Observation::Active)),
            context: context(),
            recent_failures: Vec::new(),
        });
        assert_eq!(inactive.exit_code(), 0);
        assert_eq!(unreachable.exit_code(), 1);
        assert!(inactive.runtime.manual_indefinite.is_none());
        assert!(unreachable
            .layers
            .iter()
            .all(|layer| layer.api_active.is_none()));
    }

    #[test]
    fn future_or_incomplete_snapshots_fall_back_to_legacy_details() {
        let mut future = snapshot(Observation::Active);
        future.schema_version = 2;
        let report = build_report(ReportInput {
            mode: RuntimeMode::Block,
            legacy_api: Some([true, false, false, false, false]),
            snapshot: Some(future),
            context: context(),
            recent_failures: Vec::new(),
        });
        assert_eq!(report.runtime.evidence, RuntimeEvidence::Legacy);
        assert!(report.runtime.elevated.is_none());
        assert!(report.runtime.pending_wake_restore.is_none());
    }

    #[test]
    fn allow_with_active_api_is_reported_as_failed() {
        let report = build_report(ReportInput {
            mode: RuntimeMode::Allow,
            legacy_api: None,
            snapshot: Some(snapshot(Observation::Active)),
            context: context(),
            recent_failures: Vec::new(),
        });
        assert!(report
            .layers
            .iter()
            .all(|layer| layer.state == LayerState::Failed));
        assert!(report
            .layers
            .iter()
            .all(|layer| layer.reason_code == ReasonCode::UnexpectedActiveRequest));
        assert_eq!(
            report
                .layers
                .iter()
                .find(|layer| layer.id == LayerId::LocalShutdown)
                .unwrap()
                .coverage,
            Coverage::BestEffortAbort
        );
    }

    #[test]
    fn history_is_bounded_sanitized_and_private_data_is_not_exported() {
        let text = br#"{"event":"application_start_failed","success":false,"timestamp":"2026-10-24T10:00:00+02:00","action":"C:\\Users\\fictional\\private.exe"}
{"event":"sleep_blocked","success":false,"timestamp":"bad","message":"secret"}
{"event":"sleep_unblocked","success":false,"timestamp":"2026-10-24T08:00:00Z"}
{"event":"sleep_blocked","success":true,"timestamp":"2026-10-24T08:00:00Z"}
"#;
        let failures = parse_history(text, false);
        assert_eq!(failures.len(), 3);
        assert_eq!(failures[0].code, HistoricalCode::IdlePowerCleanup);
        assert!(failures[1].timestamp.is_none());
        assert_eq!(
            failures[2].timestamp.as_deref(),
            Some("2026-10-24T08:00:00Z")
        );
        let report = build_report(ReportInput {
            mode: RuntimeMode::Inactive,
            legacy_api: None,
            snapshot: None,
            context: context(),
            recent_failures: failures,
        });
        let json = report.to_json().expect("report serializes");
        assert!(!json.contains("fictional"));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn history_discards_partial_and_unterminated_records_and_deduplicates() {
        let text = br#"partial
{"event":"sleep_blocked","success":false,"timestamp":"2026-10-24T08:00:00Z"}
{"event":"sleep_blocked","success":false,"timestamp":"2026-10-24T08:00:00Z"}"#;
        let failures = parse_history(text, true);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, HistoricalCode::IdlePowerAcquisition);
    }

    #[test]
    fn history_drops_an_unterminated_record_after_a_truncated_prefix() {
        let text = br#"truncated prefix
{"event":"sleep_blocked","success":false,"timestamp":"2026-10-24T08:01:00Z"}"#;

        assert!(parse_history(text, true).is_empty());
    }

    #[test]
    fn history_keeps_only_terminated_records_after_a_truncated_prefix() {
        let text = br#"truncated prefix
{"event":"sleep_blocked","success":false,"timestamp":"2026-10-24T08:01:00Z"}
{"event":"sleep_blocked","success":false,"timestamp":"2026-10-24T08:02:00Z"}"#;
        let failures = parse_history(text, true);

        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].timestamp.as_deref(),
            Some("2026-10-24T08:01:00Z")
        );
    }

    #[test]
    fn block_with_legacy_false_api_keeps_requested_unknown_and_marks_legacy_runtime() {
        let report = build_report(ReportInput {
            mode: RuntimeMode::Block,
            legacy_api: Some([false; 5]),
            snapshot: None,
            context: context(),
            recent_failures: Vec::new(),
        });

        let shutdown = report
            .layers
            .iter()
            .find(|layer| layer.id == LayerId::Shutdown)
            .expect("shutdown layer");
        assert_eq!(shutdown.requested, None);
        assert_eq!(shutdown.api_active, Some(false));
        assert_eq!(shutdown.state, LayerState::Unknown);
        assert_eq!(shutdown.reason_code, ReasonCode::LegacyRuntime);
    }

    #[test]
    fn inactive_ignores_supplied_runtime_data_and_reports_no_runtime() {
        let report = build_report(ReportInput {
            mode: RuntimeMode::Inactive,
            legacy_api: Some([true; 5]),
            snapshot: Some(snapshot(Observation::Active)),
            context: context(),
            recent_failures: Vec::new(),
        });

        let shutdown = report
            .layers
            .iter()
            .find(|layer| layer.id == LayerId::Shutdown)
            .expect("shutdown layer");
        assert_eq!(shutdown.requested, Some(false));
        assert_eq!(shutdown.api_active, Some(false));
        assert_eq!(shutdown.reason_code, ReasonCode::NoRuntime);
    }

    #[test]
    fn unreachable_ignores_supplied_runtime_data_and_reports_ipc_unreachable() {
        let report = build_report(ReportInput {
            mode: RuntimeMode::Unreachable,
            legacy_api: Some([true; 5]),
            snapshot: Some(snapshot(Observation::Active)),
            context: context(),
            recent_failures: Vec::new(),
        });

        let shutdown = report
            .layers
            .iter()
            .find(|layer| layer.id == LayerId::Shutdown)
            .expect("shutdown layer");
        assert_eq!(shutdown.requested, None);
        assert_eq!(shutdown.api_active, None);
        assert_eq!(shutdown.reason_code, ReasonCode::IpcUnreachable);
    }

    #[test]
    fn requested_block_with_inactive_snapshot_is_an_activation_failure() {
        let report = build_report(ReportInput {
            mode: RuntimeMode::Block,
            legacy_api: None,
            snapshot: Some(snapshot(Observation::Inactive)),
            context: context(),
            recent_failures: Vec::new(),
        });

        let shutdown = report
            .layers
            .iter()
            .find(|layer| layer.id == LayerId::Shutdown)
            .expect("shutdown layer");
        assert_eq!(shutdown.requested, Some(true));
        assert_eq!(shutdown.api_active, Some(false));
        assert_eq!(shutdown.state, LayerState::Failed);
        assert_eq!(shutdown.reason_code, ReasonCode::ActivationFailed);
    }

    #[test]
    fn human_text_names_layer_causes_and_unknown_local_context() {
        let mut layers = LAYER_IDS
            .iter()
            .map(|id| LayerSnapshot {
                id: *id,
                observation: Observation::Unknown,
            })
            .collect::<Vec<_>>();
        layers[0].observation = Observation::ElevationRequired;
        layers[2].observation = Observation::TaskAbsent;
        layers[4].observation = Observation::ActivationFailed;
        let mut local = context();
        local.inspection = Inspection::TimedOut;
        local.autostart.task_identity = TaskIdentity::Unverified;
        let report = build_report(ReportInput {
            mode: RuntimeMode::Block,
            legacy_api: None,
            snapshot: Some(RuntimeSnapshot {
                layers,
                ..snapshot(Observation::Unknown)
            }),
            context: local,
            recent_failures: Vec::new(),
        });
        let text = report.human_text();

        assert!(text.contains("Shutdown"));
        assert!(text.contains("elevation"));
        assert!(text.contains("Update"));
        assert!(text.contains("task"));
        assert!(text.contains("Idle power"));
        assert!(text.contains("activation"));
        assert!(text.contains("timed out"));
        assert!(text.contains("autostart"));
        assert!(text.contains("unverified"));
        assert!(!text.contains("C:\\Users\\"));
    }
}
