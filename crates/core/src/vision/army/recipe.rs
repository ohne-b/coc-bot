use super::{Card, Deployment, Kind};
use crate::{Error, Result, calibration::Resolution, device::Point};
use opencv::core::Mat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub struct BoundCard {
    pub card: Card,
    pub portrait: Vec<u8>,
    pub builder: bool,
}

impl BoundCard {
    pub fn new(card: &Card, image: &Mat, builder: bool) -> Result<Self> {
        Ok(Self {
            card: card.clone(),
            portrait: portrait(image, card.slot)?,
            builder,
        })
    }

    pub fn remaining(&self, image: &Mat, digits: &super::Digits) -> Result<Option<u32>> {
        let raised = matches!(self.card.kind, Kind::Hero | Kind::Warden | Kind::Siege)
            && super::hero_deployed(image, self.card.slot)?;
        if !matches_portrait(
            image,
            self.card.slot,
            &self.portrait,
            raised && !self.builder,
        )? {
            return Ok(None);
        }
        if super::empty(image, self.card.slot)? {
            return Ok(Some(0));
        }
        if !self.builder
            && matches!(self.card.kind, Kind::Hero | Kind::Warden | Kind::Siege)
            && raised
        {
            return Ok(Some(0));
        }
        if matches!(self.card.kind, Kind::Sized)
            || (self.card.kind == Kind::Counted && !self.builder)
        {
            let Some(crate::calibration::Region(x, y, w, h)) = self.card.count_region else {
                return Ok(None);
            };
            let top = y.saturating_sub(16);
            return Ok(digits
                .card_count(
                    &super::crop(
                        image,
                        crate::calibration::Region(x, top, w + 6, h + y - top),
                    )?,
                    self.builder,
                )
                .ok());
        }
        let colour = super::hsv(image)?;
        let slot = self.card.slot;
        let green = |p: opencv::core::Vec3b| (35..=85).contains(&p[0]) && p[1] > 100 && p[2] > 80;
        if self.builder && self.card.kind == Kind::Counted {
            return Ok(self
                .builder_troop_state(&colour)?
                .map(|state| u32::from(state == 0)));
        }
        if self.builder {
            // The swap button remains on a surviving hero before its stage-two placement.
            let swap = super::share(
                &colour,
                crate::calibration::Region(slot.0, slot.1 + 25, 38, 26),
                green,
            )? > 0.25;
            if swap {
                return Ok(Some(1));
            }
            return Ok(raised.then_some(0));
        }
        Ok(Some(1))
    }

    fn builder_troop_state(&self, colour: &Mat) -> Result<Option<u8>> {
        let slot = self.card.slot;
        let region = crate::calibration::Region(
            slot.0.checked_sub(32).ok_or(Error::Unreadable)?,
            slot.1.checked_sub(42).ok_or(Error::Unreadable)?,
            64,
            10,
        );
        let waiting_header = crate::calibration::Region(
            slot.0 + 3,
            slot.1.checked_sub(46).ok_or(Error::Unreadable)?,
            30,
            6,
        );
        if super::share(colour, waiting_header, |p| {
            (90..=170).contains(&p[0]) && p[1] > 100 && p[2] > 60
        })? > 0.8
        {
            return Ok(Some(0));
        }
        let health = super::share(colour, region, |p| p[0] <= 85 && p[1] > 110 && p[2] > 90)?;
        if health
            + super::share(colour, region, |p| {
                (p[1] < 60 || p[0] <= 85) && (25..=100).contains(&p[2])
            })?
            > 0.85
        {
            // Ability charge is the raised header above the health bar, not health itself.
            let header = crate::calibration::Region(
                slot.0 + 3,
                slot.1.checked_sub(76).ok_or(Error::Unreadable)?,
                30,
                20,
            );
            if super::share(colour, header, |p| {
                (135..=165).contains(&p[0]) && p[1] > 100 && p[2] > 120
            })? > 0.5
            {
                return Ok(Some(1));
            }
            if super::share(colour, header, |p| p[1] < 60 && (20..=100).contains(&p[2]))? > 0.5 {
                return Ok(Some(2));
            }
        }
        Ok(None)
    }

