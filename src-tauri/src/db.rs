//! Banco local (SQLite): sessões, falas, kits, configurações e o que a análise produz.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Row, params};
use serde::Serialize;

use crate::audio::Source;

/// Uma entrada por versão do esquema; a posição na lista é a versão.
const MIGRATIONS: &[&str] = &[
    "
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
    ",
    "
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE kits (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        markdown TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    -- A sessão guarda uma cópia do kit: editar o kit depois não muda sessões antigas.
    ALTER TABLE sessions ADD COLUMN kit_name TEXT;
    ALTER TABLE sessions ADD COLUMN kit_markdown TEXT;
    ALTER TABLE sessions ADD COLUMN candidate_name TEXT;
    ALTER TABLE sessions ADD COLUMN candidate_resume TEXT;
    ALTER TABLE sessions ADD COLUMN candidate_checkpoints TEXT;
    ALTER TABLE sessions ADD COLUMN coverage_json TEXT;
    ALTER TABLE sessions ADD COLUMN summary TEXT;
    ALTER TABLE sessions ADD COLUMN notes_json TEXT;
    CREATE TABLE cards (
        id INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        t_ms INTEGER NOT NULL,
        kind TEXT NOT NULL,
        text TEXT NOT NULL,
        priority TEXT NOT NULL
    );
    CREATE TABLE marks (
        id INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        t_ms INTEGER NOT NULL,
        kind TEXT NOT NULL DEFAULT 'mark',    -- 'mark' | 'pause' | 'resume'
        note TEXT
    );
    CREATE TABLE llm_usage (
        id INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        purpose TEXT NOT NULL,                -- 'analise' | 'notas'
        model TEXT NOT NULL,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cached_tokens INTEGER NOT NULL,
        latency_ms INTEGER NOT NULL
    );
    ",
];

/// Versão do esquema que cria a tabela de kits (e recebe os kits de exemplo).
const KITS_VERSION: usize = 1;

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

#[derive(Debug, Default, Clone)]
pub struct NewSession {
    pub other_side: Option<String>,
    pub kit_name: Option<String>,
    pub kit_markdown: Option<String>,
    pub candidate_name: Option<String>,
    pub candidate_resume: Option<String>,
    pub candidate_checkpoints: Option<String>,
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
    pub kit_name: Option<String>,
    pub candidate_name: Option<String>,
    pub has_notes: bool,
}

/// Tudo de uma sessão, menos as falas.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub id: i64,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub other_side: Option<String>,
    pub interrupted: bool,
    pub kit_name: Option<String>,
    pub kit_markdown: Option<String>,
    pub candidate_name: Option<String>,
    pub candidate_resume: Option<String>,
    pub candidate_checkpoints: Option<String>,
    pub coverage_json: Option<String>,
    pub summary: Option<String>,
    pub notes_json: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KitRow {
    pub id: i64,
    pub name: String,
    pub markdown: String,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: i64,
    pub t_ms: u64,
    pub kind: String,
    pub text: String,
    pub priority: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub id: i64,
    pub t_ms: u64,
    pub kind: String,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
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

fn u64_col(r: &Row, i: usize) -> rusqlite::Result<u64> {
    Ok(r.get::<_, i64>(i)? as u64)
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
            if i == KITS_VERSION {
                for markdown in crate::kit::EXAMPLES {
                    tx.execute(
                        "INSERT INTO kits (name, markdown, updated_at) VALUES (?1, ?2, ?3)",
                        params![crate::kit::parse(markdown).title, markdown, now_unix_ms()],
                    )
                    .map_err(err)?;
                }
            }
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

    // Sessões

    pub fn create_session(&self, new: &NewSession) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (started_at, other_side, kit_name, kit_markdown,
                                   candidate_name, candidate_resume, candidate_checkpoints)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                now_unix_ms(),
                new.other_side,
                new.kit_name,
                new.kit_markdown,
                new.candidate_name,
                new.candidate_resume,
                new.candidate_checkpoints
            ],
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

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.started_at, s.duration_ms, s.other_side, s.interrupted,
                        (SELECT COUNT(*) FROM segments g WHERE g.session_id = s.id AND g.echo = 0),
                        s.kit_name, s.candidate_name, s.notes_json IS NOT NULL
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
                    kit_name: r.get(6)?,
                    candidate_name: r.get(7)?,
                    has_notes: r.get(8)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn session_detail(&self, id: i64) -> Result<SessionDetail, String> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT id, started_at, duration_ms, other_side, interrupted, kit_name, kit_markdown,
                        candidate_name, candidate_resume, candidate_checkpoints, coverage_json,
                        summary, notes_json
                 FROM sessions WHERE id = ?1",
                [id],
                |r| {
                    Ok(SessionDetail {
                        id: r.get(0)?,
                        started_at: r.get(1)?,
                        duration_ms: r.get(2)?,
                        other_side: r.get(3)?,
                        interrupted: r.get(4)?,
                        kit_name: r.get(5)?,
                        kit_markdown: r.get(6)?,
                        candidate_name: r.get(7)?,
                        candidate_resume: r.get(8)?,
                        candidate_checkpoints: r.get(9)?,
                        coverage_json: r.get(10)?,
                        summary: r.get(11)?,
                        notes_json: r.get(12)?,
                    })
                },
            )
            .map_err(err)
    }

    pub fn save_coverage(
        &self,
        session_id: i64,
        coverage_json: &str,
        summary: &str,
    ) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET coverage_json = ?2, summary = ?3 WHERE id = ?1",
                params![session_id, coverage_json, summary],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn save_notes(&self, session_id: i64, notes_json: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET notes_json = ?2 WHERE id = ?1",
                params![session_id, notes_json],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn delete_session(&self, id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM sessions WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }

    /// Apaga a transcrição e o que deriva dela, mantendo as notas revisadas.
    pub fn delete_transcript(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM segments WHERE session_id = ?1", [id])
            .map_err(err)?;
        conn.execute("DELETE FROM cards WHERE session_id = ?1", [id])
            .map_err(err)?;
        conn.execute("UPDATE sessions SET summary = NULL WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }

    /// Apaga a transcrição de sessões com notas, mais antigas que `days` dias.
    /// Devolve quantas sessões foram afetadas.
    pub fn purge_old_transcripts(&self, days: i64) -> Result<usize, String> {
        let cutoff = now_unix_ms() - days * 24 * 3600 * 1000;
        let ids: Vec<i64> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM sessions s WHERE started_at < ?1 AND notes_json IS NOT NULL
                     AND EXISTS (SELECT 1 FROM segments g WHERE g.session_id = s.id)",
                )
                .map_err(err)?;
            let rows = stmt.query_map([cutoff], |r| r.get(0)).map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        for id in &ids {
            self.delete_transcript(*id)?;
        }
        Ok(ids.len())
    }

    pub fn delete_everything(&self) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute_batch("DELETE FROM sessions; VACUUM;")
            .map_err(err)
    }

    // Falas

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
                    start_ms: u64_col(r, 2)?,
                    end_ms: u64_col(r, 3)?,
                    text: r.get(4)?,
                    latency_ms: u64_col(r, 5)?,
                    echo: r.get(6)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    // Cards, marcações e uso

    pub fn insert_card(
        &self,
        session_id: i64,
        t_ms: u64,
        kind: &str,
        text: &str,
        priority: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cards (session_id, t_ms, kind, text, priority) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, t_ms as i64, kind, text, priority],
        )
        .map_err(err)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn session_cards(&self, session_id: i64) -> Result<Vec<Card>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, t_ms, kind, text, priority FROM cards WHERE session_id = ?1 ORDER BY t_ms, id",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([session_id], |r| {
                Ok(Card {
                    id: r.get(0)?,
                    t_ms: u64_col(r, 1)?,
                    kind: r.get(2)?,
                    text: r.get(3)?,
                    priority: r.get(4)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn insert_mark(
        &self,
        session_id: i64,
        t_ms: u64,
        kind: &str,
        note: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO marks (session_id, t_ms, kind, note) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, t_ms as i64, kind, note],
        )
        .map_err(err)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn session_marks(&self, session_id: i64) -> Result<Vec<Mark>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, t_ms, kind, note FROM marks WHERE session_id = ?1 ORDER BY t_ms, id",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([session_id], |r| {
                Ok(Mark {
                    id: r.get(0)?,
                    t_ms: u64_col(r, 1)?,
                    kind: r.get(2)?,
                    note: r.get(3)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn insert_usage(
        &self,
        session_id: i64,
        purpose: &str,
        u: &crate::llm::Usage,
    ) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO llm_usage (session_id, purpose, model, input_tokens, output_tokens,
                                        cached_tokens, latency_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_id,
                    purpose,
                    u.model,
                    u.input_tokens as i64,
                    u.output_tokens as i64,
                    u.cached_tokens as i64,
                    u.latency_ms as i64
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn session_usage(&self, session_id: i64) -> Result<UsageTotals, String> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cached_tokens), 0)
                 FROM llm_usage WHERE session_id = ?1",
                [session_id],
                |r| {
                    Ok(UsageTotals {
                        calls: r.get(0)?,
                        input_tokens: r.get(1)?,
                        output_tokens: r.get(2)?,
                        cached_tokens: r.get(3)?,
                    })
                },
            )
            .map_err(err)
    }

    // Configurações

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        }) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(err(e)),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(err)?;
        Ok(())
    }

    // Kits

    pub fn list_kits(&self) -> Result<Vec<KitRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, markdown, updated_at FROM kits ORDER BY name COLLATE NOCASE")
            .map_err(err)?;
        let rows = stmt.query_map([], kit_row).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn get_kit(&self, id: i64) -> Result<KitRow, String> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT id, name, markdown, updated_at FROM kits WHERE id = ?1",
                [id],
                kit_row,
            )
            .map_err(err)
    }

    /// Cria (sem `id`) ou atualiza um kit. O nome vem do título do Markdown.
    pub fn save_kit(&self, id: Option<i64>, markdown: &str) -> Result<i64, String> {
        let mut name = crate::kit::parse(markdown).title;
        if name.is_empty() {
            name = "Kit sem título".into();
        }
        let conn = self.conn.lock().unwrap();
        match id {
            Some(id) => {
                conn.execute(
                    "UPDATE kits SET name = ?2, markdown = ?3, updated_at = ?4 WHERE id = ?1",
                    params![id, name, markdown, now_unix_ms()],
                )
                .map_err(err)?;
                Ok(id)
            }
            None => {
                conn.execute(
                    "INSERT INTO kits (name, markdown, updated_at) VALUES (?1, ?2, ?3)",
                    params![name, markdown, now_unix_ms()],
                )
                .map_err(err)?;
                Ok(conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_kit(&self, id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM kits WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }
}

fn kit_row(r: &Row) -> rusqlite::Result<KitRow> {
    Ok(KitRow {
        id: r.get(0)?,
        name: r.get(1)?,
        markdown: r.get(2)?,
        updated_at: r.get(3)?,
    })
}
