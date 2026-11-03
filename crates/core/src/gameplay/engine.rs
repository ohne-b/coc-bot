use super::{
    Options, Strategy,
    profile::Profile,
    wall_run::{Finish, WallRun},
};
use crate::{
    Error, Result,
    calibration::Screen,
    device::{Adb, Gesture, Point, process::Transport},
    telemetry::{
        Attack, Balance, Loot, Reading, Resource, ResourceObservation, SessionMode, Village,
        WallUpgrade,
    },
    vision::{
        Detection, Digits,
        army::{self, Card, Deployment},
        crop,
    },
};
use opencv::{
    core::{self, Mat},
    prelude::*,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub operation: String,
    pub village: Village,
    pub searches: u32,
    pub opponent: Loot,
    pub damage: Option<u8>,
    pub stars: Option<u8>,
}
#[derive(Default)]
pub struct Outcome {
    pub attack: Option<Attack>,
    pub wall: Option<WallUpgrade>,
    pub resources: Vec<ResourceObservation>,
    pub completed: bool,
}
#[derive(Clone)]
enum Stage {
    Settle,
    Navigate,
    Zoom(u8),
    Cart(CartPhase),
    Check {
        index: usize,
        opened: bool,
    },
    Menu,
    Scout,
    Next {
        previous: Mat,
        stable: Option<Mat>,
    },
    Plan {
        screen: Screen,
        pinches: u8,
        previous: Option<Vec<Card>>,
    },
    Deploy {
        source: Screen,
        steps: VecDeque<Deployment>,
        selected: bool,
        sent: u32,
        waited: bool,
    },
    Battle,
    Result {
        screen: Screen,
        previous: Option<super::results::BattleResult>,
    },
    Surrender,
    Return,
    Walls,
    Update {
        version: String,
        presses: u8,
    },
    Done,
}

#[derive(Clone, Copy)]
enum CartPhase {
    Find,
    Claim,
    Read {
        previous: Option<(u64, u64)>,
        started: u64,
    },
    Close,
}

