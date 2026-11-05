use base64::{Engine as _, prelude::BASE64_STANDARD};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use coc_bot_core::{
    Error, Result,
    calibration::{Calibration, Resolution, Screen},
    device::{
        Adb, DeviceConfig, Gesture, InputMode, Point, TouchDevice,
        ldplayer::{self, Connection, LdPlayer},
        process::{Invocation, Transport},
    },
    vision::{Detector, read_image},
};
use opencv::{core::Mat, imgproc, prelude::*};
use tokio_util::sync::CancellationToken;

const EVENTS: &str = "add device 1: /dev/input/event4\n name: ldtouch\n ABS (0003): ABS_MT_SLOT : value 0, min 0, max 9\n ABS_MT_POSITION_X : value 0, min 0, max 1279, fuzz 0\n ABS_MT_POSITION_Y : value 0, min 0, max 719, fuzz 0\n";
const INSTANCES: &str =
    "0,LDPlayer,0,0,1,1234,5678,1280,720,240\n1,Farm 2,0,0,0,-1,-1,1280,720,240\n";

#[derive(Default)]
struct State {
    calls: Vec<Invocation>,
    overrides: BTreeMap<String, Vec<u8>>,
    frame: Vec<u8>,
    fail_touch: bool,
    guard_after_input: bool,
    cancel_touch: bool,
    cancel_capture: bool,
    cancel_discovery: bool,
    capture_delay: Duration,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Transport for Fake {
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
        let delay = self.0.lock().unwrap().capture_delay;
        if args.ends_with("exec-out screencap") && !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let mut state = self.0.lock().unwrap();
        state.calls.push(command);
        if args.contains("Get-Process") && state.cancel_discovery {
            cancel.cancel();
            return Ok(vec![]);
        }
        if let Some(value) = state.overrides.get(&args) {
            return Ok(value.clone());
        }
        if args.ends_with("exec-out screencap") {
            if state.cancel_capture {
                cancel.cancel();
            }
            return Ok(state.frame.clone());
        }
        if (args.contains("shell input tap") || args.contains("toybox base64 -d"))
            && state.guard_after_input
        {
            state.guard_after_input = false;
            return Err(Error::InputGuard);
        }
        if args.contains("toybox base64 -d") && state.fail_touch {
            state.fail_touch = false;
            return Err(Error::Timeout);
        }
        if args.contains("toybox base64 -d") && state.cancel_touch {
            state.cancel_touch = false;
            cancel.cancel();
            return Err(Error::Cancelled);
        }
        Ok(match args.as_str() {
            "devices" => {
                b"List of devices attached\nemulator-5554\tdevice\nemulator-5556\toffline\n"
                    .to_vec()
            }
            "list2" => INSTANCES.as_bytes().to_vec(),
            "-s emulator-5554 shell getprop sys.boot_completed" => b"1\n".to_vec(),
            "-s emulator-5554 shell wm size" => b"Physical size: 1280x720\n".to_vec(),
            "-s emulator-5554 shell wm density" => b"Physical density: 240\n".to_vec(),
            "-s emulator-5554 shell uname -m" => b"x86_64\n".to_vec(),
            "-s emulator-5554 shell getevent -pl" => EVENTS.as_bytes().to_vec(),
            _ => vec![],
        })
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
fn detector() -> Arc<Detector> {
    Arc::new(
        Detector::load(
            &Calibration::load(&root().join("config.json")).unwrap(),
            &root(),
        )
        .unwrap(),
    )
}

fn setup() -> (tempfile::TempDir, DeviceConfig, Fake) {
    let folder = tempfile::tempdir().unwrap();
    for file in ["adb.exe", "ldconsole.exe"] {
        std::fs::write(folder.path().join(file), []).unwrap();
    }
    let image =
        read_image(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/english-home.png"))
            .unwrap();
    let mut rgba = Mat::default();
    imgproc::cvt_color_def(&image, &mut rgba, imgproc::COLOR_BGR2RGBA).unwrap();
    let fake = Fake::default();
    fake.0.lock().unwrap().frame = [1280_u32, 720, 1]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .chain(rgba.data_bytes().unwrap().iter().copied())
        .collect();
    let config = DeviceConfig {
        adb: folder.path().join("adb.exe"),
        serial: "emulator-5554".into(),
        resolution: Resolution(1280, 720),
        instance_index: 0,
    };
    (folder, config, fake)
}

#[test]
fn parses_devices_instances_display_and_touch_without_guessing() {
    let instances = ldplayer::parse_instances(INSTANCES);
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[1].serial, "emulator-5556");
    assert!(!instances[1].running);
    assert!(ldplayer::parse_instances("--bad,name,0,0,1,0,0\n255,name,0,0,1,0,0\n").is_empty());
    let devices = ldplayer::parse_devices(
        "List of devices attached\na device\nb offline\nc unauthorized\n--bad device\n",
    );
    assert_eq!(
        devices.iter().map(|d| &d.connection).collect::<Vec<_>>(),
        vec![
            &Connection::Connected,
            &Connection::Offline,
            &Connection::Unauthorized
        ]
    );
    let display = ldplayer::parse_display(
        "Physical size: 720x1280\nOverride size: 1280x720",
        "Physical density: 320\nOverride density: 240",
    )
    .unwrap();
    assert_eq!(display.resolution, Resolution(1280, 720));
    assert_eq!(display.dpi, 240);
    assert!(
        ldplayer::parse_display(
            "Physical size: 1280x720\nOverride size: broken",
            "Physical density: 240"
        )
        .is_err()
    );
    assert_eq!(
        TouchDevice::parse("x86_64", EVENTS, Resolution(1280, 720))
            .unwrap()
            .unwrap()
            .node(),
        "/dev/input/event4"
    );
    assert!(TouchDevice::parse("i686", EVENTS, Resolution(1280, 720)).is_err());
    assert!(
        TouchDevice::parse("x86_64", EVENTS, Resolution(1600, 900))
            .unwrap()
            .is_none()
    );
    assert!(
        TouchDevice::parse(
            "x86_64",
            &EVENTS.replace("event4", "event4;reboot"),
            Resolution(1280, 720)
        )
        .unwrap()
        .is_none()
    );
    assert!(
        TouchDevice::parse(
            "x86_64",
            &EVENTS.replace("min 0", "min 1"),
            Resolution(1280, 720)
        )
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn dry_run_and_cancelled_commands_never_send_input_or_launch() {
    let (_folder, config, fake) = setup();
    let token = CancellationToken::new();
    let mut adb = Adb::new(config.clone(), InputMode::Observe, fake.clone()).unwrap();
    assert!(matches!(
        adb.tap(detector(), &[Screen::Home], Point(85, 620), &token)
            .await,
        Err(Error::InputDisabled)
    ));
    assert!(matches!(
        adb.gesture(
            detector(),
            &[Screen::Home],
            Gesture::Taps(vec![Point(1, 2)]),
            &token
        )
        .await,
        Err(Error::InputDisabled)
    ));
    assert!(adb.launch_game(&token).await.is_err());
    assert!(adb.stop_game(&token).await.is_err());
    assert!(fake.0.lock().unwrap().calls.is_empty());
    assert!(adb.capture(&token).await.is_ok());
    fake.0.lock().unwrap().calls.clear();
    token.cancel();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    assert!(matches!(
        adb.tap(detector(), &[Screen::Home], Point(85, 620), &token)
            .await,
        Err(Error::Cancelled)
    ));
    assert!(fake.0.lock().unwrap().calls.is_empty());
}

#[tokio::test]
async fn only_a_fresh_expected_screen_can_receive_a_tap() {
    let (_folder, config, fake) = setup();
    let token = CancellationToken::new();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    assert!(matches!(
        adb.tap(detector(), &[Screen::Builder], Point(85, 620), &token)
            .await,
        Err(Error::InputGuard)
    ));
    assert!(
        fake.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| !call.args.iter().any(|a| a == "input"))
    );
    adb.tap(detector(), &[Screen::Home], Point(85, 620), &token)
        .await
        .unwrap();
    assert_eq!(
        fake.0.lock().unwrap().calls.last().unwrap().args,
        ["-s", "emulator-5554", "shell", "input", "tap", "85", "620"].map(std::ffi::OsString::from)
    );
    fake.0.lock().unwrap().frame[12..].fill(0);
    assert!(matches!(
        adb.swipe(
            detector(),
            &[Screen::Home],
            Point(1, 1),
            Point(2, 2),
            &token
        )
        .await,
        Err(Error::InputGuard)
    ));
}

#[tokio::test]
async fn rejects_bad_settings_points_display_and_disconnected_devices() {
    let (_folder, config, fake) = setup();
    for serial in ["emulator-5556", "-s", "emulator-5554; reboot", ""] {
        let mut invalid = config.clone();
        invalid.serial = serial.into();
        assert!(Adb::new(invalid, InputMode::Live, fake.clone()).is_err());
    }
    let token = CancellationToken::new();
    let mut adb = Adb::new(config.clone(), InputMode::Live, fake.clone()).unwrap();
    assert!(
        adb.tap(detector(), &[Screen::Home], Point(u32::MAX, 0), &token)
            .await
            .is_err()
    );
    assert!(fake.0.lock().unwrap().calls.is_empty());
    fake.0.lock().unwrap().overrides.insert(
        "-s emulator-5554 shell wm density".into(),
        b"Physical density: 320".to_vec(),
    );
    assert!(
        adb.tap(detector(), &[Screen::Home], Point(1, 1), &token)
            .await
            .is_err()
    );
    fake.0.lock().unwrap().overrides.insert(
        "devices".into(),
        b"List of devices attached\nemulator-5554 unauthorized".to_vec(),
    );
    assert!(adb.verify(&token).await.is_err());
    assert!(
        fake.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| !call.args.iter().any(|a| a == "input"))
    );
}

#[tokio::test]
async fn cancelled_capture_never_advances_to_input_and_bad_captures_are_bounded() {
    let (_folder, config, fake) = setup();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    fake.0.lock().unwrap().cancel_capture = true;
    assert!(matches!(
        adb.tap(
            detector(),
            &[Screen::Home],
            Point(1, 1),
            &CancellationToken::new()
        )
        .await,
        Err(Error::Cancelled)
    ));
    assert!(
        fake.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| !call.args.iter().any(|a| a == "input"))
    );
    {
        let mut state = fake.0.lock().unwrap();
        state.cancel_capture = false;
        state.frame.clear();
        state.calls.clear();
    }
    assert!(matches!(
        adb.capture(&CancellationToken::new()).await,
        Err(Error::Frame)
    ));
    assert_eq!(fake.0.lock().unwrap().calls.len(), 3);
}

#[tokio::test]
async fn failed_multitouch_releases_every_finger_with_a_separate_deadline() {
    let (_folder, config, fake) = setup();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    fake.0.lock().unwrap().fail_touch = true;
    assert!(matches!(
        adb.gesture(
            detector(),
            &[Screen::Home],
            Gesture::Hold {
                points: vec![Point(400, 300), Point(700, 300)],
                duration: Duration::from_secs(1)
            },
            &CancellationToken::new()
        )
        .await,
        Err(Error::Timeout)
    ));
    let state = fake.0.lock().unwrap();
    let commands: Vec<_> = state
        .calls
        .iter()
        .filter(|call| call.args.iter().any(|arg| arg == "/dev/input/event4"))
        .collect();
    assert_eq!(commands.len(), 2);
    for command in &commands {
        assert!(command.args.iter().any(|arg| arg == "-T"));
        assert!(!command.args.iter().any(|arg| arg == "exec-in"));
        let replay = command.args[6].to_string_lossy();
        assert!(replay.contains("set -o pipefail || exit"));
        assert!(replay.contains("toybox dd ibs=4096 obs=24"));
        for frame in &command.input {
            assert!(frame.bytes.is_ascii());
            assert_eq!(frame.bytes.last(), Some(&b'\n'));
            assert_eq!(
                BASE64_STANDARD
                    .decode(frame.bytes.trim_ascii())
                    .unwrap()
                    .len()
                    % 24,
                0
            );
        }
    }
    assert_eq!(commands[1].timeout, Duration::from_secs(2));
    assert_eq!(
        BASE64_STANDARD
            .decode(commands[1].input[0].bytes.trim_ascii())
            .unwrap()
            .len(),
        6 * 24
    );
    assert!(
        commands[1]
            .args
            .iter()
            .any(|arg| arg == "/dev/input/event4")
    );
}

#[tokio::test]
async fn a_dispatched_transport_failure_cannot_claim_that_input_was_withheld() {
    for gesture in [false, true] {
        let (_folder, config, fake) = setup();
        fake.0.lock().unwrap().guard_after_input = true;
        let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
        let cancel = CancellationToken::new();
        let result = if gesture {
            adb.gesture(
                detector(),
                &[Screen::Home],
                Gesture::Taps(vec![Point(400, 300)]),
                &cancel,
            )
            .await
        } else {
            adb.tap(detector(), &[Screen::Home], Point(400, 300), &cancel)
                .await
        };
        assert!(
            matches!(result, Err(Error::Device(message)) if message.contains("after dispatch"))
        );
        let state = fake.0.lock().unwrap();
        let inputs = state
            .calls
            .iter()
            .filter(|call| {
                call.args
                    .iter()
                    .any(|arg| arg == "tap" || arg == "/dev/input/event4")
            })
            .count();
        assert_eq!(inputs, if gesture { 2 } else { 1 });
    }
}

#[tokio::test]
async fn touch_frames_preserve_windows_control_bytes_and_press_timing() {
    let (_folder, config, fake) = setup();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    adb.gesture(
        detector(),
        &[Screen::Home],
        Gesture::Taps(vec![Point(10, 26), Point(13, 126)]),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let state = fake.0.lock().unwrap();
    let command = state
        .calls
        .iter()
        .find(|call| call.args.iter().any(|arg| arg == "/dev/input/event4"))
        .unwrap();
    assert_eq!(command.input.len(), 4);
    for (index, frame) in command.input.iter().enumerate() {
        assert_eq!(frame.wait, Duration::from_millis(30));
        let line = String::from_utf8(frame.bytes.clone())
            .unwrap()
            .replace('\n', "\r\n");
        let decoded = BASE64_STANDARD.decode(line.trim()).unwrap();
        if index % 2 == 0 {
            assert_eq!(decoded.len(), 7 * 24);
            let point = if index == 0 {
                Point(10, 26)
            } else {
                Point(13, 126)
            };
            assert_eq!(&decoded[2 * 24 + 20..3 * 24], &point.0.to_le_bytes());
            assert_eq!(&decoded[3 * 24 + 20..4 * 24], &point.1.to_le_bytes());
        } else {
            assert_eq!(decoded.len(), 4 * 24);
            assert_eq!(&decoded[24 + 20..2 * 24], &(-1_i32).to_le_bytes());
        }
    }
}

#[tokio::test]
async fn cancellation_during_multitouch_still_releases_fingers() {
    let (_folder, config, fake) = setup();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    fake.0.lock().unwrap().cancel_touch = true;
    let token = CancellationToken::new();
    assert!(matches!(
        adb.gesture(
            detector(),
            &[Screen::Home],
            Gesture::Taps(vec![Point(400, 300)]),
            &token
        )
        .await,
        Err(Error::Cancelled)
    ));
    assert!(token.is_cancelled());
    let state = fake.0.lock().unwrap();
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|call| call.args.iter().any(|arg| arg == "/dev/input/event4"))
            .count(),
        2
    );
}

