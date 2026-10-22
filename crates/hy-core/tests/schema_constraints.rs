//! The database itself refuses rows that break the schema, independent of the Rust API.
//! Each case inserts one bad row with raw SQL into a freshly migrated database.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use hy_core::migrations;
use rusqlite::{Connection, params};

const DEV: &str = "0199f0a0-0000-7000-8000-000000000001";
const ID1: &str = "0199f0a0-0000-7000-8000-0000000000a1";
const ID2: &str = "0199f0a0-0000-7000-8000-0000000000a2";

fn db() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    migrations::migrate(&mut conn).unwrap();
    conn
}

/// Column values for an `events` row; tests override one at a time.
#[derive(Clone)]
struct Ev {
    id: &'static str,
    schema_version: i64,
    type_name: &'static str,
    zone: &'static str,
    local_date: &'static str,
    source: &'static str,
    source_ref: Option<&'static str>,
    status: &'static str,
    confidence: Option<f64>,
    supersedes: Option<&'static str>,
    origin: &'static str,
    payload: &'static str,
}

impl Default for Ev {
    fn default() -> Self {
        Self {
            id: ID1,
            schema_version: 1,
            type_name: "water.logged",
            zone: "Asia/Kolkata",
            local_date: "2026-10-21",
            source: "manual",
            source_ref: None,
            status: "self_reported",
            confidence: None,
            supersedes: None,
            origin: DEV,
            payload: r#"{"amount_ml":250}"#,
        }
    }
}

fn insert_event(conn: &Connection, e: &Ev) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO events (id, schema_version, type, occurred_at, recorded_at, zone, local_date,
           source, source_ref, status, confidence, supersedes, origin_device, payload)
         VALUES (?1, ?2, ?3, 0, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            e.id,
            e.schema_version,
            e.type_name,
            e.zone,
            e.local_date,
            e.source,
            e.source_ref,
            e.status,
            e.confidence,
            e.supersedes,
            e.origin,
            e.payload
        ],
    )
}

#[test]
fn a_valid_event_row_is_accepted() {
    let conn = db();
    insert_event(&conn, &Ev::default()).unwrap();
    // The reserved V3 source is accepted by the column; the Rust layer refuses it.
    let imported = Ev {
        id: ID2,
        source: "imported",
        status: "measured",
        source_ref: Some("watch:1"),
        ..Ev::default()
    };
    insert_event(&conn, &imported).unwrap();
}

#[test]
fn event_column_checks() {
    let bad_rows = vec![
        (
            "short id",
            Ev {
                id: "abc",
                ..Ev::default()
            },
        ),
        (
            "schema version 0",
            Ev {
                schema_version: 0,
                ..Ev::default()
            },
        ),
        (
            "type without family",
            Ev {
                type_name: "water",
                ..Ev::default()
            },
        ),
        (
            "upper-case type",
            Ev {
                type_name: "Water.Logged",
                ..Ev::default()
            },
        ),
        (
            "empty zone",
            Ev {
                zone: "",
                ..Ev::default()
            },
        ),
        (
            "bad local date",
            Ev {
                local_date: "21/10/2026",
                ..Ev::default()
            },
        ),
        (
            "unknown source",
            Ev {
                source: "guess",
                ..Ev::default()
            },
        ),
        (
            "source_ref without imported",
            Ev {
                source_ref: Some("x"),
                ..Ev::default()
            },
        ),
        (
            "unknown status",
            Ev {
                status: "maybe",
                ..Ev::default()
            },
        ),
        (
            "confidence above 1",
            Ev {
                confidence: Some(1.01),
                ..Ev::default()
            },
        ),
        (
            "confidence below 0",
            Ev {
                confidence: Some(-0.1),
                ..Ev::default()
            },
        ),
        (
            "self supersede",
            Ev {
                supersedes: Some(ID1),
                ..Ev::default()
            },
        ),
        (
            "dangling supersede",
            Ev {
                supersedes: Some(ID2),
                ..Ev::default()
            },
        ),
        (
            "short origin",
            Ev {
                origin: "pc-1",
                ..Ev::default()
            },
        ),
        (
            "payload not json",
            Ev {
                payload: "amount=250",
                ..Ev::default()
            },
        ),
        (
            "payload array",
            Ev {
                payload: "[250]",
                ..Ev::default()
            },
        ),
    ];
    for (why, row) in bad_rows {
        let conn = db();
        assert!(insert_event(&conn, &row).is_err(), "accepted: {why}");
    }
}

