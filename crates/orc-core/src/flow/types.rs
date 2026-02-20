use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub intent: String,
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub profile: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub input_from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub name: String,
    pub source_prompt: String,
    pub master_provider: String,
    pub stages: Vec<Stage>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub name: String,
    pub prompt: String,
    pub master_provider: String,
    pub available_providers: Vec<String>,
}
