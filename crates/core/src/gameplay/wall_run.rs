use super::engine::difference;
use crate::{
    Error, Result,
    calibration::Screen,
    device::{Adb, Point, process::Transport},
    telemetry::{Resource, Village},
    vision::crop,
};
use opencv::core::Mat;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
enum Phase {
    Begin,
    Open,
    Top {
        previous: Option<Mat>,
        passes: u8,
    },
    Scan {
        previous: Option<Mat>,
        passes: u8,
        cheapest: Option<u32>,
        seen: bool,
    },
    Find {
        price: Option<u32>,
        passes: u8,
    },
    Select,
    Choose,
    Confirm {
        resource: Resource,
        balance: u32,
    },
    Verify {
        resource: Resource,
        cost: u32,
        balance: u32,
    },
    Close {
        reason: Finish,
    },
}
#[derive(Clone, Copy)]
pub enum Finish {
    NoWalls,
    NeedResources,
    Limit,
}
#[derive(Default)]
pub struct Update {
    pub upgraded: Option<(Resource, u32)>,
    pub finished: Option<Finish>,
}
#[derive(Clone)]
pub struct WallRun {
    village: Village,
    reserves: [u32; 2],
    phase: Phase,
    shift: u32,
    pub gems: Option<u32>,
    previous_gems: Option<u32>,
    balances: Option<(u32, u32)>,
    previous_balances: Option<(u32, u32)>,
    excluded: Vec<Resource>,
    wait_until: u64,
    since: u64,
}
impl WallRun {
    #[cfg(test)]
    pub(super) fn pending_for_test() -> Self {
        let mut run = Self::new(Some(543), 0, Village::Home, [0, 0]);
        run.phase = Phase::Verify {
            resource: Resource::Gold,
            cost: 1,
            balance: 2,
        };
        run
    }
    pub fn verification_pending(&self) -> bool {
        matches!(self.phase, Phase::Verify { .. })
    }
    pub fn new(gems: Option<u32>, now: u64, village: Village, reserves: [u32; 2]) -> Self {
        Self {
            village,
            reserves,
            phase: Phase::Begin,
            shift: 0,
            gems,
            previous_gems: None,
            balances: None,
            previous_balances: None,
            excluded: vec![],
            wait_until: now,
            since: now,
        }
    }
    pub fn operation(&self) -> &'static str {
        match self.phase {
            Phase::Top { .. } | Phase::Scan { .. } | Phase::Find { .. } => "Finding cheapest wall",
            Phase::Confirm { .. } => "Checking wall price",
            Phase::Verify { .. } => "Verifying wall upgrade",
            _ => "Upgrading walls",
        }
    }
    fn go(&mut self, phase: Phase, now: u64) {
        self.phase = phase;
        self.since = now;
        self.wait_until = now + 800;
        self.previous_gems = None;
    }
    pub async fn tick<T: Transport>(
        &mut self,
        vision: &super::GameVision<'_>,
        adb: &mut Adb<T>,
        frame: &super::FrameView<'_>,
        limit: bool,
        cancel: &CancellationToken,
    ) -> Result<Update> {
        let profile = vision.profile;
        let digits = vision.digits;
        let detector = vision.detector.clone();
        let image = frame.image;
        let now = frame.elapsed_ms;
        let village = self.village;
        let crate::vision::Detection::Known(screen) = *frame.detection else {
            return Ok(Update::default());
        };
        let mut update = Update::default();
        if now < self.wait_until {
            return Ok(update);
        }
        if now.saturating_sub(self.since) > 60_000 {
            return Err(Error::Timeout);
        }
        let name = if village == Village::Home {
            "home"
        } else {
            "builder"
        };
        let home = if village == Village::Home {
            Screen::Home
        } else {
            Screen::Builder
        };
        let spec = profile.walls.get(name).ok_or_else(|| {
            Error::Calibration("Walls are not calibrated for this village".into())
        })?;
        if limit
            && !matches!(
                self.phase,
                Phase::Confirm { .. } | Phase::Verify { .. } | Phase::Close { .. }
            )
        {
            self.go(
                Phase::Close {
                    reason: Finish::Limit,
                },
                now,
            );
        }
        let list = detector.matches(image, Screen::BuildList)?;
        let action = |frame: &Mat, screen, name| {
            profile.action(screen, name, frame).map(|point| (point, ()))
        };
        match self.phase.clone() {
            Phase::Begin => {
                if screen == home {
                    self.shift = spec.gem_shift(image)?;
                    let gems = spec.gems(image, digits, false, self.shift)?;
                    let resources = profile.capacity.get(name).ok_or(Error::Unreadable)?;
                    let balance = |key: &str| -> Result<u32> {
                        u32::try_from(digits.trailing(
                            &crop(image, resources.get(key).ok_or(Error::Unreadable)?.bar)?,
                            0.65,
                            110,
                        )?)
                        .map_err(|_| Error::Unreadable)
                    };
                    let balances = (balance("gold")?, balance("elixir")?);
                    if self.previous_gems == Some(gems) && self.previous_balances == Some(balances)
                    {
                        if self.gems.is_some_and(|known| gems < known) {
                            return Err(Error::Gameplay(
                                "Gem balance decreased; wall upgrades stopped".into(),
                            ));
                        }
                        self.gems = Some(gems);
                        self.balances = Some(balances);
                        self.excluded.clear();
                        self.go(Phase::Open, now);
                    } else {
                        self.previous_gems = Some(gems);
                        self.previous_balances = Some(balances);
                    }
                }
            }
            Phase::Open => {
                if list {
                    self.go(
                        Phase::Top {
                            previous: None,
                            passes: 0,
                        },
                        now,
                    );
                } else if screen == home || screen == Screen::WallSelected {
                    adb.tap(detector, &[screen], spec.build, cancel).await?;
                    self.wait_until = now + 1000;
                }
            }
            Phase::Top { previous, passes } => {
                if list && matches!(screen, Screen::BuildList | Screen::WallSelected) {
                    let current = crop(image, spec.row.region)?;
                    if previous
                        .as_ref()
                        .is_some_and(|p| difference(p, &current).is_ok_and(|v| v < 1.5))
                        || passes >= 32
                    {
                        self.go(
                            Phase::Scan {
                                previous: None,
                                passes: 0,
                                cheapest: None,
                                seen: false,
                            },
                            now,
                        );
                    } else {
                        adb.swipe(detector, &[screen], spec.scroll[1], spec.scroll[0], cancel)
                            .await?;
                        self.phase = Phase::Top {
                            previous: Some(current),
                            passes: passes + 1,
                        };
                        self.wait_until = now + 1000;
                    }
                }
            }
            Phase::Scan {
                previous,
                passes,
                mut cheapest,
                mut seen,
            } => {
                if list && matches!(screen, Screen::BuildList | Screen::WallSelected) {
                    let rows = spec.rows(image, digits)?;
                    seen |= !rows.is_empty();
                    for (price, _) in rows {
                        if let Some(price) = price {
                            cheapest = Some(cheapest.map_or(price, |old| old.min(price)));
                        }
                    }
                    let current = crop(image, spec.row.region)?;
                    if previous
                        .as_ref()
                        .is_some_and(|p| difference(p, &current).is_ok_and(|v| v < 1.5))
                        || passes >= 32
                    {
                        self.go(
                            if seen {
                                Phase::Find {
                                    price: cheapest,
                                    passes: 0,
                                }
                            } else {
                                Phase::Close {
                                    reason: Finish::NoWalls,
                                }
                            },
                            now,
                        );
                    } else {
                        adb.swipe(detector, &[screen], spec.scroll[0], spec.scroll[1], cancel)
                            .await?;
                        self.phase = Phase::Scan {
                            previous: Some(current),
                            passes: passes + 1,
                            cheapest,
                            seen,
                        };
                        self.wait_until = now + 1000;
                    }
                }
            }
            Phase::Find { price, passes } => {
                if list && matches!(screen, Screen::BuildList | Screen::WallSelected) {
                    if spec
                        .rows(image, digits)?
                        .iter()
                        .any(|(value, _)| *value == price)
                    {
                        adb.tap_checked(
                            detector,
                            &[screen],
                            |frame| {
                                let y = spec
                                    .rows(frame, digits)?
                                    .into_iter()
                                    .rev()
                                    .find(|(value, _)| *value == price)
                                    .map(|(_, y)| y)
                                    .ok_or(Error::Unreadable)?;
                                Ok((Point(spec.row.tap_x, y), ()))
                            },
                            cancel,
                        )
                        .await?;
                        self.go(Phase::Select, now);
                    } else if passes < 32 {
                        adb.swipe(detector, &[screen], spec.scroll[1], spec.scroll[0], cancel)
                            .await?;
                        self.phase = Phase::Find {
                            price,
                            passes: passes + 1,
                        };
                        self.wait_until = now + 1000;
                    } else {
                        return Err(Error::Unreadable);
                    }
                }
            }
            Phase::Select => {
                if screen == Screen::WallSelected {
                    self.go(Phase::Choose, now);
                    self.wait_until = now + 1500;
                }
            }
            Phase::Choose => {
                if screen == Screen::WallSelected {
                    let gems = spec.gems(image, digits, list, self.shift)?;
                    if self.previous_gems != Some(gems) {
                        self.previous_gems = Some(gems);
                        return Ok(update);
                    }
                    if Some(gems) != self.gems {
                        return Err(Error::Gameplay(
                            "Gem balance changed; wall upgrades stopped".into(),
                        ));
                    }
                    let buttons = spec.buttons(image)?;
                    let resource = [Resource::Gold, Resource::Elixir].into_iter().find(|r| {
                        !self.excluded.contains(r)
                            && buttons.iter().any(|b| b.resource == *r && b.affordable)
                    });
                    if let Some(resource) = resource {
                        let balances = self.balances.ok_or(Error::Unreadable)?;
                        let balance = if resource == Resource::Gold {
                            balances.0
                        } else {
                            balances.1
                        };
                        adb.tap_checked(
                            detector,
                            &[screen],
                            |frame| {
                                let choice = spec
                                    .buttons(frame)?
                                    .into_iter()
                                    .find(|b| b.resource == resource && b.affordable)
                                    .ok_or(Error::InputGuard)?;
                                Ok((choice.point, ()))
                            },
                            cancel,
                        )
                        .await?;
                        self.go(Phase::Confirm { resource, balance }, now);
                    } else {
                        self.go(
                            Phase::Close {
                                reason: Finish::NeedResources,
                            },
                            now,
                        );
                    }
                }
            }
            Phase::Confirm { resource, balance } => {
                if screen == spec.confirm.screen {
                    let reserve = if resource == Resource::Gold {
                        self.reserves[0]
                    } else {
                        self.reserves[1]
                    };
                    if spec
                        .confirm_cost(image, digits, resource, balance, reserve)
                        .is_err()
                    {
                        adb.tap_checked(
                            detector,
                            &[screen],
                            |frame| action(frame, screen, "close"),
                            cancel,
                        )
                        .await?;
                        self.excluded.push(resource);
                        self.go(Phase::Select, now);
                    } else {
                        // A failed ADB reply cannot prove the purchase tap was not delivered.
                        let cost = spec.confirm_cost(image, digits, resource, balance, reserve)?;
                        self.go(
                            Phase::Verify {
                                resource,
                                cost,
                                balance,
                            },
                            now,
                        );
                        let cost = adb
                            .tap_checked(
                                detector,
                                &[screen],
                                |frame| {
                                    let cost = spec
                                        .confirm_cost(frame, digits, resource, balance, reserve)?;
                                    Ok((profile.action(screen, "confirm", frame)?, cost))
                                },
                                cancel,
                            )
                            .await?;
                        self.go(
                            Phase::Verify {
                                resource,
                                cost,
                                balance,
                            },
                            now,
                        );
                    }
                }
            }
            Phase::Verify {
                resource,
                cost,
                balance,
            } => {
                if screen == spec.confirm.screen {
                    return Ok(update);
                }
                if screen == home {
                    let gems = spec.gems(image, digits, false, self.shift)?;
                    if self.previous_gems != Some(gems) {
                        self.previous_gems = Some(gems);
                        return Ok(update);
                    }
                    if Some(gems) != self.gems {
                        return Err(Error::Gameplay(
                            "Gem balance changed after upgrade; stopped".into(),
                        ));
                    }
                    let key = if resource == Resource::Gold {
                        "gold"
                    } else {
                        "elixir"
                    };
                    let bar = profile
                        .capacity
                        .get(name)
                        .and_then(|v| v.get(key))
                        .ok_or(Error::Unreadable)?
                        .bar;
                    let after = u32::try_from(digits.trailing(&crop(image, bar)?, 0.65, 110)?)
                        .map_err(|_| Error::Unreadable)?;
                    if balance.checked_sub(cost) != Some(after) {
                        return Err(Error::Gameplay(
                            "Wall spending could not be verified; stopped before another upgrade"
                                .into(),
                        ));
                    }
                    update.upgraded = Some((resource, cost));
                    self.go(Phase::Begin, now);
                    self.previous_balances = None;
                } else if matches!(screen, Screen::WallSelected | Screen::BuildList) {
                    let name = if screen == Screen::WallSelected && !list {
                        "build"
                    } else {
                        "close"
                    };
                    adb.tap_checked(
                        detector,
                        &[screen],
                        |frame| action(frame, screen, name),
                        cancel,
                    )
                    .await?;
                    self.wait_until = now + 1000;
                }
            }
            Phase::Close { reason } => {
                if screen == home {
                    update.finished = Some(reason);
                } else if matches!(
                    screen,
                    Screen::WallSelected
                        | Screen::BuildList
                        | Screen::WallConfirm
                        | Screen::BuilderWallConfirm
                ) {
                    let name = if screen == Screen::WallSelected && !list {
                        "build"
                    } else {
                        "close"
                    };
                    adb.tap_checked(
                        detector,
                        &[screen],
                        |frame| action(frame, screen, name),
                        cancel,
                    )
                    .await?;
                    self.wait_until = now + 1000;
                }
            }
        }
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calibration::Calibration,
        gameplay::{
            FrameView, GameVision,
            profile::Profile,
            test_device::{Fake, root},
        },
        vision::{Detection, Detector, Digits},
    };
    use opencv::{
        core::{Rect, Scalar},
        prelude::*,
    };
    use std::{path::Path, sync::Arc};

    #[tokio::test]
    async fn wall_flow_scans_selects_checks_reserves_and_rejects_unconfirmed_spending() {
        let profile = Profile::load(&root()).unwrap();
        let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
        let detector = Arc::new(
            Detector::load(
                &Calibration::load(&root().join("config.json")).unwrap(),
                &root(),
            )
            .unwrap(),
        );
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector,
        };
        for village in [Village::Home, Village::Builder] {
            let (name, selection, listing, amount, cost, gem_y) = match village {
                Village::Home => (
                    "home",
                    "home-wall-selection",
                    "build_list_scroll2",
                    5_048_373,
                    2_500_000,
                    174,
                ),
                Village::Builder => (
                    "builder",
                    "builder-wall-gold",
                    "builder_build_list",
                    2_899_915,
                    240_000,
                    124,
                ),
            };
            let (_, selected) = Fake::new(&format!("english-numeric-{selection}.png"));
            let (_, list) = Fake::new(&format!("english-{listing}.png"));
            let (_, gold) = Fake::new(&format!("english-numeric-{name}-wall-confirm-gold.png"));
            let (_, elixir) = Fake::new(&format!("english-numeric-{name}-wall-confirm-elixir.png"));
            let (_, mut home) = Fake::new(&format!("english-{name}.png"));
            // Replay the same captured balances/gems in both HUD positions; no upgrade was captured.
            for (source, target) in [
                (Rect::new(702, 26, 178, 26), Rect::new(1040, 26, 178, 26)),
                (Rect::new(702, 76, 178, 26), Rect::new(1040, 74, 178, 26)),
                (
                    Rect::new(766, gem_y, 114, 26),
                    Rect::new(1100, gem_y, 114, 26),
                ),
            ] {
                Mat::roi_mut(&mut home, target)
                    .unwrap()
                    .set_to_def(&Scalar::all(0.0))
                    .unwrap();
                Mat::roi(&selected, source)
                    .unwrap()
                    .copy_to(&mut Mat::roi_mut(&mut home, target).unwrap())
                    .unwrap();
            }
            for reserve in [1_000_000, 3_000_000] {
                let temp = tempfile::tempdir().unwrap();
                let mut wall = WallRun::new(None, 0, village, [reserve; 2]);
                let mut confirmations = 0;
                let mut finished = false;
                for step in 1..40 {
                    let image = match wall.phase {
                        Phase::Begin | Phase::Verify { .. } | Phase::Close { .. } => &home,
                        Phase::Open
                        | Phase::Top { .. }
                        | Phase::Scan { .. }
                        | Phase::Find { .. } => &list,
                        Phase::Select | Phase::Choose => &selected,
                        Phase::Confirm {
                            resource: Resource::Gold,
                            ..
                        } => &gold,
                        Phase::Confirm { .. } => &elixir,
                    };
                    let fake = Fake::from_image(image);
                    let mut adb = fake.adb(temp.path());
                    let detection = vision.detector.detect(image).unwrap();
                    let frame = FrameView {
                        image,
                        detection: &detection,
                        elapsed_ms: step * 2000,
                        captured_at_ms: 1_700_000_000_000,
                    };
                    let confirming = matches!(wall.phase, Phase::Confirm { .. });
                    let result = wall
                        .tick(&vision, &mut adb, &frame, false, &CancellationToken::new())
                        .await;
                    if confirming && wall.verification_pending() {
                        confirmations += fake.taps.lock().unwrap().len();
                    }
                    match result {
                        Err(Error::Gameplay(message)) if reserve == 1_000_000 => {
                            assert!(
                                message.contains("spending could not be verified"),
                                "{message}"
                            );
                            assert!(wall.verification_pending());
                            finished = true;
                            break;
                        }
                        Ok(update) => {
                            assert!(update.upgraded.is_none());
                            if let Some(reason) = update.finished {
                                assert!(matches!(reason, Finish::NeedResources));
                                assert_eq!(reserve, 3_000_000);
                                finished = true;
                                break;
                            }
                        }
                        Err(error) => panic!("step {step}: {error}"),
                    }
                }
                assert!(finished, "Wall replay did not finish");
                assert_eq!(confirmations, usize::from(reserve == 1_000_000));
            }
            // Verify accounting independently with a simulated pre-upgrade balance.
            for known_gems in [1543, 1544] {
                let temp = tempfile::tempdir().unwrap();
                let fake = Fake::from_image(&home);
                let mut adb = fake.adb(temp.path());
                let mut wall = WallRun::new(Some(known_gems), 0, village, [1_000_000; 2]);
                wall.phase = Phase::Verify {
                    resource: Resource::Gold,
                    cost,
                    balance: amount + cost,
                };
                let detection = vision.detector.detect(&home).unwrap();
                for step in 1..=2 {
                    let frame = FrameView {
                        image: &home,
                        detection: &detection,
                        elapsed_ms: step * 2000,
                        captured_at_ms: 1_700_000_000_000,
                    };
                    let result = wall
                        .tick(&vision, &mut adb, &frame, false, &CancellationToken::new())
                        .await;
                    if step == 1 {
                        assert!(result.unwrap().upgraded.is_none());
                    } else if known_gems == 1543 {
                        assert_eq!(result.unwrap().upgraded, Some((Resource::Gold, cost)));
                        assert!(!wall.verification_pending());
                    } else {
                        assert!(
                            matches!(result, Err(Error::Gameplay(message)) if message.contains("Gem balance changed"))
                        );
                        assert!(wall.verification_pending());
                    }
                }
                assert!(fake.taps.lock().unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn confirmation_delivered_before_transport_timeout_stays_unverified() {
        let profile = Profile::load(&root()).unwrap();
        let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
        let detector = Arc::new(
            Detector::load(
                &Calibration::load(&root().join("config.json")).unwrap(),
                &root(),
            )
            .unwrap(),
        );
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector,
        };
        let (mut fake, image) = Fake::new("english-wall_confirm_gold.png");
        fake.fail_after_tap = true;
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut wall = WallRun::new(Some(543), 0, Village::Home, [1_000_000, 1_000_000]);
        wall.phase = Phase::Confirm {
            resource: Resource::Gold,
            balance: 30_000_000,
        };
        let frame = FrameView {
            image: &image,
            detection: &Detection::Known(Screen::WallConfirm),
            elapsed_ms: 1000,
            captured_at_ms: 1_700_000_000_000,
        };
        assert!(matches!(
            wall.tick(&vision, &mut adb, &frame, false, &CancellationToken::new())
                .await,
            Err(Error::Timeout)
        ));
        assert_eq!(fake.taps.lock().unwrap().len(), 1);
        assert!(wall.verification_pending());
    }
}
