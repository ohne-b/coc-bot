use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Village {
    Home,
    Builder,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Gold,
    Elixir,
    Dark,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Balance {
    pub amount: u32,
    pub capacity: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Reading {
    Known(Balance),
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceObservation {
    pub village: Village,
    pub resource: Resource,
    pub checked_at_ms: i64,
    pub reading: Reading,
}

impl ResourceObservation {
    pub fn validate(self) -> Result<()> {
        timestamp(self.checked_at_ms)?;
        if self.village == Village::Builder && self.resource == Resource::Dark {
            return Err(invalid("Builder Base has no Dark Elixir"));
        }
        if let Reading::Known(balance) = self.reading
            && balance
                .capacity
                .is_some_and(|capacity| capacity == 0 || balance.amount > capacity)
        {
            return Err(invalid("Resource amount exceeds its capacity"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceValue {
    pub latest: ResourceObservation,
    pub last_known: Option<Balance>,
    pub observed_at_ms: Option<i64>,
}

impl ResourceValue {
    pub fn is_fresh(&self, now_ms: i64, max_age_ms: u32) -> bool {
        matches!(self.latest.reading, Reading::Known(_))
            && self.observed_at_ms.is_some_and(|at| {
                now_ms >= at && now_ms.saturating_sub(at) <= i64::from(max_age_ms)
            })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Loot {
    pub gold: Option<u32>,
    pub elixir: Option<u32>,
    pub dark: Option<u32>,
}

/// Only a confirmed result belongs here; scout loot and storage deltas are not earnings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Attack {
    pub ordinal: u32,
    pub village: Village,
    pub completed_at_ms: i64,
    pub loot: Loot,
    pub damage: Option<u8>,
    pub stars: Option<u8>,
}

impl Attack {
    pub fn validate(&self) -> Result<()> {
        timestamp(self.completed_at_ms)?;
        if self.ordinal == 0
            || self.damage.is_some_and(|v| v > 100)
            || self.stars.is_some_and(|v| {
                v > if self.village == Village::Builder {
                    6
                } else {
                    3
                }
            })
            || (self.village == Village::Builder && self.loot.dark.is_some())
        {
            return Err(invalid("Invalid battle result"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WallUpgrade {
    pub ordinal: u32,
    pub village: Village,
    pub completed_at_ms: i64,
    pub resource: Resource,
    pub cost: u32,
}

impl WallUpgrade {
    pub fn validate(&self) -> Result<()> {
        timestamp(self.completed_at_ms)?;
        if self.ordinal == 0 || self.cost == 0 || self.resource == Resource::Dark {
            return Err(invalid("Invalid wall upgrade"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    Observe,
    Farming,
    Walls,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Running,
    Paused,
    Stopped,
    Completed,
    Failed,
    Interrupted,
}

impl SessionState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Paused)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Session {
    pub id: i64,
    pub mode: SessionMode,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    pub state: SessionState,
    /// Monotonic engine time, excluding pauses; never derived from the wall clock.
    pub active_ms: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    RecoveryStarted,
    RecoveryCompleted,
    DetectionFailed,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Activity {
    pub sequence: u32,
    pub at_ms: i64,
    pub kind: ActivityKind,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ResourceTotal {
    pub amount: Option<i64>,
    pub measured_attacks: u32,
    pub average_per_attack: Option<f64>,
    pub per_hour: Option<f64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Statistics {
    pub village: Village,
    pub attacks: u32,
    pub attacks_per_hour: Option<f64>,
    pub gold: ResourceTotal,
    pub elixir: ResourceTotal,
    pub dark: ResourceTotal,
    pub wall_upgrades: u32,
    pub wall_gold: i64,
    pub wall_elixir: i64,
}

pub(crate) fn rate(amount: Option<i64>, active_ms: i64) -> Option<f64> {
    amount
        .filter(|_| active_ms > 0)
        .map(|v| v as f64 * 3_600_000.0 / active_ms as f64)
}

pub(crate) fn timestamp(value: i64) -> Result<()> {
    // Milliseconds and counters cross JSON IPC as exact JavaScript integers.
    if !(0..=9_007_199_254_740_991).contains(&value) {
        return Err(invalid(
            "Timestamp or duration is outside the supported range",
        ));
    }
    Ok(())
}

pub(crate) fn invalid(message: &str) -> Error {
    Error::History(message.into())
}
