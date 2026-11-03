use coc_bot_core::{storage::Store, telemetry::*};
use rusqlite::Connection;

fn attack(ordinal: u32) -> Attack {
    Attack {
        ordinal,
        village: Village::Home,
        completed_at_ms: 1000 + i64::from(ordinal),
        loot: Loot {
            gold: Some(1_000_000),
            elixir: Some(500_000),
            dark: None,
        },
        damage: Some(50),
        stars: Some(1),
    }
}

fn observation(at: i64, reading: Reading) -> ResourceObservation {
    ResourceObservation {
        village: Village::Home,
        resource: Resource::Gold,
        checked_at_ms: at,
        reading,
    }
}

#[test]
fn failed_readings_preserve_last_known_values_and_age_without_becoming_zero() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("history.sqlite3")).unwrap();
    let id = store.begin_session(SessionMode::Observe, 100).unwrap().id;
    store
        .observe(id, observation(110, Reading::Failed))
        .unwrap();
    let values = store.resources(id).unwrap();
    assert_eq!(values[0].last_known, None);
    assert_eq!(values[0].observed_at_ms, None);
    assert!(!values[0].is_fresh(110, 100));
    let balance = Balance {
        amount: 10,
        capacity: Some(100),
    };
    store
        .observe(id, observation(200, Reading::Known(balance)))
        .unwrap();
    let values = store.resources(id).unwrap();
    assert!(values[0].is_fresh(300, 100));
    assert!(!values[0].is_fresh(301, 100));
    assert!(!values[0].is_fresh(199, 100));
    for (at, reading) in [(201, Reading::Failed), (202, Reading::Unavailable)] {
        store.observe(id, observation(at, reading)).unwrap();
        let values = store.resources(id).unwrap();
        assert_eq!(values[0].last_known, Some(balance));
        assert_eq!(values[0].observed_at_ms, Some(200));
        assert!(!values[0].is_fresh(at, 100));
        assert_eq!(values[0].latest.reading, reading);
    }
    assert!(
        store
            .observe(id, observation(199, Reading::Known(balance)))
            .is_err()
    );
    assert!(
        store
            .observe(id, observation(202, Reading::Failed))
            .is_err()
    );
    assert!(
        store
            .observe(
                id,
                observation(
                    203,
                    Reading::Known(Balance {
                        amount: 101,
                        capacity: Some(100)
                    })
                )
            )
            .is_err()
    );
    assert_eq!(store.resources(id).unwrap()[0].latest.checked_at_ms, 202);
    let json = serde_json::to_value(&store.resources(id).unwrap()[0]).unwrap();
    assert_eq!(json["latest"]["reading"]["status"], "unavailable");
}

#[test]
fn statistics_use_confirmed_results_with_partial_coverage_and_separate_wall_spending() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("history.sqlite3")).unwrap();
    let id = store.begin_session(SessionMode::Farming, 100).unwrap().id;
    let empty = store.statistics(id, Village::Home).unwrap();
    assert_eq!(empty.attacks, 0);
    assert_eq!(empty.gold.amount, None);
    assert_eq!(empty.attacks_per_hour, None);
    store.record_attack(id, &attack(1)).unwrap();
    store.record_attack(id, &attack(1)).unwrap();
    let partial = Attack {
        loot: Loot {
            gold: Some(0),
            elixir: None,
            dark: Some(1000),
        },
        ..attack(2)
    };
    store.record_attack(id, &partial).unwrap();
    let unreadable = Attack {
        loot: Loot::default(),
        ..attack(3)
    };
    store.record_attack(id, &unreadable).unwrap();
    let wall = WallUpgrade {
        ordinal: 1,
        village: Village::Home,
        completed_at_ms: 1100,
        resource: Resource::Gold,
        cost: 500_000,
    };
    store.record_wall(id, &wall).unwrap();
    store.record_wall(id, &wall).unwrap();
    store
        .observe(
            id,
            observation(
                1200,
                Reading::Known(Balance {
                    amount: 26_000_000,
                    capacity: Some(26_000_000),
                }),
            ),
        )
        .unwrap();
    store
        .update_session(id, SessionState::Paused, 1_800_000, 3_000_000)
        .unwrap();
    let stats = store.statistics(id, Village::Home).unwrap();
    assert_eq!(stats.attacks, 3);
    assert_eq!(stats.attacks_per_hour, Some(6.0));
    assert_eq!(stats.gold.amount, Some(1_000_000));
    assert_eq!(stats.gold.measured_attacks, 2);
    assert_eq!(stats.gold.average_per_attack, Some(500_000.0));
    assert_eq!(stats.gold.per_hour, Some(2_000_000.0));
    assert_eq!(stats.elixir.measured_attacks, 1);
    assert_eq!(stats.dark.amount, Some(1000));
    assert_eq!(stats.wall_upgrades, 1);
    assert_eq!(stats.wall_gold, 500_000);
    assert_eq!(
        store.statistics(id, Village::Builder).unwrap().gold.amount,
        None
    );
    assert_eq!(
        store.attacks(id, None, 2).unwrap(),
        vec![unreadable, partial]
    );
    assert_eq!(store.attacks(id, Some(2), 2).unwrap(), vec![attack(1)]);
    assert_eq!(store.walls(id, None, 10).unwrap(), vec![wall.clone()]);
    assert!(
        store
            .record_attack(
                id,
                &Attack {
                    loot: Loot::default(),
                    ..attack(1)
                }
            )
            .is_err()
    );
    assert!(
        store
            .record_wall(id, &WallUpgrade { cost: 1, ..wall })
            .is_err()
    );
    assert_eq!(store.statistics(id, Village::Home).unwrap(), stats);
}

