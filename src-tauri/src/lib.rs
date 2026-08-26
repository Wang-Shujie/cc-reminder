pub mod actions;
pub mod agents;
pub mod channels;
pub mod commands;
pub mod diagnostics;
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

use std::sync::Arc;

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

/// Local UTC offset used by the live pipeline, from persisted settings.
///
/// The frontend reports its real zone offset (`-Date#getTimezoneOffset()*60`,
/// east-positive seconds) at every bootstrap and we persist it alongside
/// settings — the same frontend-reported pattern as the Task 19 pause fix,
/// because chrono's own local-offset lookup needs the `clock` feature we
/// deliberately do not enable. Documented fallback: `0` (= UTC) until the
/// first report lands, so quiet hours are correct in local time from the
/// first run AFTER the first launch. A stale stored value (out of chrono's
/// range) degrades to UTC rather than failing startup.
fn local_offset_from_stored(stored_seconds: i32) -> FixedOffset {
    FixedOffset::east_opt(stored_seconds).unwrap_or_else(|| {
        FixedOffset::east_opt(0).expect("the zero offset is always representable")
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            setup_core(app.handle())?;
            Ok(())
        })
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            // Second launch restores the existing main window. Focus alone is
            // not enough: with the default close-to-tray behavior the window
            // may be HIDDEN, so it must be shown (and unminimized) first.
            if let Some(window) = app.get_webview_window("main") {
                // Query failures fall back to "hidden + minimized" so the
                // restore path stays maximally aggressive instead of leaving
                // the user with an invisible window.
                let visible = window.is_visible().unwrap_or(false);
                let minimized = window.is_minimized().unwrap_or(true);
                let ops = restore_window_ops(visible, minimized);
                if ops.show {
                    let _ = window.show();
                }
                if ops.unminimize {
                    let _ = window.unminimize();
                }
                if ops.focus {
                    let _ = window.set_focus();
                }
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Directory-open dialog for the Projects page (Task 18). The
        // capability grants ONLY `dialog:allow-open`; save/message prompts and
        // file picks stay unavailable to the WebView.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(tray_menu_state())
        .on_window_event(|window, event| {
            // 关闭时最小化到托盘 (`close_to_tray`) decides what closing the
            // main window means: hide-and-keep-running (default) or a real
            // quit. Both exit routes (this one and Cmd-Q / menu quit) converge
            // on RunEvent::Exit below, which runs the graceful Task-14
            // shutdown.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                // Persisted user preference; fall back to the documented
                // default (`true`) if the state or DB read fails so a
                // transient error never changes close semantics silently.
                let close_to_tray = window
                    .app_handle()
                    .try_state::<CoreState>()
                    .and_then(|state| state.config.get_settings().ok())
                    .map(|settings| settings.close_to_tray)
                    .unwrap_or(true);
                if close_action(close_to_tray) == CloseAction::HideToTray {
                    let _ = window.hide();
                    api.prevent_close();
                }
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
            commands::diagnostics::export_diagnostics,
            commands::diagnostics::clear_history,
            commands::diagnostics::set_debug_logging,
            commands::updates::check_for_updates,
            commands::updates::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building CC Reminder")
        .run(|app, event| {
            // Every exit route lands here (close-to-exit window, Cmd-Q / menu
            // quit, updater relaunch): run the graceful Task-14 shutdown once,
            // right before the process goes away.
            if let tauri::RunEvent::Exit = event {
                shutdown_core(app);
            }
        });
}

/// What a main-window `CloseRequested` should do, decided purely from the
/// persisted 关闭时最小化到托盘 setting so both branches stay unit-testable
/// without constructing a Tauri window (the handler above stays thin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseAction {
    /// Hide the window and keep the app running in the background.
    HideToTray,
    /// Let the window close; the app exits through [`RunEvent::Exit`].
    Exit,
}

pub(crate) fn close_action(close_to_tray: bool) -> CloseAction {
    if close_to_tray {
        CloseAction::HideToTray
    } else {
        CloseAction::Exit
    }
}

/// Which window operations a second launch must apply to bring the main
/// window back in front of the user. Decided purely from the window's current
/// visibility/minimized state so the single-instance handler above stays thin
/// glue (same pattern as [`close_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RestoreWindowOps {
    /// `WebviewWindow::show` — needed when the window is hidden (the default
    /// close-to-tray outcome).
    pub show: bool,
    /// `WebviewWindow::unminimize` — a minimized window is not raised by focus.
    pub unminimize: bool,
    /// `WebviewWindow::set_focus` — always applied on relaunch.
    pub focus: bool,
}

pub(crate) fn restore_window_ops(visible: bool, minimized: bool) -> RestoreWindowOps {
    RestoreWindowOps {
        show: !visible,
        unminimize: minimized,
        focus: true,
    }
}

/// Task-14 Step-7 shutdown, shared by EVERY exit route: stop accepting IPC
/// (the accept loop selects on the same token), cancel the delivery-worker
/// and retention loops, then wait ≤10s for the in-flight send pass to drain
/// before the process goes away. Best-effort past the cancel signal — at the
/// deadline the OS reclaims whatever is still running, exactly like a crash:
/// stale leases/processing rows are recovered by the next startup.
fn shutdown_core(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<CoreState>() else {
        return;
    };
    if let Some(token) = state.cancel_token.lock().unwrap().take() {
        token.cancel();
    }
    let worker_task = state.worker_task.lock().unwrap().take();
    if let Some(task) = worker_task {
        // Awaiting the join handle covers "active sends finish": `run` only
        // returns after the current pass completes, so this waits for real
        // work, not just the cancellation handshake.
        let _ = tauri::async_runtime::block_on(tokio::time::timeout(
            std::time::Duration::from_secs(10),
            task,
        ));
    }
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

    // 0. Shared diagnostics logger: every production log line below goes
    //    through this one instance and its mandatory-redactor chokepoint, so
    //    diagnostic exports contain exactly what was logged at runtime.
    let diagnostics = std::sync::Arc::new(diagnostics::Diagnostics::init(&paths.logs)?);

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
    let local_offset = local_offset_from_stored(
        config
            .get_settings()
            .map(|settings| settings.local_offset_seconds)
            .unwrap_or(0),
    );
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

    // One cancel signal drives every background loop (IPC accept, delivery
    // worker, retention): Task-14 graceful shutdown (`shutdown_core`, from
    // RunEvent::Exit) cancels all three together.
    let cancel = CancellationToken::new();

    // 6. Start IPC: drives process_live, replies Accepted only after durable
    //    commit, rejects unrecognized helpers without establishing trust.
    //    The accept loop selects on the same cancel token so an exiting app
    //    stops admitting hook traffic; requests already received still get
    //    their durable reply before the loop breaks.
    let pipeline_for_ipc = pipeline.clone();
    let ipc_diagnostics = diagnostics.clone();
    let mut server =
        ipc::server::IpcServer::bind(paths.endpoint()).map_err(std::io::Error::other)?;
    let ipc_cancel = cancel.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let received = tokio::select! {
                _ = ipc_cancel.cancelled() => None,
                next = server.receiver.recv() => next,
            };
            let Some((request, response)) = received else {
                break;
            };
            let reply = match pipeline_for_ipc.process_live(request).await {
                Ok(outcome) => {
                    let event_id = match outcome {
                        pipeline::LiveOutcome::Processed { event_id } => event_id,
                        pipeline::LiveOutcome::Duplicate { event_id } => event_id,
                    };
                    IngressResponse::Accepted { event_id }
                }
                Err(_) => {
                    // Redacted one-liner through the Diagnostics chokepoint;
                    // never the request contents.
                    ipc_diagnostics.info("ipc", "ingress request rejected");
                    IngressResponse::Rejected {
                        // The hook contract requires a neutral outcome + exit 0; the
                        // rejection code is diagnostic only. An unrecognized helper
                        // surface as `unrecognized` so the hook does not retry.
                        error_code: "unrecognized".to_owned(),
                    }
                }
            };
            let _ = response.send(reply).await;
        }
    });

    // 7. Start the worker with the real sender bridge.
    let sender_factory = Arc::new(worker::ProductionSenderFactory::new(credentials.clone()));
    let worker_config = WorkerConfig {
        database: database.clone(),
        credentials: credentials.clone(),
        sender_factory,
        max_concurrent_sends: 4,
        max_batch: 8,
        lease_duration: chrono::Duration::seconds(30),
        tick_interval: std::time::Duration::from_secs(2),
    };
    // Shared bounded channel for revision-only core:// events. Producers: the
    // delivery worker (below) and command bodies via CoreState.core_events;
    // the single consumer is the forwarder task spawned right after.
    let (core_event_sink, mut core_event_receiver) =
        tokio::sync::mpsc::channel::<worker::CoreEvent>(64);
    let worker_events = core_event_sink.clone();
    let worker = DeliveryWorker::new(worker_config, worker_events);

    // 7b. `core://` forwarder: the single consumer of the bounded event
    //     channel shared by the delivery worker and the command surface.
    //     Each CoreEvent becomes a revision-only payload on its topic
    //     (`worker::CoreEvent::core_topic`), matching what
    //     `src/lib/backend.tsx` subscribe listeners expect — they refetch on
    //     any revision bump and never trust payload details. The channel is
    //     bounded (64): when the WebView is slow the producer drops the
    //     NEWEST notification (see `worker::emit`) because revisions
    //     self-heal on the next tick/refetch. The task ends when every sender
    //     drops (worker cancelled + app state gone).
    let forwarder_app = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let mut revisions: std::collections::HashMap<&'static str, u64> =
            std::collections::HashMap::new();
        while let Some(event) = core_event_receiver.recv().await {
            let topic = event.core_topic();
            let revision = revisions.entry(topic).and_modify(|r| *r += 1).or_insert(1);
            let _ = forwarder_app.emit(topic, serde_json::json!({ "revision": revision }));
        }
    });
    let worker_cancel = cancel.clone();
    // Keep the join handle: RunEvent::Exit waits (≤10s) on it so an in-flight
    // send pass finishes instead of being killed mid-write.
    let worker_task = tauri::async_runtime::spawn(async move {
        let _ = worker.run(worker_cancel).await;
    });

    // 8. Retention (Task 20 Step 5): one pass immediately after startup, then
    //    every 24 hours. Spawned — never blocks setup; failures are logged
    //    through Diagnostics and tolerated; shutdown rides the same cancel
    //    token as the worker. This is production-only startup work: it lives
    //    behind `run()`, which `cargo test` never invokes.
    let retention_service =
        storage::retention::RetentionService::new(database.clone(), paths.logs.clone());
    let retention_diagnostics = diagnostics.clone();
    let retention_cancel = cancel.clone();
    tauri::async_runtime::spawn(storage::retention::run_forever(
        retention_service,
        retention_diagnostics,
        std::time::Duration::from_secs(24 * 60 * 60),
        retention_cancel,
    ));

    // Manage the shared state for commands, and stash the worker cancel token
    // + join handle so RunEvent::Exit can perform the Task-14 graceful
    // shutdown (≤10s wait for active sends).
    let mut state = CoreState::new(
        config,
        events,
        queue,
        integrations,
        credentials,
        cipher,
        diagnostics,
    );
    {
        let mut guard = state.cancel_token.lock().unwrap();
        *guard = Some(cancel);
    }
    {
        let mut guard = state.worker_task.lock().unwrap();
        *guard = Some(worker_task);
    }
    {
        // Commands push revision notifications onto the same channel the
        // worker uses; the forwarder task above is their only consumer.
        state.core_events = core_event_sink;
    }
    {
        // Production root for the bundled signed helper (manifest + bytes).
        // Resolution is FIXED via Tauri's resource-dir API; failure leaves
        // `None` and apply_hook_action reports the typed
        // `configuration.helper_unavailable` instead of guessing a path.
        state.resources_dir = app.path().resource_dir().ok();
    }
    {
        // Autostart is applied only from save_settings (plan Task 15); the
        // control delegates to the official autostart plugin.
        use tauri_plugin_autostart::ManagerExt;
        let handle = app.clone();
        state.autostart_control = Arc::new(move |enable| {
            let manager = handle.autolaunch();
            let result = if enable {
                manager.enable()
            } else {
                manager.disable()
            };
            result.map_err(|error| format!("autostart plugin failed: {error}"))
        });
    }
    app.manage(state);

    Ok(())
}

