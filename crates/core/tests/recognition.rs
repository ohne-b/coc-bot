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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/python")
}
fn profile() -> Calibration {
    Calibration::load(&root().join("config.english.json")).unwrap()
}

#[test]
fn english_captured_screens_match_baseline() {
    #[derive(Deserialize)]
    struct Fixture {
        file: String,
        state: Option<Screen>,
    }
    let manifest: BTreeMap<String, Fixture> =
        serde_json::from_slice(&std::fs::read(root().join("fixtures/english.json")).unwrap())
            .unwrap();
    let detector = Detector::load(&profile(), &root()).unwrap();
    for (name, fixture) in manifest {
        let image = read_image(&root().join("fixtures").join(fixture.file)).unwrap();
        assert_eq!(
            detector.detect(&image).unwrap(),
            fixture.state.map_or(Detection::Unknown, Detection::Known),
            "fixture {name}"
        );
    }
}

#[test]
fn rejects_black_frames_wrong_dimensions_and_ambiguous_screens() {
    let mut calibration = profile();
    let detector = Detector::load(&calibration, &root()).unwrap();
    for file in ["english-home.png", "english-builder.png"] {
        let frame = read_image(&root().join("fixtures").join(file)).unwrap();
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
    let home = read_image(&root().join("fixtures/english-home.png")).unwrap();
    assert_eq!(
        ambiguous.detect(&home).unwrap(),
        Detection::Ambiguous(vec![Screen::Home, Screen::Builder])
    );
}

#[test]
fn pending_english_templates_never_authorize_input() {
    let mut calibration = profile();
    calibration
        .screens
        .get_mut(&Screen::Disconnected)
        .unwrap()
        .anchors = calibration.screens[&Screen::Home].anchors.clone();
    let detector = Detector::load(&calibration, &root()).unwrap();
    assert_eq!(
        detector
            .detect(&read_image(&root().join("fixtures/english-home.png")).unwrap())
            .unwrap(),
        Detection::Known(Screen::Home)
    );
}

#[test]
fn profiles_validate_and_bad_regions_paths_and_tolerances_fail_closed() {
    for file in ["config.json", "config.english.json", "config.legacy.json"] {
        let profile = Calibration::load(&root().join(file)).unwrap();
        Detector::load(&profile, &root()).unwrap();
    }
    assert!(Calibration::load(&root().join("config.example.json")).is_err());
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
        serde_json::from_slice(&std::fs::read(root().join("config.english.json")).unwrap())
            .unwrap();
    for resource in ["gold", "elixir"] {
        let region: Region =
            serde_json::from_value(config["capacity"]["home"][resource]["max"].clone()).unwrap();
        let image =
            read_image(&root().join(format!("fixtures/english-tooltip_{resource}.png"))).unwrap();
        assert_eq!(
            digits.number(&crop(&image, region).unwrap(), 0.65).unwrap(),
            26_000_000
        );
    }
    let gems = read_image(&root().join("fixtures/gems-over-walls.png")).unwrap();
    assert_eq!(digits.trailing(&gems, 0.65, 70).unwrap(), 1096);
    let Region(x, y, w, h) =
        serde_json::from_value(config["screens"]["home_battle"]["damage"].clone()).unwrap();
    for (name, expected) in [("49", 49), ("94-two-stars", 94), ("100", 100)] {
        let image = read_image(&root().join(format!("fixtures/battle-damage-{name}.png"))).unwrap();
        assert_eq!(
            digits
                .percent(&crop(&image, Region(x - 1060, y - 495, w, h)).unwrap())
                .unwrap(),
            expected
        );
    }
    let mut image = read_image(&root().join("fixtures/battle-damage-94-two-stars.png")).unwrap();
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
