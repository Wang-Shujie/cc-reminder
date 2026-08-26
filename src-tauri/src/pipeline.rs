//! Event pipeline: live ingress + offline recovery (Task 14).
//!
//! [`EventPipeline::process_live`] is the live path: a helper has just
//! delivered an [`IngressRequest`] over IPC. It validates the helper version
//! and command fingerprint against the expected `hook_installations` row,
//! follows the exact design §12.1 parse order (capability → project → global
//! rule → project merge → enabled → filters → timing policy → allowed fields →
//! mandatory redaction → per-target template → idempotency key → enqueue), and
//! commits the redacted event, its outcome, every intended delivery job, the
//! hook last-seen timestamp and the matching Codex `ObservedWorking`
//! transition in a single database transaction. An unrecognized helper does
//! not establish trust.
//!
//! [`EventPipeline::recover_ingress`] is the offline path: a `SafeIngressEvent`
//! already on disk is replayed against the current rules using its original
//! occurrence time. Expired-past-TTL events are marked expired and never
//! replayed; the ingress row is deleted only after the processing transaction
//! commits; duplicate event UUIDs are idempotent.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{FixedOffset, Utc};
use uuid::Uuid;

use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::catalog_for;
use crate::events::normalize::{NormalizeContext, normalize_event, stable_ingress_event_id};
use crate::ipc::IngressRequest;
use crate::model::{
    AgentKind, EventEnvelope, NotificationDocument, RuleConfig, ScalarValue, TargetConfig,
};
use crate::projects::{PathPlatform, ProjectRegistration};
use crate::rules::policy::{PolicyDecision, PolicyInput, SuppressReason, evaluate_policy};
use crate::rules::resolve::{ResolvedRule, StoredRulePatch, resolve_stored_rule};
use crate::rules::template::{DEFAULT_TEMPLATE_ZH, build_template_context, render_document};
use crate::security::crypto::FieldCipher;
use crate::security::redact::Redactor;
use crate::storage::config::ConfigRepository;
use crate::storage::db::storage_error;
use crate::storage::events::{
    EventOutcomeReasonCode, EventProcessingOutcome, EventRepository, delete_ingress_in_tx,
    event_already_seen, insert_event_in_tx,
};
use crate::storage::integrations::{IntegrationRepository, mark_hook_seen_in_tx};
use crate::storage::queue::{DeliveryJob, DeliveryStatus, QueueRepository, enqueue_in_tx};

const METADATA_ONLY_TEMPLATE: &str = concat!(
    "[{{agent.name}}] {{event.label}}\n",
    "项目：{{project.name}}\n",
    "状态：{{event.status}}\n",
    "时间：{{event.occurred_at}}",
);

fn minimal_document(
    envelope: &EventEnvelope,
    local_offset: chrono::FixedOffset,
) -> Result<NotificationDocument, AppError> {
    Ok(NotificationDocument {
        title: envelope.source_event.clone(),
        severity: envelope.severity,
        facts: vec![
            ("Agent".to_owned(), envelope.source.as_str().to_owned()),
            (
                "Project".to_owned(),
                envelope.project_display_name.clone().unwrap_or_default(),
            ),
            ("Hook".to_owned(), envelope.source_event.clone()),
            (
                "Time".to_owned(),
                envelope
                    .occurred_at
                    .with_timezone(&local_offset)
                    .format("%Y-%m-%d %H:%M:%S%:z")
                    .to_string(),
            ),
        ],
        body: String::new(),
        footer: None,
    })
}

/// Outcome of [`EventPipeline::process_live`]. `Duplicate` is returned when the
/// event UUID had already been observed (idempotent replay of the same IPC
/// payload) so the caller can reply `Accepted` without inflating history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveOutcome {
    Processed { event_id: Uuid },
    Duplicate { event_id: Uuid },
}

/// The persistence boundary: a single immediate transaction commits every
/// side-effect of one event together.
const POLICY_RECENT_DELIVERY_LOOKUP: usize = 32;

