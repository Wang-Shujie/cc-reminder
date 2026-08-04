pub mod actions;
pub mod error;
pub mod events;
pub mod hook_command;
pub mod ipc;
pub mod model;
pub mod paths;
pub mod projects;
pub mod rules;
pub mod security;
pub mod storage;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_| {
            let paths = paths::AppPaths::discover()?;
            paths.ensure()?;
            storage::db::Database::open(&paths.database)
                .map_err(|error| std::io::Error::other(error.code))?;
            let spool = storage::spool::Spool::new(paths.spool.clone())
                .map_err(|error| std::io::Error::other(error.code))?;
            spool
                .drain(500)
                .map_err(|error| std::io::Error::other(error.code))?;
            let mut server =
                ipc::server::IpcServer::bind(paths.endpoint()).map_err(std::io::Error::other)?;
            tauri::async_runtime::spawn(async move {
                while let Some((request, response)) = server.receiver.recv().await {
                    let reply = match hook_command::persist_ipc_request(&paths, request) {
                        Ok(event_id) => ipc::IngressResponse::Accepted { event_id },
                        Err(error_code) => ipc::IngressResponse::Rejected { error_code },
                    };
                    let _ = response.send(reply).await;
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .run(tauri::generate_context!())
        .expect("error while running CC Reminder");
}