    pub fn ability_ready(&self, image: &Mat, digits: &super::Digits) -> Result<Option<bool>> {
        if self.remaining(image, digits)? != Some(0) {
            return Ok(None);
        }
        if !self.builder {
            let pixels = portrait(image, self.card.slot)?;
            let saturation = pixels
                .chunks_exact(3)
                .map(|p| {
                    let max = *p.iter().max().unwrap();
                    let min = *p.iter().min().unwrap();
                    if max == 0 {
                        0.
                    } else {
                        f64::from(max - min) / f64::from(max) * 255.
                    }
                })
                .sum::<f64>()
                / 256.;
            return Ok(Some(saturation >= 20.));
        }
        if super::empty(image, self.card.slot)? {
            return Ok(Some(false));
        }
        Ok(if self.card.kind == Kind::Hero {
            Some(super::hero_charge(image, self.card.slot)? > 0)
        } else {
            self.builder_troop_state(&super::hsv(image)?)?
                .map(|s| s == 1)
        })
    }
}

fn gray_match(current: &[u8], saved: &[u8], face: bool) -> bool {
    for dy in -2..=2 {
        for dx in -2..=2 {
            let mut pairs = Vec::with_capacity(144);
            for y in 2..if face { 8 } else { 14 } {
                for x in 2..14 {
                    let gray = |p: &[u8], i: usize| {
                        f64::from(p[i]) * 0.114
                            + f64::from(p[i + 1]) * 0.587
                            + f64::from(p[i + 2]) * 0.299
                    };
                    pairs.push((
                        gray(current, ((y + dy) * 16 + x + dx) as usize * 3),
                        gray(saved, (y * 16 + x) as usize * 3),
                    ));
                }
            }
            let n = pairs.len() as f64;
            let a = pairs.iter().map(|p| p.0).sum::<f64>() / n;
            let b = pairs.iter().map(|p| p.1).sum::<f64>() / n;
            let va = pairs.iter().map(|p| (p.0 - a).powi(2)).sum::<f64>();
            let vb = pairs.iter().map(|p| (p.1 - b).powi(2)).sum::<f64>();
            let cross = pairs.iter().map(|p| (p.0 - a) * (p.1 - b)).sum::<f64>();
            if va > n * 25. && vb > n * 25. && cross / (va * vb).sqrt() > 0.9 {
                return true;
            }
        }
    }
    false
}

fn matches_portrait(image: &Mat, slot: Point, saved: &[u8], raised: bool) -> Result<bool> {
    use opencv::{
        core::{Mat, Size},
        imgproc,
        prelude::*,
    };
    for (w, h) in [(48, 40), (44, 32), (56, 46), (64, 54)] {
        for lift in [-16_i32, -8, 0, 8, 16, 20, 24, 32, 40] {
            let patch = super::crop(
                image,
                crate::calibration::Region(
                    slot.0.checked_sub(w / 2).ok_or(Error::Unreadable)?,
                    slot.1
                        .checked_add_signed(-(h as i32 / 2) - lift)
                        .ok_or(Error::Unreadable)?,
                    w,
                    h,
                ),
            )?;
            let mut small = Mat::default();
            imgproc::resize(
                &patch,
                &mut small,
                Size::new(16, 16),
                0.,
                0.,
                imgproc::INTER_AREA,
            )?;
            let pixels = small.data_bytes()?;
            // Raised hero cards replace the lower portrait with ability artwork; retain the face.
            let face = raised
                && (0..8)
                    .flat_map(|y| {
                        (2..14).flat_map(move |x| (0..3).map(move |c| (y * 16 + x) * 3 + c))
                    })
                    .map(|i| (f64::from(pixels[i]) - f64::from(saved[i])).abs())
                    .sum::<f64>()
                    / 288.
                    <= 18.;
            if portrait_difference(pixels, saved) <= 18.
                || gray_match(pixels, saved, false)
                || face
                || (raised && gray_match(pixels, saved, true))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub supports_ability: bool,
    #[serde(default)]
    pub portrait: Vec<u8>,
    pub label: String,
    /// One-based position in the visible army bar, left to right.
    pub slot: usize,
    pub kind: UnitKind,
    pub point: Point,
    /// Zero means the remaining recognized count, never a guessed tap count.
    pub count: u32,
    pub delay_ms: u32,
    pub interval_ms: u32,
    pub ability: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Troop,
    Hero,
    Siege,
    Spell,
}

#[derive(Clone, Debug, Serialize)]
pub struct Unit {
    pub supports_ability: bool,
    pub kind: UnitKind,
    pub slot: usize,
    pub center: Point,
    pub count: Option<u32>,
    pub portrait: Vec<u8>,
}
pub fn inventory(cards: &[Card], image: &opencv::core::Mat, builder: bool) -> Result<Vec<Unit>> {
    cards
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Ok(Unit {
                supports_ability: matches!(c.kind, Kind::Hero | Kind::Warden)
                    || (builder && c.kind == Kind::Counted),
                kind: unit_kind(cards, c, builder),
                slot: i + 1,
                center: c.slot,
                count: available(c, builder),
                portrait: portrait(image, c.slot)?,
            })
        })
        .collect()
}

