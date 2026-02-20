use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use crate::session::claude::discover_claude_sessions_in;
use crate::session::codex::discover_codex_sessions_in;
use crate::session::gemini::discover_gemini_sessions_in;
use crate::session::copilot::discover_copilot_sessions_in;
use crate::session::opencode::discover_opencode_sessions_in;
use crate::session::kimi::discover_kimi_sessions_in;
use crate::session::amp::discover_amp_sessions_in;
use crate::session::droid::discover_droid_sessions_in;
use crate::session::types::UnifiedSession;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndex {
    pub sessions: Vec<UnifiedSession>,
    pub built_at: DateTime<Utc>,
}

pub async fn build_index(claude_root: &Path, codex_root: &Path) -> Result<SessionIndex> {
    build_index_with_gemini(claude_root, codex_root, None).await
}

pub async fn build_index_with_gemini(
    claude_root: &Path,
    codex_root: &Path,
    gemini_root: Option<&Path>,
) -> Result<SessionIndex> {
    build_index_full(claude_root, codex_root, gemini_root, None, None).await
}

/// Full index builder: Claude + Codex + Gemini + Copilot + OpenCode + Kimi + Amp + Droid.
///
/// The first five parsers run in parallel via `tokio::join!`.
/// Kimi, Amp, and Droid are added sequentially afterwards (they are stubs/best-guess
/// parsers that return quickly, so the overhead is negligible).
///
/// `copilot_root` overrides the default `~/.copilot/session-state/`.
/// `opencode_db` overrides the default `~/.local/share/opencode/opencode.db`.
/// Pass `None` for either to use their defaults.  If the path doesn't exist
/// the arm silently returns an empty vec.
pub async fn build_index_full(
    claude_root: &Path,
    codex_root: &Path,
    gemini_root: Option<&Path>,
    copilot_root: Option<&Path>,
    opencode_db: Option<PathBuf>,
) -> Result<SessionIndex> {
    // Determine Gemini root: use provided override or default to ~/.gemini/tmp
    let gemini_root_owned: PathBuf = match gemini_root {
        Some(p) => p.to_path_buf(),
        None => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".gemini")
            .join("tmp"),
    };

    // Determine Copilot root: use provided override or default to ~/.copilot/session-state
    let copilot_root_owned: PathBuf = match copilot_root {
        Some(p) => p.to_path_buf(),
        None => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".copilot")
            .join("session-state"),
    };

    // Determine OpenCode DB path: use provided override or default to ~/.local/share/opencode/opencode.db
    let opencode_db_owned: PathBuf = opencode_db.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db")
    });

    // Default roots for the new parsers
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let kimi_root = home.join(".kimi").join("sessions");
    let amp_root = {
        let primary = home.join(".amp").join("sessions");
        if primary.exists() {
            primary
        } else {
            home.join(".config").join("amp").join("sessions")
        }
    };
    let droid_root = home.join(".factory").join("sessions");

    // codex discovery is sync; wrap in spawn_blocking so it can run in parallel
    let codex_root_owned = codex_root.to_path_buf();
    let (claude_result, codex_result, gemini_sessions, copilot_sessions, opencode_sessions) = tokio::join!(
        discover_claude_sessions_in(claude_root),
        tokio::task::spawn_blocking(move || discover_codex_sessions_in(&codex_root_owned)),
        discover_gemini_sessions_in(&gemini_root_owned),
        discover_copilot_sessions_in(&copilot_root_owned),
        discover_opencode_sessions_in(&opencode_db_owned),
    );

    let mut all = Vec::new();
    all.extend(claude_result.unwrap_or_default());
    // spawn_blocking returns Result<Result<...>>, so double-unwrap
    all.extend(codex_result.ok().and_then(|r| r.ok()).unwrap_or_default());
    all.extend(gemini_sessions);
    all.extend(copilot_sessions);
    all.extend(opencode_sessions);

    // Kimi, Amp, Droid — added sequentially (stubs/best-guess; silently no-op if paths missing)
    let kimi_sessions = discover_kimi_sessions_in(&kimi_root).await;
    let amp_sessions = discover_amp_sessions_in(&amp_root).await;
    let droid_sessions = discover_droid_sessions_in(&droid_root).await;
    all.extend(kimi_sessions);
    all.extend(amp_sessions);
    all.extend(droid_sessions);

    all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(SessionIndex {
        sessions: all,
        built_at: Utc::now(),
    })
}

