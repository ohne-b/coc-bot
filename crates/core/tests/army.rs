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
fn adjacent_troop_stacks_do_not_become_siege_units() {
    let mut image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let bar = fixture("battle-bar-dragons-15-balloons-4.png");
    bar.copy_to(&mut Mat::roi_mut(&mut image, Rect::new(0, 586, bar.cols(), bar.rows())).unwrap())
        .unwrap();
    let layout: Layout =
        serde_json::from_value(config()["screens"]["home_scout"]["auto"].clone()).unwrap();
    let cards = army::cards(&image, layout.bar, false, &digits()).unwrap();
    assert_eq!(cards.len(), 10);
    assert_eq!((cards[0].count, cards[1].count), (Some(15), Some(4)));
    assert_eq!((cards[0].group, cards[1].group), (0, 0));
    assert_eq!(cards[2].kind, Kind::Siege);
    let plan = army::plan(&cards, &layout, Point(125, 346), false, false, false).unwrap();
    for card in &cards[..2] {
        assert!(plan.contains(&Deployment::Hold {
            slot: card.slot,
            seconds: layout.hold
        }));
    }
    let ring = army::plan(&cards, &layout, Point(125, 346), true, false, false).unwrap();
    for card in &cards[..2] {
        assert!(ring.iter().any(|step| matches!(step, Deployment::Spread {slot, count, spaced: true, ..} if *slot == card.slot && Some(*count) == card.count)));
    }
    assert_eq!(
        ring.iter()
            .filter(|step| matches!(step, Deployment::Spread { spaced: false, .. }))
            .count(),
        3
    );
}

#[test]
fn explicit_actions_split_units_place_heroes_and_cast_only_requested_spells() {
    use army::recipe::{Recipe, Step, UnitKind, portrait};
    let mut image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let bar = fixture("battle-bar-dragons-15-balloons-4.png");
    bar.copy_to(&mut Mat::roi_mut(&mut image, Rect::new(0, 586, bar.cols(), bar.rows())).unwrap())
        .unwrap();
    let layout: Layout =
        serde_json::from_value(config()["screens"]["home_scout"]["auto"].clone()).unwrap();
    let cards = army::cards(&image, layout.bar, false, &digits()).unwrap();
    let action = |slot: usize, kind, count, point| Step {
        supports_ability: true,
        portrait: portrait(&image, cards[slot - 1].slot).unwrap(),
        label: String::new(),
        slot,
        kind,
        point,
        count,
        delay_ms: 0,
        interval_ms: 300,
        ability: false,
    };
    for (slot, kind) in [
        (1, UnitKind::Troop),
        (3, UnitKind::Siege),
        (4, UnitKind::Hero),
    ] {
        for point in [
            Point(0, 0),
            Point(0, 1000),
            Point(1000, 0),
            Point(1000, 1000),
        ] {
            let recipe = Recipe {
                steps: vec![action(slot, kind, 1, point)],
            };
            let plan = recipe.compile(&cards, &image, false).unwrap();
            assert!(matches!(
                plan.as_slice(),
                [Deployment::PreciseDrop { point: target, count: 1, .. }] if *target == point
            ));
        }
        for point in [Point(500, 500), Point(0, 1001), Point(1001, 0)] {
            let recipe = Recipe {
                steps: vec![action(slot, kind, 1, point)],
            };
            assert!(recipe.compile(&cards, &image, false).is_err());
        }
    }
    let mut recipe = Recipe {
        steps: vec![
            action(1, UnitKind::Troop, 5, Point(0, 400)),
            action(1, UnitKind::Troop, 0, Point(1000, 600)),
            action(3, UnitKind::Siege, 1, Point(300, 0)),
            action(4, UnitKind::Hero, 1, Point(700, 0)),
            action(5, UnitKind::Hero, 1, Point(700, 1000)),
            action(8, UnitKind::Spell, 1, Point(640, 320)),
        ],
    };
    recipe.steps[5].delay_ms = 5000;
    let plan = recipe.compile(&cards, &image, false).unwrap();
    assert!(matches!(
        plan[0],
        Deployment::PreciseDrop {
            count: 5,
            point: Point(0, 400),
            ..
        }
    ));
    assert!(matches!(
        plan[1],
        Deployment::PreciseDrop {
            count: 10,
            point: Point(1000, 600),
            ..
        }
    ));
    assert!(matches!(plan[5], Deployment::Wait(5000)));
    assert!(matches!(plan[6], Deployment::PreciseDrop { count: 1, .. }));
    assert_eq!(plan.len(), 7);
    recipe
        .steps
        .push(action(1, UnitKind::Troop, 1, Point(0, 300)));
    assert!(recipe.compile(&cards, &image, false).is_err());
    recipe.steps.pop();
    recipe.steps[0].portrait.fill(0);
    assert!(recipe.compile(&cards, &image, false).is_err());
    recipe.steps[0] = action(4, UnitKind::Hero, 1, Point(0, 300));
    recipe.steps[0].ability = true;
    assert!(recipe.compile(&cards, &image, false).is_err());
}

