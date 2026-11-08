//! Global record of every `orc run` attempt, at `~/.orc/history.jsonl`
//! (`$ORC_HOME/history.jsonl`). Routing learns from it; `orc stats` shows it.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One attempt by one provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub ts: DateTime<Utc>,
    pub repo: String,
    pub run_id: String,
    pub attempt: u32,
    pub category: String,
    pub provider: String,
    pub model: Option<String>,
    /// Verdict label: pass | fail | stuck | limit.
    pub verdict: String,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
}

pub fn history_path() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("history.jsonl"))
}

pub fn append_record(path: &Path, record: &AttemptRecord) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

/// All readable records; a missing file is empty history, unreadable lines are skipped.
pub fn load_records(path: &Path) -> Vec<AttemptRecord> {
    let Ok(data) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    data.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// How one provider has done.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ProviderStats {
    pub provider: String,
    pub attempts: u32,
    pub passes: u32,
    pub fails: u32,
    pub stuck: u32,
    pub limits: u32,
    pub total_duration_ms: u64,
    /// Sum over attempts that reported a cost; None if none did.
    pub total_cost_usd: Option<f64>,
}

impl ProviderStats {
    /// Attempts that say something about skill: everything but rate limits.
    pub fn scored(&self) -> u32 {
        self.attempts - self.limits
    }

    pub fn pass_rate(&self) -> Option<f64> {
        (self.scored() > 0).then(|| self.passes as f64 / self.scored() as f64)
    }

    pub fn avg_duration_ms(&self) -> u64 {
        if self.attempts == 0 { 0 } else { self.total_duration_ms / self.attempts as u64 }
    }

    fn add(&mut self, r: &AttemptRecord) {
        self.attempts += 1;
        match r.verdict.as_str() {
            "pass" => self.passes += 1,
            "fail" => self.fails += 1,
            "stuck" => self.stuck += 1,
            "limit" => self.limits += 1,
            _ => {}
        }
        self.total_duration_ms += r.duration_ms;
        if let Some(cost) = r.cost_usd {
            *self.total_cost_usd.get_or_insert(0.0) += cost;
        }
    }
}

/// Per-provider stats, optionally for one category, sorted by provider name.
pub fn stats_for(records: &[AttemptRecord], category: Option<&str>) -> Vec<ProviderStats> {
    let mut by_provider: BTreeMap<&str, ProviderStats> = BTreeMap::new();
    for r in records.iter().filter(|r| category.is_none_or(|c| r.category == c)) {
        by_provider
            .entry(&r.provider)
            .or_insert_with(|| ProviderStats { provider: r.provider.clone(), ..Default::default() })
            .add(r);
    }
    by_provider.into_values().collect()
}

/// Stats grouped by category.
pub fn stats_by_category(records: &[AttemptRecord]) -> BTreeMap<String, Vec<ProviderStats>> {
    let mut categories: Vec<&str> = records.iter().map(|r| r.category.as_str()).collect();
    categories.sort_unstable();
    categories.dedup();
    categories.into_iter().map(|c| (c.to_string(), stats_for(records, Some(c)))).collect()
}

/// When `provider`'s most recent attempt (any category) was a rate limit, its time.
pub fn last_limited_at(records: &[AttemptRecord], provider: &str) -> Option<DateTime<Utc>> {
    let last = records.iter().filter(|r| r.provider == provider).max_by_key(|r| r.ts)?;
    (last.verdict == "limit").then_some(last.ts)
}

#[cfg(test)]
pub(crate) fn record(provider: &str, category: &str, verdict: &str, minutes_ago: i64) -> AttemptRecord {
    AttemptRecord {
        ts: Utc::now() - chrono::Duration::minutes(minutes_ago),
        repo: "/r".into(),
        run_id: "run".into(),
        attempt: 1,
        category: category.into(),
        provider: provider.into(),
        model: None,
        verdict: verdict.into(),
        duration_ms: 1000,
        cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_load_skips_bad_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h/history.jsonl");
        assert!(load_records(&path).is_empty());
        append_record(&path, &record("claude", "debug", "pass", 0)).unwrap();
        std::fs::OpenOptions::new().append(true).open(&path).map(|mut f| {
            use std::io::Write;
            writeln!(f, "not json").unwrap();
        }).unwrap();
        append_record(&path, &record("codex", "debug", "limit", 0)).unwrap();
        let loaded = load_records(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].provider, "codex");
    }

    #[test]
    fn stats_count_verdicts_and_exclude_limits_from_pass_rate() {
        let mut records = vec![
            record("codex", "debug", "pass", 5),
            record("codex", "debug", "fail", 4),
            record("codex", "debug", "limit", 3),
            record("claude", "debug", "pass", 2),
            record("claude", "implement", "fail", 1),
        ];
        records[0].cost_usd = Some(0.25);
        records[1].cost_usd = Some(0.5);
        let debug = stats_for(&records, Some("debug"));
        let codex = debug.iter().find(|s| s.provider == "codex").unwrap();
        assert_eq!((codex.attempts, codex.passes, codex.fails, codex.limits), (3, 1, 1, 1));
        assert_eq!(codex.scored(), 2);
        assert_eq!(codex.pass_rate(), Some(0.5));
        assert_eq!(codex.total_cost_usd, Some(0.75));
        assert_eq!(debug.len(), 2);

        let by_cat = stats_by_category(&records);
        assert_eq!(by_cat.keys().collect::<Vec<_>>(), ["debug", "implement"]);
        assert_eq!(by_cat["implement"][0].fails, 1);
    }

    #[test]
    fn last_limited_only_when_latest_attempt_was_a_limit() {
        let records = vec![record("codex", "debug", "limit", 30), record("claude", "debug", "limit", 90), record("claude", "x", "pass", 10)];
        assert!(last_limited_at(&records, "codex").is_some());
        assert!(last_limited_at(&records, "claude").is_none(), "a later pass clears it");
        assert!(last_limited_at(&records, "kimi").is_none());
    }
}
