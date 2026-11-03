use std::path::Path;

use opencv::{
    core::{self, Mat, Rect, Scalar, Size},
    imgcodecs, imgproc,
    prelude::*,
};

use crate::{Error, Result, calibration::asset_path};

const HEIGHT: i32 = 24;
const WIDTH: i32 = 20;

pub struct Digits {
    templates: Vec<(u8, Mat)>,
}

impl Digits {
    pub fn load(root: &Path, folder: &Path) -> Result<Self> {
        let directory = asset_path(root, &folder.join("0.png"))?
            .parent()
            .ok_or(Error::Unreadable)?
            .to_owned();
        let mut templates = Vec::new();
        for digit in 0..=9 {
            let prefix = format!("{digit}-");
            let mut files = vec![folder.join(format!("{digit}.png"))];
            for entry in std::fs::read_dir(&directory)? {
                let name = entry?.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(&prefix) && name.ends_with(".png") {
                    files.push(folder.join(name.as_ref()));
                }
            }
            files.sort();
            for file in files {
                let bytes = std::fs::read(asset_path(root, &file)?)?;
                let image = imgcodecs::imdecode(
                    &core::Vector::<u8>::from_slice(&bytes),
                    imgcodecs::IMREAD_GRAYSCALE,
                )?;
                if image.rows() != HEIGHT || image.cols() != WIDTH {
                    return Err(Error::Calibration(
                        "Digit templates must be 20 by 24 pixels".into(),
                    ));
                }
                let mut deviation = Scalar::default();
                core::mean_std_dev(
                    &image,
                    &mut core::no_array(),
                    &mut deviation,
                    &core::no_array(),
                )?;
                if deviation[0] <= f64::EPSILON {
                    return Err(Error::Calibration(format!(
                        "Digit template is blank: {}",
                        file.display()
                    )));
                }
                let mut normalized = Mat::default();
                image.convert_to(&mut normalized, core::CV_32F, 1.0 / 255.0, 0.0)?;
                templates.push((digit, normalized));
            }
        }
        Ok(Self { templates })
    }

