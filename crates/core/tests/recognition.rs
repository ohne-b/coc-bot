use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use coc_bot_core::{
    calibration::{Calibration, Region, Resolution, Screen, asset_path},
    vision::{Detection, Detector, Digits, crop, decode_frame, read_image},
};
use opencv::{
    core::{self, Mat, Scalar},
    prelude::*,
};
use serde::Deserialize;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
fn profile() -> Calibration {
    Calibration::load(&root().join("config.json")).unwrap()
}

#[test]
fn english_captured_screens_match_baseline() {
    #[derive(Deserialize)]
    struct Fixture {
        file: String,
        state: Option<Screen>,
    }
    let manifest: BTreeMap<String, Fixture> = serde_json::from_slice(
        &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/english.json"))
            .unwrap(),
    )
    .unwrap();
    let detector = Detector::load(&profile(), &root()).unwrap();
    for (name, fixture) in manifest {
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture.file),
        )
        .unwrap();
        assert_eq!(
            detector.detect(&image).unwrap(),
            fixture.state.map_or(Detection::Unknown, Detection::Known),
            "fixture {name}"
        );
    }
}

#[test]
fn ldplayer_top_up_needs_all_three_guards_and_retains_no_player_data() {
    let calibration = profile();
    let detector = Detector::load(&calibration, &root()).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let frame = read_image(&fixtures.join("english-ldplayer-top-up.png")).unwrap();
    assert_eq!(
        detector.detect(&frame).unwrap(),
        Detection::Known(Screen::LdplayerTopUp)
    );
    for anchor in &calibration.screens[&Screen::LdplayerTopUp].anchors {
        let Region(x, y, w, h) = anchor.region().unwrap();
        let mut missing = frame.clone();
        Mat::roi_mut(
            &mut missing,
            core::Rect::new(x as i32, y as i32, w as i32, h as i32),
        )
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
        assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
    }
    let mut different_offer = frame.clone();
    for region in [
        core::Rect::new(550, 150, 180, 28),
        core::Rect::new(310, 215, 660, 335),
    ] {
        Mat::roi_mut(&mut different_offer, region)
            .unwrap()
            .set_to(&Scalar::all(255.), &core::no_array())
            .unwrap();
    }
    assert_eq!(
        detector.detect(&different_offer).unwrap(),
        Detection::Known(Screen::LdplayerTopUp)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("english.json")).unwrap()).unwrap();
    let regions: Vec<Region> =
        serde_json::from_value(manifest["ldplayer_top_up"]["retained_regions"].clone()).unwrap();
    let mut private = frame.clone();
    for Region(x, y, w, h) in regions {
        Mat::roi_mut(
            &mut private,
            core::Rect::new(x as i32, y as i32, w as i32, h as i32),
        )
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
    }
    assert!(private.data_bytes().unwrap().iter().all(|v| *v == 0));
}

