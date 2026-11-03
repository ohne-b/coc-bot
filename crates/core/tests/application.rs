use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use coc_bot_core::{
    Error, Result,
    application::Application,
    calibration::Screen,
    device::process::{Invocation, Transport},
    settings::{SelectedDevice, Settings},
    storage::Store,
    telemetry::{SessionState, Village},
    vision::{Detection, read_image},
};
use opencv::{core::Mat, imgproc, prelude::*};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Fake {
    frame: Arc<Vec<u8>>,
    stall: Arc<AtomicBool>,
    entered: Arc<Notify>,
    bad_display: bool,
}

impl Fake {
    fn new() -> Self {
        Self::capture("english-home.png")
    }

    fn capture(name: &str) -> Self {
        let image = read_image(&assets().join("fixtures").join(name)).unwrap();
        let mut rgba = Mat::default();
        imgproc::cvt_color_def(&image, &mut rgba, imgproc::COLOR_BGR2RGBA).unwrap();
        let frame = [1280_u32, 720, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .chain(rgba.data_bytes().unwrap().iter().copied())
            .collect();
        Self {
            frame: Arc::new(frame),
            stall: Arc::new(AtomicBool::new(false)),
            entered: Arc::new(Notify::new()),
            bad_display: false,
        }
    }
}

impl Transport for Fake {
    async fn run(&self, command: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let args = command
            .args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            !args.contains("input ")
                && !args.contains("exec-in")
                && !args.contains("launch")
                && !args.contains("force-stop"),
            "Observation must never send input: {args}"
        );
        Ok(match args.as_str() {
            "devices" => b"List of devices attached\nemulator-5554 device\n".to_vec(),
            "-s emulator-5554 shell getprop sys.boot_completed" => b"1".to_vec(),
            "-s emulator-5554 shell wm size" => b"Physical size: 1280x720".to_vec(),
            "-s emulator-5554 shell wm density" => {
                if self.bad_display {
                    b"Physical density: 320".to_vec()
                } else {
                    b"Physical density: 240".to_vec()
                }
            }
            "-s emulator-5554 exec-out screencap" => {
                self.entered.notify_one();
                if self.stall.load(Ordering::SeqCst) {
                    cancel.cancelled().await;
                    return Err(Error::Cancelled);
                }
                self.frame.as_ref().clone()
            }
            _ => panic!("Unexpected command {args}"),
        })
    }
}

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/python")
}
fn preferences(folder: &Path) -> Settings {
    std::fs::write(folder.join("adb.exe"), []).unwrap();
    std::fs::write(folder.join("ldconsole.exe"), []).unwrap();
    Settings {
        device: Some(SelectedDevice {
            folder: folder.to_owned(),
            index: 0,
        }),
        ..Settings::default()
    }
}
async fn frame(app: &Application) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if app.snapshot().frame_sequence.is_some() {
                break;
            }
            assert!(app.snapshot().error.is_none(), "{:?}", app.snapshot().error);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn observer_persists_real_readings_and_rejects_old_session_screenshots() {
    let folder = tempfile::tempdir().unwrap();
    let settings = preferences(folder.path());
    let app = Application::with_transport(folder.path().join("data"), assets(), Fake::new())
        .await
        .unwrap();
    assert!(
        app.start(coc_bot_core::telemetry::SessionMode::Observe)
            .await
            .is_err()
    );
    app.save_settings(settings.clone()).await.unwrap();
    app.start(coc_bot_core::telemetry::SessionMode::Observe)
        .await
        .unwrap();
    frame(&app).await;
    let snapshot = app.snapshot();
    assert_eq!(snapshot.detection, Detection::Known(Screen::Home));
    assert_eq!(snapshot.resources.len(), 3);
    assert!(
        snapshot
            .resources
            .iter()
            .all(|value| value.latest.village == Village::Home)
    );
    assert!(
        snapshot
            .resources
            .iter()
            .any(|value| value.last_known.is_some())
    );
    let id = snapshot.session.unwrap().id;
    assert_eq!(
        snapshot
            .resources
            .iter()
            .find(|value| value.latest.resource == coc_bot_core::telemetry::Resource::Gold)
            .unwrap()
            .last_known
            .unwrap()
            .amount,
        5_049_702
    );
    let sequence = snapshot.frame_sequence.unwrap();
    assert!(
        app.screenshot(id, sequence)
            .unwrap()
            .starts_with(&[0xff, 0xd8])
    );
    assert!(
        app.start(coc_bot_core::telemetry::SessionMode::Observe)
            .await
            .is_err()
    );
    assert!(app.save_settings(Settings::default()).await.is_err());
    app.stop().await.unwrap();
    assert_eq!(
        app.detail(id).await.unwrap().session.state,
        SessionState::Stopped
    );
    app.start(coc_bot_core::telemetry::SessionMode::Observe)
        .await
        .unwrap();
    assert!(app.screenshot(id, sequence).is_err());
    frame(&app).await;
    assert!(app.snapshot().session.unwrap().id > id);
    app.shutdown().await.unwrap();
    app.shutdown().await.unwrap();
    let store = Store::open(&folder.path().join("data/history.sqlite3")).unwrap();
    assert_eq!(store.settings().unwrap(), settings);
    assert_eq!(store.sessions(None, 10).unwrap().len(), 2);
    assert!(
        store
            .sessions(None, 10)
            .unwrap()
            .iter()
            .all(|s| s.state == SessionState::Stopped)
    );
}

