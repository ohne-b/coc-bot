use coc_bot_core::{
    calibration::Resolution,
    gameplay::results::{self, ResultLayout},
    vision::{Digits, read_image},
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

// Exercise layout selection with captured readable loot while result-specific captures are collected.
fn layout() -> ResultLayout {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("config.json")).unwrap()).unwrap();
    let scout = &config["screens"]["home_scout"];
    let guards: Vec<_> = scout["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|anchor| {
            serde_json::json!({
                "file": anchor["file"], "region": anchor["roi"], "max_score": 0.02
            })
        })
        .collect();
    let mut layout: ResultLayout = serde_json::from_value(serde_json::json!({ "name":"captured-loot-parser", "guards": guards, "loot": scout["loot"] })).unwrap();
    layout.load(&root(), Resolution(1280, 720)).unwrap();
    layout
}

#[test]
fn guarded_numeric_reader_rejects_ambiguous_or_wrong_layouts_and_preserves_unknowns() {
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    let image = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/english-home_scout.png"),
    )
    .unwrap();
    let mut variant = layout();
    let read = results::read(
        std::slice::from_ref(&variant),
        &image,
        &digits,
        coc_bot_core::telemetry::Village::Home,
    )
    .unwrap();
    assert_eq!(
        (read.loot.gold, read.loot.elixir, read.loot.dark),
        (Some(1_557_102), Some(1_646_314), Some(10_150))
    );
    assert!(read.damage.is_none() && read.stars.is_none());
    variant.loot.remove("dark");
    assert!(
        results::read(
            std::slice::from_ref(&variant),
            &image,
            &digits,
            coc_bot_core::telemetry::Village::Home
        )
        .unwrap()
        .loot
        .dark
        .is_none()
    );
    variant
        .bonus
        .insert("elixir".into(), variant.loot["elixir"]);
    assert!(
        results::read(
            std::slice::from_ref(&variant),
            &image,
            &digits,
            coc_bot_core::telemetry::Village::Home
        )
        .unwrap()
        .loot
        .elixir
        .is_none()
    );
    variant.bonus_included = Some(true);
    assert_eq!(
        results::read(
            std::slice::from_ref(&variant),
            &image,
            &digits,
            coc_bot_core::telemetry::Village::Home
        )
        .unwrap()
        .loot
        .elixir,
        Some(1_646_314)
    );
    variant.bonus_included = Some(false);
    assert_eq!(
        results::read(
            std::slice::from_ref(&variant),
            &image,
            &digits,
            coc_bot_core::telemetry::Village::Home
        )
        .unwrap()
        .loot
        .elixir,
        Some(3_292_628)
    );
    assert!(
        results::read(
            &[variant, layout()],
            &image,
            &digits,
            coc_bot_core::telemetry::Village::Home
        )
        .unwrap()
        .loot
        .elixir
        .is_none()
    );
    let wrong =
        read_image(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/english-home.png"))
            .unwrap();
    assert!(
        results::read(
            &[layout()],
            &wrong,
            &digits,
            coc_bot_core::telemetry::Village::Home
        )
        .unwrap()
        .loot
        .elixir
        .is_none()
    );
}

#[test]
fn result_guards_must_be_independent_and_inside_the_frame() {
    let mut variant = layout();
    variant.guards[1].region = variant.guards[0].region;
    assert!(variant.load(&root(), Resolution(1280, 720)).is_err());
    variant.guards[1].region = Some(coc_bot_core::calibration::Region(u32::MAX, 0, 10, 10));
    assert!(variant.load(&root(), Resolution(1280, 720)).is_err());
}

