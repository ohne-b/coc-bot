#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
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
mod updates;

struct Desktop {
    application: Result<Arc<Application>, String>,
    closing: AtomicBool,
    drained: AtomicBool,
    updates: updates::Updates,
}

impl Desktop {
    fn core(&self) -> Result<&Application, String> {
        if self.closing.load(Ordering::SeqCst) || self.drained.load(Ordering::SeqCst) {
            return Err("The application has stopped. Restart it to continue.".into());
        }
        self.application
            .as_ref()
            .map(Arc::as_ref)
            .map_err(Clone::clone)
    }

    async fn drain(&self) -> Result<(), String> {
        self.drained.store(true, Ordering::SeqCst);
        if let Ok(core) = &self.application {
            core.shutdown().await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[tauri::command]
fn snapshot(state: State<'_, Desktop>) -> Result<Snapshot, String> {
    Ok(state.application.as_ref().map_err(Clone::clone)?.snapshot())
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
async fn discover(
    state: State<'_, Desktop>,
    folder: Option<PathBuf>,
) -> Result<Vec<Installation>, String> {
    state
        .core()?
        .discover(folder)
        .await
        .map_err(|e| e.to_string())
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
async fn scan_army(
    state: State<'_, Desktop>,
    screen: coc_bot_core::calibration::Screen,
) -> Result<(), String> {
    state
        .core()?
        .scan_army(screen)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn preview_deployment(
    state: State<'_, Desktop>,
    screen: coc_bot_core::calibration::Screen,
) -> Result<coc_bot_core::vision::army::recipe::Recipe, String> {
    state
        .core()?
        .preview_deployment(screen)
        .map_err(|e| e.to_string())
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

#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    finish(&app, true);
}

fn finish(app: &tauri::AppHandle, restart: bool) {
    let state = app.state::<Desktop>();
    if state.closing.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<Desktop>();
        state.updates.shutdown().await;
        let _ = state.drain().await;
        if restart {
            handle.restart();
        } else {
            handle.exit(0);
        }
    });
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
                drained: AtomicBool::new(false),
                updates: updates::Updates::default(),
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
            scan_army,
            preview_deployment,
            history,
            session_detail,
            updates::desktop_update,
            restart_app
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                finish(window.app_handle(), false);
            }
        })
        .run(context)
        .expect("Unable to launch CoC Bot");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drained_application_keeps_its_snapshot_but_rejects_new_work() {
        let directory = tempfile::tempdir().unwrap();
        let assets = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
        let core = Arc::new(
            Application::open(directory.path().to_path_buf(), assets)
                .await
                .unwrap(),
        );
        let state = Desktop {
            application: Ok(core.clone()),
            closing: AtomicBool::new(false),
            drained: AtomicBool::new(false),
            updates: updates::Updates::default(),
        };
        assert!(state.core().is_ok());
        state.closing.store(true, Ordering::SeqCst);
        assert!(state.core().is_err());
        state.drain().await.unwrap();
        // An installer launch failure reopens the window, not the drained bot engine.
        state.closing.store(false, Ordering::SeqCst);
        assert!(state.core().is_err());
        assert!(core.snapshot().session.is_none());
        state.drain().await.unwrap();
    }
}