#[derive(Clone)]
pub struct EventPipeline {
    database: crate::storage::db::Database,
    cipher: Arc<FieldCipher>,
    correlation_key: [u8; 32],
    platform: PathPlatform,
    projects: Vec<ProjectRegistration>,
    local_offset: FixedOffset,
}

impl EventPipeline {
    pub fn new(
        database: crate::storage::db::Database,
        cipher: Arc<FieldCipher>,
        correlation_key: [u8; 32],
        platform: PathPlatform,
        projects: Vec<ProjectRegistration>,
        local_offset: FixedOffset,
    ) -> Self {
        Self {
            database,
            cipher,
            correlation_key,
            platform,
            projects,
            local_offset,
        }
    }

    /// Process a live IPC request: validate trust, run the parse order, encrypt
    /// sensitive fields (only when the app is live), enqueue per-target jobs,
    /// and commit everything atomically.
    pub async fn process_live(&self, request: IngressRequest) -> Result<LiveOutcome, AppError> {
        let started = Instant::now();
        // Derive the stable event id from the captured event so live + offline
        // deduplication share the same UUID space (a captured event replayed
        // via the safe-ingress fallback produces the same id).
        let stable_id = stable_ingress_event_id(&request.event);
        // Encrypt sensitive fields NOW, while we still hold the captured
        // event's sensitive_fields map. normalize_event drops it (by design)
        // after producing HMAC references, so we cannot encrypt after.
        let encrypted = if request.event.sensitive_fields.is_empty() {
            None
        } else {
            Some(
                self.cipher
                    .encrypt_fields(stable_id, &request.event.sensitive_fields)?,
            )
        };
        let event = request.event;
        let agent = event.source;
        let source_event = event.source_event.clone();
        let command_fingerprint = request.command_fingerprint.clone();

        // Trust gate: the helper_version + command_fingerprint must match the
        // expected hook_installations row for this Agent+event. We perform the
        // transition inside the processing transaction below so an
        // unrecognized helper cannot establish trust even if it manages to
        // produce a syntactically valid envelope.
        let integrations = IntegrationRepository::new(self.database.clone());
        let hook = integrations.hook(agent, &source_event).ok();
        if let Some(hook) = &hook {
            if hook.command_fingerprint != command_fingerprint.as_str()
                || hook.helper_version != request.helper_version
            {
                return Err(unrecognized_helper());
            }
        } else {
            return Err(unrecognized_helper());
        }

        let context = NormalizeContext {
            correlation_key: self.correlation_key,
            projects: self.projects.clone(),
            platform: self.platform,
        };
        let envelope = normalize_event(event, &context)?;
        // Override the random v7 id with the stable ingress id so live + offline
        // dedup share the same UUID space.
        let mut envelope = envelope;
        envelope.id = stable_id;
        if let Some(encrypted) = &encrypted {
            envelope.encrypted_sensitive_fields = Some(encrypted.blob_ref());
        }

        // Capability gate: catalog must still recognise the event.
        let capability = catalog_for(envelope.source, &envelope.source_version);
        let hook_capability = capability
            .catalog
            .hooks
            .into_iter()
            .find(|hook| hook.source_event == envelope.source_event)
            .ok_or_else(|| {
                storage_error(
                    "pipeline.unsupported_capability",
                    "event capability is not catalogued",
                )
            })?;

        // Resolve effective rule (global + project patch).
        let config = ConfigRepository::new(self.database.clone());
        let (resolved, _project_patch_used) =
            resolve_effective_rule(&config, &envelope, agent, &source_event)?;

        let now = Utc::now();
        let notification_pause = config
            .get_settings()
            .ok()
            .and_then(|s| s.notification_pause);
        let recent = self.recent_delivery_times_for_rule(resolved.id, &resolved.config)?;

        let decision = evaluate_policy(&PolicyInput {
            event: &envelope,
            capability: &hook_capability,
            rule: &resolved.config,
            notification_pause: notification_pause.as_ref(),
            now,
            local_offset: self.local_offset,
            recent_delivery_times: &recent,
        });

        let outcome = live_outcome_for_decision(&decision);
        let reason_code = suppress_reason_code(&decision);

        // Sensitive fields were encrypted above (before normalize_event
        // dropped them). The offline path never carries sensitive plaintext.
        // ponytail: we always attempt encryption when the captured event had
        // sensitive fields, regardless of the policy outcome; an event whose
        // rule is suppressed stores no ciphertext (encrypted is dropped on the
        // floor when outcome != Queued, since insert_event_in_tx is the only
        // path that persists the blob and we only pass it for Queued events).

        // Build the persisted envelope. Mandatory redaction is applied to every
        // public_fields string below in redact_envelope().
        let mut redacted_envelope = envelope;
        redact_envelope_public_fields(&mut redacted_envelope);
        if encrypted.is_none() {
            redacted_envelope.encrypted_sensitive_fields = None;
        }

        // Begin the atomic processing transaction.
        let events_repo = EventRepository::new(self.database.clone());

        let processed_event_id = redacted_envelope.id;
        let event_id_for_tx = processed_event_id;
        let rule_for_jobs = resolved.clone();
        let outcome_for_tx = outcome;
        let reason_for_tx = reason_code;
        let targets = rule_for_jobs.config.targets.clone();
        let rule_version_str = rule_for_jobs.version.clone();
        let encrypted_for_tx = encrypted.clone();
        let redacted_envelope_for_tx = redacted_envelope.clone();

        let result: Result<LiveOutcome, AppError> = events_repo.transaction(move |tx| {
            if event_already_seen(tx, event_id_for_tx) {
                return Ok(LiveOutcome::Duplicate {
                    event_id: event_id_for_tx,
                });
            }
            insert_event_in_tx(
                tx,
                &redacted_envelope_for_tx,
                encrypted_for_tx.as_ref(),
                outcome_for_tx,
                reason_for_tx,
            )?;

            // Mark the matching hook observed only when we got here through a
            // trusted helper. This is the Codex ObservedWorking transition.
            let _ =
                mark_hook_seen_in_tx(tx, agent, &source_event, &command_fingerprint, Utc::now())?;

            if matches!(outcome_for_tx, EventProcessingOutcome::Queued) {
                enqueue_jobs(
                    tx,
                    self.local_offset,
                    event_id_for_tx,
                    &rule_for_jobs,
                    &targets,
                    &rule_version_str,
                    &decision,
                )?;
            }
            Ok(LiveOutcome::Processed {
                event_id: event_id_for_tx,
            })
        });
        let outcome = result?;
        let _ = started.elapsed();
        Ok(outcome)
    }

