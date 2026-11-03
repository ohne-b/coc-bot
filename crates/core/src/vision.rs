use std::{collections::BTreeSet, path::Path};

use opencv::{
    core::{self, Mat, MatTraitConst, Rect, Scalar, Size},
    imgcodecs, imgproc,
};
use serde::Serialize;

use crate::{
    Error, Result,
    calibration::{Anchor, Calibration, Region, Resolution, Screen, asset_path},
};

pub mod army;
mod digits;
pub mod target;
pub use digits::Digits;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "screens", rename_all = "snake_case")]
pub enum Detection {
    Known(Screen),
    Unknown,
    Ambiguous(Vec<Screen>),
}

struct LoadedAnchor {
    spec: Anchor,
    template: Mat,
}
struct LoadedScreen {
    name: Screen,
    groups: Vec<Vec<LoadedAnchor>>,
    overrides: BTreeSet<Screen>,
}

pub struct Detector {
    resolution: Resolution,
    screens: Vec<LoadedScreen>,
}

impl Detector {
    pub fn load(profile: &Calibration, root: &Path) -> Result<Self> {
        profile.validate()?;
        let mut screens = Vec::new();
        for (&name, spec) in &profile.screens {
            // Uncaptured screens cannot authorize input.
            if !profile.is_observed(name) {
                continue;
            }
            let groups = std::iter::once(&spec.anchors)
                .chain(&spec.alt_anchors)
                .map(|group| {
                    group
                        .iter()
                        .map(|spec| {
                            let template = read_image(&asset_path(root, &spec.file)?)?;
                            let Region(_, _, width, height) = spec.region()?;
                            let mut mean = Scalar::default();
                            let mut deviation = Scalar::default();
                            core::mean_std_dev(
                                &template,
                                &mut mean,
                                &mut deviation,
                                &core::no_array(),
                            )?;
                            if (spec.roi.is_some()
                                && (template.cols() != width as i32
                                    || template.rows() != height as i32))
                                || template.cols() > width as i32
                                || template.rows() > height as i32
                                || deviation.0[..3].iter().all(|s| *s < 5.0)
                            {
                                return Err(Error::Calibration(format!(
                                    "Anchor is flat or larger than its region: {}",
                                    spec.file.display()
                                )));
                            }
                            Ok(LoadedAnchor {
                                spec: spec.clone(),
                                template,
                            })
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?;
            screens.push(LoadedScreen {
                name,
                groups,
                overrides: spec.overrides.clone(),
            });
        }
        Ok(Self {
            resolution: profile.resolution,
            screens,
        })
    }

    pub fn detect(&self, image: &Mat) -> Result<Detection> {
        check_frame(image, self.resolution)?;
        let mut matched = Vec::new();
        let mut overridden = BTreeSet::new();
        for screen in &self.screens {
            let found = self.matches(image, screen.name)?;
            if found {
                matched.push(screen.name);
                overridden.extend(screen.overrides.iter().copied());
            }
        }
        matched.retain(|name| !overridden.contains(name));
        Ok(match matched.as_slice() {
            [] => Detection::Unknown,
            [name] => Detection::Known(*name),
            _ => Detection::Ambiguous(matched),
        })
    }

    pub fn matches(&self, image: &Mat, name: Screen) -> Result<bool> {
        check_frame(image, self.resolution)?;
        let Some(screen) = self.screens.iter().find(|screen| screen.name == name) else {
            return Ok(false);
        };
        for group in &screen.groups {
            let mut matches = true;
            for anchor in group {
                let score = anchor.score(image, self.resolution)?;
                if !score.is_finite() || score > anchor.spec.max_score {
                    matches = false;
                    break;
                }
            }
            if matches {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl LoadedAnchor {
    fn score(&self, image: &Mat, resolution: Resolution) -> Result<f64> {
        let Region(x, y, width, height) = self.spec.region()?;
        let left = x.saturating_sub(self.spec.margin);
        let top = y.saturating_sub(self.spec.margin);
        let right = (x + width + self.spec.margin).min(resolution.0);
        let bottom = (y + height + self.spec.margin).min(resolution.1);
        let region = Mat::roi(
            image,
            Rect::new(
                left as i32,
                top as i32,
                (right - left) as i32,
                (bottom - top) as i32,
            ),
        )?
        .try_clone()?;
        let mut best = f64::INFINITY;
        for scale in &self.spec.scales {
            let mut scaled = Mat::default();
            let template = if *scale == 1.0 {
                &self.template
            } else {
                imgproc::resize(
                    &self.template,
                    &mut scaled,
                    Size::default(),
                    *scale,
                    *scale,
                    imgproc::INTER_LINEAR,
                )?;
                &scaled
            };
            if template.rows() <= region.rows() && template.cols() <= region.cols() {
                best = best.min(match_min(&region, template)?);
            }
        }
        Ok(best)
    }
}

pub(crate) fn match_min(region: &Mat, template: &Mat) -> Result<f64> {
    let mut result = Mat::default();
    // Keep the original coarse search and bounded full-resolution refinement.
    if region.total() >= 16 * template.total() && template.rows() >= 2 && template.cols() >= 2 {
        let mut small = Mat::default();
        let mut small_template = Mat::default();
        imgproc::resize(
            region,
            &mut small,
            Size::default(),
            0.5,
            0.5,
            imgproc::INTER_AREA,
        )?;
        imgproc::resize(
            template,
            &mut small_template,
            Size::default(),
            0.5,
            0.5,
            imgproc::INTER_AREA,
        )?;
        imgproc::match_template_def(
            &small,
            &small_template,
            &mut result,
            imgproc::TM_SQDIFF_NORMED,
        )?;
        let mut location = core::Point::default();
        core::min_max_loc(
            &result,
            None,
            None,
            Some(&mut location),
            None,
            &core::no_array(),
        )?;
        let left = (2 * location.x - 4).max(0);
        let top = (2 * location.y - 4).max(0);
        let refined = Mat::roi(
            region,
            Rect::new(
                left,
                top,
                (template.cols() + 8).min(region.cols() - left),
                (template.rows() + 8).min(region.rows() - top),
            ),
        )?;
        imgproc::match_template_def(&refined, template, &mut result, imgproc::TM_SQDIFF_NORMED)?;
    } else {
        imgproc::match_template_def(region, template, &mut result, imgproc::TM_SQDIFF_NORMED)?;
    }
    let mut minimum = f64::INFINITY;
    core::min_max_loc(
        &result,
        Some(&mut minimum),
        None,
        None,
        None,
        &core::no_array(),
    )?;
    Ok(minimum)
}

pub fn read_image(path: &Path) -> Result<Mat> {
    let bytes = std::fs::read(path)?;
    let image = imgcodecs::imdecode(
        &core::Vector::<u8>::from_slice(&bytes),
        imgcodecs::IMREAD_COLOR,
    )?;
    if image.empty() {
        return Err(Error::Frame);
    }
    Ok(image)
}

pub fn crop(image: &Mat, region: Region) -> Result<Mat> {
    region.validate(Resolution(image.cols() as u32, image.rows() as u32))?;
    let Region(x, y, w, h) = region;
    Ok(Mat::roi(image, Rect::new(x as i32, y as i32, w as i32, h as i32))?.try_clone()?)
}

pub fn decode_frame(bytes: &[u8], resolution: Resolution) -> Result<Mat> {
    resolution.validate()?;
    if bytes.len() < 12 {
        return Err(Error::Frame);
    }
    let read = |offset| {
        u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("checked header"),
        )
    };
    if Resolution(read(0), read(4)) != resolution {
        return Err(Error::Resolution);
    }
    if read(8) != 1 {
        return Err(Error::Frame);
    }
    let pixels = resolution.0 as usize * resolution.1 as usize * 4;
    let header = bytes
        .len()
        .checked_sub(pixels)
        .filter(|h| [12, 16].contains(h))
        .ok_or(Error::Frame)?;
    let data = Mat::from_slice(&bytes[header..])?;
    let rgba = data.reshape(4, resolution.1 as i32)?;
    let mut bgr = Mat::default();
    imgproc::cvt_color_def(&rgba, &mut bgr, imgproc::COLOR_RGBA2BGR)?;
    Ok(bgr)
}

fn check_frame(image: &Mat, resolution: Resolution) -> Result<()> {
    if image.cols() != resolution.0 as i32 || image.rows() != resolution.1 as i32 {
        return Err(Error::Resolution);
    }
    if image.typ() != core::CV_8UC3 {
        return Err(Error::Frame);
    }
    Ok(())
}
