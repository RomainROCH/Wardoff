use crate::power_intent::PowerIntent;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum number of pending and active session entries retained together.
pub(crate) const MAX_SESSIONS: usize = 64;
/// Maximum session duration accepted by the pure ledger.
pub(crate) const MAX_DURATION_MS: u64 = 604_800_000;
/// Maximum number of Unicode scalar values in a session reason.
pub(crate) const MAX_REASON_CHARS: usize = 80;
/// Lifetime of an unclaimed begin ticket.
pub(crate) const BEGIN_TTL_MS: u64 = 5_000;

/// Stable identity of a client process instance.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OwnerKey {
    pub(crate) pid: u32,
    pub(crate) creation_time: u64,
}

/// Runtime generation and monotonic session identifier.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct SessionKey {
    pub(crate) runtime_id: [u8; 16],
    pub(crate) generation: u64,
    pub(crate) id: u64,
}

/// Requested session duration, intent, and user-visible reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SessionSpec {
    pub(crate) duration_ms: u64,
    pub(crate) intent: PowerIntent,
    pub(crate) reason: String,
}

/// Read-only session state returned to callers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SessionView {
    pub(crate) key: SessionKey,
    pub(crate) owner_pid: u32,
    pub(crate) reason: String,
    pub(crate) intent: PowerIntent,
    pub(crate) remaining_ms: u64,
    pub(crate) revision: u64,
}

/// Bounded, clock-injected session state machine with no operating-system effects.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionLedger {
    runtime_id: [u8; 16],
    generation: u64,
    next_id: u64,
    admission_disabled: bool,
    manual: Option<PowerIntent>,
    entries: BTreeMap<SessionKey, LedgerEntry>,
}

#[derive(Clone, Debug)]
struct LedgerEntry {
    owner: OwnerKey,
    spec: Option<SessionSpec>,
    deadline: u64,
    revision: u64,
}

impl SessionLedger {
    /// Creates a ledger for one runtime generation and optional manual intent.
    pub(crate) fn new(runtime_id: [u8; 16], manual: Option<PowerIntent>) -> Self {
        Self {
            runtime_id,
            manual,
            ..Self::default()
        }
    }

