use coc_bot_core::{
    calibration::{Region, Resolution},
    device::Point,
    vision::{
        Digits,
        army::{self, Deployment, Kind, automatic, recipe},
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

#[test]
fn fixed_deployment_timing_is_rejected_before_an_army_is_read() {
    let action = recipe::Step {
        supports_ability: false,
        portrait: vec![],
        label: "Troops".into(),
        slot: 1,
        kind: recipe::UnitKind::Troop,
        point: Point(0, 500),
        count: 18,
        delay_ms: 0,
        interval_ms: 10_000,
        ability: false,
    };
    for (count, delay, valid) in [(18, 0, true), (18, 1, false), (20, 0, false), (0, 0, true)] {
        let plan = recipe::Recipe {
            steps: vec![recipe::Step {
                count,
                delay_ms: delay,
                ..action.clone()
            }],
        };
        assert_eq!(plan.validate(Resolution(1280, 720)).is_ok(), valid);
    }
    let plan = recipe::Recipe {
        steps: vec![action.clone(), recipe::Step { count: 1, ..action }],
    };
    assert!(plan.validate(Resolution(1280, 720)).is_err());
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
        let inventory = recipe::inventory(&cards, true);
        let generated = automatic::generate(&inventory, false, true, false, false).unwrap();
        let plan = generated.compile(&cards, &image, true).unwrap();
        let count = if file.contains("barbarian") { 4 } else { 1 };
        for card in cards.iter().filter(|c| c.kind != Kind::Hero) {
            assert!(
                plan.iter().any(|step| matches!(step, Deployment::PreciseDrop {
                    card: bound, count: quantity, initial_count, ..
                } if bound.card.slot == card.slot && *quantity == count && *initial_count == count)),
                "{file}: {plan:?}"
            );
            assert_eq!(
                plan.iter().any(|step| matches!(step,
                Deployment::PreciseAbility { card: bound } if bound.card.slot == card.slot)),
                card.kind == Kind::Counted
            );
        }
        let first_ability = plan
            .iter()
            .position(|step| matches!(step, Deployment::PreciseAbility { .. }))
            .unwrap();
        assert!(
            plan[..first_ability]
                .iter()
                .any(|step| matches!(step, Deployment::Wait(5000)))
        );
        assert!(
            plan[first_ability..]
                .iter()
                .all(|step| !matches!(step, Deployment::PreciseDrop { .. }))
        );
        let stacked = automatic::generate(&inventory, false, true, true, false)
            .unwrap()
            .compile(&cards, &image, true)
            .unwrap();
        assert!(
            stacked
                .iter()
                .all(|s| !matches!(s,Deployment::PreciseDrop{card, point,..}
                    if card.card.kind != Kind::Hero && *point != Point(0,500)))
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
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    bar.validate(Resolution(1280, 720)).unwrap();
    let cards = army::cards(&image, bar, false, &digits()).unwrap();
    assert_eq!(
        cards
            .iter()
            .filter(|c| c.kind == Kind::Counted)
            .map(|c| c.count)
            .collect::<Vec<_>>(),
        [Some(33), Some(11)]
    );
    let inventory = recipe::inventory(&cards, false);
    let plan = automatic::generate(&inventory, true, false, false, true)
        .unwrap()
        .compile(&cards, &image, false)
        .unwrap();
    for unit in &inventory {
        let placed: u32 = plan
            .iter()
            .filter_map(|step| match step {
                Deployment::PreciseDrop { card, count, .. } if card.logical_slot == unit.slot => {
                    Some(*count)
                }
                _ => None,
            })
            .sum();
        assert_eq!(Some(placed), unit.count);
    }
    let spell = inventory
        .iter()
        .find(|unit| unit.kind == recipe::UnitKind::Spell)
        .unwrap();
    let casts: Vec<_> = plan
        .iter()
        .filter_map(|step| match step {
            Deployment::PreciseDrop {
                card,
                count,
                point,
                interval_ms,
                ..
            } if card.logical_slot == spell.slot => {
                assert!(*interval_ms >= 800);
                Some((*count, *point))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        casts.iter().map(|(count, _)| *count).collect::<Vec<_>>(),
        [4, 4, 3]
    );
    assert_ne!(casts[0].1, casts[1].1);
    assert!(
        matches!(plan.last(), Some(Deployment::PreciseDrop { card, .. }) if card.logical_slot == 1)
    );
}

#[test]
fn adjacent_troop_stacks_do_not_become_siege_units() {
    let mut image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let bar = fixture("battle-bar-dragons-15-balloons-4.png");
    bar.copy_to(&mut Mat::roi_mut(&mut image, Rect::new(0, 586, bar.cols(), bar.rows())).unwrap())
        .unwrap();
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    let cards = army::cards(&image, bar, false, &digits()).unwrap();
    assert_eq!(cards.len(), 10);
    assert_eq!((cards[0].count, cards[1].count), (Some(15), Some(4)));
    assert_eq!((cards[0].group, cards[1].group), (0, 0));
    assert_eq!(cards[2].kind, Kind::Siege);
    let inventory = recipe::inventory(&cards, false);
    assert!(
        inventory[..2]
            .iter()
            .all(|unit| unit.kind == recipe::UnitKind::Troop)
    );
    assert_eq!(
        inventory
            .iter()
            .filter(|unit| unit.kind == recipe::UnitKind::Spell)
            .count(),
        3
    );
    for ring in [false, true] {
        let plan = automatic::generate(&inventory, ring, false, false, false)
            .unwrap()
            .compile(&cards, &image, false)
            .unwrap();
        for unit in &inventory {
            let placed: u32 = plan
                .iter()
                .filter_map(|step| match step {
                    Deployment::PreciseDrop {
                        card, count, point, ..
                    } if card.logical_slot == unit.slot => {
                        assert_eq!(
                            card.portrait,
                            recipe::portrait(&image, cards[unit.slot - 1].slot).unwrap()
                        );
                        if !ring && unit.kind == recipe::UnitKind::Troop {
                            assert_eq!(*point, Point(0, 500));
                        }
                        Some(*count)
                    }
                    _ => None,
                })
                .sum();
            assert_eq!(Some(placed), unit.count);
        }
    }
    let mut unreadable = cards.clone();
    unreadable[0].count = None;
    let inventory = recipe::inventory(&unreadable, false);
    assert!(automatic::generate(&inventory, false, false, false, false).is_err());
}

#[test]
fn explicit_actions_split_units_place_heroes_and_cast_only_requested_spells() {
    use army::recipe::{Recipe, Step, UnitKind, portrait};
    let mut image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let bar = fixture("battle-bar-dragons-15-balloons-4.png");
    bar.copy_to(&mut Mat::roi_mut(&mut image, Rect::new(0, 586, bar.cols(), bar.rows())).unwrap())
        .unwrap();
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    let cards = army::cards(&image, bar, false, &digits()).unwrap();
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
    let rebound = recipe.compile(&cards, &image, false).unwrap();
    assert!(matches!(&rebound[0], Deployment::PreciseDrop { card, .. }
        if card.portrait == portrait(&image, cards[0].slot).unwrap()));
    recipe.steps[0].kind = UnitKind::Spell;
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
fn plans_follow_slots_while_live_cards_reject_identity_changes() {
    use army::recipe::{Recipe, Step, portrait, unit_kind};
    let source = fixture("battlefield-classic.png");
    let selected = fixture("battlefield-selected-army.png");
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    let original = army::cards(&source, bar, false, &digits()).unwrap();
    let current = army::cards(&selected, bar, false, &digits()).unwrap();
    assert_eq!(original.len(), 10);
    assert_eq!(current.len(), 10);
    assert_eq!(
        original.iter().map(|card| card.kind).collect::<Vec<_>>(),
        current.iter().map(|card| card.kind).collect::<Vec<_>>()
    );
    assert!(current.iter().any(|card| card.kind == Kind::Hero));
    assert!(current.iter().any(|card| card.kind == Kind::Warden));
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
                recipe.compile(&original, &source, false).unwrap();
                let mut wrong = army::recipe::BoundCard::new(&original[i], &source, false).unwrap();
                wrong.portrait = recipe.steps[0].portrait.clone();
                assert!(
                    wrong.remaining(&source, &digits()).unwrap().is_none(),
                    "Different units {i} and {j} matched"
                );
            }
        }
    }
}

#[test]
fn captured_selected_archer_requires_its_complete_raised_frame_and_fresh_identity() {
    use army::recipe::BoundCard;
    use opencv::imgproc;
    let before = fixture("army-home-archer-before.png");
    let selected = fixture("army-home-archer-selected.png");
    let digits = digits();
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    let cards = army::cards(&before, bar, false, &digits).unwrap();
    let selected_cards = army::cards(&selected, bar, false, &digits).unwrap();
    assert_eq!(cards[0].slot, Point(59, 651));
    assert_eq!(cards[0].kind, Kind::Counted);
    assert_eq!(selected_cards[0].kind, Kind::Counted);
    assert_eq!(selected_cards[0].count, Some(1));
    assert_eq!(
        cards
            .iter()
            .map(|card| (card.kind, card.count))
            .collect::<Vec<_>>(),
        selected_cards
            .iter()
            .map(|card| (card.kind, card.count))
            .collect::<Vec<_>>()
    );
    for (i, card) in cards.iter().enumerate() {
        assert!(!army::selected(&before, card.slot).unwrap());
        assert_eq!(army::selected(&selected, card.slot).unwrap(), i == 0);
        let bound = BoundCard::new(card, &before, false).unwrap();
        assert_eq!(bound.remaining(&selected, &digits).unwrap(), card.count);
    }
    let bound = BoundCard::new(&cards[0], &before, false).unwrap();
    for x in 56..=62 {
        assert!(
            army::selected(&selected, Point(x, 651)).unwrap(),
            "center {x}"
        );
    }
    let mut wrong = bound.clone();
    wrong.portrait = BoundCard::new(&cards[1], &before, false).unwrap().portrait;
    assert_eq!(wrong.remaining(&selected, &digits).unwrap(), None);

    for missing in [
        Rect::new(8, 620, 15, 70),
        Rect::new(100, 620, 12, 70),
        Rect::new(20, 576, 80, 20),
        Rect::new(20, 700, 80, 19),
    ] {
        let mut obscured = selected.clone();
        Mat::roi_mut(&mut obscured, missing)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert!(
            !army::selected(&obscured, cards[0].slot).unwrap(),
            "{missing:?}"
        );
    }
    let mut no_bright_side = selected.clone();
    Mat::roi_mut(&mut no_bright_side, Rect::new(104, 620, 4, 70))
        .unwrap()
        .set_to(&Scalar::all(195.), &core::no_array())
        .unwrap();
    assert!(!army::selected(&no_bright_side, cards[0].slot).unwrap());
    let mut gray = Mat::default();
    imgproc::cvt_color_def(&before, &mut gray, imgproc::COLOR_BGR2GRAY).unwrap();
    let mut exhausted = Mat::default();
    imgproc::cvt_color_def(&gray, &mut exhausted, imgproc::COLOR_GRAY2BGR).unwrap();
    assert!(!army::selected(&exhausted, cards[0].slot).unwrap());
    assert_eq!(bound.remaining(&exhausted, &digits).unwrap(), Some(0));
}

#[test]
fn malformed_and_empty_bars_never_produce_a_deployment() {
    let image =
        Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.0)).unwrap();
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    assert!(army::cards(&image, Region(0, 715, 100, 100), false, &digits()).is_err());
    assert!(
        army::cards(&image, bar, false, &digits())
            .unwrap()
            .is_empty()
    );
    assert!(automatic::generate(&[], false, false, false, false).is_err());
    assert!(
        Region(1280, 100, 100, 100)
            .validate(Resolution(1280, 720))
            .is_err()
    );
}

#[test]
fn builder_card_states_require_the_bound_portrait_and_deployment_evidence() {
    use army::recipe::BoundCard;
    let before = fixture("deployment-builder-before.png");
    let bar: Region =
        serde_json::from_value(config()["screens"]["builder_scout"]["army_bar"].clone()).unwrap();
    let cards = army::cards(&before, bar, true, &digits()).unwrap();
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
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    let cards = army::cards(&before, bar, false, &digits()).unwrap();
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

#[test]
fn captured_dead_home_heroes_keep_their_identity_before_an_ability_is_skipped() {
    use army::recipe::BoundCard;
    let before = fixture("army-home-spent-scan.png");
    let after = fixture("army-home-spent-after.png");
    let bar: Region =
        serde_json::from_value(config()["screens"]["home_scout"]["army_bar"].clone()).unwrap();
    let digits = digits();
    let cards = army::cards(&before, bar, false, &digits).unwrap();
    for i in 3..7 {
        let bound = BoundCard::new(&cards[i], &before, false).unwrap();
        assert_eq!(bound.remaining(&before, &digits).unwrap(), Some(1));
        assert_eq!(
            bound.remaining(&after, &digits).unwrap(),
            Some(0),
            "hero {i}"
        );
        assert_eq!(
            bound.ability_ready(&after, &digits).unwrap(),
            Some(i == 4),
            "hero {i}"
        );
        for (j, other) in cards.iter().enumerate().filter(|(j, _)| *j != i) {
            let mut wrong = BoundCard::new(other, &before, false).unwrap();
            wrong.card = bound.card.clone();
            assert_eq!(
                wrong.remaining(&after, &digits).unwrap(),
                None,
                "hero {i} with portrait {j}"
            );
            assert_eq!(wrong.ability_ready(&after, &digits).unwrap(), None);
        }
        for value in [0., 80.] {
            let mut obscured = after.clone();
            Mat::roi_mut(
                &mut obscured,
                Rect::new(cards[i].slot.0 as i32 - 48, 530, 96, 190),
            )
            .unwrap()
            .set_to(&Scalar::all(value), &core::no_array())
            .unwrap();
            assert_eq!(bound.remaining(&obscured, &digits).unwrap(), None);
            assert_eq!(bound.ability_ready(&obscured, &digits).unwrap(), None);
        }
    }
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