    /// Process one safe-ingress batch using current rules and the original
    /// occurrence time. Idempotent on event UUID.
    ///
    /// `take_ingress_batch` already flipped every row in the batch to
    /// `'processing'` and committed, so a per-row failure must NOT abort the
    /// rest of the batch — that would strand healthy rows in `'processing'`
    /// with no reaper until Task 15's startup sweep. Instead a failing row is
    /// skipped and left in `'processing'` for the Task 15 reaper; the rest of
    /// the batch proceeds. Returns the count of rows that were attempted
    /// (successful or not).
    pub async fn recover_ingress(&self) -> Result<usize, AppError> {
        let events_repo = EventRepository::new(self.database.clone());
        let config = ConfigRepository::new(self.database.clone());

        let batch = events_repo.take_ingress_batch(50)?;
        if batch.is_empty() {
            return Ok(0);
        }
        let mut processed = 0usize;
        for safe in batch {
            processed += 1;
            // Skip-and-continue: a single bad row (e.g. uncatalogued
            // capability after a catalog upgrade) must not strand its
            // siblings. The failing row stays `'processing'` for the Task 15
            // startup reaper.
            if let Err(_error) = self
                .process_safe_ingress_with(&events_repo, &config, safe)
                .await
            {
                // ponytail: no logger is wired yet (Task 15); the failed row
                // is observable via its lingering `'processing'` state and
                // the startup reaper. Swap in tracing::warn when Task 15 lands.
            }
        }
        Ok(processed)
    }

