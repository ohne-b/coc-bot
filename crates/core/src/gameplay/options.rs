use crate::{
    Error, Result,
    calibration::{Resolution, Screen},
    telemetry::{Resource, Village},
    vision::army::Layout,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    #[default]
    Stack,
    Ring,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub villages: Vec<Village>,
    pub minimum_gold: u32,
    pub minimum_elixir: u32,
    pub minimum_dark: u32,
    pub targets: Vec<Resource>,
    pub until_full: bool,
    pub collect_cart: bool,
    pub attacks: u32,
    pub minutes: u32,
    pub searches: u32,
    pub strategy: Strategy,
    pub earthquake: bool,
    pub builder_stack: bool,
    pub surrender_after: u32,
    pub surrender_damage: u8,
    pub surrender_stars: u8,
    pub trophy_floor: u32,
    pub recoveries: u8,
    pub wall_upgrades: u32,
    pub gold_reserve: u32,
    pub elixir_reserve: u32,
    pub wall_farm_threshold: u32,
    pub layouts: BTreeMap<Screen, Layout>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            villages: vec![Village::Home],
            minimum_gold: 0,
            minimum_elixir: 0,
            minimum_dark: 0,
            targets: vec![Resource::Gold, Resource::Elixir],
            until_full: false,
            collect_cart: true,
            attacks: 0,
            minutes: 0,
            searches: 50,
            strategy: Strategy::Stack,
            earthquake: false,
            builder_stack: false,
            surrender_after: 0,
            surrender_damage: 0,
            surrender_stars: 0,
            trophy_floor: 0,
            recoveries: 3,
            wall_upgrades: 0,
            gold_reserve: 1_000_000,
            elixir_reserve: 1_000_000,
            wall_farm_threshold: 20_000_000,
            layouts: BTreeMap::new(),
        }
    }
}
impl Options {
    pub fn validate(&self, resolution: Resolution) -> Result<()> {
        if self.villages.is_empty()
            || self.villages.len() > 2
            || (self.villages.len() == 2 && self.villages[0] == self.villages[1])
            || self.targets.len() > 3
            || (self.until_full && self.targets.is_empty())
            || self
                .targets
                .iter()
                .enumerate()
                .any(|(i, v)| self.targets[..i].contains(v))
            || self.attacks > 1_000_000
            || self.searches > 1_000_000
            || self.minutes > 10_080
            || self.wall_upgrades > 1_000_000
            || self.surrender_after > 240
            || self.surrender_damage > 100
            || self.surrender_stars > 3
            || self.trophy_floor > 20_000
            || self.recoveries > 20
            || [
                self.minimum_gold,
                self.minimum_elixir,
                self.minimum_dark,
                self.gold_reserve,
                self.elixir_reserve,
                self.wall_farm_threshold,
            ]
            .into_iter()
            .any(|v| v > 1_000_000_000)
        {
            return Err(Error::Calibration(
                "Farming settings are outside the supported range".into(),
            ));
        }
        if self.villages == [Village::Builder] && self.targets == [Resource::Dark] {
            return Err(Error::Calibration("Builder Base has no Dark Elixir".into()));
        }
        for (screen, layout) in &self.layouts {
            if !matches!(
                screen,
                Screen::HomeScout | Screen::BuilderScout | Screen::BuilderStage2
            ) {
                return Err(Error::Calibration(
                    "Layout is not a deployment screen".into(),
                ));
            }
            layout.validate(resolution)?;
            if (*screen == Screen::HomeScout) != layout.points.is_empty() {
                return Err(Error::Calibration(
                    "Layout belongs to a different village".into(),
                ));
            }
            if *screen == Screen::HomeScout
                && ((self.strategy == Strategy::Ring && layout.ring.is_empty())
                    || (self.earthquake && layout.quake.is_empty()))
            {
                return Err(Error::Calibration(
                    "The selected strategy needs calibrated deployment points".into(),
                ));
            }
        }
        Ok(())
    }
}
