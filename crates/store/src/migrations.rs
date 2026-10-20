use crate::{Error, Result};
use rusqlite::{Connection, TransactionBehavior};

pub const SCHEMA_VERSION: u32 = 3;
const APPLICATION_ID: u32 = 0x4d59_4152;

pub(crate) struct Migration {
    pub version: u32,
    pub sql: &'static str,
}
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../migrations/0001_sessions.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../migrations/0002_context_leases.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../migrations/0003_indexes.sql"),
    },
];

pub(crate) fn migrate(connection: &mut Connection, migrations: &[Migration]) -> Result<()> {
    for (index, migration) in migrations.iter().enumerate() {
        if migration.version as usize != index + 1 {
            return Err(Error::MigrationSequence);
        }
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let app: u32 = tx.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if version > migrations.last().map_or(0, |m| m.version) {
        return Err(Error::FutureSchema);
    }
    if app != APPLICATION_ID {
        let tables: u32 = tx.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if app != 0 || version != 0 || tables != 0 {
            return Err(Error::ForeignDatabase);
        }
    }
    for migration in migrations.iter().filter(|m| m.version > version) {
        tx.execute_batch(migration.sql)?;
        tx.pragma_update(None, "user_version", migration.version)?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_sequence_is_atomic_idempotent_and_rejects_future_versions() {
        let mut db = Connection::open_in_memory().unwrap();
        let steps = [Migration {
            version: 1,
            sql: "CREATE TABLE fixture (id TEXT);",
        }];
        migrate(&mut db, &steps).unwrap();
        migrate(&mut db, &steps).unwrap();
        let broken = [
            Migration {
                version: 1,
                sql: "",
            },
            Migration {
                version: 2,
                sql: "CREATE TABLE rollback_fixture (id TEXT);",
            },
            Migration {
                version: 3,
                sql: "INSERT INTO missing_fixture VALUES (1);",
            },
        ];
        assert!(migrate(&mut db, &broken).is_err());
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='rollback_fixture'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            0
        );
        db.pragma_update(None, "user_version", 9).unwrap();
        assert!(matches!(
            migrate(&mut db, MIGRATIONS),
            Err(Error::FutureSchema)
        ));
        assert!(matches!(
            migrate(
                &mut db,
                &[Migration {
                    version: 2,
                    sql: ""
                }]
            ),
            Err(Error::MigrationSequence)
        ));
    }

    #[test]
    fn upgrade_preserves_history_and_constraints_protect_storage() {
        let mut db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&mut db, &MIGRATIONS[..1]).unwrap();
        db.execute("INSERT INTO sessions VALUES ('s','local-user','fixture','/fixture','/fixture',NULL,0,0)", []).unwrap();
        db.execute("INSERT INTO messages VALUES ('m','s',NULL,'ASSISTANT','COMPLETE','fixture','BUILD','preserved','[]',NULL,NULL,0)", []).unwrap();
        migrate(&mut db, MIGRATIONS).unwrap();
        assert_eq!(
            db.query_row("SELECT content FROM messages", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        assert!(
            db.execute("UPDATE messages SET role='unknown'", [])
                .is_err()
        );
        assert!(
            db.execute("UPDATE messages SET parts_json='invalid'", [])
                .is_err()
        );
        assert!(
            db.execute(
                "INSERT INTO session_leases VALUES ('missing','owner',100)",
                []
            )
            .is_err()
        );
        db.execute(
            "INSERT INTO session_contexts VALUES ('s','[]',16384,0,0,0)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO session_leases VALUES ('s','owner',100)", [])
            .unwrap();
        db.execute("DELETE FROM sessions WHERE id='s'", []).unwrap();
        for table in ["messages", "session_contexts", "session_leases"] {
            let count = db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, u32>(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn unrelated_database_is_not_adopted() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE unrelated (data TEXT);")
            .unwrap();
        assert!(matches!(
            migrate(&mut db, MIGRATIONS),
            Err(Error::ForeignDatabase)
        ));
    }
}