#[tokio::test]
async fn checked_gestures_resolve_fresh_frames_and_reject_stale_or_cancelled_resolvers() {
    let (_folder, config, fake) = setup();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    let token = CancellationToken::new();
    let gesture = || Gesture::TimedTaps {
        points: vec![Point(10, 20), Point(30, 40)],
        spacing: Duration::from_millis(100),
    };
    let value = adb
        .gesture_checked(
            detector(),
            &[Screen::Home],
            |image| {
                assert_eq!(image.cols(), 1280);
                assert_eq!(image.rows(), 720);
                Ok((gesture(), 7))
            },
            &token,
        )
        .await
        .unwrap();
    assert_eq!(value, 7);
    fake.0.lock().unwrap().calls.clear();
    assert!(matches!(
        adb.gesture_checked(
            detector(),
            &[Screen::Builder],
            |_| { panic!("A mismatched frame must not reach the resolver") },
            &token
        )
        .await,
        Err::<(), _>(Error::InputGuard)
    ));
    assert!(matches!(
        adb.gesture_checked(
            detector(),
            &[Screen::Home],
            |_| {
                std::thread::sleep(Duration::from_millis(2100));
                Ok((gesture(), ()))
            },
            &token
        )
        .await,
        Err(Error::InputGuard)
    ));
    assert!(matches!(
        adb.gesture_checked(
            detector(),
            &[Screen::Home],
            |_| {
                token.cancel();
                Ok((gesture(), ()))
            },
            &token
        )
        .await,
        Err(Error::Cancelled)
    ));
    assert!(
        fake.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| !call.args.iter().any(|arg| arg == "/dev/input/event4"))
    );
}