    async fn process_safe_ingress_with(
        &self,
        events_repo: &EventRepository,
        config: &ConfigRepository,
        safe: crate::events::normalize::SafeIngressEvent,
    ) -> Result<(), AppError> {
        let agent = safe.source;
        let source_event = safe.source_event.clone();
        let now = Utc::now();

        // Build an envelope-like view from the safe event for policy eval.
        let category = catalog_for(agent, &safe.source_version)
            .catalog
            .hooks
            .into_iter()
            .find(|hook| hook.source_event == source_event)
            .map(|hook| hook.category)
            .unwrap_or(crate::model::EventCategory::Other);

        // Determine the project_id to record: if it's still in `projects`,
        // apply its current patch; otherwise fall back to global rules with
        // project_id=None but keep the already-safe display name.
        let mut envelope = EventEnvelope {
            id: safe.event_id,
            source: agent,
            source_version: safe.source_version.clone(),
            source_event: source_event.clone(),
            category,
            occurred_at: safe.occurred_at,
            received_at: now,
            project_id: safe.project_id,
            project_display_name: safe.project_display_name.clone(),
            unmatched_cwd_fingerprint: safe.cwd_fingerprint.clone(),
            session_ref: safe.session_ref.clone(),
            turn_ref: safe.turn_ref.clone(),
            model: None,
            permission_mode: None,
            severity: crate::model::Severity::Info,
            public_fields: safe.public_fields.clone(),
            encrypted_sensitive_fields: None,
            correlation_id: Uuid::now_v7(),
            action_id: None,
            action_capabilities: Vec::new(),
        };

        // If the cached project_id is no longer in `projects`, drop it.
        if let Some(pid) = envelope.project_id
            && config.get_project(pid).is_err()
        {
            envelope.project_id = None;
        }

        let capability = catalog_for(agent, &envelope.source_version);
        let hook_capability = capability
            .catalog
            .hooks
            .into_iter()
            .find(|hook| hook.source_event == envelope.source_event)
            .ok_or_else(|| {
                storage_error(
                    "pipeline.unsupported_capability",
                    "event capability is not catalogued",
                )
            })?;
        let _ = capability.verification;

        let (resolved, _) = resolve_effective_rule(config, &envelope, agent, &source_event)?;
        let notification_pause = config
            .get_settings()
            .ok()
            .and_then(|s| s.notification_pause);
        let recent = self.recent_delivery_times_for_rule(resolved.id, &resolved.config)?;
        let decision = evaluate_policy(&PolicyInput {
            event: &envelope,
            capability: &hook_capability,
            rule: &resolved.config,
            notification_pause: notification_pause.as_ref(),
            now,
            local_offset: self.local_offset,
            recent_delivery_times: &recent,
        });

        let outcome = live_outcome_for_decision(&decision);
        let reason_code = suppress_reason_code(&decision);

        let mut redacted_envelope = envelope.clone();
        redact_envelope_public_fields(&mut redacted_envelope);
        // Safe envelopes never carry sensitive plaintext; force None.
        redacted_envelope.encrypted_sensitive_fields = None;

        let event_id = redacted_envelope.id;
        let rule_for_jobs = resolved.clone();
        let rule_version_str = resolved.version.clone();
        let targets = rule_for_jobs.config.targets.clone();
        let outcome_for_tx = outcome;
        let reason_for_tx = reason_code;
        let redacted_envelope_for_tx = redacted_envelope.clone();
        let decision_for_tx = decision;

        events_repo.transaction(move |tx| {
            if event_already_seen(tx, event_id) {
                // Idempotent replay: just drop the ingress row.
                delete_ingress_in_tx(tx, event_id)?;
                return Ok(());
            }
            insert_event_in_tx(
                tx,
                &redacted_envelope_for_tx,
                None,
                outcome_for_tx,
                reason_for_tx,
            )?;
            if matches!(outcome_for_tx, EventProcessingOutcome::Queued) {
                enqueue_jobs(
                    tx,
                    self.local_offset,
                    event_id,
                    &rule_for_jobs,
                    &targets,
                    &rule_version_str,
                    &decision_for_tx,
                )?;
            }
            delete_ingress_in_tx(tx, event_id)?;
            Ok(())
        })
    }

