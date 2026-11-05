use coc_bot_core::{
    calibration::{Region, Resolution},
    device::Point,
    vision::{
        Digits,
        army::{
            self, Deployment,
            recipe::{Recipe, Step, UnitKind},
            roster::{Locate, Pan, Roster, ScanStep, Scanner},
        },
        read_image,
    },
};
use opencv::{
    core::{self, Mat, Rect, Scalar},
    prelude::*,
};
use std::path::{Path, PathBuf};

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
fn digits() -> Digits {
    Digits::load(&assets(), Path::new("calibration/digits")).unwrap()
}
fn fixture(name: &str) -> Mat {
    read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}
fn bar(builder: bool) -> Region {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(assets().join("config.json")).unwrap()).unwrap();
    serde_json::from_value(
        config["screens"][if builder {
            "builder_scout"
        } else {
            "home_scout"
        }]["army_bar"]
            .clone(),
    )
    .unwrap()
}

#[test]
fn captured_complete_bars_scan_without_saved_portraits() {
    let digits = digits();
    for (file, builder, expected) in [
        ("battlefield-classic.png", false, 10),
        ("deployment-builder-before.png", true, 7),
    ] {
        let image = fixture(file);
        let mut scan = Scanner::new(bar(builder), builder).unwrap();
        let ScanStep::Complete(mut roster) = scan.observe(&image, &digits).unwrap() else {
            panic!("{file}: a complete visible army must never request input");
        };
        assert_eq!(roster.inventory().len(), expected);
        let units = roster.inventory();
        for unit in units {
            let Locate::Ready(bound) = roster
                .locate(unit.slot, &image, &digits)
                .unwrap_or_else(|e| panic!("{file}, slot {}: {e}", unit.slot))
            else {
                panic!("visible slot");
            };
            assert_eq!(bound.logical_slot, unit.slot);
            assert_eq!(bound.remaining(&image, &digits).unwrap(), unit.count);
        }
    }
}

#[test]
fn captured_builder_swap_controls_are_not_required_for_a_read_only_inventory() {
    let digits = digits();
    for file in [
        "army-builder-scout.png",
        "army-builder-battle.png",
        "army-builder-selected.png",
    ] {
        let image = fixture(file);
        assert!(
            Mat::roi(&image, Rect::new(0, 0, 1280, 545))
                .unwrap()
                .data_bytes()
                .unwrap()
                .iter()
                .all(|p| *p == 0)
        );
        let cards = army::cards(&image, bar(true), true, &digits).unwrap();
        assert_eq!(cards.len(), 7, "{file}");
        let mut scanner = Scanner::new(bar(true), true).unwrap();
        let ScanStep::Complete(mut roster) = scanner.observe(&image, &digits).unwrap() else {
            panic!("{file}: a fully visible army must not send input")
        };
        let units = roster.inventory();
        assert_eq!(units.len(), 7);
        assert_eq!(units[0].kind, UnitKind::Hero);
        for unit in &units {
            assert_eq!(
                unit.kind,
                if unit.slot == 1 {
                    UnitKind::Hero
                } else {
                    UnitKind::Troop
                }
            );
            assert_eq!(unit.count, Some(1));
            let Locate::Ready(bound) = roster
                .locate(unit.slot, &image, &digits)
                .unwrap_or_else(|e| panic!("{file} slot{}: {e}", unit.slot))
            else {
                panic!("visible slot")
            };
            assert_eq!(bound.remaining(&image, &digits).unwrap(), Some(1));
        }
    }
    let image = fixture("army-builder-scout.png");
    let card = army::cards(&image, bar(true), true, &digits)
        .unwrap()
        .remove(0);
    let bound = army::recipe::BoundCard::new(&card, &image, true).unwrap();
    let mut missing_header = image.try_clone().unwrap();
    Mat::roi_mut(
        &mut missing_header,
        Rect::new(card.slot.0 as i32 - 32, 588, 64, 24),
    )
    .unwrap()
    .set_to(&Scalar::all(0.), &core::no_array())
    .unwrap();
    assert_eq!(bound.remaining(&missing_header, &digits).unwrap(), None);
    assert_eq!(
        bound
            .remaining(&fixture("deployment-builder-hero-deployed.png"), &digits)
            .unwrap(),
        Some(0)
    );
    let mut missing_camp_header = fixture("army-builder-battle.png");
    Mat::roi_mut(&mut missing_camp_header, Rect::new(300, 588, 88, 28))
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
    assert!(
        Scanner::new(bar(true), true)
            .unwrap()
            .observe(&missing_camp_header, &digits)
            .is_err()
    );
}

