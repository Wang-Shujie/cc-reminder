//! Full checked Hook mutation transaction (Task 11, design 9.4).
//!
//! [`HookInstaller`] orchestrates every owned-Hook mutation as a single checked
//! transaction: acquire the app lock, re-read/hash and STOP on drift, produce a
//! structured patch, encrypt ONLY the previous `hooks` subtree + source hash +
//! mode into `config_snapshots`, install/verify the helper, atomically replace
//! the config, re-parse to confirm the exact owned entries, then record
//! `hook_installations` rows carrying SEPARATE command and definition
//! fingerprints. If the encryption store is unavailable the mutation aborts
//! BEFORE any Agent config is written.
//!
//! Trust semantics (design 9.3): Claude entries are `NotRequired`. A Codex entry
//! is `NeedsUserConfirmation` until an ingress request for the same agent/event
//! arrives carrying the stored expected command fingerprint, at which point it
//! becomes `ObservedWorking`. A binary-only helper replacement at the SAME
//! canonical command path preserves both fingerprints and therefore trust; any
//! change to a serialized definition field (command, matcher, timeout,
//! commandWindows) changes a fingerprint and resets the affected entry to
//! `NeedsUserConfirmation`. The bypass flag is never used.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agents::{
    EntryHealth, HealthAggregate, HookEntryHealth, HookHealth, HookSelection, Installation,
};
use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::CatalogVerification;
use crate::installer::helper::HelperInstaller;
use crate::installer::{
    ConfigPatch, OwnedHookEntry, atomic_replace_checked, hook_definition_fingerprint,
    inspect_owned_entries, owned_command, patch_claude_settings, patch_codex_hooks, sha256_hex,
};
use crate::model::{AgentKind, HookInstallationRecord, InstallationHealth, TrustStatus};
use crate::security::crypto::{FieldCipher, snapshot_aad};
use crate::security::permissions::ensure_private_directory;
use crate::storage::integrations::IntegrationRepository;

/// Exact lifecycle action. Rule-save commands never implicitly trigger any of
/// these; a selection change is an explicit `Repair`, not a side effect of a
/// setter (design 8.4, 9.4 — the constraint lands on the future command layer,
/// but [`HookInstaller`] is shaped so a selection change can only happen here).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookAction {
    Install,
    Repair,
    UpgradeHelper,
    Uninstall,
}

/// Wired dependencies needed to mutate/inspect Hooks. The shell constructs one
/// of these once the data key and helper are known; `cipher = None` models an
/// unavailable encryption store and blocks every mutation.
#[derive(Clone)]
pub struct HookEnvironment {
    pub repository: IntegrationRepository,
    pub cipher: Option<FieldCipher>,
    pub helper: HelperInstaller,
    pub home: PathBuf,
    pub codex_home: Option<PathBuf>,
}

#[derive(Clone)]
pub struct HookInstaller {
    agent: AgentKind,
    config_path: PathBuf,
    repository: IntegrationRepository,
    cipher: Option<FieldCipher>,
    helper: HelperInstaller,
}

impl HookInstaller {
    pub fn new(
        agent: AgentKind,
        config_path: PathBuf,
        repository: IntegrationRepository,
        cipher: Option<FieldCipher>,
        helper: HelperInstaller,
    ) -> Self {
        Self {
            agent,
            config_path,
            repository,
            cipher,
            helper,
        }
    }

    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    /// Read-only health of the owned Hooks for `selection`. Reports per-entry
    /// health + trust, the aggregate, and whether the installed owned events
    /// differ from the required selection.
    pub fn inspect(&self, selection: &HookSelection) -> Result<HookHealth, AppError> {
        let bytes = fs::read(&self.config_path).unwrap_or_default();
        let installed = inspect_owned_entries(self.agent, &bytes)?;
        let mut installed_by_event: std::collections::BTreeMap<String, OwnedHookEntry> =
            std::collections::BTreeMap::new();
        for entry in &installed {
            installed_by_event.insert(entry.source_event.clone(), entry.clone());
        }
        let installed_events: std::collections::BTreeSet<String> =
            installed_by_event.keys().cloned().collect();

        let agent_upgrade_required = self.repository.agent(self.agent).is_ok_and(|record| {
            record.capability_verification == CatalogVerification::UpgradeRequired
        });

        let mut entries = Vec::new();
        for event in &selection.events {
            let row = self.repository.hook(self.agent, event).ok();
            let Some(installed_entry) = installed_by_event.get(event) else {
                // Desired but absent on disk.
                entries.push(HookEntryHealth {
                    source_event: event.clone(),
                    command_fingerprint: String::new(),
                    definition_fingerprint: String::new(),
                    trust_status: row
                        .as_ref()
                        .map(|r| r.trust_status)
                        .unwrap_or(self.default_trust()),
                    health: if agent_upgrade_required {
                        EntryHealth::AgentUpgradeRequired
                    } else {
                        EntryHealth::Missing
                    },
                });
                continue;
            };
            // Compare against what a LIVE helper reports for itself (the
            // canonical stable-path command, Windows component included) — not
            // against the parsed-back config entry, whose schema may have
            // dropped fields like Claude's commandWindows.
            let cmd_fp = crate::hook_command::command_fingerprint(&owned_command(
                &selection.helper_path,
                self.agent,
                event,
            ));
            let def_fp = hook_definition_fingerprint(self.agent, installed_entry);
            let health = entry_health(
                row.as_ref(),
                &cmd_fp,
                &def_fp,
                self.agent,
                agent_upgrade_required,
            );
            let trust = row
                .as_ref()
                .map(|r| r.trust_status)
                .unwrap_or(self.default_trust());
            entries.push(HookEntryHealth {
                source_event: event.clone(),
                command_fingerprint: cmd_fp,
                definition_fingerprint: def_fp,
                trust_status: trust,
                health,
            });
        }

        let selection_out_of_date = installed_events != *selection.events();
        let aggregate = aggregate_health(&entries, selection_out_of_date, agent_upgrade_required);

        Ok(HookHealth {
            agent: self.agent,
            entries,
            aggregate,
            selection_out_of_date,
        })
    }