#[test]
fn captured_hero_meters_and_stars_match_baseline() {
    for n in 1..=3 {
        assert_eq!(
            army::hero_charge(
                &fixture(&format!("builder-hero-meter-{n}.png")),
                Point(129, 128)
            )
            .unwrap(),
            n
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
    assert!(army::hero_charge(&fixture("builder-hero-meter-1.png"), Point(0, 0)).is_err());
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
fn bound_portraits_survive_selection_but_reject_every_other_captured_unit() {
    use army::recipe::{Recipe, Step, portrait, unit_kind};
    let source = fixture("battlefield-classic.png");
    let selected = fixture("battlefield-selected-army.png");
    let layout: Layout =
        serde_json::from_value(config()["screens"]["home_scout"]["auto"].clone()).unwrap();
    let original = army::cards(&source, layout.bar, false, &digits()).unwrap();
    let current = army::cards(&selected, layout.bar, false, &digits()).unwrap();
    assert_eq!(original.len(), 10);
    assert_eq!(current.len(), 10);
    for (i, c) in original.iter().enumerate() {
        assert_eq!(
            army::selected(&selected, c.slot).unwrap(),
            i == 0,
            "selection slot {i}"
        );
    }
    assert_eq!(
        army::recipe::BoundCard::new(&original[0], &source, false)
            .unwrap()
            .remaining(&selected, &digits())
            .unwrap(),
        Some(15)
    );
    for i in 0..original.len() {
        let step = Step {
            supports_ability: true,
            portrait: portrait(&source, original[i].slot).unwrap(),
            label: String::new(),
            slot: i + 1,
            kind: unit_kind(&original, &original[i], false),
            point: Point(0, 500),
            count: 1,
            delay_ms: 0,
            interval_ms: 300,
            ability: false,
        };
        let mut recipe = Recipe { steps: vec![step] };
        recipe.compile(&current, &selected, false).unwrap();
        for j in 0..original.len() {
            if i != j {
                recipe.steps[0].portrait = portrait(&source, original[j].slot).unwrap();
                assert!(
                    recipe.compile(&original, &source, false).is_err(),
                    "Different units {i} and {j} matched"
                );
            }
        }
    }
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

#[test]
fn builder_card_states_require_the_bound_portrait_and_deployment_evidence() {
    use army::recipe::BoundCard;
    let before = fixture("deployment-builder-before.png");
    let layout: Layout =
        serde_json::from_value(config()["screens"]["builder_scout"]["auto"].clone()).unwrap();
    let cards = army::cards(&before, layout.bar, true, &digits()).unwrap();
    let hero = BoundCard::new(&cards[0], &before, true).unwrap();
    assert_eq!(hero.remaining(&before, &digits()).unwrap(), Some(1));
    let deployed = fixture("deployment-builder-hero-deployed.png");
    assert_eq!(hero.remaining(&deployed, &digits()).unwrap(), Some(0));
    assert_eq!(
        hero.ability_ready(&deployed, &digits()).unwrap(),
        Some(true)
    );
    let low = low_health(&deployed, hero.card.slot);
    assert_eq!(hero.remaining(&low, &digits()).unwrap(), Some(0));
    assert_eq!(hero.ability_ready(&low, &digits()).unwrap(), Some(true));
    let survivors = fixture("deployment-builder-survivors.png");
    assert_eq!(hero.remaining(&survivors, &digits()).unwrap(), Some(1));
    let selected = fixture("deployment-builder-selected.png");
    assert!(army::selected(&selected, cards[1].slot).unwrap());
    for card in cards.iter().skip(2) {
        assert!(!army::selected(&selected, card.slot).unwrap());
    }
    let mut troop = BoundCard::new(&cards[1], &before, true).unwrap();
    assert_eq!(troop.remaining(&selected, &digits()).unwrap(), Some(1));
    assert_eq!(troop.remaining(&survivors, &digits()).unwrap(), Some(0));
    for (file, ready) in [
        ("deployment-builder-full-health-ready.png", true),
        ("deployment-builder-full-health-used.png", false),
    ] {
        let image = fixture(file);
        assert_eq!(troop.remaining(&image, &digits()).unwrap(), Some(0));
        assert_eq!(troop.ability_ready(&image, &digits()).unwrap(), Some(ready));
    }
    let low = fixture("deployment-builder-low-health-ready.png");
    let low_troop = BoundCard::new(&cards[3], &before, true).unwrap();
    assert_eq!(low_troop.remaining(&low, &digits()).unwrap(), Some(0));
    assert_eq!(
        low_troop.ability_ready(&low, &digits()).unwrap(),
        Some(true)
    );
    troop.card.slot = Point(850, 648);
    let ready = fixture("deployment-builder-ability-ready.png");
    let used = fixture("deployment-builder-ability-used.png");
    assert_eq!(troop.remaining(&ready, &digits()).unwrap(), Some(0));
    assert_eq!(troop.ability_ready(&ready, &digits()).unwrap(), Some(true));
    assert_eq!(troop.remaining(&used, &digits()).unwrap(), Some(0));
    assert_eq!(troop.ability_ready(&used, &digits()).unwrap(), Some(false));
    let black = Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
    for card in cards {
        let bound = BoundCard::new(&card, &before, true).unwrap();
        assert_eq!(bound.remaining(&black, &digits()).unwrap(), None);
        assert_eq!(bound.ability_ready(&black, &digits()).unwrap(), None);
    }
}

#[test]
fn home_health_bars_distinguish_individually_placed_heroes_from_the_battlefield() {
    use army::recipe::BoundCard;
    let before = fixture("battlefield-classic.png");
    let after = fixture("deployment-home-heroes-deployed.png");
    let layout: Layout =
        serde_json::from_value(config()["screens"]["home_scout"]["auto"].clone()).unwrap();
    let cards = army::cards(&before, layout.bar, false, &digits()).unwrap();
    for (i, card) in cards.iter().enumerate() {
        let bound = BoundCard::new(card, &before, false).unwrap();
        let expected = [11, 4, 1, 0, 0, 1, 1, 3, 2, 1][i];
        assert_eq!(
            bound.remaining(&after, &digits()).unwrap(),
            Some(expected),
            "slot {}",
            i + 1
        );
        if (3..7).contains(&i) {
            assert_eq!(
                bound.ability_ready(&after, &digits()).unwrap(),
                if expected == 0 { Some(true) } else { None }
            );
            for (j, other) in cards.iter().enumerate().filter(|(j, _)| *j != i) {
                let mut wrong = bound.clone();
                wrong.portrait = army::recipe::portrait(&before, other.slot).unwrap();
                assert_eq!(
                    wrong.remaining(&after, &digits()).unwrap(),
                    None,
                    "slot {} bound to {}",
                    i + 1,
                    j + 1
                );
                assert_eq!(wrong.ability_ready(&after, &digits()).unwrap(), None);
            }
            if expected == 0 {
                let low = low_health(&after, card.slot);
                assert_eq!(bound.remaining(&low, &digits()).unwrap(), Some(0));
                assert_eq!(bound.ability_ready(&low, &digits()).unwrap(), Some(true));
            }
        }
    }
    let used = fixture("deployment-home-ability-used.png");
    let queen = BoundCard::new(&cards[4], &before, false).unwrap();
    assert_eq!(queen.ability_ready(&used, &digits()).unwrap(), Some(false));
    let siege = BoundCard::new(&cards[2], &before, false).unwrap();
    assert_eq!(
        siege
            .remaining(&fixture("deployment-home-siege-deployed.png"), &digits())
            .unwrap(),
        None
    );
    let spell = BoundCard::new(&cards[7], &before, false).unwrap();
    assert_eq!(spell.remaining(&used, &digits()).unwrap(), Some(2));
}

fn low_health(source: &Mat, slot: Point) -> Mat {
    let mut image = source.clone();
    // Simulate a nearly depleted bar while retaining its captured rails and leftmost fill.
    for y in slot.1 - 85..slot.1 - 54 {
        for x in slot.0 - 27..slot.0 + 32 {
            let p = image.at_2d_mut::<core::Vec3b>(y as i32, x as i32).unwrap();
            if p[1] > 180 && p[0] < 110 {
                *p = core::Vec3b::from([15, 50, 22]);
            }
        }
    }
    image
}
