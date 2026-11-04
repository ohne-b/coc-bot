use std::{path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::{Result, settings::Settings, telemetry::*};

/// Own this connection on a background worker; SQLite calls are blocking.
pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = transaction.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let application: i64 =
            transaction.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if version == 0 && application == 0 {
            let objects: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
                [],
                |r| r.get(0),
            )?;
            if objects != 0 {
                return Err(invalid(
                    "The selected database belongs to another application",
                ));
            }
            transaction.execute_batch(include_str!("schema.sql"))?;
        } else if !(1..=2).contains(&version) || application != 1_129_268_034 {
            return Err(invalid(
                "Unsupported database version or application; data was not changed",
            ));
        }
        if version < 2 {
            transaction.execute_batch("CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), data TEXT NOT NULL CHECK (json_valid(data))) STRICT; PRAGMA user_version = 2;")?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }

    pub fn settings(&self) -> Result<Settings> {
        let data: Option<String> = self
            .connection
            .query_row("SELECT data FROM settings WHERE id = 1", [], |r| r.get(0))
            .optional()?;
        let settings: Settings = match data {
            Some(data) => serde_json::from_str(&data)?,
            None => Settings::default(),
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        self.connection.execute(
            "INSERT INTO settings (id, data) VALUES (1, ?1) ON CONFLICT (id) DO UPDATE SET data = excluded.data",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    pub fn begin_session(&mut self, mode: SessionMode, at_ms: i64) -> Result<Session> {
        timestamp(at_ms)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO sessions (mode, started_at_ms, updated_at_ms, state, active_ms)
             VALUES (?1, ?2, ?2, 'running', 0)",
            params![key(mode)?, at_ms],
        )?;
        let session = session(&transaction, transaction.last_insert_rowid())?;
        timestamp(session.id)?;
        transaction.commit()?;
        Ok(session)
    }

    pub fn session(&self, id: i64) -> Result<Session> {
        session(&self.connection, id)
    }

    pub fn sessions(&self, before_id: Option<i64>, limit: u32) -> Result<Vec<Session>> {
        page(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT id FROM sessions WHERE (?1 IS NULL OR id < ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let ids = statement.query_map(params![before_id, limit], |r| r.get::<_, i64>(0))?;
        ids.map(|id| session(&self.connection, id?)).collect()
    }

    pub fn update_session(
        &mut self,
        id: i64,
        state: SessionState,
        active_ms: i64,
        at_ms: i64,
    ) -> Result<Session> {
        timestamp(active_ms)?;
        timestamp(at_ms)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = require_active(&transaction, id)?;
        if active_ms < previous.active_ms {
            return Err(invalid("Session active time cannot decrease"));
        }
        transaction.execute(
            "UPDATE sessions SET state = ?1, active_ms = ?2, updated_at_ms = ?3 WHERE id = ?4",
            params![key(state)?, active_ms, at_ms, id],
        )?;
        let result = session(&transaction, id)?;
        transaction.commit()?;
        Ok(result)
    }

    /// Call once after acquiring the desktop single-instance lock; never resume input after a crash.
    pub fn interrupt_open_sessions(&mut self, at_ms: i64) -> Result<usize> {
        timestamp(at_ms)?;
        Ok(self.connection.execute(
            "UPDATE sessions SET state = 'interrupted', updated_at_ms = ?1 WHERE state IN ('running', 'paused')",
            [at_ms],
        )?)
    }

    pub fn observe(&mut self, id: i64, observation: ResourceObservation) -> Result<()> {
        observation.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_active(&transaction, id)?;
        let village = key(observation.village)?;
        let resource = key(observation.resource)?;
        let previous: Option<(i64, String, Option<String>)> = transaction.query_row(
            "SELECT checked_at_ms, latest, last_known FROM resources WHERE session_id = ?1 AND village = ?2 AND resource = ?3",
            params![id, village, resource], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        let latest = serde_json::to_string(&observation)?;
        if let Some((at, prior, _)) = &previous
            && (*at > observation.checked_at_ms
                || (*at == observation.checked_at_ms && *prior != latest))
        {
            return Err(invalid(
                "Resource observation is stale or conflicts with an existing reading",
            ));
        }
        let known = match observation.reading {
            Reading::Known(mut balance) => {
                // Closing the tooltip hides capacity, but does not change the storage maximum.
                if balance.capacity.is_none() {
                    let prior = previous
                        .as_ref()
                        .and_then(|(_, _, known)| known.as_deref())
                        .map(serde_json::from_str::<Balance>)
                        .transpose()?;
                    balance.capacity = prior
                        .and_then(|v| v.capacity)
                        .filter(|maximum| *maximum >= balance.amount);
                }
                Some(serde_json::to_string(&balance)?)
            }
            _ => None,
        };
        transaction.execute(
            "INSERT INTO resources (session_id, village, resource, checked_at_ms, latest, last_known, observed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (session_id, village, resource) DO UPDATE SET
               checked_at_ms = excluded.checked_at_ms, latest = excluded.latest,
               last_known = COALESCE(excluded.last_known, resources.last_known),
               observed_at_ms = COALESCE(excluded.observed_at_ms, resources.observed_at_ms)",
            params![id, village, resource, observation.checked_at_ms, latest, known,
                    known.as_ref().map(|_| observation.checked_at_ms)],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn resources(&self, id: i64) -> Result<Vec<ResourceValue>> {
        self.session(id)?;
        let mut statement = self.connection.prepare(
            "SELECT latest, last_known, observed_at_ms FROM resources WHERE session_id = ?1 ORDER BY village, resource",
        )?;
        let rows = statement.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get(2)?,
            ))
        })?;
        rows.map(|row| {
            let (latest, known, observed_at_ms) = row?;
            let latest: ResourceObservation = serde_json::from_str(&latest)?;
            latest.validate()?;
            Ok(ResourceValue {
                latest,
                last_known: known.map(|v| serde_json::from_str(&v)).transpose()?,
                observed_at_ms,
            })
        })
        .collect()
    }

    pub fn record_attack(&mut self, id: i64, attack: &Attack) -> Result<()> {
        attack.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_live_session(&transaction, id)?;
        let data = serde_json::to_string(attack)?;
        let previous = transaction
            .query_row(
                "SELECT data FROM attacks WHERE session_id = ?1 AND ordinal = ?2",
                params![id, attack.ordinal],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if !duplicate(previous, &data)? {
            transaction.execute(
                "INSERT INTO attacks (session_id, ordinal, village, gold, elixir, dark, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, attack.ordinal, key(attack.village)?, attack.loot.gold, attack.loot.elixir, attack.loot.dark, data],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_wall(&mut self, id: i64, wall: &WallUpgrade) -> Result<()> {
        wall.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_live_session(&transaction, id)?;
        let data = serde_json::to_string(wall)?;
        let previous = transaction
            .query_row(
                "SELECT data FROM walls WHERE session_id = ?1 AND ordinal = ?2",
                params![id, wall.ordinal],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if !duplicate(previous, &data)? {
            transaction.execute(
                "INSERT INTO walls (session_id, ordinal, village, resource, cost, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, wall.ordinal, key(wall.village)?, key(wall.resource)?, wall.cost, data],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_activity(&mut self, id: i64, activity: &Activity) -> Result<()> {
        timestamp(activity.at_ms)?;
        if activity.sequence == 0
            || activity.message.trim().is_empty()
            || activity.message.len() > 2048
        {
            return Err(invalid(
                "Activity needs a sequence and a message of at most 2048 bytes",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_active(&transaction, id)?;
        let data = serde_json::to_string(activity)?;
        let previous = transaction
            .query_row(
                "SELECT data FROM activity WHERE session_id = ?1 AND sequence = ?2",
                params![id, activity.sequence],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if !duplicate(previous, &data)? {
            transaction.execute(
                "INSERT INTO activity (session_id, sequence, data) VALUES (?1, ?2, ?3)",
                params![id, activity.sequence, data],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn attacks(&self, id: i64, before_ordinal: Option<u32>, limit: u32) -> Result<Vec<Attack>> {
        self.records(
            "SELECT data FROM attacks WHERE session_id = ?1 AND (?2 IS NULL OR ordinal < ?2) ORDER BY ordinal DESC LIMIT ?3",
            id, before_ordinal, limit,
        )
    }

    pub fn walls(
        &self,
        id: i64,
        before_ordinal: Option<u32>,
        limit: u32,
    ) -> Result<Vec<WallUpgrade>> {
        self.records(
            "SELECT data FROM walls WHERE session_id = ?1 AND (?2 IS NULL OR ordinal < ?2) ORDER BY ordinal DESC LIMIT ?3",
            id, before_ordinal, limit,
        )
    }

    pub fn activity(
        &self,
        id: i64,
        before_sequence: Option<u32>,
        limit: u32,
    ) -> Result<Vec<Activity>> {
        self.records(
            "SELECT data FROM activity WHERE session_id = ?1 AND (?2 IS NULL OR sequence < ?2) ORDER BY sequence DESC LIMIT ?3",
            id, before_sequence, limit,
        )
    }

    fn records<T: DeserializeOwned>(
        &self,
        sql: &str,
        id: i64,
        before: Option<u32>,
        limit: u32,
    ) -> Result<Vec<T>> {
        self.session(id)?;
        page(limit)?;
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params![id, before, limit], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn statistics(&mut self, id: i64, village: Village) -> Result<Statistics> {
        let transaction = self.connection.transaction()?;
        let session = session(&transaction, id)?;
        let village_key = key(village)?;
        let mut result = transaction.query_row(
            "SELECT COUNT(*), SUM(gold), COUNT(gold), SUM(elixir), COUNT(elixir), SUM(dark), COUNT(dark)
             FROM attacks WHERE session_id = ?1 AND village = ?2", params![id, village_key],
            |row| {
                let attacks: u32 = row.get(0)?;
                let total = |column| -> rusqlite::Result<ResourceTotal> {
                    let amount = row.get(column)?;
                    let measured_attacks: u32 = row.get(column + 1)?;
                    Ok(ResourceTotal {
                        amount, measured_attacks,
                        average_per_attack: amount.filter(|_| measured_attacks > 0).map(|v| v as f64 / f64::from(measured_attacks)),
                        per_hour: rate(amount, session.active_ms),
                    })
                };
                Ok(Statistics {
                    village, attacks, attacks_per_hour: rate(Some(i64::from(attacks)), session.active_ms),
                    gold: total(1)?, elixir: total(3)?, dark: total(5)?, wall_upgrades: 0, wall_gold: 0, wall_elixir: 0,
                })
            },
        )?;
        (result.wall_upgrades, result.wall_gold, result.wall_elixir) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN resource = 'gold' THEN cost END), 0),
                    COALESCE(SUM(CASE WHEN resource = 'elixir' THEN cost END), 0)
             FROM walls WHERE session_id = ?1 AND village = ?2",
            params![id, village_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        transaction.commit()?;
        Ok(result)
    }
}

fn key(value: impl Serialize) -> Result<String> {
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}

fn decode_key<T: DeserializeOwned>(value: String) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(value))?)
}

fn session(connection: &Connection, id: i64) -> Result<Session> {
    let row = connection.query_row(
        "SELECT mode, started_at_ms, updated_at_ms, state, active_ms FROM sessions WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get::<_, String>(3)?,
                r.get(4)?,
            ))
        },
    )?;
    Ok(Session {
        id,
        mode: decode_key(row.0)?,
        started_at_ms: row.1,
        updated_at_ms: row.2,
        state: decode_key(row.3)?,
        active_ms: row.4,
    })
}

fn require_active(transaction: &Transaction<'_>, id: i64) -> Result<Session> {
    let session = session(transaction, id)?;
    if !session.state.is_active() {
        return Err(invalid("The session has already ended"));
    }
    Ok(session)
}

fn require_live_session(transaction: &Transaction<'_>, id: i64) -> Result<()> {
    if require_active(transaction, id)?.mode == SessionMode::Observe {
        return Err(invalid(
            "Observation sessions cannot record gameplay results",
        ));
    }
    Ok(())
}

fn duplicate(previous: Option<String>, data: &str) -> Result<bool> {
    match previous {
        Some(previous) if previous != data => {
            Err(invalid("Conflicting result for an existing operation"))
        }
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

fn page(limit: u32) -> Result<()> {
    if !(1..=100).contains(&limit) {
        return Err(invalid("History page size must be between 1 and 100"));
    }
    Ok(())
}