    pub fn number(&self, image: &Mat, minimum_score: f64) -> Result<u64> {
        let glyphs = glyphs(image)?;
        if !(1..=10).contains(&glyphs.len()) {
            return Err(Error::Unreadable);
        }
        let digits = glyphs
            .iter()
            .map(|glyph| {
                self.classify(glyph, minimum_score)?
                    .ok_or(Error::Unreadable)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(value(&digits))
    }

    pub fn card_count(&self, image: &Mat, suffix: bool) -> Result<u32> {
        let mut digits = glyphs(image)?
            .iter()
            .map(|glyph| self.classify(glyph, 0.75))
            .collect::<Result<Vec<_>>>()?;
        if !(2..=4).contains(&digits.len()) {
            return Err(Error::Unreadable);
        }
        let marker = if suffix {
            digits.pop().flatten()
        } else {
            digits.remove(0)
        };
        if marker.is_some() {
            return Err(Error::Unreadable);
        }
        let number = value(
            &digits
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or(Error::Unreadable)?,
        );
        if !(1..=300).contains(&number) {
            return Err(Error::Unreadable);
        }
        Ok(number as u32)
    }

    pub fn percent(&self, image: &Mat) -> Result<u8> {
        self.percent_up_to(image, 100)
    }

    pub fn percent_up_to(&self, image: &Mat, maximum: u8) -> Result<u8> {
        if maximum > 200 {
            return Err(Error::Unreadable);
        }
        let digits = glyphs(image)?
            .iter()
            .map(|glyph| self.classify(glyph, 0.75))
            .collect::<Result<Vec<_>>>()?;
        if !(2..=4).contains(&digits.len()) || digits.last() != Some(&None) {
            return Err(Error::Unreadable);
        }
        let number = value(
            &digits[..digits.len() - 1]
                .iter()
                .copied()
                .collect::<Option<Vec<_>>>()
                .ok_or(Error::Unreadable)?,
        );
        if number > u64::from(maximum) {
            return Err(Error::Unreadable);
        }
        Ok(number as u8)
    }

    pub fn fraction(&self, image: &Mat) -> Result<(u64, u64)> {
        let digits = glyphs(image)?
            .iter()
            .map(|glyph| self.classify(glyph, 0.65))
            .collect::<Result<Vec<_>>>()?;
        if !(3..=21).contains(&digits.len()) {
            return Err(Error::Unreadable);
        }
        let separators: Vec<_> = digits
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.is_none().then_some(i))
            .collect();
        let [split] = separators.as_slice() else {
            return Err(Error::Unreadable);
        };
        if *split == 0 || *split == digits.len() - 1 || *split > 10 || digits.len() - split - 1 > 10
        {
            return Err(Error::Unreadable);
        }
        let current = value(
            &digits[..*split]
                .iter()
                .copied()
                .collect::<Option<Vec<_>>>()
                .ok_or(Error::Unreadable)?,
        );
        let maximum = value(
            &digits[split + 1..]
                .iter()
                .copied()
                .collect::<Option<Vec<_>>>()
                .ok_or(Error::Unreadable)?,
        );
        if maximum == 0 || current > maximum {
            return Err(Error::Unreadable);
        }
        Ok((current, maximum))
    }

    /// Read the final aligned run, excluding labels behind a translucent resource bar.
    pub fn trailing(&self, image: &Mat, minimum_score: f64, max_saturation: u8) -> Result<u64> {
        let components = Components::new(image, max_saturation)?;
        let mut solid = Vec::new();
        for id in 1..components.stats.rows() {
            let rect = components.rect(id)?;
            let mut brightness = 0_u64;
            for y in rect.y..rect.y + rect.height {
                for x in rect.x..rect.x + rect.width {
                    if *components.labels.at_2d::<i32>(y, x)? == id {
                        brightness += components.hsv.at_2d::<core::Vec3b>(y, x)?[2] as u64;
                    }
                }
            }
            let area = *components.stats.at_2d::<i32>(id, imgproc::CC_STAT_AREA)?;
            if brightness as f64 / area as f64 >= 235.0 {
                solid.push((id, rect));
            }
        }
        let tallest = solid
            .iter()
            .map(|(_, rect)| rect.height)
            .max()
            .ok_or(Error::Unreadable)?;
        solid.retain(|(_, rect)| rect.height as f64 >= 0.6 * tallest as f64);
        solid.sort_by_key(|(_, rect)| std::cmp::Reverse(rect.x));
        let reference = solid[0].1;
        let mut canvases = Vec::new();
        let mut last_left = None;
        for (id, rect) in solid {
            if (rect.y - reference.y).abs() > 2 || rect.height < reference.height - 2 {
                continue;
            }
            if last_left.is_some_and(|left| {
                (left - (rect.x + rect.width)) as f64 > 1.2 * reference.height as f64
            }) {
                break;
            }
            let mut glyph = components.glyph(id)?;
            if rect.height > reference.height + 2 {
                // A tooltip border can join a HUD digit below its text line at the crop edge.
                if rect.y + rect.height != image.rows() || rect.height > 2 * reference.height {
                    return Err(Error::Unreadable);
                }
                glyph =
                    Mat::roi(&glyph, Rect::new(0, 0, rect.width, reference.height))?.try_clone()?;
            }
            let pieces = if rect.width as f64 > 1.4 * rect.height as f64 {
                split_glyphs(
                    &glyph,
                    (rect.width as f64 / (rect.height as f64 * 0.95))
                        .round_ties_even()
                        .max(2.0) as i32,
                )?
            } else {
                vec![glyph]
            };
            for piece in pieces.iter().rev() {
                canvases.push(canvas(piece)?);
            }
            last_left = Some(rect.x);
        }
        if !(1..=10).contains(&canvases.len()) {
            return Err(Error::Unreadable);
        }
        let digits = canvases
            .iter()
            .rev()
            .map(|glyph| {
                self.classify(glyph, minimum_score)?
                    .ok_or(Error::Unreadable)
            })
            .collect::<Result<Vec<_>>>()?;
        if digits.len() > 1 && digits[0] == 0 {
            return Err(Error::Unreadable);
        }
        Ok(value(&digits))
    }

    fn classify(&self, glyph: &Mat, minimum_score: f64) -> Result<Option<u8>> {
        if !minimum_score.is_finite() || !(0.65..=1.0).contains(&minimum_score) {
            return Err(Error::Unreadable);
        }
        let mut scores = [-1.0_f64; 10];
        for (digit, template) in &self.templates {
            let mut result = Mat::default();
            imgproc::match_template_def(glyph, template, &mut result, imgproc::TM_CCOEFF_NORMED)?;
            let score = *result.at_2d::<f32>(0, 0)? as f64;
            if score.is_finite() {
                scores[*digit as usize] = scores[*digit as usize].max(score);
            }
        }
        let mut ordered: Vec<_> = scores.into_iter().enumerate().collect();
        ordered.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (digit, best) = ordered[0];
        Ok((best >= minimum_score && best - ordered[1].1 >= 0.15).then_some(digit as u8))
    }
}

fn value(digits: &[u8]) -> u64 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + *digit as u64)
}