#[test]
fn selecting_an_undeployed_builder_hero_keeps_it_available() {
    let digits = digits();
    let before = fixture("army-builder-hero-waiting.png");
    let after = fixture("army-builder-hero-selected.png");
    let cards = army::cards(&before, bar(true), true, &digits).unwrap();
    let card = &cards[0];
    assert_eq!(card.kind, army::Kind::Hero);
    let bound = army::recipe::BoundCard::new(card, &before, true).unwrap();
    assert!(!army::selected(&before, card.slot).unwrap());
    assert!(army::selected(&after, card.slot).unwrap());
    assert!(!army::hero_deployed(&before, card.slot).unwrap());
    assert!(!army::hero_deployed(&after, card.slot).unwrap());
    assert_eq!(bound.remaining(&before, &digits).unwrap(), Some(1));
    assert_eq!(bound.remaining(&after, &digits).unwrap(), Some(1));
    assert_eq!(bound.ability_ready(&after, &digits).unwrap(), None);
    let mut obscured = after.try_clone().unwrap();
    Mat::roi_mut(
        &mut obscured,
        Rect::new(card.slot.0 as i32 - 40, 580, 80, 24),
    )
    .unwrap()
    .set_to(&Scalar::all(0.), &core::no_array())
    .unwrap();
    assert_eq!(bound.remaining(&obscured, &digits).unwrap(), None);
    let mut missing_health = fixture("deployment-builder-hero-deployed.png");
    assert_eq!(bound.remaining(&missing_health, &digits).unwrap(), Some(0));
    Mat::roi_mut(
        &mut missing_health,
        Rect::new(card.slot.0 as i32 - 32, 571, 64, 10),
    )
    .unwrap()
    .set_to(&Scalar::all(0.), &core::no_array())
    .unwrap();
    // The intact charge meter and fresh head still positively identify a deployed hero.
    assert_eq!(bound.remaining(&missing_health, &digits).unwrap(), Some(0));
    Mat::roi_mut(
        &mut missing_health,
        Rect::new(card.slot.0 as i32 - 38, 545, 76, 25),
    )
    .unwrap()
    .set_to(&Scalar::all(0.), &core::no_array())
    .unwrap();
    assert_eq!(bound.remaining(&missing_health, &digits).unwrap(), None);
    assert_eq!(bound.ability_ready(&missing_health, &digits).unwrap(), None);
}

fn stationary_roster(image: &Mat, builder: bool, digits: &Digits) -> Roster {
    let mut scanner = Scanner::new(bar(builder), builder).unwrap();
    for _ in 0..4 {
        if let ScanStep::Complete(roster) = scanner.observe(image, digits).unwrap() {
            return roster;
        }
    }
    panic!("stationary complete roster");
}

