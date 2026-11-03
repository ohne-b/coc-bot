use std::path::{Path, PathBuf};

use coc_bot_core::{
    calibration::{Calibration, Region, Screen},
    gameplay::{profile::Profile, results},
    telemetry::Village,
    vision::{Detection, Detector, Digits, army, crop, green_text, read_image},
};
use opencv::{
    core::{self, Mat, Scalar, Size},
    imgproc,
};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/python")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("fixtures/english-readings.json")).unwrap())
        .unwrap()
}

#[test]
fn captured_numeric_ground_truth_matches_native_readers() {
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    let detector = Detector::load(
        &Calibration::load(&root().join("config.json")).unwrap(),
        &root(),
    )
    .unwrap();
    let mut failures = vec![];
    for case in manifest()["fixtures"].as_array().unwrap() {
        let image =
            read_image(&root().join("fixtures").join(case["file"].as_str().unwrap())).unwrap();
        let expected = case["screen"]
            .as_str()
            .map(|s| serde_json::from_value::<Screen>(Value::String(s.into())).unwrap());
        assert_eq!(
            detector.detect(&image).unwrap(),
            expected.map_or(Detection::Unknown, Detection::Known),
            "{}",
            case["id"]
        );
        for (name, spec) in case["readings"].as_object().unwrap() {
            if spec["status"] == "unavailable" || spec["reader"] == "silver_stars" {
                continue; // Silver Home stars have no calibrated native reader yet.
            }
            let region: Region = serde_json::from_value(spec["roi"].clone()).unwrap();
            let mut patch = crop(&image, region).unwrap();
            let reader = spec["reader"].as_str().unwrap();
            if reader.starts_with("green_") {
                patch = green_text(&patch).unwrap();
            }
            let actual = match reader {
                "percent" | "green_percent" => digits.percent_up_to(&patch, 200).map(u64::from),
                "trailing_number" => digits.trailing(&patch, 0.65, 110),
                "wall_row_price" => {
                    let mut scaled = Mat::default();
                    imgproc::resize(
                        &patch,
                        &mut scaled,
                        Size::default(),
                        2.0,
                        2.0,
                        imgproc::INTER_CUBIC,
                    )
                    .unwrap();
                    digits.trailing(&scaled, 0.65, 160)
                }
                "gold_stars" => army::stars(
                    &image,
                    &serde_json::from_value::<Vec<Region>>(spec["slots"].clone()).unwrap(),
                )
                .map(u64::from),
                "number" | "green_number" => digits.number(&patch, 0.75),
                _ => panic!("Unsupported annotation reader {reader}"),
            };
            let expected = spec["value"].as_u64();
            if actual.as_ref().ok().copied() != expected {
                failures.push(format!(
                    "{} {name}: expected {expected:?}, got {actual:?}",
                    case["id"]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn real_results_select_one_layout_and_keep_unavailable_earnings_unknown() {
    let profile = Profile::load(&root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    for case in manifest()["fixtures"].as_array().unwrap() {
        let image =
            read_image(&root().join("fixtures").join(case["file"].as_str().unwrap())).unwrap();
        for (screen, village) in [
            (Screen::HomeResult, Village::Home),
            (Screen::BuilderResult, Village::Builder),
        ] {
            let result = results::read(
                &profile.screen(screen).unwrap().result_readings,
                &image,
                &digits,
                village,
            )
            .unwrap();
            let matching = case["screen"] == serde_json::to_value(screen).unwrap();
            let field = |key: &str| {
                if matching {
                    case["readings"][key]["value"].as_u64()
                } else {
                    None
                }
            };
            let uncertain_bonus = case["id"] == "home-victory";
            assert_eq!(
                result.loot.gold.map(u64::from),
                if uncertain_bonus {
                    None
                } else {
                    field("loot.gold")
                },
                "{} {screen:?}",
                case["id"]
            );
            assert_eq!(
                result.loot.elixir.map(u64::from),
                if uncertain_bonus {
                    None
                } else {
                    field("loot.elixir")
                }
            );
            assert_eq!(
                result.loot.dark.map(u64::from),
                if uncertain_bonus {
                    None
                } else {
                    field("loot.dark")
                }
            );
            assert_eq!(
                result.damage.map(u64::from),
                field("damage_percent"),
                "{}",
                case["id"]
            );
            assert_eq!(
                result.stars.map(u64::from),
                if village == Village::Builder {
                    field("stars")
                } else {
                    None
                }
            );
        }
    }
}

#[test]
fn a_tall_component_inside_the_hud_cannot_silently_drop_leading_digits() {
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    let image = read_image(&root().join("fixtures/english-numeric-home-gold-tooltip.png")).unwrap();
    let patch = crop(&image, Region(1040, 26, 178, 26)).unwrap();
    assert_eq!(digits.trailing(&patch, 0.65, 110).unwrap(), 7_099_808);
    let mut extended = Mat::default();
    core::copy_make_border(
        &patch,
        &mut extended,
        0,
        2,
        0,
        0,
        core::BORDER_CONSTANT,
        Scalar::all(0.0),
    )
    .unwrap();
    assert!(digits.trailing(&extended, 0.65, 110).is_err());
}