struct Components {
    labels: Mat,
    stats: Mat,
    hsv: Mat,
}

impl Components {
    fn new(image: &Mat, saturation: u8) -> Result<Self> {
        if image.empty() || image.typ() != core::CV_8UC3 {
            return Err(Error::Unreadable);
        }
        let mut hsv = Mat::default();
        imgproc::cvt_color_def(image, &mut hsv, imgproc::COLOR_BGR2HSV)?;
        let mut mask = Mat::default();
        core::in_range(
            &hsv,
            &Scalar::new(0.0, 0.0, 200.0, 0.0),
            &Scalar::new(180.0, saturation as f64, 255.0, 0.0),
            &mut mask,
        )?;
        let mut labels = Mat::default();
        let mut stats = Mat::default();
        let mut centroids = Mat::default();
        imgproc::connected_components_with_stats(
            &mask,
            &mut labels,
            &mut stats,
            &mut centroids,
            8,
            core::CV_32S,
        )?;
        if stats.rows() <= 1 {
            return Err(Error::Unreadable);
        }
        Ok(Self { labels, stats, hsv })
    }

    fn rect(&self, id: i32) -> Result<Rect> {
        Ok(Rect::new(
            *self.stats.at_2d(id, 0)?,
            *self.stats.at_2d(id, 1)?,
            *self.stats.at_2d(id, 2)?,
            *self.stats.at_2d(id, 3)?,
        ))
    }

    fn glyph(&self, id: i32) -> Result<Mat> {
        let rect = self.rect(id)?;
        let mut mask = Mat::new_rows_cols_with_default(
            rect.height,
            rect.width,
            core::CV_32F,
            Scalar::all(0.0),
        )?;
        for y in 0..rect.height {
            for x in 0..rect.width {
                if *self.labels.at_2d::<i32>(rect.y + y, rect.x + x)? == id {
                    *mask.at_2d_mut::<f32>(y, x)? = 1.0;
                }
            }
        }
        Ok(mask)
    }
}