#[test]
fn unreadable_builder_hero_retries_without_input_and_requires_a_positive_read() {
    let digits = digits();
    let before = fixture("army-builder-hero-flash-before.png");
    let clear = fixture("army-builder-hero-flash-clear.png");
    let mut flash = fixture("army-builder-hero-flash.png");
    // Hide its remaining head evidence without moving the surrounding bar.
    Mat::roi_mut(&mut flash, Rect::new(124, 580, 28, 52))
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
    let mut roster = stationary_roster(&before, true, &digits);
    assert!(matches!(
        roster.locate(1, &clear, &digits).unwrap(),
        Locate::Ready(_)
    ));
    assert!(matches!(
        roster.locate(1, &flash, &digits).unwrap(),
        Locate::Wait
    ));
    let Locate::Ready(card) = roster.locate(1, &clear, &digits).unwrap() else {
        panic!("a clear health frame and fresh identity allow the hero again")
    };
    assert_eq!(card.remaining(&clear, &digits).unwrap(), Some(0));
    assert_eq!(card.ability_ready(&clear, &digits).unwrap(), Some(true));
    assert_eq!(card.remaining(&flash, &digits).unwrap(), None);
    assert_eq!(card.ability_ready(&flash, &digits).unwrap(), None);
    assert_eq!(army::hero_charge(&flash, card.card.slot).unwrap(), 3);

    // A fresh successful read resets the budget; four unreadable captures never authorize input.
    for _ in 0..3 {
        assert!(matches!(
            roster.locate(1, &flash, &digits).unwrap(),
            Locate::Wait
        ));
    }
    assert!(roster.locate(1, &flash, &digits).is_err());
    assert!(roster.locate(1, &clear, &digits).is_err());

    let mut roster = stationary_roster(&before, true, &digits);
    assert!(matches!(
        roster.locate(1, &clear, &digits).unwrap(),
        Locate::Ready(_)
    ));
    assert!(matches!(
        roster.locate(1, &flash, &digits).unwrap(),
        Locate::Wait
    ));
    assert!(roster.locate(2, &flash, &digits).is_err());

    for replace_portrait in [true, false] {
        let mut changed = clear.try_clone().unwrap();
        let target = Rect::new(85, 582, 88, 128);
        if replace_portrait {
            let wrong_unit = Mat::roi(&clear, Rect::new(199, 582, 88, 128)).unwrap();
            wrong_unit
                .copy_to(&mut Mat::roi_mut(&mut changed, target).unwrap())
                .unwrap();
        } else {
            Mat::roi_mut(&mut changed, target)
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
        }
        let mut roster = stationary_roster(&before, true, &digits);
        let mut rejected = false;
        for _ in 0..4 {
            match roster.locate(1, &changed, &digits) {
                Ok(Locate::Wait) => {}
                Err(_) => {
                    rejected = true;
                    break;
                }
                _ => panic!("changed or hidden portrait must not authorize any input"),
            }
        }
        assert!(rejected);
        assert!(roster.locate(1, &clear, &digits).is_err());
    }
}

#[test]
fn captured_builder_activation_uses_fresh_head_and_structured_meter_evidence() {
    let digits = digits();
    let before = fixture("army-builder-charge-before.png");
    let card = army::cards(&before, bar(true), true, &digits)
        .unwrap()
        .remove(0);
    let bound = army::recipe::BoundCard::new(&card, &before, true).unwrap();
    assert_eq!(bound.remaining(&before, &digits).unwrap(), Some(1));
    assert_eq!(
        bound
            .ability_ready(&fixture("army-builder-activation-early.png"), &digits)
            .unwrap(),
        None
    );
    for (name, charge, ready) in [
        ("army-builder-activation-consumed.png", 0, false),
        ("army-builder-charge-empty.png", 0, false),
        ("army-builder-charge-one.png", 1, true),
        ("army-builder-charge-two.png", 2, true),
        ("army-builder-hero-flash-clear.png", 3, true),
        ("army-builder-hero-flash.png", 3, true),
    ] {
        let frame = fixture(name);
        assert_eq!(bound.remaining(&frame, &digits).unwrap(), Some(0), "{name}");
        assert_eq!(
            bound.ability_ready(&frame, &digits).unwrap(),
            Some(ready),
            "{name}"
        );
        assert_eq!(army::hero_charge(&frame, card.slot).unwrap(), charge);
    }
    assert_eq!(
        bound
            .ability_ready(&fixture("army-builder-head-obscured.png"), &digits)
            .unwrap(),
        None
    );

    let consumed = fixture("army-builder-activation-consumed.png");
    let mut flat_meter = consumed.try_clone().unwrap();
    Mat::roi_mut(&mut flat_meter, Rect::new(88, 545, 82, 25))
        .unwrap()
        .set_to(&Scalar::all(25.), &core::no_array())
        .unwrap();
    assert_eq!(bound.ability_ready(&flat_meter, &digits).unwrap(), None);
    for region in [
        Rect::new(0, 0, 1280, 720),
        Rect::new(124, 580, 28, 52),
        Rect::new(88, 545, 82, 25),
    ] {
        let mut hidden = consumed.try_clone().unwrap();
        Mat::roi_mut(&mut hidden, region)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert_eq!(
            bound.ability_ready(&hidden, &digits).unwrap(),
            None,
            "{region:?}"
        );
    }
    // The meter alone cannot turn another hero into this attack's bound hero.
    let other = fixture("battlefield-classic.png");
    for other_card in army::cards(&other, bar(false), false, &digits)
        .unwrap()
        .iter()
        .filter(|c| c.kind == army::Kind::Hero)
    {
        let mut wrong = army::recipe::BoundCard::new(other_card, &other, true).unwrap();
        wrong.card = card.clone();
        assert_eq!(wrong.ability_ready(&consumed, &digits).unwrap(), None);
    }
}

