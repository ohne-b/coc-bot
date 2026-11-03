#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use coc_bot_core::{
    application::{Application, Installation, SessionDetail, Snapshot},
    settings::SettingsUpdate,
    telemetry::{Session, SessionMode},
};
use tauri::{Manager, State};

#[cfg(feature = "desktop-fixture")]
mod fixture;
mod installation;

struct Desktop {
    application: Result<Arc<Application>, String>,
    closing: AtomicBool,
}

impl Desktop {
    fn core(&self) -> Result<&Application, String> {
        self.application
            .as_ref()
            .map(Arc::as_ref)
            .map_err(Clone::clone)
    }
}

#[tauri::command]
fn snapshot(state: State<'_, Desktop>) -> Result<Snapshot, String> {
    Ok(state.core()?.snapshot())
}
#[tauri::command]
fn screenshot(
    state: State<'_, Desktop>,
    session_id: i64,
    sequence: u64,
) -> Result<tauri::ipc::Response, String> {
    state
        .core()?
        .screenshot(session_id, sequence)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn save_settings(state: State<'_, Desktop>, update: SettingsUpdate) -> Result<(), String> {
    state
        .core()?
        .update_settings(update)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn discover(state: State<'_, Desktop>) -> Result<Vec<Installation>, String> {
    state.core()?.discover().await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn start(state: State<'_, Desktop>, mode: Option<SessionMode>) -> Result<(), String> {
    state
        .core()?
        .start(mode.unwrap_or(SessionMode::Observe))
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn pause(state: State<'_, Desktop>, paused: bool) -> Result<(), String> {
    state.core()?.pause(paused).await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn stop(state: State<'_, Desktop>) -> Result<(), String> {
    state.core()?.stop().await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn history(state: State<'_, Desktop>, before: Option<i64>) -> Result<Vec<Session>, String> {
    state
        .core()?
        .history(before)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn session_detail(state: State<'_, Desktop>, id: i64) -> Result<SessionDetail, String> {
    state.core()?.detail(id).await.map_err(|e| e.to_string())
}

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--check-installation")) {
        let result = std::env::current_exe()
            .map_err(coc_bot_core::Error::from)
            .and_then(|exe| installation::check(exe.parent().ok_or(coc_bot_core::Error::Frame)?));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let context = tauri::generate_context!();
    #[cfg(feature = "desktop-fixture")]
    let context = {
        let mut context = context;
        context.config_mut().identifier.push_str(".fixture");
        context.config_mut().app.windows[0].title = "CoC Bot (fixture)".into();
        context
    };
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    #[cfg(not(feature = "desktop-fixture"))]
    let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());
    builder
        .setup(|app| {
            let assets = if cfg!(debug_assertions) {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets")
            } else {
                app.path().resource_dir()?
            };
            #[cfg(not(feature = "desktop-fixture"))]
            let application = tauri::async_runtime::block_on(Application::open(
                app.path().app_local_data_dir()?,
                assets,
            ))
            .map(Arc::new)
            .map_err(|e| e.to_string());
            #[cfg(feature = "desktop-fixture")]
            let application = match tauri::async_runtime::block_on(fixture::open(assets)) {
                Ok((core, data)) => {
                    app.manage(data);
                    Ok(Arc::new(core))
                }
                Err(error) => Err(error.to_string()),
            };
            app.manage(Desktop {
                application,
                closing: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pause,
            snapshot,
            screenshot,
            save_settings,
            discover,
            start,
            stop,
            history,
            session_detail
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                let state = app.state::<Desktop>();
                if state.closing.swap(true, Ordering::SeqCst) {
                    return;
                }
                let core = state.application.as_ref().ok().cloned();
                tauri::async_runtime::spawn(async move {
                    if let Some(core) = core {
                        let _ = core.shutdown().await;
                    }
                    app.exit(0);
                });
            }
        })
        .run(context)
        .expect("Unable to launch CoC Bot");
}
