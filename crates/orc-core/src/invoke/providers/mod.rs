pub mod claude;
pub mod codex;

pub use claude::ClaudeProvider;
pub use codex::CodexProvider;

use crate::invoke::provider::Provider;

/// Get a provider by name
pub fn get_provider(name: &str) -> Option<Box<dyn Provider>> {
    match name {
        "claude" => Some(Box::new(ClaudeProvider)),
        "codex" => Some(Box::new(CodexProvider)),
        _ => None,
    }
}

/// Get all registered provider names
pub fn all_providers() -> Vec<&'static str> {
    vec!["claude", "codex"]
}