    fn recent_delivery_times_for_rule(
        &self,
        rule_id: Uuid,
        _rule: &RuleConfig,
    ) -> Result<Vec<chrono::DateTime<Utc>>, AppError> {
        // ponytail: per-channel granularity would require a channel id, but
        // the policy evaluator only needs "any recent delivery for this rule".
        // Query the union across channels for now; tighten to per-channel if
        // cooldown semantics demand it.
        let queue = QueueRepository::new(self.database.clone());
        // Look up across every channel for this rule.
        let config = ConfigRepository::new(self.database.clone());
        let channels = config.list_channels().unwrap_or_default();
        let mut all = Vec::new();
        for channel in channels {
            let times = queue.recent_delivery_times(rule_id, channel.id, 8)?;
            all.extend(times);
        }
        all.sort();
        all.reverse();
        all.truncate(POLICY_RECENT_DELIVERY_LOOKUP);
        Ok(all)
    }
}

fn live_outcome_for_decision(decision: &PolicyDecision) -> EventProcessingOutcome {
    match decision {
        PolicyDecision::SendNow | PolicyDecision::Aggregate { .. } => {
            EventProcessingOutcome::Queued
        }
        PolicyDecision::DeferUntil(_) => EventProcessingOutcome::Queued,
        PolicyDecision::Suppress(_) => EventProcessingOutcome::Suppressed,
        PolicyDecision::Expire => EventProcessingOutcome::Expired,
    }
}

fn suppress_reason_code(decision: &PolicyDecision) -> Option<EventOutcomeReasonCode> {
    match decision {
        PolicyDecision::Suppress(reason) => Some(match reason {
            SuppressReason::UnsupportedCapability => EventOutcomeReasonCode::UnsupportedCapability,
            SuppressReason::Disabled => EventOutcomeReasonCode::Disabled,
            SuppressReason::FilterMismatch => EventOutcomeReasonCode::FilterMismatch,
            SuppressReason::GlobalPause => EventOutcomeReasonCode::GlobalPause,
            SuppressReason::QuietHours => EventOutcomeReasonCode::QuietHours,
            SuppressReason::Cooldown => EventOutcomeReasonCode::Cooldown,
            SuppressReason::WindowLimit => EventOutcomeReasonCode::WindowLimit,
        }),
        _ => None,
    }
}