fn glyphs(image: &Mat) -> Result<Vec<Mat>> {
    let components = Components::new(image, 70)?;
    let mut found = (1..components.stats.rows())
        .map(|id| Ok((id, components.rect(id)?)))
        .collect::<Result<Vec<_>>>()?;
    let tallest = found
        .iter()
        .map(|(_, r)| r.height)
        .max()
        .ok_or(Error::Unreadable)?;
    found.retain(|(_, r)| r.height as f64 >= 0.6 * tallest as f64);
    let top = median(found.iter().map(|(_, r)| r.y).collect());
    let bottom = median(found.iter().map(|(_, r)| r.y + r.height).collect());
    let slack = 0.2 * (bottom - top);
    found.retain(|(_, r)| {
        (r.y as f64 - top).abs() <= slack && ((r.y + r.height) as f64 - bottom).abs() <= slack
    });
    found.sort_by_key(|(_, r)| r.x);
    found
        .iter()
        .map(|(id, _)| canvas(&components.glyph(*id)?))
        .collect()
}

fn median(mut values: Vec<i32>) -> f64 {
    values.sort_unstable();
    (values[values.len() / 2] as f64 + values[(values.len() - 1) / 2] as f64) / 2.0
}

fn canvas(glyph: &Mat) -> Result<Mat> {
    let width = (glyph.cols() as f64 * HEIGHT as f64 / glyph.rows() as f64)
        .round_ties_even()
        .clamp(1.0, WIDTH as f64) as i32;
    let mut resized = Mat::default();
    imgproc::resize(
        glyph,
        &mut resized,
        Size::new(width, HEIGHT),
        0.0,
        0.0,
        imgproc::INTER_AREA,
    )?;
    let mut output =
        Mat::new_rows_cols_with_default(HEIGHT, WIDTH, core::CV_32F, Scalar::all(0.0))?;
    let mut target = Mat::roi_mut(
        &mut output,
        Rect::new((WIDTH - width) / 2, 0, width, HEIGHT),
    )?;
    resized.copy_to(&mut target)?;
    Ok(output)
}

