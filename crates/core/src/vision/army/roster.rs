use super::{
    Card, Digits,
    recipe::{self, BoundCard, Unit},
};
use crate::{
    Error, Result,
    calibration::{Region, Resolution},
    device::Point,
};
use opencv::{
    core::{self, Mat, Rect, Scalar},
    imgproc,
    prelude::*,
};

const MAX_SLOTS: usize = 64;
const MAX_CAPTURES: u16 = 512;
const MAX_SETTLE_CAPTURES: u8 = 8;
const EDGE: i32 = 8;
const SHORT_SCROLL: i32 = 48;
const RETURN_SCROLL: i32 = 64;

fn unreadable(message: &str) -> Error {
    Error::Gameplay(format!("Army bar {message}"))
}

/// The caller executes this drag, waits for the bar to settle, then supplies a capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pan {
    pub from: Point,
    pub to: Point,
}
impl Pan {
    fn dx(self) -> i32 {
        self.to.0 as i32 - self.from.0 as i32
    }
    fn limit(self) -> i32 {
        if self.dx().abs() <= RETURN_SCROLL {
            44
        } else {
            360
        }
    }

    fn measurement_limit(self) -> i32 {
        self.limit().max(self.dx().abs())
    }
}

pub enum ScanStep {
    Pan(Pan),
    /// Wait 250ms without input and provide another capture while scrolling settles.
    Wait,
    Complete(Roster),
}

pub enum Locate {
    Pan(Pan),
    /// Wait 250ms without input while a scroll or card animation settles.
    Wait,
    Ready(BoundCard),
}

struct Settling {
    slot: Option<usize>,
    pan: Pan,
    stable: u8,
    captures: u8,
    distance: u32,
    rebound: u32,
    total: i32,
}

impl Settling {
    fn new(slot: Option<usize>, pan: Pan, dx: i32, at_edge: bool) -> Result<Self> {
        let mut state = Self {
            slot,
            pan,
            stable: 0,
            captures: 0,
            distance: 0,
            rebound: 0,
            total: 0,
        };
        state.movement(dx, at_edge)?;
        Ok(state)
    }

    fn movement(&mut self, dx: i32, at_edge: bool) -> Result<()> {
        self.distance += dx.unsigned_abs();
        self.total += dx;
        if dx.abs() > 2 && dx.signum() != self.pan.dx().signum() {
            self.rebound += dx.unsigned_abs();
            if self.rebound > 12 || !at_edge {
                return Err(unreadable("reversed direction away from its settling edge"));
            }
        }
        if self.distance > self.pan.limit() as u32 {
            return Err(unreadable("did not settle within its scroll distance"));
        }
        Ok(())
    }

