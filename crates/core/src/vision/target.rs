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
    pub fn find_near(
        &self,
        image: &Mat,
        region: Region,
        point: Point,
        offset: [i32; 2],
    ) -> Result<Option<Hit>> {
        let resolution = Resolution(image.cols() as u32, image.rows() as u32);
        region.validate(resolution)?;
        point.validate(resolution)?;
        let scale = self.scales.iter().copied().fold(0.0, f64::max);
        let radius = |size: i32, offset: i32| {
            ((f64::from(size) / 2.0 + f64::from(offset.unsigned_abs())) * scale).ceil() as u32 + 8
        };
        let rx = radius(self.image.cols(), offset[0]);
        let ry = radius(self.image.rows(), offset[1]);
        let x = point.0.saturating_sub(rx).max(region.0);
        let y = point.1.saturating_sub(ry).max(region.1);
        let right = point
            .0
            .saturating_add(rx)
            .saturating_add(1)
            .min(region.0 + region.2);
        let bottom = point
            .1
            .saturating_add(ry)
            .saturating_add(1)
            .min(region.1 + region.3);
        if right <= x || bottom <= y {
            return Ok(None);
        }
        self.find(image, Region(x, y, right - x, bottom - y))
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
            let mut mask = Mat::default();
            let threshold = self.threshold as f32;
            let threshold = if f64::from(threshold) > self.threshold {
                threshold.next_down()
            } else {
                threshold
            };
            core::in_range(
                &scores,
                &core::Scalar::all(f64::from(f32::MIN)),
                &core::Scalar::all(f64::from(threshold)),
                &mut mask,
            )?;
            if hits.len() + core::count_non_zero(&mask)? as usize > 16384 {
                return Err(Error::Unreadable);
            }
            let mut points = core::Vector::<core::Point>::new();
            core::find_non_zero(&mask, &mut points)?;
            for point in points {
                hits.push(Hit {
                    x: point.x + region.0 as i32,
                    y: point.y + region.1 as i32,
                    width: template.cols(),
                    height: template.rows(),
                    scale,
                    score: f64::from(*scores.at_2d::<f32>(point.y, point.x)?),
                });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_score_filter_preserves_exact_threshold_boundaries() {
        let mut template = Template {
            image: Mat::from_slice_2d(&[[0_u8, 50, 100], [150, 200, 250]]).unwrap(),
            scales: vec![1.0],
            threshold: 0.2,
        };
        let source = Mat::from_slice_2d(&[[5_u8, 50, 100], [150, 200, 250]]).unwrap();
        let region = Region(0, 0, 3, 2);
        let score = template.find(&source, region).unwrap().unwrap().score;
        assert!(score > 0.0 && score < 0.2);
        template.threshold = score.next_down();
        assert!(template.find(&source, region).unwrap().is_none());
        template.threshold = score;
        assert_eq!(
            template.find(&source, region).unwrap().unwrap().score,
            score
        );
    }

    #[test]
    fn repetitive_matches_still_exceed_the_hit_limit() {
        let rows: Vec<Vec<u8>> = (0..260)
            .map(|y| {
                (0..260)
                    .map(|x| if (x + y) % 2 == 0 { 0 } else { 255 })
                    .collect()
            })
            .collect();
        let source = Mat::from_slice_2d(&rows).unwrap();
        let template = Template {
            image: Mat::from_slice_2d(&[[0_u8, 255], [255, 0]]).unwrap(),
            scales: vec![1.0],
            threshold: 0.01,
        };
        assert!(matches!(
            template.find_all(&source, Region(0, 0, 260, 260)),
            Err(Error::Unreadable)
        ));
    }
}