pub fn unit_kind(cards: &[Card], card: &Card, builder: bool) -> UnitKind {
    let last_group = cards
        .iter()
        .filter(|c| c.kind == Kind::Counted)
        .map(|c| c.group)
        .max()
        .unwrap_or(0);
    match card.kind {
        Kind::Hero | Kind::Warden => UnitKind::Hero,
        Kind::Siege => UnitKind::Siege,
        Kind::Counted if !builder && card.group > 0 && card.group == last_group => UnitKind::Spell,
        Kind::Counted if !builder && card.group > 0 => UnitKind::Siege,
        _ => UnitKind::Troop,
    }
}

fn available(card: &Card, builder: bool) -> Option<u32> {
    match card.kind {
        Kind::Hero | Kind::Warden | Kind::Siege => Some(1),
        Kind::Counted if builder => Some(1),
        _ => card.count,
    }
}

impl Recipe {
    pub fn validate(&self, _resolution: Resolution) -> Result<()> {
        if self.steps.is_empty() || self.steps.len() > 64 {
            return Err(Error::Calibration(
                "Add between 1 and 64 deployment actions".into(),
            ));
        }
        let mut duration = 0u64;
        for step in &self.steps {
            if step.portrait.len() != 768
                || !(1..=20).contains(&step.slot)
                || step.label.chars().count() > 40
                || step.count > 300
                || step.delay_ms > 180_000
                || !(100..=10_000).contains(&step.interval_ms)
                || step.point.1 > 1000
                || step.point.0 > 1000
                || (!step.ability
                    && step.kind != UnitKind::Spell
                    && ![step.point.0, step.point.1]
                        .iter()
                        .any(|p| *p == 0 || *p == 1000))
                || (!step.ability
                    && step.kind != UnitKind::Spell
                    && ![step.point.0, step.point.1]
                        .iter()
                        .any(|p| (120..=880).contains(p)))
                || (step.ability && !matches!(step.kind, UnitKind::Hero | UnitKind::Troop))
            {
                return Err(Error::Calibration(
                    "Invalid deployment action or position".into(),
                ));
            }
            duration += u64::from(step.delay_ms);
        }
        if duration > 180_000 {
            return Err(Error::Calibration(
                "Deployment delays exceed three minutes".into(),
            ));
        }
        Ok(())
    }

