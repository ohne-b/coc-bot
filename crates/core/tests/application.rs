use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    telemetry::{SessionMode, SessionState, Village},
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
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap();
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
                && !args.contains("toybox base64 -d")
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
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
async fn contextual_tooltips_publish_the_captured_balance_and_capacity_for_the_correct_village() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/english-readings.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut checked = 0;
    for case in manifest["fixtures"].as_array().unwrap() {
        if !case["id"].as_str().unwrap().ends_with("-tooltip") {
            continue;
        }
        let folder = tempfile::tempdir().unwrap();
        let app = Application::with_transport(
            folder.path().join("data"),
            assets(),
            Fake::capture(case["file"].as_str().unwrap()),
        )
        .await
        .unwrap();
        app.save_settings(preferences(folder.path())).await.unwrap();
        app.start(SessionMode::Observe).await.unwrap();
        frame(&app).await;
        let snapshot = app.snapshot();
        let village = if case["screen"] == "home" {
            Village::Home
        } else {
            Village::Builder
        };
        for (key, reading) in case["readings"].as_object().unwrap() {
            let Some(currency) = key.strip_prefix("capacity.") else {
                continue;
            };
            let resource =
                serde_json::from_value::<coc_bot_core::telemetry::Resource>(currency.into())
                    .unwrap();
            let observed = snapshot
                .resources
                .iter()
                .find(|r| r.latest.village == village && r.latest.resource == resource)
                .unwrap();
            let balance = observed.last_known.unwrap();
            assert_eq!(
                u64::from(balance.amount),
                case["readings"][format!("balance.{currency}")]["value"]
                    .as_u64()
                    .unwrap(),
                "{}",
                case["id"]
            );
            assert_eq!(
                balance.capacity.map(u64::from),
                reading["value"].as_u64(),
                "{}",
                case["id"]
            );
            checked += 1;
        }
        app.shutdown().await.unwrap();
    }
    assert_eq!(checked, 5);
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
    for mode in [SessionMode::Observe, SessionMode::Farming] {
        let folder = tempfile::tempdir().unwrap();
        let mut fake = Fake::new();
        fake.bad_display = true;
        let app = Application::with_transport(folder.path().join("data"), assets(), fake)
            .await
            .unwrap();
        app.save_settings(preferences(folder.path())).await.unwrap();
        app.start(mode).await.unwrap();
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
}

#[derive(Clone)]
struct BrokenCapture {
    base: Fake,
    calls: Arc<Mutex<Vec<String>>>,
    launched: Arc<AtomicBool>,
    stays_broken: bool,
    stall_launch: bool,
    entered_launch: Arc<Notify>,
    quitting: Arc<AtomicBool>,
    stop_polls: Arc<AtomicUsize>,
    shutdown_delay: usize,
    entered_quit: Arc<Notify>,
}

impl BrokenCapture {
    fn new(stays_broken: bool, stall_launch: bool) -> Self {
        Self {
            base: Fake::new(),
            calls: Arc::new(Mutex::new(vec![])),
            launched: Arc::new(AtomicBool::new(false)),
            stays_broken,
            stall_launch,
            entered_launch: Arc::new(Notify::new()),
            quitting: Arc::new(AtomicBool::new(false)),
            stop_polls: Arc::new(AtomicUsize::new(0)),
            shutdown_delay: 0,
            entered_quit: Arc::new(Notify::new()),
        }
    }
}

impl Transport for BrokenCapture {
    async fn run(&self, command: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let args = command
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        self.calls.lock().unwrap().push(args.clone());
        match args.as_str() {
            "list2" => {
                let stopped = self.quitting.load(Ordering::SeqCst)
                    && self.stop_polls.fetch_add(1, Ordering::SeqCst) >= self.shutdown_delay;
                Ok(if stopped {
                    b"0,LDPlayer,0,0,0,0,0\n".to_vec()
                } else {
                    b"0,LDPlayer,0,0,1,123,456\n".to_vec()
                })
            }
            "quit --index 0" => {
                self.quitting.store(true, Ordering::SeqCst);
                self.stop_polls.store(0, Ordering::SeqCst);
                self.entered_quit.notify_one();
                Ok(vec![])
            }
            "launch --index 0" => {
                assert!(
                    self.stop_polls.load(Ordering::SeqCst) > self.shutdown_delay,
                    "launch raced shutdown"
                );
                self.entered_launch.notify_one();
                if self.stall_launch {
                    cancel.cancelled().await;
                    return Err(Error::Cancelled);
                }
                self.launched.store(true, Ordering::SeqCst);
                self.quitting.store(false, Ordering::SeqCst);
                Ok(vec![])
            }
            "-s emulator-5554 exec-out screencap"
                if self.stays_broken || !self.launched.load(Ordering::SeqCst) =>
            {
                Err(Error::Frame)
            }
            "-s emulator-5554 shell dumpsys window" => {
                Ok(b"mCurrentFocus=com.supercell.clashofclans/Game".to_vec())
            }
            _ => self.base.run(command, cancel).await,
        }
    }
}