pub fn save_index(path: &Path, index: &SessionIndex) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    // First line: metadata with built_at
    let meta = serde_json::json!({ "built_at": index.built_at, "_meta": true });
    writeln!(file, "{}", serde_json::to_string(&meta)?)?;
    // Remaining lines: one session per line
    for session in &index.sessions {
        writeln!(file, "{}", serde_json::to_string(session)?)?;
    }
    Ok(())
}

pub fn load_index(path: &Path) -> Result<SessionIndex> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut lines = reader.lines();

    // First line is metadata
    let first = lines.next().ok_or_else(|| anyhow::anyhow!("empty index file"))??;
    let meta: serde_json::Value = serde_json::from_str(&first)?;
    let built_at: DateTime<Utc> = serde_json::from_value(
        meta.get("built_at").cloned().ok_or_else(|| anyhow::anyhow!("missing built_at"))?
    )?;

    let mut sessions = Vec::new();
    for line in lines {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        if let Ok(session) = serde_json::from_str::<UnifiedSession>(&line) {
            sessions.push(session);
        }
    }

    Ok(SessionIndex { sessions, built_at })
}

pub fn is_stale(index: &SessionIndex, ttl_secs: i64) -> bool {
    let age = Utc::now().signed_duration_since(index.built_at);
    age.num_seconds() > ttl_secs
}

pub fn default_index_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    Ok(home.join(".orc").join("sessions.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_claude_fixture(dir: &Path) {
        let project_dir = dir.join("claude_root").join("-Users-me-project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_file = project_dir.join("abc12345-0000-0000-0000-000000000001.jsonl");
        let lines = vec![
            r#"{"type":"user","uuid":"u1","timestamp":"2026-02-20T10:00:00Z","sessionId":"abc12345-0000-0000-0000-000000000001","cwd":"/Users/me/project","message":{"role":"user","content":"Claude task"}}"#,
            r#"{"type":"assistant","uuid":"u2","timestamp":"2026-02-20T10:01:00Z","message":{"role":"assistant","content":"Done."}}"#,
        ];
        fs::write(&session_file, lines.join("\n")).unwrap();
    }

    fn write_codex_fixture(dir: &Path) {
        let codex_root = dir.join("codex_root");
        fs::create_dir_all(&codex_root).unwrap();
        let session_file = codex_root.join("rollout-xyz789abc.jsonl");
        let lines = vec![
            r#"{"timestamp":"2026-02-19T10:00:00Z","type":"session_meta","payload":{"id":"ses_xyz789","cwd":"/Users/me/project2","git":{"branch":"dev"}}}"#,
            r#"{"timestamp":"2026-02-19T10:01:00Z","type":"event_msg","payload":{"role":"user","content":[{"type":"input_text","text":"Codex task"}]}}"#,
            r#"{"timestamp":"2026-02-19T10:02:00Z","type":"event_msg","payload":{"role":"assistant","content":[{"type":"output_text","text":"Done."}]}}"#,
        ];
        fs::write(&session_file, lines.join("\n")).unwrap();
    }

    #[tokio::test]
    async fn build_index_merges_both_parsers() {
        let tmp = TempDir::new().unwrap();
        write_claude_fixture(tmp.path());
        write_codex_fixture(tmp.path());

        let claude_root = tmp.path().join("claude_root");
        let codex_root = tmp.path().join("codex_root");

        let index = build_index(&claude_root, &codex_root).await.unwrap();
        assert!(index.sessions.len() >= 2, "Should have at least 2 sessions");
    }

    #[tokio::test]
    async fn index_roundtrip() {
        let tmp = TempDir::new().unwrap();
        write_claude_fixture(tmp.path());
        write_codex_fixture(tmp.path());

        let claude_root = tmp.path().join("claude_root");
        let codex_root = tmp.path().join("codex_root");

        let index = build_index(&claude_root, &codex_root).await.unwrap();
        let count = index.sessions.len();

        let index_path = tmp.path().join("sessions.jsonl");
        save_index(&index_path, &index).unwrap();
        let loaded = load_index(&index_path).unwrap();

        assert_eq!(loaded.sessions.len(), count);
    }

    #[test]
    fn staleness_check() {
        let old_index = SessionIndex {
            sessions: vec![],
            built_at: Utc::now() - chrono::Duration::seconds(400),
        };
        assert!(is_stale(&old_index, 300), "Index older than TTL should be stale");

        let fresh_index = SessionIndex {
            sessions: vec![],
            built_at: Utc::now(),
        };
        assert!(!is_stale(&fresh_index, 300), "Fresh index should not be stale");
    }
}