#[test]
fn rejects_black_frames_wrong_dimensions_and_ambiguous_screens() {
    let mut calibration = profile();
    let detector = Detector::load(&calibration, &root()).unwrap();
    for file in ["english-home.png", "english-builder.png"] {
        let frame = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap();
        let mut dimmed = Mat::default();
        frame.convert_to(&mut dimmed, -1, 0.5, 0.0).unwrap();
        assert_eq!(detector.detect(&dimmed).unwrap(), Detection::Unknown);
    }
    let black =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    assert_eq!(detector.detect(&black).unwrap(), Detection::Unknown);
    assert!(detector.detect(&Mat::default()).is_err());
    let mut duplicate = calibration.screens[&Screen::Home].clone();
    duplicate.overrides.clear();
    calibration.screens.insert(Screen::Builder, duplicate);
    let ambiguous = Detector::load(&calibration, &root()).unwrap();
    let home =
        read_image(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/english-home.png"))
            .unwrap();
    assert_eq!(
        ambiguous.detect(&home).unwrap(),
        Detection::Ambiguous(vec![Screen::Home, Screen::Builder])
    );
}

#[test]
fn pending_english_templates_never_authorize_input() {
    let mut calibration = profile();
    calibration.screens.insert(
        Screen::Disconnected,
        calibration.screens[&Screen::Home].clone(),
    );
    assert!(Detector::load(&calibration, &root()).is_err());
}

#[test]
fn home_storage_tooltips_keep_all_guards_during_the_battle_button_animation() {
    let detector = Detector::load(&profile(), &root()).unwrap();
    let frame = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/english-home-gold-tooltip-flame.png"),
    )
    .unwrap();
    assert_eq!(
        detector.detect(&frame).unwrap(),
        Detection::Known(Screen::Home)
    );
    for region in [
        core::Rect::new(39, 669, 102, 37),
        core::Rect::new(85, 599, 37, 26),
        core::Rect::new(1060, 77, 48, 23),
        core::Rect::new(32, 165, 55, 54),
    ] {
        let mut missing = frame.clone();
        Mat::roi_mut(&mut missing, region)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
    }
    let mut dimmed = Mat::default();
    frame.convert_to(&mut dimmed, -1, 0.5, 0.).unwrap();
    assert_eq!(detector.detect(&dimmed).unwrap(), Detection::Unknown);
}

#[test]
fn home_battle_uses_surrender_and_damage_when_resource_icons_are_obscured() {
    let detector = Detector::load(&profile(), &root()).unwrap();
    let frame = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/battlefield-home-dropped.png"),
    )
    .unwrap();
    assert_eq!(
        detector.detect(&frame).unwrap(),
        Detection::Known(Screen::HomeBattle)
    );
    for region in [
        core::Rect::new(16, 515, 150, 45),
        core::Rect::new(1063, 467, 205, 95),
    ] {
        let mut missing = frame.clone();
        Mat::roi_mut(&mut missing, region)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
    }
}

#[test]
fn home_countdown_requires_end_battle_and_damage_before_the_first_placement() {
    let detector = Detector::load(&profile(), &root()).unwrap();
    let frame = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/battlefield-home-countdown.png"),
    )
    .unwrap();
    assert_eq!(
        detector.detect(&frame).unwrap(),
        Detection::Known(Screen::HomeBattle)
    );
    for region in [
        core::Rect::new(16, 515, 150, 45),
        core::Rect::new(1063, 467, 205, 95),
    ] {
        let mut missing = frame.clone();
        Mat::roi_mut(&mut missing, region)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
    }
}

#[test]
fn a_siege_warning_can_cover_damage_without_hiding_both_battle_controls() {
    let detector = Detector::load(&profile(), &root()).unwrap();
    let frame = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/battlefield-home-siege-warning.png"),
    )
    .unwrap();
    assert_eq!(
        detector.detect(&frame).unwrap(),
        Detection::Known(Screen::HomeBattle)
    );
    for region in [
        core::Rect::new(16, 515, 150, 45),
        core::Rect::new(1160, 330, 120, 160),
    ] {
        let mut missing = frame.clone();
        Mat::roi_mut(&mut missing, region)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
    }
    let mut covered = Mat::default();
    frame.convert_to(&mut covered, -1, 0.5, 0.).unwrap();
    assert_eq!(detector.detect(&covered).unwrap(), Detection::Unknown);
}

