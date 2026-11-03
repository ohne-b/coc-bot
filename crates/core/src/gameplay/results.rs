use super::profile::Target;
use crate::{
    Error, Result,
    calibration::{Region, Resolution},
    telemetry::Loot,
    vision::{Digits, army, crop},
};
use opencv::core::Mat;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Deserialize)]
pub struct ResultLayout {
    pub name: String,
    pub guards: Vec<Target>,
    #[serde(default)]
    pub loot: BTreeMap<String, Region>,
    pub damage: Option<Region>,
    #[serde(default)]
    pub stars: Vec<Region>,
    #[serde(default)]
    pub bonus: BTreeMap<String, Region>,
    pub bonus_included: Option<bool>,
}

#[derive(Default)]
pub struct BattleResult {
    pub loot: Loot,
    pub damage: Option<u8>,
    pub stars: Option<u8>,
}

impl ResultLayout {
    pub fn load(&mut self, root: &Path, resolution: Resolution) -> Result<()> {
        if self.name.is_empty() || self.name.len() > 80 || !(2..=8).contains(&self.guards.len()) {
            return Err(Error::Calibration(
                "Result layouts require a name and independent guards".into(),
            ));
        }
        for (index, guard) in self.guards.iter().enumerate() {
            let Region(x, y, w, h) = guard
                .region
                .ok_or_else(|| Error::Calibration("Result guards require fixed regions".into()))?;
            Region(x, y, w, h).validate(resolution)?;
            if guard.max_score > 0.05
                || guard.max_score <= 0.0
                || self.guards[..index].iter().any(|other| {
                    other.region.is_some_and(|Region(a, b, c, d)| {
                        x < a + c && a < x + w && y < b + d && b < y + h
                    })
                })
            {
                return Err(Error::Calibration(
                    "Result guards overlap or are too permissive".into(),
                ));
            }
        }
        for guard in &mut self.guards {
            guard.load(root, resolution)?;
        }
        if self.stars.len() > 6
            || self
                .loot
                .keys()
                .chain(self.bonus.keys())
                .any(|key| !matches!(key.as_str(), "gold" | "elixir" | "dark"))
        {
            return Err(Error::Calibration("Invalid result fields".into()));
        }
        for region in self
            .loot
            .values()
            .chain(self.bonus.values())
            .chain(self.damage.iter())
            .chain(&self.stars)
        {
            region.validate(resolution)?;
        }
        Ok(())
    }
}

pub fn read(layouts: &[ResultLayout], image: &Mat, digits: &Digits) -> Result<BattleResult> {
    let mut selected = None;
    for layout in layouts {
        let mut matches = true;
        for guard in &layout.guards {
            if guard.locate(image)?.is_none() {
                matches = false;
                break;
            }
        }
        if matches {
            if selected.is_some() {
                return Ok(BattleResult::default());
            }
            selected = Some(layout);
        }
    }
    let Some(layout) = selected else {
        return Ok(BattleResult::default());
    };
    let number = |region: &Region| {
        crop(image, *region)
            .ok()
            .and_then(|patch| digits.number(&patch, 0.75).ok())
            .and_then(|value| u32::try_from(value).ok())
    };
    let earned = |key: &str| {
        let base = layout.loot.get(key).and_then(number)?;
        match (layout.bonus.get(key), layout.bonus_included) {
            (Some(region), Some(false)) => base.checked_add(number(region)?),
            (Some(_), None) => None,
            _ => Some(base),
        }
    };
    Ok(BattleResult {
        loot: Loot {
            gold: earned("gold"),
            elixir: earned("elixir"),
            dark: earned("dark"),
        },
        damage: layout
            .damage
            .and_then(|region| crop(image, region).ok())
            .and_then(|patch| digits.percent(&patch).ok()),
        stars: if layout.stars.is_empty() {
            None
        } else {
            army::stars(image, &layout.stars).ok()
        },
    })
}
