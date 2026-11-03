use crate::{
    Error, Result,
    calibration::{Region, Resolution, Screen},
    device::Point,
    telemetry::Resource,
    vision::{Digits, crop, target::Template},
};
use opencv::{
    core::{self, Mat, Size, Vec3b},
    imgproc,
    prelude::*,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
pub struct Spec {
    pub build: Point,
    pub row: Row,
    pub scroll: [Point; 2],
    pub gems: Hud,
    pub min_white: u32,
    pub confirm: Confirm,
    pub button: Button,
    pub hud_gold: Option<Hud>,
    pub hud_elixir: Option<Hud>,
}
#[derive(Deserialize)]
pub struct Hud {
    pub list: Region,
    pub village: Region,
}
#[derive(Deserialize)]
pub struct Row {
    pub file: PathBuf,
    pub region: Region,
    pub max_score: f64,
    pub tap_x: u32,
    pub price: [i32; 4],
    pub scales: Vec<f64>,
    #[serde(skip)]
    template: Option<Template>,
}
#[derive(Deserialize)]
pub struct Button {
    file: PathBuf,
    search: Region,
    scales: Vec<f64>,
    max_score: f64,
    price: [i32; 4],
    tap: [i32; 2],
    icon_min: u32,
    #[serde(skip)]
    template: Option<Template>,
}
#[derive(Deserialize)]
pub struct Confirm {
    pub screen: Screen,
    pub price: Region,
    pub icon: Region,
    min_white: u32,
    max_score: f64,
    icons: BTreeMap<String, PathBuf>,
    price_color: Option<String>,
    #[serde(skip)]
    templates: BTreeMap<String, Template>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub resource: Resource,
    pub point: Point,
    pub affordable: bool,
}

fn invalid() -> Error {
    Error::Calibration("Invalid wall calibration".into())
}
fn region_at(x: i32, y: i32, band: [i32; 4], scale: f64, res: Resolution) -> Result<Region> {
    let [dx, dy, w, h] = band;
    let values = [
        x + (f64::from(dx) * scale).round() as i32,
        y + (f64::from(dy) * scale).round() as i32,
        (f64::from(w) * scale).round() as i32,
        (f64::from(h) * scale).round() as i32,
    ];
    let [x, y, w, h] = values.map(|v| u32::try_from(v).map_err(|_| invalid()));
    let region = Region(x?, y?, w?, h?);
    region.validate(res)?;
    Ok(region)
}
fn resolution(image: &Mat) -> Resolution {
    Resolution(image.cols() as u32, image.rows() as u32)
}
fn colour_count(image: &Mat, predicate: impl Fn(Vec3b) -> bool) -> Result<u32> {
    if image.empty() || image.typ() != core::CV_8UC3 {
        return Err(Error::Frame);
    }
    let mut count = 0;
    for y in 0..image.rows() {
        for x in 0..image.cols() {
            count += u32::from(predicate(*image.at_2d::<Vec3b>(y, x)?));
        }
    }
    Ok(count)
}
fn hsv(image: &Mat) -> Result<Mat> {
    let mut result = Mat::default();
    imgproc::cvt_color_def(image, &mut result, imgproc::COLOR_BGR2HSV)?;
    Ok(result)
}
fn white(image: &Mat) -> Result<u32> {
    colour_count(image, |p| p[0] > 200 && p[1] > 200 && p[2] > 200)
}
pub fn currency(image: &Mat, minimum: u32) -> Result<Option<Resource>> {
    let image = hsv(image)?;
    let gold = colour_count(&image, |p| {
        (18..=35).contains(&p[0]) && p[1] > 120 && p[2] > 150
    })?;
    let elixir = colour_count(&image, |p| {
        (135..=165).contains(&p[0]) && p[1] > 100 && p[2] > 100
    })?;
    Ok(if gold.max(elixir) < minimum || gold == elixir {
        None
    } else if gold > elixir {
        Some(Resource::Gold)
    } else {
        Some(Resource::Elixir)
    })
}
impl Spec {
    pub fn load(&mut self, root: &Path, res: Resolution) -> Result<()> {
        for point in [self.build, self.scroll[0], self.scroll[1]] {
            point.validate(res)?;
        }
        for region in [
            self.row.region,
            self.gems.list,
            self.gems.village,
            self.button.search,
            self.confirm.price,
            self.confirm.icon,
        ] {
            region.validate(res)?;
        }
        for hud in self.hud_gold.iter().chain(self.hud_elixir.iter()) {
            hud.list.validate(res)?;
            hud.village.validate(res)?;
        }
        if self.row.tap_x >= res.0
            || self.row.price[2] <= 0
            || self.row.price[3] <= 0
            || self
                .row
                .price
                .iter()
                .chain(self.button.price.iter())
                .chain(self.button.tap.iter())
                .any(|v| v.unsigned_abs() > 8192)
            || self.button.price[2] <= 0
            || self.button.price[3] <= 0
            || self.min_white == 0
            || self.confirm.min_white == 0
            || self.button.icon_min == 0
            || !matches!(
                self.confirm.screen,
                Screen::WallConfirm | Screen::BuilderWallConfirm
            )
            || self
                .confirm
                .price_color
                .as_deref()
                .is_some_and(|v| v != "green")
        {
            return Err(invalid());
        }
        self.row.template = Some(Template::load(
            root,
            &self.row.file,
            &self.row.scales,
            self.row.max_score,
        )?);
        self.button.template = Some(Template::load(
            root,
            &self.button.file,
            &self.button.scales,
            self.button.max_score,
        )?);
        for (key, file) in &self.confirm.icons {
            if !matches!(key.as_str(), "gold" | "elixir") {
                return Err(invalid());
            }
            self.confirm.templates.insert(
                key.clone(),
                Template::load(root, file, &[1.0], self.confirm.max_score)?,
            );
        }
        if self.confirm.templates.is_empty() {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn gem_shift(&self, image: &Mat) -> Result<u32> {
        let Region(x, y, _, h) = self.gems.village;
        for shift in [0, 50] {
            let Some(top) = y.checked_sub(shift + 6) else {
                continue;
            };
            let region = Region(x + 128, top, 38, h + 12);
            let colour = hsv(&crop(image, region)?)?;
            let green = colour_count(&colour, |p| {
                (35..=85).contains(&p[0]) && p[1] > 120 && p[2] > 120
            })?;
            if f64::from(green) / f64::from(region.2 * region.3) >= 0.25 {
                return Ok(shift);
            }
        }
        Err(Error::Unreadable)
    }
    pub fn gems(&self, image: &Mat, digits: &Digits, list: bool, shift: u32) -> Result<u32> {
        let Region(x, y, w, h) = if list {
            self.gems.list
        } else {
            self.gems.village
        };
        let region = Region(x, y.checked_sub(shift).ok_or(Error::Unreadable)?, w, h);
        u32::try_from(digits.trailing(&crop(image, region)?, 0.65, 110)?)
            .map_err(|_| Error::Unreadable)
    }
    pub fn balance(
        &self,
        image: &Mat,
        digits: &Digits,
        resource: Resource,
        list: bool,
    ) -> Result<u32> {
        let hud = match resource {
            Resource::Gold => &self.hud_gold,
            Resource::Elixir => &self.hud_elixir,
            Resource::Dark => return Err(Error::InputGuard),
        };
        let hud = hud.as_ref().ok_or(Error::Unreadable)?;
        u32::try_from(digits.trailing(
            &crop(image, if list { hud.list } else { hud.village })?,
            0.65,
            110,
        )?)
        .map_err(|_| Error::Unreadable)
    }
    pub fn rows(&self, image: &Mat, digits: &Digits) -> Result<Vec<(Option<u32>, u32)>> {
        let mut hits = self
            .row
            .template
            .as_ref()
            .ok_or_else(invalid)?
            .find_all(image, self.row.region)?;
        hits.sort_by_key(|h| h.y);
        let mut result = Vec::new();
        for hit in hits {
            let y = (hit.y + hit.height / 2) as u32;
            if result
                .last()
                .is_some_and(|(_, old): &(Option<u32>, u32)| y - old <= hit.height as u32)
            {
                continue;
            }
            let [x, dy, w, h] = self.row.price;
            let price = (|| {
                if y as i32 + dy + h > (self.row.region.1 + self.row.region.3) as i32 {
                    return Err(Error::Unreadable);
                }
                let region = region_at(0, y as i32, [x, dy, w, h], 1.0, resolution(image))?;
                let mut scaled = Mat::default();
                imgproc::resize(
                    &crop(image, region)?,
                    &mut scaled,
                    Size::default(),
                    2.0,
                    2.0,
                    imgproc::INTER_CUBIC,
                )?;
                u32::try_from(digits.trailing(&scaled, 0.65, 160)?).map_err(|_| Error::Unreadable)
            })()
            .ok();
            result.push((price, y));
        }
        Ok(result)
    }
    pub fn buttons(&self, image: &Mat) -> Result<Vec<Choice>> {
        let mut result = Vec::new();
        for hit in self
            .button
            .template
            .as_ref()
            .ok_or_else(invalid)?
            .find_all(image, self.button.search)?
        {
            let Ok(region) = region_at(
                hit.x,
                hit.y,
                self.button.price,
                hit.scale,
                resolution(image),
            ) else {
                continue;
            };
            let band = crop(image, region)?;
            let Some(resource) = currency(&band, self.button.icon_min)? else {
                continue;
            };
            let point = Point(
                u32::try_from(hit.x + (f64::from(self.button.tap[0]) * hit.scale).round() as i32)
                    .map_err(|_| Error::InputGuard)?,
                u32::try_from(hit.y + (f64::from(self.button.tap[1]) * hit.scale).round() as i32)
                    .map_err(|_| Error::InputGuard)?,
            );
            point.validate(resolution(image))?;
            result.push(Choice {
                resource,
                point,
                affordable: f64::from(white(&band)?)
                    >= f64::from(self.min_white) * hit.scale * hit.scale,
            });
        }
        Ok(result)
    }
    pub fn confirm_cost(
        &self,
        image: &Mat,
        digits: &Digits,
        resource: Resource,
        balance: u32,
        reserve: u32,
    ) -> Result<u32> {
        let key = match resource {
            Resource::Gold => "gold",
            Resource::Elixir => "elixir",
            Resource::Dark => return Err(Error::InputGuard),
        };
        let icon = self.confirm.templates.get(key).ok_or(Error::InputGuard)?;
        if icon.find(image, self.confirm.icon)?.is_none() {
            return Err(Error::InputGuard);
        }
        let mut price = crop(image, self.confirm.price)?;
        if self.confirm.price_color.as_deref() == Some("green") {
            price = crate::vision::green_text(&price)?;
        }
        if white(&price)? < self.confirm.min_white {
            return Err(Error::InputGuard);
        }
        let cost =
            u32::try_from(digits.trailing(&price, 0.65, 110)?).map_err(|_| Error::Unreadable)?;
        if cost == 0
            || balance
                .checked_sub(cost)
                .is_none_or(|remaining| remaining < reserve)
        {
            return Err(Error::InputGuard);
        }
        Ok(cost)
    }
}