#[tokio::test]
async fn a_connected_device_with_broken_capture_is_restarted_and_recovery_is_bounded() {
    for stays_broken in [false, true] {
        let folder = tempfile::tempdir().unwrap();
        let fake = BrokenCapture::new(stays_broken, false);
        let app = Application::with_transport(folder.path().join("data"), assets(), fake.clone())
            .await
            .unwrap();
        let mut settings = preferences(folder.path());
        settings.farming.recoveries = 2;
        app.save_settings(settings).await.unwrap();
        app.start(SessionMode::Farming).await.unwrap();
        // Two restarts include six seconds of backoff; allow CI scheduling and vision work too.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let snapshot = app.snapshot();
                if stays_broken {
                    if snapshot.session.unwrap().state == SessionState::Failed {
                        assert_eq!(snapshot.error.as_deref(), Some("Recovery limit reached"));
                        break;
                    }
                } else if snapshot.frame_sequence.is_some() {
                    assert_eq!(snapshot.detection, Detection::Known(Screen::Home));
                    assert!(snapshot.error.is_none());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        app.stop().await.unwrap();
        let calls = fake.calls.lock().unwrap().clone();
        let restarts = if stays_broken { 2 } else { 1 };
        assert_eq!(
            calls
                .iter()
                .filter(|args| *args == "quit --index 0")
                .count(),
            restarts
        );
        assert_eq!(
            calls
                .iter()
                .filter(|args| *args == "launch --index 0")
                .count(),
            restarts
        );
        assert!(
            calls
                .iter()
                .all(|args| !args.contains("input ") && !args.contains("toybox base64 -d"))
        );
        app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn stop_cancels_an_emulator_restart_without_launching_the_game() {
    let folder = tempfile::tempdir().unwrap();
    let fake = BrokenCapture::new(false, true);
    let app = Application::with_transport(folder.path().join("data"), assets(), fake.clone())
        .await
        .unwrap();
    app.save_settings(preferences(folder.path())).await.unwrap();
    app.start(SessionMode::Farming).await.unwrap();
    tokio::time::timeout(Duration::from_secs(6), fake.entered_launch.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), app.stop())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(app.snapshot().session.unwrap().state, SessionState::Stopped);
    assert!(!fake.launched.load(Ordering::SeqCst));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn emulator_restart_waits_for_shutdown_and_stop_cancels_that_wait() {
    for cancel_wait in [false, true] {
        let folder = tempfile::tempdir().unwrap();
        let mut fake = BrokenCapture::new(false, false);
        fake.shutdown_delay = if cancel_wait { usize::MAX } else { 2 };
        let app = Application::with_transport(folder.path().join("data"), assets(), fake.clone())
            .await
            .unwrap();
        app.save_settings(preferences(folder.path())).await.unwrap();
        app.start(SessionMode::Farming).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), fake.entered_quit.notified())
            .await
            .unwrap();
        if !cancel_wait {
            tokio::time::timeout(Duration::from_secs(8), fake.entered_launch.notified())
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), app.stop())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fake.launched.load(Ordering::SeqCst), !cancel_wait);
        assert_eq!(app.snapshot().session.unwrap().state, SessionState::Stopped);
        app.shutdown().await.unwrap();
    }
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
    assert!(matches!(
        app.discover(Some(PathBuf::from("relative"))).await,
        Err(Error::Device(_))
    ));
    let incomplete = folder.path().join("incomplete");
    std::fs::create_dir(&incomplete).unwrap();
    std::fs::write(incomplete.join("adb.exe"), []).unwrap();
    assert!(
        app.update_settings(SettingsUpdate::Device(Some(SelectedDevice {
            folder: incomplete,
            index: 0,
        })))
        .await
        .is_err()
    );
    assert_eq!(app.snapshot().settings, settings);
    app.update_settings(SettingsUpdate::Device(settings.device.clone()))
        .await
        .unwrap();
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
async fn saved_wall_option_selects_workflow_without_enabling_observation_input() {
    let folder = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.stall.store(true, Ordering::SeqCst);
    let app = Application::with_transport(folder.path().join("data"), assets(), fake)
        .await
        .unwrap();
    for (enabled, requested, expected) in [
        (false, SessionMode::Farming, SessionMode::Farming),
        (true, SessionMode::Farming, SessionMode::Walls),
        (true, SessionMode::Observe, SessionMode::Observe),
        (false, SessionMode::Walls, SessionMode::Walls),
    ] {
        let mut settings = preferences(folder.path());
        settings.farming.upgrade_walls = enabled;
        app.save_settings(settings).await.unwrap();
        app.start(requested).await.unwrap();
        assert_eq!(app.snapshot().session.unwrap().mode, expected);
        app.stop().await.unwrap();
    }
    app.shutdown().await.unwrap();
}

#[test]
fn old_settings_keep_wall_upgrades_disabled_and_preserve_reserves() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("history.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let mut settings = preferences(folder.path());
    settings.farming.gold_reserve = 2_000_000;
    store.save_settings(&settings).unwrap();
    let mut old = serde_json::to_value(&settings).unwrap();
    old["farming"]
        .as_object_mut()
        .unwrap()
        .remove("upgrade_walls");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE settings SET data = ?1", [old.to_string()])
        .unwrap();
    assert_eq!(Store::open(&path).unwrap().settings().unwrap(), settings);
    settings.farming.upgrade_walls = true;
    store.save_settings(&settings).unwrap();
    assert_eq!(Store::open(&path).unwrap().settings().unwrap(), settings);
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