#[test]
fn captured_scenery_does_not_become_an_extra_partial_army_card() {
    let digits = digits();
    for (file, builder) in [
        ("battlefield-forest.png", false),
        ("battlefield-forest-small.png", false),
        ("battlefield-haunted.png", false),
        ("battlefield-builder-cave.png", true),
    ] {
        let image = fixture(file);
        let bar = bar(builder);
        let expected = army::cards(&image, bar, builder, &digits).unwrap().len();
        let mut scanner = Scanner::new(bar, builder).unwrap();
        let mut complete = false;
        for _ in 0..4 {
            if let ScanStep::Complete(roster) = scanner
                .observe(&image, &digits)
                .unwrap_or_else(|e| panic!("{file}: {e}"))
            {
                assert_eq!(roster.inventory().len(), expected, "{file}");
                complete = true;
                break;
            }
        }
        assert!(complete, "{file}");
    }
    // This capture is already at 100% with depleted/active camps; it is not a one-hero army.
    assert!(
        Scanner::new(bar(true), true)
            .unwrap()
            .observe(&fixture("battlefield-builder-clearing.png"), &digits)
            .is_err()
    );
}

#[test]
fn captured_selection_and_hero_deployment_preserve_runtime_binding() {
    let digits = digits();
    for (before, after, builder, slot, expected) in [
        (
            "battlefield-classic.png",
            "battlefield-selected-army.png",
            false,
            1,
            15,
        ),
        (
            "deployment-builder-before.png",
            "deployment-builder-selected.png",
            true,
            2,
            1,
        ),
        (
            "deployment-builder-before.png",
            "deployment-builder-hero-deployed.png",
            true,
            1,
            0,
        ),
    ] {
        let mut roster = stationary_roster(&fixture(before), builder, &digits);
        let frame = fixture(after);
        let Locate::Ready(card) = roster
            .locate(slot, &frame, &digits)
            .unwrap_or_else(|e| panic!("{after}: {e}"))
        else {
            panic!("visible current card");
        };
        assert_eq!(card.remaining(&frame, &digits).unwrap(), Some(expected));
    }
    let before = fixture("battlefield-classic.png");
    let mut roster = stationary_roster(&before, false, &digits);
    let mut changed = before.try_clone().unwrap();
    let cards = army::cards(&before, bar(false), false, &digits).unwrap();
    let target = cards[0].slot;
    let source = cards[4].slot;
    Mat::roi(
        &before,
        Rect::new(source.0 as i32 - 30, source.1 as i32 - 25, 60, 50),
    )
    .unwrap()
    .copy_to(
        &mut Mat::roi_mut(
            &mut changed,
            Rect::new(target.0 as i32 - 30, target.1 as i32 - 25, 60, 50),
        )
        .unwrap(),
    )
    .unwrap();
    for _ in 0..3 {
        assert!(matches!(
            roster.locate(1, &changed, &digits).unwrap(),
            Locate::Wait
        ));
    }
    assert!(roster.locate(1, &changed, &digits).is_err());
    assert!(roster.locate(1, &before, &digits).is_err());
}

