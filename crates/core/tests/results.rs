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
