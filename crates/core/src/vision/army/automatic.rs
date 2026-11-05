use super::recipe::{Recipe, Step, Unit, UnitKind};
use crate::{Error, Result, calibration::Resolution, device::Point};

const BORDER: [Point; 8] = [
    Point(0, 500),
    Point(0, 0),
    Point(500, 0),
    Point(1000, 0),
    Point(1000, 500),
    Point(1000, 1000),
    Point(500, 1000),
    Point(0, 1000),
];
const INTERIOR: [Point; 4] = [
    Point(350, 350),
    Point(650, 350),
    Point(650, 650),
    Point(350, 650),
];
const QUAKE: [Point; 4] = [
    Point(250, 250),
    Point(750, 250),
    Point(750, 750),
    Point(250, 750),
];

pub fn generate(
    inventory: &[Unit],
    ring: bool,
    builder: bool,
    builder_stack: bool,
    earthquake: bool,
) -> Result<Recipe> {
    if inventory.is_empty() || inventory.len() > 64 {
        return Err(Error::Gameplay(
            "The army must contain 1 to 64 readable slots".into(),
        ));
    }
    for (i, unit) in inventory.iter().enumerate() {
        if !(1..=64).contains(&unit.slot)
            || inventory[..i].iter().any(|old| old.slot == unit.slot)
            || unit.count.is_none_or(|count| count > 999)
        {
            return Err(Error::Gameplay(
                "Army slots or quantities could not be read reliably".into(),
            ));
        }
    }
    let active: Vec<_> = inventory
        .iter()
        .filter(|unit| unit.count != Some(0))
        .collect();
    let ability = |unit: &Unit| {
        unit.supports_ability
            && (unit.kind == UnitKind::Hero || (builder && unit.kind == UnitKind::Troop))
    };
    let mut parts: Vec<_> = active
        .iter()
        .map(|unit| {
            if earthquake && unit.kind == UnitKind::Spell {
                unit.count.unwrap().div_ceil(4) as usize
            } else {
                1
            }
        })
        .collect();
    let mandatory =
        parts.iter().sum::<usize>() + active.iter().filter(|unit| ability(unit)).count();
    if mandatory > 64 {
        return Err(Error::Gameplay(
            "The complete army needs more than 64 deployment actions".into(),
        ));
    }
    let desired: Vec<_> = active
        .iter()
        .zip(&parts)
        .map(|(unit, &minimum)| {
            let preferred = match unit.kind {
                UnitKind::Troop if ring && !builder => BORDER.len(),
                UnitKind::Spell if !earthquake => INTERIOR.len(),
                _ => minimum,
            };
            preferred.min(unit.count.unwrap() as usize)
        })
        .collect();
    let mut extra = 64 - mandatory;
    while extra > 0 {
        let mut added = false;
        for (part, &limit) in parts.iter_mut().zip(&desired) {
            if extra > 0 && *part < limit {
                *part += 1;
                extra -= 1;
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    let mut recipe = Recipe { steps: Vec::new() };
    let order = if builder {
        [
            UnitKind::Hero,
            UnitKind::Troop,
            UnitKind::Siege,
            UnitKind::Spell,
        ]
    } else if ring {
        [
            UnitKind::Hero,
            UnitKind::Siege,
            UnitKind::Spell,
            UnitKind::Troop,
        ]
    } else {
        [
            UnitKind::Troop,
            UnitKind::Siege,
            UnitKind::Hero,
            UnitKind::Spell,
        ]
    };
    for kind in order {
        for (role_index, (unit, &parts)) in active
            .iter()
            .zip(&parts)
            .filter(|(unit, _)| unit.kind == kind)
            .enumerate()
        {
            let count = unit.count.unwrap();
            for part in 0..parts {
                let quantity = if kind == UnitKind::Spell && earthquake {
                    (count - part as u32 * 4).min(4)
                } else {
                    count / parts as u32 + u32::from((part as u32) < count % parts as u32)
                };
                let point = match kind {
                    UnitKind::Troop if builder && !builder_stack => {
                        BORDER[role_index % BORDER.len()]
                    }
                    UnitKind::Troop if !builder && ring => {
                        BORDER[(part * BORDER.len() / parts + role_index) % BORDER.len()]
                    }
                    UnitKind::Troop => Point(0, 500),
                    UnitKind::Hero => Point(0, 250 + (role_index % 5) as u32 * 100),
                    UnitKind::Siege => Point(0, 650),
                    UnitKind::Spell if earthquake => QUAKE[part % QUAKE.len()],
                    UnitKind::Spell => INTERIOR[part * INTERIOR.len() / parts],
                };
                recipe
                    .steps
                    .push(step(unit, point, quantity, false, 0, builder));
            }
        }
        if !builder && ring && kind == UnitKind::Hero {
            for unit in active
                .iter()
                .filter(|unit| unit.kind == UnitKind::Hero && ability(unit))
            {
                recipe
                    .steps
                    .push(step(unit, Point(0, 500), 1, true, 0, builder));
            }
        }
    }
    if builder || !ring {
        let mut first = true;
        // Camp abilities follow every placement; an uncharged hero must not block deploying troops.
        for kind in [UnitKind::Troop, UnitKind::Hero] {
            for unit in active
                .iter()
                .filter(|unit| unit.kind == kind && ability(unit))
            {
                recipe.steps.push(step(
                    unit,
                    Point(0, 500),
                    1,
                    true,
                    if first { 5000 } else { 0 },
                    builder,
                ));
                first = false;
            }
        }
    }
    recipe.validate(Resolution(1280, 720))?;
    Ok(recipe)
}

fn step(
    unit: &Unit,
    point: Point,
    count: u32,
    ability: bool,
    delay_ms: u32,
    builder: bool,
) -> Step {
    Step {
        supports_ability: unit.supports_ability,
        portrait: Vec::new(),
        label: format!(
            "{} {}",
            match unit.kind {
                UnitKind::Troop => "Troop",
                UnitKind::Hero => "Hero",
                UnitKind::Siege => "Siege",
                UnitKind::Spell => "Spell",
            },
            unit.slot
        ),
        slot: unit.slot,
        kind: unit.kind,
        point,
        count,
        delay_ms,
        interval_ms: match unit.kind {
            UnitKind::Spell => 800,
            UnitKind::Troop if !builder => 100,
            _ => 500,
        },
        ability,
    }
}