    /// Apply a checked mutation. See the module docs for the full transaction.
    pub fn apply(
        &self,
        action: HookAction,
        selection: &HookSelection,
    ) -> Result<Installation, AppError> {
        // Encryption is mandatory: no mutation writes Agent config without an
        // encrypted rollback snapshot (design 9.4.5).
        let cipher = self.cipher.as_ref().ok_or_else(|| AppError {
            domain: ErrorDomain::SecretStore,
            code: "security.encryption_unavailable".to_owned(),
            message: "config encryption store is unavailable".to_owned(),
            suggested_action: Some("unlock the application data key and retry".to_owned()),
        })?;

        let desired: Vec<OwnedHookEntry> = match action {
            HookAction::Uninstall => Vec::new(),
            HookAction::Install | HookAction::Repair | HookAction::UpgradeHelper => {
                self.desired_entries(selection)
            }
        };

        // Helper must be present at the wired installer's stable path, and the
        // selection must point at that same path (a wiring sanity check that
        // prevents a stale selection from recording a foreign helper).
        if action != HookAction::Uninstall {
            let installed_helper = self.helper.stable_path();
            // The selection must point at the wired helper's stable path AND carry
            // its manifest version — otherwise a stale selection (e.g. a
            // selection cached before a helper upgrade) would record a foreign or
            // stale helper path/version in the installation rows.
            if !installed_helper.exists()
                || installed_helper != selection.helper_path
                || selection.helper_version != *self.helper.manifest_version()
            {
                return Err(AppError {
                    domain: ErrorDomain::Update,
                    code: "update.helper_not_installed".to_owned(),
                    message: "signed helper is not installed at the expected path".to_owned(),
                    suggested_action: Some("run helper installation first".to_owned()),
                });
            }
        }

        self.ensure_config_seed()?;
        let bytes = fs::read(&self.config_path).map_err(|_| write_failed())?;
        let inspected_hash = sha256_hex(&bytes);
        let patch = self.patch(&bytes, &desired)?;
        let file_mode = current_mode(&self.config_path);

        // Encrypt ONLY the previous hooks subtree + source hash + mode. The AAD
        // binds ciphertext to the snapshot id; `snapshot_aad` is shared with the
        // decrypt path so the binding is verifiable on recovery.
        let snapshot_id = Uuid::now_v7();
        let encrypted = cipher.encrypt_snapshot(snapshot_id, &patch.before_hooks_subtree)?;
        self.repository
            .save_snapshot(&crate::model::ConfigSnapshotRecord {
                id: snapshot_id,
                agent: self.agent,
                ciphertext: encrypted.ciphertext,
                nonce: encrypted.nonce,
                aad: snapshot_aad(snapshot_id),
                source_hash: patch.before_hash.clone(),
                file_mode,
                created_at: Utc::now(),
            })?;

        // Test seam: simulate an external editor changing the file between the
        // inspection read above and the atomic replace below so the replace's
        // independent re-read detects drift. No effect in production builds.
        if forced_drift_enabled() {
            let _ = fs::write(&self.config_path, b"{\"external\":\"drift\"}");
        }

        atomic_replace_checked(&self.config_path, &inspected_hash, &patch.bytes, file_mode)?;

        // Re-parse to confirm the exact owned entries, then record rows with
        // separate command/definition fingerprints.
        let final_bytes = fs::read(&self.config_path).map_err(|_| write_failed())?;
        let installed = inspect_owned_entries(self.agent, &final_bytes)?;
        let records = self.build_records(&installed, &patch.after_hash, selection)?;
        self.repository.replace_hooks(self.agent, &records)?;

        Ok(Installation {
            agent: self.agent,
            records,
        })
    }

