//! Banco local (SQLite): sessões e as falas transcritas de cada uma.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::audio::Source;

/// Uma entrada por versão do esquema; a posição na lista é a versão.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE sessions (
        id INTEGER PRIMARY KEY,
        started_at INTEGER NOT NULL,          -- unix ms
        duration_ms INTEGER,                  -- NULL enquanto a sessão está aberta
        other_side TEXT,                      -- app da chamada ou arquivo de replay
        interrupted INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE segments (
        id INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        source TEXT NOT NULL,                 -- 'mic' | 'sistema'
        start_ms INTEGER NOT NULL,            -- relógio da sessão
        end_ms INTEGER NOT NULL,
        text TEXT NOT NULL,
        latency_ms INTEGER NOT NULL,
        echo INTEGER NOT NULL DEFAULT 0       -- fala do microfone descartada como eco
    );
    CREATE INDEX segments_by_session ON segments(session_id, start_ms);
"];

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: i64,
    pub source: Source,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub latency_ms: u64,
    pub echo: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: i64,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub other_side: Option<String>,
    /// O app fechou no meio da sessão; a duração é a da última fala.
    pub interrupted: bool,
    pub segments: i64,
}

fn err(e: rusqlite::Error) -> String {
    format!("banco de dados: {e}")
}

pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::init(Connection::open(path).map_err(err)?)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(mut conn: Connection) -> Result<Self, String> {
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(err)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(err)?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.transaction().map_err(err)?;
            tx.execute_batch(sql).map_err(err)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)
                .map_err(err)?;
            tx.commit().map_err(err)?;
        }
        // Sessões que ficaram abertas são de um app que fechou no meio.
        conn.execute(
            "UPDATE sessions SET interrupted = 1, duration_ms = COALESCE(
                 (SELECT MAX(end_ms) FROM segments WHERE session_id = sessions.id), 0)
             WHERE duration_ms IS NULL",
            [],
        )
        .map_err(err)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    pub fn create_session(&self, other_side: Option<&str>) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (started_at, other_side) VALUES (?1, ?2)",
            params![now_unix_ms(), other_side],
        )
        .map_err(err)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn finish_session(&self, id: i64, duration_ms: u64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET duration_ms = ?2 WHERE id = ?1",
                params![id, duration_ms as i64],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Grava a fala e devolve o id dela.
    pub fn insert_segment(&self, session_id: i64, s: &Segment) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO segments (session_id, source, start_ms, end_ms, text, latency_ms, echo)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                s.source.label(),
                s.start_ms as i64,
                s.end_ms as i64,
                s.text,
                s.latency_ms as i64,
                s.echo
            ],
        )
        .map_err(err)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.started_at, s.duration_ms, s.other_side, s.interrupted,
                        (SELECT COUNT(*) FROM segments g WHERE g.session_id = s.id AND g.echo = 0)
                 FROM sessions s ORDER BY s.started_at DESC",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SessionSummary {
                    id: r.get(0)?,
                    started_at: r.get(1)?,
                    duration_ms: r.get(2)?,
                    other_side: r.get(3)?,
                    interrupted: r.get(4)?,
                    segments: r.get(5)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Falas da sessão em ordem de tempo, incluindo as descartadas como eco.
    pub fn session_segments(&self, session_id: i64) -> Result<Vec<Segment>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, source, start_ms, end_ms, text, latency_ms, echo
                 FROM segments WHERE session_id = ?1 ORDER BY start_ms, id",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([session_id], |r| {
                let source: String = r.get(1)?;
                Ok(Segment {
                    id: r.get(0)?,
                    source: Source::from_label(&source).unwrap_or(Source::Sistema),
                    start_ms: r.get::<_, i64>(2)? as u64,
                    end_ms: r.get::<_, i64>(3)? as u64,
                    text: r.get(4)?,
                    latency_ms: r.get::<_, i64>(5)? as u64,
                    echo: r.get(6)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn delete_session(&self, id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM sessions WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }
}
