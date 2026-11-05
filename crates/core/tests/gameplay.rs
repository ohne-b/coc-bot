use coc_bot_core::{
    Error, Result,
    calibration::{Calibration, Resolution, Screen},
    device::{
        Adb, DeviceConfig, InputMode,
        process::{Invocation, Transport},
    },
    gameplay::{Engine, Options, profile::Profile},
    telemetry::{SessionMode, Village},
    vision::{Detection, Detector, Digits, army, read_image},
};
use opencv::{
    core::{self, Mat, Rect, Scalar, Size},
    imgproc,
    prelude::*,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const EVENTS: &str = "add device 1: /dev/input/event4\n ABS_MT_SLOT : value 0, min 0, max 9\n ABS_MT_POSITION_X : value 0, min 0, max 1279\n ABS_MT_POSITION_Y : value 0, min 0, max 719\n";
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
struct State {
    screen: String,
    frames: BTreeMap<String, Mat>,
    inputs: usize,
    world: World,
}

// Simulated camera and army state, rendered using captured scenery/HUD/card pixels.
// Only decoded device input changes this state; engine stages never advance it.
struct World {
    texture: Mat,
    cards: Vec<Mat>,
    range: [f64; 2],
    scale: f64,
    offset: [f64; 2],
    selected: bool,
    remaining: usize,
    deployed: usize,
    selections: usize,
    camera_moves: usize,
    builder: bool,
    anchors: BTreeMap<String, Vec<Rect>>,
}

fn fixture(name: &str) -> Mat {
    read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn paste(source: &Mat, from: Rect, target: &mut Mat, to: Rect) {
    Mat::roi(source, from)
        .unwrap()
        .copy_to(&mut Mat::roi_mut(target, to).unwrap())
        .unwrap();
}

fn framebuffer(image: &Mat) -> Vec<u8> {
    let mut rgba = Mat::default();
    imgproc::cvt_color_def(image, &mut rgba, imgproc::COLOR_BGR2RGBA).unwrap();
    [1280_u32, 720, 1]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .chain(rgba.data_bytes().unwrap().iter().copied())
        .collect()
}

impl World {
    fn new(builder: bool) -> Self {
        let profile = Profile::load(&root()).unwrap();
        let source = if builder {
            Screen::BuilderScout
        } else {
            Screen::HomeScout
        };
        let range = profile
            .screen(source)
            .unwrap()
            .camera
            .as_ref()
            .unwrap()
            .pan_range;
        let home = fixture("battle-bar-dragons-15-balloons-4.png");
        let captured = if builder {
            fixture("builder-barbarian-bar.png")
        } else {
            home.try_clone().unwrap()
        };
        let from = if builder {
            Rect::new(199, 0, 88, 120)
        } else {
            Rect::new(190, 0, 88, 134)
        };
        let mut cards = Vec::new();
        for count in 0..=4 {
            let mut card = Mat::roi(&captured, from).unwrap().try_clone().unwrap();
            if count == 0 {
                let mut gray = Mat::default();
                imgproc::cvt_color_def(&card, &mut gray, imgproc::COLOR_BGR2GRAY).unwrap();
                imgproc::cvt_color_def(&gray, &mut card, imgproc::COLOR_GRAY2BGR).unwrap();
            } else if count < 4 {
                // Captured spell labels use the same quantity glyphs as troop cards.
                // Keep the original prefix/suffix, replacing only the numeric glyph.
                let x = [0, 1070, 970, 876][count];
                let to = if builder {
                    Rect::new(52, 2, 15, 20)
                } else {
                    Rect::new(67, 11, 15, 20)
                };
                paste(&home, Rect::new(x, 11, 15, 20), &mut card, to);
            }
            cards.push(card);
        }
        let calibration = Calibration::load(&root().join("config.json")).unwrap();
        let anchors = calibration
            .screens
            .iter()
            .map(|(screen, profile)| {
                let name = serde_json::to_value(screen)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned();
                let regions = profile
                    .anchors
                    .iter()
                    .chain(profile.alt_anchors.iter().flatten())
                    .map(|anchor| {
                        let coc_bot_core::calibration::Region(x, y, w, h) =
                            anchor.region().unwrap();
                        Rect::new(x as i32, y as i32, w as i32, h as i32)
                    })
                    .collect();
                (name, regions)
            })
            .collect();
        Self {
            texture: fixture(if builder {
                "camera-builder-start.png"
            } else {
                "camera-home-start.png"
            }),
            cards,
            range,
            scale: 1.,
            offset: [0.; 2],
            selected: false,
            remaining: 4,
            deployed: 0,
            selections: 0,
            camera_moves: 0,
            builder,
            anchors,
        }
    }

    fn render(&self, screen: &str, captured: &Mat) -> Mat {
        let mut image = Mat::default();
        let transform = Mat::from_slice_2d(&[
            [self.scale, 0., self.offset[0] + 640. * (1. - self.scale)],
            [0., self.scale, self.offset[1] + 360. * (1. - self.scale)],
        ])
        .unwrap();
        imgproc::warp_affine(
            &self.texture,
            &mut image,
            &transform,
            Size::new(1280, 720),
            imgproc::INTER_LINEAR,
            core::BORDER_CONSTANT,
            Scalar::all(0.),
        )
        .unwrap();
        for rect in [
            Rect::new(0, 0, 1280, 90),
            Rect::new(0, 0, 270, 210),
            Rect::new(980, 0, 300, 180),
            Rect::new(0, 580, 1280, 140),
        ] {
            paste(captured, rect, &mut image, rect);
        }
        Mat::roi_mut(&mut image, Rect::new(80, 585, 1120, 135))
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        if self.builder && matches!(screen, "builder_battle" | "builder_second_active") {
            let rect = Rect::new(1170, 522, 95, 36);
            paste(captured, rect, &mut image, rect);
        }
        let active_screen = if screen == "builder_second_active" {
            "builder_battle"
        } else {
            screen
        };
        if let Some(anchors) = self.anchors.get(active_screen) {
            for &rect in anchors {
                paste(captured, rect, &mut image, rect);
            }
        }
        let card = &self.cards[self.remaining];
        let y = if self.builder { 588 } else { 586 };
        paste(
            card,
            Rect::new(0, 0, card.cols(), card.rows()),
            &mut image,
            Rect::new(100, y, card.cols(), card.rows()),
        );
        if self.selected {
            imgproc::rectangle(
                &mut image,
                Rect::new(99, 623, 90, 85),
                Scalar::all(255.),
                2,
                imgproc::LINE_8,
                0,
            )
            .unwrap();
        }
        image
    }
}

impl State {
    fn image(&self) -> Mat {
        if matches!(
            self.screen.as_str(),
            "home_scout"
                | "home_battle"
                | "builder_scout"
                | "builder_battle"
                | "builder_stage2"
                | "builder_second_active"
        ) {
            self.world.render(&self.screen, &self.frames[&self.screen])
        } else {
            self.frames[&self.screen].clone()
        }
    }

    fn tap(&mut self, x: i32, y: i32) {
        if y >= 585 {
            assert!((95..=195).contains(&x), "Unexpected army selection {x},{y}");
            self.world.selected = true;
            self.world.selections += 1;
        } else {
            assert!(
                self.world.selected && self.world.remaining > 0,
                "Deployment without a selected available troop"
            );
            self.world.remaining -= 1;
            self.world.deployed += 1;
            self.screen = match self.screen.as_str() {
                "home_scout" => "home_battle",
                "builder_scout" => "builder_battle",
                "builder_stage2" => "builder_second_active",
                s => s,
            }
            .into();
        }
    }

    fn gesture(&mut self, invocation: &Invocation) {
        let mut slot = 0;
        let mut positions = [[0i32; 2]; 2];
        let mut first = None;
        let mut last = positions;
        let mut fingers = 0;
        for frame in &invocation.input {
            let bytes: Vec<_> = std::str::from_utf8(&frame.bytes)
                .unwrap()
                .split_whitespace()
                .flat_map(|event| {
                    assert_eq!(event.len(), 24 * 5);
                    event.as_bytes().chunks_exact(5).map(|byte| {
                        assert_eq!(&byte[..2], b"\\0");
                        u8::from_str_radix(std::str::from_utf8(&byte[2..]).unwrap(), 8).unwrap()
                    })
                })
                .collect();
            let mut released = false;
            for event in bytes.chunks_exact(24) {
                let kind = u16::from_le_bytes(event[16..18].try_into().unwrap());
                let code = u16::from_le_bytes(event[18..20].try_into().unwrap());
                let value = i32::from_le_bytes(event[20..24].try_into().unwrap());
                if kind != 3 {
                    continue;
                }
                match code {
                    47 => {
                        slot = value as usize;
                        assert!(slot < 2);
                    }
                    57 if value >= 0 => {
                        fingers = fingers.max(slot + 1);
                    }
                    57 => released = true,
                    53 => positions[slot][0] = value,
                    54 => positions[slot][1] = value,
                    _ => {}
                }
            }
            if released {
                if let Some(start) = first.take() {
                    if fingers == 2 {
                        let start: [[i32; 2]; 2] = start;
                        let gap = start[1][0] - start[0][0];
                        let end_gap = last[1][0] - last[0][0];
                        if gap != end_gap {
                            self.world.scale = if end_gap == 420 {
                                1.25
                            } else if end_gap > gap {
                                1.3
                            } else {
                                1.
                            };
                        } else {
                            for axis in 0..2 {
                                let extent = [1280., 720.][axis];
                                let limit = ((extent + self.world.range[axis]) * self.world.scale
                                    - extent)
                                    / 2.;
                                self.world.offset[axis] = (self.world.offset[axis]
                                    + f64::from(last[0][axis] - start[0][axis]))
                                .clamp(-limit, limit);
                            }
                        }
                        self.world.camera_moves += 1;
                    } else if start[0] == last[0] {
                        self.tap(last[0][0], last[0][1]);
                    } else {
                        assert!(start[0][1] >= 585, "Unexpected single-finger camera drag");
                    }
                }
                fingers = 0;
            } else {
                first.get_or_insert(positions);
                last = positions;
            }
        }
    }
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
            ("home_result", "english-numeric-home-defeat.png"),
            ("builder", "english-builder.png"),
            ("builder_menu", "english-builder_menu.png"),
            ("builder_scout", "english-builder_current.png"),
            ("builder_battle", "english-builder_battle.png"),
            ("builder_stage2", "english-builder_fourth_current.png"),
            (
                "builder_second_active",
                "battlefield-builder-second-active.png",
            ),
            ("builder_result", "english-numeric-builder-two-stars.png"),
        ];
        let frames = files
            .into_iter()
            .map(|(name, file)| (name.to_owned(), fixture(file)))
            .collect();
        Self(Arc::new(Mutex::new(State {
            screen: if builder { "builder" } else { "home" }.into(),
            frames,
            inputs: 0,
            world: World::new(builder),
        })))
    }
    fn image(&self) -> Mat {
        let state = self.0.lock().unwrap();
        state.image()
    }
    fn screen(&self, name: &str) {
        let mut state = self.0.lock().unwrap();
        state.screen = name.into();
        if name == "builder_stage2" {
            assert_eq!(state.world.remaining, 0);
            state.world.remaining = 4;
            state.world.selected = false;
            state.world.scale = 1.;
            state.world.offset = [0.; 2];
        }
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
            return Ok(framebuffer(&state.image()));
        }
        if args.contains("shell input tap") {
            state.inputs += 1;
            if matches!(
                state.screen.as_str(),
                "home_scout"
                    | "home_battle"
                    | "builder_scout"
                    | "builder_battle"
                    | "builder_stage2"
                    | "builder_second_active"
            ) {
                let coordinates: Vec<i32> = args
                    .split_whitespace()
                    .rev()
                    .take(2)
                    .map(|n| n.parse().unwrap())
                    .collect();
                state.tap(coordinates[1], coordinates[0]);
                return Ok(vec![]);
            }
            let next = match (state.screen.as_str(), args.as_str()) {
                ("home", a) if a.ends_with("85 620") => "home_menu",
                ("home_menu", a) if a.ends_with("218 530") => "home_army",
                ("home_army", a) if a.ends_with("1130 641") => "home_scout",
                ("home_result", a) if a.ends_with("640 617") => "home",
                ("builder", a) if a.ends_with("85 620") => "builder_menu",
                ("builder_menu", a) if a.ends_with("950 472") => "builder_scout",
                ("builder_result", a) if a.ends_with("640 607") => "builder",
                ("home", a) if a.contains("tap 1120 ") => "home",
                ("builder", a) if a.contains("tap 1120 ") => "builder",
                _ => panic!("Unexpected tap on {}: {args}", state.screen),
            };
            state.screen = next.into();
            return Ok(vec![]);
        }
        if args.contains("/dev/input/event") {
            state.inputs += 1;
            state.gesture(&invocation);
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
        let mut inventories = Vec::new();
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
            if let Some((screen, units)) = result.army {
                assert_eq!(units.len(), 1);
                assert_eq!(units[0].kind, army::recipe::UnitKind::Troop);
                assert_eq!(units[0].count, Some(4));
                inventories.push(screen);
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
        if village == Village::Home {
            assert_eq!(records[0].loot.gold, Some(1871));
            assert_eq!(records[0].loot.elixir, Some(0));
            assert_eq!(records[0].damage, Some(5));
            assert_eq!(records[0].stars, None);
        } else {
            assert_eq!(records[0].loot.gold, Some(75_000));
            assert_eq!(records[0].loot.elixir, None);
            assert_eq!(records[0].damage, Some(72));
            assert_eq!(records[0].stars, Some(2));
        }
        assert_eq!(records[0].loot.dark, None);
        let state = fake.0.lock().unwrap();
        assert!(state.inputs > 10);
        assert!(state.world.camera_moves > 10);
        assert_eq!(state.world.remaining, 0);
        assert_eq!(
            state.world.selections,
            if village == Village::Builder { 2 } else { 1 }
        );
        assert_eq!(
            state.world.deployed,
            if village == Village::Builder { 8 } else { 4 }
        );
        assert_eq!(
            inventories,
            if village == Village::Builder {
                vec![Screen::BuilderScout, Screen::BuilderStage2]
            } else {
                vec![Screen::HomeScout]
            }
        );
        assert_eq!(stage2, village == Village::Builder);
    }
}

#[test]
fn simulated_army_preserves_captured_portraits_and_readable_quantities() {
    let profile = Profile::load(&root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    for builder in [false, true] {
        let fake = Fake::new(builder);
        fake.screen(if builder {
            "builder_scout"
        } else {
            "home_scout"
        });
        let source = if builder {
            Screen::BuilderScout
        } else {
            Screen::HomeScout
        };
        let bar = profile.screen(source).unwrap().army_bar.unwrap();
        let cards = army::cards(&fake.image(), bar, builder, &digits).unwrap();
        assert_eq!(cards.len(), 1, "{builder}: {cards:?}");
        assert_eq!(cards[0].count, Some(4), "{builder}: {cards:?}");
        let bound = army::recipe::BoundCard::new(&cards[0], &fake.image(), builder).unwrap();
        for count in (0..=4).rev() {
            for selected in [false, true] {
                let mut state = fake.0.lock().unwrap();
                state.world.remaining = count;
                state.world.selected = selected;
                let image = state.image();
                assert_eq!(
                    army::selected(&image, cards[0].slot).unwrap(),
                    selected,
                    "{builder}/{count}"
                );
                assert_eq!(
                    bound.remaining(&image, &digits).unwrap(),
                    Some(count as u32),
                    "{builder}/{count}/{selected}"
                );
            }
        }
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
fn deployment_settings_validate_village_roles_without_legacy_points() {
    use coc_bot_core::{
        gameplay::Strategy,
        vision::army::{
            Layout,
            recipe::{Recipe, Step, UnitKind},
        },
    };
    let resolution = Resolution(1280, 720);
    let mut options = Options {
        strategy: Strategy::Ring,
        earthquake: true,
        ..Options::default()
    };
    options.layouts.insert(Screen::HomeScout, Layout::default());
    assert!(options.validate(resolution).is_ok());
    let mut step: Step = serde_json::from_value(serde_json::json!({
        "slot": 1, "label": "Troop", "kind": "troop", "point": [0, 500], "count": 1,
        "delay_ms": 0, "interval_ms": 100, "ability": true
    }))
    .unwrap();
    for (screen, kind, ability, valid) in [
        (Screen::HomeScout, UnitKind::Troop, true, false),
        (Screen::HomeScout, UnitKind::Hero, true, true),
        (Screen::HomeScout, UnitKind::Spell, false, true),
        (Screen::BuilderScout, UnitKind::Troop, true, true),
        (Screen::BuilderScout, UnitKind::Spell, false, false),
        (Screen::BuilderStage2, UnitKind::Siege, false, false),
    ] {
        step.kind = kind;
        step.ability = ability;
        options.layouts.clear();
        options.layouts.insert(
            screen,
            Layout {
                recipe: Some(Recipe {
                    steps: vec![step.clone()],
                }),
            },
        );
        assert_eq!(
            options.validate(resolution).is_ok(),
            valid,
            "{screen:?}/{kind:?}"
        );
    }
}

#[test]
fn cart_target_validation_tracks_the_selected_villages() {
    let resolution = coc_bot_core::calibration::Resolution(1280, 720);
    let mut options = Options {
        until_full: true,
        cart_goal: true,
        collect_cart: false,
        ..Options::default()
    };
    assert!(options.validate(resolution).is_ok());
    options.villages.push(Village::Builder);
    assert!(options.validate(resolution).is_err());
    options.collect_cart = true;
    assert!(options.validate(resolution).is_ok());
    options.targets.clear();
    assert!(options.validate(resolution).is_err());
    options.villages = vec![Village::Builder];
    assert!(options.validate(resolution).is_ok());
    options.cart_goal = false;
    assert!(options.validate(resolution).is_err());
}
