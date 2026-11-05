use opencv::{
    core::{self, Mat, Vec3b},
    imgproc,
    prelude::*,
};
use serde::{Deserialize, Serialize};

use super::{Digits, crop};
use crate::{
    Error, Result,
    calibration::{Region, Resolution},
    device::Point,
};
pub mod automatic;
pub mod recipe;
pub mod roster;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Layout {
    #[serde(default)]
    pub recipe: Option<recipe::Recipe>,
}

impl Layout {
    pub fn validate(&self, resolution: Resolution) -> Result<()> {
        if let Some(recipe) = &self.recipe {
            recipe.validate(resolution)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Counted,
    Sized,
    Siege,
    Warden,
    Hero,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub region: Region,
    pub count_region: Option<Region>,
    pub slot: Point,
    pub kind: Kind,
    pub group: usize,
    pub count: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Deployment {
    Wait(u32),
    PreciseAbility {
        card: recipe::BoundCard,
    },
    PreciseDrop {
        card: recipe::BoundCard,
        point: Point,
        count: u32,
        interval_ms: u32,
        initial_count: u32,
    },
}

fn resolution(image: &Mat) -> Result<Resolution> {
    if image.empty() || image.typ() != core::CV_8UC3 {
        return Err(Error::Frame);
    }
    Ok(Resolution(image.cols() as u32, image.rows() as u32))
}

fn hsv(image: &Mat) -> Result<Mat> {
    resolution(image)?;
    let mut output = Mat::default();
    imgproc::cvt_color_def(image, &mut output, imgproc::COLOR_BGR2HSV)?;
    Ok(output)
}

fn share(image: &Mat, region: Region, predicate: impl Fn(Vec3b) -> bool) -> Result<f64> {
    region.validate(resolution(image)?)?;
    let Region(x, y, w, h) = region;
    let mut count = 0_u64;
    for row in y..y + h {
        for col in x..x + w {
            count += u64::from(predicate(*image.at_2d::<Vec3b>(row as i32, col as i32)?));
        }
    }
    Ok(count as f64 / (u64::from(w) * u64::from(h)) as f64)
}

pub(super) fn card_regions(image: &Mat, bar: Region, builder: bool) -> Result<Vec<Region>> {
    bar.validate(resolution(image)?)?;
    let Region(x, y, width, height) = bar;
    if height < 20 {
        return Err(Error::Unreadable);
    }
    let colour = hsv(image)?;
    let mut gray = Mat::default();
    imgproc::cvt_color_def(image, &mut gray, imgproc::COLOR_BGR2GRAY)?;
    let mut start = None;
    let mut spans = Vec::new();
    for column in 0..=width {
        let busy = if column == width {
            false
        } else {
            let mut sum = 0.0;
            let mut squared = 0.0;
            for row in y..y + height {
                let value = f64::from(*gray.at_2d::<u8>(row as i32, (x + column) as i32)?);
                sum += value;
                squared += value * value;
            }
            (squared / f64::from(height) - (sum / f64::from(height)).powi(2))
                .max(0.0)
                .sqrt()
                > 18.0
        };
        if busy {
            start.get_or_insert(column);
        } else if let Some(left) = start.take() {
            let w = column - left;
            let region = Region(x + left, y, w, height);
            if (8..=110).contains(&w)
                && (share(&colour, region, |p| p[1] > 100 && p[2] > 80)? >= 0.43
                    || (builder
                        && (builder_card_header(&colour, region)?
                            || card_rails(&colour, region)?
                            || builder_camp_frame(image, &colour, region)?)))
            {
                spans.push(Region(x + left, y, w, height));
            }
        }
    }
    if !builder {
        // Portrait colours do not define card boundaries: pale troops still have the same frame.
        for frame in home_card_frames(&colour, bar)? {
            spans.retain(|r| r.0 + r.2 <= frame.0 || r.0 >= frame.0 + frame.2);
            spans.push(frame);
        }
        spans.sort_by_key(|r| r.0);
    }
    Ok(spans)
}

fn home_card_frame(colour: &Mat, Region(x, y, width, height): Region) -> Result<bool> {
    if !(82..=90).contains(&width) || y < 2 || y + height + 2 >= colour.rows() as u32 {
        return Ok(false);
    }
    let frame = |p: Vec3b| (80..=170).contains(&p[0]) && p[1] > 80 && p[2] > 80;
    Ok(
        (share(colour, Region(x + 3, y - 2, width - 6, 1), frame)? >= 0.95
            || card_rails(colour, Region(x, y, width, height))?)
            && share(colour, Region(x, y + height + 2, width, 1), frame)? >= 0.95,
    )
}

fn home_card_frames(colour: &Mat, bar: Region) -> Result<Vec<Region>> {
    let Region(x, y, width, height) = bar;
    if y < 2 || y + height + 2 >= colour.rows() as u32 {
        return Ok(Vec::new());
    }
    let mut frames = Vec::new();
    let mut start = None;
    for column in x..=x + width {
        let blue = if column == x + width {
            false
        } else {
            let p = *colour.at_2d::<Vec3b>((y + height + 2) as i32, column as i32)?;
            (80..=170).contains(&p[0]) && p[1] > 80 && p[2] > 80
        };
        if blue {
            start.get_or_insert(column);
        } else if let Some(left) = start.take() {
            let region = Region(left, y, column - left, height);
            if (82..=90).contains(&region.2) {
                frames.push(region);
            }
        }
    }
    Ok(frames)
}

fn builder_card_header(colour: &Mat, Region(x, y, width, height): Region) -> Result<bool> {
    if !(65..=110).contains(&width) || height < 80 {
        return Ok(false);
    }
    let blue = |p: Vec3b| (80..=170).contains(&p[0]) && p[1] > 100 && p[2] > 80;
    Ok(share(colour, Region(x, y, width, 28), blue)? >= 0.7)
}

fn card_rails(colour: &Mat, Region(x, y, width, height): Region) -> Result<bool> {
    if !(65..=110).contains(&width) || height < 80 {
        return Ok(false);
    }
    let blue = |p: Vec3b| (80..=170).contains(&p[0]) && p[1] > 100 && p[2] > 80;
    // Camp headers and side rails remain when the green swap button disappears in battle.
    let rail = |p: Vec3b| blue(p) || (p[1] < 60 && p[2] > 200);
    let left = share(colour, Region(x + 1, y + height / 4, 2, height / 2), rail)?;
    let right = share(
        colour,
        Region(x + width - 3, y + height / 4, 2, height / 2),
        rail,
    )?;
    Ok(left >= 0.6 && right >= 0.6)
}

fn builder_camp_frame(image: &Mat, colour: &Mat, Region(x, y, w, h): Region) -> Result<bool> {
    if !(82..=94).contains(&w) || !(80..=128).contains(&h) {
        return Ok(false);
    }
    if share(image, Region(x + 2, y + 2, 24, 18), |p| {
        p.iter().all(|v| *v > 215)
    })? * 432.
        < 30.
    {
        return Ok(false);
    }
    let rail = |p: Vec3b| p[2] > 80 && (p[1] < 60 || (80..=170).contains(&p[0]));
    for dx in [1, w - 3] {
        if share(colour, Region(x + dx, y + h / 4, 2, h / 2), rail)? < 0.6 {
            return Ok(false);
        }
    }
    let mut separator = false;
    for dy in 27..=33 {
        separator |= share(colour, Region(x + 4, y + dy, w - 8, 1), |p| p[2] > 80)? >= 0.95;
    }
    let mut footer = false;
    for dy in h - 3..h {
        footer |= share(colour, Region(x + 4, y + dy, w - 8, 1), rail)? >= 0.8;
    }
    Ok(separator && footer)
}

pub fn cards(image: &Mat, bar: Region, camps: bool, digits: &Digits) -> Result<Vec<Card>> {
    let Region(_, y, _, height) = bar;
    let colour = hsv(image)?;
    let mut spans = Vec::new();
    for region in card_regions(image, bar, camps)? {
        if region.2 >= 65
            && (share(&colour, region, |p| p[1] > 100 && p[2] > 80)? >= 0.43
                || (!camps && home_card_frame(&colour, region)?)
                || (camps
                    && ((builder_card_header(&colour, region)? && card_rails(&colour, region)?)
                        || builder_camp_frame(image, &colour, region)?)))
        {
            spans.push((region.0, region.0 + region.2));
        }
    }
    if spans.len() > 20 {
        return Err(Error::Unreadable);
    }
    let mut found = Vec::new();
    let mut group = 0;
    let mut previous = None;
    for (left, right) in spans {
        if previous.is_some_and(|last| left - last > 20) {
            group += 1;
        }
        previous = Some(right);
        let slot = Point((left + right) / 2, y + height / 2);
        let is_selected = !camps && selected(image, slot)?;
        let white = |p: Vec3b| p[0] > 215 && p[1] > 215 && p[2] > 215;
        let label_y = if is_selected {
            y.saturating_sub(4)
        } else {
            y + 2
        };
        let label_white = share(image, Region(right - 40, label_y, 38, 18), white)? * 684.0;
        let kind = if camps {
            let magenta = share(&colour, Region(right - 40, y + 2, 36, 16), |p| {
                (135..=165).contains(&p[0]) && p[1] > 100 && p[2] > 120
            })?;
            let number = share(image, Region(left + 2, y + 2, 24, 18), white)? * 432.0;
            if label_white >= 100.0 {
                Kind::Sized
            } else if magenta >= 0.5 || number >= 30.0 {
                Kind::Counted
            } else {
                Kind::Hero
            }
        } else if label_white >= 100.0 {
            Kind::Counted
        } else {
            let region = Region(left, y, right - left, height);
            let cyan = share(&colour, region, |p| {
                (80..=95).contains(&p[0]) && p[1] > 100 && p[2] > 120
            })?;
            let others = share(&colour, region, |p| {
                (95..=140).contains(&p[0]) && p[1] > 100 && p[2] > 120
            })?;
            let bottom = height * 3 / 5;
            let green = share(
                &colour,
                Region(left, y + bottom, right - left, height - bottom),
                |p| (40..=80).contains(&p[0]) && p[1] > 120 && p[2] > 120,
            )? * f64::from((right - left) * (height - bottom));
            if cyan > others {
                Kind::Siege
            } else if green >= 200.0 {
                Kind::Warden
            } else {
                Kind::Hero
            }
        };
        let top = if camps { Some(y) } else { y.checked_sub(10) };
        let count_region = top.map(|top| {
            if is_selected {
                Region(slot.0.saturating_sub(16), top, 55, 26)
            } else {
                Region(left + 25, top, right - left - 27, 26)
            }
        });
        let count = count_region
            .and_then(|Region(x, y, w, h)| {
                let region = if is_selected {
                    let top = y.saturating_sub(16);
                    Region(x, top, w + 6, h + y - top)
                } else {
                    Region(x, y, w, h)
                };
                crop(image, region).ok()
            })
            .and_then(|label| digits.card_count(&label, camps).ok());
        let count = if camps && empty(image, slot)? {
            let mut deviation = core::Scalar::default();
            core::mean_std_dev(
                &crop(image, Region(left + 10, y + 40, right - left - 20, 40))?,
                &mut core::no_array(),
                &mut deviation,
                &core::no_array(),
            )?;
            if deviation.0[..3].iter().all(|value| *value <= 5.) {
                continue;
            }
            Some(0)
        } else {
            count
        };
        found.push(Card {
            region: Region(left, y, right - left, height),
            count_region,
            slot,
            kind,
            group,
            count,
        });
    }
    Ok(found)
}

pub fn empty(image: &Mat, slot: Point) -> Result<bool> {
    let res = resolution(image)?;
    slot.validate(res)?;
    let left = slot.0.saturating_sub(40);
    let top = slot.1.saturating_sub(38);
    let right = (slot.0 + 40).min(res.0);
    let bottom = (slot.1 + 50).min(res.1);
    let colour = hsv(&crop(image, Region(left, top, right - left, bottom - top))?)?;
    Ok(core::mean(&colour, &core::no_array())?[1] < 20.0)
}

pub fn selected(image: &Mat, slot: Point) -> Result<bool> {
    slot.validate(resolution(image)?)?;
    let white = |x: u32, y: u32| -> Result<bool> {
        Ok(image
            .at_2d::<Vec3b>(y as i32, x as i32)?
            .iter()
            .all(|c| *c > 215))
    };
    if slot.0 < 55 || slot.0 + 55 >= image.cols() as u32 || image.rows() < 720 {
        return Ok(false);
    }
    let mut shaded = false;
    let mut bright_side = false;
    for side in [-1_i32, 1] {
        let mut border = false;
        let mut neutral_border = false;
        for offset in 38..=55 {
            let x = (slot.0 as i32 + side * offset) as u32;
            let mut count = 0;
            let mut neutral = 0;
            for y in 625..685 {
                count += u32::from(white(x, y)?);
                let pixel = image.at_2d::<Vec3b>(y as i32, x as i32)?;
                let low = *pixel.iter().min().unwrap();
                let high = *pixel.iter().max().unwrap();
                neutral += u32::from(low >= 185 && high - low <= 10);
            }
            border |= count >= 45;
            neutral_border |= neutral >= 45;
        }
        if !border && !neutral_border {
            return Ok(false);
        }
        shaded |= !border;
        bright_side |= border;
    }
    if shaded {
        // The selected frame has a shaded rail; require its raised top and opposite bright rail too.
        if !bright_side {
            return Ok(false);
        }
        let mut raised_top = false;
        for y in 576..596 {
            let mut count = 0;
            for x in slot.0 - 32..slot.0 + 32 {
                count += u32::from(white(x, y)?);
            }
            raised_top |= count >= 50;
        }
        if !raised_top {
            return Ok(false);
        }
    }
    for y in 700..719 {
        let mut count = 0;
        for x in slot.0 - 32..slot.0 + 32 {
            count += u32::from(white(x, y)?);
        }
        if count >= 50 {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn hero_charge(image: &Mat, slot: Point) -> Result<u8> {
    let colour = hsv(image)?;
    let top = slot.1.checked_sub(89).ok_or(Error::Unreadable)?;
    let left = slot.0.checked_sub(36).ok_or(Error::Unreadable)?;
    let mut charge = 0;
    for (offset, width) in [(0, 20), (22, 19), (43, 30)] {
        if share(&colour, Region(left + offset, top, width, 8), |p| {
            (140..=165).contains(&p[0]) && p[1] > 100 && p[2] > 150
        })? < 0.5
        {
            return Ok(charge);
        }
        charge += 1;
    }
    Ok(charge)
}

pub fn hero_deployed(image: &Mat, slot: Point) -> Result<bool> {
    hero_health(image, slot, 85)
}

pub(super) fn hero_waiting_health(image: &Mat, slot: Point) -> Result<bool> {
    hero_health(image, slot, 58)
}

fn hero_health(image: &Mat, slot: Point, top: u32) -> Result<bool> {
    let colour = hsv(image)?;
    let x = slot.0.checked_sub(32).ok_or(Error::Unreadable)?;
    // Deployment raises a framed health bar with visible green, yellow or red fill.
    for y in slot.1.checked_sub(top).ok_or(Error::Unreadable)?..slot.1 - (top - 20) {
        let rail = |row| share(image, Region(x, row, 64, 1), |p| p.iter().all(|v| *v < 60));
        if rail(y)? > 0.9
            && rail(y + 11)? > 0.9
            && share(image, Region(x, y + 3, 64, 7), |p| {
                *p.iter().max().unwrap() > 20
            })? > 0.85
            // Dark scenery behind the army bar is not a depleted health bar.
            && share(&colour, Region(x, y + 3, 64, 7), |p| {
                p[0] <= 85 && p[1] > 110 && p[2] > 90
            })? > 0.01
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn stars(image: &Mat, regions: &[Region]) -> Result<u8> {
    if regions.is_empty() || regions.len() > 6 {
        return Err(Error::Unreadable);
    }
    let colour = hsv(image)?;
    let mut count = 0;
    for &region in regions {
        if share(&colour, region, |p| p[2] > 0)? == 0.0 {
            return Err(Error::Unreadable);
        }
        if share(&colour, region, |p| {
            (8..=30).contains(&p[0]) && p[1] >= 120 && p[2] >= 150
        })? >= 0.25
        {
            count += 1;
        }
    }
    Ok(count)
}