    fn observe(&mut self, dx: i32, at_edge: bool) -> Result<()> {
        self.movement(dx, at_edge)?;
        self.captures += 1;
        self.stable = if dx.abs() <= 2 { self.stable + 1 } else { 0 };
        if self.captures >= MAX_SETTLE_CAPTURES && self.stable < 2 {
            return Err(unreadable("did not settle after scrolling"));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Entry {
    bound: BoundCard,
    x: i32,
    left: i32,
    width: u32,
}

/// A roster is scoped to one attack and one verified horizontal coordinate system.
pub struct Roster {
    bar: Region,
    builder: bool,
    entries: Vec<Entry>,
    offset: i32,
    max_offset: i32,
    previous: Mat,
    pending: Option<Pan>,
    pending_refused: bool,
    settling: Option<Settling>,
    stalled_scrolls: u8,
    unreadable: Option<(usize, u8)>,
    captures: u16,
    failed: bool,
}

impl Roster {
    pub fn inventory(&self) -> Vec<Unit> {
        let cards: Vec<_> = self.entries.iter().map(|e| e.bound.card.clone()).collect();
        self.entries
            .iter()
            .map(|e| Unit {
                supports_ability: matches!(
                    e.bound.card.kind,
                    super::Kind::Hero | super::Kind::Warden
                ) || (self.builder && e.bound.card.kind == super::Kind::Counted),
                kind: recipe::unit_kind(&cards, &e.bound.card, self.builder),
                slot: e.bound.logical_slot,
                count: recipe::available(&e.bound.card, self.builder),
            })
            .collect()
    }

    pub(super) fn bound_cards(&self) -> Vec<BoundCard> {
        self.entries.iter().map(|e| e.bound.clone()).collect()
    }

    /// Only a guarded refusal before dispatch can cancel the expected movement.
    pub fn cancel_pending_scroll(&mut self) -> Result<()> {
        if self.failed || self.pending.is_none() || self.pending_refused {
            return Err(unreadable("has no pending scroll to cancel"));
        }
        self.pending_refused = true;
        Ok(())
    }

    /// Rebinds the slot by measured translation, never by searching for its portrait.
    pub fn locate(&mut self, slot: usize, image: &Mat, digits: &Digits) -> Result<Locate> {
        if self.failed {
            return Err(unreadable("tracking already failed"));
        }
        let result = self.locate_inner(slot, image, digits);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn locate_inner(&mut self, slot: usize, image: &Mat, digits: &Digits) -> Result<Locate> {
        if self.unreadable.is_some_and(|(target, _)| target != slot) {
            return Err(unreadable(
                "requested slot changed while its card was unreadable",
            ));
        }
        let entry = self
            .entries
            .get(
                slot.checked_sub(1)
                    .ok_or_else(|| unreadable("slot is invalid"))?,
            )
            .ok_or_else(|| unreadable("slot is missing"))?
            .clone();
        let regions = visible_regions(&self.entries, self.offset, self.bar);
        let refused = std::mem::take(&mut self.pending_refused);
        let pending = if refused {
            self.pending = None;
            None
        } else {
            self.pending.take()
        };
        let mut settling = self.settling.take();
        if settling
            .as_ref()
            .is_some_and(|state| state.slot != Some(slot))
        {
            return Err(unreadable(
                "requested slot changed before scrolling settled",
            ));
        }
        let dx = measure_translation(
            &self.previous,
            image,
            self.bar,
            &regions,
            pending.map_or_else(
                || {
                    settling
                        .as_ref()
                        .map_or(6, |state| state.pan.measurement_limit())
                },
                Pan::measurement_limit,
            ),
            pending.is_none() && settling.is_none() && !refused,
        )?;
        if let Some(pan) = pending {
            if dx.abs() > 2 && dx.signum() != pan.dx().signum() {
                return Err(unreadable("did not move toward the requested slot"));
            }
            settling = Some(Settling::new(Some(slot), pan, dx, false)?);
        } else if let Some(state) = &mut settling {
            let reversing = dx.abs() > 2 && dx.signum() != state.pan.dx().signum();
            let at_edge = reversing
                && self.endpoint_visible(image, digits, state.pan.dx() > 0, self.offset - dx)?;
            state.observe(dx, at_edge)?;
        } else if dx.abs() > 2 {
            return Err(unreadable("moved without a requested scroll"));
        }
        if pending.is_some() || settling.is_some() {
            self.offset -= dx;
        }
        let outside_range = self.offset < -3 || self.offset > self.max_offset + 3;
        if outside_range {
            let excess = (-self.offset).max(self.offset - self.max_offset);
            if (pending.is_none() && settling.is_none())
                || excess > 12
                || !self.endpoint_visible(image, digits, self.offset < 0, self.offset)?
            {
                return Err(unreadable("moved outside the scanned range"));
            }
        }
        self.previous = image.try_clone()?;
        self.captures += 1;
        if self.captures > MAX_CAPTURES {
            return Err(unreadable("scroll limit reached"));
        }
        let left = entry.left - self.offset;
        let right = left + entry.width as i32;
        // Overflow arrows also fade the adjacent border needed to prove card selection.
        let clear_left = self.bar.0 as i32
            + if overflow_arrow(image, self.bar, false)? {
                56
            } else {
                EDGE
            };
        let clear_right = (self.bar.0 + self.bar.2) as i32
            - if overflow_arrow(image, self.bar, true)? {
                56
            } else {
                EDGE
            };
        if let Some(state) = settling {
            if state.stable < 2 {
                self.settling = Some(state);
                return Ok(Locate::Wait);
            }
            if outside_range {
                return Err(unreadable("did not return to the scanned range"));
            }
            self.stalled_scrolls = if state.total.abs() <= 2 {
                self.stalled_scrolls + 1
            } else {
                0
            };
        }
        if left >= clear_left && right <= clear_right {
            let mut bound = reposition(&entry.bound, entry.x - self.offset)?;
            let mut count = bound.remaining(image, digits)?;
            if count.is_none() {
                // Rounded scroll measurements can miss a narrow portrait patch by a pixel or two.
                let mut current = super::cards(image, self.bar, self.builder, digits)?
                    .into_iter()
                    .filter(|card| {
                        card.kind == entry.bound.card.kind
                            && card.region.2 >= 78
                            && card.region.2.abs_diff(entry.width) <= 8
                            && card.slot.0.abs_diff(bound.card.slot.0) <= 3
                    });
                if let Some(card) = current.next()
                    && current.next().is_none()
                {
                    bound = reposition(&entry.bound, card.slot.0 as i32)?;
                    count = bound.remaining(image, digits)?;
                }
            }
            let Some(count) = count else {
                let attempts = self.unreadable.map_or(1, |(_, attempts)| attempts + 1);
                if attempts >= 4 {
                    return Err(unreadable("slot changed or its count cannot be read"));
                }
                self.unreadable = Some((slot, attempts));
                return Ok(Locate::Wait);
            };
            bound.card.count = Some(count);
            self.captures = 0;
            self.stalled_scrolls = 0;
            self.unreadable = None;
            return Ok(Locate::Ready(bound));
        }
        if outside_range {
            return Err(unreadable("requested slot is missing at the scanned edge"));
        }
        if self.stalled_scrolls > 2 {
            return Err(unreadable("did not move toward the requested slot"));
        }
        let direction = if left < clear_left { 1 } else { -1 };
        let current_regions = visible_regions(&self.entries, self.offset, self.bar);
        let remaining = if direction > 0 {
            self.offset
        } else {
            self.max_offset - self.offset
        };
        // A short drag can be ignored; retry only after measured stillness, within the same motion budget.
        let short_step = if self.stalled_scrolls > 0 {
            RETURN_SCROLL
        } else {
            SHORT_SCROLL
        };
        let maximum = if self.stalled_scrolls > 0 {
            short_step
        } else {
            remaining.clamp(short_step, 280)
        };
        let pan = pan(
            image,
            self.bar,
            &current_regions,
            direction,
            maximum,
            short_step,
        )?;
        self.pending = Some(pan);
        Ok(Locate::Pan(pan))
    }

    fn endpoint_visible(
        &self,
        image: &Mat,
        digits: &Digits,
        left: bool,
        offset: i32,
    ) -> Result<bool> {
        let view = full_cards(image, self.bar, self.builder, digits)?;
        let (at_edge, observed, entry) = if left {
            (
                left_end(&view, self.bar),
                view.cards.first(),
                self.entries.first(),
            )
        } else {
            (
                right_end(&view, self.bar),
                view.cards.last(),
                self.entries.last(),
            )
        };
        let (Some(card), Some(entry)) = (observed, entry) else {
            return Ok(false);
        };
        let x = entry.x - offset;
        if !at_edge || (card.slot.0 as i32 - x).abs() > 3 || card.kind != entry.bound.card.kind {
            return Ok(false);
        }
        Ok(reposition(&entry.bound, x)?
            .remaining(image, digits)?
            .is_some())
    }
}

fn reposition(bound: &BoundCard, x: i32) -> Result<BoundCard> {
    let shift = x - bound.card.slot.0 as i32;
    let shifted = |value: u32| {
        value
            .checked_add_signed(shift)
            .ok_or_else(|| unreadable("position is outside the bar"))
    };
    let mut rebound = bound.clone();
    rebound.card.slot.0 = shifted(bound.card.slot.0)?;
    rebound.card.region.0 = shifted(bound.card.region.0)?;
    if let Some(region) = &mut rebound.card.count_region {
        region.0 = shifted(region.0)?;
    }
    Ok(rebound)
}

#[derive(Clone, Copy)]
enum Phase {
    Left,
    Probe,
    ReturnLeft,
    Right,
    Finish { stable: u8, captures: u8 },
    Complete,
}

pub struct Scanner {
    bar: Region,
    builder: bool,
    phase: Phase,
    previous: Option<Mat>,
    regions: Vec<Region>,
    pending: Option<Pan>,
    pending_refused: bool,
    entries: Vec<Entry>,
    settling: Option<Settling>,
    offset: i32,
    captures: u16,
    stalled_scrolls: u8,
    failed: bool,
}

impl Scanner {
    pub fn new(bar: Region, builder: bool) -> Result<Self> {
        bar.validate(Resolution(1280, 720))?;
        if bar.2 < 800 || bar.3 < 80 {
            return Err(unreadable("calibration is too small"));
        }
        Ok(Self {
            bar,
            builder,
            phase: Phase::Left,
            previous: None,
            regions: Vec::new(),
            pending: None,
            pending_refused: false,
            entries: Vec::new(),
            settling: None,
            offset: 0,
            captures: 0,
            stalled_scrolls: 0,
            failed: false,
        })
    }

    pub fn observe(&mut self, image: &Mat, digits: &Digits) -> Result<ScanStep> {
        if self.failed {
            return Err(unreadable("scan already failed"));
        }
        let result = self.observe_inner(image, digits);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Only a guarded refusal before dispatch can cancel the expected movement.
    pub fn cancel_pending_scroll(&mut self) -> Result<()> {
        if self.failed || self.pending.is_none() || self.pending_refused {
            return Err(unreadable("has no pending scroll to cancel"));
        }
        self.pending_refused = true;
        Ok(())
    }

    fn observe_inner(&mut self, image: &Mat, digits: &Digits) -> Result<ScanStep> {
        self.captures += 1;
        if self.captures > MAX_CAPTURES {
            return Err(unreadable("scan limit reached"));
        }
        if matches!(self.phase, Phase::Complete) {
            return Err(unreadable("scan is already complete"));
        }
        let view = full_cards(image, self.bar, self.builder, digits)?;
        let cards = &view.cards;
        if std::mem::take(&mut self.pending_refused) {
            let pan = self.pending.take().ok_or(Error::InputGuard)?;
            let previous = self.previous.as_ref().ok_or(Error::InputGuard)?;
            if translation(previous, image, self.bar, &self.regions, 6)?.abs() > 2 {
                return Err(unreadable("moved after a refused scroll"));
            }
            self.previous = Some(image.try_clone()?);
            self.regions = cards.iter().map(|card| card.region).collect();
            return self.request(image, pan.dx().signum(), &view);
        }
        if self.previous.is_none() && left_end(&view, self.bar) && right_end(&view, self.bar) {
            self.stitch(cards, image)?;
            return self.complete(image);
        }
        if let Phase::Finish { stable, captures } = self.phase {
            let dx = translation(
                self.previous
                    .as_ref()
                    .ok_or_else(|| unreadable("final capture is missing"))?,
                image,
                self.bar,
                &self.regions,
                16,
            )?;
            if !right_end(&view, self.bar) || dx.abs() > 12 {
                return Err(unreadable("moved away from its final edge"));
            }
            self.offset -= dx;
            self.stitch(cards, image)?;
            self.previous = Some(image.try_clone()?);
            self.regions = cards.iter().map(|c| c.region).collect();
            let stable = if dx.abs() <= 2 { stable + 1 } else { 0 };
            if stable >= 2 {
                return self.complete(image);
            }
            if captures + 1 >= 4 {
                return Err(unreadable("final edge did not settle"));
            }
            self.phase = Phase::Finish {
                stable,
                captures: captures + 1,
            };
            return Ok(ScanStep::Wait);
        }
        let mut rebounded = false;
        let dx = if let Some(previous) = &self.previous {
            let at_edge = match self.phase {
                Phase::Left | Phase::ReturnLeft => left_end(&view, self.bar),
                Phase::Right => right_end(&view, self.bar),
                _ => false,
            };
            let state = if let Some(mut state) = self.settling.take() {
                let dx = translation(
                    previous,
                    image,
                    self.bar,
                    &self.regions,
                    state.pan.measurement_limit(),
                )?;
                state.observe(dx, at_edge)?;
                state
            } else {
                let requested = self
                    .pending
                    .take()
                    .ok_or_else(|| unreadable("capture has no requested scroll"))?;
                let dx = translation(
                    previous,
                    image,
                    self.bar,
                    &self.regions,
                    requested.measurement_limit(),
                )?;
                Settling::new(None, requested, dx, at_edge)?
            };
            if state.stable < 2 {
                self.settling = Some(state);
                self.previous = Some(image.try_clone()?);
                self.regions = cards.iter().map(|c| c.region).collect();
                return Ok(ScanStep::Wait);
            }
            rebounded = state.rebound > 0;
            Some(if rebounded && !matches!(self.phase, Phase::Right) {
                0
            } else {
                state.total
            })
        } else {
            None
        };
        self.previous = Some(image.try_clone()?);
        self.regions = cards.iter().map(|c| c.region).collect();
        let stalled_direction = if dx.is_some_and(|dx| dx.abs() <= 2) {
            match self.phase {
                Phase::Left | Phase::ReturnLeft if !left_end(&view, self.bar) => Some(1),
                Phase::Probe if !left_end(&view, self.bar) || !right_end(&view, self.bar) => {
                    Some(-1)
                }
                Phase::Right if !right_end(&view, self.bar) => Some(-1),
                _ => None,
            }
        } else {
            None
        };
        if let Some(direction) = stalled_direction {
            if self.stalled_scrolls < 2 {
                self.stalled_scrolls += 1;
                return self.request(image, direction, &view);
            }
        } else {
            self.stalled_scrolls = 0;
        }
        match self.phase {
            Phase::Left => {
                if dx.is_some_and(|dx| dx.abs() <= 2) {
                    if !left_end(&view, self.bar) {
                        return Err(unreadable("left edge is clipped or scrolling failed"));
                    }
                    self.phase = Phase::Probe;
                    return self.request(image, -1, &view);
                }
                self.request(image, 1, &view)
            }
            Phase::Probe => {
                if dx.is_some_and(|dx| dx.abs() <= 2) {
                    if !left_end(&view, self.bar) || !right_end(&view, self.bar) {
                        return Err(unreadable(
                            "did not scroll and the complete bar is not visible",
                        ));
                    }
                    self.stitch(cards, image)?;
                    return self.complete(image);
                }
                self.phase = Phase::ReturnLeft;
                self.request(image, 1, &view)
            }
            Phase::ReturnLeft => {
                if dx.is_some_and(|dx| dx.abs() <= 2) {
                    if !left_end(&view, self.bar) {
                        return Err(unreadable("left edge could not be confirmed"));
                    }
                    self.stitch(cards, image)?;
                    self.phase = Phase::Right;
                    return self.request(image, -1, &view);
                }
                self.request(image, 1, &view)
            }
            Phase::Right => {
                let moved = dx.ok_or_else(|| unreadable("capture is missing"))?;
                if moved.abs() > 2 {
                    self.offset -= moved;
                }
                self.stitch(cards, image)?;
                if rebounded || dx.is_some_and(|dx| dx.abs() <= 2) {
                    if !right_end(&view, self.bar) {
                        return Err(unreadable("right edge is clipped or scrolling failed"));
                    }
                    self.phase = Phase::Finish {
                        stable: 0,
                        captures: 0,
                    };
                    return Ok(ScanStep::Wait);
                }
                self.request(image, -1, &view)
            }
            Phase::Finish { .. } => unreachable!(),
            Phase::Complete => unreachable!(),
        }
    }

    fn request(&mut self, image: &Mat, direction: i32, view: &View) -> Result<ScanStep> {
        let at_edge = if direction > 0 {
            left_end(view, self.bar)
        } else {
            right_end(view, self.bar)
        };
        let short_step = if matches!(self.phase, Phase::ReturnLeft) && !at_edge {
            RETURN_SCROLL
        } else {
            SHORT_SCROLL
        };
        let distance = if at_edge || matches!(self.phase, Phase::Probe | Phase::ReturnLeft) {
            short_step
        } else {
            280
        };
        let pan = pan(
            image,
            self.bar,
            &self.regions,
            direction,
            distance,
            short_step,
        )?;
        self.pending = Some(pan);
        Ok(ScanStep::Pan(pan))
    }

    fn stitch(&mut self, cards: &[Card], image: &Mat) -> Result<()> {
        let seeded = !self.entries.is_empty();
        let mut overlaps = 0;
        let mut identities = 0;
        if let (Some(first), Some(last)) = (cards.first(), cards.last()) {
            let left = first.slot.0 as i32 + self.offset;
            let right = last.slot.0 as i32 + self.offset;
            if self.entries.iter().any(|entry| {
                entry.x >= left - 3
                    && entry.x <= right + 3
                    && !cards
                        .iter()
                        .any(|card| (entry.x - card.slot.0 as i32 - self.offset).abs() <= 3)
            }) {
                return Err(unreadable("a previously read slot is missing"));
            }
        }
        for card in cards {
            let x = card.slot.0 as i32 + self.offset;
            if let Some(existing) = self.entries.iter_mut().find(|e| (e.x - x).abs() <= 3) {
                if existing.bound.card.kind != card.kind
                    || existing.width.abs_diff(card.region.2) > 8
                {
                    return Err(unreadable(&format!(
                        "overlap changed at slot {} while reading the army",
                        existing.bound.logical_slot
                    )));
                }
                if let (Some(a), Some(b)) = (existing.bound.card.count, card.count)
                    && a != b
                {
                    return Err(unreadable("quantity changed while reading the army"));
                }
                identities += usize::from(recipe::matches_portrait(
                    image,
                    card,
                    &existing.bound.portrait,
                    false,
                )?);
                existing.bound.card.count = card.count.or(existing.bound.card.count);
                // Slot identity is spatial. Refresh animated artwork only within a verified overlap.
                existing.bound.portrait = recipe::portrait(image, card.slot)?;
                overlaps += 1;
                continue;
            }
            if self.entries.last().is_some_and(|last| x <= last.x + 60) {
                return Err(unreadable("overlap does not preserve slot order"));
            }
            if self.entries.len() >= MAX_SLOTS {
                return Err(unreadable("contains more than 64 slots"));
            }
            let mut bound = BoundCard::new(card, image, self.builder)?;
            bound.logical_slot = self.entries.len() + 1;
            self.entries.push(Entry {
                bound,
                x,
                left: card.region.0 as i32 + self.offset,
                width: card.region.2,
            });
        }
        if seeded && (overlaps < 2 || identities < 2) {
            return Err(unreadable(
                "scroll has fewer than two verified overlapping cards",
            ));
        }
        Ok(())
    }

    fn complete(&mut self, image: &Mat) -> Result<ScanStep> {
        let mut group = 0;
        let mut previous_right = None;
        for entry in &mut self.entries {
            if previous_right.is_some_and(|right| entry.left - right > 20) {
                group += 1;
            }
            entry.bound.card.group = group;
            previous_right = Some(entry.left + entry.width as i32);
        }
        self.phase = Phase::Complete;
        Ok(ScanStep::Complete(Roster {
            bar: self.bar,
            builder: self.builder,
            entries: std::mem::take(&mut self.entries),
            offset: self.offset,
            max_offset: self.offset,
            previous: image.try_clone()?,
            pending: None,
            pending_refused: false,
            settling: None,
            stalled_scrolls: 0,
            unreadable: None,
            captures: 0,
            failed: false,
        }))
    }
}

struct View {
    cards: Vec<Card>,
    left_clipped: bool,
    right_clipped: bool,
}

fn full_cards(image: &Mat, bar: Region, builder: bool, digits: &Digits) -> Result<View> {
    let mut cards = super::cards(image, bar, builder, digits)?;
    let regions = super::card_regions(image, bar, builder)?;
    let left_arrow = overflow_arrow(image, bar, false)?;
    let right_arrow = overflow_arrow(image, bar, true)?;
    // Fixed overflow arrows can merge with an edge portrait and move its apparent centre.
    let left = bar.0 + if left_arrow { 24 } else { EDGE as u32 };
    let right = bar.0 + bar.2 - if right_arrow { 24 } else { EDGE as u32 };
    if regions.iter().any(|r| {
        r.2 >= 78 && r.0 >= left && r.0 + r.2 <= right && !cards.iter().any(|c| c.region == *r)
    }) {
        return Err(unreadable("contains an unrecognized card"));
    }
    cards.retain(|c| c.region.2 >= 78 && c.region.0 >= left && c.region.0 + c.region.2 <= right);
    if cards.is_empty() {
        return Err(unreadable("has no complete readable cards"));
    }
    if cards
        .windows(2)
        .any(|pair| pair[1].slot.0 - pair[0].slot.0 > 160)
    {
        return Err(unreadable("contains a missing card between visible slots"));
    }
    let first = cards.first().unwrap().region.0;
    let last = cards.last().unwrap().region;
    let left_clipped = regions.iter().any(|r| r.0 < first && r.2 >= 15) || left_arrow;
    let right_clipped = regions.iter().any(|r| r.0 >= last.0 + last.2 && r.2 >= 15) || right_arrow;
    Ok(View {
        cards,
        left_clipped,
        right_clipped,
    })
}

fn left_end(view: &View, bar: Region) -> bool {
    !view.left_clipped
        && view
            .cards
            .first()
            .is_some_and(|c| (EDGE as u32..=150).contains(&(c.region.0 - bar.0)))
}
fn right_end(view: &View, bar: Region) -> bool {
    !view.right_clipped
        && view
            .cards
            .last()
            .is_some_and(|c| c.region.0 + c.region.2 + EDGE as u32 <= bar.0 + bar.2)
}

fn overflow_arrow(image: &Mat, bar: Region, right: bool) -> Result<bool> {
    // The fixed white arrow remains visible even when the next card is only a few pixels wide.
    let y = bar.1 + bar.3 / 2;
    let outer = if right {
        bar.0 + bar.2 - 20
    } else {
        bar.0 + 19
    };
    let direction = if right { 1_i32 } else { -1 };
    let mut white = 0;
    let mut dark = 0;
    for row in 0..8_u32 {
        for column in 0..=row {
            let x = outer
                .checked_add_signed(direction * column as i32)
                .ok_or(Error::Frame)?;
            let p = image.at_2d::<core::Vec3b>((y - 7 + row) as i32, x as i32)?;
            white += usize::from(p.iter().all(|v| *v > 215));
        }
        let x = outer.checked_add_signed(-direction).ok_or(Error::Frame)?;
        let p = image.at_2d::<core::Vec3b>((y - 7 + row) as i32, x as i32)?;
        dark += usize::from(p.iter().all(|v| *v < 60));
    }
    Ok(white >= 34 && dark >= 7)
}

fn visible_regions(entries: &[Entry], offset: i32, bar: Region) -> Vec<Region> {
    entries
        .iter()
        .filter_map(|e| {
            let left = e.left - offset;
            (left >= bar.0 as i32 + EDGE && left + e.width as i32 <= (bar.0 + bar.2) as i32 - EDGE)
                .then_some(Region(
                    left as u32,
                    e.bound.card.region.1,
                    e.width,
                    e.bound.card.region.3,
                ))
        })
        .collect()
}

fn pan(
    image: &Mat,
    bar: Region,
    regions: &[Region],
    direction: i32,
    maximum: i32,
    short_step: i32,
) -> Result<Pan> {
    let mut regions = regions.to_vec();
    regions.sort_by_key(|r| r.0);
    let mut repeated = false;
    for pair in regions.windows(2) {
        let portrait = |region: Region| {
            recipe::portrait(image, Point(region.0 + region.2 / 2, bar.1 + bar.3 / 2))
        };
        if recipe::portrait_difference(&portrait(pair[0])?, &portrait(pair[1])?) <= 18. {
            repeated = true;
            break;
        }
    }
    // A long drag can enter a repeated section even when the current whole strip is unique.
    let distances: &[i32] = if !repeated && translation(image, image, bar, &regions, 360).is_ok() {
        &[280, 160, short_step]
    } else {
        &[short_step]
    };
    let mut runs: Vec<(i32, i32)> = Vec::new();
    for region in &regions {
        let left = region.0 as i32;
        let right = (region.0 + region.2) as i32;
        if let Some(last) = runs.last_mut()
            && left - last.1 <= 20
        {
            last.1 = right;
        } else {
            runs.push((left, right));
        }
    }
    runs.sort_by_key(|(left, right)| std::cmp::Reverse(right - left));
    let inside_card = |x: i32| {
        regions
            .iter()
            .any(|r| x >= r.0 as i32 + 16 && x <= (r.0 + r.2) as i32 - 16)
    };
    for &distance in distances {
        if distance > maximum {
            continue;
        }
        for &(left, right) in &runs {
            let pair = (left + 16..right - 16)
                .filter_map(|from| {
                    let to = from + distance * direction;
                    (to >= left + 16 && to <= right - 16 && inside_card(from) && inside_card(to))
                        .then_some((from, to))
                })
                .min_by_key(|(from, to)| (from + to - left - right).abs());
            if let Some((from, to)) = pair {
                // Never drag a unit into the field beyond the card strip, including at either endpoint.
                let y = bar.1 + bar.3 / 2;
                return Ok(Pan {
                    from: Point(from as u32, y),
                    to: Point(to as u32, y),
                });
            }
        }
    }
    Err(unreadable(
        "has no safe scroll path inside the visible cards",
    ))
}

fn translation(
    before: &Mat,
    after: &Mat,
    bar: Region,
    regions: &[Region],
    limit: i32,
) -> Result<i32> {
    measure_translation(before, after, bar, regions, limit, false)
}

fn measure_translation(
    before: &Mat,
    after: &Mat,
    bar: Region,
    regions: &[Region],
    limit: i32,
    allow_changed_cards: bool,
) -> Result<i32> {
    if before.size()? != core::Size::new(1280, 720)
        || after.size()? != before.size()?
        || before.typ() != core::CV_8UC3
        || after.typ() != core::CV_8UC3
    {
        return Err(Error::Frame);
    }
    // Quantity labels, health headers, level badges and Builder swap controls change during battle.
    let strip = Rect::new(
        bar.0 as i32,
        bar.1 as i32 + 28,
        bar.2 as i32,
        (bar.3 as i32 - 48).min(44),
    );
    let gray = |image: &Mat| -> Result<Mat> {
        let mut out = Mat::default();
        imgproc::cvt_color_def(&Mat::roi(image, strip)?, &mut out, imgproc::COLOR_BGR2GRAY)?;
        Ok(out)
    };
    let before_strip = gray(before)?;
    let after_strip = gray(after)?;
    // A best match at a clipped search boundary does not prove movement stayed within it.
    let search = limit + 8;
    let template_region = Rect::new(
        search,
        0,
        before_strip.cols() - 2 * search,
        before_strip.rows(),
    );
    let template = Mat::roi(&before_strip, template_region)?;
    let mut mask = Mat::new_rows_cols_with_default(
        template.rows(),
        template.cols(),
        core::CV_8UC1,
        Scalar::all(0.),
    )?;
    for region in regions {
        let left = (region.0 as i32 + 8 - strip.x - search).max(0);
        let right = (region.0 as i32 + region.2 as i32 - 8 - strip.x - search).min(template.cols());
        if right > left {
            Mat::roi_mut(&mut mask, Rect::new(left, 0, right - left, template.rows()))?
                .set_to(&Scalar::all(255.), &core::no_array())?;
        }
    }
    if core::count_non_zero(&mask)? < 900 {
        return Err(unreadable("has too little stable detail for scrolling"));
    }
    let mut scores = Mat::default();
    imgproc::match_template(
        &after_strip,
        &template,
        &mut scores,
        imgproc::TM_SQDIFF_NORMED,
        &mask,
    )?;
    let values = scores.data_typed::<f32>()?;
    let (index, score) = values
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .ok_or_else(|| unreadable("movement could not be measured"))?;
    let competing = values
        .iter()
        .enumerate()
        .any(|(i, &v)| i.abs_diff(index) > 5 && v.is_finite() && v <= score + 0.012);
    if score > 0.12 || competing {
        if allow_changed_cards {
            // Spent cards can change artwork between actions; three unchanged interiors still prove motion.
            let mut stable = Vec::new();
            for &region in regions {
                let patch = Rect::new(
                    region.0 as i32 + 8 - strip.x,
                    0,
                    region.2 as i32 - 16,
                    strip.height,
                );
                let a = Mat::roi(&before_strip, patch)?;
                let b = Mat::roi(&after_strip, patch)?;
                let mut contrast = true;
                for image in [&a, &b] {
                    let mut deviation = Scalar::default();
                    core::mean_std_dev(
                        image,
                        &mut core::no_array(),
                        &mut deviation,
                        &core::no_array(),
                    )?;
                    contrast &= deviation[0] > 5.;
                }
                if !contrast {
                    continue;
                }
                let mut score = Mat::default();
                imgproc::match_template_def(&b, &a, &mut score, imgproc::TM_SQDIFF_NORMED)?;
                if *score.at_2d::<f32>(0, 0)? <= 0.04 {
                    let mut matches = Mat::default();
                    imgproc::match_template_def(
                        &after_strip,
                        &a,
                        &mut matches,
                        imgproc::TM_SQDIFF_NORMED,
                    )?;
                    let values = matches.data_typed::<f32>()?;
                    let best = values
                        .iter()
                        .copied()
                        .enumerate()
                        .filter(|(_, score)| score.is_finite())
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    if let Some((index, score)) = best
                        && (index as i32 - patch.x).abs() <= 2
                        && !values.iter().enumerate().any(|(i, &value)| {
                            i.abs_diff(index) > 5 && value.is_finite() && value <= score + 0.012
                        })
                    {
                        stable.push(region);
                    }
                }
            }
            let left = stable.iter().map(|r| r.0).min().unwrap_or(0);
            let right = stable.iter().map(|r| r.0 + r.2).max().unwrap_or(0);
            if stable.len() >= 3 && right - left >= 200 {
                return translation(before, after, bar, &stable, limit);
            }
        }
        return Err(unreadable("movement is ambiguous"));
    }
    let dx = index as i32 - search;
    if dx.abs() > limit {
        return Err(unreadable("movement exceeds its requested bound"));
    }
    Ok(dx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(name: &str) -> Mat {
        crate::vision::read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(format!("army-home-overflow-{name}.png")),
        )
        .unwrap()
    }
    fn digits() -> Digits {
        Digits::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
            Path::new("calibration/digits"),
        )
        .unwrap()
    }
    const BAR: Region = Region(0, 598, 1280, 106);

    #[test]
    fn captured_ignored_runtime_scroll_retries_boundedly_without_claiming_motion() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-runtime-return-{name}.png")),
            )
            .unwrap()
        };
        let origin = load("origin");
        let before = load("before");
        let stalled = load("stalled");
        let digits = digits();
        let cards = super::super::cards(&origin, BAR, false, &digits).unwrap();
        let regions: Vec<_> = cards.iter().map(|card| card.region).collect();
        assert_eq!(
            translation(&origin, &before, BAR, &regions, 48).unwrap(),
            -20
        );
        assert_eq!(
            translation(&before, &stalled, BAR, &regions, 48).unwrap(),
            0
        );
        let setup = || {
            let (mut scanner, last) = stitched_home(&digits);
            let ScanStep::Complete(mut roster) = scanner.complete(&last).unwrap() else {
                panic!()
            };
            let shift = cards[0].slot.0 as i32 - roster.entries[0].x;
            for entry in &mut roster.entries {
                entry.x += shift;
                entry.left += shift;
            }
            roster.max_offset += shift;
            for (i, card) in cards.iter().enumerate() {
                let entry = &mut roster.entries[i];
                entry.bound = BoundCard::new(card, &origin, false).unwrap();
                entry.bound.logical_slot = i + 1;
                entry.x = card.slot.0 as i32;
                entry.left = card.region.0 as i32;
                entry.width = card.region.2;
            }
            roster.offset = 20;
            roster.previous = before.clone();
            assert!(
                matches!(roster.locate(1,&before,&digits).unwrap(),Locate::Pan(pan) if pan.dx()==48)
            );
            roster
        };
        let settle = |roster: &mut Roster| {
            for _ in 0..2 {
                assert!(matches!(
                    roster.locate(1, &stalled, &digits).unwrap(),
                    Locate::Wait
                ));
                assert_eq!(roster.offset, 20);
            }
            roster.locate(1, &stalled, &digits)
        };
        let mut roster = setup();
        for attempt in 1..=2 {
            let Locate::Pan(pan) = settle(&mut roster).unwrap() else {
                panic!("retry stays a scroll")
            };
            assert_eq!(pan.dx(), 64);
            assert_eq!(pan.limit(), 44);
            assert_eq!(roster.stalled_scrolls, attempt);
            assert_eq!(roster.offset, 20);
        }
        assert!(settle(&mut roster).is_err());
        assert!(roster.failed);

        let mut roster = setup();
        assert!(matches!(settle(&mut roster).unwrap(), Locate::Pan(_)));
        // This separately captured left viewport proves arrival only when its actual movement is measured.
        for _ in 0..2 {
            assert!(matches!(
                roster.locate(1, &origin, &digits).unwrap(),
                Locate::Wait
            ));
        }
        let Locate::Ready(card) = roster.locate(1, &origin, &digits).unwrap() else {
            panic!()
        };
        assert_eq!(card.logical_slot, 1);
        assert_eq!(card.card.slot.0, 59);
        assert_eq!(card.card.count, Some(1));
        assert_eq!(roster.offset, 0);
        assert_eq!(roster.stalled_scrolls, 0);

        let mut roster = setup();
        assert!(matches!(
            roster.locate(1, &stalled, &digits).unwrap(),
            Locate::Wait
        ));
        assert!(roster.locate(2, &stalled, &digits).is_err());
        for ambiguous in [false, true] {
            let mut wrong =
                Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
            if !ambiguous {
                Mat::roi(&before, Rect::new(8, 590, 1272, 130))
                    .unwrap()
                    .copy_to(&mut Mat::roi_mut(&mut wrong, Rect::new(0, 590, 1272, 130)).unwrap())
                    .unwrap();
            }
            assert!(setup().locate(1, &wrong, &digits).is_err());
        }
        let mut roster = setup();
        roster.entries[0].bound.portrait = roster.entries[1].bound.portrait.clone();
        assert!(matches!(settle(&mut roster).unwrap(), Locate::Pan(_)));
        for _ in 0..5 {
            assert!(matches!(
                roster.locate(1, &origin, &digits).unwrap(),
                Locate::Wait
            ));
        }
        assert!(roster.locate(1, &origin, &digits).is_err());
    }

    #[test]
    fn stationary_tracking_uses_unchanged_cards_when_spent_artwork_changes() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/fixtures/army-home-spent-{name}.png")),
            )
            .unwrap()
        };
        let digits = digits();
        let scan = load("scan");
        let before = load("before");
        let after = load("after");
        let setup = || {
            let mut scanner = Scanner::new(BAR, false).unwrap();
            let ScanStep::Complete(mut roster) = scanner.observe(&scan, &digits).unwrap() else {
                panic!()
            };
            roster.previous = before.clone();
            roster
        };
        let mut roster = setup();
        let regions = visible_regions(&roster.entries, 0, BAR);
        assert!(translation(&before, &after, BAR, &regions, 6).is_err());
        let Locate::Ready(card) = roster.locate(5, &after, &digits).unwrap() else {
            panic!()
        };
        assert_eq!(card.card.slot, Point(546, 651));
        assert_eq!(card.remaining(&after, &digits).unwrap(), Some(0));
        assert_eq!(roster.offset, 0);
        for state in ["pending", "settling", "refused"] {
            let mut roster = setup();
            let pan = Pan {
                from: Point(640, 651),
                to: Point(688, 651),
            };
            if state == "settling" {
                roster.settling = Some(Settling::new(Some(5), pan, 5, false).unwrap());
            } else {
                roster.pending = Some(pan);
                roster.pending_refused = state == "refused";
            }
            assert!(roster.locate(5, &after, &digits).is_err(), "{state}");
        }
        for dx in [-97_i32, -12, -6, -3, 3, 6, 12, 97] {
            let mut moved =
                Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
            let width = 1280 - dx.abs();
            Mat::roi(&after, Rect::new((-dx).max(0), 530, width, 190))
                .unwrap()
                .copy_to(
                    &mut Mat::roi_mut(&mut moved, Rect::new(dx.max(0), 530, width, 190)).unwrap(),
                )
                .unwrap();
            assert!(
                setup().locate(5, &moved, &digits).is_err(),
                "unrequested shift {dx}"
            );
        }
        let mut wrong = setup();
        wrong.entries[4].bound.portrait = wrong.entries[0].bound.portrait.clone();
        for _ in 0..3 {
            assert!(matches!(
                wrong.locate(5, &after, &digits).unwrap(),
                Locate::Wait
            ));
        }
        assert!(
            wrong.locate(5, &after, &digits).is_err(),
            "the target still needs its own identity"
        );
        let mut insufficient =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
        Mat::roi(&before, Rect::new(90, 530, 195, 190))
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut insufficient, Rect::new(90, 530, 195, 190)).unwrap())
            .unwrap();
        assert!(
            setup().locate(5, &insufficient, &digits).is_err(),
            "two unchanged cards do not prove tracking"
        );
    }

    #[test]
    fn stationary_fallback_rejects_repeated_interiors_after_full_slot_shifts() {
        let source = fixture("left");
        let mut before =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
        let mut regions = Vec::new();
        for x in (10..1175).step_by(97) {
            Mat::roi(&source, Rect::new(301, 598, 97, 106))
                .unwrap()
                .copy_to(&mut Mat::roi_mut(&mut before, Rect::new(x, 598, 97, 106)).unwrap())
                .unwrap();
            regions.push(Region(x as u32, 598, 88, 106));
        }
        for dx in [97, 194] {
            let mut after =
                Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
            Mat::roi(&before, Rect::new(0, 598, 1280 - dx, 106))
                .unwrap()
                .copy_to(&mut Mat::roi_mut(&mut after, Rect::new(dx, 598, 1280 - dx, 106)).unwrap())
                .unwrap();
            for (index, region) in regions.iter().enumerate() {
                if ![3, 6, 9].contains(&index) {
                    Mat::roi_mut(
                        &mut after,
                        Rect::new(region.0 as i32, 598, region.2 as i32, 106),
                    )
                    .unwrap()
                    .set_to(&Scalar::all(0.), &core::no_array())
                    .unwrap();
                }
            }
            assert!(translation(&before, &after, BAR, &regions, 6).is_err());
            let aliases = [regions[3], regions[6], regions[9]];
            assert_eq!(translation(&before, &after, BAR, &aliases, 6).unwrap(), 0);
            assert!(
                measure_translation(&before, &after, BAR, &regions, 6, true).is_err(),
                "three repeated survivors cannot disguise a {dx}px scroll"
            );
        }
    }

    #[test]
    fn captured_overflow_edges_and_spring_motion_preserve_the_first_slot() {
        let digits = digits();
        let mut scanner = Scanner::new(BAR, false).unwrap();
        for (name, direction) in [("left", 1), ("spring", 1), ("settled", -1)] {
            let image = fixture(name);
            let view = full_cards(&image, BAR, false, &digits).unwrap();
            assert_eq!(view.cards.len(), 12, "{name}");
            assert!(left_end(&view, BAR), "{name}");
            assert!(!right_end(&view, BAR), "{name}");
            assert!(view.cards.iter().all(|card| card.group == 0));
            assert_eq!(view.cards[0].count, Some(1));
            assert_eq!(view.cards[1].count, Some(4));
            assert_eq!(view.cards[2].count, Some(76));
            assert!(view.cards[3..].iter().all(|card| card.count == Some(1)));
            let mut result = scanner.observe(&image, &digits).unwrap();
            for _ in 0..2 {
                if matches!(result, ScanStep::Wait) {
                    result = scanner.observe(&image, &digits).unwrap();
                }
            }
            let ScanStep::Pan(pan) = result else {
                panic!("settled overflow must not publish a partial army");
            };
            assert_eq!(pan.dx().signum(), direction);
        }
        let image = fixture("right");
        let view = full_cards(&image, BAR, false, &digits).unwrap();
        assert!(!left_end(&view, BAR));
        assert!(right_end(&view, BAR));
        assert_eq!(view.cards.len(), 12);
        let mut unreadable = fixture("left");
        for region in [
            Rect::new(400, 588, 94, 40),
            Rect::new(400, 628, 9, 76),
            Rect::new(484, 628, 10, 76),
        ] {
            Mat::roi_mut(&mut unreadable, region)
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
        }
        assert!(full_cards(&unreadable, BAR, false, &digits).is_err());
        let mut blank = image.try_clone().unwrap();
        blank.set_to(&Scalar::all(0.), &core::no_array()).unwrap();
        assert!(full_cards(&blank, BAR, false, &digits).is_err());
    }

    fn stitched_home(digits: &Digits) -> (Scanner, Mat) {
        let mut scanner = Scanner::new(BAR, false).unwrap();
        let first = fixture("left");
        let first_view = full_cards(&first, BAR, false, digits).unwrap();
        scanner.stitch(&first_view.cards, &first).unwrap();
        // The first manually captured gesture was 400px; its measured content translation was 496px.
        scanner.offset = 496;
        let mut previous = fixture("middle-1");
        let mut view = full_cards(&previous, BAR, false, digits).unwrap();
        scanner.stitch(&view.cards, &previous).unwrap();
        for (name, expected) in [("middle-2", -287), ("middle-3", -256), ("right", -262)] {
            let next = fixture(name);
            let regions = view.cards.iter().map(|c| c.region).collect::<Vec<_>>();
            let dx = translation(&previous, &next, BAR, &regions, 360).unwrap();
            assert!((dx - expected).abs() <= 1, "{name}: {dx}");
            scanner.offset -= dx;
            view = full_cards(&next, BAR, false, digits).unwrap();
            scanner
                .stitch(&view.cards, &next)
                .unwrap_or_else(|e| panic!("{name}, offset{}: {e}", scanner.offset));
            previous = next;
        }
        (scanner, previous)
    }

    #[test]
    fn captured_scroll_pairs_measure_actual_motion_and_stitch_all_26_slots() {
        let digits = digits();
        let (mut scanner, previous) = stitched_home(&digits);
        let ScanStep::Complete(mut roster) = scanner.complete(&previous).unwrap() else {
            panic!()
        };
        let units = roster.inventory();
        assert_eq!(units.len(), 26);
        assert!(
            units[..18]
                .iter()
                .all(|u| u.kind == recipe::UnitKind::Troop)
        );
        assert_eq!(units[18].kind, recipe::UnitKind::Siege);
        assert!(
            units[19..23]
                .iter()
                .all(|u| u.kind == recipe::UnitKind::Hero)
        );
        assert!(
            units[23..]
                .iter()
                .all(|u| u.kind == recipe::UnitKind::Spell)
        );
        assert_eq!(
            units.iter().map(|u| u.count.unwrap()).collect::<Vec<_>>(),
            [vec![1, 4, 76], vec![1; 20], vec![3, 2, 1]].concat()
        );
        for slot in 15..=26 {
            let Locate::Ready(bound) = roster.locate(slot, &previous, &digits).unwrap() else {
                panic!("visible slot {slot}")
            };
            assert_eq!(bound.logical_slot, slot);
            assert_eq!(
                bound.remaining(&previous, &digits).unwrap(),
                units[slot - 1].count
            );
        }
        assert!(matches!(
            roster.locate(1, &previous, &digits).unwrap(),
            Locate::Pan(_)
        ));
    }

    #[test]
    fn siege_warning_animation_does_not_change_its_bound_unit() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
        };
        let before = load("army-home-siege-warning-low.png");
        let after = load("army-home-siege-warning-high.png");
        let digits = digits();
        let original = full_cards(&before, BAR, false, &digits).unwrap();
        let current = full_cards(&after, BAR, false, &digits).unwrap();
        let siege = original
            .cards
            .iter()
            .find(|c| c.kind == super::super::Kind::Siege)
            .unwrap();
        let bound = BoundCard::new(siege, &before, false).unwrap();
        let mut scanner = Scanner::new(BAR, false).unwrap();
        scanner.stitch(&original.cards, &before).unwrap();
        let regions = original.cards.iter().map(|c| c.region).collect::<Vec<_>>();
        scanner.offset = -translation(&before, &after, BAR, &regions, 360).unwrap();
        scanner.stitch(&current.cards, &after).unwrap();
        for card in &current.cards {
            let shifted = reposition(&bound, card.slot.0 as i32).unwrap();
            assert_eq!(
                shifted.remaining(&after, &digits).unwrap(),
                if card.kind == super::super::Kind::Siege {
                    Some(1)
                } else {
                    None
                },
                "{:?}",
                card.slot
            );
        }
        let card = current
            .cards
            .iter()
            .find(|c| c.kind == super::super::Kind::Siege)
            .unwrap();
        let shifted = reposition(&bound, card.slot.0 as i32).unwrap();
        let mut covered = after.try_clone().unwrap();
        Mat::roi_mut(
            &mut covered,
            Rect::new(card.slot.0 as i32 - 40, 600, 32, 104),
        )
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
        assert_eq!(shifted.remaining(&covered, &digits).unwrap(), None);
    }

    #[test]
    fn edge_spring_uses_bounded_read_only_recaptures() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
        };
        let before = load("army-home-edge-spring-before.png");
        let after = load("army-home-edge-spring-after.png");
        let digits = digits();
        let start = || {
            let mut scanner = Scanner::new(BAR, false).unwrap();
            scanner.phase = Phase::ReturnLeft;
            scanner.previous = Some(before.try_clone().unwrap());
            scanner.regions = full_cards(&before, BAR, false, &digits)
                .unwrap()
                .cards
                .iter()
                .map(|c| c.region)
                .collect();
            scanner.pending = Some(Pan {
                from: Point(568, 651),
                to: Point(728, 651),
            });
            scanner
        };
        let mut scanner = start();
        assert!(matches!(
            scanner.observe(&after, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&after, &digits).unwrap(),
            ScanStep::Wait
        ));
        let ScanStep::Pan(pan) = scanner.observe(&after, &digits).unwrap() else {
            panic!("settled left edge")
        };
        assert!(pan.dx() < 0);
        assert_eq!(scanner.entries.len(), 12);
        assert_eq!(scanner.offset, 0);
        let mut scanner = start();
        assert!(matches!(
            scanner.observe(&after, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&before, &digits).unwrap(),
            ScanStep::Wait
        ));
        let mut failed = false;
        for frame in [&after, &before].into_iter().cycle().take(8) {
            match scanner.observe(frame, &digits) {
                Ok(ScanStep::Wait) => {}
                Err(_) => {
                    failed = true;
                    break;
                }
                _ => panic!("a bouncing bar cannot authorize another drag"),
            }
        }
        assert!(failed);

        // At the right end, retain the measured rebound in the existing coordinate system.
        let right = fixture("right");
        let mut shifted =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
        Mat::roi(&right, Rect::new(0, 545, 1277, 175))
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut shifted, Rect::new(3, 545, 1277, 175)).unwrap())
            .unwrap();
        let mut scanner = Scanner::new(BAR, false).unwrap();
        scanner.phase = Phase::Right;
        scanner.offset = 10;
        let view = full_cards(&right, BAR, false, &digits).unwrap();
        scanner.stitch(&view.cards, &right).unwrap();
        scanner.regions = view.cards.iter().map(|c| c.region).collect();
        scanner.previous = Some(right);
        scanner.pending = Some(Pan {
            from: Point(900, 651),
            to: Point(740, 651),
        });
        assert!(matches!(
            scanner.observe(&shifted, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&shifted, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&shifted, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&shifted, &digits).unwrap(),
            ScanStep::Wait
        ));
        let ScanStep::Complete(roster) = scanner.observe(&shifted, &digits).unwrap() else {
            panic!("settled right edge")
        };
        assert_eq!(roster.offset, 7);
    }

    #[test]
    fn overflow_arrow_occlusion_cannot_shift_a_saved_slot_center() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
        };
        let before = load("army-home-arrow-occluded.png");
        let after = load("army-home-arrow-clear.png");
        let digits = digits();
        let raw = super::super::cards(&before, BAR, false, &digits).unwrap();
        assert_eq!(raw.last().unwrap().kind, super::super::Kind::Siege);
        let view = full_cards(&before, BAR, false, &digits).unwrap();
        assert!(
            view.cards
                .iter()
                .all(|c| c.kind != super::super::Kind::Siege)
        );
        let mut scanner = Scanner::new(BAR, false).unwrap();
        scanner.stitch(&view.cards, &before).unwrap();
        let regions = view.cards.iter().map(|c| c.region).collect::<Vec<_>>();
        scanner.offset = -translation(&before, &after, BAR, &regions, 360).unwrap();
        let view = full_cards(&after, BAR, false, &digits).unwrap();
        scanner.stitch(&view.cards, &after).unwrap();
        let siege = scanner
            .entries
            .iter()
            .filter(|e| e.bound.card.kind == super::super::Kind::Siege)
            .collect::<Vec<_>>();
        assert_eq!(siege.len(), 1);
        assert_eq!(siege[0].width, 81);
        assert_eq!(siege[0].bound.card.slot.0, 1016);
    }

    #[test]
    fn overlap_requires_neighbor_identity_and_preserves_each_slot_category() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
        };
        let before = load("army-home-overlap-before.png");
        let after = load("army-home-overlap-after.png");
        let digits = digits();
        let original = full_cards(&before, BAR, false, &digits).unwrap();
        let current = full_cards(&after, BAR, false, &digits).unwrap();
        let regions = original.cards.iter().map(|c| c.region).collect::<Vec<_>>();
        let offset = -translation(&before, &after, BAR, &regions, 360).unwrap();
        let setup = || {
            let mut scanner = Scanner::new(BAR, false).unwrap();
            scanner.stitch(&original.cards, &before).unwrap();
            scanner.offset = offset;
            scanner
        };
        let mut scanner = setup();
        let old_siege = scanner
            .entries
            .iter()
            .find(|e| e.bound.card.kind == super::super::Kind::Siege)
            .unwrap()
            .bound
            .clone();
        let current_siege = current
            .cards
            .iter()
            .find(|c| c.kind == super::super::Kind::Siege)
            .unwrap();
        assert!(
            !recipe::matches_portrait(&after, current_siege, &old_siege.portrait, false).unwrap()
        );
        scanner.stitch(&current.cards, &after).unwrap();
        let fresh = scanner
            .entries
            .iter()
            .find(|e| e.bound.card.kind == super::super::Kind::Siege)
            .unwrap();
        let fresh = reposition(&fresh.bound, current_siege.slot.0 as i32).unwrap();
        assert_eq!(fresh.remaining(&after, &digits).unwrap(), Some(1));

        let mut changed_category = current.cards.clone();
        changed_category
            .iter_mut()
            .find(|c| c.kind == super::super::Kind::Siege)
            .unwrap()
            .kind = super::super::Kind::Hero;
        assert!(setup().stitch(&changed_category, &after).is_err());
        let mut missing = current.cards.clone();
        missing.remove(2);
        assert!(setup().stitch(&missing, &after).is_err());
        let mut lost_identity = setup();
        for entry in &mut lost_identity.entries {
            entry.bound.portrait.fill(0);
        }
        assert!(lost_identity.stitch(&current.cards, &after).is_err());

        let mut hidden = after.try_clone().unwrap();
        let card = &current.cards[2];
        Mat::roi_mut(
            &mut hidden,
            Rect::new(card.region.0 as i32 - 4, 545, card.region.2 as i32 + 8, 175),
        )
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
        assert!(full_cards(&hidden, BAR, false, &digits).is_err());
    }

    #[test]
    fn complete_roster_uses_the_settled_final_capture() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
        };
        let before = load("army-home-final-edge-before.png");
        let after = load("army-home-final-edge-after.png");
        let digits = digits();
        let view = full_cards(&before, BAR, false, &digits).unwrap();
        let regions = view.cards.iter().map(|c| c.region).collect::<Vec<_>>();
        let dx = translation(&before, &after, BAR, &regions, 16).unwrap();
        assert_eq!(dx, 3);
        let setup = || {
            let mut scanner = Scanner::new(BAR, false).unwrap();
            scanner.phase = Phase::Right;
            scanner.offset = 20;
            scanner.stitch(&view.cards, &before).unwrap();
            scanner.regions = regions.clone();
            scanner.previous = Some(before.try_clone().unwrap());
            scanner.pending = Some(Pan {
                from: Point(900, 651),
                to: Point(740, 651),
            });
            scanner
        };
        let mut scanner = setup();
        assert!(matches!(
            scanner.observe(&before, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&after, &digits).unwrap(),
            ScanStep::Wait
        ));
        assert!(matches!(
            scanner.observe(&after, &digits).unwrap(),
            ScanStep::Wait
        ));
        for _ in 0..2 {
            assert!(matches!(
                scanner.observe(&after, &digits).unwrap(),
                ScanStep::Wait
            ));
        }
        let ScanStep::Complete(mut roster) = scanner.observe(&after, &digits).unwrap() else {
            panic!("stable final edge")
        };
        assert_eq!(roster.offset, 17);
        assert!(matches!(
            roster.locate(1, &after, &digits).unwrap(),
            Locate::Ready(_)
        ));
        assert!(
            roster.locate(1, &before, &digits).is_err(),
            "later unrequested motion remains forbidden"
        );

        let mut scanner = setup();
        assert!(matches!(
            scanner.observe(&before, &digits).unwrap(),
            ScanStep::Wait
        ));
        let mut failed = false;
        for frame in [&after, &before].into_iter().cycle().take(8) {
            match scanner.observe(frame, &digits) {
                Ok(ScanStep::Wait) => {}
                Err(_) => {
                    failed = true;
                    break;
                }
                _ => panic!("an endlessly bouncing edge never completes"),
            }
        }
        assert!(failed);
    }

    fn locate_frame(name: &str) -> Mat {
        crate::vision::read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(format!("army-home-locate-{name}.png")),
        )
        .unwrap()
    }

    fn captured_home_return(digits: &Digits, before: &Mat) -> Roster {
        let (mut scanner, previous) = stitched_home(digits);
        let ScanStep::Complete(mut roster) = scanner.complete(&previous).unwrap() else {
            panic!()
        };
        // This return was captured after a scan whose first-slot origin was x67.
        let shift = 67 - roster.entries[0].x;
        for entry in &mut roster.entries {
            entry.x += shift;
            entry.left += shift;
        }
        roster.max_offset += shift;
        // Resume near slot 1; all 26 identities came from the complete scan.
        let first = super::super::cards(before, BAR, false, digits).unwrap()[0].clone();
        assert_eq!(first.slot.0, 47);
        roster.offset = roster.entries[0].x - first.slot.0 as i32;
        roster.previous = before.try_clone().unwrap();
        roster
    }

    #[test]
    fn reached_slot_waits_for_captured_rebound_before_it_is_actionable() {
        let before = locate_frame("before");
        let reached = locate_frame("reached");
        let settled = locate_frame("settled");
        let digits = digits();
        let mut roster = captured_home_return(&digits, &before);
        let Locate::Pan(pan) = roster.locate(1, &before, &digits).unwrap() else {
            panic!("the first card is still clipped")
        };
        assert!(pan.dx() > 0);
        assert!(matches!(
            roster.locate(1, &reached, &digits).unwrap(),
            Locate::Wait
        ));
        let regions = visible_regions(&roster.entries, roster.offset, BAR);
        assert_eq!(
            translation(&reached, &settled, BAR, &regions, 16).unwrap(),
            -4
        );
        for _ in 0..2 {
            assert!(matches!(
                roster.locate(1, &settled, &digits).unwrap(),
                Locate::Wait
            ));
        }
        let Locate::Ready(card) = roster.locate(1, &settled, &digits).unwrap() else {
            panic!("two stable recaptures make the card actionable")
        };
        assert_eq!(card.card.count, Some(1));
        assert!((card.card.slot.0 as i32 - 60).abs() <= 1);
        assert!(matches!(
            roster.locate(1, &settled, &digits).unwrap(),
            Locate::Ready(_)
        ));
        let Locate::Pan(pan) = roster.locate(26, &settled, &digits).unwrap() else {
            panic!("the next slot can be sought without a false unrequested-motion error")
        };
        assert!(pan.dx() < 0);
    }

    #[test]
    fn runtime_settling_rejects_changed_targets_and_endless_motion() {
        let digits = digits();
        let before = locate_frame("before");
        let reached = locate_frame("reached");
        let settled = locate_frame("settled");
        let setup = || {
            let mut roster = captured_home_return(&digits, &before);
            assert!(matches!(
                roster.locate(1, &before, &digits).unwrap(),
                Locate::Pan(_)
            ));
            assert!(matches!(
                roster.locate(1, &reached, &digits).unwrap(),
                Locate::Wait
            ));
            roster
        };
        assert!(setup().locate(26, &settled, &digits).is_err());
        let mut roster = setup();
        let mut rejected = false;
        for i in 0..MAX_SETTLE_CAPTURES {
            let frame = if i % 2 == 0 { &settled } else { &reached };
            match roster.locate(1, frame, &digits) {
                Ok(Locate::Wait) => {}
                Err(_) => {
                    rejected = true;
                    break;
                }
                _ => panic!("oscillation is never actionable"),
            }
        }
        assert!(rejected);
        assert!(
            roster.locate(1, &settled, &digits).is_err(),
            "failed tracking cannot resume"
        );

        let mut roster = setup();
        for _ in 0..2 {
            assert!(matches!(
                roster.locate(1, &settled, &digits).unwrap(),
                Locate::Wait
            ));
        }
        assert!(matches!(
            roster.locate(1, &settled, &digits).unwrap(),
            Locate::Ready(_)
        ));
        assert!(
            roster.locate(1, &reached, &digits).is_err(),
            "settling permission ends at Ready"
        );
    }

    #[test]
    fn owned_endpoint_overshoot_must_return_inside_the_scanned_range() {
        let digits = digits();
        let before = locate_frame("right-before");
        let reached = locate_frame("right-reached");
        let settled = locate_frame("right-settled");
        let setup = || {
            let (mut scanner, previous) = stitched_home(&digits);
            let ScanStep::Complete(mut roster) = scanner.complete(&previous).unwrap() else {
                panic!()
            };
            let settled_cards = super::super::cards(&settled, BAR, false, &digits).unwrap();
            roster.max_offset =
                roster.entries.last().unwrap().x - settled_cards.last().unwrap().slot.0 as i32;
            // Replay the final captured rightward scroll, 27px before the scanned endpoint.
            roster.offset = roster.max_offset - 27;
            roster.previous = before.try_clone().unwrap();
            assert!(matches!(
                roster.locate(26, &before, &digits).unwrap(),
                Locate::Pan(_)
            ));
            roster
        };
        let mut roster = setup();
        assert!(matches!(
            roster.locate(26, &reached, &digits).unwrap(),
            Locate::Wait
        ));
        assert_eq!(roster.offset - roster.max_offset, 8);
        for _ in 0..2 {
            assert!(matches!(
                roster.locate(26, &settled, &digits).unwrap(),
                Locate::Wait
            ));
        }
        let Locate::Ready(card) = roster.locate(26, &settled, &digits).unwrap() else {
            panic!("settled endpoint should be actionable")
        };
        assert_eq!(card.logical_slot, 26);
        assert_eq!(card.card.count, Some(1));
        assert!(roster.offset <= roster.max_offset + 3);
        assert!(matches!(
            roster.locate(1, &settled, &digits).unwrap(),
            Locate::Pan(_)
        ));

        let mut roster = setup();
        assert!(matches!(
            roster.locate(26, &reached, &digits).unwrap(),
            Locate::Wait
        ));
        assert!(matches!(
            roster.locate(26, &reached, &digits).unwrap(),
            Locate::Wait
        ));
        assert!(
            roster.locate(26, &reached, &digits).is_err(),
            "a stable out-of-range bar is not ready"
        );

        let mut roster = setup();
        roster.entries.last_mut().unwrap().bound.portrait.fill(0);
        assert!(
            roster.locate(26, &reached, &digits).is_err(),
            "the actual endpoint must match"
        );
        let mut roster = setup();
        roster.pending = None;
        assert!(
            roster.locate(26, &reached, &digits).is_err(),
            "unrequested motion remains forbidden"
        );
    }

    #[test]
    fn requested_scroll_inertia_settles_within_its_original_motion_budget() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-inertia-{name}.png")),
            )
            .unwrap()
        };
        let before = load("before");
        let reached = load("reached");
        let moving = load("moving");
        let settled = load("settled");
        let digits = digits();
        let setup = || {
            let (mut scanner, previous) = stitched_home(&digits);
            let ScanStep::Complete(mut roster) = scanner.complete(&previous).unwrap() else {
                panic!()
            };
            // Captured slot5 is the first complete card at x88; slot3 is still offscreen.
            roster.offset = roster.entries[4].x - 88;
            roster.previous = before.try_clone().unwrap();
            assert!(matches!(
                roster.locate(3, &before, &digits).unwrap(),
                Locate::Pan(_)
            ));
            assert!(matches!(
                roster.locate(3, &reached, &digits).unwrap(),
                Locate::Wait
            ));
            roster
        };
        let mut roster = setup();
        let first_motion = roster.settling.as_ref().unwrap().distance;
        assert!((218..=220).contains(&first_motion));
        assert!(matches!(
            roster.locate(3, &moving, &digits).unwrap(),
            Locate::Wait
        ));
        assert!((32..=34).contains(&(roster.settling.as_ref().unwrap().distance - first_motion)));
        for _ in 0..2 {
            assert!(matches!(
                roster.locate(3, &settled, &digits).unwrap(),
                Locate::Wait
            ));
        }
        let Locate::Ready(card) = roster.locate(3, &settled, &digits).unwrap() else {
            panic!("stable card")
        };
        assert_eq!(card.card.count, Some(76));
        assert!((card.card.slot.0 as i32 - 162).abs() <= 3);
        assert!(
            roster.locate(3, &moving, &digits).is_err(),
            "later movement is not owned"
        );

        assert!(
            setup().locate(3, &before, &digits).is_err(),
            "mid-bar direction reversal is not an edge spring"
        );
        let shifted = |dx: i32| {
            let transform = Mat::from_slice_2d(&[[1., 0., f64::from(dx)], [0., 1., 0.]]).unwrap();
            let mut frame = Mat::default();
            imgproc::warp_affine(
                &reached,
                &mut frame,
                &transform,
                reached.size().unwrap(),
                imgproc::INTER_NEAREST,
                core::BORDER_CONSTANT,
                Scalar::all(0.),
            )
            .unwrap();
            frame
        };
        assert!(
            setup().locate(3, &shifted(160), &digits).is_err(),
            "continued motion cannot exceed the original drag budget"
        );
        let mut roster = setup();
        for dx in (1..MAX_SETTLE_CAPTURES).map(|i| i32::from(i) * 3) {
            assert!(matches!(
                roster.locate(3, &shifted(dx), &digits).unwrap(),
                Locate::Wait
            ));
        }
        assert!(
            roster
                .locate(3, &shifted(i32::from(MAX_SETTLE_CAPTURES) * 3), &digits)
                .is_err(),
            "continuous drift never becomes ready"
        );
    }

    #[test]
    fn captured_builder_stage_two_inventory_retains_dead_camps() {
        let image = crate::vision::read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/army-builder-stage2-survivors.png"),
        )
        .unwrap();
        let bar = Region(0, 588, 1280, 120);
        let digits = digits();
        let mut scanner = Scanner::new(bar, true).unwrap();
        let ScanStep::Complete(mut roster) = scanner.observe(&image, &digits).unwrap() else {
            panic!("all nine slots are visible: no input")
        };
        let inventory = roster.inventory();
        assert_eq!(
            inventory.iter().map(|u| u.count).collect::<Vec<_>>(),
            vec![
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1)
            ]
        );
        assert_eq!(inventory[0].kind, recipe::UnitKind::Hero);
        assert!(
            inventory[1..]
                .iter()
                .all(|u| u.kind == recipe::UnitKind::Troop)
        );
        for unit in &inventory {
            let Locate::Ready(bound) = roster.locate(unit.slot, &image, &digits).unwrap() else {
                panic!("visible slot")
            };
            assert_eq!(
                bound.remaining(&image, &digits).unwrap(),
                unit.count,
                "slot{}",
                unit.slot
            );
        }
        let plan =
            super::super::automatic::generate(&inventory, false, true, false, false).unwrap();
        assert!(!plan.steps.iter().any(|s| [3, 4].contains(&s.slot)));
        for region in [Rect::new(302, 590, 24, 18), Rect::new(310, 628, 68, 40)] {
            let mut hidden = image.try_clone().unwrap();
            Mat::roi_mut(&mut hidden, region)
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
            assert!(
                Scanner::new(bar, true)
                    .unwrap()
                    .observe(&hidden, &digits)
                    .is_err(),
                "unreadable card cannot vanish or become an invented zero"
            );
        }
    }

    #[test]
    fn captured_siege_scroll_needs_one_more_stable_capture() {
        let load = |i| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-siege-settle-{i}.png")),
            )
            .unwrap()
        };
        let digits = digits();
        let (mut scanner, previous) = stitched_home(&digits);
        let ScanStep::Complete(mut roster) = scanner.complete(&previous).unwrap() else {
            panic!()
        };
        let scanned = crate::vision::read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/army-home-siege-settle-bound.png"),
        )
        .unwrap();
        let siege = super::super::cards(&scanned, BAR, false, &digits)
            .unwrap()
            .into_iter()
            .find(|c| c.kind == super::super::Kind::Siege)
            .unwrap();
        // Use the fresh siege identity from this attack, not the older scan helper's artwork.
        let mut bound = BoundCard::new(&siege, &scanned, false).unwrap();
        bound.logical_slot = 19;
        roster.entries[18].bound = bound;
        let before = load(128);
        roster.offset = roster.entries[5].x - 132;
        roster.previous = before.try_clone().unwrap();
        assert!(matches!(
            roster.locate(19, &before, &digits).unwrap(),
            Locate::Pan(_)
        ));
        for i in 129..=133 {
            assert!(
                matches!(roster.locate(19, &load(i), &digits).unwrap(), Locate::Wait),
                "frame{i} is still settling"
            );
        }
        let state = roster.settling.as_ref().unwrap();
        assert_eq!(state.captures, 4);
        assert_eq!(state.stable, 1);
        assert!((337..=341).contains(&state.distance));
        let Locate::Ready(card) = roster.locate(19, &load(134), &digits).unwrap() else {
            panic!("last capture is stable")
        };
        assert_eq!(card.card.kind, super::super::Kind::Siege);
        assert_eq!(card.card.count, Some(1));
        assert!((card.card.slot.0 as i32 - 1070).abs() <= 3);
    }

    #[test]
    fn captured_return_scrolls_follow_the_complete_roster() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-return-{name}.png")),
            )
            .unwrap()
        };
        let digits = digits();
        let (mut scanner, previous) = stitched_home(&digits);
        let start = load("start");
        let prior = full_cards(&previous, BAR, false, &digits).unwrap();
        let regions = prior.cards.iter().map(|c| c.region).collect::<Vec<_>>();
        scanner.offset -= translation(&previous, &start, BAR, &regions, 16).unwrap();
        let view = full_cards(&start, BAR, false, &digits).unwrap();
        scanner.stitch(&view.cards, &start).unwrap();
        let ScanStep::Complete(mut roster) = scanner.complete(&start).unwrap() else {
            panic!()
        };
        assert_eq!(roster.inventory().len(), 26);
        let mut last_offset = roster.offset;
        for name in ["0", "1", "2"] {
            let frame = load(name);
            if name != "0" {
                for _ in 0..2 {
                    assert!(matches!(
                        roster.locate(1, &frame, &digits).unwrap(),
                        Locate::Wait
                    ));
                }
            }
            let Locate::Pan(pan) = roster.locate(1, &frame, &digits).unwrap() else {
                panic!("first slot remains offscreen")
            };
            assert!(pan.dx() > 0);
            assert!(roster.offset <= last_offset);
            last_offset = roster.offset;
        }
    }

    #[test]
    fn intermediate_scrolls_settle_before_the_next_gesture() {
        let digits = digits();
        let before = fixture("middle-1");
        let reached = fixture("middle-2");
        let setup = || {
            let mut scanner = Scanner::new(BAR, false).unwrap();
            let first = fixture("left");
            scanner
                .stitch(
                    &full_cards(&first, BAR, false, &digits).unwrap().cards,
                    &first,
                )
                .unwrap();
            scanner.offset = 496;
            let view = full_cards(&before, BAR, false, &digits).unwrap();
            scanner.stitch(&view.cards, &before).unwrap();
            scanner.regions = view.cards.iter().map(|card| card.region).collect();
            scanner.previous = Some(before.clone());
            scanner.phase = Phase::Right;
            assert!(matches!(
                scanner.request(&before, -1, &view).unwrap(),
                ScanStep::Pan(_)
            ));
            scanner
        };
        let mut scanner = setup();
        assert!(!right_end(
            &full_cards(&reached, BAR, false, &digits).unwrap(),
            BAR
        ));
        for _ in 0..2 {
            assert!(matches!(
                scanner.observe(&reached, &digits).unwrap(),
                ScanStep::Wait
            ));
            assert!(scanner.pending.is_none());
            assert_eq!(scanner.offset, 496, "stitch only the settled view");
        }
        assert!(
            matches!(scanner.observe(&reached, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() < 0)
        );
        assert!((scanner.offset - 783).abs() <= 1);

        let (mut scanner, right) = stitched_home(&digits);
        let ScanStep::Complete(mut roster) = scanner.complete(&right).unwrap() else {
            panic!()
        };
        assert!(matches!(
            roster.locate(1, &right, &digits).unwrap(),
            Locate::Pan(_)
        ));
        let middle = fixture("middle-3");
        for _ in 0..2 {
            assert!(matches!(
                roster.locate(1, &middle, &digits).unwrap(),
                Locate::Wait
            ));
            assert!(roster.pending.is_none());
            assert!(
                roster.entries[0].left - roster.offset < 0,
                "target is still offscreen"
            );
        }
        assert!(
            matches!(roster.locate(1, &middle, &digits).unwrap(), Locate::Pan(pan) if pan.dx() > 0)
        );

        let shifted = |dx| {
            let transform = Mat::from_slice_2d(&[[1., 0., f64::from(dx)], [0., 1., 0.]]).unwrap();
            let mut image = Mat::default();
            imgproc::warp_affine(
                &reached,
                &mut image,
                &transform,
                reached.size().unwrap(),
                imgproc::INTER_NEAREST,
                core::BORDER_CONSTANT,
                Scalar::all(0.),
            )
            .unwrap();
            image
        };
        for (dx, message) in [(-80, "scroll distance"), (10, "reversed direction")] {
            let mut scanner = setup();
            assert!(matches!(
                scanner.observe(&reached, &digits).unwrap(),
                ScanStep::Wait
            ));
            let error = scanner
                .observe(&shifted(dx), &digits)
                .err()
                .expect("unsafe tail must fail");
            assert!(error.to_string().contains(message), "{error}");
            assert!(
                scanner.observe(&reached, &digits).is_err(),
                "failed scan stays failed"
            );
        }
        let mut scanner = setup();
        assert!(matches!(
            scanner.observe(&reached, &digits).unwrap(),
            ScanStep::Wait
        ));
        for i in 1..MAX_SETTLE_CAPTURES {
            assert!(matches!(
                scanner
                    .observe(&shifted(-3 * i32::from(i)), &digits)
                    .unwrap(),
                ScanStep::Wait
            ));
        }
        assert!(
            scanner
                .observe(&shifted(-3 * i32::from(MAX_SETTLE_CAPTURES)), &digits)
                .is_err()
        );
    }

    #[test]
    fn refused_scrolls_require_an_unchanged_frame_before_retrying() {
        let digits = digits();
        let image = fixture("left");
        let mut scanner = Scanner::new(BAR, false).unwrap();
        assert!(scanner.cancel_pending_scroll().is_err());
        let ScanStep::Pan(requested) = scanner.observe(&image, &digits).unwrap() else {
            panic!("overflow starts with a left-edge probe")
        };
        scanner.cancel_pending_scroll().unwrap();
        assert!(scanner.cancel_pending_scroll().is_err());
        assert!(
            matches!(scanner.observe(&image, &digits).unwrap(), ScanStep::Pan(pan) if pan == requested)
        );
        assert!(matches!(scanner.phase, Phase::Left));
        assert!(
            scanner.entries.is_empty(),
            "refusal is not evidence of a blocked edge"
        );
        scanner.cancel_pending_scroll().unwrap();
        assert!(scanner.observe(&fixture("spring"), &digits).is_err());
        assert!(scanner.cancel_pending_scroll().is_err());
        assert!(scanner.observe(&image, &digits).is_err());

        let (mut scanner, image) = stitched_home(&digits);
        let ScanStep::Complete(mut roster) = scanner.complete(&image).unwrap() else {
            panic!()
        };
        assert!(roster.cancel_pending_scroll().is_err());
        let Locate::Pan(requested) = roster.locate(1, &image, &digits).unwrap() else {
            panic!()
        };
        roster.cancel_pending_scroll().unwrap();
        assert!(roster.cancel_pending_scroll().is_err());
        assert!(
            matches!(roster.locate(1, &image, &digits).unwrap(), Locate::Pan(pan) if pan == requested)
        );
        roster.cancel_pending_scroll().unwrap();
        assert!(roster.locate(1, &fixture("middle-3"), &digits).is_err());
        assert!(roster.cancel_pending_scroll().is_err());
        assert!(roster.locate(1, &image, &digits).is_err());
    }

    #[test]
    fn cards_under_overflow_arrows_scroll_clear_before_selection() {
        let digits = digits();
        let setup = || {
            let (mut scanner, image) = stitched_home(&digits);
            let ScanStep::Complete(roster) = scanner.complete(&image).unwrap() else {
                panic!()
            };
            roster
        };
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
        };
        let selected = load("army-home-siege-overflow-selected.png");
        assert!(overflow_arrow(&selected, BAR, true).unwrap());
        assert!(
            !super::super::selected(&selected, Point(1220, 651)).unwrap(),
            "an occluded right border cannot prove selection"
        );
        let mut roster = setup();
        roster.offset = roster.entries[18].x - 1220;
        roster.previous = selected.clone();
        assert!(
            matches!(roster.locate(19, &selected, &digits).unwrap(), Locate::Pan(pan) if pan.dx() < 0)
        );

        let left = load("army-home-inertia-before.png");
        assert!(overflow_arrow(&left, BAR, false).unwrap());
        let mut roster = setup();
        roster.offset = roster.entries[4].x - 88;
        roster.previous = left.clone();
        assert!(
            matches!(roster.locate(5, &left, &digits).unwrap(), Locate::Pan(pan) if pan.dx() > 0)
        );

        // At real endpoints the arrow disappears, so the first and last slots remain reachable.
        for (name, slot) in [("left", 1), ("right", 26)] {
            let image = fixture(name);
            assert!(!overflow_arrow(&image, BAR, slot == 26).unwrap());
            let mut roster = setup();
            if slot == 1 {
                roster.offset = 0;
            }
            roster.previous = image.clone();
            assert!(matches!(
                roster.locate(slot, &image, &digits).unwrap(),
                Locate::Ready(_)
            ));
        }
    }

    #[test]
    fn longer_unique_scrolls_retain_the_same_measurement_budget() {
        let image = fixture("left");
        let view = full_cards(&image, BAR, false, &digits()).unwrap();
        let regions = view
            .cards
            .iter()
            .map(|card| card.region)
            .collect::<Vec<_>>();
        for direction in [-1, 1] {
            let requested = pan(&image, BAR, &regions, direction, 280, SHORT_SCROLL).unwrap();
            assert_eq!(requested.dx(), direction * 280);
            assert_eq!(requested.limit(), 360);
            let mut settling = Settling::new(None, requested, direction * 280, false).unwrap();
            for delta in [2, 1, 1] {
                settling.observe(direction * delta, false).unwrap();
            }
            assert_eq!(settling.distance, 284);
            assert!(settling.stable >= 2);
            assert!(settling.observe(direction * 77, false).is_err());
        }
        let image = crate::vision::read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/army-builder-scout.png"),
        )
        .unwrap();
        let bar = Region(0, 588, 1280, 120);
        let view = full_cards(&image, bar, true, &digits()).unwrap();
        let regions = view
            .cards
            .iter()
            .map(|card| card.region)
            .collect::<Vec<_>>();
        let requested = pan(&image, bar, &regions, 1, 280, SHORT_SCROLL).unwrap();
        assert_eq!(requested.dx(), 48);
        assert_eq!(requested.limit(), 44);
    }

    #[test]
    fn endpoint_proofs_and_the_short_probe_return_do_not_use_traversal_drags() {
        let digits = digits();
        let left = fixture("left");
        let mut scanner = Scanner::new(BAR, false).unwrap();
        assert!(
            matches!(scanner.observe(&left, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() == 48)
        );
        for _ in 0..2 {
            assert!(matches!(
                scanner.observe(&left, &digits).unwrap(),
                ScanStep::Wait
            ));
        }
        assert!(
            matches!(scanner.observe(&left, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() == -48)
        );
        assert!(matches!(scanner.phase, Phase::Probe));

        for (phase, frame, direction, distance) in [
            (Phase::Probe, "left", -1, 48),
            (Phase::ReturnLeft, "left", 1, 48),
            (Phase::ReturnLeft, "middle-1", 1, 64),
            (Phase::Right, "right", -1, 48),
            (Phase::Right, "middle-1", -1, 280),
            (Phase::Left, "middle-1", 1, 280),
        ] {
            let image = fixture(frame);
            let view = full_cards(&image, BAR, false, &digits).unwrap();
            if frame == "left" {
                assert!(left_end(&view, BAR));
            }
            let mut scanner = Scanner::new(BAR, false).unwrap();
            scanner.phase = phase;
            scanner.regions = view.cards.iter().map(|card| card.region).collect();
            let ScanStep::Pan(pan) = scanner.request(&image, direction, &view).unwrap() else {
                panic!()
            };
            assert_eq!(pan.dx(), direction * distance, "{frame}");
            assert_eq!(pan.limit(), if distance <= 64 { 44 } else { 360 });
        }
    }

    #[test]
    fn runtime_scrolls_shorten_before_the_known_endpoint() {
        let digits = digits();
        let setup = || {
            let (mut scanner, image) = stitched_home(&digits);
            let ScanStep::Complete(roster) = scanner.complete(&image).unwrap() else {
                panic!()
            };
            roster
        };
        for (name, offset, slot, expected) in [
            ("left", 0, 26, -280),
            ("middle-3", 496 + 287 + 256, 26, -160),
        ] {
            let mut roster = setup();
            let image = fixture(name);
            roster.offset = offset;
            roster.previous = image.clone();
            assert!(
                matches!(roster.locate(slot, &image, &digits).unwrap(), Locate::Pan(pan) if pan.dx() == expected),
                "{name}"
            );
        }
        let before = locate_frame("before");
        let mut roster = captured_home_return(&digits, &before);
        assert!((0..48).contains(&roster.offset));
        assert!(
            matches!(roster.locate(1, &before, &digits).unwrap(), Locate::Pan(pan) if pan.dx() == 48)
        );

        let before = locate_frame("right-before");
        let settled = locate_frame("right-settled");
        let mut roster = setup();
        let cards = super::super::cards(&settled, BAR, false, &digits).unwrap();
        roster.max_offset = roster.entries.last().unwrap().x - cards.last().unwrap().slot.0 as i32;
        roster.offset = roster.max_offset - 27;
        roster.previous = before.clone();
        assert!(
            matches!(roster.locate(26, &before, &digits).unwrap(), Locate::Pan(pan) if pan.dx() == -48)
        );
    }

    #[test]
    fn captured_forty_pixel_drag_moves_and_settles_within_the_short_scroll_bound() {
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-short-scroll-{name}.png")),
            )
            .unwrap()
        };
        let digits = digits();
        let before = load("before");
        let moved = load("moved");
        let settled = load("settled");
        let regions = |image| {
            full_cards(image, BAR, false, &digits)
                .unwrap()
                .cards
                .into_iter()
                .map(|card| card.region)
                .collect::<Vec<_>>()
        };
        let pan = Pan {
            from: Point(980, 651),
            to: Point(940, 651),
        };
        assert_eq!(pan.limit(), 44);
        let movement = translation(&before, &moved, BAR, &regions(&before), pan.limit()).unwrap();
        assert_eq!(movement, -24);
        assert_eq!(
            translation(&moved, &settled, BAR, &regions(&moved), pan.limit()).unwrap(),
            0
        );
        let state = Settling::new(None, pan, movement, false).unwrap();
        assert_eq!(state.distance, 24);
        assert!(state.distance < pan.limit() as u32);
    }

    #[test]
    fn captured_short_drags_move_both_directions_without_increasing_the_bound() {
        let digits = digits();
        for (direction, name, expected) in [(-1, "left", -44), (1, "right", 36)] {
            let load = |phase| {
                crate::vision::read_image(
                    &Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures")
                        .join(format!("army-home-scroll48-{name}-{phase}.png")),
                )
                .unwrap()
            };
            let before = load("before");
            let moved = load("moved");
            let settled = load("settled");
            let regions = |image| {
                full_cards(image, BAR, false, &digits)
                    .unwrap()
                    .cards
                    .into_iter()
                    .map(|card| card.region)
                    .collect::<Vec<_>>()
            };
            let pan = Pan {
                from: Point(640, 651),
                to: Point((640 + direction * 48) as u32, 651),
            };
            assert_eq!(pan.limit(), 44);
            let dx = translation(&before, &moved, BAR, &regions(&before), pan.limit()).unwrap();
            assert_eq!(dx, expected);
            let tail = translation(&moved, &settled, BAR, &regions(&moved), pan.limit()).unwrap();
            assert_eq!(tail, 0);
            let mut state = Settling::new(None, pan, dx, false).unwrap();
            state.observe(tail, false).unwrap();
            assert!(state.distance <= 44);
            let excess = 45 - state.distance as i32;
            assert!(
                state.observe(direction * excess, false).is_err(),
                "45px must still fail"
            );
        }
    }

    #[test]
    fn current_card_geometry_corrects_rounding_without_searching_for_another_unit() {
        let digits = digits();
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-siege-drift-{name}.png")),
            )
            .unwrap()
        };
        let before = load("bound");
        let image = load("current");
        let cards = super::super::cards(&before, BAR, false, &digits).unwrap();
        let siege = cards
            .iter()
            .find(|card| card.kind == super::super::Kind::Siege)
            .unwrap();
        let mut bound = BoundCard::new(siege, &before, false).unwrap();
        bound.logical_slot = 19;
        assert_eq!(
            reposition(&bound, 1115)
                .unwrap()
                .remaining(&image, &digits)
                .unwrap(),
            None
        );
        assert_eq!(
            reposition(&bound, 1117)
                .unwrap()
                .remaining(&image, &digits)
                .unwrap(),
            Some(1)
        );
        let setup = |frame: &Mat, predicted| {
            let (mut scanner, last) = stitched_home(&digits);
            let ScanStep::Complete(mut roster) = scanner.complete(&last).unwrap() else {
                panic!()
            };
            roster.entries[18].bound = bound.clone();
            roster.offset = roster.entries[18].x - predicted;
            roster.previous = frame.clone();
            roster
        };
        let mut roster = setup(&image, 1115);
        let offset = roster.offset;
        let Locate::Ready(current) = roster.locate(19, &image, &digits).unwrap() else {
            panic!()
        };
        assert_eq!(current.logical_slot, 19);
        assert_eq!(current.card.slot, Point(1117, 651));
        assert_eq!(current.card.count, Some(1));
        assert_eq!(
            roster.offset, offset,
            "card outline refinement is not camera movement"
        );

        let mut missing = image.clone();
        Mat::roi_mut(&mut missing, Rect::new(1070, 590, 100, 120))
            .unwrap()
            .set_to(&Scalar::all(0.), &core::no_array())
            .unwrap();
        for scenario in ["missing", "wrong", "outside"] {
            let frame = if scenario == "missing" {
                &missing
            } else {
                &image
            };
            let mut roster = setup(frame, if scenario == "outside" { 1113 } else { 1115 });
            if scenario == "wrong" {
                let hero = cards
                    .iter()
                    .find(|card| card.kind == super::super::Kind::Hero)
                    .unwrap();
                roster.entries[18].bound.portrait = recipe::portrait(&before, hero.slot).unwrap();
            }
            for _ in 0..3 {
                assert!(
                    matches!(roster.locate(19, frame, &digits).unwrap(), Locate::Wait),
                    "{scenario}"
                );
            }
            assert!(roster.locate(19, frame, &digits).is_err(), "{scenario}");
        }
    }

    #[test]
    fn ignored_return_scrolls_retry_twice_without_claiming_an_endpoint() {
        let digits = digits();
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-ignored-return-{name}.png")),
            )
            .unwrap()
        };
        let before = load("before");
        let probe = load("probe");
        let stalled = load("stalled");
        let settled = |scanner: &mut Scanner, frame: &Mat| {
            for _ in 0..2 {
                assert!(matches!(
                    scanner.observe(frame, &digits).unwrap(),
                    ScanStep::Wait
                ));
            }
            scanner.observe(frame, &digits)
        };
        let setup = || {
            let mut scanner = Scanner::new(BAR, false).unwrap();
            assert!(
                matches!(scanner.observe(&before, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() == 48)
            );
            assert!(
                matches!(settled(&mut scanner, &before).unwrap(), ScanStep::Pan(pan) if pan.dx() == -48)
            );
            assert!(
                matches!(settled(&mut scanner, &probe).unwrap(), ScanStep::Pan(pan) if pan.dx() == 64)
            );
            assert!(matches!(scanner.phase, Phase::ReturnLeft));
            scanner
        };
        let mut scanner = setup();
        for attempt in 1..=2 {
            assert!(
                matches!(settled(&mut scanner, &stalled).unwrap(), ScanStep::Pan(pan) if pan.dx() == 64)
            );
            assert_eq!(scanner.stalled_scrolls, attempt);
            assert!(scanner.entries.is_empty());
            assert!(matches!(scanner.phase, Phase::ReturnLeft));
        }
        assert!(settled(&mut scanner, &stalled).is_err());
        assert!(
            scanner.observe(&before, &digits).is_err(),
            "failed scans cannot resume"
        );

        let mut scanner = setup();
        assert!(matches!(
            settled(&mut scanner, &stalled).unwrap(),
            ScanStep::Pan(_)
        ));
        // Reusing the earlier captured endpoint simulates a successful retry response.
        assert!(
            matches!(settled(&mut scanner, &before).unwrap(), ScanStep::Pan(pan) if pan.dx() == 48)
        );
        assert_eq!(scanner.stalled_scrolls, 0);
        assert!(
            matches!(settled(&mut scanner, &before).unwrap(), ScanStep::Pan(pan) if pan.dx() < 0)
        );
        assert!(matches!(scanner.phase, Phase::Right));
        assert!(!scanner.entries.is_empty());
    }

    #[test]
    fn registration_searches_past_its_limit_without_accepting_excess_or_aliases() {
        let digits = digits();
        let before = fixture("left");
        let regions = full_cards(&before, BAR, false, &digits)
            .unwrap()
            .cards
            .iter()
            .map(|card| card.region)
            .collect::<Vec<_>>();
        let shifted = |image: &Mat, dx: i32| {
            let transform = Mat::from_slice_2d(&[[1., 0., f64::from(dx)], [0., 1., 0.]]).unwrap();
            let mut frame = Mat::default();
            imgproc::warp_affine(
                image,
                &mut frame,
                &transform,
                image.size().unwrap(),
                imgproc::INTER_NEAREST,
                core::BORDER_CONSTANT,
                Scalar::all(0.),
            )
            .unwrap();
            frame
        };
        for (requested, dx) in [48, 64].into_iter().flat_map(|requested| {
            [-65, -64, -48, -45, -44, 44, 45, 48, 64, 65].map(|dx| (requested, dx))
        }) {
            let moved = shifted(&before, dx);
            let measured = translation(&before, &moved, BAR, &regions, 44);
            let mut scanner = Scanner::new(BAR, false).unwrap();
            scanner.previous = Some(before.clone());
            scanner.regions = regions.clone();
            scanner.phase = if dx > 0 { Phase::Left } else { Phase::Right };
            scanner.pending = Some(Pan {
                from: Point(640, 651),
                to: Point((640 + dx.signum() * requested) as u32, 651),
            });
            if dx.abs() == 44 {
                assert_eq!(measured.unwrap(), dx);
                assert!(matches!(
                    scanner.observe(&moved, &digits).unwrap(),
                    ScanStep::Wait
                ));
            } else {
                assert!(measured.is_err(), "{dx}px cannot be clipped to44px");
                assert!(scanner.observe(&moved, &digits).is_err());
            }
        }

        let mut repeated =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
        let mut regions = Vec::new();
        for x in (0..1280).step_by(96) {
            let width = (1280 - x).min(96);
            Mat::roi(&before, Rect::new(301, 598, width, 106))
                .unwrap()
                .copy_to(&mut Mat::roi_mut(&mut repeated, Rect::new(x, 598, width, 106)).unwrap())
                .unwrap();
            if width == 96 && (96..=1056).contains(&x) {
                regions.push(Region((x + 8) as u32, 598, 80, 106));
            }
        }
        assert_eq!(
            translation(&repeated, &repeated, BAR, &regions, 44).unwrap(),
            0
        );
        for dx in [-48, -12, -6, -3, 3, 6, 12, 48] {
            assert!(
                !measure_translation(&repeated, &shifted(&repeated, dx), BAR, &regions, 6, true)
                    .is_ok_and(|movement| movement.abs() <= 2),
                "repeated cards cannot hide an unrequested {dx}px movement"
            );
        }
        let flat =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(80.)).unwrap();
        assert!(measure_translation(&flat, &flat, BAR, &regions, 6, true).is_err());
        for (limit, dx) in [44, 64]
            .into_iter()
            .flat_map(|limit| [-48, -44, 44, 48].map(|dx| (limit, dx)))
        {
            let error =
                translation(&repeated, &shifted(&repeated, dx), BAR, &regions, limit).unwrap_err();
            assert!(
                error.to_string().contains("ambiguous"),
                "{dx}px has an opposing repeated-card match in the search margin"
            );
        }
    }

    #[test]
    fn captured_sixty_four_pixel_return_keeps_the_short_measurement_bound() {
        let digits = digits();
        let load = |name| {
            crate::vision::read_image(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(format!("army-home-return64-{name}.png")),
            )
            .unwrap()
        };
        let before = load("before");
        let probe = load("probe");
        let moving = load("moving");
        let settled = load("settled");
        let mut scanner = Scanner::new(BAR, false).unwrap();
        assert!(
            matches!(scanner.observe(&before, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() == 48)
        );
        for frame in [&before, &probe] {
            for _ in 0..2 {
                assert!(matches!(
                    scanner.observe(frame, &digits).unwrap(),
                    ScanStep::Wait
                ));
            }
            let ScanStep::Pan(pan) = scanner.observe(frame, &digits).unwrap() else {
                panic!()
            };
            assert_eq!(
                pan.dx(),
                if matches!(scanner.phase, Phase::Probe) {
                    -48
                } else {
                    64
                }
            );
            assert_eq!(pan.limit(), 44);
        }
        assert!(matches!(scanner.phase, Phase::ReturnLeft));
        assert_eq!(scanner.pending.unwrap().measurement_limit(), 64);
        for frame in [&moving, &settled] {
            assert!(matches!(
                scanner.observe(frame, &digits).unwrap(),
                ScanStep::Wait
            ));
        }
        assert!(
            matches!(scanner.observe(&settled, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() == 48)
        );
        assert!(left_end(
            &full_cards(&settled, BAR, false, &digits).unwrap(),
            BAR
        ));
        assert!(
            scanner.entries.is_empty(),
            "the endpoint still needs its blocked proof"
        );
        for _ in 0..2 {
            assert!(matches!(
                scanner.observe(&settled, &digits).unwrap(),
                ScanStep::Wait
            ));
        }
        assert!(
            matches!(scanner.observe(&settled, &digits).unwrap(), ScanStep::Pan(pan) if pan.dx() < 0)
        );
        assert!(matches!(scanner.phase, Phase::Right));
        assert_eq!(scanner.entries[0].bound.card.count, Some(1));
    }
}
