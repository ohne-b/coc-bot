use coc_bot_core::{
    Error, Result,
    calibration::{Calibration, Resolution, Screen},
    device::{
        Adb, DeviceConfig, InputMode,
        process::{Invocation, Transport},
    },
    gameplay::{Engine, Options, profile::Profile},
    telemetry::{SessionMode, Village},
    vision::{Detection, Detector, Digits, read_image},
};
use opencv::{core::Mat, imgproc, prelude::*};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const EVENTS: &str = "add device 1: /dev/input/event4\n ABS_MT_SLOT : value 0, min 0, max 9\n ABS_MT_POSITION_X : value 0, min 0, max 1279\n ABS_MT_POSITION_Y : value 0, min 0, max 719\n";
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/python")
}
struct State {
    screen: String,
    frames: BTreeMap<String, (Mat, Vec<u8>)>,
    inputs: usize,
}
#[derive(Clone)]
struct Fake(Arc<Mutex<State>>);
impl Fake {
    fn new(builder: bool) -> Self {
        let files = [
            ("home", "english-home.png"),
            ("home_menu", "english-home_menu.png"),
            ("home_army", "english-home_army.png"),
            ("home_scout", "english-home_scout.png"),
            ("home_battle", "english-home_battle.png"),
            ("home_result", "english-home_result.png"),
            ("builder", "english-builder.png"),
            ("builder_menu", "english-builder_menu.png"),
            ("builder_scout", "english-builder_current.png"),
            ("builder_battle", "english-builder_battle.png"),
            ("builder_stage2", "english-builder_fourth_current.png"),
            ("builder_battle2", "english-builder_progress.png"),
            ("builder_result", "english-builder_result_or_stage2.png"),
        ];
        let frames = files
            .into_iter()
            .map(|(name, file)| {
                let image = read_image(&root().join("fixtures").join(file)).unwrap();
                let mut rgba = Mat::default();
                imgproc::cvt_color_def(&image, &mut rgba, imgproc::COLOR_BGR2RGBA).unwrap();
                let bytes = [1280_u32, 720, 1]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .chain(rgba.data_bytes().unwrap().iter().copied())
                    .collect();
                (name.to_owned(), (image, bytes))
            })
            .collect();
        Self(Arc::new(Mutex::new(State {
            screen: if builder { "builder" } else { "home" }.into(),
            frames,
            inputs: 0,
        })))
    }
    fn image(&self) -> Mat {
        let state = self.0.lock().unwrap();
        state.frames[&state.screen].0.clone()
    }
    fn screen(&self, name: &str) {
        self.0.lock().unwrap().screen = name.into();
    }
}
impl Transport for Fake {
    async fn run(&self, invocation: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let args = invocation
            .args
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let mut state = self.0.lock().unwrap();
        if args.ends_with("exec-out screencap") {
            return Ok(state.frames[&state.screen].1.clone());
        }
        if args.contains("shell input tap") {
            state.inputs += 1;
            let next = match (state.screen.as_str(), args.as_str()) {
                ("home", a) if a.ends_with("85 620") => "home_menu",
                ("home_menu", a) if a.ends_with("218 530") => "home_army",
                ("home_army", a) if a.ends_with("1130 641") => "home_scout",
                ("home_result", a) if a.ends_with("640 617") => "home",
                ("builder", a) if a.ends_with("85 620") => "builder_menu",
                ("builder_menu", a) if a.ends_with("950 472") => "builder_scout",
                ("builder_result", a) if a.ends_with("640 607") => "builder",
                _ => panic!("Unexpected tap on {}: {args}", state.screen),
            };
            state.screen = next.into();
            return Ok(vec![]);
        }
        if args.contains("exec-in dd") {
            state.inputs += 1;
            let deploy = invocation
                .input
                .iter()
                .flat_map(|f| f.bytes.chunks_exact(24))
                .any(|event| {
                    u16::from_le_bytes(event[16..18].try_into().unwrap()) == 3
                        && u16::from_le_bytes(event[18..20].try_into().unwrap()) == 54
                        && i32::from_le_bytes(event[20..24].try_into().unwrap()) < 580
                });
            if deploy {
                state.screen = match state.screen.as_str() {
                    "home_scout" => "home_battle",
                    "builder_scout" => "builder_battle",
                    "builder_stage2" => "builder_battle2",
                    s => s,
                }
                .into();
            }
            return Ok(vec![]);
        }
        Ok(match args.as_str() {
            "devices" => b"List of devices attached\nemulator-5554\tdevice\n".to_vec(),
            "-s emulator-5554 shell getprop sys.boot_completed" => b"1".to_vec(),
            "-s emulator-5554 shell wm size" => b"Physical size: 1280x720".to_vec(),
            "-s emulator-5554 shell wm density" => b"Physical density: 240".to_vec(),
            "-s emulator-5554 shell uname -m" => b"x86_64".to_vec(),
            "-s emulator-5554 shell getevent -pl" => EVENTS.as_bytes().to_vec(),
            _ => panic!("Unexpected command: {args}"),
        })
    }
}