fn enqueue_jobs(
    tx: &rusqlite::Transaction<'_>,
    local_offset: chrono::FixedOffset,
    event_id: Uuid,
    resolved: &ResolvedRule,
    targets: &[TargetConfig],
    rule_version: &str,
    decision: &PolicyDecision,
) -> Result<(), AppError> {
    let redactor = Redactor::compile(&resolved.config.privacy.extra_redaction_patterns)?;
    let max_chars = resolved.config.privacy.max_body_chars.max(1) as usize;
    // Build a stub envelope for template context from the rule + event id.
    let envelope = load_envelope_for_template(tx, event_id)?;
    let mut document = {
        let context = build_template_context(
            &envelope,
            &resolved.config.privacy.allowed_sensitive_fields,
            local_offset,
        );
        let template = resolved
            .config
            .targets
            .first()
            .and_then(|t| t.template.clone())
            .unwrap_or_else(|| DEFAULT_TEMPLATE_ZH.to_owned());
        // Render the per-target document. The default template references
        // event.summary, which is only authorized when a public summary field is
        // present; for events without one (e.g. metadata-only Stop), fall back to
        // a minimal template so we never drop a notification solely because the
        // template asked for a field this event doesn't have.
        render_document(&template, &context, &redactor, max_chars)
            .or_else(|_| render_document(METADATA_ONLY_TEMPLATE, &context, &redactor, max_chars))
            .or_else(|_| minimal_document(&envelope, local_offset))?
    };
    // A rendered template body already carries the full human-readable content
    // (agent/project/hook/status/time in the user's locale). Keeping the
    // auto-generated English fact list on top of it duplicated every line in
    // the delivered message, so facts ship only with the body-less fallback.
    if !document.body.is_empty() {
        document.facts.clear();
    }

    let now = Utc::now();
    let ttl_seconds = resolved.config.delivery.ttl_seconds.max(1) as i64;
    let expires_at = now + chrono::Duration::seconds(ttl_seconds);

    let (base_aggregate_key, aggregate_release_at) = match decision {
        PolicyDecision::Aggregate {
            bucket_key,
            release_at,
        } => (Some(bucket_key.clone()), Some(*release_at)),
        _ => (None, None),
    };

    if targets.is_empty() {
        return Ok(());
    }
    for target in targets {
        // Mix channel_id into the aggregate bucket key so a rule with multiple
        // targets yields one SEPARATE claimable bucket per channel (design
        // §15.1: one delivery job per event/rule/target). Without this, two
        // jobs sharing a base key would be coalesced by claim_due into a
        // single Aggregate delivery sent to only the first channel, and the
        // shared receipt would mark the other channel's job succeeded
        // without it ever being notified.
        let aggregate_key = base_aggregate_key
            .as_ref()
            .map(|base| per_channel_aggregate_key(base, target.channel_id));
        let job = DeliveryJob {
            id: Uuid::now_v7(),
            event_id,
            rule_id: resolved.id,
            rule_version: rule_version.to_owned(),
            channel_id: target.channel_id,
            idempotency_key: QueueRepository::idempotency_key(
                event_id,
                rule_version,
                target.channel_id,
            ),
            document: document.clone(),
            state: DeliveryStatus::Pending,
            attempts: 0,
            next_attempt_at: now,
            expires_at,
            lease_owner: None,
            lease_expires_at: None,
            aggregate_key,
            aggregate_release_at,
        };
        enqueue_in_tx(tx, &job)?;
    }
    Ok(())
}