#[test]
fn an_event_is_superseded_at_most_once() {
    let conn = db();
    insert_event(&conn, &Ev::default()).unwrap();
    insert_event(
        &conn,
        &Ev {
            id: ID2,
            supersedes: Some(ID1),
            ..Ev::default()
        },
    )
    .unwrap();
    let third = Ev {
        id: "0199f0a0-0000-7000-8000-0000000000a3",
        supersedes: Some(ID1),
        ..Ev::default()
    };
    assert!(insert_event(&conn, &third).is_err());
}

#[test]
fn a_correction_keeps_the_entry_type_and_a_retraction_is_final() {
    let conn = db();
    let id3 = "0199f0a0-0000-7000-8000-0000000000a3";
    let id4 = "0199f0a0-0000-7000-8000-0000000000a4";
    insert_event(&conn, &Ev::default()).unwrap();
    // Another type does not correct a water entry.
    let phone = Ev {
        id: ID2,
        type_name: "phone.interruption",
        payload: r#"{"label":"work"}"#,
        supersedes: Some(ID1),
        ..Ev::default()
    };
    let err = insert_event(&conn, &phone).unwrap_err().to_string();
    assert!(err.contains("keeps the entry's type"), "{err}");
    // A retraction does.
    let retraction = Ev {
        id: ID2,
        type_name: "entry.retracted",
        payload: "{}",
        supersedes: Some(ID1),
        ..Ev::default()
    };
    insert_event(&conn, &retraction).unwrap();
    // Nothing supersedes a retraction: neither a correction nor another retraction.
    for (id, type_name, payload) in [
        (id3, "entry.retracted", "{}"),
        (id4, "water.logged", r#"{"amount_ml":250}"#),
    ] {
        let e = Ev {
            id,
            type_name,
            payload,
            supersedes: Some(ID2),
            ..Ev::default()
        };
        assert!(insert_event(&conn, &e).is_err(), "{type_name}");
    }
}

#[test]
fn a_guided_routine_has_one_result_and_each_step_once_before_it() {
    let conn = db();
    let step = |id, i: u32, session: &str| {
        Ev {
        id,
        type_name: "movement.step",
        payload: Box::leak(
            format!(
                r#"{{"session_id":"{session}","routine_id":"tue","routine_version":1,"variant":"seated","step_index":{i},"steps_total":3,"outcome":"done"}}"#
            )
            .into_boxed_str(),
        ),
        ..Ev::default()
    }
    };
    let result = |id, session: &str, supersedes| {
        Ev {
        id,
        type_name: "movement.session",
        supersedes,
        payload: Box::leak(
            format!(
                r#"{{"session_id":"{session}","routine_id":"tue","routine_version":1,"variant":"seated","outcome":"partial","steps_done":1,"steps_skipped":0,"steps_total":3}}"#
            )
            .into_boxed_str(),
        ),
        ..Ev::default()
    }
    };
    const S1: &str = "0199f0a0-0000-7000-8000-0000000000b1";
    const S2: &str = "0199f0a0-0000-7000-8000-0000000000b2";
    let id = |n: u8| -> &'static str {
        Box::leak(format!("0199f0a0-0000-7000-8000-0000000000{n:02x}").into_boxed_str())
    };
    insert_event(&conn, &step(id(1), 0, S1)).unwrap();
    // The same step twice.
    assert!(insert_event(&conn, &step(id(2), 0, S1)).is_err());
    insert_event(&conn, &step(id(3), 1, S1)).unwrap();
    insert_event(&conn, &result(id(4), S1, None)).unwrap();
    // A second result, and a step after the result.
    assert!(insert_event(&conn, &result(id(5), S1, None)).is_err());
    assert!(insert_event(&conn, &step(id(6), 2, S1)).is_err());
    // A correction (an event superseding the result) is not a second result.
    insert_event(&conn, &result(id(7), S1, Some(id(4)))).unwrap();
    // Another routine is independent.
    insert_event(&conn, &step(id(8), 0, S2)).unwrap();
    insert_event(&conn, &result(id(9), S2, None)).unwrap();
}