    pub fn compile(
        &self,
        cards: &[Card],
        image: &opencv::core::Mat,
        builder: bool,
    ) -> Result<Vec<Deployment>> {
        self.validate(Resolution(1280, 720))?;
        let kind = |c: &Card| unit_kind(cards, c, builder);
        let mut remaining: Vec<_> = cards.iter().map(|c| available(c, builder)).collect();
        let mut deployed = vec![false; cards.len()];
        let mut activated = vec![false; cards.len()];
        let mut steps = Vec::new();
        let mut duration = 0u64;
        for action in &self.steps {
            let i = action.slot - 1;
            let card = cards
                .get(i)
                .ok_or_else(|| Error::Gameplay(format!("Army slot {} is missing", action.slot)))?;
            let current = portrait(image, card.slot)?;
            if portrait_difference(&current, &action.portrait) > 18.0 {
                return Err(Error::Gameplay(format!(
                    "Army slot {} changed; bind this action to the current army",
                    action.slot
                )));
            }
            if kind(card) != action.kind {
                return Err(Error::Gameplay(format!(
                    "Army slot {} does not match the planned unit type",
                    action.slot
                )));
            }
            if action.delay_ms > 0 {
                steps.push(Deployment::Wait(action.delay_ms));
            }
            duration += u64::from(action.delay_ms);
            if action.ability {
                if builder && card.kind == Kind::Sized {
                    return Err(Error::Calibration(
                        "This troop uses a passive ability; no activation action is supported"
                            .into(),
                    ));
                }
                if !deployed[i] || (!builder && (activated[i] || action.kind != UnitKind::Hero)) {
                    return Err(Error::Calibration(
                        "A hero ability must follow its placement and occur once".into(),
                    ));
                }
                steps.push(Deployment::PreciseAbility {
                    card: BoundCard::new(card, image, builder)?,
                });
                activated[i] = true;
            } else {
                let available = remaining[i].ok_or_else(|| {
                    Error::Gameplay(format!("Count for army slot {} is unreadable", action.slot))
                })?;
                let count = if action.count == 0 {
                    available
                } else {
                    action.count
                };
                if count == 0 || count > available {
                    return Err(Error::Gameplay(format!(
                        "Army slot {} has insufficient units for this plan",
                        action.slot
                    )));
                }
                remaining[i] = Some(available - count);
                duration += u64::from(count) * u64::from(action.interval_ms);
                deployed[i] = true;
                steps.push(Deployment::PreciseDrop {
                    card: BoundCard::new(card, image, builder)?,
                    point: action.point,
                    count,
                    interval_ms: action.interval_ms,
                    initial_count: available,
                });
            }
        }
        if duration > 180_000 {
            return Err(Error::Calibration(
                "Deployment waits and intervals exceed three minutes".into(),
            ));
        }
        Ok(steps)
    }
}

pub(super) fn portrait_difference(current: &[u8], saved: &[u8]) -> f64 {
    // A selected army card rises and grows slightly; compare its inner portrait over that motion.
    let mut best = current
        .iter()
        .zip(saved)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
        .sum::<f64>()
        / 768.;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let mut total = 0.;
            for y in 2..14 {
                for x in 2..14 {
                    for channel in 0..3 {
                        let a = current[((y + dy) * 16 + x + dx) as usize * 3 + channel];
                        let b = saved[(y * 16 + x) as usize * 3 + channel];
                        total += (f64::from(a) - f64::from(b)).abs();
                    }
                }
            }
            best = best.min(total / (12. * 12. * 3.));
        }
    }
    best
}

pub fn portrait(image: &opencv::core::Mat, slot: Point) -> Result<Vec<u8>> {
    use opencv::{
        core::{Mat, Size},
        imgproc,
        prelude::*,
    };
    let patch = super::crop(
        image,
        crate::calibration::Region(
            slot.0.checked_sub(24).ok_or(Error::Unreadable)?,
            slot.1.checked_sub(20).ok_or(Error::Unreadable)?,
            48,
            40,
        ),
    )?;
    let mut small = Mat::default();
    imgproc::resize(
        &patch,
        &mut small,
        Size::new(16, 16),
        0.0,
        0.0,
        imgproc::INTER_AREA,
    )?;
    Ok(small.data_bytes()?.to_vec())
}
