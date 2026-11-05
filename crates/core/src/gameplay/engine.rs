use super::{
    Options, Strategy,
    profile::Profile,
    wall_run::{Finish, WallRun},
};
use crate::{
    Error, Result,
    calibration::Screen,
    device::{Adb, Gesture, process::Transport},
    telemetry::{
        Attack, Balance, Loot, Reading, Resource, ResourceObservation, SessionMode, Village,
        WallUpgrade,
    },
    vision::{
        Detection, Digits,
        army::{self, Deployment},
        crop,
    },
};
use opencv::{
    core::{self, Mat},
    prelude::*,
};
use serde::Serialize;
use std::{
    cell::Cell,
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
    pub army: Option<(Screen, Vec<army::recipe::Unit>)>,
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
    },
    Normalize {
        screen: Screen,
        routine: super::camera::Normalization,
    },
    ReadArmy {
        screen: Screen,
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
    heroes: VecDeque<usize>,
    automatic: bool,
    repeat_ability: bool,
    deployment_source: Option<Screen>,
    builder_complete: Cell<bool>,
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
    daily_actions: u8,
    top_up_closes: u8,
    unconfirmed_input: bool,
    pending_drops: u32,
    ability_since: Option<u64>,
    camera: Option<super::camera::Camera>,
    scanner: Option<army::roster::Scanner>,
    roster: Option<army::roster::Roster>,
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
            heroes: VecDeque::new(),
            automatic: false,
            repeat_ability: false,
            deployment_source: None,
            builder_complete: Cell::new(false),
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
            daily_actions: 0,
            top_up_closes: 0,
            unconfirmed_input: false,
            pending_drops: 0,
            ability_since: None,
            camera: None,
            scanner: None,
            roster: None,
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
            Stage::Plan { .. } | Stage::ReadArmy { .. } => "Reading army",
            Stage::Normalize { .. } => "Positioning camera",
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
        if matches!(stage, Stage::Plan { .. }) {
            self.camera = None;
            self.scanner = None;
            self.roster = None;
            self.pending_drops = 0;
            self.repeat_ability = false;
        }
        self.stage = stage;
        self.since = now;
        self.not_before = now;
        self.last_action = 0;
        self.ability_since = None;
        self.pans = 0;
    }
    fn delay(&mut self, now: u64, millis: u64) {
        self.not_before = now.saturating_add(millis);
    }
    pub fn recovery_safe(&self) -> bool {
        !self.unconfirmed_input
    }

    pub fn recover(&mut self, now: u64) -> Result<u8> {
        if !self.recovery_safe() {
            return Err(Error::Gameplay(
                "An input has not been confirmed; stopped without replaying the deployment".into(),
            ));
        }
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
            |image| {
                if screen == Screen::BuilderBattle
                    && !builder_stage_ready(
                        profile,
                        vision.digits,
                        self.deployment_source,
                        image,
                        &self.builder_complete,
                    )?
                {
                    return Err(Error::InputGuard);
                }
                Ok((profile.action(screen, name, image)?, ()))
            },
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
            && self.battle_started != 0
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
        if matches!(screen, Screen::BuilderBattle) && self.options.surrender_stars > 0 {
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
        self.unconfirmed_input = false;
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
                    displayed_loot: Loot::default(),
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
        if matches!(
            detection,
            Detection::Known(Screen::HomeBattle | Screen::BuilderBattle)
        ) && self.pending_attack
            && self.battle_started == 0
        {
            self.battle_started = now.max(1);
        }
        let in_combat = matches!(
            self.stage,
            Stage::Deploy { .. }
                | Stage::Battle
                | Stage::Surrender
                | Stage::Plan { .. }
                | Stage::ReadArmy { .. }
                | Stage::Normalize { .. }
        );
        if in_combat && matches!(detection, Detection::Known(Screen::BuilderStage2)) {
            if self.deployment_source != Some(Screen::BuilderStage2) {
                self.builder_complete.set(false);
            }
            self.deployment_source = Some(Screen::BuilderStage2);
        }
        let source = match self.stage {
            Stage::Plan { screen, .. }
            | Stage::Normalize { screen, .. }
            | Stage::ReadArmy { screen } => Some(screen),
            Stage::Deploy { source, .. } => Some(source),
            Stage::Battle | Stage::Surrender => self.deployment_source,
            _ => None,
        };
        let builder_ready =
            if in_combat && matches!(detection, Detection::Known(Screen::BuilderBattle)) {
                Some(builder_stage_ready(
                    profile,
                    digits,
                    source,
                    image,
                    &self.builder_complete,
                )?)
            } else {
                None
            };
        let next_builder_stage = in_combat
            && matches!(detection, Detection::Known(Screen::BuilderStage2))
            && !matches!(
                self.stage,
                Stage::Plan {
                    screen: Screen::BuilderStage2,
                    ..
                } | Stage::Normalize {
                    screen: Screen::BuilderStage2,
                    ..
                } | Stage::ReadArmy {
                    screen: Screen::BuilderStage2
                } | Stage::Deploy {
                    source: Screen::BuilderStage2,
                    ..
                } | Stage::Surrender
            );
        let finished = (in_combat
            && matches!(
                detection,
                Detection::Known(
                    Screen::HomeResult | Screen::BuilderResult | Screen::Home | Screen::Builder
                )
            ))
            || next_builder_stage;
        if !finished
            && self.camera.is_some()
            && matches!(self.stage, Stage::Deploy { .. })
            && now.saturating_sub(self.since) >= 180_000
        {
            return Err(Error::Gameplay(
                "Deployment timed out; no further inputs sent".into(),
            ));
        }
        if !finished && now < self.not_before {
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
        if !finished
            && !matches!(self.stage, Stage::Walls)
            && now.saturating_sub(self.since) > 240_000
        {
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
        if matches!(
            screen,
            Screen::UpdateOffer
                | Screen::BuildersBusy
                | Screen::DailyReward
                | Screen::LdplayerTopUp
        ) && self
            .wall
            .as_ref()
            .is_some_and(WallRun::verification_pending)
        {
            return Err(Error::Gameplay(
                "Wall spending is unverified; stopped before changing operations".into(),
            ));
        }
        if screen == Screen::LdplayerTopUp {
            if self.unconfirmed_input {
                return Err(Error::Gameplay(
                    "An input has not been confirmed; stopped before closing the LDPlayer offer"
                        .into(),
                ));
            }
            if self.top_up_closes >= 3 {
                return Err(Error::Gameplay(
                    "LDPlayer offer did not close after three attempts".into(),
                ));
            }
            if self
                .action(vision, adb, screen, "close", now, cancel)
                .await?
            {
                self.top_up_closes += 1;
                self.delay(now, 1500);
            }
            return Ok(outcome);
        }
        self.top_up_closes = 0;
        if screen == Screen::DailyReward {
            if self.daily_actions >= 3 {
                return Err(Error::Gameplay(
                    "Daily Reward did not clear after three actions".into(),
                ));
            }
            let action = if profile
                .screen(screen)?
                .targets
                .get("claim")
                .map(|target| target.locate(image))
                .transpose()?
                .flatten()
                .is_some()
            {
                "claim"
            } else {
                "close"
            };
            if self
                .action(vision, adb, screen, action, now, cancel)
                .await?
            {
                self.daily_actions += 1;
                self.delay(now, 1500);
            }
            return Ok(outcome);
        }
        self.daily_actions = 0;
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
        if next_builder_stage {
            self.unconfirmed_input = false;
            if self.options.surrender_stars > 0 && !self.climbing {
                self.go(Stage::Surrender, now);
            } else {
                self.go(Stage::Plan { screen }, now);
                self.delay(now, 1500);
            }
            return Ok(outcome);
        }
        if !self.unconfirmed_input && builder_ready == Some(false) {
            return Ok(outcome);
        }
        if matches!(
            self.stage,
            Stage::Deploy { .. } | Stage::Battle | Stage::Normalize { .. } | Stage::ReadArmy { .. }
        ) && matches!(screen, Screen::HomeBattle | Screen::BuilderBattle)
            && !self.unconfirmed_input
            && self.surrender_reached(profile, digits, image, screen, now)
        {
            self.go(Stage::Surrender, now);
            return Ok(outcome);
        }
        let input_detector = detector.clone();
        let completed = &self.builder_complete;
        let guard = |image: &Mat| -> Result<()> {
            if input_detector.detect(image)? == Detection::Known(Screen::BuilderBattle)
                && !builder_stage_ready(profile, digits, source, image, completed)?
            {
                return Err(Error::InputGuard);
            }
            Ok(())
        };
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
                Screen::HomeBattle | Screen::BuilderBattle => {}
                Screen::HomeScout | Screen::BuilderScout | Screen::BuilderStage2 => {
                    let village = if screen == Screen::HomeScout {
                        Village::Home
                    } else {
                        Village::Builder
                    };
                    if let Some(index) = self.options.villages.iter().position(|v| *v == village) {
                        self.village_index = index;
                        self.go(Stage::Plan { screen }, now);
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
                        if let Some(point) = target.locate(image)? {
                            if self.last_action == 0 || now.saturating_sub(self.last_action) >= 2000
                            {
                                adb.tap_checked(
                                    detector,
                                    &[screen],
                                    |fresh| {
                                        Ok((
                                            target
                                                .locate_near(fresh, point)?
                                                .ok_or(Error::Unreadable)?,
                                            (),
                                        ))
                                    },
                                    cancel,
                                )
                                .await?;
                                self.go(Stage::Cart(CartPhase::Claim), now);
                                self.delay(now, 500);
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
                let targets = if self.mode == SessionMode::Walls {
                    vec![Resource::Gold, Resource::Elixir]
                } else {
                    self.options.targets.clone()
                };
                let targets: Vec<_> = targets
                    .into_iter()
                    .filter(|r| self.village() == Village::Home || *r != Resource::Dark)
                    .collect();
                let resources = if self.village() == Village::Home {
                    &[Resource::Gold, Resource::Elixir, Resource::Dark][..]
                } else {
                    &[Resource::Gold, Resource::Elixir][..]
                };
                if index < resources.len() {
                    let resource = resources[index];
                    let required = (self.options.until_full || self.mode == SessionMode::Walls)
                        && targets.contains(&resource);
                    let read_timeout = if required { 15_000 } else { 3000 };
                    let key = resource_name(resource);
                    let spec = profile
                        .capacity
                        .get(self.name())
                        .and_then(|r| r.get(key))
                        .ok_or_else(|| {
                            Error::Calibration(format!("Missing {key} capacity calibration"))
                        })?;
                    let amount = digits
                        .trailing(&crop(image, spec.bar)?, 0.65, 110)
                        .ok()
                        .and_then(|value| u32::try_from(value).ok());
                    let Some(amount) = amount else {
                        if now.saturating_sub(self.since) <= read_timeout {
                            return Ok(outcome);
                        }
                        if required {
                            return Err(Error::Unreadable);
                        }
                        if opened && spec.label.locate(image)?.is_some() {
                            adb.tap(detector, &[screen], spec.tap, cancel).await?;
                            self.delay(now, 1000);
                        }
                        self.stage = Stage::Check {
                            index: index + 1,
                            opened: false,
                        };
                        self.since = now;
                        return Ok(outcome);
                    };
                    let cache_key = (self.name().to_owned(), key.to_owned());
                    if self.capacities.get(&cache_key).is_some_and(|v| amount > *v) {
                        self.capacities.remove(&cache_key);
                    }
                    let maximum = if spec.label.locate(image)?.is_some() {
                        let value = digits
                            .number(&crop(image, spec.maximum)?, 0.65)
                            .ok()
                            .and_then(|value| u32::try_from(value).ok())
                            .filter(|value| *value > 0 && *value >= amount);
                        if let Some(value) = value {
                            self.capacities.insert(cache_key, value);
                        }
                        value
                    } else {
                        self.capacities.get(&cache_key).copied()
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
                    } else if now.saturating_sub(self.since) > read_timeout {
                        if required {
                            return Err(Error::Unreadable);
                        }
                        if spec.label.locate(image)?.is_some() {
                            adb.tap(detector, &[screen], spec.tap, cancel).await?;
                            self.delay(now, 1000);
                        }
                        self.stage = Stage::Check {
                            index: index + 1,
                            opened: false,
                        };
                        self.since = now;
                    }
                } else {
                    let cart_goal = self.mode == SessionMode::Farming
                        && self.options.until_full
                        && self.village() == Village::Builder
                        && self.options.cart_goal;
                    let full = (!targets.is_empty() || cart_goal)
                        && (!cart_goal || self.cart_full)
                        && targets.iter().all(|r| {
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
                    self.go(Stage::Plan { screen }, now);
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
                    self.go(Stage::Plan { screen }, now);
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
            Stage::Normalize {
                screen: source,
                mut routine,
            } => {
                if now.saturating_sub(self.since) > 90_000 {
                    return Err(Error::Gameplay(
                        "Camera positioning timed out; no deployment input sent".into(),
                    ));
                }
                let allowed = deploy_screens(source);
                if !allowed.contains(&screen) {
                    return Ok(outcome);
                }
                let adjustment = routine.next(image)?;
                if !matches!(adjustment, super::camera::Adjustment::Ready) {
                    let delay = if let super::camera::Adjustment::Move(gesture) = adjustment {
                        // Pans still need two stationary captures; pinches keep their zoom delay.
                        let delay = if matches!(&gesture, Gesture::Pan { .. }) {
                            250
                        } else {
                            700
                        };
                        adb.gesture_checked(
                            detector.clone(),
                            &allowed,
                            |frame| {
                                guard(frame)?;
                                Ok((gesture, ()))
                            },
                            cancel,
                        )
                        .await?;
                        delay
                    } else {
                        250
                    };
                    self.stage = Stage::Normalize {
                        screen: source,
                        routine,
                    };
                    self.delay(now, delay);
                } else {
                    let geometry = profile.screen(source)?.camera.clone().ok_or_else(|| {
                        Error::Calibration("Camera geometry is missing for this village".into())
                    })?;
                    self.camera = Some(super::camera::Camera::new(
                        routine.geometry(&geometry)?,
                        image,
                    )?);
                    self.go(Stage::ReadArmy { screen: source }, now);
                }
            }
            Stage::ReadArmy { screen: source } => {
                use army::roster::ScanStep;
                let allowed = deploy_screens(source);
                if !allowed.contains(&screen) {
                    return Ok(outcome);
                }
                if now.saturating_sub(self.since) > 90_000 {
                    return Err(Error::Timeout);
                }
                match self
                    .scanner
                    .as_mut()
                    .ok_or(Error::InputGuard)?
                    .observe(image, digits)?
                {
                    ScanStep::Wait => self.delay(now, 250),
                    ScanStep::Pan(pan) => {
                        let result = adb
                            .gesture_checked(
                                detector.clone(),
                                &allowed,
                                |frame| {
                                    guard(frame)?;
                                    Ok((
                                        Gesture::Drag {
                                            from: pan.from,
                                            to: pan.to,
                                        },
                                        (),
                                    ))
                                },
                                cancel,
                            )
                            .await;
                        if matches!(result, Err(Error::InputGuard)) {
                            self.scanner
                                .as_mut()
                                .ok_or(Error::InputGuard)?
                                .cancel_pending_scroll()?;
                        }
                        result?;
                        self.delay(now, 350);
                    }
                    ScanStep::Complete(roster) => {
                        let inventory = roster.inventory();
                        let generated;
                        let recipe = if let Some(recipe) = self
                            .options
                            .layouts
                            .get(&source)
                            .and_then(|layout| layout.recipe.as_ref())
                        {
                            recipe
                        } else {
                            generated = army::automatic::generate(
                                &inventory,
                                self.options.strategy == Strategy::Ring,
                                source != Screen::HomeScout,
                                self.options.builder_stack,
                                self.options.earthquake,
                            )?;
                            &generated
                        };
                        let steps = recipe.compile_roster(&roster)?.into();
                        self.heroes = if self.automatic && source != Screen::HomeScout {
                            inventory
                                .iter()
                                .filter(|u| u.kind == army::recipe::UnitKind::Hero)
                                .map(|u| u.slot)
                                .collect()
                        } else {
                            VecDeque::new()
                        };
                        outcome.army = Some((source, inventory));
                        self.roster = Some(roster);
                        self.scanner = None;
                        self.hero_ready_at = now + 3000;
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
                }
            }
            Stage::Plan { screen: source, .. } => {
                let allowed = deploy_screens(source);
                if !allowed.contains(&screen) {
                    return Ok(outcome);
                }
                let spec = profile.screen(source)?;
                let bar = spec
                    .army_bar
                    .ok_or_else(|| Error::Calibration("Army bar is not calibrated".into()))?;
                self.scanner = Some(army::roster::Scanner::new(
                    bar,
                    source != Screen::HomeScout,
                )?);
                self.automatic = self
                    .options
                    .layouts
                    .get(&source)
                    .and_then(|layout| layout.recipe.as_ref())
                    .is_none();
                self.deployment_source = Some(source);
                self.heroes.clear();
                if source != Screen::BuilderStage2 {
                    self.builder_complete.set(false);
                    self.battle_started = if screen == source { 0 } else { now.max(1) };
                    self.previous_damage = None;
                    self.previous_stars = None;
                }
                self.pending_attack = true;
                self.go(
                    Stage::Normalize {
                        screen: source,
                        routine: Default::default(),
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
                if !waited
                    && let Some(
                        Deployment::PreciseDrop { card, .. } | Deployment::PreciseAbility { card },
                    ) = steps.front_mut()
                    && card.logical_slot > 0
                {
                    match self.roster.as_mut().ok_or(Error::InputGuard)?.locate(
                        card.logical_slot,
                        image,
                        digits,
                    )? {
                        army::roster::Locate::Pan(pan) => {
                            let result = adb
                                .gesture_checked(
                                    detector.clone(),
                                    &allowed,
                                    |frame| {
                                        guard(frame)?;
                                        Ok((
                                            Gesture::Drag {
                                                from: pan.from,
                                                to: pan.to,
                                            },
                                            (),
                                        ))
                                    },
                                    cancel,
                                )
                                .await;
                            if matches!(result, Err(Error::InputGuard)) {
                                self.roster
                                    .as_mut()
                                    .ok_or(Error::InputGuard)?
                                    .cancel_pending_scroll()?;
                            }
                            result?;
                            self.stage = Stage::Deploy {
                                source,
                                steps,
                                selected: false,
                                sent,
                                waited,
                            };
                            self.delay(now, 350);
                            return Ok(outcome);
                        }
                        army::roster::Locate::Wait => {
                            self.stage = Stage::Deploy {
                                source,
                                steps,
                                selected: false,
                                sent,
                                waited,
                            };
                            self.delay(now, 250);
                            return Ok(outcome);
                        }
                        army::roster::Locate::Ready(bound) => {
                            *card = bound;
                        }
                    }
                }
                let Some(step) = steps.front().cloned() else {
                    self.repeat_ability = false;
                    self.go(Stage::Battle, now);
                    return Ok(outcome);
                };
                let mut done = false;
                match step {
                    Deployment::Wait(ms) => {
                        if !waited {
                            waited = true;
                            self.delay(now, u64::from(ms));
                        } else {
                            done = true;
                        }
                    }
                    Deployment::PreciseDrop {
                        card,
                        point,
                        count,
                        interval_ms,
                        initial_count,
                    } => {
                        let slot = card.card.slot;
                        if waited {
                            // Siege cards change artwork after deployment; accept the new health frame
                            // only as the response to this already identity-checked, pending input.
                            let left = if self.unconfirmed_input
                                && !card.builder
                                && card.card.kind == army::Kind::Siege
                                && initial_count == 1
                                && sent == 0
                                && army::hero_deployed(image, slot)?
                            {
                                Some(0)
                            } else {
                                card.remaining(image, digits)?
                            };
                            let pending = self.pending_drops.max(1);
                            if left == Some(initial_count - sent - pending) {
                                self.unconfirmed_input = false;
                                sent += pending;
                                self.pending_drops = 0;
                                waited = false;
                                done = sent >= count;
                                self.not_before = now
                                    .max(self.last_action.saturating_add(u64::from(interval_ms)));
                            } else if left.is_some_and(|n| n < initial_count - sent - pending)
                                || now.saturating_sub(self.last_action) > 4000
                            {
                                return Err(Error::Gameplay("Deployment could not be confirmed; stopped without repeating the drop".into()));
                            }
                        } else {
                            let camera = self.camera.as_mut().ok_or(Error::InputGuard)?;
                            let location = camera.track_position(image, point)?;
                            if super::camera::visible(location).is_none() {
                                if self.pans >= 6 {
                                    return Err(Error::Gameplay(
                                        "Deployment position could not be brought into view".into(),
                                    ));
                                }
                                adb.gesture_checked(
                                    detector.clone(),
                                    &allowed,
                                    |frame| {
                                        guard(frame)?;
                                        Ok((super::camera::pan_toward(location), ()))
                                    },
                                    cancel,
                                )
                                .await?;
                                self.pans += 1;
                                self.delay(now, 600);
                            } else if !selected {
                                adb.tap_checked(
                                    detector.clone(),
                                    &allowed,
                                    |frame| {
                                        guard(frame)?;
                                        Ok((slot, ()))
                                    },
                                    cancel,
                                )
                                .await?;
                                selected = true;
                                self.delay(now, 250);
                            } else {
                                self.unconfirmed_input = true;
                                let batch = if !card.builder
                                    && card.card.kind == army::Kind::Counted
                                    && card.card.group == 0
                                {
                                    (1 + 470 / interval_ms).min(8).min(count - sent)
                                } else {
                                    1
                                };
                                self.pending_drops = batch;
                                let resolve = |frame: &Mat| {
                                    guard(frame)?;
                                    if !army::selected(frame, slot)? {
                                        return Err(Error::Gameplay("The requested army card was not selected; no unit deployed".into()));
                                    }
                                    if card.remaining(frame, digits)? != Some(initial_count - sent)
                                    {
                                        return Err(Error::Gameplay(
                                            "Army count changed before deployment".into(),
                                        ));
                                    }
                                    let target = camera.position(frame, point)?;
                                    super::camera::visible(target).ok_or(Error::InputGuard)
                                };
                                let started = std::time::Instant::now();
                                let result = if batch == 1 {
                                    adb.tap_checked(
                                        detector,
                                        &allowed,
                                        |frame| Ok((resolve(frame)?, ())),
                                        cancel,
                                    )
                                    .await
                                } else {
                                    adb.gesture_checked(
                                        detector,
                                        &allowed,
                                        |frame| {
                                            Ok((
                                                Gesture::TimedTaps {
                                                    points: vec![resolve(frame)?; batch as usize],
                                                    spacing: Duration::from_millis(u64::from(
                                                        interval_ms,
                                                    )),
                                                },
                                                (),
                                            ))
                                        },
                                        cancel,
                                    )
                                    .await
                                };
                                if matches!(result, Err(Error::InputGuard)) {
                                    self.unconfirmed_input = false;
                                    self.pending_drops = 0;
                                }
                                result?;
                                waited = true;
                                self.last_action =
                                    now.saturating_add(started.elapsed().as_millis() as u64);
                                self.delay(self.last_action, 150);
                            }
                        }
                    }
                    Deployment::PreciseAbility { card } => {
                        let mut ready = card.ability_ready(image, digits)?;
                        if ready == Some(true) && self.repeat_ability {
                            ready = Some(army::hero_charge(image, card.card.slot)? == 3);
                        }
                        if waited {
                            if ready == Some(false) {
                                done = true;
                                self.unconfirmed_input = false;
                            } else if now.saturating_sub(self.last_action) > 4000 {
                                return Err(Error::Gameplay("Ability activation could not be confirmed; stopped without repeating it".into()));
                            }
                        } else if ready == Some(true) {
                            self.unconfirmed_input = true;
                            let result = adb
                                .tap_checked(
                                    detector,
                                    &allowed,
                                    |frame| {
                                        guard(frame)?;
                                        if card.ability_ready(frame, digits)? != Some(true) {
                                            return Err(Error::InputGuard);
                                        }
                                        if self.repeat_ability
                                            && army::hero_charge(frame, card.card.slot)? != 3
                                        {
                                            return Err(Error::InputGuard);
                                        }
                                        Ok((card.card.slot, ()))
                                    },
                                    cancel,
                                )
                                .await;
                            if matches!(result, Err(Error::InputGuard)) {
                                self.unconfirmed_input = false;
                            }
                            result?;
                            waited = true;
                            self.last_action = now;
                            self.delay(now, 500);
                        } else if ready == Some(false)
                            && (self.automatic
                                || !card.builder
                                || army::empty(image, card.card.slot)?)
                        {
                            done = true;
                        } else {
                            let started = *self.ability_since.get_or_insert(now);
                            if now.saturating_sub(started) >= 30_000 {
                                return Err(Error::Gameplay(if ready.is_none() {
                                    "The planned ability could not be read; no activation input sent"
                                } else {
                                    "The planned ability did not become ready within 30 seconds"
                                }.into()));
                            }
                            self.delay(now, 250);
                        }
                    }
                }
                if done {
                    if self.repeat_ability {
                        self.hero_ready_at = now + 3000;
                    }
                    steps.pop_front();
                    selected = false;
                    sent = 0;
                    waited = false;
                    self.pans = 0;
                    self.ability_since = None;
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
                if matches!(screen, Screen::HomeBattle | Screen::BuilderBattle) {
                    if !self.speed_checked {
                        self.speed_checked = true;
                        if let Some(target) = &profile.speed_button
                            && target.locate(image)?.is_some()
                        {
                            adb.tap_checked(
                                detector,
                                &[screen],
                                |frame| {
                                    guard(frame)?;
                                    Ok((target.locate(frame)?.ok_or(Error::Unreadable)?, ()))
                                },
                                cancel,
                            )
                            .await?;
                            return Ok(outcome);
                        }
                    }
                    if now >= self.hero_ready_at
                        && let Some(&slot) = self.heroes.front()
                    {
                        let source = self.deployment_source.ok_or(Error::InputGuard)?;
                        if !deploy_screens(source).contains(&screen) {
                            return Ok(outcome);
                        }
                        match self
                            .roster
                            .as_mut()
                            .ok_or(Error::InputGuard)?
                            .locate(slot, image, digits)?
                        {
                            army::roster::Locate::Pan(pan) => {
                                let result = adb
                                    .gesture_checked(
                                        detector.clone(),
                                        &[screen],
                                        |frame| {
                                            guard(frame)?;
                                            Ok((
                                                Gesture::Drag {
                                                    from: pan.from,
                                                    to: pan.to,
                                                },
                                                (),
                                            ))
                                        },
                                        cancel,
                                    )
                                    .await;
                                if matches!(result, Err(Error::InputGuard)) {
                                    self.roster
                                        .as_mut()
                                        .ok_or(Error::InputGuard)?
                                        .cancel_pending_scroll()?;
                                }
                                result?;
                                self.delay(now, 350);
                            }
                            army::roster::Locate::Wait => self.delay(now, 250),
                            army::roster::Locate::Ready(card) => {
                                self.heroes.rotate_left(1);
                                self.hero_ready_at = now + 1000;
                                if card.ability_ready(image, digits)? == Some(true)
                                    && army::hero_charge(image, card.card.slot)? == 3
                                {
                                    self.repeat_ability = true;
                                    self.go(
                                        Stage::Deploy {
                                            source,
                                            steps: vec![Deployment::PreciseAbility { card }].into(),
                                            selected: false,
                                            sent: 0,
                                            waited: false,
                                        },
                                        now,
                                    );
                                }
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
                            attack.displayed_loot = reading.displayed_loot;
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
                    Screen::HomeBattle | Screen::BuilderBattle | Screen::BuilderStage2
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
pub(crate) fn builder_stage_ready(
    profile: &Profile,
    digits: &Digits,
    source: Option<Screen>,
    image: &Mat,
    completed: &Cell<bool>,
) -> Result<bool> {
    if completed.get() {
        return Ok(false);
    }
    let Some(region) = profile.screen(Screen::BuilderBattle)?.damage else {
        return Ok(false);
    };
    let Ok(damage) = digits.percent_up_to(&crop(image, region)?, 200) else {
        return Ok(false);
    };
    Ok(builder_damage_ready(source, damage, completed))
}

fn builder_damage_ready(source: Option<Screen>, damage: u8, completed: &Cell<bool>) -> bool {
    let end = match source {
        Some(Screen::BuilderScout) => 100,
        Some(Screen::BuilderStage2) => 200,
        _ => return false,
    };
    if damage >= end {
        completed.set(true);
    }
    !completed.get() && (source != Some(Screen::BuilderStage2) || damage >= 100)
}

pub(crate) fn deploy_screens(source: Screen) -> Vec<Screen> {
    match source {
        Screen::HomeScout => vec![Screen::HomeScout, Screen::HomeBattle],
        Screen::BuilderScout => vec![Screen::BuilderScout, Screen::BuilderBattle],
        Screen::BuilderStage2 => vec![Screen::BuilderStage2, Screen::BuilderBattle],
        _ => vec![],
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
        device::Point,
        gameplay::{
            FrameView, GameVision,
            test_device::{Fake, root},
        },
        vision::Detector,
    };
    use opencv::imgproc;
    use std::{path::Path, sync::Arc};

    #[test]
    fn deployment_sources_share_active_builder_screen_but_not_countdowns() {
        assert_eq!(
            deploy_screens(Screen::HomeScout),
            vec![Screen::HomeScout, Screen::HomeBattle]
        );
        assert_eq!(
            deploy_screens(Screen::BuilderScout),
            vec![Screen::BuilderScout, Screen::BuilderBattle]
        );
        assert_eq!(
            deploy_screens(Screen::BuilderStage2),
            vec![Screen::BuilderStage2, Screen::BuilderBattle]
        );
        assert!(deploy_screens(Screen::Home).is_empty());
    }

    #[tokio::test]
    async fn builder_stage_identity_requires_countdown_and_fresh_damage() {
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
            detector: detector.clone(),
        };
        for (file, first, second) in [
            ("battlefield-builder-first-star.png", true, false),
            ("battlefield-builder-first-complete.png", false, true),
            ("battlefield-builder-second-active.png", false, true),
            ("english-home.png", false, false),
        ] {
            let (_, image) = Fake::new(file);
            assert_eq!(
                builder_stage_ready(
                    &profile,
                    &digits,
                    Some(Screen::BuilderScout),
                    &image,
                    &Cell::new(false)
                )
                .unwrap(),
                first,
                "{file}"
            );
            assert_eq!(
                builder_stage_ready(
                    &profile,
                    &digits,
                    Some(Screen::BuilderStage2),
                    &image,
                    &Cell::new(false)
                )
                .unwrap(),
                second,
                "{file}"
            );
            assert!(
                !builder_stage_ready(&profile, &digits, None, &image, &Cell::new(false)).unwrap()
            );
        }
        // The identical 100% display cannot establish stage two by itself.
        for stage in 0..4 {
            for file in ["battlefield-builder-first-complete.png", "english-home.png"] {
                let (fake, image) = Fake::new(file);
                let temp = tempfile::tempdir().unwrap();
                let mut adb = fake.adb(temp.path());
                let mut engine =
                    Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
                engine.pending_attack = true;
                engine.deployment_source = Some(Screen::BuilderScout);
                engine.stage = match stage {
                    0 => Stage::Normalize {
                        screen: Screen::BuilderScout,
                        routine: Default::default(),
                    },
                    1 => Stage::ReadArmy {
                        screen: Screen::BuilderScout,
                    },
                    2 => Stage::Deploy {
                        source: Screen::BuilderScout,
                        steps: VecDeque::new(),
                        selected: false,
                        sent: 0,
                        waited: false,
                    },
                    _ => Stage::Battle,
                };
                engine.not_before = 1500;
                let (_, lower) = Fake::new("battlefield-builder-first-star.png");
                for now in [1000, 2000] {
                    let observed =
                        if now == 2000 && file == "battlefield-builder-first-complete.png" {
                            &lower
                        } else {
                            &image
                        };
                    engine
                        .tick(
                            &vision,
                            &mut adb,
                            &FrameView {
                                image: observed,
                                detection: &Detection::Known(Screen::BuilderBattle),
                                elapsed_ms: now,
                                captured_at_ms: now as i64,
                            },
                            &CancellationToken::new(),
                        )
                        .await
                        .unwrap();
                }
                assert_eq!(engine.deployment_source, Some(Screen::BuilderScout));
                assert!(
                    engine
                        .tick(
                            &vision,
                            &mut adb,
                            &FrameView {
                                image: &image,
                                detection: &Detection::Known(Screen::BuilderBattle),
                                elapsed_ms: 240001,
                                captured_at_ms: 240001
                            },
                            &CancellationToken::new()
                        )
                        .await
                        .is_err()
                );
                assert!(fake.taps.lock().unwrap().is_empty());
            }
        }
        // A completed first stage appearing between tick and dispatch is refused fresh.
        let (fake, _) = Fake::new("battlefield-builder-first-complete.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        engine.deployment_source = Some(Screen::BuilderScout);
        assert!(matches!(
            engine
                .action(
                    &vision,
                    &mut adb,
                    Screen::BuilderBattle,
                    "surrender",
                    1000,
                    &CancellationToken::new()
                )
                .await,
            Err(Error::InputGuard)
        ));
        assert!(fake.taps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pending_builder_input_survives_a_surrender_limit_until_stage_transition() {
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
        let (_, before) = Fake::new("deployment-builder-before.png");
        let cards = army::cards(
            &before,
            profile
                .screen(Screen::BuilderScout)
                .unwrap()
                .army_bar
                .unwrap(),
            true,
            &digits,
        )
        .unwrap();
        let card = army::recipe::BoundCard::new(&cards[1], &before, true).unwrap();
        let temp = tempfile::tempdir().unwrap();
        for ability in [false, true] {
            let mut engine = Engine::new(
                Options {
                    villages: vec![Village::Builder],
                    surrender_after: 1,
                    ..Options::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.pending_attack = true;
            engine.unconfirmed_input = true;
            engine.battle_started = 1;
            engine.last_action = 500;
            engine.stage = Stage::Deploy {
                source: Screen::BuilderScout,
                steps: vec![if ability {
                    Deployment::PreciseAbility { card: card.clone() }
                } else {
                    Deployment::PreciseDrop {
                        card: card.clone(),
                        point: Point(0, 500),
                        count: 1,
                        interval_ms: 300,
                        initial_count: 1,
                    }
                }]
                .into(),
                selected: true,
                sent: 0,
                waited: true,
            };
            for now in [2000, 2500, 3000] {
                let transition = now == 3000;
                let (fake, image) = Fake::new(if transition {
                    "english-builder_fourth_current.png"
                } else {
                    "battlefield-builder-first-complete.png"
                });
                let mut adb = fake.adb(temp.path());
                engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image: &image,
                            detection: &Detection::Known(if transition {
                                Screen::BuilderStage2
                            } else {
                                Screen::BuilderBattle
                            }),
                            elapsed_ms: now,
                            captured_at_ms: now as i64,
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                assert!(fake.taps.lock().unwrap().is_empty());
                if transition {
                    assert!(matches!(
                        engine.stage,
                        Stage::Plan {
                            screen: Screen::BuilderStage2
                        }
                    ));
                    assert!(engine.recovery_safe());
                } else {
                    assert!(matches!(engine.stage, Stage::Deploy { waited: true, .. }));
                    assert!(!engine.recovery_safe());
                }
            }
        }
    }

    #[tokio::test]
    async fn stage_two_countdown_updates_identity_without_restarting_a_surrender() {
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
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        engine.deployment_source = Some(Screen::BuilderScout);
        engine.stage = Stage::Surrender;
        engine.builder_complete.set(true);
        let temp = tempfile::tempdir().unwrap();
        for (file, screen, now) in [
            (
                "english-builder_fourth_current.png",
                Screen::BuilderStage2,
                1000,
            ),
            (
                "battlefield-builder-second-active.png",
                Screen::BuilderBattle,
                4000,
            ),
        ] {
            let (fake, image) = Fake::new(file);
            let mut adb = fake.adb(temp.path());
            engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &image,
                        detection: &Detection::Known(screen),
                        elapsed_ms: now,
                        captured_at_ms: now as i64,
                    },
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(matches!(engine.stage, Stage::Surrender));
            assert!(!engine.builder_complete.get());
            assert_eq!(engine.deployment_source, Some(Screen::BuilderStage2));
            assert_eq!(fake.taps.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn completed_builder_stages_never_reopen_after_lower_readings() {
        for (source, cases) in [
            (
                Screen::BuilderScout,
                [(99, true), (100, false), (99, false), (0, false)],
            ),
            (
                Screen::BuilderStage2,
                [(100, true), (199, true), (200, false), (199, false)],
            ),
        ] {
            let completed = Cell::new(false);
            for (damage, ready) in cases {
                assert_eq!(
                    builder_damage_ready(Some(source), damage, &completed),
                    ready
                );
            }
        }
        assert!(!builder_damage_ready(
            Some(Screen::BuilderStage2),
            99,
            &Cell::new(false)
        ));
    }

    // Simulate a continuing battle around isolated captured ability cards.
    fn continuing_builder_hud(image: &mut Mat) {
        let (_, hud) = Fake::new("battlefield-builder-first-star.png");
        for rect in [
            core::Rect::new(20, 475, 150, 45),
            core::Rect::new(1060, 495, 210, 70),
        ] {
            Mat::roi(&hud, rect)
                .unwrap()
                .copy_to(&mut Mat::roi_mut(image, rect).unwrap())
                .unwrap();
        }
    }

    #[tokio::test]
    async fn abilities_wait_for_readiness_with_a_deadline_and_no_blind_input() {
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
        let (_, before) = Fake::new("deployment-builder-before.png");
        let bar = profile
            .screen(Screen::BuilderScout)
            .unwrap()
            .army_bar
            .unwrap();
        let cards = army::cards(&before, bar, true, &digits).unwrap();
        let bound = army::recipe::BoundCard::new(&cards[1], &before, true).unwrap();
        for (file, automatic, skipped) in [
            ("deployment-builder-full-health-used.png", false, false),
            ("deployment-builder-full-health-used.png", true, true),
            ("english-home.png", false, false),
            ("english-home.png", true, false),
        ] {
            let (_, mut image) = Fake::new(file);
            continuing_builder_hud(&mut image);
            let fake = Fake::from_image(&image);
            let temp = tempfile::tempdir().unwrap();
            let mut adb = fake.adb(temp.path());
            let mut engine = Engine::new(
                Options {
                    villages: vec![Village::Builder],
                    ..Default::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.automatic = automatic;
            engine.stage = Stage::Deploy {
                source: Screen::BuilderScout,
                steps: vec![Deployment::PreciseAbility {
                    card: bound.clone(),
                }]
                .into(),
                selected: false,
                sent: 0,
                waited: false,
            };
            for now in [1000, 2500, 30000, 31000] {
                let result = engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image: &image,
                            detection: &Detection::Known(Screen::BuilderBattle),
                            elapsed_ms: now,
                            captured_at_ms: now as i64,
                        },
                        &CancellationToken::new(),
                    )
                    .await;
                if skipped {
                    result.unwrap();
                    assert!(
                        matches!(engine.stage, Stage::Deploy { ref steps, .. } if steps.is_empty())
                    );
                    break;
                }
                assert_eq!(
                    result.is_err(),
                    now == 31000,
                    "{file}, {now}, automatic={automatic}"
                );
            }
            assert!(fake.taps.lock().unwrap().is_empty());
            assert!(engine.recovery_safe());
        }
    }

    #[tokio::test]
    async fn terminal_frames_interrupt_all_preparation_and_deployment_deadlines() {
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
        for phase in 0..3 {
            for (screen, file) in [
                (Screen::HomeResult, "english-numeric-home-defeat.png"),
                (
                    Screen::BuilderResult,
                    "english-numeric-builder-two-stars.png",
                ),
                (Screen::BuilderStage2, "english-builder_fourth_current.png"),
            ] {
                let (fake, image) = Fake::new(file);
                let temp = tempfile::tempdir().unwrap();
                let mut adb = fake.adb(temp.path());
                let mut engine =
                    Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
                let source = if screen == Screen::HomeResult {
                    Screen::HomeScout
                } else {
                    Screen::BuilderScout
                };
                engine.pending_attack = true;
                engine.not_before = 400_000;
                engine.stage = match phase {
                    0 => Stage::Normalize {
                        screen: source,
                        routine: Default::default(),
                    },
                    1 => Stage::ReadArmy { screen: source },
                    _ => Stage::Deploy {
                        source,
                        steps: vec![Deployment::Wait(1000)].into(),
                        selected: false,
                        sent: 0,
                        waited: true,
                    },
                };
                engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image: &image,
                            detection: &Detection::Known(screen),
                            elapsed_ms: 300_001,
                            captured_at_ms: 300_001,
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                if screen == Screen::BuilderStage2 {
                    assert!(matches!(
                        engine.stage,
                        Stage::Plan {
                            screen: Screen::BuilderStage2
                        }
                    ));
                } else {
                    assert!(
                        matches!(engine.stage, Stage::Result { screen: found, .. } if found == screen)
                    );
                }
                assert!(fake.taps.lock().unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn battle_clock_and_surrender_interrupt_camera_and_roster_preparation() {
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
        for normalize in [true, false] {
            let (fake, image) = Fake::new("english-home_battle.png");
            let temp = tempfile::tempdir().unwrap();
            let mut adb = fake.adb(temp.path());
            let mut engine = Engine::new(
                Options {
                    surrender_after: 5,
                    ..Default::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.pending_attack = true;
            engine.not_before = 2000;
            engine.stage = if normalize {
                Stage::Normalize {
                    screen: Screen::HomeScout,
                    routine: Default::default(),
                }
            } else {
                Stage::ReadArmy {
                    screen: Screen::HomeScout,
                }
            };
            for now in [1000, 6000] {
                engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image: &image,
                            detection: &Detection::Known(Screen::HomeBattle),
                            elapsed_ms: now,
                            captured_at_ms: now as i64,
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
            }
            assert_eq!(engine.battle_started, 1000);
            assert!(matches!(engine.stage, Stage::Surrender));
            assert!(fake.taps.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn spent_bound_abilities_advance_without_tapping_but_pending_input_stays_separate() {
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
        for builder in [false, true] {
            let source = if builder {
                Screen::BuilderScout
            } else {
                Screen::HomeScout
            };
            let (_, before) = Fake::new(if builder {
                "deployment-builder-before.png"
            } else {
                "battlefield-classic.png"
            });
            let cards = army::cards(
                &before,
                profile.screen(source).unwrap().army_bar.unwrap(),
                builder,
                &digits,
            )
            .unwrap();
            let bound =
                army::recipe::BoundCard::new(&cards[if builder { 1 } else { 3 }], &before, builder)
                    .unwrap();
            let (_, mut image) = Fake::new(if builder {
                "deployment-builder-survivors.png"
            } else {
                "battlefield-classic.png"
            });
            if builder {
                continuing_builder_hud(&mut image);
            }
            if !builder {
                // Explicitly simulate a spent gray Home card using its captured portrait.
                let rect = core::Rect::new(bound.card.slot.0 as i32 - 44, 600, 88, 120);
                let mut gray = Mat::default();
                imgproc::cvt_color_def(
                    &Mat::roi(&image, rect).unwrap(),
                    &mut gray,
                    imgproc::COLOR_BGR2GRAY,
                )
                .unwrap();
                let mut colour = Mat::default();
                imgproc::cvt_color_def(&gray, &mut colour, imgproc::COLOR_GRAY2BGR).unwrap();
                colour
                    .copy_to(&mut Mat::roi_mut(&mut image, rect).unwrap())
                    .unwrap();
            }
            assert_eq!(bound.ability_ready(&image, &digits).unwrap(), Some(false));
            assert!(army::empty(&image, bound.card.slot).unwrap());
            let next_card =
                army::recipe::BoundCard::new(&cards[if builder { 2 } else { 0 }], &before, builder)
                    .unwrap();
            for waited in [false, true] {
                let fake = Fake::from_image(&image);
                let temp = tempfile::tempdir().unwrap();
                let mut adb = fake.adb(temp.path());
                let mut engine =
                    Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
                engine.unconfirmed_input = waited;
                engine.stage = Stage::Deploy {
                    source,
                    steps: vec![
                        Deployment::PreciseAbility {
                            card: bound.clone(),
                        },
                        Deployment::PreciseDrop {
                            card: next_card.clone(),
                            point: Point(0, 500),
                            count: 1,
                            interval_ms: 500,
                            initial_count: if builder { 1 } else { 11 },
                        },
                    ]
                    .into(),
                    selected: false,
                    sent: 0,
                    waited,
                };
                engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image: &image,
                            detection: &Detection::Known(if builder {
                                Screen::BuilderBattle
                            } else {
                                Screen::HomeBattle
                            }),
                            elapsed_ms: 1000,
                            captured_at_ms: 1000,
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                assert!(
                    matches!(engine.stage,Stage::Deploy{ref steps,..} if steps.len()==1 && matches!(steps.front(),Some(Deployment::PreciseDrop{..})))
                );
                assert!(engine.recovery_safe());
                assert!(fake.taps.lock().unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn recurring_builder_hero_requires_full_fresh_charge_and_confirmed_consumption() {
        use crate::device::{
            DeviceConfig, InputMode,
            process::{Invocation, Transport},
        };
        #[derive(Clone)]
        struct Interrupted {
            fake: Fake,
            cancel_after_tap: bool,
        }
        impl Transport for Interrupted {
            async fn run(
                &self,
                command: Invocation,
                cancel: &CancellationToken,
            ) -> Result<Vec<u8>> {
                let tapped = command.args.iter().any(|arg| arg == "tap");
                let result = self.fake.run(command, cancel).await;
                if tapped && self.cancel_after_tap {
                    cancel.cancel();
                    return Err(Error::Cancelled);
                }
                result
            }
        }
        let mut profile = Profile::load(&root()).unwrap();
        profile.speed_button = None;
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
            detector: detector.clone(),
        };
        let (_, before) = Fake::new("deployment-builder-before.png");
        let (_, deployed) = Fake::new("deployment-builder-hero-deployed.png");
        let (_, mut partial) = Fake::new("english-builder_battle.png");
        let bar = core::Rect::new(0, 545, 1280, 175);
        Mat::roi(&deployed, bar)
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut partial, bar).unwrap())
            .unwrap();
        let mut full = partial.try_clone().unwrap();
        let (_, meter) = Fake::new("builder-hero-meter-3.png");
        // Only the captured meter is repositioned; the current hero portrait and HUD are unchanged.
        Mat::roi(&meter, core::Rect::new(93, 39, 73, 8))
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut full, core::Rect::new(93, 559, 73, 8)).unwrap())
            .unwrap();
        assert_eq!(
            detector.detect(&full).unwrap(),
            Detection::Known(Screen::BuilderBattle)
        );
        assert_eq!(army::hero_charge(&full, Point(129, 648)).unwrap(), 3);
        assert_eq!(army::hero_charge(&partial, Point(129, 648)).unwrap(), 1);
        for scenario in [
            "repeat",
            "fresh_changed",
            "timeout",
            "cancel",
            "unreadable_confirmation",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let make_adb = |image: &Mat, failure: bool, cancellation: bool| {
                let mut fake = Fake::from_image(image);
                fake.fail_after_tap = failure;
                std::fs::write(temp.path().join("adb.exe"), []).unwrap();
                let adb = Adb::new(
                    DeviceConfig {
                        adb: temp.path().join("adb.exe"),
                        serial: "emulator-5554".into(),
                        instance_index: 0,
                        resolution: profile.resolution,
                    },
                    InputMode::Live,
                    Interrupted {
                        fake: fake.clone(),
                        cancel_after_tap: cancellation,
                    },
                )
                .unwrap();
                (fake, adb)
            };
            let mut scanner = army::roster::Scanner::new(
                profile
                    .screen(Screen::BuilderScout)
                    .unwrap()
                    .army_bar
                    .unwrap(),
                true,
            )
            .unwrap();
            let army::roster::ScanStep::Complete(roster) =
                scanner.observe(&before, &digits).unwrap()
            else {
                panic!("complete captured roster")
            };
            let mut engine = Engine::new(
                Options {
                    villages: vec![Village::Builder],
                    ..Default::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.stage = Stage::Battle;
            engine.automatic = true;
            engine.pending_attack = true;
            engine.deployment_source = Some(Screen::BuilderScout);
            engine.heroes = vec![1].into();
            engine.roster = Some(roster);
            let cancel = CancellationToken::new();
            let (quiet, mut adb) = make_adb(&partial, false, false);
            let frame = |image, now| FrameView {
                image,
                detection: &Detection::Known(Screen::BuilderBattle),
                elapsed_ms: now,
                captured_at_ms: now as i64,
            };
            engine
                .tick(&vision, &mut adb, &frame(&partial, 1000), &cancel)
                .await
                .unwrap();
            assert!(matches!(engine.stage, Stage::Battle));
            assert!(quiet.taps.lock().unwrap().is_empty());
            engine
                .tick(&vision, &mut adb, &frame(&full, 2000), &cancel)
                .await
                .unwrap();
            assert!(
                matches!(engine.stage,Stage::Deploy{ref steps,..} if matches!(steps.front(),Some(Deployment::PreciseAbility{card}) if card.logical_slot==1))
            );
            let (activation, mut adb) = make_adb(
                if scenario == "fresh_changed" {
                    &partial
                } else {
                    &full
                },
                scenario == "timeout",
                scenario == "cancel",
            );
            let activated = engine
                .tick(&vision, &mut adb, &frame(&full, 2300), &cancel)
                .await;
            if scenario == "fresh_changed" {
                assert!(matches!(activated, Err(Error::InputGuard)));
                assert!(engine.recovery_safe());
                assert!(activation.taps.lock().unwrap().is_empty());
                let (retry, mut adb) = make_adb(&full, false, false);
                engine
                    .tick(&vision, &mut adb, &frame(&full, 2500), &cancel)
                    .await
                    .unwrap();
                assert_eq!(retry.taps.lock().unwrap().len(), 1);
                assert!(!engine.recovery_safe());
                engine
                    .tick(&vision, &mut adb, &frame(&partial, 3100), &cancel)
                    .await
                    .unwrap();
                assert!(engine.recovery_safe());
                assert_eq!(retry.taps.lock().unwrap().len(), 1);
                continue;
            }
            if matches!(scenario, "timeout" | "cancel") {
                assert!(activated.is_err(), "{scenario}");
                assert!(!engine.recovery_safe());
                assert!(engine.recover(4000).is_err());
                assert_eq!(
                    activation.taps.lock().unwrap().len(),
                    usize::from(scenario != "fresh_changed")
                );
                if scenario == "cancel" {
                    assert!(cancel.is_cancelled());
                }
                continue;
            }
            activated.unwrap();
            assert_eq!(activation.taps.lock().unwrap().len(), 1);
            assert!(!engine.recovery_safe());
            if scenario == "unreadable_confirmation" {
                let black = Mat::new_rows_cols_with_default(
                    720,
                    1280,
                    core::CV_8UC3,
                    core::Scalar::all(0.),
                )
                .unwrap();
                engine
                    .tick(&vision, &mut adb, &frame(&black, 3000), &cancel)
                    .await
                    .unwrap();
                assert!(!engine.recovery_safe());
                assert!(
                    engine
                        .tick(&vision, &mut adb, &frame(&black, 7000), &cancel)
                        .await
                        .is_err()
                );
                assert!(engine.recover(8000).is_err());
                assert_eq!(activation.taps.lock().unwrap().len(), 1);
                continue;
            }
            engine
                .tick(&vision, &mut adb, &frame(&partial, 3000), &cancel)
                .await
                .unwrap();
            assert!(engine.recovery_safe());
            engine
                .tick(&vision, &mut adb, &frame(&partial, 3300), &cancel)
                .await
                .unwrap();
            assert!(matches!(engine.stage, Stage::Battle));
            assert!(!engine.repeat_ability);
            engine
                .tick(&vision, &mut adb, &frame(&full, 6100), &cancel)
                .await
                .unwrap();
            assert!(matches!(engine.stage, Stage::Deploy { .. }));
            engine
                .tick(&vision, &mut adb, &frame(&full, 6400), &cancel)
                .await
                .unwrap();
            assert_eq!(activation.taps.lock().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn a_troop_batch_requires_its_exact_count_and_confirmation_overlaps_spacing() {
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
        let (_, before) = Fake::new("camera-home-raised-top.png");
        let bar = profile.screen(Screen::HomeScout).unwrap().army_bar.unwrap();
        let cards = army::cards(&before, bar, false, &digits).unwrap();
        let bound = army::recipe::BoundCard::new(&cards[0], &before, false).unwrap();
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        engine.stage = Stage::Deploy {
            source: Screen::HomeScout,
            steps: vec![Deployment::PreciseDrop {
                card: bound,
                point: Point(0, 500),
                count: 6,
                interval_ms: 100,
                initial_count: 15,
            }]
            .into(),
            selected: true,
            sent: 0,
            waited: true,
        };
        engine.unconfirmed_input = true;
        engine.pending_drops = 3;
        engine.last_action = 1000;
        let temp = tempfile::tempdir().unwrap();
        for (file, now, confirmed) in [
            ("camera-home-raised-right.png", 1500, false),
            ("camera-home-raised-left.png", 2000, true),
        ] {
            let (fake, image) = Fake::new(file);
            let mut adb = fake.adb(temp.path());
            engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &image,
                        detection: &Detection::Known(Screen::HomeBattle),
                        elapsed_ms: now,
                        captured_at_ms: now as i64,
                    },
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert_eq!(engine.recovery_safe(), confirmed);
            assert!(fake.taps.lock().unwrap().is_empty());
        }
        assert!(matches!(
            engine.stage,
            Stage::Deploy {
                sent: 3,
                waited: false,
                ..
            }
        ));
        assert_eq!(engine.not_before, 2000);
    }

    #[tokio::test]
    async fn an_unsent_army_scroll_retries_without_claiming_endpoint_motion() {
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
        let (_, mut image) = Fake::new("battlefield-classic.png");
        let (_, overflow) = Fake::new("army-home-overflow-left.png");
        let rect = core::Rect::new(0, 580, 1280, 140);
        Mat::roi(&overflow, rect)
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut image, rect).unwrap())
            .unwrap();
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        engine.stage = Stage::ReadArmy {
            screen: Screen::HomeScout,
        };
        engine.scanner = Some(
            army::roster::Scanner::new(
                profile.screen(Screen::HomeScout).unwrap().army_bar.unwrap(),
                false,
            )
            .unwrap(),
        );
        let temp = tempfile::tempdir().unwrap();
        for now in [1000, 2000, 3000] {
            let fake = if now < 3000 {
                Fake::new("english-home.png").0
            } else {
                Fake::from_image(&image)
            };
            let mut adb = fake.adb(temp.path());
            let result = engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &image,
                        detection: &Detection::Known(Screen::HomeScout),
                        elapsed_ms: now,
                        captured_at_ms: now as i64,
                    },
                    &CancellationToken::new(),
                )
                .await;
            if now < 3000 {
                assert!(matches!(result, Err(Error::InputGuard)));
                assert!(fake.taps.lock().unwrap().is_empty());
            } else {
                result.unwrap();
                assert_eq!(fake.taps.lock().unwrap().len(), 1);
            }
            assert!(matches!(engine.stage, Stage::ReadArmy { .. }));
        }
    }

    #[tokio::test]
    async fn custom_drop_retries_only_pre_dispatch_refusals_and_keeps_confirmed_counts() {
        use army::recipe::BoundCard;
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
            detector: detector.clone(),
        };
        let (_, selected) = Fake::new("battlefield-selected-army.png");
        let (_, mut image) = Fake::new("camera-home-start.png");
        let bar = core::Rect::new(0, 580, 1280, 140);
        Mat::roi(&selected, bar)
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut image, bar).unwrap())
            .unwrap();
        for (count, interval_ms, sent, refused) in [
            (1, 1000, 0, false),
            (1, 1000, 0, true),
            (5, 100, 0, true),
            (6, 100, 3, true),
        ] {
            let mut fake = if refused {
                Fake::new("english-home.png").0
            } else {
                Fake::from_image(&image)
            };
            fake.fail_after_tap = !refused;
            let (_, original) = Fake::new("battlefield-classic.png");
            let bar = profile.screen(Screen::HomeScout).unwrap().army_bar.unwrap();
            let cards = army::cards(&original, bar, false, &digits).unwrap();
            let card = BoundCard::new(&cards[0], &original, false).unwrap();
            let mut engine =
                Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
            engine.camera = Some(
                super::super::camera::Camera::new(
                    profile
                        .screen(Screen::HomeScout)
                        .unwrap()
                        .camera
                        .clone()
                        .unwrap(),
                    &image,
                )
                .unwrap(),
            );
            engine.pending_attack = true;
            engine.stage = Stage::Deploy {
                source: Screen::HomeScout,
                steps: vec![Deployment::PreciseDrop {
                    card,
                    point: Point(0, 500),
                    count,
                    interval_ms,
                    // Simulate already confirmed drops while retaining the captured current quantity.
                    initial_count: 15 + sent,
                }]
                .into(),
                selected: true,
                sent,
                waited: false,
            };
            let temp = tempfile::tempdir().unwrap();
            let mut adb = fake.adb(temp.path());
            let frame = FrameView {
                image: &image,
                detection: &Detection::Known(Screen::HomeScout),
                elapsed_ms: 1000,
                captured_at_ms: 1000,
            };
            let error = engine
                .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                .await
                .err()
                .expect("custom input must fail");
            if refused {
                assert!(matches!(error, Error::InputGuard));
                assert!(engine.recovery_safe());
                assert_eq!(engine.pending_drops, 0);
                assert!(fake.taps.lock().unwrap().is_empty());
                assert!(
                    matches!(engine.stage, Stage::Deploy { sent: saved, waited: false, .. } if saved == sent)
                );
                let retry = Fake::from_image(&image);
                let mut adb = retry.adb(temp.path());
                engine
                    .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                    .await
                    .unwrap();
                assert_eq!(retry.taps.lock().unwrap().len(), 1);
                assert_eq!(
                    engine.pending_drops,
                    if interval_ms == 100 {
                        (count - sent).min(5)
                    } else {
                        1
                    }
                );
                assert!(!engine.recovery_safe());
                assert!(
                    matches!(engine.stage, Stage::Deploy { sent: saved, waited: true, .. } if saved == sent)
                );
                continue;
            }
            assert!(matches!(error, Error::Timeout));
            assert_eq!(fake.taps.lock().unwrap().len(), 1);
            assert!(!engine.recovery_safe());
            assert!(engine.recover(2000).is_err());
            assert_eq!(fake.taps.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn custom_deadline_keeps_a_completed_result_and_sends_no_more_input() {
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
        let (fake, image) = Fake::new("english-numeric-home-victory.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        let (_, camera_frame) = Fake::new("camera-home-start.png");
        engine.camera = Some(
            super::super::camera::Camera::new(
                profile
                    .screen(Screen::HomeScout)
                    .unwrap()
                    .camera
                    .clone()
                    .unwrap(),
                &camera_frame,
            )
            .unwrap(),
        );
        engine.pending_attack = true;
        engine.stage = Stage::Deploy {
            source: Screen::HomeScout,
            steps: vec![Deployment::Wait(10000)].into(),
            selected: false,
            sent: 0,
            waited: true,
        };
        engine.not_before = 190000;
        let frame = FrameView {
            image: &image,
            detection: &Detection::Known(Screen::HomeResult),
            elapsed_ms: 180001,
            captured_at_ms: 180001,
        };
        engine
            .tick(&vision, &mut adb, &frame, &CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(engine.stage, Stage::Result { .. }));
        assert!(fake.taps.lock().unwrap().is_empty());

        let (fake, image) = Fake::new("english-builder_fourth_current.png");
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
        engine.camera = Some(
            super::super::camera::Camera::new(
                profile
                    .screen(Screen::HomeScout)
                    .unwrap()
                    .camera
                    .clone()
                    .unwrap(),
                &camera_frame,
            )
            .unwrap(),
        );
        engine.pending_attack = true;
        engine.unconfirmed_input = true;
        engine.stage = Stage::Deploy {
            source: Screen::BuilderScout,
            steps: vec![Deployment::Wait(10000)].into(),
            selected: true,
            sent: 0,
            waited: true,
        };
        engine.not_before = 190000;
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    image: &image,
                    detection: &Detection::Known(Screen::BuilderStage2),
                    elapsed_ms: 180001,
                    captured_at_ms: 180001,
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(matches!(
            engine.stage,
            Stage::Plan {
                screen: Screen::BuilderStage2,
                ..
            }
        ));
        assert!(engine.recovery_safe());
        assert!(fake.taps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pending_siege_drop_accepts_the_changed_artwork_without_repeating_input() {
        use army::recipe::BoundCard;
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
        let (_, before) = Fake::new("battlefield-classic.png");
        let (fake, after) = Fake::new("deployment-home-siege-deployed.png");
        let cards = army::cards(
            &before,
            profile.screen(Screen::HomeScout).unwrap().army_bar.unwrap(),
            false,
            &digits,
        )
        .unwrap();
        let card = BoundCard::new(&cards[2], &before, false).unwrap();
        assert_eq!(card.remaining(&after, &digits).unwrap(), None);
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        engine.pending_attack = true;
        engine.unconfirmed_input = true;
        engine.stage = Stage::Deploy {
            source: Screen::HomeScout,
            steps: vec![Deployment::PreciseDrop {
                card,
                point: Point(0, 500),
                count: 1,
                interval_ms: 300,
                initial_count: 1,
            }]
            .into(),
            selected: true,
            sent: 0,
            waited: true,
        };
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    image: &after,
                    detection: &Detection::Known(Screen::HomeBattle),
                    elapsed_ms: 1000,
                    captured_at_ms: 1000,
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(engine.recovery_safe());
        assert!(matches!(engine.stage,Stage::Deploy {ref steps,..} if steps.is_empty()));
        assert!(fake.taps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ldplayer_offer_only_closes_with_fresh_guards_and_no_pending_input() {
        let profile = Profile::load(&root()).unwrap();
        let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
        let calibration = Calibration::load(&root().join("config.json")).unwrap();
        let detector = Arc::new(Detector::load(&calibration, &root()).unwrap());
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector: detector.clone(),
        };
        let (_, original) = Fake::new("english-ldplayer-top-up.png");
        let temp = tempfile::tempdir().unwrap();
        for case in 0..8 {
            let mut fresh = original.clone();
            if (1..=3).contains(&case) {
                let crate::calibration::Region(x, y, w, h) =
                    calibration.screens[&Screen::LdplayerTopUp].anchors[case - 1]
                        .region()
                        .unwrap();
                Mat::roi_mut(
                    &mut fresh,
                    core::Rect::new(x as i32, y as i32, w as i32, h as i32),
                )
                .unwrap()
                .set_to(&core::Scalar::all(0.), &core::no_array())
                .unwrap();
            }
            let fake = Fake::from_image(&fresh);
            let mut adb = fake.adb(temp.path());
            let mut engine =
                Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
            engine.unconfirmed_input = case == 4;
            if case == 5 {
                engine.wall = Some(WallRun::pending_for_test());
            }
            let detection = if case == 6 {
                Detection::Unknown
            } else {
                Detection::Known(Screen::LdplayerTopUp)
            };
            let cancel = CancellationToken::new();
            if case == 7 {
                cancel.cancel();
            }
            let result = engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &original,
                        detection: &detection,
                        elapsed_ms: 3000,
                        captured_at_ms: 3000,
                    },
                    &cancel,
                )
                .await;
            assert_eq!(result.is_ok(), matches!(case, 0 | 6), "case {case}");
            let taps = fake.taps.lock().unwrap();
            if case == 0 {
                assert_eq!(*taps, vec!["-s emulator-5554 shell input tap 974 134"]);
            } else {
                assert!(taps.is_empty(), "case {case}");
            }
            assert!(matches!(engine.stage, Stage::Settle));
        }
    }

    #[tokio::test]
    async fn ldplayer_offer_close_attempts_are_bounded_until_a_recognized_screen_returns() {
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
        let (fake, image) = Fake::new("english-ldplayer-top-up.png");
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        let frame = FrameView {
            image: &image,
            detection: &Detection::Known(Screen::LdplayerTopUp),
            elapsed_ms: 3000,
            captured_at_ms: 3000,
        };
        for now in [3000, 5000, 7000] {
            engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        elapsed_ms: now,
                        ..frame
                    },
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
        }
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    detection: &Detection::Unknown,
                    elapsed_ms: 9000,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let result = engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    elapsed_ms: 10000,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(result, Err(Error::Gameplay(message)) if message.contains("three attempts"))
        );
        assert_eq!(fake.taps.lock().unwrap().len(), 3);
        assert_eq!(engine.top_up_closes, 3);
        let (home, home_image) = Fake::new("english-home.png");
        let mut home_adb = home.adb(temp.path());
        engine
            .tick(
                &vision,
                &mut home_adb,
                &FrameView {
                    image: &home_image,
                    detection: &Detection::Known(Screen::Home),
                    elapsed_ms: 11000,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(engine.top_up_closes, 0);
        assert!(matches!(engine.stage, Stage::Navigate));
        assert!(home.taps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn daily_reward_claims_before_closing_and_bounds_stuck_dialogs() {
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
            detector: detector.clone(),
        };
        let temp = tempfile::tempdir().unwrap();
        let (claim, image) = Fake::new("english-daily_reward_claim.png");
        assert_eq!(
            detector.detect(&image).unwrap(),
            Detection::Known(Screen::DailyReward)
        );
        let mut adb = claim.adb(temp.path());
        let mut engine = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        let frame = FrameView {
            image: &image,
            detection: &Detection::Known(Screen::DailyReward),
            elapsed_ms: 3000,
            captured_at_ms: 3000,
        };
        engine
            .tick(&vision, &mut adb, &frame, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            *claim.taps.lock().unwrap(),
            vec!["-s emulator-5554 shell input tap 661 304"]
        );
        let (closed, after) = Fake::new("english-daily_reward.png");
        let mut adb = closed.adb(temp.path());
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    image: &after,
                    elapsed_ms: 5000,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            *closed.taps.lock().unwrap(),
            vec!["-s emulator-5554 shell input tap 1077 104"]
        );
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    image: &after,
                    elapsed_ms: 7000,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let result = engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    image: &after,
                    elapsed_ms: 9000,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(result, Err(Error::Gameplay(message)) if message.contains("three actions"))
        );
        assert_eq!(closed.taps.lock().unwrap().len(), 2);

        let mut changed = Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
        let fresh = Fake::from_image(&after);
        let mut adb = fresh.adb(temp.path());
        let result = changed
            .tick(&vision, &mut adb, &frame, &CancellationToken::new())
            .await;
        assert!(matches!(result, Err(Error::Unreadable)));
        assert!(fresh.taps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn daily_reward_closes_only_with_both_captured_anchors() {
        let profile = Profile::load(&root()).unwrap();
        let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
        let calibration = Calibration::load(&root().join("config.json")).unwrap();
        let detector = Arc::new(Detector::load(&calibration, &root()).unwrap());
        let vision = GameVision {
            profile: &profile,
            digits: &digits,
            detector: detector.clone(),
        };
        let temp = tempfile::tempdir().unwrap();
        for (fixture, point) in [
            ("english-daily_reward.png", "1077 104"),
            ("english-daily_reward_claim.png", "661 304"),
        ] {
            let (_, original) = Fake::new(fixture);
            for missing in [None, Some(0), Some(1)] {
                let mut image = original.clone();
                if let Some(index) = missing {
                    let crate::calibration::Region(x, y, w, h) =
                        calibration.screens[&Screen::DailyReward].anchors[index]
                            .region()
                            .unwrap();
                    Mat::roi_mut(
                        &mut image,
                        core::Rect::new(x as i32, y as i32, w as i32, h as i32),
                    )
                    .unwrap()
                    .set_to(&core::Scalar::all(0.0), &core::no_array())
                    .unwrap();
                }
                let fake = Fake::from_image(&image);
                let mut adb = fake.adb(temp.path());
                let mut engine =
                    Engine::new(Options::default(), SessionMode::Farming, &profile).unwrap();
                let detection = detector.detect(&image).unwrap();
                assert_eq!(
                    detection,
                    if missing.is_none() {
                        Detection::Known(Screen::DailyReward)
                    } else {
                        Detection::Unknown
                    }
                );
                engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            image: &image,
                            detection: &detection,
                            elapsed_ms: 3000,
                            captured_at_ms: 3000,
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                let taps = fake.taps.lock().unwrap();
                if missing.is_none() {
                    assert_eq!(
                        *taps,
                        vec![format!("-s emulator-5554 shell input tap {point}")]
                    );
                } else {
                    assert!(taps.is_empty());
                }
            }
        }
    }

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
    async fn normal_farming_reads_all_storage_tooltips_without_requiring_stop_targets() {
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
            detector: detector.clone(),
        };
        for (village, key, index, expected) in [
            (Village::Home, "gold", 0, 26_000_000),
            (Village::Home, "elixir", 1, 26_000_000),
            (Village::Home, "dark", 2, 430_000),
            (Village::Builder, "gold", 0, 5_500_000),
            (Village::Builder, "elixir", 1, 5_500_000),
        ] {
            let name = if village == Village::Home {
                "home"
            } else {
                "builder"
            };
            let screen = if village == Village::Home {
                Screen::Home
            } else {
                Screen::Builder
            };
            let (fake, image) = Fake::new(&format!("english-numeric-{name}-{key}-tooltip.png"));
            assert_eq!(detector.detect(&image).unwrap(), Detection::Known(screen));
            let temp = tempfile::tempdir().unwrap();
            let mut adb = fake.adb(temp.path());
            let mut engine = Engine::new(
                Options {
                    villages: vec![village],
                    targets: vec![],
                    until_full: false,
                    ..Options::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.stage = Stage::Check {
                index,
                opened: true,
            };
            engine
                .capacities
                .insert((name.into(), key.into()), expected + 1);
            let outcome = engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &image,
                        detection: &Detection::Known(screen),
                        elapsed_ms: 3000,
                        captured_at_ms: 3000,
                    },
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            let Reading::Known(balance) = outcome.resources[0].reading else {
                panic!("capacity unavailable")
            };
            assert_eq!(balance.capacity, Some(expected), "{name} {key}");
            assert!(!outcome.completed);
            assert_eq!(fake.taps.lock().unwrap().len(), 1);
            assert!(
                matches!(engine.stage, Stage::Check { index: next, opened: false } if next == index + 1)
            );
        }
    }

    #[tokio::test]
    async fn balance_retries_captured_unreadable_frame_with_a_deadline() {
        for required in [false, true] {
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
                detector: detector.clone(),
            };
            let (fake, unreadable) = Fake::new("english-home-elixir-unreadable.png");
            let (readable_fake, readable) = Fake::new("english-home-elixir-readable.png");
            for image in [&unreadable, &readable] {
                assert_eq!(
                    detector.detect(image).unwrap(),
                    Detection::Known(Screen::Home)
                );
            }
            let spec = &profile.capacity["home"]["elixir"];
            assert!(
                digits
                    .trailing(&crop(&unreadable, spec.bar).unwrap(), 0.65, 110)
                    .is_err()
            );
            let temp = tempfile::tempdir().unwrap();
            let mut adb = fake.adb(temp.path());
            let mut engine = Engine::new(
                Options::default(),
                if required {
                    SessionMode::Walls
                } else {
                    SessionMode::Farming
                },
                &profile,
            )
            .unwrap();
            engine.stage = Stage::Check {
                index: 1,
                opened: true,
            };
            engine.since = 1000;
            let frame = FrameView {
                image: &unreadable,
                detection: &Detection::Known(Screen::Home),
                elapsed_ms: 2000,
                captured_at_ms: 2000,
            };
            for elapsed_ms in [2000, if required { 16000 } else { 3000 }] {
                let outcome = engine
                    .tick(
                        &vision,
                        &mut adb,
                        &FrameView {
                            elapsed_ms,
                            ..frame
                        },
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                assert!(outcome.resources.is_empty());
                assert!(matches!(
                    engine.stage,
                    Stage::Check {
                        index: 1,
                        opened: true
                    }
                ));
            }
            assert!(fake.taps.lock().unwrap().is_empty());
            if required {
                assert!(matches!(
                    engine
                        .tick(
                            &vision,
                            &mut adb,
                            &FrameView {
                                elapsed_ms: 16001,
                                ..frame
                            },
                            &CancellationToken::new()
                        )
                        .await,
                    Err(Error::Unreadable)
                ));
            }
            let mut adb = readable_fake.adb(temp.path());
            let outcome = engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &readable,
                        elapsed_ms: if required { 17000 } else { 3500 },
                        ..frame
                    },
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(matches!(
                outcome.resources[0].reading,
                Reading::Known(Balance {
                    amount: 3_500_219,
                    capacity: Some(26_000_000)
                })
            ));
            assert_eq!(engine.recovery_count, 0);
            assert_eq!(readable_fake.taps.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn unreadable_optional_balance_closes_its_tooltip_before_the_next_target() {
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
            detector: detector.clone(),
        };
        let (_, mut image) = Fake::new("english-numeric-home-gold-tooltip.png");
        let crate::calibration::Region(x, y, w, h) = profile.capacity["home"]["gold"].bar;
        opencv::imgproc::rectangle(
            &mut image,
            core::Rect::new(x as i32, y as i32, w as i32, h as i32),
            core::Scalar::all(0.0),
            -1,
            opencv::imgproc::LINE_8,
            0,
        )
        .unwrap();
        assert_eq!(
            detector.detect(&image).unwrap(),
            Detection::Known(Screen::Home)
        );
        let fake = Fake::from_image(&image);
        let temp = tempfile::tempdir().unwrap();
        let mut adb = fake.adb(temp.path());
        let mut engine = Engine::new(
            Options {
                until_full: true,
                targets: vec![Resource::Elixir],
                ..Options::default()
            },
            SessionMode::Farming,
            &profile,
        )
        .unwrap();
        engine.stage = Stage::Check {
            index: 0,
            opened: true,
        };
        let frame = FrameView {
            image: &image,
            detection: &Detection::Known(Screen::Home),
            elapsed_ms: 4000,
            captured_at_ms: 4000,
        };
        let outcome = engine
            .tick(&vision, &mut adb, &frame, &CancellationToken::new())
            .await
            .unwrap();
        assert!(outcome.resources.is_empty());
        assert_eq!(
            *fake.taps.lock().unwrap(),
            vec!["-s emulator-5554 shell input tap 1120 38"]
        );
        assert!(matches!(
            engine.stage,
            Stage::Check {
                index: 1,
                opened: false
            }
        ));
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    elapsed_ms: 4500,
                    ..frame
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(fake.taps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn missing_optional_capacity_is_bounded_but_required_capacity_fails_closed() {
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
        for required in [false, true] {
            let (fake, image) = Fake::new("english-home.png");
            let temp = tempfile::tempdir().unwrap();
            let mut adb = fake.adb(temp.path());
            let mut engine = Engine::new(
                Options {
                    until_full: required,
                    ..Options::default()
                },
                SessionMode::Farming,
                &profile,
            )
            .unwrap();
            engine.stage = Stage::Check {
                index: 0,
                opened: false,
            };
            let frame = FrameView {
                image: &image,
                detection: &Detection::Known(Screen::Home),
                elapsed_ms: 1000,
                captured_at_ms: 1000,
            };
            engine
                .tick(&vision, &mut adb, &frame, &CancellationToken::new())
                .await
                .unwrap();
            assert_eq!(
                *fake.taps.lock().unwrap(),
                vec!["-s emulator-5554 shell input tap 1120 38"]
            );
            let result = engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        elapsed_ms: 17000,
                        ..frame
                    },
                    &CancellationToken::new(),
                )
                .await;
            if required {
                assert!(matches!(result, Err(Error::Unreadable)));
            } else {
                assert!(result.unwrap().resources.is_empty());
                assert!(matches!(
                    engine.stage,
                    Stage::Check {
                        index: 1,
                        opened: false
                    }
                ));
            }
            assert_eq!(fake.taps.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn visible_elixir_cart_is_opened_without_panning() {
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
            detector: detector.clone(),
        };
        let (fake, image) = Fake::new("english-builder-visible-cart.png");
        assert_eq!(
            detector.detect(&image).unwrap(),
            Detection::Known(Screen::Builder)
        );
        let target = &profile.screens[&Screen::Builder].targets["cart"];
        let point = target.locate(&image).unwrap().unwrap();
        assert!((795..=836).contains(&point.0) && (340..=370).contains(&point.1));
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
        engine.stage = Stage::Cart(CartPhase::Find);
        engine
            .tick(
                &vision,
                &mut adb,
                &FrameView {
                    image: &image,
                    detection: &Detection::Known(Screen::Builder),
                    elapsed_ms: 3000,
                    captured_at_ms: 3000,
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            *fake.taps.lock().unwrap(),
            vec![format!(
                "-s emulator-5554 shell input tap {} {}",
                point.0, point.1
            )]
        );
        assert_eq!(engine.pans, 0);
        assert!(matches!(engine.stage, Stage::Cart(CartPhase::Claim)));

        // The elixir bubble alone must not identify a cart or another collector.
        let mut no_cart = image.try_clone().unwrap();
        opencv::imgproc::rectangle(
            &mut no_cart,
            core::Rect::new(789, 337, 52, 38),
            core::Scalar::all(0.0),
            -1,
            opencv::imgproc::LINE_8,
            0,
        )
        .unwrap();
        assert!(target.locate(&no_cart).unwrap().is_none());
        for moved in [false, true] {
            let mut fresh = no_cart.try_clone().unwrap();
            if moved {
                Mat::roi(&image, core::Rect::new(789, 304, 52, 71))
                    .unwrap()
                    .copy_to(
                        &mut Mat::roi_mut(&mut fresh, core::Rect::new(400, 300, 52, 71)).unwrap(),
                    )
                    .unwrap();
                assert!(target.locate(&fresh).unwrap().is_some());
            }
            let fake = Fake::from_image(&fresh);
            let mut adb = fake.adb(temp.path());
            engine.stage = Stage::Cart(CartPhase::Find);
            let result = engine
                .tick(
                    &vision,
                    &mut adb,
                    &FrameView {
                        image: &image,
                        detection: &Detection::Known(Screen::Builder),
                        elapsed_ms: 6000,
                        captured_at_ms: 6000,
                    },
                    &CancellationToken::new(),
                )
                .await;
            assert!(matches!(result, Err(Error::Unreadable)));
            assert!(fake.taps.lock().unwrap().is_empty());
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
            index: 2,
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
            index: 2,
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
        for screen in [
            Screen::UpdateOffer,
            Screen::BuildersBusy,
            Screen::DailyReward,
            Screen::LdplayerTopUp,
        ] {
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
