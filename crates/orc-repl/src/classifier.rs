use orc_core::invoke::provider::{InvokeOpts, Provider};
use orc_core::invoke::stream::execute;

const VALID_CATEGORIES: &[&str] = &["plan", "implement", "review", "debug", "question"];

/// Build the prompt sent to the master provider for classification.
pub fn build_classify_prompt(user_message: &str) -> String {
    format!(
        "Classify this coding task into exactly one category. Respond with JSON only, no other text.\n\
Task: \"{}\"\n\
Categories: plan, implement, review, debug, question\n\
Respond: {{\"category\": \"<one of the above>\"}}",
        user_message
    )
}

fn parse_category(value: &serde_json::Value) -> Option<String> {
    let category = value.get("category")?.as_str()?;
    if VALID_CATEGORIES.contains(&category) {
        return Some(category.to_string());
    }
    None
}

/// Parse the classifier response and return a valid category.
pub fn parse_classify_response(response: &str) -> Option<String> {
    let trimmed = response.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(category) = parse_category(&value) {
            return Some(category);
        }
    }

    for line in response.lines() {
        let line = line.trim();
        if line.starts_with('{') && line.ends_with('}') {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(category) = parse_category(&value) {
                    return Some(category);
                }
            }
        }
    }

    if let Some(start) = response.find('{') {
        if let Some(end_rel) = response[start..].find('}') {
            let end = start + end_rel + 1;
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response[start..end]) {
                if let Some(category) = parse_category(&value) {
                    return Some(category);
                }
            }
        }
    }

    None
}

/// Classify a user message by invoking the master provider.
pub async fn classify(
    provider: &dyn Provider,
    user_message: &str,
    working_dir: &str,
) -> Option<String> {
    let prompt = build_classify_prompt(user_message);
    let opts = InvokeOpts {
        message: prompt,
        working_dir: working_dir.to_string(),
        model: Some("haiku".to_string()),
        timeout_secs: 15,
        ..Default::default()
    };

    match execute(provider, opts).await {
        Ok(result) => parse_classify_response(&result.response)
            .or_else(|| parse_classify_response(&result.raw_output)),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_classify_response_valid_json() {
        let response = r#"{"category": "implement"}"#;
        let result = parse_classify_response(response);
        assert_eq!(result, Some("implement".to_string()));
    }

    #[test]
    fn parse_classify_response_with_extra_text() {
        let response = "Here is the classification:\n```json\n{\"category\": \"plan\"}\n```";
        let result = parse_classify_response(response);
        assert_eq!(result, Some("plan".to_string()));
    }

    #[test]
    fn parse_classify_response_invalid() {
        let response = "I'm not sure what you mean.";
        let result = parse_classify_response(response);
        assert!(result.is_none());
    }

    #[test]
    fn parse_classify_response_unknown_category_returns_none() {
        let response = r#"{"category": "dance"}"#;
        let result = parse_classify_response(response);
        assert!(result.is_none());
    }

    #[test]
    fn build_classify_prompt_includes_message() {
        let prompt = build_classify_prompt("add rate limiting to the API");
        assert!(prompt.contains("add rate limiting to the API"));
        assert!(prompt.contains("plan"));
        assert!(prompt.contains("implement"));
        assert!(prompt.contains("review"));
        assert!(prompt.contains("debug"));
        assert!(prompt.contains("question"));
    }
}