/// These viewports simulate scrolling captured pixels; they do not establish live swipe behavior.
struct Strip {
    image: Mat,
    builder: bool,
    bar: Region,
    digits: Digits,
    offset: i32,
    max: i32,
    centers: Vec<i32>,
}
impl Strip {
    fn new(builder: bool, slots: &[usize], gaps: &[i32]) -> Self {
        let source = fixture(if builder {
            "deployment-builder-before.png"
        } else {
            "battlefield-classic.png"
        });
        let bar = bar(builder);
        let digits = digits();
        let cards = army::cards(&source, bar, builder, &digits).unwrap();
        let mut left = 90;
        let mut placements = Vec::new();
        for (i, &slot) in slots.iter().enumerate() {
            let card = &cards[slot];
            placements.push((left, card));
            left += card.region.2 as i32 + gaps.get(i).copied().unwrap_or(12);
        }
        let width = (left + 78).max(1280);
        let mut image =
            Mat::new_rows_cols_with_default(720, width, core::CV_8UC3, Scalar::all(0.)).unwrap();
        let mut centers = Vec::new();
        for (left, card) in placements {
            let crop = Rect::new(card.region.0 as i32 - 2, 0, card.region.2 as i32 + 4, 720);
            Mat::roi(&source, crop)
                .unwrap()
                .copy_to(
                    &mut Mat::roi_mut(&mut image, Rect::new(left - 2, 0, crop.width, 720)).unwrap(),
                )
                .unwrap();
            centers.push(left + card.region.2 as i32 / 2);
        }
        Self {
            image,
            builder,
            bar,
            digits,
            offset: 0,
            max: width - 1280,
            centers,
        }
    }
    fn frame(&self) -> Mat {
        Mat::roi(&self.image, Rect::new(self.offset, 0, 1280, 720))
            .unwrap()
            .try_clone()
            .unwrap()
    }
    fn pan(&mut self, pan: Pan) {
        let cards = army::cards(&self.frame(), self.bar, self.builder, &self.digits).unwrap();
        for point in [pan.from, pan.to] {
            assert!(
                cards
                    .iter()
                    .any(|c| point.0 >= c.region.0 + 8 && point.0 <= c.region.0 + c.region.2 - 8),
                "drag endpoint outside a card: {pan:?}, offset{}, cards{:?}",
                self.offset,
                cards.iter().map(|c| c.region).collect::<Vec<_>>()
            );
        }
        let left = pan.from.0.min(pan.to.0);
        let right = pan.from.0.max(pan.to.0);
        let crossed: Vec<_> = cards
            .iter()
            .filter(|c| c.region.0 + c.region.2 >= left && c.region.0 <= right)
            .collect();
        assert!(
            crossed
                .windows(2)
                .all(|pair| pair[1].region.0 - pair[0].region.0 - pair[0].region.2 <= 20),
            "drag crosses blank space: {pan:?}"
        );
        let requested = pan.to.0 as i32 - pan.from.0 as i32;
        // Short gestures move less than the finger distance after the game's drag threshold.
        let movement = if matches!(requested.abs(), 48 | 64) {
            requested.signum() * 36
        } else {
            requested
        };
        self.offset = (self.offset - movement).clamp(0, self.max);
    }
    fn scan(&mut self, builder: bool) -> Roster {
        let digits = digits();
        let mut scanner = Scanner::new(bar(builder), builder).unwrap();
        for _ in 0..256 {
            match scanner.observe(&self.frame(), &digits).unwrap() {
                ScanStep::Pan(pan) => self.pan(pan),
                ScanStep::Wait => (),
                ScanStep::Complete(roster) => return roster,
            }
        }
        panic!("bounded complete scan");
    }
    fn locate(&mut self, roster: &mut Roster, slot: usize) {
        let digits = digits();
        for _ in 0..256 {
            match roster
                .locate(slot, &self.frame(), &digits)
                .unwrap_or_else(|error| panic!("slot {slot}, offset {}: {error}", self.offset))
            {
                Locate::Pan(pan) => self.pan(pan),
                Locate::Wait => {}
                Locate::Ready(bound) => {
                    assert_eq!(bound.logical_slot, slot);
                    assert!(
                        (bound.card.slot.0 as i32 - (self.centers[slot - 1] - self.offset)).abs()
                            <= 3
                    );
                    assert!(bound.remaining(&self.frame(), &digits).unwrap().is_some());
                    return;
                }
            }
        }
        panic!("bounded lookup");
    }
}

fn step(slot: usize, kind: UnitKind) -> Step {
    Step {
        slot,
        kind,
        label: String::new(),
        portrait: Vec::new(),
        supports_ability: false,
        point: Point(0, 500),
        count: 1,
        delay_ms: 0,
        interval_ms: 300,
        ability: false,
    }
}

