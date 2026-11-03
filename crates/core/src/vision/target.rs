use super::{crop, read_image};
use crate::{
    Error, Result,
    calibration::{Region, Resolution, asset_path},
    device::Point,
};
use opencv::{
    core::{self, Mat, Size},
    imgproc,
    prelude::*,
};
use std::path::Path;

pub struct Template {
    image: Mat,
    scales: Vec<f64>,
    threshold: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    pub score: f64,
}
impl Hit {
    pub fn point(self, offset: [i32; 2], resolution: Resolution) -> Result<Point> {
        let x = self.x + self.width / 2 + (f64::from(offset[0]) * self.scale).round() as i32;
        let y = self.y + self.height / 2 + (f64::from(offset[1]) * self.scale).round() as i32;
        let point = Point(
            u32::try_from(x).map_err(|_| Error::InputGuard)?,
            u32::try_from(y).map_err(|_| Error::InputGuard)?,
        );
        point.validate(resolution)?;
        Ok(point)
    }
}
impl Template {
    pub fn load(root: &Path, file: &Path, scales: &[f64], threshold: f64) -> Result<Self> {
        if scales.is_empty()
            || scales.len() > 32
            || scales
                .iter()
                .any(|n| !n.is_finite() || !(0.5..=2.0).contains(n))
            || !threshold.is_finite()
            || !(0.0..=0.2).contains(&threshold)
        {
            return Err(Error::Calibration(
                "Invalid target matching parameters".into(),
            ));
        }
        let image = read_image(&asset_path(root, file)?)?;
        let mut deviation = core::Scalar::default();
        core::mean_std_dev(
            &image,
            &mut core::no_array(),
            &mut deviation,
            &core::no_array(),
        )?;
        if deviation[0] + deviation[1] + deviation[2] < 1.0 {
            return Err(Error::Calibration("Target template has no contrast".into()));
        }
        Ok(Self {
            image,
            scales: scales.to_vec(),
            threshold,
        })
    }
    pub fn find(&self, image: &Mat, region: Region) -> Result<Option<Hit>> {
        Ok(self.find_all(image, region)?.into_iter().next())
    }
    pub fn find_all(&self, image: &Mat, region: Region) -> Result<Vec<Hit>> {
        let source = crop(image, region)?;
        let mut hits = Vec::new();
        for &scale in &self.scales {
            let mut template = Mat::default();
            imgproc::resize(
                &self.image,
                &mut template,
                Size::default(),
                scale,
                scale,
                imgproc::INTER_LINEAR,
            )?;
            if template.rows() > source.rows() || template.cols() > source.cols() {
                continue;
            }
            let mut scores = Mat::default();
            imgproc::match_template_def(
                &source,
                &template,
                &mut scores,
                imgproc::TM_SQDIFF_NORMED,
            )?;
            for y in 0..scores.rows() {
                for x in 0..scores.cols() {
                    let score = f64::from(*scores.at_2d::<f32>(y, x)?);
                    if score.is_finite() && score <= self.threshold {
                        if hits.len() >= 16384 {
                            return Err(Error::Unreadable);
                        }
                        hits.push(Hit {
                            x: x + region.0 as i32,
                            y: y + region.1 as i32,
                            width: template.cols(),
                            height: template.rows(),
                            scale,
                            score,
                        });
                    }
                }
            }
        }
        hits.sort_by(|a, b| a.score.total_cmp(&b.score));
        let mut found: Vec<Hit> = Vec::new();
        for hit in hits {
            if found.iter().all(|old| {
                (hit.x - old.x).abs() >= hit.width.min(old.width)
                    || (hit.y - old.y).abs() >= hit.height.min(old.height)
            }) {
                found.push(hit);
            }
        }
        Ok(found)
    }
}
