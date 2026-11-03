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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Layout {
    pub bar: Region,
    #[serde(default)]
    pub deploy: Option<Point>,
    #[serde(default)]
    pub hold: u32,
    #[serde(default)]
    pub points: Vec<Point>,
    #[serde(default)]
    pub spells: Vec<Point>,
    #[serde(default)]
    pub ring: Vec<Point>,
    #[serde(default)]
    pub spell_spread: Vec<Point>,
    #[serde(default)]
    pub quake: Vec<Point>,
}

impl Layout {
    pub fn validate(&self, resolution: Resolution) -> Result<()> {
        self.bar.validate(resolution)?;
        if let Some(point) = self.deploy {
            point.validate(resolution)?;
        }
        if self.bar.3 < 20
            || self.hold > 60
            || (self.points.is_empty() && (self.spells.is_empty() || self.hold == 0))
        {
            return Err(Error::Calibration(
                "Invalid army bar or deployment layout".into(),
            ));
        }
        for points in [
            &self.points,
            &self.spells,
            &self.ring,
            &self.spell_spread,
            &self.quake,
        ] {
            if points.len() > 32 {
                return Err(Error::Calibration("Too many deployment points".into()));
            }
            for point in points {
                point.validate(resolution)?;
            }
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
    pub slot: Point,
    pub kind: Kind,
    pub group: usize,
    pub count: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Deployment {
    Drop {
        slot: Point,
        point: Point,
        count: u32,
    },
    Ability {
        slots: Vec<Point>,
        wait_ms: u32,
    },
    Hold {
        slot: Point,
        seconds: u32,
    },
    Spread {
        slot: Point,
        points: Vec<Point>,
        count: u32,
        burst: u32,
        fingers: bool,
        spaced: bool,
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

pub fn cards(image: &Mat, bar: Region, camps: bool, digits: &Digits) -> Result<Vec<Card>> {
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
            if (65..=98).contains(&w)
                && share(&colour, Region(x + left, y, w, height), |p| {
                    p[1] > 100 && p[2] > 80
                })? >= 0.43
            {
                spans.push((x + left, x + column));
            }
        }
    }
    if spans.len() > 20 {
        return Err(Error::Unreadable);
    }
    let mut found = Vec::new();
    let mut group = 0;
    let mut previous = None;
    for (left, right) in spans {
        if previous.is_some_and(|last| left - last > 17) {
            group += 1;
        }
        previous = Some(right);
        let white = |p: Vec3b| p[0] > 215 && p[1] > 215 && p[2] > 215;
        let label_white = share(image, Region(right - 40, y + 2, 38, 18), white)? * 684.0;
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
        let count = top
            .and_then(|top| crop(image, Region(left + 25, top, right - left - 27, 26)).ok())
            .and_then(|label| digits.card_count(&label, camps).ok());
        found.push(Card {
            slot: Point((left + right) / 2, y + height / 2),
            kind,
            group,
            count,
        });
    }
    Ok(found)
}

pub fn plan(
    cards: &[Card],
    layout: &Layout,
    deploy: Point,
    ring: bool,
    quake: bool,
    stack: bool,
) -> Result<Vec<Deployment>> {
    if cards.is_empty() {
        return Err(Error::Unreadable);
    }
    let mut steps = Vec::new();
    if !layout.points.is_empty() {
        let mut camps = Vec::new();
        let fallback = cards.iter().filter_map(|c| c.count).max().unwrap_or(1);
        for card in cards {
            if card.kind == Kind::Hero {
                steps.push(Deployment::Drop {
                    slot: card.slot,
                    point: deploy,
                    count: 1,
                });
                steps.push(Deployment::Ability {
                    slots: vec![card.slot],
                    wait_ms: 0,
                });
            } else {
                let point = if stack {
                    deploy
                } else {
                    layout.points[camps.len() % layout.points.len()]
                };
                let count = if card.kind == Kind::Sized {
                    card.count.unwrap_or(fallback)
                } else {
                    1
                };
                camps.push(Deployment::Drop {
                    slot: card.slot,
                    point,
                    count,
                });
            }
        }
        if !camps.is_empty() {
            let slots = camps
                .iter()
                .filter_map(|step| match step {
                    Deployment::Drop { slot, .. } => Some(*slot),
                    _ => None,
                })
                .collect();
            steps.extend(camps);
            steps.push(Deployment::Ability {
                slots,
                wait_ms: 5000,
            });
        }
        return Ok(steps);
    }
    if layout.spells.is_empty()
        || (ring && layout.ring.is_empty())
        || (quake && layout.quake.is_empty())
    {
        return Err(Error::Calibration(
            "The selected deployment strategy has no points".into(),
        ));
    }
    let last = cards
        .iter()
        .filter(|c| c.kind == Kind::Counted)
        .map(|c| c.group)
        .max()
        .unwrap_or(0);
    let mut troops = Vec::new();
    let mut siege = Vec::new();
    let mut heroes = Vec::new();
    let mut spells = Vec::new();
    for card in cards {
        match card.kind {
            Kind::Counted if card.group > 0 && card.group < last => siege.push(card),
            Kind::Counted if card.group == 0 => troops.push(card),
            Kind::Counted => spells.push(card),
            Kind::Siege => siege.push(card),
            Kind::Warden => heroes.insert(0, card),
            _ => heroes.push(card),
        }
    }
    let drop = |card: &Card| Deployment::Drop {
        slot: card.slot,
        point: deploy,
        count: 1,
    };
    if ring {
        steps.extend(heroes.iter().map(|c| drop(c)));
        if !heroes.is_empty() {
            steps.push(Deployment::Ability {
                slots: heroes.iter().map(|c| c.slot).collect(),
                wait_ms: 0,
            });
        }
    } else {
        steps.extend(troops.iter().map(|c| Deployment::Hold {
            slot: c.slot,
            seconds: layout.hold,
        }));
    }
    steps.extend(siege.iter().map(|c| drop(c)));
    if !ring {
        for hero in heroes {
            steps.push(drop(hero));
            steps.push(Deployment::Ability {
                slots: vec![hero.slot],
                wait_ms: 0,
            });
        }
    }
    for (index, card) in spells.iter().enumerate() {
        let mut points = if quake {
            layout.quake.iter().flat_map(|p| [*p; 4]).collect()
        } else if ring && !layout.spell_spread.is_empty() {
            layout.spell_spread.clone()
        } else {
            layout.spells.clone()
        };
        if !quake {
            let offset = index * 3 % points.len();
            points.rotate_left(offset);
        }
        let count = if ring || quake { card.count } else { None };
        steps.push(Deployment::Spread {
            slot: card.slot,
            points,
            count: count.unwrap_or(30),
            burst: count.unwrap_or(if quake { 1 } else { 4 }),
            fingers: ring,
            spaced: false,
        });
    }
    if ring {
        for card in troops {
            steps.push(Deployment::Spread {
                slot: card.slot,
                points: layout.ring.clone(),
                count: card.count.unwrap_or(60),
                burst: 20,
                fingers: true,
                spaced: true,
            });
        }
    }
    Ok(steps)
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

pub fn hero_ready(image: &Mat, slot: Point) -> Result<bool> {
    let colour = hsv(image)?;
    let top = slot.1.checked_sub(89).ok_or(Error::Unreadable)?;
    let left = slot.0.checked_sub(36).ok_or(Error::Unreadable)?;
    for (offset, width) in [(0, 20), (22, 19), (43, 30)] {
        if share(&colour, Region(left + offset, top, width, 8), |p| {
            (140..=165).contains(&p[0]) && p[1] > 100 && p[2] > 150
        })? < 0.5
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn stars(image: &Mat, regions: &[Region]) -> Result<u8> {
    if regions.is_empty() || regions.len() > 6 {
        return Err(Error::Unreadable);
    }
    let colour = hsv(image)?;
    let mut count = 0;
    for &region in regions {
        if share(&colour, region, |p| {
            (8..=30).contains(&p[0]) && p[1] >= 120 && p[2] >= 150
        })? >= 0.25
        {
            count += 1;
        }
    }
    Ok(count)
}
