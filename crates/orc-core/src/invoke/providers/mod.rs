//! Headless runners for coding-agent CLIs. One file per CLI.
//! To add one: implement `Provider`, add it to `get_provider` and `all_providers`,
//! and test its command and parser against real output from that CLI.

pub mod amp;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod kimi;
pub mod opencode;
pub mod pi;

pub use amp::AmpProvider;
pub use claude::ClaudeProvider;
pub use codex::CodexProvider;
pub use copilot::CopilotProvider;
pub use gemini::GeminiProvider;
pub use kimi::KimiProvider;
pub use opencode::OpenCodeProvider;
pub use pi::PiProvider;

use crate::invoke::provider::Provider;

/// Get a provider by name
pub fn get_provider(name: &str) -> Option<Box<dyn Provider>> {
    match name {
        "claude" => Some(Box::new(ClaudeProvider)),
        "codex" => Some(Box::new(CodexProvider)),
        "opencode" => Some(Box::new(OpenCodeProvider)),
        "gemini" => Some(Box::new(GeminiProvider)),
        "kimi" => Some(Box::new(KimiProvider)),
        "amp" => Some(Box::new(AmpProvider)),
        "copilot" => Some(Box::new(CopilotProvider)),
        "pi" => Some(Box::new(PiProvider)),
        _ => None,
    }
}

/// All providers that can run tasks headless, in default hand-off order.
pub fn all_providers() -> Vec<&'static str> {
    vec!["claude", "codex", "opencode", "gemini", "kimi", "amp", "copilot", "pi"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_provider_resolves_to_itself() {
        for name in all_providers() {
            let p = get_provider(name).unwrap_or_else(|| panic!("{name} not registered"));
            assert_eq!(p.name(), name);
        }
        assert!(get_provider("nope").is_none());
    }

    #[test]
    fn reviewers_are_the_read_only_ones() {
        let ro: Vec<_> = all_providers().into_iter().filter(|n| get_provider(n).unwrap().supports_read_only()).collect();
        assert_eq!(ro, ["claude", "codex", "opencode", "pi"]);
    }
}
