use crate::{
    Error, Result,
    calibration::{Region, Resolution, Screen},
    device::Point,
    vision::{army::Layout, target::Template},
};
use opencv::{core::Mat, prelude::*};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
pub struct Profile {
    pub digits: PathBuf,
    pub resolution: Resolution,
    pub screens: BTreeMap<Screen, ScreenSpec>,
    pub zoom_out: Option<Zoom>,
    pub speed_button: Option<Target>,
    pub capacity: BTreeMap<String, BTreeMap<String, Capacity>>,
    pub cart_text: Option<Region>,
    pub walls: BTreeMap<String, super::walls::Spec>,
}
#[derive(Deserialize)]
pub struct ScreenSpec {
    #[serde(default)]
    pub result_readings: Vec<super::results::ResultLayout>,
    #[serde(default)]
    pub actions: BTreeMap<String, Point>,
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
    #[serde(default)]
    pub loot: BTreeMap<String, Region>,
    pub damage: Option<Region>,
    pub trophies: Option<Region>,
    #[serde(default)]
    pub stars: Vec<Region>,
    pub deploy: Option<Point>,
    pub auto: Option<Layout>,
    pub sweep: Option<Sweep>,
}
#[derive(Deserialize)]
pub struct Sweep {
    pub slots: Vec<Point>,
    pub deploy: Point,
}
#[derive(Deserialize)]
pub struct Zoom {
    pub center: Point,
    pub gap: [u32; 2],
    pub pinches: u8,
    pub battle_pinches: Option<u8>,
    pub selection: Option<Target>,
}
#[derive(Deserialize)]
pub struct Capacity {
    pub bar: Region,
    pub tap: Point,
    #[serde(rename = "max")]
    pub maximum: Region,
    pub label: Target,
}