fn split_glyphs(glyph: &Mat, count: i32) -> Result<Vec<Mat>> {
    let width = glyph.cols();
    let ink = (0..width)
        .map(|x| {
            (0..glyph.rows())
                .map(|y| Ok(*glyph.at_2d::<f32>(y, x)?))
                .sum::<Result<f32>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let mut edges = vec![0];
    let mut cut = 0;
    for part in 1..count {
        let centre = (part as f64 * width as f64 / count as f64).round_ties_even() as i32;
        let reach = (width / (2 * count)).max(1);
        let low = (cut + 1).max(centre - reach);
        let high = (width - 1).min(centre + reach);
        cut = (low..=high)
            .min_by(|a, b| ink[*a as usize].total_cmp(&ink[*b as usize]))
            .ok_or(Error::Unreadable)?;
        edges.push(cut);
    }
    edges.push(width);
    edges
        .windows(2)
        .map(|edge| {
            let columns: Vec<_> = (edge[0]..edge[1])
                .filter(|x| ink[*x as usize] > 0.15 * glyph.rows() as f32)
                .collect();
            let mut rows = Vec::new();
            for y in 0..glyph.rows() {
                for x in edge[0]..edge[1] {
                    if *glyph.at_2d::<f32>(y, x)? != 0.0 {
                        rows.push(y);
                        break;
                    }
                }
            }
            let (&left, &right) = columns
                .first()
                .zip(columns.last())
                .ok_or(Error::Unreadable)?;
            let (&top, &bottom) = rows.first().zip(rows.last()).ok_or(Error::Unreadable)?;
            Ok(Mat::roi(
                glyph,
                Rect::new(left, top, right - left + 1, bottom - top + 1),
            )?
            .try_clone()?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(text: &str) -> Mat {
        let mut baseline = 0;
        let size =
            imgproc::get_text_size(text, imgproc::FONT_HERSHEY_SIMPLEX, 0.8, 2, &mut baseline)
                .unwrap();
        let mut image =
            Mat::new_rows_cols_with_default(30, size.width + 10, core::CV_8UC3, Scalar::all(0.0))
                .unwrap();
        imgproc::put_text(
            &mut image,
            text,
            core::Point::new(4, 24),
            imgproc::FONT_HERSHEY_SIMPLEX,
            0.8,
            Scalar::all(255.0),
            2,
            imgproc::LINE_8,
            false,
        )
        .unwrap();
        image
    }

    fn digits() -> Digits {
        Digits {
            templates: (0..=9)
                .map(|d| (d, glyphs(&rendered(&d.to_string())).unwrap().remove(0)))
                .collect(),
        }
    }

    fn fraction(left: &str, right: &str) -> Mat {
        let left = rendered(left);
        let right = rendered(right);
        let mut image = Mat::new_rows_cols_with_default(
            30,
            left.cols() + right.cols() + 20,
            core::CV_8UC3,
            Scalar::all(0.0),
        )
        .unwrap();
        left.copy_to(&mut Mat::roi_mut(&mut image, Rect::new(0, 0, left.cols(), 30)).unwrap())
            .unwrap();
        right
            .copy_to(
                &mut Mat::roi_mut(&mut image, Rect::new(left.cols() + 20, 0, right.cols(), 30))
                    .unwrap(),
            )
            .unwrap();
        // Align the separator to the digits; OpenCV's Hershey slash extends beyond their baseline.
        imgproc::line(
            &mut image,
            core::Point::new(left.cols() + 4, 24),
            core::Point::new(left.cols() + 12, 6),
            Scalar::all(255.0),
            2,
            imgproc::LINE_8,
            0,
        )
        .unwrap();
        image
    }

    #[test]
    fn numbers_fractions_and_ambiguous_glyphs() {
        let mut digits = digits();
        assert_eq!(digits.number(&rendered("504 548"), 0.75).unwrap(), 504548);
        assert_eq!(
            digits.trailing(&rendered("504 548"), 0.75, 70).unwrap(),
            504548
        );
        assert_eq!(
            digits.fraction(&fraction("120", "1600")).unwrap(),
            (120, 1600)
        );
        for text in ["1600", "1600 / 120", "120 / 0", "12 / 10 / 3"] {
            assert!(digits.fraction(&rendered(text)).is_err(), "{text}");
        }
        assert!(digits.fraction(&fraction("1600", "120")).is_err());
        assert!(digits.fraction(&fraction("120", "0")).is_err());
        assert!(digits.trailing(&rendered("0012"), 0.75, 70).is_err());
        let seven = glyphs(&rendered("7")).unwrap().remove(0);
        digits.templates.push((9, seven));
        assert!(digits.number(&rendered("7"), 0.75).is_err());
    }

    #[test]
    fn builder_percent_has_an_explicit_two_stage_bound() {
        let reader = digits();
        for value in [101, 199, 200] {
            let image = rendered(&format!("{value}%"));
            assert_eq!(reader.percent_up_to(&image, 200).unwrap(), value);
            assert!(reader.percent(&image).is_err());
        }
        assert!(reader.percent_up_to(&rendered("201%"), 200).is_err());
    }

    #[test]
    fn trailing_digits_ignore_different_sized_background_labels() {
        let mut image =
            Mat::new_rows_cols_with_default(30, 120, core::CV_8UC3, Scalar::all(0.0)).unwrap();
        imgproc::put_text(
            &mut image,
            "ahl",
            core::Point::new(2, 22),
            imgproc::FONT_HERSHEY_SIMPLEX,
            0.45,
            Scalar::all(255.0),
            1,
            imgproc::LINE_8,
            false,
        )
        .unwrap();
        imgproc::put_text(
            &mut image,
            "1 073",
            core::Point::new(36, 24),
            imgproc::FONT_HERSHEY_SIMPLEX,
            0.8,
            Scalar::all(255.0),
            2,
            imgproc::LINE_8,
            false,
        )
        .unwrap();
        assert_eq!(digits().trailing(&image, 0.75, 70).unwrap(), 1073);
    }
}
