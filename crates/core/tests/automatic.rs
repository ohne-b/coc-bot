use coc_bot_core::{
    calibration::Resolution,
    device::Point,
    vision::army::{
        automatic::generate,
        recipe::{Recipe, Unit, UnitKind},
    },
};

fn unit(slot: usize, kind: UnitKind, count: u32, supports_ability: bool) -> Unit {
    Unit {
        slot,
        kind,
        count: Some(count),
        supports_ability,
    }
}
fn totals(recipe: &Recipe, slot: usize) -> u32 {
    recipe
        .steps
        .iter()
        .filter(|step| step.slot == slot && !step.ability)
        .map(|step| step.count)
        .sum()
}

#[test]
fn home_plans_preserve_quantities_roles_and_ring_or_stack_positions() {
    let army = [
        unit(1, UnitKind::Troop, 301, false),
        unit(2, UnitKind::Hero, 1, true),
        unit(3, UnitKind::Siege, 1, false),
        unit(4, UnitKind::Spell, 9, false),
    ];
    for ring in [false, true] {
        let recipe = generate(&army, ring, false, false, false).unwrap();
        recipe.validate(Resolution(1280, 720)).unwrap();
        for unit in &army {
            assert_eq!(totals(&recipe, unit.slot), unit.count.unwrap());
        }
        assert!(recipe.steps.iter().all(|step| step.portrait.is_empty()));
        let troops: Vec<_> = recipe
            .steps
            .iter()
            .filter(|step| step.kind == UnitKind::Troop)
            .collect();
        assert_eq!(troops.len(), if ring { 8 } else { 1 });
        if !ring {
            assert_eq!(troops[0].point, Point(0, 500));
        }
        let hero = recipe
            .steps
            .iter()
            .find(|step| step.kind == UnitKind::Hero && !step.ability)
            .unwrap();
        let siege = recipe
            .steps
            .iter()
            .find(|step| step.kind == UnitKind::Siege)
            .unwrap();
        assert_ne!(hero.point, siege.point);
        assert!(
            recipe
                .steps
                .iter()
                .filter(|step| step.kind == UnitKind::Spell)
                .all(|step| step.interval_ms >= 800
                    && (1..1000).contains(&step.point.0)
                    && (1..1000).contains(&step.point.1))
        );
        let ability = recipe.steps.iter().position(|step| step.ability).unwrap();
        let first_troop = recipe
            .steps
            .iter()
            .position(|step| step.kind == UnitKind::Troop)
            .unwrap();
        if ring {
            assert!(ability < first_troop);
            assert!(
                recipe
                    .steps
                    .iter()
                    .position(|step| step.kind == UnitKind::Siege)
                    .unwrap()
                    < recipe
                        .steps
                        .iter()
                        .position(|step| step.kind == UnitKind::Spell)
                        .unwrap()
            );
        } else {
            assert!(ability > first_troop);
        }
    }
}

#[test]
fn builder_repeated_camps_keep_their_slots_and_activate_only_after_placements() {
    let army = [
        unit(1, UnitKind::Troop, 1, true),
        unit(2, UnitKind::Troop, 1, true),
        unit(3, UnitKind::Troop, 6, false),
        unit(4, UnitKind::Hero, 1, true),
    ];
    for stack in [false, true] {
        let recipe = generate(&army, false, true, stack, false).unwrap();
        assert_eq!(recipe.steps[0].kind, UnitKind::Hero);
        let first_ability = recipe.steps.iter().position(|step| step.ability).unwrap();
        assert!(
            recipe.steps[..first_ability]
                .iter()
                .all(|step| !step.ability)
        );
        assert!(
            recipe.steps[first_ability..]
                .iter()
                .all(|step| step.ability)
        );
        assert_eq!(recipe.steps[first_ability].delay_ms, 5000);
        assert_eq!(
            recipe
                .steps
                .iter()
                .filter(|step| step.ability)
                .map(|step| step.slot)
                .collect::<Vec<_>>(),
            [1, 2, 4]
        );
        for unit in &army {
            assert_eq!(totals(&recipe, unit.slot), unit.count.unwrap());
        }
        let troops: Vec<_> = recipe
            .steps
            .iter()
            .filter(|step| step.kind == UnitKind::Troop && !step.ability)
            .collect();
        assert_eq!(troops.len(), 3);
        if stack {
            assert!(troops.iter().all(|step| step.point == Point(0, 500)));
        } else {
            assert_ne!(troops[0].point, troops[1].point);
        }
    }
}