    /// Reserves a short-lived ticket for an owner.
    pub(crate) fn begin(&mut self, owner: OwnerKey, now: u64) -> Result<SessionKey, String> {
        if self.admission_disabled {
            return Err("session admission is disabled".to_string());
        }
        if self.entries.len() >= MAX_SESSIONS {
            return Err("session capacity exhausted".to_string());
        }
        let deadline = now
            .checked_add(BEGIN_TTL_MS)
            .ok_or_else(|| "session begin deadline overflow".to_string())?;
        let id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "session identifier exhausted".to_string())?;
        self.next_id = id;
        let key = SessionKey {
            runtime_id: self.runtime_id,
            generation: self.generation,
            id,
        };
        self.entries.insert(
            key,
            LedgerEntry {
                owner,
                spec: None,
                deadline,
                revision: 0,
            },
        );
        Ok(key)
    }

    /// Claims a ticket with a bounded session specification.
    pub(crate) fn acquire(
        &mut self,
        owner: OwnerKey,
        key: SessionKey,
        mut spec: SessionSpec,
        now: u64,
    ) -> Result<SessionView, String> {
        let reason = validate_reason(&spec.reason)?;
        spec.reason = reason;
        validate_duration(spec.duration_ms)?;
        validate_current_key(self, key)?;
        let Some(entry) = self.entries.get(&key) else {
            return Err("session is not pending or active".to_string());
        };
        if entry.owner != owner {
            return Err("session owner mismatch".to_string());
        }
        if entry.deadline <= now {
            self.entries.remove(&key);
            return Err("session ticket expired".to_string());
        }
        if let Some(original) = &entry.spec {
            if *original != spec {
                return Err("session specification mismatch".to_string());
            }
            return Ok(view_for(key, entry, now));
        }
        let deadline = now
            .checked_add(spec.duration_ms)
            .ok_or_else(|| "session deadline overflow".to_string())?;
        let Some(entry) = self.entries.get_mut(&key) else {
            return Err("session disappeared during acquisition".to_string());
        };
        entry.spec = Some(spec);
        entry.deadline = deadline;
        entry.revision = 1;
        Ok(view_for(key, entry, now))
    }

    /// Extends an owned session when its revision is current.
    pub(crate) fn extend(
        &mut self,
        owner: OwnerKey,
        key: SessionKey,
        expected_revision: u64,
        duration_ms: u64,
        now: u64,
    ) -> Result<SessionView, String> {
        validate_duration(duration_ms)?;
        validate_current_key(self, key)?;
        let Some(entry) = self.entries.get(&key) else {
            return Err("session is not active".to_string());
        };
        if entry.owner != owner {
            return Err("session owner mismatch".to_string());
        }
        if entry.spec.is_none() {
            return Err("session is not active".to_string());
        }
        if entry.deadline <= now {
            self.entries.remove(&key);
            return Err("session expired".to_string());
        }
        if entry.revision != expected_revision {
            return Err("session revision mismatch".to_string());
        }
        let deadline = now
            .checked_add(duration_ms)
            .ok_or_else(|| "session deadline overflow".to_string())?;
        let next_revision = entry
            .revision
            .checked_add(1)
            .ok_or_else(|| "session revision exhausted".to_string())?;
        let Some(entry) = self.entries.get_mut(&key) else {
            return Err("session disappeared during extension".to_string());
        };
        entry.deadline = deadline;
        entry.revision = next_revision;
        Ok(view_for(key, entry, now))
    }

    /// Releases an owned session; a repeated release is idempotent.
    pub(crate) fn release(&mut self, owner: OwnerKey, key: SessionKey) -> Result<bool, String> {
        validate_current_key(self, key)?;
        let Some(entry) = self.entries.get(&key) else {
            return Ok(false);
        };
        if entry.owner != owner {
            return Err("session owner mismatch".to_string());
        }
        self.cancel_session(key)
    }

    /// Cancels a current key without changing manual intent or peer sessions.
    ///
    /// This is for trusted in-process controls. Pipe clients must keep using
    /// `release`, which checks process ownership before delegating here.
    pub(crate) fn cancel_session(&mut self, key: SessionKey) -> Result<bool, String> {
        validate_current_key(self, key)?;
        Ok(self.entries.remove(&key).is_some())
    }

    /// Cancels all tickets/sessions while preserving the manual intent.
    ///
    /// Advancing even an empty generation invalidates old starts. Exhaustion
    /// disables admission instead of allowing generation reuse.
    pub(crate) fn cancel_sessions(&mut self) {
        self.entries.clear();
        match self.generation.checked_add(1) {
            Some(next) => self.generation = next,
            None => self.admission_disabled = true,
        }
    }

    /// Removes expired tickets, expired sessions, and sessions whose owners exited.
    pub(crate) fn expire<F>(&mut self, now: u64, mut is_alive: F) -> bool
    where
        F: FnMut(OwnerKey) -> bool,
    {
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.deadline <= now || !is_alive(entry.owner)).then_some(*key)
            })
            .collect();
        let changed = !expired.is_empty();
        for key in expired {
            self.entries.remove(&key);
        }
        changed
    }

    /// Sets or clears the manual intent.
    pub(crate) fn set_manual(&mut self, manual: Option<PowerIntent>) {
        self.manual = manual;
    }

    /// Returns the manual intent, if one is selected.
    pub(crate) fn manual(&self) -> Option<PowerIntent> {
        self.manual
    }

    /// Returns the effective intent, excluding pending tickets.
    pub(crate) fn effective_intent(&self) -> Option<PowerIntent> {
        if self.manual == Some(PowerIntent::SystemAndDisplay)
            || self.entries.values().any(|entry| {
                entry
                    .spec
                    .as_ref()
                    .is_some_and(|spec| spec.intent == PowerIntent::SystemAndDisplay)
            })
        {
            Some(PowerIntent::SystemAndDisplay)
        } else if self.manual == Some(PowerIntent::SystemOnly)
            || self.entries.values().any(|entry| {
                entry
                    .spec
                    .as_ref()
                    .is_some_and(|spec| spec.intent == PowerIntent::SystemOnly)
            })
        {
            Some(PowerIntent::SystemOnly)
        } else {
            None
        }
    }

    /// Returns one live view, rechecking expiry at acknowledgement time.
    pub(crate) fn active_view(&self, key: SessionKey, now: u64) -> Option<SessionView> {
        self.entries
            .get(&key)
            .filter(|entry| entry.spec.is_some() && entry.deadline > now)
            .map(|entry| view_for(key, entry, now))
    }

    /// Returns active views with remaining durations evaluated at `now`.
    pub(crate) fn views(&self, now: u64) -> Vec<SessionView> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.spec.is_some() && entry.deadline > now)
            .map(|(key, entry)| view_for(*key, entry, now))
            .collect()
    }

    /// Returns unique owners currently represented by entries.
    pub(crate) fn owners(&self) -> Vec<OwnerKey> {
        self.entries
            .values()
            .map(|entry| entry.owner)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Reports whether pending or active entries exist.
    pub(crate) fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Cancels all intents and entries and advances the runtime generation.
    pub(crate) fn global_stop(&mut self) {
        self.manual = None;
        self.cancel_sessions();
    }
}