    /// Record that an ingress request arrived for `source_event` with
    /// `command_fingerprint`. Moves a Codex entry from `NeedsUserConfirmation`
    /// to `ObservedWorking` only when the fingerprint matches the stored
    /// expected command. No-op for Claude (trust is `NotRequired`).
    pub fn observe_ingress(
        &self,
        source_event: &str,
        command_fingerprint: &str,
    ) -> Result<TrustStatus, AppError> {
        self.repository.mark_hook_seen(
            self.agent,
            source_event,
            command_fingerprint,
            Utc::now(),
        )?;
        Ok(self
            .repository
            .hook(self.agent, source_event)
            .map(|row| row.trust_status)
            .unwrap_or(self.default_trust()))
    }

    // ---- helpers -----------------------------------------------------------

    fn default_trust(&self) -> TrustStatus {
        match self.agent {
            AgentKind::ClaudeCode => TrustStatus::NotRequired,
            AgentKind::Codex => TrustStatus::NeedsUserConfirmation,
        }
    }

    fn desired_entries(&self, selection: &HookSelection) -> Vec<OwnedHookEntry> {
        selection
            .events()
            .iter()
            .map(|event| OwnedHookEntry {
                source_event: event.clone(),
                matcher: Some(String::new()),
                command: owned_command(&selection.helper_path, self.agent, event),
                timeout_seconds: 1,
            })
            .collect()
    }

    fn patch(&self, source: &[u8], desired: &[OwnedHookEntry]) -> Result<ConfigPatch, AppError> {
        match self.agent {
            AgentKind::ClaudeCode => patch_claude_settings(source, desired),
            AgentKind::Codex => patch_codex_hooks(source, desired),
        }
    }