#[tokio::test]
async fn an_isolated_tooltip_cannot_authorize_a_village_resource_reading() {
    let folder = tempfile::tempdir().unwrap();
    let app = Application::with_transport(
        folder.path().join("data"),
        assets(),
        Fake::capture("english-tooltip_gold.png"),
    )
    .await
    .unwrap();
    app.save_settings(preferences(folder.path())).await.unwrap();
    app.start(coc_bot_core::telemetry::SessionMode::Observe)
        .await
        .unwrap();
    frame(&app).await;
    let snapshot = app.snapshot();
    assert_eq!(snapshot.detection, Detection::Unknown);
    assert!(snapshot.resources.is_empty());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_and_shutdown_cancel_an_owned_capture_and_drain_the_worker() {
    for shutdown in [false, true] {
        let folder = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        fake.stall.store(true, Ordering::SeqCst);
        let app = Application::with_transport(folder.path().join("data"), assets(), fake.clone())
            .await
            .unwrap();
        app.save_settings(preferences(folder.path())).await.unwrap();
        app.start(coc_bot_core::telemetry::SessionMode::Observe)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), fake.entered.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            if shutdown {
                app.shutdown().await
            } else {
                app.stop().await
            }
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(app.snapshot().session.unwrap().state, SessionState::Stopped);
        assert!(app.snapshot().frame_sequence.is_none());
        app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn an_abandoned_start_request_remains_cancellable() {
    let folder = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.stall.store(true, Ordering::SeqCst);
    let app = Arc::new(
        Application::with_transport(folder.path().join("data"), assets(), fake.clone())
            .await
            .unwrap(),
    );
    app.save_settings(preferences(folder.path())).await.unwrap();
    let connection =
        rusqlite::Connection::open(folder.path().join("data/history.sqlite3")).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let pending = app.clone();
    let start = tokio::spawn(async move {
        pending
            .start(coc_bot_core::telemetry::SessionMode::Observe)
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    start.abort();
    assert!(start.await.unwrap_err().is_cancelled());
    connection.execute_batch("ROLLBACK").unwrap();
    tokio::time::timeout(Duration::from_secs(3), fake.entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), app.stop())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(app.snapshot().session.unwrap().state, SessionState::Stopped);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn device_failure_is_reported_and_recorded_without_capturing_or_input() {
    let folder = tempfile::tempdir().unwrap();
    let mut fake = Fake::new();
    fake.bad_display = true;
    let app = Application::with_transport(folder.path().join("data"), assets(), fake)
        .await
        .unwrap();
    app.save_settings(preferences(folder.path())).await.unwrap();
    app.start(coc_bot_core::telemetry::SessionMode::Observe)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while app.snapshot().session.as_ref().unwrap().state == SessionState::Running {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let snapshot = app.snapshot();
    assert_eq!(
        snapshot.session.as_ref().unwrap().state,
        SessionState::Failed
    );
    assert!(snapshot.error.unwrap().contains("240 DPI"));
    assert!(snapshot.frame_sequence.is_none());
    assert_eq!(
        app.detail(snapshot.session.unwrap().id)
            .await
            .unwrap()
            .activity
            .len(),
        1
    );
    app.shutdown().await.unwrap();
}

#[test]
fn settings_migration_preserves_history_and_never_defaults_on_corruption() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("history.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../src/storage/schema.sql"))
        .unwrap();
    connection.execute("INSERT INTO sessions (mode, started_at_ms, updated_at_ms, state, active_ms) VALUES ('observe', 100, 110, 'stopped', 10)", []).unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.sessions(None, 50).unwrap()[0].active_ms, 10);
    assert_eq!(store.settings().unwrap(), Settings::default());
    store.save_settings(&preferences(folder.path())).unwrap();
    connection
        .execute("UPDATE settings SET data = '{\"device\":true}'", [])
        .unwrap();
    assert!(store.settings().is_err());
    assert!(Store::open(&path).unwrap().settings().is_err());
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(
        store
            .save_settings(&Settings {
                device: Some(SelectedDevice {
                    folder: "relative".into(),
                    index: 0
                }),
                ..Settings::default()
            })
            .is_err()
    );
}

#[tokio::test]
async fn settings_patches_preserve_other_editors_and_failed_updates_are_atomic() {
    use coc_bot_core::{gameplay::Options, settings::SettingsUpdate};
    let folder = tempfile::tempdir().unwrap();
    let app = Application::with_transport(folder.path().join("data"), assets(), Fake::new())
        .await
        .unwrap();
    let settings = preferences(folder.path());
    app.save_settings(settings.clone()).await.unwrap();
    let mut layout = app.snapshot().default_layouts[&Screen::HomeScout].clone();
    layout.deploy = Some(coc_bot_core::device::Point(123, 456));
    app.update_settings(SettingsUpdate::Layout {
        screen: Screen::HomeScout,
        layout: Some(layout.clone()),
    })
    .await
    .unwrap();
    app.update_settings(SettingsUpdate::Farming(Options {
        attacks: 7,
        ..Options::default()
    }))
    .await
    .unwrap();
    let saved = app.snapshot().settings;
    assert_eq!(saved.device, settings.device);
    assert_eq!(saved.farming.layouts[&Screen::HomeScout], layout);
    assert_eq!(saved.farming.attacks, 7);
    assert!(
        app.update_settings(SettingsUpdate::Farming(Options {
            villages: vec![],
            ..Options::default()
        }))
        .await
        .is_err()
    );
    assert_eq!(app.snapshot().settings, saved);
    app.shutdown().await.unwrap();
    let store = Store::open(&folder.path().join("data/history.sqlite3")).unwrap();
    assert_eq!(store.settings().unwrap(), saved);
}

#[tokio::test]
async fn pause_freezes_active_time_and_keeps_settings_locked() {
    let folder = tempfile::tempdir().unwrap();
    let app = Application::with_transport(folder.path().join("data"), assets(), Fake::new())
        .await
        .unwrap();
    let settings = preferences(folder.path());
    app.save_settings(settings.clone()).await.unwrap();
    app.start(coc_bot_core::telemetry::SessionMode::Observe)
        .await
        .unwrap();
    frame(&app).await;
    app.pause(true).await.unwrap();
    let paused = app.snapshot().session.unwrap();
    assert_eq!(paused.state, SessionState::Paused);
    assert!(app.save_settings(settings).await.is_err());
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert_eq!(app.snapshot().session.unwrap().active_ms, paused.active_ms);
    app.pause(false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    app.stop().await.unwrap();
    let ended = app.snapshot().session.unwrap();
    assert!(ended.active_ms >= paused.active_ms + 20);
    assert!(ended.active_ms < paused.active_ms + 500);
    app.shutdown().await.unwrap();
}
