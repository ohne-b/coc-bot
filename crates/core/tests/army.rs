use coc_bot_core::{
    calibration::{Region, Resolution},
    device::Point,
    vision::{
        Digits,
        army::{self, Deployment, Kind, Layout},
        read_image,
    },
};
use opencv::{
    core::{self, Mat, Rect, Scalar},
    prelude::*,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
fn digits() -> Digits {
    Digits::load(&root(), Path::new("calibration/digits")).unwrap()
}
fn fixture(name: &str) -> Mat {
    read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}
fn config() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("config.json")).unwrap()).unwrap()
}

#[test]
fn captured_builder_camps_and_survivors_keep_their_deployment_roles() {
    let digits = digits();
    for (file, expected) in [
        (
            "builder-stage1-bar.png",
            vec![
                Kind::Hero,
                Kind::Counted,
                Kind::Counted,
                Kind::Counted,
                Kind::Counted,
                Kind::Counted,
                Kind::Counted,
            ],
        ),
        ("builder-stage2-bar.png", vec![Kind::Counted; 8]),
        (
            "builder-barbarian-bar.png",
            vec![
                Kind::Hero,
                Kind::Sized,
                Kind::Sized,
                Kind::Sized,
                Kind::Sized,
                Kind::Sized,
                Kind::Sized,
            ],
        ),
    ] {
        let image = fixture(file);
        let bar = Region(0, 0, image.cols() as u32, image.rows() as u32);
        let cards = army::cards(&image, bar, true, &digits).unwrap();
        assert_eq!(
            cards.iter().map(|c| c.kind).collect::<Vec<_>>(),
            expected,
            "{file}"
        );
        let mut layout: Layout =
            serde_json::from_value(config()["screens"]["builder_scout"]["auto"].clone()).unwrap();
        layout.bar = bar;
        let plan = army::plan(&cards, &layout, Point(100, 100), false, false, false).unwrap();
        let count = if file.contains("barbarian") { 4 } else { 1 };
        for (index, card) in cards.iter().filter(|c| c.kind != Kind::Hero).enumerate() {
            assert!(
                plan.contains(&Deployment::Drop {
                    slot: card.slot,
                    point: layout.points[index % layout.points.len()],
                    count
                }),
                "{file}: {plan:?}"
            );
        }
        assert!(matches!(
            plan.last(),
            Some(Deployment::Ability { wait_ms: 5000, .. })
        ));
        let stacked = army::plan(&cards, &layout, Point(100, 100), false, false, true).unwrap();
        assert!(
            stacked
                .iter()
                .all(|s| !matches!(s,Deployment::Drop{point,..} if *point!=Point(100,100)))
        );
    }
}

#[test]
fn captured_counts_drive_ring_and_earthquake_deployment() {
    let mut image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let bar = fixture("battle-bar-valkyrie-33-spells-11.png");
    bar.copy_to(&mut Mat::roi_mut(&mut image, Rect::new(0, 586, bar.cols(), bar.rows())).unwrap())
        .unwrap();
    let layout: Layout =
        serde_json::from_value(config()["screens"]["home_scout"]["auto"].clone()).unwrap();
    layout.validate(Resolution(1280, 720)).unwrap();
    let cards = army::cards(&image, layout.bar, false, &digits()).unwrap();
    assert_eq!(
        cards
            .iter()
            .filter(|c| c.kind == Kind::Counted)
            .map(|c| c.count)
            .collect::<Vec<_>>(),
        [Some(33), Some(11)]
    );
    let plan = army::plan(&cards, &layout, Point(100, 300), true, true, false).unwrap();
    assert!(matches!(
        plan.last(),
        Some(Deployment::Spread {
            count: 33,
            spaced: true,
            ..
        })
    ));
    let spells = plan
        .iter()
        .find_map(|step| match step {
            Deployment::Spread {
                points,
                count: 11,
                spaced: false,
                ..
            } => Some(points),
            _ => None,
        })
        .unwrap();
    assert_eq!(&spells[..4], &[layout.quake[0]; 4]);
    assert_eq!(&spells[4..8], &[layout.quake[1]; 4]);
}

#[test]
fn captured_hero_meters_and_stars_match_baseline() {
    for n in 1..=3 {
        assert_eq!(
            army::hero_ready(
                &fixture(&format!("builder-hero-meter-{n}.png")),
                Point(129, 128)
            )
            .unwrap(),
            n == 3
        );
    }
    let regions: Vec<Region> =
        serde_json::from_value(config()["screens"]["builder_battle"]["stars"].clone()).unwrap();
    let regions: Vec<_> = regions
        .into_iter()
        .map(|Region(x, y, w, h)| Region(x - 1060, y - 495, w, h))
        .collect();
    for n in 0..=3 {
        assert_eq!(
            army::stars(&fixture(&format!("builder-stars-{n}.png")), &regions).unwrap(),
            n
        );
    }
    assert!(army::hero_ready(&fixture("builder-hero-meter-1.png"), Point(0, 0)).is_err());
    assert!(army::stars(&fixture("builder-stars-0.png"), &[Region(9999, 0, 10, 10)]).is_err());
    let mut redacted = fixture("builder-stars-2.png");
    let Region(x, y, w, h) = regions[0];
    Mat::roi_mut(
        &mut redacted,
        Rect::new(x as i32, y as i32, w as i32, h as i32),
    )
    .unwrap()
    .set_to(&Scalar::all(0.0), &core::no_array())
    .unwrap();
    assert!(army::stars(&redacted, &regions).is_err());
}

#[test]
fn malformed_and_empty_bars_never_produce_a_deployment() {
    let image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let mut layout: Layout =
        serde_json::from_value(config()["screens"]["home_scout"]["auto"].clone()).unwrap();
    assert!(army::cards(&image, Region(0, 715, 100, 100), false, &digits()).is_err());
    assert!(
        army::cards(&image, layout.bar, false, &digits())
            .unwrap()
            .is_empty()
    );
    assert!(army::plan(&[], &layout, Point(100, 100), false, false, false).is_err());
    layout.ring = vec![Point(1280, 100)];
    assert!(layout.validate(Resolution(1280, 720)).is_err());
}
