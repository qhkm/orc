use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub master_provider: String,
    pub providers: Vec<ProviderConfig>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn config_dir() -> Result<PathBuf> {
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
    }

    #[test]
    fn config_dir_returns_path() {
        let dir = config_dir().unwrap();
        assert!(dir.to_string_lossy().contains(".orc"));
    }
}