#[derive(Deserialize)]
pub struct Target {
    pub file: PathBuf,
    pub max_score: f64,
    #[serde(default = "scales")]
    pub scales: Vec<f64>,
    pub region: Option<Region>,
    #[serde(default)]
    pub offset: [i32; 2],
    #[serde(default)]
    pub avoid: Vec<Region>,
    pub pan: Option<[Point; 2]>,
    #[serde(default = "one")]
    pub pan_attempts: u8,
    #[serde(default)]
    pub search_first: bool,
    #[serde(default)]
    alt_files: Vec<Alternative>,
    #[serde(skip)]
    looks: Vec<(Template, [i32; 2])>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Alternative {
    File(PathBuf),
    Offset { file: PathBuf, offset: [i32; 2] },
}
fn scales() -> Vec<f64> {
    vec![1.0]
}
fn one() -> u8 {
    1
}
impl Target {
    pub fn load(&mut self, root: &Path, resolution: Resolution) -> Result<()> {
        if let Some(region) = self.region {
            region.validate(resolution)?;
        }
        if let Some(pan) = self.pan {
            for point in pan {
                point.validate(resolution)?;
            }
        }
        for region in &self.avoid {
            region.validate(resolution)?;
        }
        if self.pan_attempts > 10
            || self.alt_files.len() > 8
            || self.offset.iter().any(|v| v.unsigned_abs() > 8192)
        {
            return Err(Error::Calibration("Invalid target search".into()));
        }
        self.looks.push((
            Template::load(root, &self.file, &self.scales, self.max_score)?,
            self.offset,
        ));
        for alt in &self.alt_files {
            let (file, offset) = match alt {
                Alternative::File(file) => (file, [0, 0]),
                Alternative::Offset { file, offset } => (file, *offset),
            };
            if offset.iter().any(|v| v.unsigned_abs() > 8192) {
                return Err(Error::Calibration("Invalid target offset".into()));
            }
            self.looks.push((
                Template::load(root, file, &self.scales, self.max_score)?,
                offset,
            ));
        }
        Ok(())
    }
    pub fn locate(&self, image: &Mat) -> Result<Option<Point>> {
        self.locate_with_hint(image, None)
    }
    pub fn locate_near(&self, image: &Mat, point: Point) -> Result<Option<Point>> {
        self.locate_with_hint(image, Some(point))
    }
    fn locate_with_hint(&self, image: &Mat, hint: Option<Point>) -> Result<Option<Point>> {
        let resolution = Resolution(image.cols() as u32, image.rows() as u32);
        if let Some(point) = hint {
            point.validate(resolution)?;
        }
        let region = self
            .region
            .unwrap_or(Region(0, 0, resolution.0, resolution.1));
        for (template, offset) in &self.looks {
            let hit = if let Some(point) = hint {
                template.find_near(image, region, point, *offset)?
            } else {
                template.find(image, region)?
            };
            if let Some(hit) = hit {
                let point = hit.point(*offset, resolution)?;
                if self.avoid.iter().any(|Region(x, y, w, h)| {
                    point.0 >= *x && point.0 < x + w && point.1 >= *y && point.1 < y + h
                }) {
                    return Ok(None);
                }
                return Ok(Some(point));
            }
        }
        Ok(None)
    }
}
impl Profile {
    pub fn load(root: &Path) -> Result<Self> {
        let mut profile: Self = serde_json::from_slice(&std::fs::read(root.join("config.json"))?)?;
        let res = profile.resolution;
        res.validate()?;
        for spec in profile.screens.values_mut() {
            if spec.result_readings.len() > 8 {
                return Err(Error::Calibration("Too many result layouts".into()));
            }
            for layout in &mut spec.result_readings {
                layout.load(root, res)?;
            }
            for point in spec.actions.values().chain(spec.deploy.iter()) {
                point.validate(res)?;
            }
            for region in spec
                .loot
                .values()
                .chain(spec.damage.iter())
                .chain(spec.trophies.iter())
                .chain(spec.stars.iter())
            {
                region.validate(res)?;
            }
            for target in spec.targets.values_mut() {
                target.load(root, res)?;
            }
            if let Some(layout) = &mut spec.auto {
                layout.deploy = layout.deploy.or(spec.deploy);
                layout.validate(res)?;
            }
            if let Some(sweep) = &spec.sweep {
                sweep.deploy.validate(res)?;
                if sweep.slots.len() > 20 {
                    return Err(Error::Calibration("Too many sweep slots".into()));
                }
                for slot in &sweep.slots {
                    slot.validate(res)?;
                }
            }
        }
        for resources in profile.capacity.values_mut() {
            for capacity in resources.values_mut() {
                capacity.bar.validate(res)?;
                capacity.maximum.validate(res)?;
                capacity.tap.validate(res)?;
                capacity.label.load(root, res)?;
            }
        }
        if let Some(zoom) = &mut profile.zoom_out {
            zoom.center.validate(res)?;
            if zoom.pinches > 10
                || zoom.battle_pinches.is_some_and(|n| n > 10)
                || zoom.gap[0] <= zoom.gap[1]
                || zoom.gap[0] > res.0
                || zoom.gap[1] == 0
            {
                return Err(Error::Calibration("Invalid zoom calibration".into()));
            }
            if let Some(selection) = &mut zoom.selection {
                selection.load(root, res)?;
            }
        }
        if let Some(target) = &mut profile.speed_button {
            target.load(root, res)?;
        }
        if let Some(region) = profile.cart_text {
            region.validate(res)?;
        }
        for spec in profile.walls.values_mut() {
            spec.load(root, res)?;
        }
        Ok(profile)
    }
    pub fn screen(&self, screen: Screen) -> Result<&ScreenSpec> {
        self.screens
            .get(&screen)
            .ok_or_else(|| Error::Calibration(format!("Screen {screen:?} is not calibrated")))
    }
    pub fn action(&self, screen: Screen, name: &str, image: &Mat) -> Result<Point> {
        let spec = self.screen(screen)?;
        if let Some(target) = spec.targets.get(name) {
            return target.locate(image)?.ok_or(Error::Unreadable);
        }
        spec.actions
            .get(name)
            .copied()
            .ok_or_else(|| Error::Calibration(format!("Missing {screen:?} {name} action")))
    }
}