#[test]
fn builder_two_stage_results_keep_full_damage_and_visible_stars_without_guessing() {
    use coc_bot_core::{
        calibration::{Calibration, Region, Screen},
        gameplay::profile::Profile,
        telemetry::{Attack, Village},
        vision::{Detection, Detector, crop},
    };
    use opencv::{
        core::{self, Mat, Rect, Scalar},
        prelude::*,
    };
    let profile = Profile::load(&root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    let detector = Detector::load(
        &Calibration::load(&root().join("config.json")).unwrap(),
        &root(),
    )
    .unwrap();
    let layouts = &profile
        .screen(Screen::BuilderResult)
        .unwrap()
        .result_readings;
    for phase in ["gold", "silver"] {
        let image = read_image(&Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/english-numeric-builder-200-{phase}.png"
        )))
        .unwrap();
        assert_eq!(
            detector.detect(&image).unwrap(),
            Detection::Known(Screen::BuilderResult)
        );
        let read = results::read(layouts, &image, &digits, Village::Builder).unwrap();
        assert_eq!(
            (read.damage, read.stars, read.loot.gold),
            (Some(200), Some(3), Some(170000))
        );
        assert_eq!((read.loot.elixir, read.loot.dark), (None, None));
        assert!(
            digits
                .percent_up_to(&crop(&image, Region(584, 290, 119, 54)).unwrap(), 200)
                .is_err(),
            "The old clipped crop cannot become0%"
        );
        assert!(
            digits
                .percent(&crop(&image, Region(550, 290, 180, 54)).unwrap())
                .is_err(),
            "Home percentages remain bounded to100"
        );
        let mut attack = Attack {
            ordinal: 1,
            village: Village::Builder,
            completed_at_ms: 1,
            loot: read.loot,
            displayed_loot: read.displayed_loot,
            damage: read.damage,
            stars: read.stars,
        };
        attack.validate().unwrap();
        attack.damage = Some(201);
        assert!(attack.validate().is_err());
        for (rect, damage_missing) in [
            (Rect::new(550, 290, 180, 54), true),
            (Rect::new(435, 172, 100, 80), false),
        ] {
            let mut hidden = image.try_clone().unwrap();
            Mat::roi_mut(&mut hidden, rect)
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
            // A single surviving bright pixel cannot turn a redacted star into a genuine zero.
            if !damage_missing {
                *hidden
                    .at_2d_mut::<core::Vec3b>(rect.y + 10, rect.x + 10)
                    .unwrap() = core::Vec3b::all(255);
            }
            let read = results::read(layouts, &hidden, &digits, Village::Builder).unwrap();
            assert_eq!(read.loot.gold, Some(170000));
            if damage_missing {
                assert_eq!(read.damage, None);
            } else {
                assert_eq!(read.stars, None);
            }
        }
        for alteration in ["dim", "dark", "black"] {
            let mut obscured = image.try_clone().unwrap();
            for rect in [
                Rect::new(435, 172, 100, 80),
                Rect::new(592, 125, 100, 80),
                Rect::new(746, 172, 100, 80),
            ] {
                if alteration == "dim" {
                    let mut patch = Mat::default();
                    Mat::roi(&image, rect)
                        .unwrap()
                        .convert_to(&mut patch, -1, 0.5, 0.)
                        .unwrap();
                    patch
                        .copy_to(&mut Mat::roi_mut(&mut obscured, rect).unwrap())
                        .unwrap();
                } else {
                    Mat::roi_mut(&mut obscured, rect)
                        .unwrap()
                        .set_to(
                            &Scalar::all(if alteration == "dark" { 32. } else { 0. }),
                            &core::no_array(),
                        )
                        .unwrap();
                }
            }
            assert_eq!(
                detector.detect(&obscured).unwrap(),
                Detection::Known(Screen::BuilderResult)
            );
            let read = results::read(layouts, &obscured, &digits, Village::Builder).unwrap();
            assert_eq!((read.loot.gold, read.damage), (Some(170000), Some(200)));
            assert_eq!(
                read.stars, None,
                "{phase} {alteration} cannot prove an empty star"
            );
        }
        for rect in [Rect::new(776, 385, 82, 24), Rect::new(591, 582, 104, 49)] {
            let mut missing = image.try_clone().unwrap();
            Mat::roi_mut(&mut missing, rect)
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
            assert_ne!(
                detector.detect(&missing).unwrap(),
                Detection::Known(Screen::BuilderResult)
            );
            let read = results::read(layouts, &missing, &digits, Village::Builder).unwrap();
            assert!(read.loot.gold.is_none() && read.damage.is_none() && read.stars.is_none());
        }
        let mut dimmed = Mat::default();
        image.convert_to(&mut dimmed, -1, 0.5, 0.).unwrap();
        assert_eq!(detector.detect(&dimmed).unwrap(), Detection::Unknown);
    }
}

#[test]
fn empty_result_stars_require_their_calibrated_slot_appearance() {
    use coc_bot_core::{calibration::Screen, gameplay::profile::Profile, telemetry::Village};
    let mut profile = Profile::load(&root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    for (file, count) in [
        ("english-numeric-builder-zero.png", 0),
        ("english-numeric-builder-two-stars.png", 2),
    ] {
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap();
        let layouts = &profile
            .screen(Screen::BuilderResult)
            .unwrap()
            .result_readings;
        assert_eq!(
            results::read(layouts, &image, &digits, Village::Builder)
                .unwrap()
                .stars,
            Some(count)
        );
    }
    profile
        .screens
        .get_mut(&Screen::BuilderResult)
        .unwrap()
        .result_readings[0]
        .empty_stars
        .clear();
    let image = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/english-numeric-builder-zero.png"),
    )
    .unwrap();
    assert_eq!(
        results::read(
            &profile
                .screen(Screen::BuilderResult)
                .unwrap()
                .result_readings,
            &image,
            &digits,
            Village::Builder
        )
        .unwrap()
        .stars,
        None
    );
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("config.json")).unwrap()).unwrap();
    for field in ["region", "max_score", "scales"] {
        let mut value = config["screens"]["builder_result"]["result_readings"][0].clone();
        value["empty_stars"][0][field] = match field {
            "region" => serde_json::json!([0, 0, 1280, 720]),
            "max_score" => serde_json::json!(0.03),
            _ => serde_json::json!([0.9, 1.0]),
        };
        let mut layout: ResultLayout = serde_json::from_value(value).unwrap();
        assert!(layout.load(&root(), Resolution(1280, 720)).is_err());
    }
}
