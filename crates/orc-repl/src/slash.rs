#[derive(Debug, Clone, PartialEq)]
pub enum SlashCommand {
    Help,
    Models,
    Providers,
    Sessions,
    Set {
        category: String,
        provider: String,
        model: Option<String>,
    },
    Budget {
        amount: Option<f64>,
    },
    Switch {
        provider: String,
    },
    Clear,
}

/// Parse user input as a slash command.
pub fn parse_slash(input: &str) -> Option<SlashCommand> {
    let input = input.trim();
    if input.is_empty() || !input.starts_with('/') {
        return None;
    }

    let parts: Vec<&str> = input.split_whitespace().collect();
    let command = parts.first().copied()?;

    match command {
        "/help" => Some(SlashCommand::Help),
        "/models" => Some(SlashCommand::Models),
        "/providers" => Some(SlashCommand::Providers),
        "/sessions" => Some(SlashCommand::Sessions),
        "/clear" => Some(SlashCommand::Clear),
        "/set" => {
            if parts.len() < 3 {
                return None;
            }
            Some(SlashCommand::Set {
                category: parts[1].to_string(),
                provider: parts[2].to_string(),
                model: parts.get(3).map(|s| s.to_string()),
            })
        }
        "/budget" => Some(SlashCommand::Budget {
            amount: parts.get(1).and_then(|s| s.parse::<f64>().ok()),
        }),
        "/switch" => {
            if parts.len() < 2 {
                return None;
            }
            Some(SlashCommand::Switch {
                provider: parts[1].to_string(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slash_commands() {
        assert_eq!(parse_slash("/help"), Some(SlashCommand::Help));
        assert_eq!(parse_slash("/models"), Some(SlashCommand::Models));
        assert_eq!(parse_slash("/providers"), Some(SlashCommand::Providers));
        assert_eq!(parse_slash("/sessions"), Some(SlashCommand::Sessions));
        assert_eq!(parse_slash("/clear"), Some(SlashCommand::Clear));
    }

    #[test]
    fn parse_set_command() {
        let cmd = parse_slash("/set implement codex");
        assert_eq!(
            cmd,
            Some(SlashCommand::Set {
                category: "implement".to_string(),
                provider: "codex".to_string(),
                model: None,
            })
        );

        let cmd = parse_slash("/set plan claude opus");
        assert_eq!(
            cmd,
            Some(SlashCommand::Set {
                category: "plan".to_string(),
                provider: "claude".to_string(),
                model: Some("opus".to_string()),
            })
        );
    }

    #[test]
    fn parse_budget_command() {
        let cmd = parse_slash("/budget");
        assert_eq!(cmd, Some(SlashCommand::Budget { amount: None }));

        let cmd = parse_slash("/budget 5.00");
        assert_eq!(cmd, Some(SlashCommand::Budget { amount: Some(5.0) }));
    }

    #[test]
    fn parse_switch_command() {
        let cmd = parse_slash("/switch codex");
        assert_eq!(
            cmd,
            Some(SlashCommand::Switch {
                provider: "codex".to_string(),
            })
        );

        // Missing provider argument returns None.
        assert_eq!(parse_slash("/switch"), None);
    }

    #[test]
    fn parse_unknown_slash_returns_none() {
        assert_eq!(parse_slash("/foobar"), None);
    }

    #[test]
    fn non_slash_input_returns_none() {
        assert_eq!(parse_slash("hello world"), None);
        assert_eq!(parse_slash(""), None);
    }
}
