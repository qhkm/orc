use chrono::{DateTime, TimeZone, Utc};
use std::path::{Path, PathBuf};
use crate::session::types::{SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Try to parse a timestamp value that may be:
/// - An integer (Unix milliseconds)
/// - A string that is either an ISO-8601 datetime or a stringified integer
///
/// Returns `None` if parsing fails entirely; callers fall back to `Utc::now()`.
fn parse_timestamp(v: &rusqlite::types::Value) -> Option<DateTime<Utc>> {
    match v {
        rusqlite::types::Value::Integer(ms) => {
            // Treat as Unix milliseconds
            let secs = ms / 1000;
            let nanos = ((ms % 1000) * 1_000_000) as u32;
            Utc.timestamp_opt(secs, nanos).single()
        }
        rusqlite::types::Value::Real(f) => {
            let ms = *f as i64;
            let secs = ms / 1000;
            let nanos = ((ms % 1000) * 1_000_000) as u32;
            Utc.timestamp_opt(secs, nanos).single()
        }
        rusqlite::types::Value::Text(s) => {
            // Try ISO string first
            if let Ok(dt) = s.parse::<DateTime<Utc>>() {
                return Some(dt);
            }
            // Try parsing as a numeric string (Unix ms)
            if let Ok(ms) = s.trim().parse::<i64>() {
                let secs = ms / 1000;
                let nanos = ((ms % 1000) * 1_000_000) as u32;
                return Utc.timestamp_opt(secs, nanos).single();
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Core synchronous query — runs inside spawn_blocking
// ---------------------------------------------------------------------------

/// Internal: open the SQLite DB at `db_path` (read-only) and return sessions.
///
/// Tries both `session`/`message` (singular) and `sessions`/`messages` (plural)
/// table names.  Returns an empty vec if the file is absent, unreadable, or has
/// an unexpected schema.
fn query_sessions_sync(db_path: PathBuf) -> Vec<UnifiedSession> {
    use rusqlite::{Connection, OpenFlags};

    let conn = match Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Determine which table names exist in this DB.
    let (session_table, message_table) = detect_tables(&conn);

    let session_table = match session_table {
        Some(t) => t,
        None => return Vec::new(),
    };
    let message_table = match message_table {
        Some(t) => t,
        None => return Vec::new(),
    };

    // -----------------------------------------------------------------------
    // Main query: sessions with message count
    // -----------------------------------------------------------------------
    let main_sql = format!(
        "SELECT s.id, s.title, s.cwd, s.created_at, s.updated_at, COUNT(m.id) AS msg_count \
         FROM {session_table} s \
         LEFT JOIN {message_table} m ON m.session_id = s.id \
         GROUP BY s.id \
         ORDER BY s.updated_at DESC",
        session_table = session_table,
        message_table = message_table,
    );

    let mut stmt = match conn.prepare(&main_sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    #[allow(dead_code)]
    struct Row {
        id: String,
        title: Option<String>,
        cwd: Option<String>,
        created_at: rusqlite::types::Value,
        updated_at: rusqlite::types::Value,
        msg_count: i64,
    }

    let rows: Vec<Row> = match stmt.query_map([], |row| {
        Ok(Row {
            id: row.get(0)?,
            title: row.get(1)?,
            cwd: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            msg_count: row.get(5)?,
        })
    }) {
        Ok(iter) => iter.flatten().collect(),
        Err(_) => return Vec::new(),
    };

    // -----------------------------------------------------------------------
    // Build UnifiedSessions
    // -----------------------------------------------------------------------
    let mut sessions = Vec::new();

    for row in rows {
        // Filter out sessions with no messages
        if row.msg_count == 0 {
            continue;
        }

        let now = Utc::now();

        let updated_at = parse_timestamp(&row.updated_at).unwrap_or(now);
        let created_at = parse_timestamp(&row.created_at).unwrap_or(updated_at);

        // Query first user message for summary
        let summary_sql = format!(
            "SELECT content FROM {message_table} \
             WHERE session_id = ?1 AND role = 'user' \
             ORDER BY created_at ASC LIMIT 1",
            message_table = message_table,
        );
        let summary: String = conn
            .query_row(&summary_sql, rusqlite::params![row.id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_default()
            .replace('\n', " ")
            .trim()
            .chars()
            .take(80)
            .collect();

        let cwd = row
            .cwd
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));

        sessions.push(UnifiedSession {
            id: row.id,
            source: SessionSource::OpenCode,
            cwd,
            repo: None,
            summary,
            updated_at,
            created_at,
            message_count: row.msg_count as usize,
            files_modified: vec![],
            original_path: db_path.clone(),
        });
    }

    sessions
}

/// Check which session/message table names actually exist in the DB.
/// Returns `(Option<session_table_name>, Option<message_table_name>)`.
fn detect_tables(conn: &rusqlite::Connection) -> (Option<String>, Option<String>) {
    let table_names: Vec<String> = {
        let mut stmt = match conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table'",
        ) {
            Ok(s) => s,
            Err(_) => return (None, None),
        };
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map(|iter| iter.flatten().collect())
            .unwrap_or_default()
    };

    let session_table = if table_names.iter().any(|n| n == "session") {
        Some("session".to_string())
    } else if table_names.iter().any(|n| n == "sessions") {
        Some("sessions".to_string())
    } else {
        None
    };

    let message_table = if table_names.iter().any(|n| n == "message") {
        Some("message".to_string())
    } else if table_names.iter().any(|n| n == "messages") {
        Some("messages".to_string())
    } else {
        None
    };

    (session_table, message_table)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover OpenCode sessions from a specific SQLite DB path.
///
/// `db_path` is the full path to the `.db` file (not a directory).
/// Returns an empty vec silently if the file doesn't exist or the schema
/// is unrecognised.
pub async fn discover_opencode_sessions_in(db_path: &Path) -> Vec<UnifiedSession> {
    let db_path = db_path.to_path_buf();
    // rusqlite is not Send/Sync, must run in spawn_blocking
    tokio::task::spawn_blocking(move || query_sessions_sync(db_path))
        .await
        .unwrap_or_default()
}

/// Discover OpenCode sessions from the default location:
/// `~/.local/share/opencode/opencode.db`.
pub async fn discover_opencode_sessions() -> Vec<UnifiedSession> {
    let db_path = match dirs::home_dir() {
        Some(h) => h
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db"),
        None => return Vec::new(),
    };
    discover_opencode_sessions_in(&db_path).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    // Helper: build a runtime and run a future synchronously.
    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    // ------------------------------------------------------------------
    // Test 1: non-existent path → empty vec, no panic
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_nonexistent_db() {
        let result = run(discover_opencode_sessions_in(Path::new(
            "/nonexistent/path/opencode.db",
        )));
        assert!(result.is_empty(), "Missing DB should yield empty vec");
    }

    // ------------------------------------------------------------------
    // Test 2: real DB with one session and two messages
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_real_db_basic() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                cwd TEXT,
                created_at INTEGER,
                updated_at INTEGER
            );
            CREATE TABLE message (
                id TEXT,
                session_id TEXT,
                role TEXT,
                content TEXT,
                created_at INTEGER
            );
            INSERT INTO session VALUES ('sess1', 'Test Session', '/home/user/project', 1700000000000, 1700000100000);
            INSERT INTO message VALUES ('msg1', 'sess1', 'user', 'Add auth', 1700000000000);
            INSERT INTO message VALUES ('msg2', 'sess1', 'assistant', 'OK', 1700000050000);",
        )
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1, "Should discover exactly one session");
        assert_eq!(sessions[0].id, "sess1");
        assert_eq!(sessions[0].source, SessionSource::OpenCode);
        assert_eq!(sessions[0].cwd, PathBuf::from("/home/user/project"));
    }

    // ------------------------------------------------------------------
    // Test 3: message_count is correct
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_message_count() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, created_at INTEGER, updated_at INTEGER);
            CREATE TABLE message (id TEXT, session_id TEXT, role TEXT, content TEXT, created_at INTEGER);
            INSERT INTO session VALUES ('s1', 'S1', '/tmp', 1700000000000, 1700000100000);
            INSERT INTO message VALUES ('m1', 's1', 'user', 'First', 1700000000000);
            INSERT INTO message VALUES ('m2', 's1', 'assistant', 'Reply', 1700000050000);
            INSERT INTO message VALUES ('m3', 's1', 'user', 'Second', 1700000060000);",
        )
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message_count, 3, "Should count all 3 messages");
    }

    // ------------------------------------------------------------------
    // Test 4: summary is extracted from first user message (truncated to 80)
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_summary_extraction() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        let long_msg = "A".repeat(200);
        conn.execute_batch(&format!(
            "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, created_at INTEGER, updated_at INTEGER);
            CREATE TABLE message (id TEXT, session_id TEXT, role TEXT, content TEXT, created_at INTEGER);
            INSERT INTO session VALUES ('s1', 'Test', '/tmp', 1700000000000, 1700000100000);
            INSERT INTO message VALUES ('m1', 's1', 'user', '{long_msg}', 1700000000000);
            INSERT INTO message VALUES ('m2', 's1', 'assistant', 'Done', 1700000050000);",
            long_msg = long_msg,
        ))
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        let summary = &sessions[0].summary;
        assert_eq!(
            summary.len(),
            80,
            "Summary should be truncated to 80 chars, got len={}",
            summary.len()
        );
        assert!(summary.chars().all(|c| c == 'A'), "Summary should be all 'A's");
    }

    // ------------------------------------------------------------------
    // Test 5: sessions with 0 messages are filtered out
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_empty_sessions_filtered() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, created_at INTEGER, updated_at INTEGER);
            CREATE TABLE message (id TEXT, session_id TEXT, role TEXT, content TEXT, created_at INTEGER);
            -- session with messages
            INSERT INTO session VALUES ('s1', 'Active', '/tmp', 1700000000000, 1700000100000);
            INSERT INTO message VALUES ('m1', 's1', 'user', 'Hello', 1700000000000);
            INSERT INTO message VALUES ('m2', 's1', 'assistant', 'Hi', 1700000050000);
            -- session with NO messages (should be filtered)
            INSERT INTO session VALUES ('s2', 'Empty', '/tmp', 1700000000000, 1700000100000);",
        )
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert_eq!(
            sessions.len(),
            1,
            "Session with 0 messages should be filtered out"
        );
        assert_eq!(sessions[0].id, "s1");
    }

    // ------------------------------------------------------------------
    // Test 6: plural table names (sessions / messages) also work
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_plural_table_names() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, created_at INTEGER, updated_at INTEGER);
            CREATE TABLE messages (id TEXT, session_id TEXT, role TEXT, content TEXT, created_at INTEGER);
            INSERT INTO sessions VALUES ('s1', 'Plural', '/home', 1700000000000, 1700000100000);
            INSERT INTO messages VALUES ('m1', 's1', 'user', 'Plural table test', 1700000000000);
            INSERT INTO messages VALUES ('m2', 's1', 'assistant', 'Works', 1700000050000);",
        )
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1, "Plural table names should be supported");
        assert_eq!(sessions[0].id, "s1");
        assert!(
            sessions[0].summary.contains("Plural table test"),
            "Summary should come from first user message"
        );
    }

    // ------------------------------------------------------------------
    // Test 7: ISO string timestamps are parsed correctly
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_iso_timestamps() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, created_at TEXT, updated_at TEXT);
            CREATE TABLE message (id TEXT, session_id TEXT, role TEXT, content TEXT, created_at TEXT);
            INSERT INTO session VALUES ('s1', 'ISO', '/tmp', '2023-11-14T23:33:20Z', '2023-11-14T23:35:00Z');
            INSERT INTO message VALUES ('m1', 's1', 'user', 'ISO ts test', '2023-11-14T23:33:20Z');",
        )
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        // Verify the timestamp was parsed (should not be Utc::now())
        let expected: DateTime<Utc> = "2023-11-14T23:35:00Z".parse().unwrap();
        assert_eq!(sessions[0].updated_at, expected);
    }

    // ------------------------------------------------------------------
    // Test 8: DB with wrong/missing schema → empty vec, no panic
    // ------------------------------------------------------------------
    #[test]
    fn test_opencode_wrong_schema() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE unrelated (foo TEXT, bar INTEGER);
            INSERT INTO unrelated VALUES ('a', 1);",
        )
        .unwrap();
        drop(conn);

        let sessions = run(discover_opencode_sessions_in(tmp.path()));
        assert!(
            sessions.is_empty(),
            "Unknown schema should yield empty vec silently"
        );
    }
}
