use std::collections::HashMap;

use orc_core::config::RouteEntry;

/// Look up the provider and model for a classified category.
pub fn route(
    category: &str,
    routing: &HashMap<String, RouteEntry>,
    master_provider: &str,
) -> RouteEntry {
    if let Some(entry) = routing.get(category) {
        return entry.clone();
    }

    RouteEntry {
        provider: master_provider.to_string(),
        model: None,
    }
}

/// Render a route entry as a user-facing string.
pub fn route_display(entry: &RouteEntry) -> String {
    match &entry.model {
        Some(model) => format!("{} ({})", entry.provider, model),
        None => entry.provider.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orc_core::config::RouteEntry;

    #[test]
    fn route_returns_configured_entry() {
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

        let result = route("plan", &routing, "claude");
        assert_eq!(result.provider, "claude");
        assert_eq!(result.model, Some("opus".to_string()));

        let result = route("implement", &routing, "claude");
        assert_eq!(result.provider, "codex");
        assert_eq!(result.model, None);
    }

    #[test]
    fn route_unknown_category_falls_back_to_master() {
        let routing = HashMap::new();
        let result = route("unknown_category", &routing, "claude");
        assert_eq!(result.provider, "claude");
        assert_eq!(result.model, None);
    }

    #[test]
    fn route_display_formats_correctly() {
        let entry = RouteEntry {
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
        };
        assert_eq!(route_display(&entry), "claude (opus)");

        let entry_no_model = RouteEntry {
            provider: "codex".to_string(),
            model: None,
        };
        assert_eq!(route_display(&entry_no_model), "codex");
    }
}
