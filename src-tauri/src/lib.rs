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
pub mod tray;
pub mod worker;

use std::sync::Arc;

use chrono::FixedOffset;
use futures_util::FutureExt;
use tauri::Manager;

use crate::commands::CoreState;
use crate::events::catalog::catalog_for;
use crate::ipc::IngressResponse;
use crate::model::AgentKind;
use crate::pipeline::EventPipeline;
use crate::security::credentials::CredentialStore;
use crate::security::crypto::CorrelationKey;
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
                    .and_then(|state| state.storage.config.get_settings().ok())
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
            // quit; an updater-triggered relaunch would land here too once the
            // updater ships): run the graceful Task-14 shutdown once, right
            // before the process goes away.
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
    // v2-issues:退出路径对中毒锁免疫(实机两次 quit abort 都在此 unwind,
    // abort 反而抹掉优雅收尾)。into_inner 无论如何取到守卫。
    {
        let mut cancel_lock = state
            .runtime
            .cancel_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(token) = cancel_lock.take() {
            token.cancel();
        }
    }
    let mut worker_lock = state
        .runtime
        .worker_task
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worker_task = worker_lock.take();
    if let Some(task) = worker_task {
        // Awaiting the join handle covers "active sends finish": `run` only
        // returns after the current pass completes, so this waits for real
        // work, not just the cancellation handshake.
        //
        // v2-issues(实机日志 2026-08-27):`tauri::async_runtime::block_on` 在
        // RunEvent::Exit 上没有 reactor 上下文,每次退出都 panic→abort,10s
        // 排水从未生效过。这里自建一次性 current-thread runtime 来等待——
        // 退出路径只等这一个 handle,不需要 tauri 的多线程运行时。
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        if let Ok(runtime) = runtime {
            let _ = runtime.block_on(tokio::time::timeout(
                std::time::Duration::from_secs(10),
                task,
            ));
        }
    }
}

