use super::profile::Target;
use crate::{
    Error, Result,
    calibration::{Region, Resolution},
    telemetry::{Loot, Village},
    vision::{Digits, army, crop},
};
use opencv::{
    core::{Mat, Vec3b},
    imgproc,
    prelude::*,
};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Deserialize)]
pub struct ResultLayout {
    pub name: String,
    pub guards: Vec<Target>,
    #[serde(default)]
    pub loot: BTreeMap<String, Region>,
    pub damage: Option<Region>,
    pub damage_color: Option<DamageColor>,
    #[serde(default)]
    pub stars: Vec<Region>,
    #[serde(default)]
    pub empty_stars: Vec<Target>,
    #[serde(default)]
    pub bonus: BTreeMap<String, Region>,
    pub bonus_included: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageColor {
    Green,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct BattleResult {
    pub loot: Loot,
    pub displayed_loot: Loot,
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
        if !self.empty_stars.is_empty() && self.empty_stars.len() != self.stars.len() {
            return Err(Error::Calibration(
                "Empty-star proofs must match the configured star slots".into(),
            ));
        }
        for (proof, region) in self.empty_stars.iter_mut().zip(&self.stars) {
            if proof.region != Some(*region) || proof.max_score != 0.02 || proof.scales != [1.0] {
                return Err(Error::Calibration(
                    "Empty-star proofs require exact slot regions and strict matching".into(),
                ));
            }
            proof.load(root, resolution)?;
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

pub fn read(
    layouts: &[ResultLayout],
    image: &Mat,
    digits: &Digits,
    village: Village,
) -> Result<BattleResult> {
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
        displayed_loot: Loot {
            gold: layout.loot.get("gold").and_then(number),
            elixir: layout.loot.get("elixir").and_then(number),
            dark: layout.loot.get("dark").and_then(number),
        },
        loot: Loot {
            gold: earned("gold"),
            elixir: earned("elixir"),
            dark: earned("dark"),
        },
        damage: layout
            .damage
            .and_then(|region| crop(image, region).ok())
            .and_then(|patch| match layout.damage_color {
                Some(DamageColor::Green) => crate::vision::green_text(&patch).ok(),
                None => Some(patch),
            })
            .and_then(|patch| {
                digits
                    .percent_up_to(
                        &patch,
                        if village == Village::Builder {
                            200
                        } else {
                            100
                        },
                    )
                    .ok()
            }),
        stars: if layout.stars.is_empty() {
            None
        } else if village == Village::Builder {
            builder_stars(image, &layout.stars, &layout.empty_stars).ok()
        } else {
            army::stars(image, &layout.stars).ok()
        },
    })
}

/// Count the visible result stars, including their silver animation state; never infer six.
pub fn builder_stars(image: &Mat, regions: &[Region], empty: &[Target]) -> Result<u8> {
    if regions.len() != 3 {
        return Err(Error::Unreadable);
    }
    let mut hsv = Mat::default();
    imgproc::cvt_color_def(image, &mut hsv, imgproc::COLOR_BGR2HSV)?;
    let mut count = 0;
    for (index, &region) in regions.iter().enumerate() {
        let patch = crop(&hsv, region)?;
        let mut visible = 0;
        let mut gold = 0;
        let mut silver = 0;
        for p in patch.data_typed::<Vec3b>()? {
            visible += usize::from(p[2] > 0);
            gold += usize::from((8..=30).contains(&p[0]) && p[1] >= 120 && p[2] >= 150);
            silver += usize::from(
                (p[1] <= 50 || (85..=120).contains(&p[0]) && p[1] <= 140) && p[2] >= 180,
            );
        }
        let total = patch.total();
        if visible * 5 < total * 4 {
            return Err(Error::Unreadable);
        }
        if gold * 4 >= total || silver * 4 >= total {
            count += 1;
        } else if empty
            .get(index)
            .map(|proof| proof.locate(image))
            .transpose()?
            .flatten()
            .is_none()
        {
            return Err(Error::Unreadable);
        }
    }
    Ok(count)
}
