CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);

CREATE TABLE app_settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE agent_installations (
  agent TEXT PRIMARY KEY,
  executable_path TEXT,
  version TEXT,
  capability_verification TEXT NOT NULL,
  health_status TEXT NOT NULL,
  last_checked_at TEXT NOT NULL
);

CREATE TABLE hook_installations (
  agent TEXT NOT NULL,
  source_event TEXT NOT NULL,
  command_fingerprint TEXT NOT NULL,
  definition_fingerprint TEXT NOT NULL,
  helper_version TEXT NOT NULL,
  config_hash TEXT NOT NULL,
  trust_status TEXT NOT NULL,
  health_status TEXT NOT NULL,
  last_seen_at TEXT,
  observed_command_fingerprint TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (agent, source_event)
);

CREATE TABLE config_snapshots (
  id TEXT PRIMARY KEY,
  agent TEXT NOT NULL,
  config_path TEXT NOT NULL,
  hook_subtree_ciphertext BLOB NOT NULL,
  nonce BLOB NOT NULL,
  aad TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  file_mode INTEGER,
  created_at TEXT NOT NULL
);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  canonical_root TEXT NOT NULL,
  worktree_mode TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE project_paths (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  canonical_path TEXT NOT NULL,
  kind TEXT NOT NULL,
  UNIQUE(canonical_path)
);

CREATE TABLE channels (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  credential_ref TEXT NOT NULL UNIQUE,
  public_config_json TEXT NOT NULL,
  health_status TEXT NOT NULL,
  paused_reason_code TEXT,
  consecutive_auth_failures INTEGER NOT NULL DEFAULT 0,
  last_succeeded_at TEXT,
  next_allowed_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE global_rules (
  id TEXT PRIMARY KEY,
  agent TEXT NOT NULL,
  source_event TEXT NOT NULL,
  version INTEGER NOT NULL,
  config_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(agent, source_event)
);

CREATE TABLE project_rule_overrides (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  agent TEXT NOT NULL,
  source_event TEXT NOT NULL,
  version INTEGER NOT NULL,
  patch_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(project_id, agent, source_event)
);

CREATE TABLE ingress_events (
  id TEXT PRIMARY KEY,
  safe_envelope_json TEXT NOT NULL,
  received_at TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','processing'))
);

CREATE TABLE events (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  source_version TEXT NOT NULL,
  source_event TEXT NOT NULL,
  category TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  received_at TEXT NOT NULL,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  project_display_name TEXT,
  unmatched_cwd_fingerprint TEXT,
  session_ref TEXT,
  turn_ref TEXT,
  model TEXT,
  permission_mode TEXT,
  severity TEXT NOT NULL,
  public_fields_json TEXT NOT NULL,
  sensitive_blob_id TEXT,
  sensitive_fields_blob BLOB,
  correlation_id TEXT NOT NULL,
  action_id TEXT,
  action_capabilities_json TEXT NOT NULL,
  processing_outcome TEXT NOT NULL,
  outcome_reason_code TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE delivery_jobs (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  rule_id TEXT NOT NULL,
  rule_version TEXT NOT NULL,
  channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  idempotency_key TEXT NOT NULL UNIQUE,
  document_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','sending','retry_wait','succeeded','failed','expired')),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  lease_owner TEXT,
  lease_expires_at TEXT,
  aggregate_key TEXT,
  aggregate_release_at TEXT,
  last_error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE delivery_attempts (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES delivery_jobs(id) ON DELETE CASCADE,
  attempt_number INTEGER NOT NULL,
  started_at TEXT NOT NULL,
  completed_at TEXT NOT NULL,
  outcome TEXT NOT NULL,
  http_status INTEGER,
  platform_code TEXT,
  error_code TEXT,
  retry_at TEXT,
  redacted_detail TEXT,
  UNIQUE(job_id, attempt_number)
);

CREATE INDEX idx_ingress_events_state_received_at
  ON ingress_events(state, received_at);
CREATE INDEX idx_events_occurred_at
  ON events(occurred_at);
CREATE INDEX idx_events_project_occurred_at
  ON events(project_id, occurred_at);
CREATE INDEX idx_events_source_event_occurred_at
  ON events(source_event, occurred_at);
CREATE INDEX idx_delivery_jobs_due
  ON delivery_jobs(state, next_attempt_at, lease_expires_at);
CREATE INDEX idx_delivery_attempts_job
  ON delivery_attempts(job_id);
CREATE INDEX idx_project_paths_project
  ON project_paths(project_id);