/// Startup ordering, v2-issues split:
///   SYNC (before the window exists — must stay fast): migrate →
///   ensure_global_rules → recover stale processing rows → manage state →
///   tray. The WebView appears here, so the keychain ACL dialog (invisible
///   and unclickable before any window existed) is now visible.
///   BACKGROUND (spawned task): keychain cipher load → spool drain →
///   recovery batch → IPC bind → worker/forwarder/retention/redetect.
///
/// v2-issues(启动慢根因):钥匙串读取(重编译后签名变化触发 ACL 授权弹窗,
/// 而弹窗在窗口出现前不可见,曾实测阻塞 5–120s+)曾卡在同步启动路径上,
/// 窗口被拖到全部初始化完成之后。现在 setup() 只做本地 SQLite 快操作就
/// 返回;重活全部进后台任务,其中钥匙串加载放在 blocking 池。加载完成前
/// 需要加密的命令拿到 typed `configuration.core_starting` 错误;IPC 未绑定
/// 期间 hook 流量由 helper 落盘,后台初始化尾部 drain 补收。
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

    // Build the repos the commands share. The cipher is deliberately LAZY:
    // the keychain read happens in the background task below.
    let events = EventRepository::new(database.clone());
    let queue = QueueRepository::new(database.clone());
    let integrations = IntegrationRepository::new(database.clone());
    let credentials = CredentialStore::system();
    let cipher = crate::security::crypto::LazyFieldCipher::new();

    // 3. Recover stale `processing` ingress rows: flip them back to pending so
    //    the recovery batch reprocesses them. Idempotent — rows that already
    //    committed their event are deduped by the pipeline's idempotency key.
    recover_stale_ingress(&events);

    // Inputs the background pipeline needs; all cheap local reads.
    let correlation_key = CorrelationKey::load_or_create(&paths.data_dir)?;
    let key_bytes = *correlation_key.expose_for_hmac();
    let projects = load_project_registrations(&config);
    let local_offset = local_offset_from_stored(
        config
            .get_settings()
            .map(|settings| settings.local_offset_seconds)
            .unwrap_or(0),
    );

    // One cancel signal drives every background loop (IPC accept, delivery
    // worker, retention): Task-14 graceful shutdown (`shutdown_core`, from
    // RunEvent::Exit) cancels all three together.
    let cancel = CancellationToken::new();

    // Shared bounded channel for revision-only core:// events. Producers: the
    // delivery worker, the IPC ingress loop (history-changed), and command
    // bodies via CoreState.core_events; the single consumer is the forwarder
    // task spawned by the background init below. Producers that emit before
    // the forwarder exists just fill the buffer (capacity 64) — revisions
    // self-heal on the next refetch.
    let (core_event_sink, mut core_event_receiver) =
        tokio::sync::mpsc::channel::<worker::CoreEvent>(64);

    // v2-issues(实机教训):panic 钩子把每个 panic 落进应用日志——tokio
    // 任务静默死亡(如 IPC 环)曾让 hook 全灭而无任何痕迹。stderr 对
    // launchd 应用不可见,这里是我们唯一的现场。同步段设置,先于任何 spawn。
    {
        let panic_diagnostics = diagnostics.clone();
        std::panic::set_hook(Box::new(move |info| {
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "<unknown>".to_owned());
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_owned());
            panic_diagnostics.info("panic", &format!("task panicked at {location}: {message}"));
        }));
    }

    // 4. Manage the shared state for commands NOW so the window can appear
    //    immediately. The worker join handle is stashed by the background init
    //    once the worker is actually spawned; RunEvent::Exit reads it.
    let rejected_counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut state = CoreState::new(
        config,
        events,
        queue,
        integrations.clone(),
        credentials.clone(),
        cipher.clone(),
        diagnostics.clone(),
    );
    {
        let mut guard = state.runtime.cancel_token.lock().unwrap();
        *guard = Some(cancel.clone());
    }
    {
        // Commands push revision notifications onto the same channel the
        // worker uses; the forwarder task is their only consumer.
        state.runtime.core_events = core_event_sink.clone();
    }
    {
        // v2-issues:IPC 拒绝计数器与命令层共享同一个 Arc。
        state.runtime.rejected_ingress = rejected_counter.clone();
    }
    {
        // Production root for the bundled signed helper (manifest + bytes).
        // Resolution is FIXED via Tauri's resource-dir API; failure leaves
        // `None` and apply_hook_action reports the typed
        // `configuration.helper_unavailable` instead of guessing a path.
        state.runtime.resources_dir = app.path().resource_dir().ok();
    }
    {
        // Autostart is applied only from save_settings (plan Task 15); the
        // control delegates to the official autostart plugin.
        use tauri_plugin_autostart::ManagerExt;
        let handle = app.clone();
        state.runtime.autostart_control = Arc::new(move |enable| {
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

    // v2-issues(设计 §18.3):原生托盘菜单——打开/健康/暂停/恢复/退出。
    // 在 state manage 之后安装,动作经 try_state 走同一命令 impl。
    tray::install(app);

    // 5. Background init (v2-issues): everything that used to block the window
    //    behind the keychain. Failures degrade visibly (typed command errors,
    //    diagnostics log) instead of a blank startup that never finishes.
    let bg_diagnostics = diagnostics.clone();
    let bg_app = app.clone();
    let bg_cancel = cancel.clone();
    let bg_spool = paths.spool.clone();
    let bg_endpoint = paths.endpoint();
    let bg_logs = paths.logs.clone();
    let bg_state_for_worker = app.state::<CoreState>().inner().clone();
    tauri::async_runtime::spawn(async move {
        // 5a. Keychain cipher load on the blocking pool — the potentially
        //     minutes-long step (ACL dialog). `load_or_create_logged` carries
        //     the watchdog that makes a hidden dialog visible in the log.
        let load_diagnostics = bg_diagnostics.clone();
        let loaded = tauri::async_runtime::spawn_blocking(move || {
            crate::security::crypto::FieldCipher::load_or_create_logged(&load_diagnostics)
        })
        .await;
        match loaded {
            Ok(Ok(cipher_value)) => cipher.set(cipher_value),
            Ok(Err(err)) => {
                bg_diagnostics.info(
                    "keychain",
                    &format!("data key load failed; encrypted commands will error: {err}"),
                );
                return;
            }
            Err(_) => {
                bg_diagnostics.info("keychain", "cipher loader task failed");
                return;
            }
        }
        let Ok(cipher_arc) = cipher.get() else {
            return;
        };

        let pipeline = EventPipeline::new(
            database.clone(),
            cipher_arc,
            key_bytes,
            if cfg!(windows) {
                crate::projects::PathPlatform::Windows
            } else {
                crate::projects::PathPlatform::Unix
            },
            projects,
            local_offset,
        );

        // 5b. Drain spool to ingress (best-effort). This also sweeps up hook
        //     traffic that arrived while the IPC socket was not yet bound.
        if let Ok(spool) = storage::spool::Spool::new(bg_spool) {
            let _ = spool.drain(500);
        }
        // Run the bounded recovery batch once before serving live traffic.
        if let Err(err) = pipeline.recover_ingress().await {
            bg_diagnostics.info("storage", &format!("ingress recovery failed: {err}"));
        }

        // 5c. IPC: drives process_live, replies Accepted only after durable
        //     commit, rejects unrecognized helpers without establishing trust.
        //     The accept loop selects on the same cancel token so an exiting
        //     app stops admitting new hook traffic; a request already taken off
        //     the channel finishes its commit-and-reply before the loop breaks.
        let Ok(mut server) = ipc::server::IpcServer::bind(bg_endpoint) else {
            bg_diagnostics.info("ipc", "ipc endpoint bind failed; hook ingress is down");
            return;
        };
        let pipeline_for_ipc = pipeline.clone();
        let ipc_diagnostics = bg_diagnostics.clone();
        let ipc_events = core_event_sink.clone();
        let ipc_rejected = rejected_counter.clone();
        let ipc_cancel = bg_cancel.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let received = tokio::select! {
                    _ = ipc_cancel.cancelled() => None,
                    next = server.receiver.recv() => next,
                };
                let Some((request, response)) = received else {
                    break;
                };
                // v2-issues(实机教训):单请求 panic 不许杀死整个接受环——
                // 环死 = hook 全灭且零日志。捕获、记录、按拒绝答复。
                let reply =
                    match std::panic::AssertUnwindSafe(pipeline_for_ipc.process_live(request))
                        .catch_unwind()
                        .await
                    {
                        // 正常路径:Processed 推 history-changed;Duplicate 不推。
                        Ok(Ok(pipeline::LiveOutcome::Processed { event_id })) => {
                            // v2-issues: 事件落库即推送 history-changed,
                            // 通知记录订阅端即时刷新(重复事件无历史变化)。
                            crate::worker::emit(
                                &ipc_events,
                                crate::worker::CoreEvent::HistoryChanged,
                            );
                            IngressResponse::Accepted { event_id }
                        }
                        Ok(Ok(pipeline::LiveOutcome::Duplicate { event_id })) => {
                            IngressResponse::Accepted { event_id }
                        }
                        Ok(Err(_)) => {
                            // Redacted one-liner through the Diagnostics chokepoint;
                            // never the request contents.
                            ipc_diagnostics.info("ipc", "ingress request rejected");
                            ipc_rejected.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            crate::worker::emit(
                                &ipc_events,
                                crate::worker::CoreEvent::HealthChanged { channel_id: None },
                            );
                            IngressResponse::Rejected {
                                // The hook contract requires a neutral outcome + exit 0; the
                                // rejection code is diagnostic only. An unrecognized helper
                                // surfaces as `unrecognized` so the hook does not retry.
                                error_code: "unrecognized".to_owned(),
                            }
                        }
                        // v2-issues(实机教训):单请求 panic 不许杀死整个接受环——
                        // 环死 = hook 全灭且零日志。捕获、记录、按拒绝答复。
                        Err(payload) => {
                            // futures_util 的 catch_unwind:Err 即 panic 载荷本体。
                            let message = payload
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                                .unwrap_or_else(|| "<non-string panic payload>".to_owned());
                            ipc_diagnostics.info(
                                "ipc",
                                &format!("ingress handler panicked: {message}; loop survived"),
                            );
                            ipc_diagnostics.info("ipc", "ingress request rejected");
                            ipc_rejected.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            crate::worker::emit(
                                &ipc_events,
                                crate::worker::CoreEvent::HealthChanged { channel_id: None },
                            );
                            IngressResponse::Rejected {
                                error_code: "unrecognized".to_owned(),
                            }
                        }
                    };
                let _ = response.send(reply).await;
            }
        });

        // 5d. Delivery worker with the real sender bridge. Keep the join
        //     handle: RunEvent::Exit waits (≤10s) on it so an in-flight send
        //     pass finishes instead of being killed mid-write.
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
        let worker_events = core_event_sink.clone();
        let worker = DeliveryWorker::new(worker_config, worker_events);
        let worker_cancel = bg_cancel.clone();
        let worker_task = tauri::async_runtime::spawn(async move {
            let _ = worker.run(worker_cancel).await;
        });
        {
            let mut guard = bg_state_for_worker.runtime.worker_task.lock().unwrap();
            *guard = Some(worker_task);
        }

        // 5e. `core://` forwarder: the single consumer of the bounded event
        //     channel shared by the delivery worker and the command surface.
        //     Each CoreEvent becomes a revision-only payload on its topic
        //     (`worker::CoreEvent::core_topic`), matching what
        //     `src/lib/backend.tsx` subscribe listeners expect — they refetch
        //     on any revision bump and never trust payload details. The
        //     channel is bounded (64): when the WebView is slow the producer
        //     drops the NEWEST notification (see `worker::emit`) because
        //     revisions self-heal on the next tick/refetch.
        let forwarder_app = bg_app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Emitter;
            let mut revisions: std::collections::HashMap<&'static str, u64> =
                std::collections::HashMap::new();
            while let Some(event) = core_event_receiver.recv().await {
                let topic = event.core_topic();
                let revision = revisions.entry(topic).and_modify(|r| *r += 1).or_insert(1);
                let _ = forwarder_app.emit(topic, serde_json::json!({ "revision": revision }));
                // 健康变化同步托盘菜单的健康条目(重建,轻量)。
                if matches!(event, crate::worker::CoreEvent::HealthChanged { .. }) {
                    crate::tray::refresh_health(&forwarder_app);
                }
            }
        });

        // 5f. Retention (Task 20 Step 5): one pass immediately after startup,
        //     then every 24 hours. Failures are logged through Diagnostics and
        //     tolerated; shutdown rides the same cancel token as the worker.
        let retention_service =
            storage::retention::RetentionService::new(database.clone(), bg_logs);
        let retention_diagnostics = bg_diagnostics.clone();
        let retention_cancel = bg_cancel.clone();
        tauri::async_runtime::spawn(storage::retention::run_forever(
            retention_service,
            retention_diagnostics,
            std::time::Duration::from_secs(24 * 60 * 60),
            retention_cancel,
        ));

        // 5g. v2-issues(计划行 2482):每 6 小时后台重检 Agent,健康状态跟上
        //     升级/卸载等外部变化,并广播 health-changed 让界面刷新。
        let redetect_integrations = integrations.clone();
        let redetect_diagnostics = bg_diagnostics.clone();
        let redetect_events = core_event_sink.clone();
        let redetect_cancel = bg_cancel.clone();
        tauri::async_runtime::spawn(agents::redetect_loop(
            redetect_integrations,
            redetect_diagnostics,
            redetect_events,
            std::time::Duration::from_secs(6 * 60 * 60),
            redetect_cancel,
        ));
        bg_diagnostics.info("startup", "background init complete");
    });

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