#[test]
fn captured_stage_two_custom_plan_skips_only_defeated_slots_and_their_actions() {
    let image = fixture("army-builder-stage2-survivors.png");
    let digits = digits();
    let ScanStep::Complete(roster) = Scanner::new(bar(true), true)
        .unwrap()
        .observe(&image, &digits)
        .unwrap()
    else {
        panic!("all nine captured slots must be available without input")
    };
    let units = roster.inventory();
    assert_eq!(units.len(), 9);
    assert_eq!(units[2].count, Some(0));
    assert_eq!(units[3].count, Some(0));
    let mut recipe = Recipe { steps: Vec::new() };
    for unit in units {
        let mut action = step(unit.slot, unit.kind);
        action.delay_ms = unit.slot as u32 * 100;
        action.point = Point(0, unit.slot as u32 * 100);
        recipe.steps.push(action.clone());
        action.ability = true;
        action.delay_ms += 50;
        recipe.steps.push(action);
    }
    let plan = recipe.compile_roster(&roster).unwrap();
    let surviving = [1, 2, 5, 6, 7, 8, 9];
    assert_eq!(plan.len(), surviving.len() * 4);
    for (actions, slot) in plan.chunks_exact(4).zip(surviving) {
        assert!(matches!(actions[0], Deployment::Wait(ms) if ms == slot * 100));
        assert!(matches!(&actions[1], Deployment::PreciseDrop {
            card, point, count: 1, initial_count: 1, ..
        } if card.logical_slot == slot as usize && *point == Point(0, slot * 100)));
        assert!(matches!(actions[2], Deployment::Wait(ms) if ms == slot * 100 + 50));
        assert!(matches!(&actions[3], Deployment::PreciseAbility { card }
            if card.logical_slot == slot as usize));
    }
    let mut mismatch = recipe.clone();
    mismatch.steps[4].kind = UnitKind::Hero;
    assert!(mismatch.compile_roster(&roster).is_err());
    let mut missing = recipe.clone();
    missing.steps.push(step(10, UnitKind::Troop));
    assert!(missing.compile_roster(&roster).is_err());
    let mut overallocated = recipe.clone();
    overallocated.steps.push(step(2, UnitKind::Troop));
    assert!(overallocated.compile_roster(&roster).is_err());
    // All remaining cannot turn an already allocated live camp into a defeated one.
    overallocated.steps.last_mut().unwrap().count = 0;
    assert!(overallocated.compile_roster(&roster).is_err());
    for region in [Rect::new(302, 590, 24, 18), Rect::new(310, 628, 68, 40)] {
        let mut hidden = image.try_clone().unwrap();
        Mat::roi_mut(&mut hidden, region)
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        assert!(
            Scanner::new(bar(true), true)
                .unwrap()
                .observe(&hidden, &digits)
                .is_err(),
            "missing identity evidence cannot be skipped as a defeated slot"
        );
    }
}

#[test]
fn sized_builder_survivors_require_exact_counts_or_explicit_all_remaining() {
    let image = fixture("builder-barbarian-bar.png");
    let cards = army::cards(
        &image,
        Region(0, 0, image.cols() as u32, image.rows() as u32),
        true,
        &digits(),
    )
    .unwrap();
    assert_eq!(cards[1].kind, army::Kind::Sized);
    assert_eq!(cards[1].count, Some(4));
    let mut action = step(2, UnitKind::Troop);
    action.count = 5;
    let mut recipe = Recipe {
        steps: vec![action],
    };
    assert!(recipe.compile(&cards, &image, true).is_err());
    recipe.steps[0].count = 0;
    let plan = recipe.compile(&cards, &image, true).unwrap();
    assert!(matches!(plan.as_slice(), [Deployment::PreciseDrop {
        card, count: 4, initial_count: 4, ..
    }] if card.logical_slot == 2));
    let mut unreadable = cards.clone();
    unreadable[1].count = None;
    assert!(recipe.compile(&unreadable, &image, true).is_err());
    recipe.steps.push(step(2, UnitKind::Troop));
    assert!(recipe.compile(&cards, &image, true).is_err());
}