/// Flip stale `processing` ingress rows back to `pending` so the recovery
/// batch re-evaluates them.
///
/// This runs during startup BEFORE this process starts its worker, and the
/// single-instance plugin prevents a second app process, so no live worker can
/// own a `processing` row at this moment — an unconditional reset is therefore
/// safe (a row left `processing` by a crashed predecessor is exactly what we
/// want to recover). The schema has no claim timestamp, so a time-bounded
/// reset is not possible without a migration; revisit if multi-process access
/// is ever introduced. The pipeline's idempotency key dedupes any row that
/// already committed its event.
fn recover_stale_ingress(events: &EventRepository) {
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

#[cfg(test)]
mod tests {
    use super::{CloseAction, RestoreWindowOps, close_action, restore_window_ops};

    #[test]
    fn close_to_tray_enabled_hides_the_window() {
        // Default setting: closing the main window hides it, app keeps
        // running in the background (hook ingress + queue stay alive).
        assert_eq!(close_action(true), CloseAction::HideToTray);
    }

    #[test]
    fn close_to_tray_disabled_really_closes_and_exits() {
        // Opt-out: no prevent_close — the window closes and the process exits
        // through RunEvent::Exit (graceful shutdown).
        assert_eq!(close_action(false), CloseAction::Exit);
    }

    #[test]
    fn hidden_window_is_shown_and_focused_on_relaunch() {
        // The default close-to-tray outcome: a HIDDEN window must be shown,
        // not just focused (focus alone is invisible to the user).
        assert_eq!(
            restore_window_ops(false, false),
            RestoreWindowOps {
                show: true,
                unminimize: false,
                focus: true
            }
        );
    }

    #[test]
    fn minimized_window_is_unminimized_and_focused_on_relaunch() {
        assert_eq!(
            restore_window_ops(true, true),
            RestoreWindowOps {
                show: false,
                unminimize: true,
                focus: true
            }
        );
    }

    #[test]
    fn visible_window_only_needs_focus_on_relaunch() {
        assert_eq!(
            restore_window_ops(true, false),
            RestoreWindowOps {
                show: false,
                unminimize: false,
                focus: true
            }
        );
    }

    #[test]
    fn hidden_and_minimized_window_gets_every_restore_step() {
        // Also covers the query-failure fallback in the handler (treated as
        // hidden + minimized): all three steps apply.
        assert_eq!(
            restore_window_ops(false, true),
            RestoreWindowOps {
                show: true,
                unminimize: true,
                focus: true
            }
        );
    }

    #[test]
    fn stored_local_offsets_map_to_fixed_offsets_with_utc_fallback() {
        // The persisted frontend report (+08:00) drives quiet hours in local
        // time...
        assert_eq!(
            super::local_offset_from_stored(8 * 3600).local_minus_utc(),
            8 * 3600
        );
        // ...the documented pre-first-report fallback is UTC...
        assert_eq!(super::local_offset_from_stored(0).local_minus_utc(), 0);
        // ...and a stale/corrupt stored value degrades to UTC instead of
        // failing startup.
        assert_eq!(
            super::local_offset_from_stored(i32::MAX).local_minus_utc(),
            0
        );
    }
}