#[test]
fn sessions_and_villages_cannot_mix_counts_or_resume_finished_work() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("history.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let first = store.begin_session(SessionMode::Observe, 100).unwrap();
    assert!(store.begin_session(SessionMode::Farming, 101).is_err());
    assert!(store.record_attack(first.id, &attack(1)).is_err());
    assert!(
        store
            .record_wall(
                first.id,
                &WallUpgrade {
                    ordinal: 1,
                    village: Village::Home,
                    completed_at_ms: 101,
                    resource: Resource::Gold,
                    cost: 1
                }
            )
            .is_err()
    );
    store
        .update_session(first.id, SessionState::Paused, 5, 105)
        .unwrap();
    assert!(
        store
            .update_session(first.id, SessionState::Running, 4, 106)
            .is_err()
    );
    store
        .update_session(first.id, SessionState::Stopped, 5, 106)
        .unwrap();
    assert!(
        store
            .update_session(first.id, SessionState::Running, 5, 107)
            .is_err()
    );
    let second = store.begin_session(SessionMode::Farming, 108).unwrap();
    assert!(second.id > first.id);
    assert!(
        store
            .observe(first.id, observation(200, Reading::Failed))
            .is_err()
    );
    store.record_attack(second.id, &attack(1)).unwrap();
    let mut builder = Attack {
        village: Village::Builder,
        damage: Some(200),
        stars: Some(6),
        ..attack(2)
    };
    builder.loot.gold = Some(123);
    store.record_attack(second.id, &builder).unwrap();
    assert_eq!(
        store.attacks(second.id, None, 1).unwrap(),
        vec![builder.clone()]
    );
    builder.ordinal = 3;
    builder.damage = Some(201);
    assert!(store.record_attack(second.id, &builder).is_err());
    builder.damage = Some(200);
    builder.loot.dark = Some(0);
    assert!(store.record_attack(second.id, &builder).is_err());
    assert_eq!(
        store.statistics(second.id, Village::Home).unwrap().attacks,
        1
    );
    assert_eq!(
        store
            .statistics(second.id, Village::Builder)
            .unwrap()
            .gold
            .amount,
        Some(123)
    );
    assert_eq!(
        store.statistics(first.id, Village::Home).unwrap().attacks,
        0
    );
    assert_eq!(
        store.sessions(None, 1).unwrap(),
        vec![store.session(second.id).unwrap()]
    );
    assert_eq!(store.sessions(Some(second.id), 1).unwrap()[0].id, first.id);
    assert!(store.sessions(None, 0).is_err());
    assert!(store.sessions(None, 101).is_err());
    drop(store);
    let mut reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.interrupt_open_sessions(2000).unwrap(), 1);
    assert_eq!(
        reopened.session(second.id).unwrap().state,
        SessionState::Interrupted
    );
    assert_eq!(reopened.interrupt_open_sessions(2001).unwrap(), 0);
    assert_eq!(
        reopened
            .statistics(second.id, Village::Home)
            .unwrap()
            .attacks,
        1
    );
    assert!(reopened.begin_session(SessionMode::Observe, 2002).is_ok());
}

#[test]
fn wall_sessions_include_farming_results_without_subtracting_upgrade_costs() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("history.sqlite3")).unwrap();
    let id = store.begin_session(SessionMode::Walls, 100).unwrap().id;
    store.record_attack(id, &attack(1)).unwrap();
    store
        .record_wall(
            id,
            &WallUpgrade {
                ordinal: 1,
                village: Village::Home,
                completed_at_ms: 1100,
                resource: Resource::Elixir,
                cost: 100_000,
            },
        )
        .unwrap();
    store.record_attack(id, &attack(2)).unwrap();
    let stats = store.statistics(id, Village::Home).unwrap();
    assert_eq!(stats.attacks, 2);
    assert_eq!(stats.elixir.amount, Some(1_000_000));
    assert_eq!(stats.wall_upgrades, 1);
    assert_eq!(stats.wall_elixir, 100_000);
}

