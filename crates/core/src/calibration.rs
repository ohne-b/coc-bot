use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    Home,
    HomeMenu,
    HomeArmy,
    HomeScout,
    HomeBattle,
    HomeResult,
    Builder,
    BuilderMenu,
    BuilderScout,
    BuilderBattle,
    BuilderStage2,
    BuilderBattle2,
    BuilderResult,
    BuilderCart,
    StarBonus,
    BuilderStarBonus,
    RewardChest,
    RewardCard,
    LayoutBlocked,
    League,
    Welcome,
    Idle,
    Disconnected,
    OtherDevice,
    BuildersBusy,
    UpdateOffer,
    SurrenderConfirm,
    BuildList,
    WallSelected,
    WallConfirm,
    BuilderWallConfirm,
    SearchFailed,
    GemOffer,
    Loading,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Resolution(pub u32, pub u32);

impl Resolution {
    pub fn validate(self) -> Result<()> {
        if self.0 == 0 || self.1 == 0 || self.0 > 8192 || self.1 > 8192 {
            return Err(invalid(
                "Resolution must be between 1 and 8192 pixels per side",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Region(pub u32, pub u32, pub u32, pub u32);

impl Region {
    pub fn validate(self, resolution: Resolution) -> Result<()> {
        let Self(x, y, width, height) = self;
        if width == 0
            || height == 0
            || x.checked_add(width)
                .is_none_or(|right| right > resolution.0)
            || y.checked_add(height)
                .is_none_or(|bottom| bottom > resolution.1)
        {
            return Err(invalid("Region is outside the calibrated screenshot"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Anchor {
    pub file: PathBuf,
    pub roi: Option<Region>,
    pub search: Option<Region>,
    #[serde(default)]
    pub margin: u32,
    #[serde(default = "default_scales")]
    pub scales: Vec<f64>,
    #[serde(default = "default_score")]
    pub max_score: f64,
}

fn default_scales() -> Vec<f64> {
    vec![1.0]
}
fn default_score() -> f64 {
    0.02
}

impl Anchor {
    pub fn region(&self) -> Result<Region> {
        match (self.roi, self.search) {
            (Some(region), None) | (None, Some(region)) => Ok(region),
            _ => Err(invalid(
                "An anchor needs exactly one region or search region",
            )),
        }
    }

    fn validate(&self, resolution: Resolution) -> Result<()> {
        self.region()?.validate(resolution)?;
        if self.margin > 40
            || self.scales.is_empty()
            || self.scales.len() > 32
            || self
                .scales
                .iter()
                .any(|s| !s.is_finite() || !(0.5..=2.0).contains(s))
            || !self.max_score.is_finite()
            || self.max_score <= 0.0
            || self.max_score > 0.05
            || (self.roi.is_some() && (self.scales != [1.0] || self.max_score != 0.02))
            || (self.search.is_some() && self.margin != 0)
        {
            return Err(invalid("Anchor tolerance or scales are invalid"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScreenProfile {
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub alt_anchors: Vec<Vec<Anchor>>,
    #[serde(default)]
    pub overrides: BTreeSet<Screen>,
}

/// The recognition part of the shared profile; device settings are never loaded here.
#[derive(Clone, Debug, Deserialize)]
pub struct Calibration {
    pub calibration_language: String,
    pub resolution: Resolution,
    pub screens: BTreeMap<Screen, ScreenProfile>,
    #[serde(default)]
    pub calibration_observed_screens: Option<BTreeSet<Screen>>,
    #[serde(default)]
    pub calibration_pending_screens: BTreeSet<Screen>,
}

impl Calibration {
    pub fn load(path: &Path) -> Result<Self> {
        if std::fs::metadata(path)?.len() > 4 * 1024 * 1024 {
            return Err(invalid("Calibration exceeds 4 MiB"));
        }
        let profile: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<()> {
        self.resolution.validate()?;
        if self.calibration_language != "en" {
            return Err(invalid("Only English calibration is supported"));
        }
        if self.screens.is_empty() {
            return Err(invalid("Calibration has no screens"));
        }
        if let Some(observed) = &self.calibration_observed_screens {
            let all: BTreeSet<_> = self.screens.keys().copied().collect();
            if observed.is_empty()
                || !observed.is_disjoint(&self.calibration_pending_screens)
                || *observed != all
            {
                return Err(invalid(
                    "Only observed screens may have English calibration templates",
                ));
            }
        } else if !self.calibration_pending_screens.is_empty() {
            return Err(invalid("Pending screens require an observed-screen list"));
        }
        for (screen, spec) in &self.screens {
            if spec.overrides.contains(screen)
                || spec.overrides.iter().any(|s| !self.screens.contains_key(s))
            {
                return Err(invalid(
                    "A screen override references itself or a missing screen",
                ));
            }
            if spec.alt_anchors.len() > 16 {
                return Err(invalid("Too many alternative anchors"));
            }
            for anchors in std::iter::once(&spec.anchors).chain(&spec.alt_anchors) {
                if !(2..=16).contains(&anchors.len()) {
                    return Err(invalid("Each screen needs 2 to 16 distinct anchors"));
                }
                for anchor in anchors {
                    anchor.validate(self.resolution)?;
                }
                let boxes: Vec<_> = anchors.iter().filter_map(|anchor| anchor.roi).collect();
                for (i, &Region(x, y, w, h)) in boxes.iter().enumerate() {
                    for &Region(other_x, other_y, other_w, other_h) in &boxes[..i] {
                        if x < other_x + other_w
                            && other_x < x + w
                            && y < other_y + other_h
                            && other_y < y + h
                        {
                            return Err(invalid("Fixed anchor regions must not overlap"));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn is_observed(&self, screen: Screen) -> bool {
        self.calibration_observed_screens
            .as_ref()
            .is_none_or(|set| set.contains(&screen))
    }
}

pub fn asset_path(root: &Path, file: &Path) -> Result<PathBuf> {
    if file.as_os_str().is_empty()
        || file
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid(
            "Calibration assets must use relative paths without traversal",
        ));
    }
    let root = root.canonicalize()?;
    let path = root.join(file).canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(invalid(
            "Calibration asset is outside its directory or is not a file",
        ));
    }
    Ok(path)
}

fn invalid(message: &str) -> Error {
    Error::Calibration(message.into())
}