#[test]
fn deleting_a_superseded_event_clears_the_link_only() {
    let conn = db();
    insert_event(&conn, &Ev::default()).unwrap();
    insert_event(
        &conn,
        &Ev {
            id: ID2,
            supersedes: Some(ID1),
            ..Ev::default()
        },
    )
    .unwrap();
    conn.execute("DELETE FROM events WHERE id = ?1", [ID1])
        .unwrap();
    let link: Option<String> = conn
        .query_row("SELECT supersedes FROM events WHERE id = ?1", [ID2], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(link, None);
}

fn exec(conn: &Connection, sql: &str) -> rusqlite::Result<usize> {
    conn.execute(sql, [])
}

#[test]
fn summary_checks() {
    let conn = db();
    let ok = format!(
        "INSERT INTO daily_summary VALUES ('hydration', '{DEV}', '2026-10-21', 1, 0.5, '{{}}', 0)"
    );
    exec(&conn, &ok).unwrap();
    for (why, sql) in [
        (
            "unknown category",
            format!(
                "INSERT INTO daily_summary VALUES ('steps', '{DEV}', '2026-10-22', 1, NULL, '{{}}', 0)"
            ),
        ),
        (
            "revision 0",
            format!(
                "INSERT INTO daily_summary VALUES ('hydration', '{DEV}', '2026-10-22', 0, NULL, '{{}}', 0)"
            ),
        ),
        (
            "coverage 2",
            format!(
                "INSERT INTO daily_summary VALUES ('hydration', '{DEV}', '2026-10-22', 1, 2.0, '{{}}', 0)"
            ),
        ),
        (
            "payload text",
            format!(
                "INSERT INTO daily_summary VALUES ('hydration', '{DEV}', '2026-10-22', 1, NULL, '\"x\"', 0)"
            ),
        ),
        ("duplicate key", ok.clone()),
        (
            "bad month",
            format!(
                "INSERT INTO monthly_summary VALUES ('hydration', '{DEV}', '2026-10-01', 1, NULL, '{{}}', 0)"
            ),
        ),
        (
            "bad provenance",
            format!(
                "INSERT INTO imported_summary VALUES ('hydration', '{DEV}', '2026-10-21', 1, NULL, '{{}}', 'email', 0)"
            ),
        ),
    ] {
        assert!(exec(&conn, &sql).is_err(), "accepted: {why}");
    }
    // Revisions only increase.
    assert!(
        exec(
            &conn,
            "UPDATE daily_summary SET revision = 1, payload = '{\"ml\":1}'"
        )
        .is_err()
    );
    exec(&conn, "UPDATE daily_summary SET revision = 2").unwrap();
    exec(
        &conn,
        &format!("INSERT INTO monthly_summary VALUES ('outcomes', '{DEV}', '2026-10', 3, NULL, '{{}}', 0)"),
    )
    .unwrap();
    assert!(exec(&conn, "UPDATE monthly_summary SET revision = 2").is_err());
}

#[test]
fn goal_setting_and_pet_checks() {
    let conn = db();
    exec(
        &conn,
        "INSERT INTO goals_history VALUES ('water_daily_ml', '2026-10-21', 2000, 0)",
    )
    .unwrap();
    for (why, sql) in [
        (
            "unknown goal",
            "INSERT INTO goals_history VALUES ('steps', '2026-10-21', 2000, 0)",
        ),
        (
            "zero goal",
            "INSERT INTO goals_history VALUES ('water_daily_ml', '2026-10-22', 0, 0)",
        ),
        (
            "setting not json",
            "INSERT INTO settings VALUES ('k', 'not json', 0)",
        ),
        (
            "empty setting key",
            "INSERT INTO settings VALUES ('', '1', 0)",
        ),
    ] {
        assert!(exec(&conn, sql).is_err(), "accepted: {why}");
    }
    let pet = |id: i64,
               name: &str,
               scale: i64,
               colour: i64,
               edge: &str,
               x: Option<i64>,
               y: Option<i64>,
               motion: i64,
               tone: &str| {
        conn.execute(
            "INSERT INTO pet_profile VALUES (?1, 'turtle', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            params![id, name, scale, colour, edge, x, y, motion, tone],
        )
    };
    assert!(
        pet(2, "T", 100, 0, "left", None, None, 1, "plain").is_err(),
        "second row"
    );
    assert!(
        pet(1, "", 100, 0, "left", None, None, 1, "plain").is_err(),
        "empty name"
    );
    assert!(
        pet(1, &"n".repeat(21), 100, 0, "left", None, None, 1, "plain").is_err(),
        "long name"
    );
    assert!(
        pet(1, "T", 74, 0, "left", None, None, 1, "plain").is_err(),
        "too small"
    );
    assert!(
        pet(1, "T", 151, 0, "left", None, None, 1, "plain").is_err(),
        "too big"
    );
    assert!(
        pet(1, "T", 100, 3, "left", None, None, 1, "plain").is_err(),
        "colour"
    );
    assert!(
        pet(1, "T", 100, 0, "top", None, None, 1, "plain").is_err(),
        "edge"
    );
    assert!(
        pet(1, "T", 100, 0, "free", Some(1), None, 1, "plain").is_err(),
        "half position"
    );
    assert!(
        pet(1, "T", 100, 0, "left", None, None, 2, "plain").is_err(),
        "motion"
    );
    assert!(
        pet(1, "T", 100, 0, "left", None, None, 1, "loud").is_err(),
        "tone"
    );
    pet(
        1,
        "Śrī tortoise twenty",
        150,
        2,
        "free",
        Some(-5),
        Some(9),
        0,
        "cheerful",
    )
    .unwrap();
}

#[test]
fn sharing_policy_and_operation_checks() {
    let conn = db();
    let off: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sharing_policy WHERE enabled = 0 AND share_from IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(off, 5, "every category is seeded and off");
    assert!(
        exec(
            &conn,
            "UPDATE sharing_policy SET enabled = 1 WHERE category = 'hydration'"
        )
        .is_err(),
        "enabled without share_from"
    );
    assert!(
        exec(
            &conn,
            "INSERT INTO sharing_policy VALUES ('wearable_summary', 0, NULL, 0, 0)"
        )
        .is_err()
    );
    assert!(exec(&conn, "UPDATE sharing_policy SET seq = -1").is_err());
    let op = |id: &str, local_seq: i64, kind: &str, state: &str, seq: Option<i64>| {
        conn.execute(
            "INSERT INTO policy_ops VALUES (?1, ?2, 'hydration', ?3, 0, ?4, ?5, 0, 0)",
            params![id, local_seq, kind, state, seq],
        )
    };
    assert!(op(ID1, 1, "toggle", "pending", None).is_err());
    assert!(op(ID1, 1, "enable", "done", None).is_err());
    assert!(
        op(ID1, 1, "enable", "accepted", None).is_err(),
        "accepted needs a server seq"
    );
    assert!(op(ID1, 1, "enable", "accepted", Some(0)).is_err());
    assert!(
        op(ID1, 0, "enable", "pending", None).is_err(),
        "local order starts at 1"
    );
    op(ID1, 1, "enable", "accepted", Some(3)).unwrap();
    assert!(
        op(ID2, 1, "disable", "pending", None).is_err(),
        "local order is unique"
    );
    // The local order and identity of an operation never change; its state can.
    assert!(exec(&conn, "UPDATE policy_ops SET local_seq = 9").is_err());
    assert!(exec(&conn, "UPDATE policy_ops SET kind = 'disable'").is_err());
    exec(
        &conn,
        "UPDATE policy_ops SET state = 'accepted', server_seq = 4",
    )
    .unwrap();
}

#[test]
fn policy_operation_counter_only_increases() {
    let conn = db();
    let value = |conn: &Connection| -> String {
        conn.query_row(
            "SELECT value FROM store_meta WHERE key = 'policy_op_seq'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(value(&conn), "0");
    exec(
        &conn,
        "UPDATE store_meta SET value = '5' WHERE key = 'policy_op_seq'",
    )
    .unwrap();
    assert!(
        exec(
            &conn,
            "UPDATE store_meta SET value = '4' WHERE key = 'policy_op_seq'"
        )
        .is_err()
    );
    assert!(
        exec(
            &conn,
            "UPDATE store_meta SET value = '5' WHERE key = 'policy_op_seq'"
        )
        .is_err()
    );
    assert!(
        exec(
            &conn,
            "UPDATE store_meta SET key = 'other' WHERE key = 'policy_op_seq'"
        )
        .is_err()
    );
    assert!(exec(&conn, "DELETE FROM store_meta WHERE key = 'policy_op_seq'").is_err());
    assert_eq!(value(&conn), "5");
}

#[test]
fn tombstone_and_consent_checks() {
    let conn = db();
    let tomb = |id: &str, cat: &str, from: &str, to: &str, state: &str| {
        conn.execute(
            "INSERT INTO tombstones VALUES (?1, ?2, ?3, ?4, ?5, NULL, 0)",
            params![id, cat, from, to, state],
        )
    };
    assert!(
        tomb(ID1, "hydration", "2026-10-16", "2026-10-12", "pending").is_err(),
        "reversed range"
    );
    assert!(
        tomb(ID1, "hydration", "2026-10-12", "2026-10-16", "gone").is_err(),
        "state"
    );
    assert!(
        tomb(ID1, "steps", "2026-10-12", "2026-10-16", "pending").is_err(),
        "category"
    );
    assert!(
        tomb("x", "hydration", "2026-10-12", "2026-10-16", "pending").is_err(),
        "id"
    );
    tomb(ID1, "hydration", "2026-10-12", "2026-10-16", "pending").unwrap();
    assert!(exec(&conn, "DELETE FROM tombstones").is_err());
    assert!(exec(&conn, "UPDATE tombstones SET category = 'outcomes'").is_err());

    assert!(
        exec(
            &conn,
            "INSERT INTO consent_log (at, kind) VALUES (0, 'agreed')"
        )
        .is_err()
    );
    assert!(
        exec(
            &conn,
            "INSERT INTO consent_log (at, kind, op_ref) VALUES (0, 'enabled', 'x')"
        )
        .is_err()
    );
    exec(&conn, "INSERT INTO consent_log (at, kind, category, notice_version) VALUES (0, 'notice_shown', 'hydration', 'n1')").unwrap();
    assert!(exec(&conn, "UPDATE consent_log SET notice_version = 'n2'").is_err());
}

#[test]
fn outbox_needs_a_device_identity_and_an_enabled_category() {
    let conn = db();
    // No device id yet: the origin check refuses everything.
    exec(&conn, "UPDATE sharing_policy SET enabled = 1, share_from = '2026-10-01' WHERE category = 'hydration'").unwrap();
    let insert = || {
        exec(
            &conn,
            &format!(
                "INSERT INTO outbox (category, origin_device, local_date, revision, payload, created_at) VALUES ('hydration', '{DEV}', '2026-10-21', 1, '{{}}', 0)"
            ),
        )
    };
    assert!(insert().is_err());
    exec(
        &conn,
        &format!("INSERT INTO store_meta VALUES ('device_id', '{DEV}')"),
    )
    .unwrap();
    insert().unwrap();
    assert!(insert().is_err(), "duplicate revision");
}

#[test]
fn strict_tables_refuse_wrong_types() {
    let conn = db();
    assert!(
        exec(
            &conn,
            "INSERT INTO goals_history VALUES ('water_daily_ml', '2026-10-21', 'lots', 0)"
        )
        .is_err()
    );
    assert!(exec(&conn, &format!("INSERT INTO daily_summary VALUES ('hydration', '{DEV}', '2026-10-21', 'one', NULL, '{{}}', 0)")).is_err());
}