    fn ensure_config_seed(&self) -> Result<(), AppError> {
        if let Some(parent) = self.config_path.parent() {
            ensure_private_directory(parent)?;
            #[cfg(windows)]
            crate::security::permissions::ensure_current_user_dacl(parent)?;
        }
        // Atomically create the seed ONLY if it is absent (O_CREAT | O_EXCL). A
        // plain exists()-then-write would clobber a config a concurrent process
        // created between the check and the write; create_new refuses to touch an
        // existing file. apply() re-reads the file afterward, so a race-created
        // file is patched from its real contents rather than a synthetic {}.
        match private_create_new(&self.config_path) {
            Ok(mut file) => {
                file.write_all(b"{}").map_err(|_| write_failed())?;
                let _ = file.sync_all();
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(write_failed()),
        }
        #[cfg(windows)]
        crate::security::permissions::ensure_current_user_dacl(&self.config_path)?;
        Ok(())
    }

    fn build_records(
        &self,
        installed: &[OwnedHookEntry],
        config_hash: &str,
        selection: &HookSelection,
    ) -> Result<Vec<HookInstallationRecord>, AppError> {
        let helper_version = selection.helper_version.to_string();
        let mut records = Vec::with_capacity(installed.len());
        for entry in installed {
            // Record the fingerprint the runtime helper self-reports (canonical
            // stable-path command incl. its Windows component), NOT a hash of
            // the parsed-back config entry: Claude's settings schema cannot
            // persist commandWindows, so deriving from it broke the process_live
            // trust gate for every real Claude Code invocation.
            let cmd_fp = crate::hook_command::command_fingerprint(&owned_command(
                &selection.helper_path,
                self.agent,
                &entry.source_event,
            ));
            let def_fp = hook_definition_fingerprint(self.agent, entry);
            let trust = self.resolve_trust(&entry.source_event, &cmd_fp, &def_fp);
            let last_seen_at = if trust == TrustStatus::ObservedWorking {
                self.repository
                    .hook(self.agent, &entry.source_event)
                    .ok()
                    .and_then(|row| row.last_seen_at)
            } else {
                None
            };
            records.push(HookInstallationRecord {
                agent: self.agent,
                source_event: entry.source_event.clone(),
                command_fingerprint: cmd_fp,
                definition_fingerprint: def_fp,
                helper_version: helper_version.clone(),
                config_hash: config_hash.to_owned(),
                trust_status: trust,
                health_status: InstallationHealth::Healthy,
                last_seen_at,
            });
        }
        Ok(records)
    }

    /// Preserve `ObservedWorking` only when BOTH the command and the definition
    /// fingerprints match a previously observed row. Any change resets Codex
    /// trust to `NeedsUserConfirmation`. Claude is always `NotRequired`.
    fn resolve_trust(
        &self,
        event: &str,
        command_fingerprint: &str,
        definition_fingerprint: &str,
    ) -> TrustStatus {
        if self.agent == AgentKind::ClaudeCode {
            return TrustStatus::NotRequired;
        }
        match self.repository.hook(self.agent, event) {
            Ok(prev)
                if prev.command_fingerprint == command_fingerprint
                    && prev.definition_fingerprint == definition_fingerprint
                    && prev.trust_status == TrustStatus::ObservedWorking =>
            {
                TrustStatus::ObservedWorking
            }
            _ => TrustStatus::NeedsUserConfirmation,
        }
    }
}

fn entry_health(
    row: Option<&crate::model::HookInstallationRecord>,
    cmd_fp: &str,
    def_fp: &str,
    agent: AgentKind,
    agent_upgrade_required: bool,
) -> EntryHealth {
    if agent_upgrade_required {
        return EntryHealth::AgentUpgradeRequired;
    }
    let Some(row) = row else {
        // Installed on disk but no recorded row: definition is untracked.
        return EntryHealth::Drifted;
    };
    if row.command_fingerprint != cmd_fp {
        // The helper path / command string moved versus what we recorded.
        return EntryHealth::HelperMismatch;
    }
    if row.definition_fingerprint != def_fp {
        // matcher / timeout / commandWindows changed.
        return EntryHealth::Drifted;
    }
    if agent == AgentKind::Codex && row.trust_status == TrustStatus::NeedsUserConfirmation {
        return EntryHealth::NeedsTrust;
    }
    EntryHealth::Healthy
}

fn aggregate_health(
    entries: &[HookEntryHealth],
    selection_out_of_date: bool,
    agent_upgrade_required: bool,
) -> HealthAggregate {
    if agent_upgrade_required {
        return HealthAggregate::Error;
    }
    let mut worst = if entries.is_empty() {
        HealthAggregate::Unknown
    } else {
        HealthAggregate::Healthy
    };
    for entry in entries {
        let severity = match entry.health {
            EntryHealth::Healthy => HealthAggregate::Healthy,
            EntryHealth::Missing
            | EntryHealth::Drifted
            | EntryHealth::HelperMismatch
            | EntryHealth::NeedsTrust => HealthAggregate::NeedsRepair,
            EntryHealth::AgentUpgradeRequired => HealthAggregate::Error,
        };
        worst = max_aggregate(worst, severity);
    }
    if selection_out_of_date && rank_aggregate(worst) < rank_aggregate(HealthAggregate::NeedsRepair)
    {
        worst = HealthAggregate::NeedsRepair;
    }
    worst
}

fn rank_aggregate(h: HealthAggregate) -> u8 {
    match h {
        HealthAggregate::Unknown => 0,
        HealthAggregate::Healthy => 1,
        HealthAggregate::NeedsRepair => 2,
        HealthAggregate::Error => 3,
    }
}

fn max_aggregate(a: HealthAggregate, b: HealthAggregate) -> HealthAggregate {
    if rank_aggregate(a) >= rank_aggregate(b) {
        a
    } else {
        b
    }
}

#[cfg(unix)]
fn current_mode(path: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn current_mode(_path: &std::path::Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn private_create_new(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_create_new(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

fn write_failed() -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: "integration.atomic_write_failed".to_owned(),
        message: "agent configuration could not be read".to_owned(),
        suggested_action: None,
    }
}

// Test-only seam: when set, [`HookInstaller::apply`] rewrites the Agent config
// file with sentinel bytes AFTER its initial read and BEFORE the atomic
// replace, so [`atomic_replace_checked`] detects drift. Mirrors
// `force_rename_failure_for_test`. No effect in production builds.
#[cfg(feature = "test-support")]
std::thread_local! {
    static FORCE_DRIFT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(feature = "test-support")]
pub fn force_config_drift_for_test(on: bool) {
    FORCE_DRIFT.with(|flag| flag.set(on));
}

#[cfg(feature = "test-support")]
fn forced_drift_enabled() -> bool {
    FORCE_DRIFT.with(|flag| flag.get())
}

#[cfg(not(feature = "test-support"))]
fn forced_drift_enabled() -> bool {
    false
}