#[tokio::test]
async fn complete_home_and_two_stage_builder_attacks_use_only_guarded_inputs() {
    let detector = Arc::new(
        Detector::load(
            &Calibration::load(&root().join("config.json")).unwrap(),
            &root(),
        )
        .unwrap(),
    );
    let mut profile = Profile::load(&root()).unwrap();
    profile.zoom_out = None;
    profile.speed_button = None;
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    for village in [Village::Home, Village::Builder] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("adb.exe"), []).unwrap();
        let fake = Fake::new(village == Village::Builder);
        let mut adb = Adb::new(
            DeviceConfig {
                adb: temp.path().join("adb.exe"),
                serial: "emulator-5554".into(),
                instance_index: 0,
                resolution: Resolution(1280, 720),
            },
            InputMode::Live,
            fake.clone(),
        )
        .unwrap();
        let mut engine = Engine::new(
            Options {
                villages: vec![village],
                attacks: 1,
                collect_cart: false,
                ..Options::default()
            },
            SessionMode::Farming,
            &profile,
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let mut records = Vec::new();
        let mut completed = false;
        let mut stage2 = false;
        for tick in 0..240 {
            if engine.progress().operation == "In battle" {
                if village == Village::Builder && !stage2 {
                    fake.screen("builder_stage2");
                    stage2 = true;
                } else {
                    fake.screen(if village == Village::Home {
                        "home_result"
                    } else {
                        "builder_result"
                    });
                }
            }
            let image = fake.image();
            let detection = detector.detect(&image).unwrap();
            let result = engine
                .tick(
                    &coc_bot_core::gameplay::GameVision {
                        profile: &profile,
                        digits: &digits,
                        detector: detector.clone(),
                    },
                    &mut adb,
                    &coc_bot_core::gameplay::FrameView {
                        image: &image,
                        detection: &detection,
                        elapsed_ms: tick * 1000,
                        captured_at_ms: 1_700_000_000_000 + tick as i64 * 1000,
                    },
                    &cancel,
                )
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "{village:?} {} at {tick}: {error}",
                        engine.progress().operation
                    )
                });
            if let Some(attack) = result.attack {
                records.push(attack);
            }
            if result.completed {
                completed = true;
                break;
            }
        }
        assert!(completed, "{village:?}: {}", engine.progress().operation);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].village, village);
        assert_eq!(records[0].ordinal, 1);
        assert!(records[0].loot.gold.is_none());
        assert!(fake.0.lock().unwrap().inputs > 10);
        assert_eq!(stage2, village == Village::Builder);
    }
}

#[tokio::test]
async fn unknown_frames_recovery_limits_and_cancellation_never_authorize_input() {
    let detector = Arc::new(
        Detector::load(
            &Calibration::load(&root().join("config.json")).unwrap(),
            &root(),
        )
        .unwrap(),
    );
    let profile = Profile::load(&root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    let vision = coc_bot_core::gameplay::GameVision {
        profile: &profile,
        digits: &digits,
        detector,
    };
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("adb.exe"), []).unwrap();
    let fake = Fake::new(false);
    let mut adb = Adb::new(
        DeviceConfig {
            adb: temp.path().join("adb.exe"),
            serial: "emulator-5554".into(),
            instance_index: 0,
            resolution: Resolution(1280, 720),
        },
        InputMode::Live,
        fake.clone(),
    )
    .unwrap();
    let mut engine = Engine::new(
        Options {
            recoveries: 2,
            ..Options::default()
        },
        SessionMode::Walls,
        &profile,
    )
    .unwrap();
    let image = fake.image();
    let cancel = CancellationToken::new();
    let mut frame = coc_bot_core::gameplay::FrameView {
        image: &image,
        detection: &Detection::Unknown,
        elapsed_ms: 0,
        captured_at_ms: 1_700_000_000_000,
    };
    engine
        .tick(&vision, &mut adb, &frame, &cancel)
        .await
        .unwrap();
    frame.elapsed_ms = 30_000;
    assert!(matches!(
        engine.tick(&vision, &mut adb, &frame, &cancel).await,
        Err(Error::Timeout)
    ));
    assert_eq!(engine.recover(frame.elapsed_ms).unwrap(), 1);
    assert_eq!(engine.recover(frame.elapsed_ms).unwrap(), 2);
    assert!(matches!(
        engine.recover(frame.elapsed_ms),
        Err(Error::Gameplay(_))
    ));
    cancel.cancel();
    frame.detection = &Detection::Known(Screen::Home);
    assert!(matches!(
        engine.tick(&vision, &mut adb, &frame, &cancel).await,
        Err(Error::Cancelled)
    ));
    assert_eq!(fake.0.lock().unwrap().inputs, 0);
    assert!(Engine::new(Options::default(), SessionMode::Observe, &profile).is_err());
}

#[test]
fn deployment_settings_reject_missing_required_strategy_points() {
    use coc_bot_core::gameplay::Strategy;
    let profile = Profile::load(&root()).unwrap();
    let mut layout = profile
        .screen(Screen::HomeScout)
        .unwrap()
        .auto
        .clone()
        .unwrap();
    layout.ring.clear();
    let mut options = Options {
        strategy: Strategy::Ring,
        ..Options::default()
    };
    options.layouts.insert(Screen::HomeScout, layout.clone());
    assert!(options.validate(profile.resolution).is_err());
    options.strategy = Strategy::Stack;
    assert!(options.validate(profile.resolution).is_ok());
    layout.quake.clear();
    options.layouts.insert(Screen::HomeScout, layout);
    options.earthquake = true;
    assert!(options.validate(profile.resolution).is_err());
}
