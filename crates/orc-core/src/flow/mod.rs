pub mod compile;
pub mod types;

pub use compile::{compile, normalize_name};
pub use types::{CompileOptions, FlowDefinition, Stage};

use anyhow::Result;
use std::path::Path;

pub fn save_flow(path: &Path, def: &FlowDefinition) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(def)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_flow(path: &Path) -> Result<FlowDefinition> {
    let data = std::fs::read_to_string(path)?;
    let def: FlowDefinition = serde_json::from_str(&data)?;
    if def.name.trim().is_empty() {
        return Err(anyhow::anyhow!("flow name is required"));
    }
    if def.stages.is_empty() {
        return Err(anyhow::anyhow!("flow must include at least one stage"));
    }
    Ok(def)
}
