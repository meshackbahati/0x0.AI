use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::util::now_utc_rfc3339;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
    pub root_path: String,
    pub category: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: i64,
    pub session_id: String,
    pub ts: String,
    pub action_type: String,
    pub command: String,
    pub target: Option<String>,
    pub status: String,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub session_id: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub sha256: Option<String>,
    pub mime: Option<String>,
    pub indexed_at: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisRecord {
    pub id: i64,
    pub session_id: String,
    pub text: String,
    pub confidence: f32,
    pub status: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteRecord {
    pub id: i64,
    pub session_id: String,
    pub ts: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationRecord {
    pub id: i64,
    pub session_id: String,
    pub source_type: String,
    pub source: String,
    pub locator: Option<String>,
    pub snippet: String,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebCacheRecord {
    pub url: String,
    pub title: Option<String>,
    pub content: String,
    pub fetched_at: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStats {
    pub sessions: i64,
    pub actions: i64,
    pub artifacts: i64,
    pub hypotheses: i64,
    pub notes: i64,
    pub citations: i64,
    pub web_cache: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
    pub root_path: String,
    pub category: Option<String>,
    pub summary: Option<String>,
    pub action_count: i64,
    pub artifact_count: i64,
    pub hypothesis_count: i64,
    pub note_count: i64,
    pub citation_count: i64,
}

pub struct StateStore {
    conn: Connection,
    max_actions_per_session: usize,
    max_artifacts_per_session: usize,
    max_cache_entries: usize,
}

impl StateStore {
    pub fn open(
        path: &Path,
        max_actions_per_session: usize,
        max_artifacts_per_session: usize,
        max_cache_entries: usize,
    ) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite db {}", path.display()))?;
        let store = Self {
            conn,
            max_actions_per_session,
            max_artifacts_per_session,
            max_cache_entries,
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA foreign_keys=ON;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                status TEXT NOT NULL,
                root_path TEXT NOT NULL,
                category TEXT,
                summary TEXT
            );

            CREATE TABLE IF NOT EXISTS actions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                ts TEXT NOT NULL,
                action_type TEXT NOT NULL,
                command TEXT NOT NULL,
                target TEXT,
                status TEXT NOT NULL,
                stdout TEXT,
                stderr TEXT,
                metadata_json TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_actions_session_ts ON actions(session_id, id DESC);

            CREATE TABLE IF NOT EXISTS artifacts (
                session_id TEXT NOT NULL,
                path TEXT NOT NULL,
                kind TEXT NOT NULL,
                size INTEGER NOT NULL,
                sha256 TEXT,
                mime TEXT,
                indexed_at TEXT NOT NULL,
                summary TEXT,
                PRIMARY KEY(session_id, path),
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS hypotheses (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                text TEXT NOT NULL,
                confidence REAL NOT NULL,
                status TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_hyp_session ON hypotheses(session_id, confidence DESC);

            CREATE TABLE IF NOT EXISTS notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                ts TEXT NOT NULL,
                note TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS citations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source TEXT NOT NULL,
                locator TEXT,
                snippet TEXT NOT NULL,
                ts TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_citations_session ON citations(session_id, id DESC);

            CREATE TABLE IF NOT EXISTS web_cache (
                url TEXT PRIMARY KEY,
                title TEXT,
                content TEXT NOT NULL,
                fetched_at TEXT NOT NULL,
                hash TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_web_cache_fetched ON web_cache(fetched_at DESC);
            "#,
        )?;
        Ok(())
    }

    pub fn create_session(&self, session_id: &str, root_path: &str) -> Result<()> {
        let now = now_utc_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions(id, created_at, updated_at, status, root_path) VALUES (?1, ?2, ?2, 'active', ?3)",
            params![session_id, now, root_path],
        )?;
        Ok(())
    }

    pub fn touch_session(
        &self,
        session_id: &str,
        status: Option<&str>,
        category: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let now = now_utc_rfc3339();
        self.conn.execute(
            "UPDATE sessions SET updated_at=?2,
                status=COALESCE(?3, status),
                category=COALESCE(?4, category),
                summary=COALESCE(?5, summary)
             WHERE id=?1",
            params![session_id, now, status, category, summary],
        )?;
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        self.conn
            .query_row(
                "SELECT id, created_at, updated_at, status, root_path, category, summary FROM sessions WHERE id=?1",
                params![session_id],
                |row| {
                    Ok(SessionRecord {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        updated_at: row.get(2)?,
                        status: row.get(3)?,
                        root_path: row.get(4)?,
                        category: row.get(5)?,
                        summary: row.get(6)?,
                    })
                },
            )
            .optional()
            .context("querying session")
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, updated_at, status, root_path, category, summary FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                status: row.get(3)?,
                root_path: row.get(4)?,
                category: row.get(5)?,
                summary: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn list_session_summaries(
        &self,
        limit: usize,
        status: Option<&str>,
        category: Option<&str>,
    ) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                s.id,
                s.created_at,
                s.updated_at,
                s.status,
                s.root_path,
                s.category,
                s.summary,
                COALESCE(a.count, 0) AS action_count,
                COALESCE(ar.count, 0) AS artifact_count,
                COALESCE(h.count, 0) AS hypothesis_count,
                COALESCE(n.count, 0) AS note_count,
                COALESCE(c.count, 0) AS citation_count
            FROM sessions s
            LEFT JOIN (
                SELECT session_id, COUNT(*) AS count
                FROM actions
                GROUP BY session_id
            ) a ON a.session_id = s.id
            LEFT JOIN (
                SELECT session_id, COUNT(*) AS count
                FROM artifacts
                GROUP BY session_id
            ) ar ON ar.session_id = s.id
            LEFT JOIN (
                SELECT session_id, COUNT(*) AS count
                FROM hypotheses
                GROUP BY session_id
            ) h ON h.session_id = s.id
            LEFT JOIN (
                SELECT session_id, COUNT(*) AS count
                FROM notes
                GROUP BY session_id
            ) n ON n.session_id = s.id
            LEFT JOIN (
                SELECT session_id, COUNT(*) AS count
                FROM citations
                GROUP BY session_id
            ) c ON c.session_id = s.id
            WHERE (?1 IS NULL OR s.status = ?1)
              AND (?2 IS NULL OR s.category = ?2)
            ORDER BY s.updated_at DESC
            LIMIT ?3
            "#,
        )?;

        let rows = stmt.query_map(params![status, category, limit as i64], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                status: row.get(3)?,
                root_path: row.get(4)?,
                category: row.get(5)?,
                summary: row.get(6)?,
                action_count: row.get(7)?,
                artifact_count: row.get(8)?,
                hypothesis_count: row.get(9)?,
                note_count: row.get(10)?,
                citation_count: row.get(11)?,
            })
        })?;

        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_action(&self, action: NewAction<'_>) -> Result<i64> {
        let ts = now_utc_rfc3339();
        self.conn.execute(
            "INSERT INTO actions(session_id, ts, action_type, command, target, status, stdout, stderr, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                action.session_id,
                ts,
                action.action_type,
                action.command,
                action.target,
                action.status,
                action.stdout,
                action.stderr,
                action
                    .metadata
                    .map(serde_json::to_string)
                    .transpose()?
                    .as_deref(),
            ],
        )?;

        self.touch_session(action.session_id, None, None, None)?;
        self.prune_actions(action.session_id)?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_actions(&self, session_id: &str, limit: usize) -> Result<Vec<ActionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, ts, action_type, command, target, status, stdout, stderr, metadata_json
             FROM actions
             WHERE session_id=?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            let metadata_raw: Option<String> = row.get(9)?;
            let metadata = metadata_raw
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .unwrap_or(None);
            Ok(ActionRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                ts: row.get(2)?,
                action_type: row.get(3)?,
                command: row.get(4)?,
                target: row.get(5)?,
                status: row.get(6)?,
                stdout: row.get(7)?,
                stderr: row.get(8)?,
                metadata,
            })
        })?;

        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn prune_actions(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM actions
             WHERE session_id = ?1
               AND id NOT IN (
                   SELECT id FROM actions WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2
               )",
            params![session_id, self.max_actions_per_session as i64],
        )?;
        Ok(())
    }

    pub fn upsert_artifact(&self, artifact: &ArtifactRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO artifacts(session_id, path, kind, size, sha256, mime, indexed_at, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(session_id, path)
             DO UPDATE SET kind=excluded.kind, size=excluded.size, sha256=excluded.sha256,
                           mime=excluded.mime, indexed_at=excluded.indexed_at, summary=excluded.summary",
            params![
                artifact.session_id,
                artifact.path,
                artifact.kind,
                artifact.size as i64,
                artifact.sha256,
                artifact.mime,
                artifact.indexed_at,
                artifact.summary,
            ],
        )?;
        self.prune_artifacts(&artifact.session_id)?;
        Ok(())
    }

    fn prune_artifacts(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM artifacts
             WHERE session_id = ?1
               AND path NOT IN (
                   SELECT path FROM artifacts WHERE session_id = ?1 ORDER BY indexed_at DESC LIMIT ?2
               )",
            params![session_id, self.max_artifacts_per_session as i64],
        )?;
        Ok(())
    }

    pub fn list_artifacts(&self, session_id: &str, limit: usize) -> Result<Vec<ArtifactRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, path, kind, size, sha256, mime, indexed_at, summary
             FROM artifacts
             WHERE session_id=?1
             ORDER BY indexed_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            Ok(ArtifactRecord {
                session_id: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
                size: row.get::<_, i64>(3)? as u64,
                sha256: row.get(4)?,
                mime: row.get(5)?,
                indexed_at: row.get(6)?,
                summary: row.get(7)?,
            })
        })?;

        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_hypothesis(
        &self,
        session_id: &str,
        text: &str,
        confidence: f32,
        status: &str,
    ) -> Result<i64> {
        let now = now_utc_rfc3339();
        self.conn.execute(
            "INSERT INTO hypotheses(session_id, text, confidence, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, text, confidence, status, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_hypothesis_status(
        &self,
        hypothesis_id: i64,
        status: &str,
        confidence: Option<f32>,
    ) -> Result<()> {
        let now = now_utc_rfc3339();
        self.conn.execute(
            "UPDATE hypotheses
             SET status=?2, confidence=COALESCE(?3, confidence), updated_at=?4
             WHERE id=?1",
            params![hypothesis_id, status, confidence, now],
        )?;
        Ok(())
    }

    pub fn list_hypotheses(&self, session_id: &str, limit: usize) -> Result<Vec<HypothesisRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, text, confidence, status, updated_at
             FROM hypotheses
             WHERE session_id=?1
             ORDER BY confidence DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            Ok(HypothesisRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                text: row.get(2)?,
                confidence: row.get(3)?,
                status: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_note(&self, session_id: &str, note: &str) -> Result<i64> {
        let ts = now_utc_rfc3339();
        self.conn.execute(
            "INSERT INTO notes(session_id, ts, note) VALUES (?1, ?2, ?3)",
            params![session_id, ts, note],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_notes(&self, session_id: &str, limit: usize) -> Result<Vec<NoteRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, ts, note FROM notes WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            Ok(NoteRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                ts: row.get(2)?,
                note: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_citation(
        &self,
        session_id: &str,
        source_type: &str,
        source: &str,
        locator: Option<&str>,
        snippet: &str,
    ) -> Result<i64> {
        let ts = now_utc_rfc3339();
        self.conn.execute(
            "INSERT INTO citations(session_id, source_type, source, locator, snippet, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, source_type, source, locator, snippet, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_citations(&self, session_id: &str, limit: usize) -> Result<Vec<CitationRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, source_type, source, locator, snippet, ts
             FROM citations
             WHERE session_id=?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            Ok(CitationRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                source_type: row.get(2)?,
                source: row.get(3)?,
                locator: row.get(4)?,
                snippet: row.get(5)?,
                ts: row.get(6)?,
            })
        })?;

        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn put_web_cache(
        &self,
        url: &str,
        title: Option<&str>,
        content: &str,
        hash: &str,
    ) -> Result<()> {
        let fetched = now_utc_rfc3339();
        self.conn.execute(
            "INSERT INTO web_cache(url, title, content, fetched_at, hash)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(url)
             DO UPDATE SET title=excluded.title, content=excluded.content,
                           fetched_at=excluded.fetched_at, hash=excluded.hash",
            params![url, title, content, fetched, hash],
        )?;
        self.prune_web_cache()?;
        Ok(())
    }

    pub fn get_web_cache(&self, url: &str) -> Result<Option<WebCacheRecord>> {
        self.conn
            .query_row(
                "SELECT url, title, content, fetched_at, hash FROM web_cache WHERE url=?1",
                params![url],
                |row| {
                    Ok(WebCacheRecord {
                        url: row.get(0)?,
                        title: row.get(1)?,
                        content: row.get(2)?,
                        fetched_at: row.get(3)?,
                        hash: row.get(4)?,
                    })
                },
            )
            .optional()
            .context("querying web cache")
    }

    pub fn search_web_cache(&self, query: &str, limit: usize) -> Result<Vec<WebCacheRecord>> {
        let like = format!("%{}%", query.replace('%', ""));
        let mut stmt = self.conn.prepare(
            "SELECT url, title, content, fetched_at, hash
             FROM web_cache
             WHERE content LIKE ?1 OR title LIKE ?1
             ORDER BY fetched_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![like, limit as i64], |row| {
            Ok(WebCacheRecord {
                url: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                fetched_at: row.get(3)?,
                hash: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn prune_web_cache(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM web_cache WHERE url NOT IN (
               SELECT url FROM web_cache ORDER BY fetched_at DESC LIMIT ?1
             )",
            params![self.max_cache_entries as i64],
        )?;
        Ok(())
    }

    pub fn search_local_notes(&self, query: &str, limit: usize) -> Result<Vec<NoteRecord>> {
        let like = format!("%{}%", query.replace('%', ""));
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, ts, note FROM notes
             WHERE note LIKE ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![like, limit as i64], |row| {
            Ok(NoteRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                ts: row.get(2)?,
                note: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn stats(&self) -> Result<StateStats> {
        Ok(StateStats {
            sessions: count_table(&self.conn, "sessions")?,
            actions: count_table(&self.conn, "actions")?,
            artifacts: count_table(&self.conn, "artifacts")?,
            hypotheses: count_table(&self.conn, "hypotheses")?,
            notes: count_table(&self.conn, "notes")?,
            citations: count_table(&self.conn, "citations")?,
            web_cache: count_table(&self.conn, "web_cache")?,
        })
    }

    pub fn session_stats(&self, session_id: &str) -> Result<StateStats> {
        Ok(StateStats {
            sessions: count_where(&self.conn, "sessions", "id", session_id)?,
            actions: count_where(&self.conn, "actions", "session_id", session_id)?,
            artifacts: count_where(&self.conn, "artifacts", "session_id", session_id)?,
            hypotheses: count_where(&self.conn, "hypotheses", "session_id", session_id)?,
            notes: count_where(&self.conn, "notes", "session_id", session_id)?,
            citations: count_where(&self.conn, "citations", "session_id", session_id)?,
            web_cache: count_table(&self.conn, "web_cache")?,
        })
    }
}

pub struct NewAction<'a> {
    pub session_id: &'a str,
    pub action_type: &'a str,
    pub command: &'a str,
    pub target: Option<&'a str>,
    pub status: &'a str,
    pub stdout: Option<&'a str>,
    pub stderr: Option<&'a str>,
    pub metadata: Option<&'a Value>,
}

fn count_table(conn: &Connection, table: &str) -> Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn.query_row(&sql, [], |row| row.get(0))?)
}

fn count_where(conn: &Connection, table: &str, field: &str, value: &str) -> Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE {field}=?1");
    Ok(conn.query_row(&sql, params![value], |row| row.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stores_and_loads_session() {
        let tmp = tempdir().expect("tmpdir");
        let db = tmp.path().join("state.db");
        let store = StateStore::open(&db, 100, 100, 100).expect("store");
        store.create_session("abc", "/tmp/x").expect("session");

        let session = store
            .get_session("abc")
            .expect("session query")
            .expect("exists");
        assert_eq!(session.id, "abc");
    }
}