#[test]
fn profiles_validate_and_bad_regions_paths_and_tolerances_fail_closed() {
    let calibration = profile();
    Detector::load(&calibration, &root()).unwrap();
    let invalid_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(invalid_file.path(), b"{}").unwrap();
    assert!(Calibration::load(invalid_file.path()).is_err());
    for region in [
        Region(0, 0, 0, 1),
        Region(u32::MAX, 0, 1, 1),
        Region(1279, 719, 2, 2),
    ] {
        assert!(region.validate(Resolution(1280, 720)).is_err());
    }
    for path in [
        "../README.md",
        "C:/Windows/win.ini",
        "calibration/../README.md",
        "",
    ] {
        assert!(asset_path(&root(), Path::new(path)).is_err());
    }
    let mut invalid = profile();
    invalid
        .screens
        .get_mut(&Screen::Home)
        .unwrap()
        .anchors
        .clear();
    assert!(invalid.validate().is_err());
    let mut invalid = profile();
    invalid.screens.get_mut(&Screen::Home).unwrap().anchors[0].max_score = f64::NAN;
    assert!(invalid.validate().is_err());
    let mut invalid = profile();
    invalid.calibration_pending_screens.insert(Screen::Home);
    assert!(invalid.validate().is_err());
    let mut invalid = profile();
    invalid.calibration_language = "id".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn recognition_requires_independent_correctly_sized_anchors() {
    let mut single = profile();
    single
        .screens
        .get_mut(&Screen::Home)
        .unwrap()
        .anchors
        .truncate(1);
    assert!(Detector::load(&single, &root()).is_err());

    let mut duplicate = profile();
    let anchors = &mut duplicate.screens.get_mut(&Screen::Home).unwrap().anchors;
    anchors[1] = anchors[0].clone();
    assert!(Detector::load(&duplicate, &root()).is_err());

    let mut overlapping = profile();
    let anchors = &mut overlapping.screens.get_mut(&Screen::Home).unwrap().anchors;
    let Region(x, y, _, _) = anchors[0].roi.unwrap();
    anchors[1].roi = Some(Region(x + 1, y, 26, 26));
    assert!(Detector::load(&overlapping, &root()).is_err());

    let mut alternative = profile();
    let screen = alternative.screens.get_mut(&Screen::Home).unwrap();
    screen.alt_anchors.push(vec![screen.anchors[0].clone()]);
    assert!(Detector::load(&alternative, &root()).is_err());

    let mut wider = profile();
    let anchor = &mut wider.screens.get_mut(&Screen::Home).unwrap().anchors[0];
    let Region(x, y, w, h) = anchor.roi.unwrap();
    anchor.roi = Some(Region(x, y, w + 1, h));
    assert!(Detector::load(&wider, &root()).is_err());
}

#[test]
fn raw_framebuffer_accepts_both_headers_without_swapping_channels() {
    for header in [12, 16] {
        let mut data: Vec<u8> = [2_u32, 1, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
        data.resize(header, 0);
        data.extend_from_slice(&[10, 20, 30, 255, 40, 50, 60, 255]);
        let image = decode_frame(&data, Resolution(2, 1)).unwrap();
        assert_eq!(image.data_bytes().unwrap(), [30, 20, 10, 60, 50, 40]);
        assert!(decode_frame(&data, Resolution(1, 1)).is_err());
        data[8] = 4;
        assert!(decode_frame(&data, Resolution(2, 1)).is_err());
        data[8] = 1;
        data.pop();
        assert!(decode_frame(&data, Resolution(2, 1)).is_err());
    }
    for length in 0..20 {
        assert!(decode_frame(&vec![0; length], Resolution(2, 1)).is_err());
    }
}

#[test]
fn real_capacity_gem_and_damage_digits_match_the_baseline() {
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("config.json")).unwrap()).unwrap();
    for resource in ["gold", "elixir"] {
        let region: Region =
            serde_json::from_value(config["capacity"]["home"][resource]["max"].clone()).unwrap();
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/english-tooltip_{resource}.png")),
        )
        .unwrap();
        assert_eq!(
            digits.number(&crop(&image, region).unwrap(), 0.65).unwrap(),
            26_000_000
        );
    }
    let gems = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gems-over-walls.png"),
    )
    .unwrap();
    assert_eq!(digits.trailing(&gems, 0.65, 70).unwrap(), 1096);
    let Region(x, y, w, h) =
        serde_json::from_value(config["screens"]["home_battle"]["damage"].clone()).unwrap();
    for (name, expected) in [("49", 49), ("94-two-stars", 94), ("100", 100)] {
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/battle-damage-{name}.png")),
        )
        .unwrap();
        assert_eq!(
            digits
                .percent(&crop(&image, Region(x - 1060, y - 495, w, h)).unwrap())
                .unwrap(),
            expected
        );
    }
    let mut image = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/battle-damage-94-two-stars.png"),
    )
    .unwrap();
    for y in 33..55 {
        for x in 158..175 {
            *image.at_2d_mut::<core::Vec3b>(y, x).unwrap() = core::Vec3b::all(255);
        }
    }
    assert!(
        digits
            .percent(&crop(&image, Region(x - 1060, y - 495, w, h)).unwrap())
            .is_err()
    );
    let blank = Mat::new_rows_cols_with_default(30, 120, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    assert!(digits.number(&blank, 0.65).is_err());
    assert!(digits.trailing(&blank, 0.65, 70).is_err());
    assert!(digits.fraction(&blank).is_err());
}

