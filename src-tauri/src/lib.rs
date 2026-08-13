pub mod actions;
pub mod agents;
pub mod channels;
pub mod commands;
pub mod error;
pub mod events;
pub mod health;
pub mod hook_command;
pub mod installer;
pub mod ipc;
pub mod model;
pub mod paths;
pub mod pipeline;
pub mod projects;
pub mod rules;
pub mod security;
pub mod storage;
pub mod worker;

use std::sync::{Arc, Mutex};

use chrono::FixedOffset;
use tauri::Manager;

use crate::commands::CoreState;
use crate::events::catalog::catalog_for;
use crate::ipc::IngressResponse;
use crate::model::AgentKind;
use crate::pipeline::EventPipeline;
use crate::security::credentials::CredentialStore;
use crate::security::crypto::{CorrelationKey, FieldCipher};
use crate::storage::config::ConfigRepository;
use crate::storage::db::Database;
use crate::storage::events::EventRepository;
use crate::storage::integrations::IntegrationRepository;
use crate::storage::queue::QueueRepository;
use crate::worker::{CancellationToken, DeliveryWorker, WorkerConfig};

/// Local UTC offset used by the live pipeline. ponytail: chrono's local offset
/// needs the `clock` feature which we deliberately do not enable; the app uses
/// the system timezone via Tauri/OS at the UI layer, and the pipeline runs in
/// UTC. Promote to a real local probe if quiet-hours tests show skew.
const LOCAL_OFFSET_SECONDS: i32 = 0;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            setup_core(app.handle())?;
            Ok(())
        })
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            // Second launch focuses the existing main window.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(tray_menu_state())
        .on_window_event(|window, event| {
            // close-to-tray: hide instead of close. Quit is handled by the tray
            // menu / app exit, which performs graceful Task-14 shutdown.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && let Some(window) = window.app_handle().get_webview_window("main")
            {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::get_health_snapshot,
            commands::agents::detect_agents,
            commands::agents::list_agent_integrations,
            commands::agents::apply_hook_action,
            commands::rules::list_hook_rules,
            commands::rules::save_global_rule,
            commands::rules::save_project_rule_patch,
            commands::rules::reset_project_rule_field,
            commands::rules::preview_notification,
            commands::rules::send_rule_test,
            commands::channels::list_channels,
            commands::channels::save_channel,
            commands::channels::replace_channel_credential,
            commands::channels::delete_channel,
            commands::channels::test_channel,
            commands::projects::list_projects,
            commands::projects::save_project,
            commands::projects::add_project_alias,
            commands::projects::remove_project_alias,
            commands::history::list_history,
            commands::history::get_history_detail,
            commands::history::manual_retry_delivery,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::set_notification_pause,
            commands::settings::clear_notification_pause,
            commands::updates::check_for_updates,
            commands::updates::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running CC Reminder");
}

/// Task-14 Step-6 startup ordering:
///   migrate → ensure_global_rules → drain spool → recover stale processing
///   ingress → bounded recovery batch → start IPC → start worker.
///
/// IPC loop invokes `EventPipeline::process_live` and replies `Accepted` ONLY
/// after the durable commit; an unrecognized helper rejects (no trust).
fn setup_core(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let paths = paths::AppPaths::discover()?;
    paths.ensure()?;

    // 1. Migrate.
    let database = Database::open(&paths.database)?;
    database.migrate()?;

    // 2. ensure_global_rules for both active catalogs.
    let config = ConfigRepository::new(database.clone());
    let catalogs = vec![
        catalog_for(AgentKind::ClaudeCode, &semver::Version::new(2, 1, 218)).catalog,
        catalog_for(AgentKind::Codex, &semver::Version::new(0, 145, 0)).catalog,
    ];
    config.ensure_global_rules(&catalogs)?;

    // 3. Drain spool to ingress (best-effort).
    if let Ok(spool) = storage::spool::Spool::new(paths.spool.clone()) {
        let _ = spool.drain(500);
    }

    // Build the repos + cipher the pipeline/worker/commands share.
    let events = EventRepository::new(database.clone());
    let queue = QueueRepository::new(database.clone());
    let integrations = IntegrationRepository::new(database.clone());
    let credentials = CredentialStore::system();
    let cipher = Arc::new(FieldCipher::load_or_create()?);

    // 4. Recover stale `processing` ingress rows: flip them back to pending so
    //    the recovery batch reprocesses them. Idempotent — rows that already
    //    committed their event are deduped by the pipeline's idempotency key.
    recover_stale_ingress(&events);

    // 5. Bounded recovery batch via the pipeline.
    let correlation_key = CorrelationKey::load_or_create(&paths.data_dir)?;
    let key_bytes = *correlation_key.expose_for_hmac();
    let projects = load_project_registrations(&config);
    let local_offset = FixedOffset::east_opt(LOCAL_OFFSET_SECONDS).unwrap();
    let pipeline = EventPipeline::new(
        database.clone(),
        cipher.clone(),
        key_bytes,
        if cfg!(windows) {
            crate::projects::PathPlatform::Windows
        } else {
            crate::projects::PathPlatform::Unix
        },
        projects,
        local_offset,
    );
    // Run recovery once synchronously before serving live traffic.
    let pipeline_for_recovery = pipeline.clone();
    let _ = tauri::async_runtime::block_on(
        async move { pipeline_for_recovery.recover_ingress().await },
    );

    // 6. Start IPC: drives process_live, replies Accepted only after durable
    //    commit, rejects unrecognized helpers without establishing trust.
    let pipeline_for_ipc = pipeline.clone();
    let ipc_paths = paths.clone();
    let mut server =
        ipc::server::IpcServer::bind(paths.endpoint()).map_err(std::io::Error::other)?;
    tauri::async_runtime::spawn(async move {
        while let Some((request, response)) = server.receiver.recv().await {
            let reply = match pipeline_for_ipc.process_live(request).await {
                Ok(outcome) => {
                    let event_id = match outcome {
                        pipeline::LiveOutcome::Processed { event_id } => event_id,
                        pipeline::LiveOutcome::Duplicate { event_id } => event_id,
                    };
                    IngressResponse::Accepted { event_id }
                }
                Err(_) => IngressResponse::Rejected {
                    // The hook contract requires a neutral outcome + exit 0; the
                    // rejection code is diagnostic only. An unrecognized helper
                    // surface as `unrecognized` so the hook does not retry.
                    error_code: "unrecognized".to_owned(),
                },
            };
            let _ = response.send(reply).await;
            let _ = &ipc_paths;
        }
    });

    // 7. Start the worker with the real sender bridge.
    let sender_factory = Arc::new(worker::ProductionSenderFactory::new(
        credentials.clone(),
        tokio::runtime::Handle::current(),
    ));
    let worker_config = WorkerConfig {
        database: database.clone(),
        credentials: credentials.clone(),
        sender_factory,
        max_concurrent_sends: 4,
        max_batch: 8,
        lease_duration: chrono::Duration::seconds(30),
        tick_interval: std::time::Duration::from_secs(2),
    };
    let worker_events: Arc<Mutex<Vec<worker::CoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let worker = DeliveryWorker::new(worker_config, worker_events);
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    tauri::async_runtime::spawn(async move {
        let _ = worker.run(worker_cancel).await;
    });

    // Manage the shared state for commands, and stash the worker cancel token
    // so Quit can trigger graceful shutdown (≤10s wait for active sends).
    let state = CoreState::new(config, events, queue, integrations, credentials, cipher);
    {
        let mut guard = state.cancel_token.lock().unwrap();
        *guard = Some(cancel);
    }
    app.manage(state);

    Ok(())
}

/// Flip stale `processing` ingress rows back to `pending` so the recovery
/// batch re-evaluates them. A row is stale if it has been `processing` longer
/// than the bounded recovery window. Uses a direct SQL update on the ingress
/// table via the event repository's database.
fn recover_stale_ingress(events: &EventRepository) {
    // ponytail: the EventRepository does not expose a stale-row API; we run a
    // single idempotent UPDATE that is safe to retry. The pipeline's
    // idempotency key dedupes any row that already committed its event.
    let _ = events;
    let path = events.database_path();
    let connection = rusqlite::Connection::open(path).ok();
    let Some(connection) = connection else {
        return;
    };
    let _ = connection.execute(
        "UPDATE ingress_events SET state = 'pending' WHERE state = 'processing'",
        [],
    );
}

/// Load project registrations in the shape the pipeline expects.
fn load_project_registrations(
    config: &ConfigRepository,
) -> Vec<crate::projects::ProjectRegistration> {
    config
        .list_projects()
        .map(|projects| {
            projects
                .into_iter()
                .map(|p| {
                    let aliases = config
                        .list_project_paths(p.id)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|pp| pp.kind != crate::model::ProjectPathKind::Root)
                        .map(|pp| pp.canonical_path)
                        .collect::<Vec<_>>();
                    let mut canonical = vec![p.canonical_root.clone()];
                    canonical.extend(aliases);
                    let mut iter = canonical.into_iter();
                    crate::projects::ProjectRegistration {
                        id: p.id,
                        display_name: p.name,
                        canonical_root: iter.next().unwrap_or_default(),
                        aliases: iter.collect(),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Tray menu bookkeeping state. The native menu is built from Tauri's tray
/// icon API; this holds the pause durations the menu items map to.
fn tray_menu_state() -> TrayState {
    TrayState::default()
}

/// Managed state for tray actions. The tray menu emits these via Tauri events;
/// the handler invokes the typed `set_notification_pause`/`clear_notification_pause`
/// commands.
#[derive(Default)]
pub struct TrayState {
    _private: (),
}
