use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    Unknown,
    LoggedIn,
    LoggedOut,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub binary: String,
    pub installed: bool,
    pub auth_status: AuthStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteEntry {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub master_provider: String,
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub routing: Option<HashMap<String, RouteEntry>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn default_routing() -> HashMap<String, RouteEntry> {
    let mut routing = HashMap::new();
    routing.insert(
        "plan".to_string(),
        RouteEntry {
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
        },
    );
    routing.insert(
        "implement".to_string(),
        RouteEntry {
            provider: "codex".to_string(),
            model: None,
        },
    );
    routing.insert(
        "review".to_string(),
        RouteEntry {
            provider: "claude".to_string(),
            model: Some("sonnet".to_string()),
        },
    );
    routing.insert(
        "debug".to_string(),
        RouteEntry {
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
        },
    );
    routing.insert(
        "question".to_string(),
        RouteEntry {
            provider: "claude".to_string(),
            model: Some("sonnet".to_string()),
        },
    );
    routing
}

/// orc's global directory: `$ORC_HOME` if set, else `~/.orc`.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("ORC_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    Ok(home.join(".orc"))
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub fn load_config(path: &Path) -> Result<Config> {
    let data = std::fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&data)?;
    Ok(config)
}

pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(path, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn config_roundtrip() {
        let cfg = Config {
            master_provider: "claude".into(),
            providers: vec![ProviderConfig {
                name: "claude".into(),
                binary: "claude".into(),
                installed: true,
                auth_status: AuthStatus::LoggedIn,
            }],
            routing: Some(default_routing()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let file = NamedTempFile::new().unwrap();
        save_config(file.path(), &cfg).unwrap();
        let loaded = load_config(file.path()).unwrap();
        assert_eq!(loaded.master_provider, "claude");
        assert_eq!(loaded.providers.len(), 1);
        assert_eq!(loaded.providers[0].name, "claude");
        assert_eq!(loaded.providers[0].auth_status, AuthStatus::LoggedIn);
        assert!(loaded.routing.is_some());
    }

    #[test]
    fn config_dir_returns_path() {
        let dir = config_dir().unwrap();
        assert!(dir.to_string_lossy().contains(".orc"));
    }

    #[test]
    fn config_routing_roundtrip() {
        let mut routing = HashMap::new();
        routing.insert(
            "plan".to_string(),
            RouteEntry {
                provider: "claude".to_string(),
                model: Some("opus".to_string()),
            },
        );
        routing.insert(
            "implement".to_string(),
            RouteEntry {
                provider: "codex".to_string(),
                model: None,
            },
        );

        let cfg = Config {
            master_provider: "claude".into(),
            providers: vec![],
            routing: Some(routing),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let file = NamedTempFile::new().unwrap();
        save_config(file.path(), &cfg).unwrap();
        let loaded = load_config(file.path()).unwrap();

        let loaded_routing = loaded.routing.unwrap();
        assert_eq!(loaded_routing["plan"].provider, "claude");
        assert_eq!(loaded_routing["plan"].model, Some("opus".to_string()));
        assert_eq!(loaded_routing["implement"].provider, "codex");
        assert_eq!(loaded_routing["implement"].model, None);
    }

    #[test]
    fn config_without_routing_loads_ok() {
        let json = r#"{"master_provider":"claude","providers":[],"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(cfg.routing.is_none());
    }
}