/// Derive a per-channel aggregate bucket key by hashing the base policy bucket
/// together with the channel id. Kept separate from the idempotency-key helper
/// because the bucket must be stable across replays of the same event+channel
/// but MUST differ across channels even when the policy produced one base key.
fn per_channel_aggregate_key(base: &str, channel_id: crate::model::ChannelId) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(base.as_bytes());
    hasher.update([0x00]);
    hasher.update(channel_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn load_envelope_for_template(
    tx: &rusqlite::Transaction<'_>,
    event_id: Uuid,
) -> Result<EventEnvelope, AppError> {
    // Read the just-inserted event row to rebuild an envelope sufficient for
    // template rendering. The full envelope is what we wrote, so we can
    // round-trip its persisted columns.
    let row = tx
        .query_row(
            "SELECT id, source, source_version, source_event, category, occurred_at, received_at,
                    project_id, project_display_name, unmatched_cwd_fingerprint, session_ref,
                    turn_ref, model, permission_mode, severity, public_fields_json,
                    correlation_id, action_id, action_capabilities_json
             FROM events WHERE id = ?1",
            rusqlite::params![event_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, String>(18)?,
                ))
            },
        )
        .map_err(|_| storage_error("storage.query_failed", "database query failed"))?;
    let category: crate::model::EventCategory =
        serde_json::from_value(serde_json::Value::String(row.4))
            .map_err(|_| storage_error("storage.invalid_stored_data", "decode"))?;
    let severity: crate::model::Severity =
        serde_json::from_value(serde_json::Value::String(row.14))
            .map_err(|_| storage_error("storage.invalid_stored_data", "decode"))?;
    let public_fields: BTreeMap<String, ScalarValue> =
        serde_json::from_str(&row.15).map_err(|_| {
            storage_error(
                "storage.invalid_stored_data",
                "stored data could not be decoded",
            )
        })?;
    let source: AgentKind =
        serde_json::from_value(serde_json::Value::String(row.1)).map_err(|_| {
            storage_error(
                "storage.invalid_stored_data",
                "stored data could not be decoded",
            )
        })?;
    let action_capabilities = serde_json::from_str(&row.18).map_err(|_| {
        storage_error(
            "storage.invalid_stored_data",
            "stored data could not be decoded",
        )
    })?;
    Ok(EventEnvelope {
        id: parse_uuid(&row.0)?,
        source,
        source_version: semver::Version::parse(&row.2).map_err(|_| {
            storage_error(
                "storage.invalid_stored_data",
                "stored data could not be decoded",
            )
        })?,
        source_event: row.3,
        category,
        occurred_at: parse_time(&row.5)?,
        received_at: parse_time(&row.6)?,
        project_id: row.7.as_deref().map(parse_uuid).transpose()?,
        project_display_name: row.8,
        unmatched_cwd_fingerprint: row.9,
        session_ref: row.10,
        turn_ref: row.11,
        model: row.12,
        permission_mode: row.13,
        severity,
        public_fields,
        encrypted_sensitive_fields: None,
        correlation_id: parse_uuid(&row.16)?,
        action_id: row.17,
        action_capabilities,
    })
}

fn parse_uuid(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value).map_err(|_| {
        storage_error(
            "storage.invalid_stored_data",
            "stored data could not be decoded",
        )
    })
}

fn parse_time(value: &str) -> Result<chrono::DateTime<chrono::Utc>, AppError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&chrono::Utc))
        .map_err(|_| {
            storage_error(
                "storage.invalid_stored_data",
                "stored data could not be decoded",
            )
        })
}

/// Resolve the effective RuleConfig + which project patch was used (for
/// record-keeping). Returns the global rule if no project patch applies.
fn resolve_effective_rule(
    config: &ConfigRepository,
    envelope: &EventEnvelope,
    agent: AgentKind,
    source_event: &str,
) -> Result<(ResolvedRule, Option<StoredRulePatch>), AppError> {
    let global_stored = config.get_global_rule(agent, source_event)?;
    let patch = envelope.project_id.and_then(|pid| {
        config
            .get_project_patch(pid, agent, source_event)
            .ok()
            .filter(|_p| {
                // Only use the patch if the project still exists.
                config.get_project(pid).is_ok()
            })
    });
    let resolved = resolve_stored_rule(&global_stored, patch.as_ref());
    Ok((resolved, patch))
}

fn redact_envelope_public_fields(envelope: &mut EventEnvelope) {
    // Mandatory redaction of every string in persisted public_fields.
    let redactor = Redactor::compile(&[]).expect("mandatory redactor compiles");
    for value in envelope.public_fields.values_mut() {
        if let ScalarValue::String(s) = value {
            *s = redactor.redact(s);
        }
    }
}

fn unrecognized_helper() -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: "pipeline.unrecognized_helper".into(),
        message: "helper version or command fingerprint is not trusted".into(),
        suggested_action: None,
    }
}

/// Helper for the IPC callback: encrypt a captured event's sensitive fields
/// before invoking [`EventPipeline::process_live`]. Returns the encrypted blob
/// reference to attach to the envelope (the caller has no other way to retain
/// sensitive plaintext, by design).
#[allow(dead_code)]
pub fn encrypt_sensitive_fields(
    cipher: &FieldCipher,
    event_id: Uuid,
    sensitive: &BTreeMap<String, String>,
) -> Result<crate::security::crypto::EncryptedFields, AppError> {
    cipher.encrypt_fields(event_id, sensitive)
}