#[test]
fn unreadable_foreign_and_newer_databases_are_not_reset() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("history.sqlite3");
    std::fs::write(&path, b"not a database").unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"not a database");
    let foreign = folder.path().join("foreign.sqlite3");
    let connection = Connection::open(&foreign).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE sqliteData (value TEXT); INSERT INTO sqliteData VALUES ('keep');",
        )
        .unwrap();
    assert!(Store::open(&foreign).is_err());
    assert_eq!(
        connection
            .query_row("SELECT value FROM sqliteData", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let future = folder.path().join("future.sqlite3");
    let mut store = Store::open(&future).unwrap();
    let id = store.begin_session(SessionMode::Observe, 100).unwrap().id;
    drop(store);
    let connection = Connection::open(&future).unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    assert!(Store::open(&future).is_err());
    assert_eq!(
        connection
            .query_row("SELECT id FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        id
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn failed_writes_roll_back_and_corrupt_records_fail_reads() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("history.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let id = store.begin_session(SessionMode::Farming, 100).unwrap().id;
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_attack AFTER INSERT ON attacks BEGIN SELECT RAISE(ABORT, 'disk write failed'); END;").unwrap();
    assert!(store.record_attack(id, &attack(1)).is_err());
    assert_eq!(store.statistics(id, Village::Home).unwrap().attacks, 0);
    connection
        .execute_batch("DROP TRIGGER fail_attack;")
        .unwrap();
    store.record_attack(id, &attack(1)).unwrap();
    connection
        .execute("UPDATE attacks SET data = '{}'", [])
        .unwrap();
    assert!(store.attacks(id, None, 10).is_err());
    assert_eq!(
        connection
            .query_row("SELECT data FROM attacks", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "{}"
    );
}

#[test]
fn activity_is_bounded_ordered_persistent_and_idempotent() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("history.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let id = store.begin_session(SessionMode::Observe, 100).unwrap().id;
    let mut activity = Activity {
        sequence: 1,
        at_ms: 200,
        kind: ActivityKind::RecoveryStarted,
        message: "Restarting the selected emulator".into(),
    };
    store.record_activity(id, &activity).unwrap();
    store.record_activity(id, &activity).unwrap();
    let first = activity.clone();
    activity.sequence = 2;
    activity.kind = ActivityKind::Error;
    activity.message = "Device disconnected".into();
    store.record_activity(id, &activity).unwrap();
    assert_eq!(store.activity(id, None, 1).unwrap(), vec![activity.clone()]);
    assert_eq!(store.activity(id, Some(2), 100).unwrap(), vec![first]);
    activity.message = "Conflicting message".into();
    assert!(store.record_activity(id, &activity).is_err());
    activity.sequence = 3;
    activity.message = "x".repeat(2049);
    assert!(store.record_activity(id, &activity).is_err());
    assert!(store.activity(id, None, 101).is_err());
    drop(store);
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .activity(id, None, 100)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn invalid_inputs_and_large_loot_never_wrap_or_become_fake_values() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("history.sqlite3")).unwrap();
    assert!(store.begin_session(SessionMode::Farming, -1).is_err());
    let id = store.begin_session(SessionMode::Farming, 100).unwrap().id;
    let mut result = attack(0);
    assert!(store.record_attack(id, &result).is_err());
    result.ordinal = 1;
    result.damage = Some(101);
    assert!(store.record_attack(id, &result).is_err());
    result.damage = Some(100);
    result.stars = Some(4);
    assert!(store.record_attack(id, &result).is_err());
    result.stars = Some(3);
    result.loot.gold = Some(u32::MAX);
    store.record_attack(id, &result).unwrap();
    result.ordinal = 2;
    store.record_attack(id, &result).unwrap();
    assert_eq!(
        store.statistics(id, Village::Home).unwrap().gold.amount,
        Some(i64::from(u32::MAX) * 2)
    );
    assert_eq!(
        store.statistics(id, Village::Home).unwrap().gold.per_hour,
        None
    );
    let bad = ResourceObservation {
        village: Village::Builder,
        resource: Resource::Dark,
        ..observation(200, Reading::Unavailable)
    };
    assert!(store.observe(id, bad).is_err());
    assert!(
        store
            .observe(id, observation(i64::MAX, Reading::Failed))
            .is_err()
    );
    assert!(
        store
            .update_session(id, SessionState::Running, i64::MAX, 200)
            .is_err()
    );
    assert!(
        store
            .record_wall(
                id,
                &WallUpgrade {
                    ordinal: 1,
                    village: Village::Home,
                    completed_at_ms: 200,
                    resource: Resource::Dark,
                    cost: 1
                }
            )
            .is_err()
    );
}