#[test]
fn simulated_identical_builder_camps_keep_global_slots_in_both_directions() {
    let mut strip = Strip::new(true, &[vec![0], vec![1; 25]].concat(), &[]);
    strip.offset = 430;
    let mut roster = strip.scan(true);
    assert_eq!(roster.inventory().len(), 26);
    assert_eq!(roster.inventory()[0].kind, UnitKind::Hero);
    assert!(
        roster.inventory()[1..]
            .iter()
            .all(|unit| unit.kind == UnitKind::Troop && unit.count == Some(1))
    );
    let plan = Recipe {
        steps: vec![step(22, UnitKind::Troop), step(2, UnitKind::Troop)],
    }
    .compile_roster(&roster)
    .unwrap();
    assert!(matches!(&plan[0], Deployment::PreciseDrop { card, .. } if card.logical_slot == 22));
    for slot in [2, 26, 13, 1, 22] {
        strip.locate(&mut roster, slot);
    }
}

#[test]
fn simulated_home_groups_are_assigned_across_the_whole_bar() {
    let slots = [vec![0; 16], vec![2, 3, 4, 7, 8, 9]].concat();
    let mut gaps = vec![12; slots.len()];
    gaps[15] = 28;
    gaps[16] = 28;
    gaps[18] = 28;
    let mut strip = Strip::new(false, &slots, &gaps);
    let mut roster = strip.scan(false);
    let units = roster.inventory();
    assert_eq!(units.len(), slots.len());
    assert!(units[..16].iter().all(|unit| unit.kind == UnitKind::Troop));
    assert_eq!(units[16].kind, UnitKind::Siege);
    assert!(units[17..19].iter().all(|unit| unit.kind == UnitKind::Hero));
    assert!(units[19..].iter().all(|unit| unit.kind == UnitKind::Spell));
    Recipe {
        steps: vec![step(1, UnitKind::Troop), step(22, UnitKind::Spell)],
    }
    .compile_roster(&roster)
    .unwrap();
    strip.locate(&mut roster, 1);
    strip.locate(&mut roster, 22);
}

#[test]
fn clipped_unmoved_and_wrong_direction_views_fail_without_renumbering() {
    let digits = digits();
    let mut strip = Strip::new(true, &[1; 25], &[]);
    strip.offset = 430;
    let mut scan = Scanner::new(bar(true), true).unwrap();
    assert!(matches!(
        scan.observe(&strip.frame(), &digits).unwrap(),
        ScanStep::Pan(_)
    ));
    for attempt in 0..3 {
        for _ in 0..2 {
            assert!(matches!(
                scan.observe(&strip.frame(), &digits).unwrap(),
                ScanStep::Wait
            ));
        }
        if attempt < 2 {
            assert!(matches!(
                scan.observe(&strip.frame(), &digits).unwrap(),
                ScanStep::Pan(_)
            ));
        }
    }
    assert!(scan.observe(&strip.frame(), &digits).is_err());
    assert!(scan.observe(&strip.frame(), &digits).is_err());
    let mut scan = Scanner::new(bar(true), true).unwrap();
    let ScanStep::Pan(pan) = scan.observe(&strip.frame(), &digits).unwrap() else {
        panic!()
    };
    strip.pan(Pan {
        from: pan.to,
        to: pan.from,
    });
    assert!(scan.observe(&strip.frame(), &digits).is_err());
    assert!(Scanner::new(Region(0, 600, 1280, 121), true).is_err());
    assert!(
        Recipe {
            steps: vec![step(65, UnitKind::Troop)]
        }
        .validate(Resolution(1280, 720))
        .is_err()
    );
    assert!(
        Recipe {
            steps: vec![step(64, UnitKind::Troop)]
        }
        .validate(Resolution(1280, 720))
        .is_ok()
    );
    let mut oversized = step(1, UnitKind::Troop);
    oversized.portrait = vec![0; 769];
    assert!(
        Recipe {
            steps: vec![oversized]
        }
        .validate(Resolution(1280, 720))
        .is_err()
    );
}

#[test]
fn excessive_inventory_is_rejected_without_a_partial_complete_result() {
    let digits = digits();
    let mut strip = Strip::new(true, &[1; 65], &[]);
    let mut scanner = Scanner::new(bar(true), true).unwrap();
    for _ in 0..512 {
        match scanner.observe(&strip.frame(), &digits) {
            Ok(ScanStep::Pan(pan)) => strip.pan(pan),
            Ok(ScanStep::Wait) => (),
            Ok(ScanStep::Complete(_)) => panic!("oversized roster accepted"),
            Err(error) => {
                assert!(error.to_string().contains("more than 64"), "{error}");
                return;
            }
        }
    }
    panic!("roster limit not enforced");
}