pub struct Engine {
    pub options: Options,
    pub mode: SessionMode,
    stage: Stage,
    since: u64,
    not_before: u64,
    last_action: u64,
    village_index: usize,
    pans: u8,
    attacks: u32,
    upgrades: u32,
    searches: u32,
    pending_attack: bool,
    battle_started: u64,
    climbing: bool,
    previous_damage: Option<u8>,
    previous_stars: Option<u8>,
    heroes: Vec<Point>,
    hero_ready_at: u64,
    speed_checked: bool,
    capacities: BTreeMap<(String, String), u32>,
    values: BTreeMap<String, u32>,
    opponent: Loot,
    full: Vec<Village>,
    recovery_count: u8,
    wall: Option<WallRun>,
    known_gems: Option<u32>,
    wall_started_upgrades: u32,
    unknown_since: Option<u64>,
    out_of_gold: bool,
    broke_attacks: u8,
    cart_full: bool,
    cart_misses: u8,
}
impl Engine {
    pub fn new(options: Options, mode: SessionMode, profile: &Profile) -> Result<Self> {
        options.validate(profile.resolution)?;
        if mode == SessionMode::Farming
            && options.until_full
            && options.cart_goal
            && options.villages.contains(&Village::Builder)
            && profile.cart_text.is_none()
        {
            return Err(Error::Calibration(
                "Elixir Cart capacity is not calibrated".into(),
            ));
        }
        if mode == SessionMode::Observe {
            return Err(Error::InputDisabled);
        }
        Ok(Self {
            options,
            mode,
            stage: Stage::Settle,
            since: 0,
            not_before: 0,
            last_action: 0,
            village_index: 0,
            pans: 0,
            attacks: 0,
            upgrades: 0,
            searches: 0,
            pending_attack: false,
            battle_started: 0,
            climbing: false,
            previous_damage: None,
            previous_stars: None,
            heroes: vec![],
            hero_ready_at: 0,
            speed_checked: false,
            capacities: BTreeMap::new(),
            values: BTreeMap::new(),
            opponent: Loot::default(),
            full: vec![],
            recovery_count: 0,
            wall: None,
            known_gems: None,
            wall_started_upgrades: 0,
            unknown_since: None,
            out_of_gold: false,
            broke_attacks: 0,
            cart_full: false,
            cart_misses: 0,
        })
    }
    pub fn village(&self) -> Village {
        self.options.villages[self.village_index]
    }
    pub fn can_restart_between_attacks(&self) -> bool {
        matches!(self.stage, Stage::Settle | Stage::Navigate)
            && !self.pending_attack
            && !self
                .wall
                .as_ref()
                .is_some_and(WallRun::verification_pending)
    }
    pub fn progress(&self) -> Progress {
        let operation = match self.stage {
            Stage::Settle => "Recognizing village",
            Stage::Navigate => "Navigating",
            Stage::Zoom(_) => "Adjusting view",
            Stage::Cart(_) => "Collecting Elixir Cart",
            Stage::Check { .. } => "Checking resources",
            Stage::Menu => "Finding an opponent",
            Stage::Scout | Stage::Next { .. } => "Scouting",
            Stage::Plan { .. } => "Reading army",
            Stage::Deploy { .. } => "Deploying",
            Stage::Battle => "In battle",
            Stage::Result { .. } => "Reading result",
            Stage::Surrender => "Ending battle",
            Stage::Return => "Returning to village",
            Stage::Walls => self
                .wall
                .as_ref()
                .map_or("Upgrading walls", WallRun::operation),
            Stage::Done => "Completed",
            Stage::Update { .. } => "Updating game",
        };
        Progress {
            operation: operation.into(),
            village: self.village(),
            searches: self.searches,
            opponent: self.opponent,
            damage: self.previous_damage,
            stars: self.previous_stars,
        }
    }
    fn go(&mut self, stage: Stage, now: u64) {
        self.stage = stage;
        self.since = now;
        self.not_before = now;
        self.last_action = 0;
        self.pans = 0;
    }
    fn delay(&mut self, now: u64, millis: u64) {
        self.not_before = now.saturating_add(millis);
    }
    pub fn recover(&mut self, now: u64) -> Result<u8> {
        if self
            .wall
            .as_ref()
            .is_some_and(WallRun::verification_pending)
        {
            return Err(Error::Gameplay(
                "Wall spending is unverified; stopped before another upgrade".into(),
            ));
        }
        if self.recovery_count >= self.options.recoveries {
            return Err(Error::Gameplay("Recovery limit reached".into()));
        }
        self.recovery_count += 1;
        self.pending_attack = false;
        self.heroes.clear();
        self.unknown_since = None;
        self.go(Stage::Settle, now);
        self.delay(now, 3000);
        Ok(self.recovery_count)
    }
    fn missed_cart(&mut self) -> Result<()> {
        self.cart_misses = self.cart_misses.saturating_add(1);
        if self.mode == SessionMode::Farming
            && self.options.until_full
            && self.options.cart_goal
            && self.cart_misses >= 5
        {
            return Err(Error::Gameplay(
                "Elixir Cart unavailable for five visits; farming stopped".into(),
            ));
        }
        Ok(())
    }
    fn home(&self) -> Screen {
        match self.village() {
            Village::Home => Screen::Home,
            Village::Builder => Screen::Builder,
        }
    }
    fn name(&self) -> &'static str {
        match self.village() {
            Village::Home => "home",
            Village::Builder => "builder",
        }
    }
    fn limit(&self, now: u64) -> bool {
        (self.options.attacks > 0 && self.attacks >= self.options.attacks)
            || (self.options.minutes > 0 && now >= u64::from(self.options.minutes) * 60_000)
            || (self.options.wall_upgrades > 0 && self.upgrades >= self.options.wall_upgrades)
    }
    fn next_village(&mut self, now: u64) -> bool {
        if let Some(index) = self
            .options
            .villages
            .iter()
            .position(|v| !self.full.contains(v))
        {
            self.village_index = index;
            self.go(Stage::Navigate, now);
            false
        } else {
            self.go(Stage::Done, now);
            true
        }
    }
    async fn action<T: Transport>(
        &mut self,
        vision: &super::GameVision<'_>,
        adb: &mut Adb<T>,
        screen: Screen,
        name: &str,
        now: u64,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let profile = vision.profile;
        let detector = vision.detector.clone();
        if self.last_action != 0 && now.saturating_sub(self.last_action) < 2000 {
            return Ok(false);
        }
        adb.tap_checked(
            detector,
            &[screen],
            |image| Ok((profile.action(screen, name, image)?, ())),
            cancel,
        )
        .await?;
        self.last_action = now.max(1);
        self.delay(now, 500);
        Ok(true)
    }
    fn surrender_reached(
        &mut self,
        profile: &Profile,
        digits: &Digits,
        image: &Mat,
        screen: Screen,
        now: u64,
    ) -> bool {
        if self.village() == Village::Builder && self.climbing {
            return false;
        }
        if self.options.surrender_after > 0
            && now.saturating_sub(self.battle_started)
                >= u64::from(self.options.surrender_after) * 1000
        {
            return true;
        }
        if screen == Screen::HomeBattle && self.options.surrender_damage > 0 {
            let current = profile
                .screens
                .get(&screen)
                .and_then(|s| s.damage)
                .and_then(|r| crop(image, r).ok())
                .and_then(|p| digits.percent(&p).ok());
            let reached = current
                .zip(self.previous_damage)
                .is_some_and(|(a, b)| a.min(b) >= self.options.surrender_damage);
            self.previous_damage = current;
            return reached;
        }
        if matches!(screen, Screen::BuilderBattle | Screen::BuilderBattle2)
            && self.options.surrender_stars > 0
        {
            let current = profile
                .screens
                .get(&Screen::BuilderBattle)
                .and_then(|s| army::stars(image, &s.stars).ok());
            let reached = current
                .zip(self.previous_stars)
                .is_some_and(|(a, b)| a.min(b) >= self.options.surrender_stars);
            self.previous_stars = current;
            return reached;
        }
        false
    }
    fn finish_attack(&mut self, screen: Screen, now: u64, at: i64, outcome: &mut Outcome) {
        if self.pending_attack {
            self.attacks += 1;
            self.broke_attacks = if self.out_of_gold {
                self.broke_attacks + 1
            } else {
                0
            };
            self.recovery_count = 0;
            if matches!(screen, Screen::HomeResult | Screen::BuilderResult) {
                outcome.attack = Some(Attack {
                    ordinal: self.attacks,
                    village: self.village(),
                    completed_at_ms: at,
                    loot: Loot::default(),
                    damage: self.previous_damage,
                    stars: self.previous_stars,
                });
            }
            self.pending_attack = false;
        }
        self.heroes.clear();
        self.go(Stage::Return, now);
    }
    pub async fn tick<T: Transport>(
        &mut self,
        vision: &super::GameVision<'_>,
        adb: &mut Adb<T>,
        frame: &super::FrameView<'_>,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        let profile = vision.profile;
        let digits = vision.digits;
        let detector = vision.detector.clone();
        let image = frame.image;
        let detection = frame.detection;
        let now = frame.elapsed_ms;
        let at = frame.captured_at_ms;
        let mut outcome = Outcome::default();
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if matches!(self.stage, Stage::Done) {
            outcome.completed = true;
            return Ok(outcome);
        }
        if now < self.not_before {
            return Ok(outcome);
        }
        if let Stage::Update { version, presses } = self.stage.clone() {
            if let Some(current) = adb.game_version(cancel).await?
                && current != version
            {
                adb.launch_game(cancel).await?;
                self.go(Stage::Settle, now);
                self.delay(now, 3000);
                return Ok(outcome);
            }
            if now.saturating_sub(self.since) > 900_000 {
                return Err(Error::Gameplay(
                    "Game update timed out; check the Play Store".into(),
                ));
            }
            if presses < 3 && adb.press_store_update(cancel).await? {
                self.stage = Stage::Update {
                    version,
                    presses: presses + 1,
                };
            } else if presses == 0 && now.saturating_sub(self.since) > 60_000 {
                return Err(Error::Gameplay(
                    "No supported Update button appeared in the Play Store".into(),
                ));
            }
            self.delay(now, 5000);
            return Ok(outcome);
        }
        if !matches!(self.stage, Stage::Walls) && now.saturating_sub(self.since) > 240_000 {
            return Err(Error::Timeout);
        }
        let Detection::Known(screen) = *detection else {
            let since = *self.unknown_since.get_or_insert(now);
            if now.saturating_sub(since) >= 30_000 {
                return Err(Error::Timeout);
            }
            if let Stage::Next { stable, .. } = &mut self.stage {
                *stable = None;
            }
            if let Stage::Result { previous, .. } = &mut self.stage {
                *previous = None;
            }
            return Ok(outcome);
        };
        self.unknown_since = None;
        if matches!(screen, Screen::UpdateOffer | Screen::BuildersBusy)
            && self
                .wall
                .as_ref()
                .is_some_and(WallRun::verification_pending)
        {
            return Err(Error::Gameplay(
                "Wall spending is unverified; stopped before changing operations".into(),
            ));
        }
        let popup = match screen {
            Screen::League => Some("close"),
            Screen::Welcome
            | Screen::LayoutBlocked
            | Screen::StarBonus
            | Screen::BuilderStarBonus => Some("ok"),
            Screen::RewardChest => Some("open"),
            Screen::RewardCard => Some("continue"),
            _ => None,
        };
        if let Some(action) = popup {
            self.action(vision, adb, screen, action, now, cancel)
                .await?;
            return Ok(outcome);
        }
        if screen == Screen::GemOffer && matches!(self.stage, Stage::Next { .. }) {
            if !self
                .action(vision, adb, screen, "close", now, cancel)
                .await?
            {
                return Ok(outcome);
            }
            self.out_of_gold = true;
            self.go(Stage::Scout, now);
            self.delay(now, 1000);
            return Ok(outcome);
        }
        if screen == Screen::BuildersBusy && matches!(self.stage, Stage::Walls) {
            if !self
                .action(vision, adb, screen, "close", now, cancel)
                .await?
            {
                return Ok(outcome);
            }
            self.full.push(self.village());
            outcome.completed = self.next_village(now);
            return Ok(outcome);
        }
        if matches!(screen, Screen::GemOffer | Screen::BuildersBusy) {
            self.action(vision, adb, screen, "close", now, cancel)
                .await?;
            return Err(Error::Gameplay(
                "A resource or builder offer interrupted the operation; no purchase was made"
                    .into(),
            ));
        }
        if screen == Screen::UpdateOffer {
            let version = adb.game_version(cancel).await?.ok_or_else(|| {
                Error::Gameplay("Installed game version could not be read".into())
            })?;
            if !self
                .action(vision, adb, screen, "update", now, cancel)
                .await?
            {
                return Ok(outcome);
            }
            self.go(
                Stage::Update {
                    version,
                    presses: 0,
                },
                now,
            );
            self.delay(now, 2000);
            return Ok(outcome);
        }
        if screen == Screen::OtherDevice {
            return Err(Error::Gameplay(
                "The account is active on another device".into(),
            ));
        }
        if matches!(
            screen,
            Screen::Disconnected | Screen::Idle | Screen::SearchFailed
        ) {
            return Err(Error::Timeout);
        }
        let in_combat = matches!(
            self.stage,
            Stage::Deploy { .. } | Stage::Battle | Stage::Surrender | Stage::Plan { .. }
        );
        if in_combat
            && matches!(
                screen,
                Screen::HomeResult | Screen::BuilderResult | Screen::Home | Screen::Builder
            )
        {
            if matches!(screen, Screen::HomeResult | Screen::BuilderResult) {
                self.go(
                    Stage::Result {
                        screen,
                        previous: None,
                    },
                    now,
                );
                // Result counters and stars animate after the return button appears.
                self.delay(now, 3000);
            } else {
                self.finish_attack(screen, now, at, &mut outcome);
            }
            return Ok(outcome);
        }
        if in_combat
            && screen == Screen::BuilderStage2
            && !matches!(
                self.stage,
                Stage::Plan {
                    screen: Screen::BuilderStage2,
                    ..
                } | Stage::Deploy {
                    source: Screen::BuilderStage2,
                    ..
                } | Stage::Surrender
            )
        {
            if self.options.surrender_stars > 0 && !self.climbing {
                self.go(Stage::Surrender, now);
            } else {
                self.go(
                    Stage::Plan {
                        screen,
                        pinches: 0,
                        previous: None,
                    },
                    now,
                );
                self.delay(now, 1500);
            }
            return Ok(outcome);
        }
        if matches!(self.stage, Stage::Deploy { .. } | Stage::Battle)
            && self.surrender_reached(profile, digits, image, screen, now)
        {
            self.go(Stage::Surrender, now);
            return Ok(outcome);
        }
        match self.stage.clone() {
            Stage::Settle => match screen {
                Screen::Home | Screen::Builder => self.go(Stage::Navigate, now),
                Screen::HomeResult | Screen::BuilderResult => {
                    self.action(vision, adb, screen, "return", now, cancel)
                        .await?;
                }
                Screen::HomeMenu
                | Screen::HomeArmy
                | Screen::BuilderMenu
                | Screen::BuildList
                | Screen::WallConfirm
                | Screen::BuilderWallConfirm
                | Screen::BuilderCart => {
                    self.action(vision, adb, screen, "close", now, cancel)
                        .await?;
                }
                Screen::WallSelected => {
                    let action = if detector.matches(image, Screen::BuildList)? {
                        "close"
                    } else {
                        "build"
                    };
                    self.action(vision, adb, screen, action, now, cancel)
                        .await?;
                }
                Screen::HomeBattle | Screen::BuilderBattle | Screen::BuilderBattle2 => {}
                Screen::HomeScout | Screen::BuilderScout | Screen::BuilderStage2 => {
                    let village = if screen == Screen::HomeScout {
                        Village::Home
                    } else {
                        Village::Builder
                    };
                    if let Some(index) = self.options.villages.iter().position(|v| *v == village) {
                        self.village_index = index;
                        self.go(
                            Stage::Plan {
                                screen,
                                pinches: 0,
                                previous: None,
                            },
                            now,
                        );
                    }
                }
                _ => {}
            },
            Stage::Navigate => {
                if self.limit(now) {
                    self.go(Stage::Done, now);
                    outcome.completed = true;
                } else if screen == self.home() {
                    self.go(Stage::Zoom(0), now);
                } else if matches!(screen, Screen::Home | Screen::Builder) {
                    let target =
                        profile
                            .screen(screen)?
                            .targets
                            .get("switch")
                            .ok_or_else(|| {
                                Error::Calibration("Village switch is not calibrated".into())
                            })?;
                    if (!target.search_first && self.pans == 0) || target.locate(image)?.is_none() {
                        if let Some([start, end]) =
                            target.pan.filter(|_| self.pans < target.pan_attempts)
                        {
                            adb.swipe(detector, &[screen], start, end, cancel).await?;
                            self.pans += 1;
                            self.delay(now, 2000);
                        } else {
                            return Err(Error::Unreadable);
                        }
                    } else {
                        self.action(vision, adb, screen, "switch", now, cancel)
                            .await?;
                    }
                }
            }
            Stage::Zoom(count) => {
                if screen == self.home() {
                    if let Some(zoom) = &profile.zoom_out
                        && count < zoom.pinches
                    {
                        adb.gesture(
                            detector,
                            &[screen],
                            Gesture::Pinch {
                                center: zoom.center,
                                start_gap: zoom.gap[0],
                                end_gap: zoom.gap[1],
                            },
                            cancel,
                        )
                        .await?;
                        self.stage = Stage::Zoom(count + 1);
                        self.delay(now, 1500);
                    } else {
                        self.cart_full = false;
                        self.go(
                            if self.village() == Village::Builder && self.options.collect_cart {
                                Stage::Cart(CartPhase::Find)
                            } else {
                                Stage::Check {
                                    index: 0,
                                    opened: false,
                                }
                            },
                            now,
                        );
                    }
                } else if matches!(screen, Screen::WallSelected | Screen::BuildList) {
                    self.action(
                        vision,
                        adb,
                        screen,
                        if screen == Screen::WallSelected
                            && !detector.matches(image, Screen::BuildList)?
                        {
                            "build"
                        } else {
                            "close"
                        },
                        now,
                        cancel,
                    )
                    .await?;
                }
            }
            Stage::Cart(step) => {
                if screen == Screen::Builder && matches!(step, CartPhase::Find) {
                    if let Some(target) = profile.screen(screen)?.targets.get("cart") {
                        if target.locate(image)?.is_some() {
                            if self
                                .action(vision, adb, screen, "cart", now, cancel)
                                .await?
                            {
                                self.go(Stage::Cart(CartPhase::Claim), now);
                            }
                        } else if let Some([start, end]) =
                            target.pan.filter(|_| self.pans < target.pan_attempts)
                        {
                            adb.swipe(detector, &[screen], start, end, cancel).await?;
                            self.pans += 1;
                            self.delay(now, 2000);
                        } else {
                            self.missed_cart()?;
                            self.go(
                                Stage::Check {
                                    index: 0,
                                    opened: false,
                                },
                                now,
                            );
                        }
                    } else {
                        self.missed_cart()?;
                        self.go(
                            Stage::Check {
                                index: 0,
                                opened: false,
                            },
                            now,
                        );
                    }
                } else if screen == Screen::BuilderCart {
                    self.cart_misses = 0;
                    if let CartPhase::Read { previous, started } = step {
                        let reading = profile
                            .cart_text
                            .and_then(|region| crop(image, region).ok())
                            .and_then(|patch| digits.fraction(&patch).ok());
                        if let Some((amount, capacity)) =
                            reading.filter(|value| Some(*value) == previous)
                        {
                            self.cart_full = amount == capacity;
                            self.stage = Stage::Cart(CartPhase::Close);
                        } else if now.saturating_sub(started) >= 2000 {
                            return Err(Error::Unreadable);
                        } else {
                            self.stage = Stage::Cart(CartPhase::Read {
                                previous: reading,
                                started,
                            });
                            self.delay(now, 250);
                        }
                        return Ok(outcome);
                    }
                    if self
                        .action(
                            vision,
                            adb,
                            screen,
                            if matches!(step, CartPhase::Close) {
                                "close"
                            } else {
                                "claim"
                            },
                            now,
                            cancel,
                        )
                        .await?
                    {
                        if !matches!(step, CartPhase::Close)
                            && self.mode == SessionMode::Farming
                            && self.options.until_full
                            && self.options.cart_goal
                        {
                            self.go(
                                Stage::Cart(CartPhase::Read {
                                    previous: None,
                                    started: now,
                                }),
                                now,
                            );
                            self.delay(now, 250);
                        } else {
                            self.stage = Stage::Cart(CartPhase::Close);
                        }
                    }
                } else if screen == Screen::Builder
                    && (matches!(step, CartPhase::Close) || now.saturating_sub(self.since) > 5000)
                {
                    if !matches!(step, CartPhase::Close) {
                        self.missed_cart()?;
                    }
                    self.go(
                        Stage::Check {
                            index: 0,
                            opened: false,
                        },
                        now,
                    );
                }
            }
            Stage::Check { index, opened } => {
                if screen != self.home() {
                    return Ok(outcome);
                }
                if self.limit(now) {
                    self.go(Stage::Done, now);
                    outcome.completed = true;
                    return Ok(outcome);
                }
                let resources = if self.mode == SessionMode::Walls {
                    vec![Resource::Gold, Resource::Elixir]
                } else {
                    self.options.targets.clone()
                };
                let resources: Vec<_> = resources
                    .into_iter()
                    .filter(|r| self.village() == Village::Home || *r != Resource::Dark)
                    .collect();
                if (self.options.until_full || self.mode == SessionMode::Walls)
                    && index < resources.len()
                {
                    let resource = resources[index];
                    let key = resource_name(resource);
                    let spec = profile
                        .capacity
                        .get(self.name())
                        .and_then(|r| r.get(key))
                        .ok_or_else(|| {
                            Error::Calibration(format!("Missing {key} capacity calibration"))
                        })?;
                    let amount =
                        u32::try_from(digits.trailing(&crop(image, spec.bar)?, 0.65, 110)?)
                            .map_err(|_| Error::Unreadable)?;
                    let cache_key = (self.name().to_owned(), key.to_owned());
                    if self.capacities.get(&cache_key).is_some_and(|v| amount > *v) {
                        self.capacities.remove(&cache_key);
                    }
                    let maximum = if let Some(maximum) = self.capacities.get(&cache_key) {
                        Some(*maximum)
                    } else if spec.label.locate(image)?.is_some() {
                        let value =
                            u32::try_from(digits.number(&crop(image, spec.maximum)?, 0.65)?)
                                .map_err(|_| Error::Unreadable)?;
                        if value == 0 || value < amount {
                            return Err(Error::Unreadable);
                        }
                        self.capacities.insert(cache_key, value);
                        Some(value)
                    } else {
                        None
                    };
                    if let Some(maximum) = maximum {
                        self.values.insert(key.into(), amount);
                        outcome.resources.push(ResourceObservation {
                            village: self.village(),
                            resource,
                            checked_at_ms: at,
                            reading: Reading::Known(Balance {
                                amount,
                                capacity: Some(maximum),
                            }),
                        });
                        if opened && spec.label.locate(image)?.is_some() {
                            adb.tap(detector, &[screen], spec.tap, cancel).await?;
                            self.delay(now, 1000);
                        }
                        self.stage = Stage::Check {
                            index: index + 1,
                            opened: false,
                        };
                        self.since = now;
                    } else if !opened {
                        adb.tap(detector, &[screen], spec.tap, cancel).await?;
                        self.stage = Stage::Check {
                            index,
                            opened: true,
                        };
                        self.since = now;
                        self.delay(now, 500);
                    } else if now.saturating_sub(self.since) > 15_000 {
                        return Err(Error::Unreadable);
                    }
                } else {
                    let cart_goal = self.mode == SessionMode::Farming
                        && self.options.until_full
                        && self.village() == Village::Builder
                        && self.options.cart_goal;
                    let full = (!resources.is_empty() || cart_goal)
                        && (!cart_goal || self.cart_full)
                        && resources.iter().all(|r| {
                            let key = resource_name(*r);
                            self.values
                                .get(key)
                                .zip(self.capacities.get(&(self.name().into(), key.into())))
                                .is_some_and(|(amount, maximum)| {
                                    *amount
                                        >= if self.mode == SessionMode::Walls
                                            && self.options.wall_farm_threshold > 0
                                        {
                                            (*maximum).min(self.options.wall_farm_threshold)
                                        } else {
                                            *maximum
                                        }
                                })
                        });
                    if self.mode == SessionMode::Walls
                        && (full || self.options.wall_farm_threshold == 0)
                    {
                        self.wall = Some(WallRun::new(
                            self.known_gems,
                            now,
                            self.village(),
                            [self.options.gold_reserve, self.options.elixir_reserve],
                        ));
                        self.wall_started_upgrades = self.upgrades;
                        self.go(Stage::Walls, now);
                    } else if full && self.options.until_full {
                        self.full.push(self.village());
                        outcome.completed = self.next_village(now);
                    } else {
                        self.climbing = if self.village() == Village::Builder
                            && self.options.trophy_floor > 0
                        {
                            profile
                                .screen(screen)?
                                .trophies
                                .and_then(|r| crop(image, r).ok())
                                .and_then(|p| digits.number(&p, 0.75).ok())
                                .is_some_and(|v| v < u64::from(self.options.trophy_floor))
                        } else {
                            false
                        };
                        self.searches = 0;
                        self.out_of_gold = false;
                        self.opponent = Loot::default();
                        self.go(Stage::Menu, now);
                    }
                }
            }
            Stage::Menu => {
                let allowed = match self.village() {
                    Village::Home => [
                        Screen::Home,
                        Screen::HomeMenu,
                        Screen::HomeArmy,
                        Screen::HomeScout,
                    ]
                    .as_slice(),
                    Village::Builder => {
                        [Screen::Builder, Screen::BuilderMenu, Screen::BuilderScout].as_slice()
                    }
                };
                if !allowed.contains(&screen) {
                    return Ok(outcome);
                }
                let action = match screen {
                    Screen::Home | Screen::Builder => Some("attack"),
                    Screen::HomeMenu | Screen::BuilderMenu => Some("find"),
                    Screen::HomeArmy => Some("attack"),
                    _ => None,
                };
                if let Some(action) = action {
                    self.action(vision, adb, screen, action, now, cancel)
                        .await?;
                } else if screen == Screen::HomeScout {
                    self.searches = 1;
                    self.go(Stage::Scout, now);
                } else if screen == Screen::BuilderScout {
                    self.go(
                        Stage::Plan {
                            screen,
                            pinches: 0,
                            previous: None,
                        },
                        now,
                    );
                }
            }
            Stage::Scout => {
                if screen != Screen::HomeScout {
                    return Ok(outcome);
                }
                let loot = &profile.screen(screen)?.loot;
                let read = |key: &str| {
                    loot.get(key)
                        .and_then(|r| crop(image, *r).ok())
                        .and_then(|p| digits.number(&p, 0.75).ok())
                        .and_then(|v| u32::try_from(v).ok())
                };
                self.opponent = Loot {
                    gold: read("gold"),
                    elixir: read("elixir"),
                    dark: read("dark"),
                };
                let passes = [
                    (self.options.minimum_gold, self.opponent.gold),
                    (self.options.minimum_elixir, self.opponent.elixir),
                    (self.options.minimum_dark, self.opponent.dark),
                ]
                .into_iter()
                .all(|(min, value)| min == 0 || value.is_some_and(|v| v >= min));
                if passes
                    || self.out_of_gold
                    || (self.options.searches > 0 && self.searches >= self.options.searches)
                {
                    if passes {
                        self.out_of_gold = false;
                    }
                    self.go(
                        Stage::Plan {
                            screen,
                            pinches: 0,
                            previous: None,
                        },
                        now,
                    );
                } else if now.saturating_sub(self.since) >= 3000 {
                    self.action(vision, adb, screen, "next", now, cancel)
                        .await?;
                    self.go(
                        Stage::Next {
                            previous: image.clone(),
                            stable: None,
                        },
                        now,
                    );
                }
            }
            Stage::Next { previous, stable } => {
                if screen == Screen::HomeScout {
                    if changed(&previous, image)? {
                        if let Some(stable) = stable
                            && difference(&stable, image)? < 2.0
                        {
                            self.searches += 1;
                            self.go(Stage::Scout, now);
                        } else {
                            self.stage = Stage::Next {
                                previous,
                                stable: Some(image.clone()),
                            };
                        }
                    } else {
                        self.stage = Stage::Next {
                            previous,
                            stable: None,
                        };
                    }
                }
                if now.saturating_sub(self.since) > 60_000 {
                    return Err(Error::Timeout);
                }
            }
            Stage::Plan {
                screen: source,
                pinches,
                previous,
            } => {
                let allowed = deploy_screens(source);
                if !allowed.contains(&screen) {
                    return Ok(outcome);
                }
                if let Some(zoom) = &profile.zoom_out
                    && pinches < zoom.battle_pinches.unwrap_or(zoom.pinches)
                {
                    adb.gesture(
                        detector,
                        &allowed,
                        Gesture::Pinch {
                            center: zoom.center,
                            start_gap: zoom.gap[0],
                            end_gap: zoom.gap[1],
                        },
                        cancel,
                    )
                    .await?;
                    self.stage = Stage::Plan {
                        screen: source,
                        pinches: pinches + 1,
                        previous: None,
                    };
                    self.delay(now, 1500);
                    return Ok(outcome);
                }
                let spec = profile.screen(source)?;
                let layout = self
                    .options
                    .layouts
                    .get(&source)
                    .or(spec.auto.as_ref())
                    .ok_or_else(|| Error::Calibration("Army bar is not calibrated".into()))?;
                let cards = army::cards(image, layout.bar, source != Screen::HomeScout, digits)?;
                if cards.is_empty() {
                    return Ok(outcome);
                }
                if source != Screen::HomeScout && previous.as_ref() != Some(&cards) {
                    self.stage = Stage::Plan {
                        screen: source,
                        pinches,
                        previous: Some(cards),
                    };
                    return Ok(outcome);
                }
                let deploy = layout
                    .deploy
                    .or(spec.deploy)
                    .ok_or_else(|| Error::Calibration("Deployment point is missing".into()))?;
                let mut steps: VecDeque<_> = army::plan(
                    &cards,
                    layout,
                    deploy,
                    self.options.strategy == Strategy::Ring,
                    self.options.earthquake,
                    self.options.builder_stack,
                )?
                .into();
                if let Some(sweep) = &spec.sweep {
                    for slot in &sweep.slots {
                        steps.push_back(Deployment::Spread {
                            slot: *slot,
                            points: vec![sweep.deploy],
                            count: 12,
                            burst: 4,
                            fingers: false,
                            spaced: false,
                        });
                    }
                }
                self.heroes = if source == Screen::HomeScout {
                    vec![]
                } else {
                    cards
                        .iter()
                        .filter(|c| c.kind == army::Kind::Hero)
                        .map(|c| c.slot)
                        .collect()
                };
                self.hero_ready_at = now + 3000;
                if source != Screen::BuilderStage2 {
                    self.battle_started = now;
                    self.previous_damage = None;
                    self.previous_stars = None;
                }
                self.pending_attack = true;
                self.speed_checked = false;
                self.go(
                    Stage::Deploy {
                        source,
                        steps,
                        selected: false,
                        sent: 0,
                        waited: false,
                    },
                    now,
                );
            }
            Stage::Deploy {
                source,
                mut steps,
                mut selected,
                mut sent,
                mut waited,
            } => {
                let allowed = deploy_screens(source);
                if !allowed.contains(&screen) {
                    return Ok(outcome);
                }
                let Some(step) = steps.front().cloned() else {
                    self.go(Stage::Battle, now);
                    return Ok(outcome);
                };
                let mut done = false;
                match step {
                    Deployment::Ability { slots, wait_ms } => {
                        if !waited && wait_ms > 0 {
                            waited = true;
                            self.delay(now, u64::from(wait_ms));
                        } else {
                            for chunk in slots.chunks(32) {
                                adb.gesture(
                                    detector.clone(),
                                    &allowed,
                                    Gesture::Taps(chunk.to_vec()),
                                    cancel,
                                )
                                .await?;
                            }
                            done = true;
                            self.hero_ready_at = now + 3000;
                        }
                    }
                    Deployment::Drop { slot, point, count } => {
                        let mut points = if selected { vec![] } else { vec![slot] };
                        selected = true;
                        let batch = count.saturating_sub(sent).min(31);
                        points.extend(std::iter::repeat_n(point, batch as usize));
                        adb.gesture(detector, &allowed, Gesture::Taps(points), cancel)
                            .await?;
                        sent += batch;
                        done = sent >= count;
                    }
                    Deployment::Hold { slot, seconds } => {
                        if army::empty(image, slot)? {
                            done = true;
                        } else if !selected {
                            adb.gesture(detector, &allowed, Gesture::Taps(vec![slot]), cancel)
                                .await?;
                            selected = true;
                        } else {
                            let spec = profile.screen(source)?;
                            let point = self
                                .options
                                .layouts
                                .get(&source)
                                .or(spec.auto.as_ref())
                                .and_then(|layout| layout.deploy)
                                .or(spec.deploy)
                                .ok_or(Error::InputGuard)?;
                            adb.gesture(
                                detector,
                                &allowed,
                                Gesture::Hold {
                                    points: vec![point],
                                    duration: Duration::from_secs(1),
                                },
                                cancel,
                            )
                            .await?;
                            sent += 1;
                            done = sent >= seconds;
                        }
                    }
                    Deployment::Spread {
                        slot,
                        points,
                        count,
                        burst,
                        fingers,
                        spaced,
                    } => {
                        if army::empty(image, slot)? {
                            done = true;
                        } else if !selected {
                            adb.gesture(detector, &allowed, Gesture::Taps(vec![slot]), cancel)
                                .await?;
                            selected = true;
                        } else {
                            let n = if fingers {
                                if spaced {
                                    8.min(points.len() as u32)
                                } else {
                                    8.min(count.saturating_sub(sent))
                                }
                            } else {
                                burst.min(32).min(count.saturating_sub(sent))
                            };
                            if n == 0 {
                                done = true;
                            } else {
                                let spots = (0..n)
                                    .map(|i| {
                                        points[if spaced {
                                            (sent as usize + i as usize * points.len() / n as usize)
                                                % points.len()
                                        } else {
                                            (sent + i) as usize % points.len()
                                        }]
                                    })
                                    .collect();
                                let gesture = if fingers {
                                    Gesture::Hold {
                                        points: spots,
                                        duration: Duration::from_millis(if spaced {
                                            1000
                                        } else {
                                            30
                                        }),
                                    }
                                } else {
                                    Gesture::Taps(spots)
                                };
                                adb.gesture(detector, &allowed, gesture, cancel).await?;
                                sent += if spaced { 1 } else { n };
                                done = sent >= if spaced { 60 } else { count };
                            }
                        }
                    }
                }
                if done {
                    steps.pop_front();
                    selected = false;
                    sent = 0;
                    waited = false;
                }
                self.stage = Stage::Deploy {
                    source,
                    steps,
                    selected,
                    sent,
                    waited,
                };
            }
            Stage::Battle => {
                if matches!(
                    screen,
                    Screen::HomeBattle | Screen::BuilderBattle | Screen::BuilderBattle2
                ) {
                    if !self.speed_checked {
                        self.speed_checked = true;
                        if let Some(target) = &profile.speed_button
                            && target.locate(image)?.is_some()
                        {
                            adb.tap_checked(
                                detector,
                                &[screen],
                                |frame| Ok((target.locate(frame)?.ok_or(Error::Unreadable)?, ())),
                                cancel,
                            )
                            .await?;
                            return Ok(outcome);
                        }
                    }
                    if now >= self.hero_ready_at {
                        for slot in &self.heroes {
                            if army::hero_ready(image, *slot)? {
                                adb.gesture(
                                    detector.clone(),
                                    &[screen],
                                    Gesture::Taps(vec![*slot]),
                                    cancel,
                                )
                                .await?;
                                self.hero_ready_at = now + 3000;
                                break;
                            }
                        }
                    }
                }
            }
            Stage::Result {
                screen: expected,
                previous,
            } => {
                if screen == expected {
                    let reading = super::results::read(
                        &profile.screen(screen)?.result_readings,
                        image,
                        digits,
                        self.village(),
                    )?;
                    let stable = previous.as_ref() == Some(&reading);
                    if stable || now.saturating_sub(self.since) >= 10_000 {
                        let reading = if stable {
                            reading
                        } else {
                            super::results::BattleResult::default()
                        };
                        self.finish_attack(screen, now, at, &mut outcome);
                        if let Some(attack) = &mut outcome.attack {
                            attack.loot = reading.loot;
                            attack.damage = reading.damage;
                            attack.stars = reading.stars;
                        }
                    } else {
                        self.stage = Stage::Result {
                            screen,
                            previous: Some(reading),
                        };
                        self.delay(now, 500);
                    }
                } else if matches!(screen, Screen::Home | Screen::Builder) {
                    self.finish_attack(screen, now, at, &mut outcome);
                }
            }
            Stage::Surrender => {
                if screen == Screen::SurrenderConfirm {
                    self.action(vision, adb, screen, "confirm", now, cancel)
                        .await?;
                } else if matches!(
                    screen,
                    Screen::HomeBattle
                        | Screen::BuilderBattle
                        | Screen::BuilderBattle2
                        | Screen::BuilderStage2
                ) {
                    self.action(vision, adb, screen, "surrender", now, cancel)
                        .await?;
                }
            }
            Stage::Return => {
                if matches!(screen, Screen::HomeResult | Screen::BuilderResult) {
                    self.action(vision, adb, screen, "return", now, cancel)
                        .await?;
                } else if matches!(screen, Screen::Home | Screen::Builder) {
                    if self.broke_attacks >= 3 {
                        return Err(Error::Gameplay("Search Gold ran out on three consecutive attacks; lower the minimum loot or search limit".into()));
                    }
                    if self.mode == SessionMode::Farming && !self.options.until_full {
                        self.village_index = (self.village_index + 1) % self.options.villages.len();
                    }
                    self.go(Stage::Navigate, now);
                    self.delay(now, 1500);
                }
            }
            Stage::Walls => {
                let mut wall = self.wall.take().ok_or(Error::InputGuard)?;
                let result = wall.tick(vision, adb, frame, self.limit(now), cancel).await;
                self.known_gems = wall.gems;
                self.wall = Some(wall);
                let update = result?;
                if let Some((resource, cost)) = update.upgraded {
                    self.upgrades += 1;
                    self.recovery_count = 0;
                    outcome.wall = Some(WallUpgrade {
                        ordinal: self.upgrades,
                        village: self.village(),
                        completed_at_ms: at,
                        resource,
                        cost,
                    });
                }
                if let Some(reason) = update.finished {
                    match reason {
                        Finish::Limit => {
                            self.go(Stage::Done, now);
                            outcome.completed = true;
                        }
                        Finish::NoWalls => {
                            self.full.push(self.village());
                            outcome.completed = self.next_village(now);
                        }
                        Finish::NeedResources => {
                            if self.upgrades == self.wall_started_upgrades
                                && self.options.wall_farm_threshold > 0
                            {
                                return Err(Error::Gameplay("No affordable wall after farming; check reserves and calibration".into()));
                            }
                            self.go(Stage::Menu, now);
                            self.searches = 0;
                        }
                    }
                }
            }
            Stage::Done => outcome.completed = true,
            Stage::Update { .. } => {}
        }
        Ok(outcome)
    }
}