#[tokio::test]
async fn checked_tap_bursts_release_after_uncertain_transport_or_cancellation() {
    for cancelled in [false, true] {
        let (_folder, config, fake) = setup();
        let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
        {
            let mut state = fake.0.lock().unwrap();
            state.fail_touch = !cancelled;
            state.cancel_touch = cancelled;
        }
        let token = CancellationToken::new();
        let result = adb
            .gesture_checked(
                detector(),
                &[Screen::Home],
                |_| {
                    Ok((
                        Gesture::TimedTaps {
                            points: vec![Point(400, 300); 5],
                            spacing: Duration::from_millis(100),
                        },
                        (),
                    ))
                },
                &token,
            )
            .await;
        if cancelled {
            assert!(matches!(result, Err(Error::Cancelled)));
        } else {
            assert!(matches!(result, Err(Error::Timeout)));
        }
        let state = fake.0.lock().unwrap();
        let commands: Vec<_> = state
            .calls
            .iter()
            .filter(|call| call.args.iter().any(|arg| arg == "/dev/input/event4"))
            .collect();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].input.len(), 10);
        assert_eq!(commands[1].timeout, Duration::from_secs(2));
        assert_eq!(commands[1].input.len(), 1);
        let release = BASE64_STANDARD
            .decode(commands[1].input[0].bytes.trim_ascii())
            .unwrap();
        assert_eq!(release.len(), 4 * 24);
        assert_eq!(&release[24 + 20..2 * 24], &(-1_i32).to_le_bytes());
    }
}