#[test]
fn large_armies_adapt_distribution_before_expansion_without_truncation() {
    let army: Vec<_> = (1..=26)
        .map(|slot| unit(slot, UnitKind::Troop, 10, false))
        .collect();
    let recipe = generate(&army, true, false, false, false).unwrap();
    assert_eq!(recipe.steps.len(), 64);
    for slot in 1..=26 {
        assert_eq!(totals(&recipe, slot), 10);
    }
    let camps: Vec<_> = (1..=26)
        .map(|slot| unit(slot, UnitKind::Troop, 1, true))
        .collect();
    let recipe = generate(&camps, false, true, false, false).unwrap();
    assert_eq!(recipe.steps.len(), 52);
    for slot in 1..=26 {
        assert_eq!(totals(&recipe, slot), 1);
    }
    let full: Vec<_> = (1..=64)
        .map(|slot| unit(slot, UnitKind::Troop, 1, false))
        .collect();
    assert_eq!(
        generate(&full, true, false, false, false)
            .unwrap()
            .steps
            .len(),
        64
    );
    let too_many: Vec<_> = (1..=33)
        .map(|slot| unit(slot, UnitKind::Troop, 1, true))
        .collect();
    assert!(generate(&too_many, false, true, false, false).is_err());
}

#[test]
fn earthquake_pattern_groups_actual_spells_in_fours_and_stays_within_budget() {
    let army = [unit(1, UnitKind::Spell, 11, false)];
    let recipe = generate(&army, false, false, false, true).unwrap();
    assert_eq!(
        recipe
            .steps
            .iter()
            .map(|step| step.count)
            .collect::<Vec<_>>(),
        [4, 4, 3]
    );
    assert_ne!(recipe.steps[0].point, recipe.steps[1].point);
    assert_ne!(recipe.steps[1].point, recipe.steps[2].point);
    assert_eq!(totals(&recipe, 1), 11);
    let mut crowded: Vec<_> = (1..=26)
        .map(|slot| unit(slot, UnitKind::Troop, 5, false))
        .collect();
    crowded.push(unit(27, UnitKind::Spell, 11, false));
    let recipe = generate(&crowded, true, false, false, true).unwrap();
    assert!(recipe.steps.len() <= 64);
    for unit in &crowded {
        assert_eq!(totals(&recipe, unit.slot), unit.count.unwrap());
    }
    assert_eq!(
        recipe
            .steps
            .iter()
            .filter(|step| step.slot == 27)
            .map(|step| step.count)
            .collect::<Vec<_>>(),
        [4, 4, 3]
    );
}

#[test]
fn invalid_unreadable_exhausted_or_overlong_armies_never_get_guessed_plans() {
    let mut unreadable = unit(1, UnitKind::Troop, 10, false);
    unreadable.count = None;
    assert!(generate(&[unreadable], false, false, false, false).is_err());
    let duplicate = unit(1, UnitKind::Troop, 10, false);
    assert!(generate(&[duplicate.clone(), duplicate], false, false, false, false).is_err());
    assert!(generate(&[], false, false, false, false).is_err());
    assert!(
        generate(
            &[unit(1, UnitKind::Troop, 0, false)],
            false,
            false,
            false,
            false
        )
        .is_err()
    );
    let recipe = generate(
        &[
            unit(1, UnitKind::Troop, 0, false),
            unit(2, UnitKind::Troop, 8, false),
        ],
        false,
        false,
        false,
        false,
    )
    .unwrap();
    assert_eq!(recipe.steps.len(), 1);
    assert_eq!(recipe.steps[0].slot, 2);
    assert!(
        generate(
            &[
                unit(1, UnitKind::Troop, 999, false),
                unit(2, UnitKind::Troop, 999, false)
            ],
            false,
            false,
            false,
            false
        )
        .is_err()
    );
    assert!(
        generate(
            &[unit(1, UnitKind::Spell, 226, false)],
            false,
            false,
            false,
            false
        )
        .is_err()
    );
}
