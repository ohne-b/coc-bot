use super::{Card, Deployment, Kind};
use crate::{Error, Result, calibration::Resolution, device::Point};
use opencv::core::Mat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub struct BoundCard {
    pub logical_slot: usize,
    pub card: Card,
    pub portrait: Vec<u8>,
    pub builder: bool,
    hero_head: Option<Vec<u8>>,
}

impl BoundCard {
    pub fn new(card: &Card, image: &Mat, builder: bool) -> Result<Self> {
        let hero_head = if (card.kind == Kind::Hero || (!builder && card.kind == Kind::Warden))
            && super::resolution(image)? == Resolution(1280, 720)
        {
            let lift = if super::hero_deployed(image, card.slot)?
                || (builder && builder_hero_meter(image, card.slot)?)
                || (!builder && super::empty(image, card.slot)?)
            {
                20
            } else {
                0
            };
            Some(capture_hero_head(image, card.slot, lift)?)
        } else {
            None
        };
        Ok(Self {
            logical_slot: 0,
            card: card.clone(),
            portrait: portrait(image, card.slot)?,
            builder,
            hero_head,
        })
    }

    pub fn remaining(&self, image: &Mat, digits: &super::Digits) -> Result<Option<u32>> {
        let raised = matches!(self.card.kind, Kind::Hero | Kind::Warden | Kind::Siege)
            && super::hero_deployed(image, self.card.slot)?;
        let empty = super::empty(image, self.card.slot)?;
        if let Some(head) = &self.hero_head
            && ((self.builder && (raised || builder_hero_meter(image, self.card.slot)?))
                || (!self.builder && empty))
            && matches_hero_head(image, self.card.slot, head)?
        {
            return Ok(Some(0));
        }
        if !matches_portrait(image, &self.card, &self.portrait, raised && !self.builder)? {
            return Ok(None);
        }
        if empty {
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
            let (x, width) = if !self.builder && super::selected(image, self.card.slot)? {
                // The raised card's white outline must stay outside the numeric crop.
                (self.card.slot.0.saturating_sub(16), 61)
            } else {
                (x, w + 6)
            };
            return Ok(digits
                .card_count(
                    &super::crop(
                        image,
                        crate::calibration::Region(x, top, width, h + y - top),
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
            if swap
                || self.builder_hero_waiting(image, &colour)?
                || super::hero_waiting_health(image, slot)?
            {
                return Ok(Some(1));
            }
            return Ok(raised.then_some(0));
        }
        Ok(Some(1))
    }

    fn builder_hero_waiting(&self, image: &Mat, colour: &Mat) -> Result<bool> {
        let x = self.card.slot.0.checked_sub(32).ok_or(Error::Unreadable)?;
        let top = self.card.region.1;
        let blue = |p: opencv::core::Vec3b| {
            ((80..=175).contains(&p[0]) && p[1] > 90 && p[2] > 80) || (p[1] < 60 && p[2] > 200)
        };
        let selected = super::selected(image, self.card.slot)?;
        let rows = if selected {
            top.saturating_sub(8)..=top + 4
        } else {
            top + 4..=top + 14
        };
        let height = if selected { 3 } else { 2 };
        for y in rows {
            // Selection raises and thickens the white frame; deployment replaces it with a health bar.
            if super::share(colour, crate::calibration::Region(x, y - 1, 64, 1), |p| {
                p[2] < 60
            })? > 0.85
                && super::share(colour, crate::calibration::Region(x, y, 64, height), |p| {
                    if selected {
                        p[1] < 60 && p[2] > 200
                    } else {
                        blue(p)
                    }
                })? > 0.85
                && super::share(
                    colour,
                    crate::calibration::Region(x, y + height, 64, 1),
                    blue,
                )? < 0.75
            {
                return Ok(true);
            }
        }
        Ok(false)
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
            if builder_hero_meter(image, self.card.slot)? {
                Some(super::hero_charge(image, self.card.slot)? > 0)
            } else {
                None
            }
        } else {
            self.builder_troop_state(&super::hsv(image)?)?
                .map(|s| s == 1)
        })
    }
}

fn capture_hero_head(image: &Mat, slot: Point, lift: u32) -> Result<Vec<u8>> {
    use opencv::{core, imgproc, prelude::*};
    let patch = super::crop(
        image,
        crate::calibration::Region(
            slot.0.checked_sub(1).ok_or(Error::Unreadable)?,
            slot.1.checked_sub(40 + lift).ok_or(Error::Unreadable)?,
            20,
            32,
        ),
    )?;
    let mut gray = Mat::default();
    imgproc::cvt_color(
        &patch,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    Ok(gray.data_bytes()?.to_vec())
}

fn matches_hero_head(image: &Mat, slot: Point, saved: &[u8]) -> Result<bool> {
    use opencv::{
        core::{self, Rect, Scalar, Size},
        imgproc,
        prelude::*,
    };
    if saved.len() != 640 {
        return Ok(false);
    }
    fn contrast(image: &impl core::ToInputArray) -> Result<bool> {
        let mut deviation = Scalar::default();
        core::mean_std_dev(
            image,
            &mut core::no_array(),
            &mut deviation,
            &core::no_array(),
        )?;
        Ok(deviation[0] > 5.)
    }
    let pixels = Mat::from_slice(saved)?;
    let reference = pixels.reshape(1, 32)?;
    if !contrast(&reference)? {
        return Ok(false);
    }
    let patch = super::crop(
        image,
        crate::calibration::Region(
            slot.0.checked_sub(5).ok_or(Error::Unreadable)?,
            slot.1.checked_sub(68).ok_or(Error::Unreadable)?,
            28,
            52,
        ),
    )?;
    let mut gray = Mat::default();
    imgproc::cvt_color(
        &patch,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    // The charged card's glow covers its lower portrait; the fresh head reference stays separate.
    for size in [Size::new(19, 30), Size::new(20, 32), Size::new(21, 34)] {
        let mut template = Mat::default();
        imgproc::resize(&reference, &mut template, size, 0., 0., imgproc::INTER_AREA)?;
        let mut result = Mat::default();
        imgproc::match_template_def(&gray, &template, &mut result, imgproc::TM_CCOEFF_NORMED)?;
        let mut score = 0.;
        let mut at = core::Point::default();
        core::min_max_loc(
            &result,
            None,
            Some(&mut score),
            None,
            Some(&mut at),
            &core::no_array(),
        )?;
        if score > 0.9
            && contrast(&Mat::roi(
                &gray,
                Rect::new(at.x, at.y, size.width, size.height),
            )?)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn builder_hero_meter(image: &Mat, slot: Point) -> Result<bool> {
    use crate::calibration::Region;
    use opencv::core;
    let x = slot.0.checked_sub(38).ok_or(Error::Unreadable)?;
    let y = slot.1.checked_sub(99).ok_or(Error::Unreadable)?;
    let mut deviation = core::Scalar::default();
    core::mean_std_dev(
        &super::crop(image, Region(x, y, 75, 19))?,
        &mut core::no_array(),
        &mut deviation,
        &core::no_array(),
    )?;
    if deviation.0[..3].iter().all(|value| *value <= 5.) {
        return Ok(false);
    }
    let colour = super::hsv(image)?;
    let dark = |p: opencv::core::Vec3b| *p.iter().max().unwrap() < 35;
    let mut matched = 0.;
    let mut area = 0.;
    for (dx, dy, w, h) in [
        (5, 8, 15, 1),
        (27, 4, 15, 1),
        (49, 0, 21, 1),
        (0, 12, 1, 6),
        (22, 8, 1, 10),
        (44, 4, 1, 14),
        (74, 4, 1, 14),
    ] {
        let region = Region(x + dx, y + dy, w, h);
        let fraction = if h == 1 || dx == 22 || dx == 44 {
            super::share(&colour, region, |p| {
                p[2] < 35
                    || ((85..=175).contains(&p[0]) && p[1] > 60 && p[2] > 80)
                    || (p[1] < 60 && p[2] > 200)
            })?
        } else {
            super::share(image, region, dark)?
        };
        if fraction < 0.8 {
            return Ok(false);
        }
        matched += fraction * f64::from(w * h);
        area += f64::from(w * h);
    }
    if matched / area < 0.95 {
        return Ok(false);
    }
    for (dx, dy, w, h) in [(4, 12, 15, 6), (26, 8, 15, 10), (48, 4, 23, 14)] {
        if super::share(&colour, Region(x + dx, y + dy, w, h), |p| {
            (20..=100).contains(&p[2]) || ((85..=175).contains(&p[0]) && p[1] > 60 && p[2] > 80)
        })? < 0.85
        {
            return Ok(false);
        }
    }
    Ok(true)
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

fn siege_sides_match(current: &[u8], saved: &[u8]) -> bool {
    // The siege warning badge bobs over its centre; both flanks must still match the fresh artwork.
    for dy in -2..=2 {
        for dx in -2..=2 {
            if [2..5, 11..14].into_iter().all(|columns| {
                let mut error = 0.;
                let mut pairs = Vec::with_capacity(36);
                for y in 2..14 {
                    for x in columns.clone() {
                        let a = ((y + dy) * 16 + x + dx) as usize * 3;
                        let b = (y * 16 + x) as usize * 3;
                        for channel in 0..3 {
                            error += (f64::from(current[a + channel])
                                - f64::from(saved[b + channel]))
                            .abs();
                        }
                        let gray = |p: &[u8], i: usize| {
                            f64::from(p[i]) * 0.114
                                + f64::from(p[i + 1]) * 0.587
                                + f64::from(p[i + 2]) * 0.299
                        };
                        pairs.push((gray(current, a), gray(saved, b)));
                    }
                }
                let n = pairs.len() as f64;
                let a = pairs.iter().map(|p| p.0).sum::<f64>() / n;
                let b = pairs.iter().map(|p| p.1).sum::<f64>() / n;
                let va = pairs.iter().map(|p| (p.0 - a).powi(2)).sum::<f64>();
                let vb = pairs.iter().map(|p| (p.1 - b).powi(2)).sum::<f64>();
                let cross = pairs.iter().map(|p| (p.0 - a) * (p.1 - b)).sum::<f64>();
                error / (n * 3.) <= 18.
                    && va > n * 25.
                    && vb > n * 25.
                    && cross / (va * vb).sqrt() > 0.9
            }) {
                return true;
            }
        }
    }
    false
}

pub(super) fn matches_portrait(
    image: &Mat,
    card: &Card,
    saved: &[u8],
    raised: bool,
) -> Result<bool> {
    use opencv::{
        core::{Mat, Size},
        imgproc,
        prelude::*,
    };
    let slot = card.slot;
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
                || (card.kind == Kind::Siege && siege_sides_match(pixels, saved))
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
    /// One-based position in the complete army bar, left to right.
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
    pub count: Option<u32>,
}
pub fn inventory(cards: &[Card], builder: bool) -> Vec<Unit> {
    cards
        .iter()
        .enumerate()
        .map(|(i, c)| Unit {
            supports_ability: matches!(c.kind, Kind::Hero | Kind::Warden)
                || (builder && c.kind == Kind::Counted),
            kind: unit_kind(cards, c, builder),
            slot: i + 1,
            count: available(c, builder),
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

pub(super) fn available(card: &Card, builder: bool) -> Option<u32> {
    if builder && card.count == Some(0) {
        return Some(0);
    }
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
            if !(1..=64).contains(&step.slot)
                || step.portrait.len() > 768
                || step.label.chars().count() > 40
                || step.count > 999
                || step.delay_ms > 180_000
                || !(100..=10_000).contains(&step.interval_ms)
                || step.point.1 > 1000
                || step.point.0 > 1000
                || (!step.ability
                    && step.kind != UnitKind::Spell
                    && ![step.point.0, step.point.1]
                        .iter()
                        .any(|p| *p == 0 || *p == 1000))
                || (step.ability && !matches!(step.kind, UnitKind::Hero | UnitKind::Troop))
            {
                return Err(Error::Calibration(
                    "Invalid deployment action or position".into(),
                ));
            }
            duration += u64::from(step.delay_ms)
                + if step.ability {
                    0
                } else {
                    u64::from(step.count) * u64::from(step.interval_ms)
                };
        }
        if duration > 180_000 {
            return Err(Error::Calibration(
                "Deployment waits and intervals exceed three minutes".into(),
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
        let bound = cards
            .iter()
            .enumerate()
            .map(|(i, card)| {
                let mut bound = BoundCard::new(card, image, builder)?;
                bound.logical_slot = i + 1;
                Ok(bound)
            })
            .collect::<Result<Vec<_>>>()?;
        self.compile_bound(&bound)
    }

    pub fn compile_roster(&self, roster: &super::roster::Roster) -> Result<Vec<Deployment>> {
        self.compile_bound(&roster.bound_cards())
    }

    fn compile_bound(&self, bound: &[BoundCard]) -> Result<Vec<Deployment>> {
        self.validate(Resolution(1280, 720))?;
        let builder = bound.first().ok_or(Error::Unreadable)?.builder;
        let cards: Vec<_> = bound.iter().map(|b| b.card.clone()).collect();
        let kind = |c: &Card| unit_kind(&cards, c, builder);
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
            if kind(card) != action.kind {
                return Err(Error::Gameplay(format!(
                    "Army slot {} does not match the planned unit type",
                    action.slot
                )));
            }
            if builder && available(card, builder) == Some(0) {
                continue;
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
                    card: bound[i].clone(),
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
                    card: bound[i].clone(),
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