#[test]
fn blank_digit_templates_and_font_variants_are_rejected() {
    for name in ["3.png", "3-blank.png"] {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("digits");
        std::fs::create_dir(&folder).unwrap();
        for digit in 0..=9 {
            let name = format!("{digit}.png");
            std::fs::copy(
                root().join("calibration/digits").join(&name),
                folder.join(name),
            )
            .unwrap();
        }
        assert!(Digits::load(directory.path(), Path::new("digits")).is_ok());
        let blank =
            Mat::new_rows_cols_with_default(24, 20, core::CV_8UC1, Scalar::all(255.0)).unwrap();
        let mut bytes = core::Vector::<u8>::new();
        opencv::imgcodecs::imencode_def(".png", &blank, &mut bytes).unwrap();
        std::fs::write(folder.join(name), bytes.as_slice()).unwrap();
        assert!(Digits::load(directory.path(), Path::new("digits")).is_err());
    }
}

#[test]
fn builder_end_battle_requires_damage_and_does_not_claim_a_stage() {
    let detector = Detector::load(&profile(), &root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    for (file, damage) in [
        ("battlefield-builder-first-star.png", 54),
        ("battlefield-builder-first-complete.png", 100),
        ("battlefield-builder-second-active.png", 100),
    ] {
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap();
        assert_eq!(
            detector.detect(&image).unwrap(),
            Detection::Known(Screen::BuilderBattle)
        );
        assert_eq!(
            digits
                .percent_up_to(&crop(&image, Region(1170, 522, 95, 36)).unwrap(), 200)
                .unwrap(),
            damage
        );
        for rect in [
            core::Rect::new(20, 475, 150, 45),
            core::Rect::new(1060, 495, 210, 70),
        ] {
            let mut missing = image.try_clone().unwrap();
            Mat::roi_mut(&mut missing, rect)
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
            assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
        }
    }
}

#[test]
fn builder_stage_two_countdown_requires_the_starts_word_and_end_battle() {
    let detector = Detector::load(&profile(), &root()).unwrap();
    let image = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/battlefield-builder-countdown-cave.png"),
    )
    .unwrap();
    assert_eq!(
        detector.detect(&image).unwrap(),
        Detection::Known(Screen::BuilderStage2)
    );
    for rect in [
        core::Rect::new(20, 475, 150, 55),
        core::Rect::new(565, 10, 155, 62),
    ] {
        let mut missing = image.try_clone().unwrap();
        Mat::roi_mut(&mut missing, rect)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert_eq!(detector.detect(&missing).unwrap(), Detection::Unknown);
    }
    for file in [
        "english-builder_current.png",
        "battlefield-builder-first-star.png",
        "battlefield-builder-second-active.png",
    ] {
        let other = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap();
        assert!(
            !detector.matches(&other, Screen::BuilderStage2).unwrap(),
            "{file}"
        );
    }
}
