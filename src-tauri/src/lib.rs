#![allow(
    missing_docs,
    reason = "initial command API is documented in AutoFox docs"
)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

pub mod download;
pub mod extract;
pub mod ipc;
pub mod logs;
pub mod mux;
pub mod queue;
mod settings;
mod system;
use queue::TaskRegistry;
use settings::SettingsState;
use tauri::Manager;

/// Starts the AutoFox desktop application.
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(SettingsState::default())
        .manage(TaskRegistry::default())
        .setup(|app| {
            app.state::<queue::TaskRegistry>().load(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            download::download_direct,
            extract::get_backend_status,
            extract::probe_url,
            logs::clear_logs,
            logs::read_logs,
            mux::download_stream,
            mux::mux_streams,
            queue::cancel_task,
            queue::list_tasks,
            queue::pause_task,
            queue::remove_task,
            queue::resume_task,
            settings::get_settings,
            settings::save_settings,
        ])
        .run(tauri::generate_context!())
}