fn resource_name(resource: Resource) -> &'static str {
    match resource {
        Resource::Gold => "gold",
        Resource::Elixir => "elixir",
        Resource::Dark => "dark",
    }
}
fn deploy_screens(source: Screen) -> Vec<Screen> {
    match source {
        Screen::HomeScout => vec![Screen::HomeScout, Screen::HomeBattle],
        Screen::BuilderScout => vec![
            Screen::BuilderScout,
            Screen::BuilderBattle,
            Screen::BuilderBattle2,
        ],
        _ => vec![Screen::BuilderStage2, Screen::BuilderBattle2],
    }
}
pub(super) fn difference(previous: &Mat, current: &Mat) -> Result<f64> {
    let mut delta = Mat::default();
    core::absdiff(previous, current, &mut delta)?;
    let mean = core::mean(&delta, &core::no_array())?;
    Ok((mean[0] + mean[1] + mean[2]) / 3.0)
}
fn changed(previous: &Mat, current: &Mat) -> Result<bool> {
    let mut delta = Mat::default();
    core::absdiff(previous, current, &mut delta)?;
    let mut count = 0;
    for y in 0..delta.rows() {
        for x in 0..delta.cols() {
            if delta.at_2d::<core::Vec3b>(y, x)?.iter().any(|v| *v > 20) {
                count += 1;
            }
        }
    }
    Ok(f64::from(count) / f64::from(delta.rows() * delta.cols()) > 0.02)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calibration::Calibration,
        gameplay::{
            FrameView, GameVision,
            test_device::{Fake, root},
        },
        vision::Detector,
    };
    use std::{path::Path, sync::Arc};

    #[tokio::test]
    async fn result_animation_settles_before_recording_or_returning() {
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
        let (fake, zero) = Fake::new("english-numeric-builder-zero.png");
        let (_, two) = Fake::new("english-numeric-builder-two-stars.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        for changing in [false, true] {
            let mut engine = Engine::new(
                Options {
                    villages: vec![Village::Builder],
                    ..Options::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.stage = Stage::Battle;
            engine.pending_attack = true;
            let frames = if changing {
                (0..=20)
                    .map(|i| (1000 + i * 500, if i % 2 == 0 { &zero } else { &two }, true))
                    .collect::<Vec<_>>()
            } else {
                vec![
                    (1000, &zero, true),
                    (2000, &zero, true),
                    (3999, &zero, true),
                    (4000, &zero, true),
                    (4500, &two, true),
                    (5000, &two, false),
                    (5500, &two, true),
                    (6000, &two, true),
                ]
            };
            let last = frames.last().unwrap().0;
            for (now, image, known) in frames {
                let detection = if known {
                    Detection::Known(Screen::BuilderResult)
                } else {
                    Detection::Unknown
                };
                let outcome = engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image,
                            detection: &detection,
                            elapsed_ms: now,
                            captured_at_ms: now as i64,
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                assert!(fake.taps.lock().unwrap().is_empty());
                if now < last {
                    assert!(outcome.attack.is_none());
                } else {
                    let attack = outcome.attack.unwrap();
                    assert_eq!(attack.loot.gold, if changing { None } else { Some(75000) });
                    assert_eq!(attack.damage, if changing { None } else { Some(72) });
                    assert_eq!(attack.stars, if changing { None } else { Some(2) });
                    assert!(matches!(engine.stage, Stage::Return));
                    assert_eq!(engine.attacks, 1);
                }
            }
        }
    }

    #[tokio::test]
    async fn cart_goal_requires_stable_readings_and_stops_after_missing_visits() {
        let mut profile = Profile::load(&root()).unwrap();
        let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
        let detector = Arc::new(
            Detector::load(
                &Calibration::load(&root().join("config.json")).unwrap(),
                &root(),
            )
            .unwrap(),
        );
        let options = Options {
            villages: vec![Village::Builder],
            targets: vec![],
            until_full: true,
            cart_goal: true,
            ..Options::default()
        };
        let mut engine = Engine::new(options.clone(), SessionMode::Farming, &profile).unwrap();
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector: detector.clone(),
        };
        let (fake, image) = Fake::new("english-builder_cart_empty.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        engine.stage = Stage::Cart(CartPhase::Claim);
        for now in [3000, 3500, 4000] {
            let frame = FrameView {
                image: &image,
                detection: &Detection::Known(Screen::BuilderCart),
                elapsed_ms: now,
                captured_at_ms: 1_700_000_000_000,
            };
            engine
                .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                .await
                .unwrap();
            if now < 4000 {
                assert!(matches!(engine.stage, Stage::Cart(CartPhase::Read { .. })));
            }
        }
        assert!(matches!(engine.stage, Stage::Cart(CartPhase::Close)));
        assert!(!engine.cart_full);
        assert_eq!(fake.taps.lock().unwrap().len(), 1);
        assert!(!engine.can_restart_between_attacks());
        let (fake, home) = Fake::new("english-builder.png");
        let mut adb = fake.adb(temp.path());
        let frame = FrameView {
            image: &home,
            detection: &Detection::Known(Screen::Builder),
            elapsed_ms: 5000,
            captured_at_ms: 1_700_000_000_000,
        };
        engine.stage = Stage::Check {
            index: 0,
            opened: false,
        };
        assert!(
            !engine
                .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                .await
                .unwrap()
                .completed
        );
        engine.stage = Stage::Check {
            index: 0,
            opened: false,
        };
        engine.cart_full = true;
        assert!(
            engine
                .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                .await
                .unwrap()
                .completed
        );
        // Composite a visible calibrated cart over the redacted village; taps leave it unchanged.
        let mut visible = home.try_clone().unwrap();
        let icon = crate::vision::read_image(
            &root().join(&profile.screen(Screen::Builder).unwrap().targets["cart"].file),
        )
        .unwrap();
        icon.copy_to(
            &mut Mat::roi_mut(
                &mut visible,
                core::Rect::new(600, 350, icon.cols(), icon.rows()),
            )
            .unwrap(),
        )
        .unwrap();
        let stuck = Fake::from_image(&visible);
        let mut stuck_adb = stuck.adb(temp.path());
        profile
            .screens
            .get_mut(&Screen::Builder)
            .unwrap()
            .targets
            .get_mut("cart")
            .unwrap()
            .region = Some(crate::calibration::Region(
            600,
            350,
            icon.cols() as u32,
            icon.rows() as u32,
        ));
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector: detector.clone(),
        };
        assert!(
            profile.screen(Screen::Builder).unwrap().targets["cart"]
                .locate(&visible)
                .unwrap()
                .is_some()
        );
        let mut failed_open = Engine::new(options.clone(), SessionMode::Farming, &profile).unwrap();
        for visit in 1..=5 {
            let now = visit * 10_000;
            failed_open.go(Stage::Cart(CartPhase::Find), now);
            failed_open
                .tick(
                    &vision,
                    &mut stuck_adb,
                    &FrameView {
                        image: &visible,
                        elapsed_ms: now,
                        ..frame
                    },
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(matches!(failed_open.stage, Stage::Cart(CartPhase::Claim)));
            let result = failed_open
                .tick(
                    &vision,
                    &mut stuck_adb,
                    &FrameView {
                        image: &visible,
                        elapsed_ms: now + 6000,
                        ..frame
                    },
                    &CancellationToken::new(),
                )
                .await;
            if visit < 5 {
                assert!(result.is_ok());
            } else {
                assert!(
                    matches!(result, Err(Error::Gameplay(message)) if message.contains("five visits"))
                );
            }
        }
        assert_eq!(stuck.taps.lock().unwrap().len(), 5);
        profile
            .screens
            .get_mut(&Screen::Builder)
            .unwrap()
            .targets
            .remove("cart");
        let mut engine = Engine::new(options, SessionMode::Farming, &profile).unwrap();
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector,
        };
        for visit in 1..=5 {
            engine.go(Stage::Cart(CartPhase::Find), 0);
            let result = engine
                .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                .await;
            if visit < 5 {
                assert!(result.is_ok());
            } else {
                assert!(
                    matches!(result, Err(Error::Gameplay(message)) if message.contains("five visits"))
                );
            }
        }
        assert!(fake.taps.lock().unwrap().is_empty());
        engine.stage = Stage::Navigate;
        assert!(engine.can_restart_between_attacks());
        engine.wall = Some(WallRun::pending_for_test());
        assert!(!engine.can_restart_between_attacks());
    }

    #[tokio::test]
    async fn pending_wall_spending_blocks_lifecycle_popups_and_recovery() {
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
        let (fake, image) = Fake::new("english-home.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(Options::default(), SessionMode::Walls, &profile).unwrap();
        engine.stage = Stage::Walls;
        engine.wall = Some(WallRun::pending_for_test());
        for screen in [Screen::UpdateOffer, Screen::BuildersBusy] {
            let frame = FrameView {
                image: &image,
                detection: &Detection::Known(screen),
                elapsed_ms: 1000,
                captured_at_ms: 1_700_000_000_000,
            };
            assert!(matches!(
                engine
                    .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                    .await,
                Err(Error::Gameplay(_))
            ));
            assert!(matches!(engine.stage, Stage::Walls));
        }
        assert!(matches!(engine.recover(1000), Err(Error::Gameplay(_))));
        assert!(fake.taps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fast_cart_popup_waits_for_claim_and_stuck_close_never_reclaims_or_overflows() {
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
        let (fake, image) = Fake::new("english-builder_cart.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(
            Options {
                villages: vec![Village::Builder],
                ..Options::default()
            },
            SessionMode::Farming,
            &profile,
        )
        .unwrap();
        engine.stage = Stage::Cart(CartPhase::Claim);
        engine.last_action = 1000;
        let cancel = CancellationToken::new();
        let mut frame = FrameView {
            image: &image,
            detection: &Detection::Known(Screen::BuilderCart),
            elapsed_ms: 1500,
            captured_at_ms: 1_700_000_000_000,
        };
        engine
            .tick(&vision, &mut adb, &frame, &cancel)
            .await
            .unwrap();
        assert!(fake.taps.lock().unwrap().is_empty());
        assert!(matches!(engine.stage, Stage::Cart(CartPhase::Claim)));
        for tick in 0..260 {
            frame.elapsed_ms = 3000 + tick * 500;
            engine
                .tick(&vision, &mut adb, &frame, &cancel)
                .await
                .unwrap();
        }
        let taps = fake.taps.lock().unwrap();
        assert_eq!(
            taps.iter().filter(|tap| tap.ends_with("941 606")).count(),
            1
        );
        assert!(taps.iter().filter(|tap| tap.ends_with("1072 71")).count() >= 60);
    }
}