fn validate_current_key(ledger: &SessionLedger, key: SessionKey) -> Result<(), String> {
    if key.runtime_id != ledger.runtime_id || key.generation != ledger.generation {
        return Err("session belongs to another runtime generation".to_string());
    }
    Ok(())
}

fn validate_duration(duration_ms: u64) -> Result<(), String> {
    if (1..=MAX_DURATION_MS).contains(&duration_ms) {
        Ok(())
    } else {
        Err("session duration must be between 1 ms and 7 days".to_string())
    }
}

fn view_for(key: SessionKey, entry: &LedgerEntry, now: u64) -> SessionView {
    let spec = entry
        .spec
        .as_ref()
        .expect("views only contain active sessions");
    SessionView {
        key,
        owner_pid: entry.owner.pid,
        reason: spec.reason.clone(),
        intent: spec.intent,
        remaining_ms: entry.deadline.saturating_sub(now),
        revision: entry.revision,
    }
}

/// Validates and canonicalizes a session reason by trimming its outer whitespace.
pub(crate) fn validate_reason(reason: &str) -> Result<String, String> {
    if reason.chars().any(char::is_control) {
        return Err("session reason must not contain control characters".to_string());
    }
    let canonical = reason.trim();
    if canonical.is_empty() || canonical.chars().count() > MAX_REASON_CHARS {
        return Err("session reason must contain 1 to 80 characters".to_string());
    }
    Ok(canonical.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_view_never_reports_an_already_expired_session() {
        let mut ledger = SessionLedger::new([8; 16], None);
        let owner = OwnerKey {
            pid: 1,
            creation_time: 9,
        };
        let key = ledger.begin(owner, 100).unwrap();
        ledger
            .acquire(
                owner,
                key,
                SessionSpec {
                    duration_ms: 1,
                    intent: PowerIntent::SystemOnly,
                    reason: "Fixture".into(),
                },
                100,
            )
            .unwrap();
        assert_eq!(ledger.active_view(key, 100).unwrap().remaining_ms, 1);
        assert!(ledger.active_view(key, 101).is_none());
    }

    fn owner(pid: u32) -> OwnerKey {
        OwnerKey {
            pid,
            creation_time: 1,
        }
    }

    fn spec(duration_ms: u64, intent: PowerIntent, reason: &str) -> SessionSpec {
        SessionSpec {
            duration_ms,
            intent,
            reason: reason.to_string(),
        }
    }

    #[test]
    fn trusted_single_cancellation_preserves_manual_and_peer_sessions() {
        for manual in [
            None,
            Some(PowerIntent::SystemOnly),
            Some(PowerIntent::SystemAndDisplay),
        ] {
            let mut ledger = SessionLedger::new([30; 16], manual);
            let selected = ledger.begin(owner(80), 10).unwrap();
            let peer = ledger.begin(owner(81), 10).unwrap();
            ledger
                .acquire(
                    owner(80),
                    selected,
                    spec(100, PowerIntent::SystemOnly, "selected"),
                    10,
                )
                .unwrap();
            ledger
                .acquire(
                    owner(81),
                    peer,
                    spec(200, PowerIntent::SystemAndDisplay, "peer"),
                    10,
                )
                .unwrap();
            let peer_before = ledger.active_view(peer, 20).unwrap();

            assert!(ledger.cancel_session(selected).unwrap());
            assert!(!ledger.cancel_session(selected).unwrap());
            assert_eq!(ledger.manual(), manual);
            assert!(ledger.active_view(selected, 20).is_none());
            assert_eq!(ledger.active_view(peer, 20).unwrap(), peer_before);
            assert_eq!(ledger.owners(), vec![owner(81)]);
            assert_eq!(
                ledger.effective_intent(),
                Some(PowerIntent::SystemAndDisplay)
            );
            assert!(ledger.cancel_session(peer).unwrap());
            assert_eq!(ledger.effective_intent(), manual);
        }
    }

    #[test]
    fn trusted_pending_cancellation_cannot_be_acquired_again() {
        let mut ledger = SessionLedger::new([31; 16], Some(PowerIntent::SystemOnly));
        let cancelled = ledger.begin(owner(82), 10).unwrap();
        let peer = ledger.begin(owner(83), 10).unwrap();
        assert!(ledger.cancel_session(cancelled).unwrap());
        assert!(!ledger.cancel_session(cancelled).unwrap());
        assert!(ledger
            .acquire(
                owner(82),
                cancelled,
                spec(100, PowerIntent::SystemAndDisplay, "cancelled"),
                11,
            )
            .is_err());
        assert!(ledger.entries.contains_key(&peer));
        assert_eq!(ledger.owners(), vec![owner(83)]);
        assert_eq!(ledger.manual(), Some(PowerIntent::SystemOnly));
        let fresh = ledger.begin(owner(82), 11).unwrap();
        assert_ne!(fresh.id, cancelled.id);
        assert_eq!(fresh.generation, cancelled.generation);
    }

    #[test]
    fn trusted_cancel_all_preserves_manual_and_invalidates_active_and_pending_keys() {
        for manual in [
            None,
            Some(PowerIntent::SystemOnly),
            Some(PowerIntent::SystemAndDisplay),
        ] {
            let mut ledger = SessionLedger::new([32; 16], manual);
            let active = ledger.begin(owner(84), 10).unwrap();
            let pending = ledger.begin(owner(85), 10).unwrap();
            ledger
                .acquire(
                    owner(84),
                    active,
                    spec(100, PowerIntent::SystemAndDisplay, "active"),
                    10,
                )
                .unwrap();
            let generation = ledger.generation;

            ledger.cancel_sessions();
            assert_eq!(ledger.manual(), manual);
            assert_eq!(ledger.effective_intent(), manual);
            assert!(!ledger.has_entries());
            assert!(ledger.owners().is_empty());
            assert_eq!(ledger.generation, generation + 1);
            assert!(ledger.release(owner(84), active).is_err());
            assert!(ledger.extend(owner(84), active, 1, 200, 11).is_err());
            assert!(ledger
                .acquire(
                    owner(85),
                    pending,
                    spec(100, PowerIntent::SystemOnly, "old ticket"),
                    11,
                )
                .is_err());
            let fresh = ledger.begin(owner(84), 11).unwrap();
            assert_ne!(fresh.generation, active.generation);
            assert!(ledger
                .acquire(
                    owner(84),
                    fresh,
                    spec(100, PowerIntent::SystemOnly, "fresh"),
                    11,
                )
                .is_ok());
        }
    }

    #[test]
    fn trusted_cancellation_checks_epoch_and_keeps_owner_release_restricted() {
        let mut ledger = SessionLedger::new([33; 16], Some(PowerIntent::SystemOnly));
        let key = ledger.begin(owner(86), 10).unwrap();
        ledger
            .acquire(
                owner(86),
                key,
                spec(100, PowerIntent::SystemAndDisplay, "protected"),
                10,
            )
            .unwrap();
        let before = ledger.views(20);
        let mut another_runtime = key;
        another_runtime.runtime_id = [34; 16];
        let mut another_generation = key;
        another_generation.generation += 1;
        assert!(ledger.cancel_session(another_runtime).is_err());
        assert!(ledger.cancel_session(another_generation).is_err());
        assert!(ledger.release(owner(87), key).is_err());
        assert_eq!(ledger.views(20), before);
        assert!(ledger.release(owner(86), key).unwrap());
        assert_eq!(ledger.manual(), Some(PowerIntent::SystemOnly));
    }

    #[test]
    fn trusted_empty_cancellation_advances_generation_and_overflow_stays_cancelled() {
        let mut ledger = SessionLedger::new([35; 16], Some(PowerIntent::SystemAndDisplay));
        let generation = ledger.generation;
        ledger.cancel_sessions();
        assert_eq!(ledger.generation, generation + 1);
        assert_eq!(ledger.manual(), Some(PowerIntent::SystemAndDisplay));

        ledger.generation = u64::MAX;
        let active = ledger.begin(owner(88), 10).unwrap();
        let pending = ledger.begin(owner(89), 10).unwrap();
        ledger
            .acquire(
                owner(88),
                active,
                spec(100, PowerIntent::SystemOnly, "last generation"),
                10,
            )
            .unwrap();
        ledger.cancel_sessions();
        ledger.cancel_sessions();
        assert_eq!(ledger.generation, u64::MAX);
        assert!(ledger.admission_disabled);
        assert!(!ledger.has_entries());
        assert_eq!(ledger.manual(), Some(PowerIntent::SystemAndDisplay));
        assert!(ledger.begin(owner(90), 11).is_err());
        assert!(ledger
            .acquire(
                owner(89),
                pending,
                spec(100, PowerIntent::SystemOnly, "old pending"),
                11,
            )
            .is_err());
        ledger.global_stop();
        assert_eq!(ledger.manual(), None);
        assert_eq!(ledger.effective_intent(), None);
    }

    #[test]
    fn reason_validation_canonicalizes_and_rejects_invalid_input() {
        assert_eq!(validate_reason("  build  ").unwrap(), "build");
        assert!(validate_reason("").is_err());
        assert!(validate_reason("\n").is_err());
        assert!(validate_reason(&"x".repeat(MAX_REASON_CHARS + 1)).is_err());
    }

    #[test]
    fn overlapping_sessions_aggregate_and_release_independently() {
        let mut ledger = SessionLedger::new([7; 16], None);
        let first = ledger.begin(owner(10), 100).unwrap();
        let second = ledger.begin(owner(11), 100).unwrap();
        ledger
            .acquire(
                owner(10),
                first,
                spec(1000, PowerIntent::SystemOnly, "first"),
                100,
            )
            .unwrap();
        ledger
            .acquire(
                owner(11),
                second,
                spec(1000, PowerIntent::SystemAndDisplay, "second"),
                100,
            )
            .unwrap();
        assert_eq!(
            ledger.effective_intent(),
            Some(PowerIntent::SystemAndDisplay)
        );
        assert!(ledger.release(owner(10), first).unwrap());
        assert_eq!(
            ledger.effective_intent(),
            Some(PowerIntent::SystemAndDisplay)
        );
    }

    #[test]
    fn manual_intent_survives_session_expiry_and_global_stop_cancels_all() {
        let mut ledger = SessionLedger::new([8; 16], Some(PowerIntent::SystemAndDisplay));
        let key = ledger.begin(owner(12), 0).unwrap();
        ledger
            .acquire(
                owner(12),
                key,
                spec(10, PowerIntent::SystemOnly, "temporary"),
                0,
            )
            .unwrap();
        assert!(ledger.expire(10, |_| true));
        assert_eq!(ledger.manual(), Some(PowerIntent::SystemAndDisplay));
        ledger.global_stop();
        assert_eq!(ledger.effective_intent(), None);
        assert!(!ledger.has_entries());
    }

    #[test]
    fn pending_tickets_are_bounded_and_expire_at_the_deadline() {
        let mut ledger = SessionLedger::new([9; 16], None);
        for pid in 1..=MAX_SESSIONS {
            assert!(ledger.begin(owner(pid as u32), 0).is_ok());
        }
        assert!(ledger.begin(owner(999), 0).is_err());
        assert!(ledger.expire(BEGIN_TTL_MS, |_| true));
    }

    #[test]
    fn ownership_revision_and_idempotent_acknowledgements_are_checked() {
        let mut ledger = SessionLedger::new([1; 16], None);
        let key = ledger.begin(owner(20), 5).unwrap();
        let original = spec(100, PowerIntent::SystemOnly, "original");
        let view = ledger.acquire(owner(20), key, original.clone(), 5).unwrap();
        assert_eq!(view.revision, 1);
        let duplicate = ledger.acquire(owner(20), key, original.clone(), 6).unwrap();
        assert_eq!(duplicate.key, view.key);
        assert_eq!(duplicate.revision, view.revision);
        assert_eq!(duplicate.reason, view.reason);
        assert_eq!(duplicate.remaining_ms, view.remaining_ms - 1);
        assert!(ledger.extend(owner(20), key, 1, 200, 10).is_ok());
        let after_extend = ledger.acquire(owner(20), key, original, 11).unwrap();
        assert_eq!(after_extend.reason, "original");
        assert_eq!(after_extend.intent, PowerIntent::SystemOnly);
        assert_eq!(after_extend.revision, 2);
        assert_eq!(after_extend.remaining_ms, 199);
        assert!(ledger.extend(owner(20), key, 1, 300, 10).is_err());
        assert!(ledger.release(owner(21), key).is_err());
        assert!(ledger.release(owner(20), key).unwrap());
        assert!(!ledger.release(owner(20), key).unwrap());
    }

    #[test]
    fn owner_exit_and_clock_jumps_remove_only_expired_entries() {
        let mut ledger = SessionLedger::new([2; 16], None);
        let first = ledger.begin(owner(30), 0).unwrap();
        let second = ledger.begin(owner(31), 0).unwrap();
        ledger
            .acquire(
                owner(30),
                first,
                spec(300, PowerIntent::SystemOnly, "alive"),
                0,
            )
            .unwrap();
        ledger
            .acquire(
                owner(31),
                second,
                spec(200, PowerIntent::SystemOnly, "gone"),
                0,
            )
            .unwrap();
        assert!(ledger.expire(150, |current| current == owner(30)));
        assert_eq!(ledger.owners(), vec![owner(30)]);
    }

    #[test]
    fn reason_with_control_character_after_trim_is_rejected() {
        assert!(validate_reason("build\n").is_err());
        assert!(validate_reason("\tbuild").is_err());
    }

    #[test]
    fn active_entries_share_the_bounded_capacity() {
        let mut ledger = SessionLedger::new([3; 16], None);
        for pid in 1..=MAX_SESSIONS {
            let key = ledger.begin(owner(pid as u32), 10).unwrap();
            ledger
                .acquire(
                    owner(pid as u32),
                    key,
                    spec(MAX_DURATION_MS, PowerIntent::SystemOnly, "active"),
                    10,
                )
                .unwrap();
        }
        assert!(ledger.begin(owner(999), 10).is_err());
    }

    #[test]
    fn duration_and_ticket_deadline_overflow_are_rejected() {
        let mut ledger = SessionLedger::new([4; 16], None);
        let key = ledger.begin(owner(40), 100).unwrap();
        assert!(ledger
            .acquire(
                owner(40),
                key,
                spec(0, PowerIntent::SystemOnly, "zero"),
                100
            )
            .is_err());
        assert!(ledger
            .acquire(
                owner(40),
                key,
                spec(MAX_DURATION_MS + 1, PowerIntent::SystemOnly, "large"),
                100
            )
            .is_err());
        assert!(ledger
            .begin(owner(41), u64::MAX - BEGIN_TTL_MS + 1)
            .is_err());
    }

    #[test]
    fn reused_pid_requires_a_distinct_creation_identity() {
        let mut ledger = SessionLedger::new([5; 16], None);
        let old_owner = OwnerKey {
            pid: 50,
            creation_time: 1,
        };
        let new_owner = OwnerKey {
            pid: 50,
            creation_time: 2,
        };
        let key = ledger.begin(old_owner, 0).unwrap();
        ledger
            .acquire(old_owner, key, spec(100, PowerIntent::SystemOnly, "old"), 0)
            .unwrap();
        assert!(ledger.extend(new_owner, key, 1, 100, 1).is_err());
        assert!(ledger.release(new_owner, key).is_err());
    }

    #[test]
    fn release_and_expiry_permit_fresh_sessions_but_old_keys_cannot_rearm() {
        let mut ledger = SessionLedger::new([6; 16], None);
        let old = ledger.begin(owner(60), 0).unwrap();
        ledger
            .acquire(owner(60), old, spec(10, PowerIntent::SystemOnly, "old"), 0)
            .unwrap();
        assert!(ledger.release(owner(60), old).unwrap());
        let fresh = ledger.begin(owner(60), 1).unwrap();
        assert_ne!(fresh.id, old.id);
        ledger
            .acquire(
                owner(60),
                fresh,
                spec(10, PowerIntent::SystemOnly, "fresh"),
                1,
            )
            .unwrap();
        assert!(ledger
            .acquire(owner(60), old, spec(10, PowerIntent::SystemOnly, "old"), 1)
            .is_err());
        assert!(ledger.expire(11, |_| true));
        let after_expiry = ledger.begin(owner(60), 12).unwrap();
        assert_ne!(after_expiry.id, fresh.id);
    }

    #[test]
    fn global_stop_never_wraps_generation_or_reopens_exhausted_admission() {
        let mut ledger = SessionLedger::new([10; 16], None);
        ledger.generation = u64::MAX;
        let stopped =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ledger.global_stop()));
        assert!(stopped.is_ok());
        assert!(ledger.begin(owner(70), 0).is_err());
    }

    #[test]
    fn global_stop_invalidates_old_generation_and_preserves_manual_clear() {
        let mut ledger = SessionLedger::new([11; 16], Some(PowerIntent::SystemOnly));
        let old = ledger.begin(owner(71), 0).unwrap();
        ledger.global_stop();
        assert_eq!(ledger.manual(), None);
        assert!(ledger
            .acquire(
                owner(71),
                old,
                spec(10, PowerIntent::SystemOnly, "stale"),
                1
            )
            .is_err());
        let fresh = ledger.begin(owner(71), 1).unwrap();
        assert_ne!(old.generation, fresh.generation);
    }

    #[test]
    fn revision_overflow_does_not_mutate_deadline_or_view() {
        let mut ledger = SessionLedger::new([12; 16], None);
        let key = ledger.begin(owner(72), 0).unwrap();
        ledger
            .acquire(
                owner(72),
                key,
                spec(100, PowerIntent::SystemOnly, "stable"),
                0,
            )
            .unwrap();
        let entry = ledger.entries.get_mut(&key).unwrap();
        entry.revision = u64::MAX;
        let before = ledger.views(10).pop().unwrap();
        assert!(ledger.extend(owner(72), key, u64::MAX, 20, 10).is_err());
        assert_eq!(ledger.views(10).pop().unwrap(), before);
    }

    #[test]
    fn acquire_deadline_overflow_is_rejected_without_wrapping() {
        let mut ledger = SessionLedger::new([13; 16], None);
        let key = ledger.begin(owner(73), 0).unwrap();
        ledger.entries.get_mut(&key).unwrap().deadline = u64::MAX;
        assert!(ledger
            .acquire(
                owner(73),
                key,
                spec(2, PowerIntent::SystemOnly, "overflow"),
                u64::MAX - 1
            )
            .is_err());
        assert!(ledger.entries.get(&key).unwrap().spec.is_none());
    }

    #[test]
    fn extension_deadline_overflow_is_rejected_without_mutation() {
        let mut ledger = SessionLedger::new([14; 16], None);
        let key = ledger.begin(owner(74), 0).unwrap();
        ledger
            .acquire(
                owner(74),
                key,
                spec(100, PowerIntent::SystemOnly, "overflow"),
                0,
            )
            .unwrap();
        ledger.entries.get_mut(&key).unwrap().deadline = u64::MAX;
        let before = ledger.entries.get(&key).unwrap().clone();
        assert!(ledger.extend(owner(74), key, 1, 2, u64::MAX - 1).is_err());
        assert_eq!(ledger.entries.get(&key).unwrap().deadline, before.deadline);
        assert_eq!(ledger.entries.get(&key).unwrap().revision, before.revision);
    }

    #[test]
    fn exhausted_session_ids_do_not_wrap_or_admit_a_ticket() {
        let mut ledger = SessionLedger::new([15; 16], None);
        ledger.next_id = u64::MAX;
        assert!(ledger.begin(owner(75), 0).is_err());
        assert_eq!(ledger.next_id, u64::MAX);
    }

    #[test]
    fn keys_from_another_runtime_guid_are_stale() {
        let mut first = SessionLedger::new([16; 16], None);
        let key = first.begin(owner(76), 0).unwrap();
        let mut second = SessionLedger::new([17; 16], None);
        assert!(second
            .acquire(
                owner(76),
                key,
                spec(10, PowerIntent::SystemOnly, "foreign"),
                0
            )
            .is_err());
        first.global_stop();
        assert!(first
            .acquire(
                owner(76),
                key,
                spec(10, PowerIntent::SystemOnly, "stale"),
                1
            )
            .is_err());
    }
}