#[tokio::test]
async fn stale_and_ambiguous_frames_cannot_authorize_input() {
    let (_folder, config, fake) = setup();
    let mut adb = Adb::new(config, InputMode::Live, fake.clone()).unwrap();
    let mut calibration = Calibration::load(&root().join("config.json")).unwrap();
    calibration
        .screens
        .insert(Screen::Builder, calibration.screens[&Screen::Home].clone());
    let ambiguous = Arc::new(Detector::load(&calibration, &root()).unwrap());
    assert!(matches!(
        adb.tap(
            ambiguous,
            &[Screen::Home],
            Point(1, 1),
            &CancellationToken::new()
        )
        .await,
        Err(Error::InputGuard)
    ));
    fake.0.lock().unwrap().capture_delay = Duration::from_millis(2100);
    assert!(matches!(
        adb.tap(
            detector(),
            &[Screen::Home],
            Point(1, 1),
            &CancellationToken::new()
        )
        .await,
        Err(Error::InputGuard)
    ));
    assert!(
        fake.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| !call.args.iter().any(|arg| arg == "input"))
    );
}

#[tokio::test]
async fn ldplayer_discovery_is_read_only_and_lifecycle_targets_one_instance() {
    let (folder, _, fake) = setup();
    let token = CancellationToken::new();
    let mut player = LdPlayer::new(
        folder.path().join("ldconsole.exe"),
        1,
        InputMode::Observe,
        fake.clone(),
    )
    .unwrap();
    assert_eq!(player.instances(&token).await.unwrap().len(), 2);
    assert!(!player.running(&token).await.unwrap());
    assert!(player.launch(&token).await.is_err());
    assert!(player.quit(&token).await.is_err());
    let mut player = LdPlayer::new(
        folder.path().join("ldconsole.exe"),
        1,
        InputMode::Live,
        fake.clone(),
    )
    .unwrap();
    player.launch(&token).await.unwrap();
    assert_eq!(
        fake.0.lock().unwrap().calls.last().unwrap().args,
        ["launch", "--index", "1"].map(std::ffi::OsString::from)
    );
    let folders = ldplayer::installed_folders(
        Some(folder.path().to_owned()),
        vec![folder.path().to_owned()],
        &fake,
        &token,
    )
    .await
    .unwrap();
    assert_eq!(folders, vec![folder.path().canonicalize().unwrap()]);
    fake.0.lock().unwrap().cancel_discovery = true;
    assert!(matches!(
        ldplayer::installed_folders(
            Some(folder.path().to_owned()),
            vec![folder.path().to_owned()],
            &fake,
            &token,
        )
        .await,
        Err(Error::Cancelled)
    ));
}

#[tokio::test]
async fn memory_probe_preserves_unknown_and_cancellation_without_device_commands() {
    if std::env::var_os("SystemRoot").is_none() {
        return;
    }
    let fake = Fake::default();
    let key = "-NoProfile -NonInteractive -Command (Get-CimInstance Win32_OperatingSystem -Property FreeVirtualMemory -ErrorAction Stop).FreeVirtualMemory";
    for (text, expected) in [
        ("1024000\r\n", Some(1000)),
        ("0", Some(0)),
        ("", None),
        ("-1", None),
        ("error", None),
    ] {
        fake.0
            .lock()
            .unwrap()
            .overrides
            .insert(key.into(), text.as_bytes().to_vec());
        assert_eq!(
            ldplayer::available_memory_mb(&fake, &CancellationToken::new())
                .await
                .unwrap(),
            expected
        );
    }
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        ldplayer::available_memory_mb(&fake, &cancel).await,
        Err(Error::Cancelled)
    ));
    let state = fake.0.lock().unwrap();
    assert_eq!(state.calls.len(), 5);
    assert!(
        state
            .calls
            .iter()
            .all(|call| call.program.ends_with("powershell.exe")
                && call.timeout == Duration::from_secs(10)
                && call.output_limit == 4096)
    );
}
