use regex::Regex;
use crate::flow::types::{CompileOptions, FlowDefinition, Stage};

const MAX_INTENT_ALIAS_DISTANCE: usize = 120;

struct IntentRule {
    id: &'static str,
    keywords: &'static [&'static str],
    default_provider: &'static str,
}

static RULES: &[IntentRule] = &[
    IntentRule {
        id: "brainstorm",
        keywords: &["brainstorm", "brainstorming", "ideate", "planning", "plan"],
        default_provider: "claude",
    },
    IntentRule {
        id: "implement",
        keywords: &["implement", "implementation", "build", "develop", "write code", "coding"],
        default_provider: "codex",
    },
    IntentRule {
        id: "cross_check",
        keywords: &["cross check", "cross-check", "double check", "validate", "verify"],
        default_provider: "kimi",
    },
    IntentRule {
        id: "review",
        keywords: &["review", "audit", "critique"],
        default_provider: "claude",
    },
];

static PROVIDER_ALIASES: &[(&str, &[&str])] = &[
    ("claude", &["claude code", "claude"]),
    ("codex", &["codex"]),
    ("opencode", &["opencode", "open code"]),
    ("kimi", &["kimi"]),
];

static PROVIDER_ORDER: &[&str] = &["claude", "codex", "kimi", "opencode"];

pub fn compile(opts: CompileOptions) -> anyhow::Result<FlowDefinition> {
    let prompt = opts.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(anyhow::anyhow!("prompt is required"));
    }

    let master = normalize_provider(&opts.master_provider);
    let master = if master.is_empty() {
        pick_available_or_default(&available_set(&opts.available_providers), PROVIDER_ORDER, "codex")
    } else {
        master
    };
    let available = available_set(&opts.available_providers);

    let lower_prompt = prompt.to_lowercase();
    let mut stages: Vec<Stage> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for rule in RULES {
        if !contains_any(&lower_prompt, rule.keywords) {
            continue;
        }

        let (provider, profile) = provider_for_intent(&lower_prompt, rule, &master, &available);
        let provider = if provider.is_empty() { master.clone() } else { provider };

        let stage_num = stages.len() + 1;
        let stage_id = format!("{}_{}", rule.id, stage_num);
        let input_from = stages.last().map(|s| s.id.clone()).unwrap_or_default();

        if !available.is_empty() && !available.contains(provider.as_str()) {
            warnings.push(format!(
                "provider {:?} for stage {:?} is not registered in config",
                provider, rule.id
            ));
        }

        stages.push(Stage {
            id: stage_id,
            intent: rule.id.to_string(),
            provider,
            profile,
            input_from,
        });
    }

    if stages.is_empty() {
        let (provider, profile) = if lower_prompt.contains("opus") {
            ("claude".to_string(), "opus".to_string())
        } else {
            (master.clone(), String::new())
        };

        if !available.is_empty() && !available.contains(provider.as_str()) {
            warnings.push(format!("provider {:?} is not registered in config", provider));
        }

        stages.push(Stage {
            id: "task_1".to_string(),
            intent: "task".to_string(),
            provider,
            profile,
            input_from: String::new(),
        });
    }

    let name = normalize_name(&opts.name);
    let name = if name.is_empty() { "default_flow".to_string() } else { name };

    warnings.sort();

    Ok(FlowDefinition {
        name,
        source_prompt: prompt,
        master_provider: master,
        stages,
        warnings,
        created_at: chrono::Utc::now(),
    })
}

pub fn normalize_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    if lower.is_empty() {
        return String::new();
    }
    let re = Regex::new(r"[^a-z0-9]+").unwrap();
    let normalized = re.replace_all(&lower, "_").to_string();
    normalized.trim_matches('_').to_string()
}

fn normalize_provider(value: &str) -> String {
    let trimmed = value.trim().to_lowercase();
    match trimmed.as_str() {
        "claude code" => "claude".to_string(),
        "open code" => "opencode".to_string(),
        _ => trimmed,
    }
}

fn available_set(providers: &[String]) -> std::collections::HashSet<String> {
    providers
        .iter()
        .map(|p| normalize_provider(p))
        .filter(|p| !p.is_empty())
        .collect()
}

fn contains_any(prompt: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| prompt.contains(&kw.to_lowercase()))
}

fn provider_for_intent(
    prompt: &str,
    rule: &IntentRule,
    master: &str,
    available: &std::collections::HashSet<String>,
) -> (String, String) {
    // Special case: review + opus nearby
    if rule.id == "review" && terms_near(prompt, rule.keywords, "opus", MAX_INTENT_ALIAS_DISTANCE) {
        return ("claude".to_string(), "opus".to_string());
    }

    if let Some(provider) = best_provider_for_keywords(prompt, rule.keywords) {
        let profile = if rule.id == "review" && prompt.contains("opus") && provider == "claude" {
            "opus".to_string()
        } else {
            String::new()
        };
        return (provider, profile);
    }

    let mut default_provider = normalize_provider(rule.default_provider);
    if !available.is_empty() && !available.contains(default_provider.as_str()) {
        if available.contains(master) {
            default_provider = master.to_string();
        } else {
            default_provider =
                pick_available_or_default(available, PROVIDER_ORDER, &default_provider);
        }
    }

    let profile =
        if rule.id == "review" && prompt.contains("opus") && default_provider == "claude" {
            "opus".to_string()
        } else {
            String::new()
        };

    (default_provider, profile)
}

fn best_provider_for_keywords(prompt: &str, keywords: &[&str]) -> Option<String> {
    let mut best_provider: Option<String> = None;
    let mut best_distance = MAX_INTENT_ALIAS_DISTANCE + 1;

    for &provider in PROVIDER_ORDER {
        if let Some(aliases) = PROVIDER_ALIASES
            .iter()
            .find(|(p, _)| *p == provider)
            .map(|(_, a)| *a)
        {
            for &alias in aliases {
                for &keyword in keywords {
                    if let Some(distance) =
                        min_distance_between_terms(prompt, alias, keyword)
                    {
                        if distance <= MAX_INTENT_ALIAS_DISTANCE && distance < best_distance {
                            best_distance = distance;
                            best_provider = Some(provider.to_string());
                        }
                    }
                }
            }
        }
    }

    best_provider
}

fn terms_near(prompt: &str, keywords: &[&str], alias: &str, max_distance: usize) -> bool {
    keywords.iter().any(|&kw| {
        min_distance_between_terms(prompt, alias, kw)
            .map(|d| d <= max_distance)
            .unwrap_or(false)
    })
}

fn min_distance_between_terms(text: &str, first: &str, second: &str) -> Option<usize> {
    let first_positions = indices_for_term(text, first);
    let second_positions = indices_for_term(text, second);

    if first_positions.is_empty() || second_positions.is_empty() {
        return None;
    }

    let mut best: Option<usize> = None;
    for &a in &first_positions {
        for &b in &second_positions {
            let distance = a.abs_diff(b);
            match best {
                None => best = Some(distance),
                Some(prev) if distance < prev => best = Some(distance),
                _ => {}
            }
        }
    }
    best
}

fn indices_for_term(text: &str, term: &str) -> Vec<usize> {
    let normalized = term.trim().to_lowercase();
    if normalized.is_empty() {
        return vec![];
    }
    let escaped = regex::escape(&normalized);
    let pattern = format!(r"\b{}\b", escaped);
    let Ok(re) = Regex::new(&pattern) else {
        return vec![];
    };
    re.find_iter(text).map(|m| m.start()).collect()
}

fn pick_available_or_default(
    available: &std::collections::HashSet<String>,
    preferred: &[&str],
    fallback: &str,
) -> String {
    if available.is_empty() {
        return fallback.to_string();
    }
    for &p in preferred {
        if available.contains(p) {
            return p.to_string();
        }
    }
    if available.contains(fallback) {
        return fallback.to_string();
    }
    available.iter().next().cloned().unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::types::CompileOptions;

    fn opts(prompt: &str) -> CompileOptions {
        CompileOptions {
            name: "test".into(),
            prompt: prompt.into(),
            master_provider: "claude".into(),
            available_providers: vec![],
        }
    }

    #[test]
    fn single_stage_brainstorm() {
        let def = compile(opts("brainstorm ideas for a new feature")).unwrap();
        assert_eq!(def.stages.len(), 1);
        assert_eq!(def.stages[0].intent, "brainstorm");
        assert_eq!(def.stages[0].provider, "claude");
        assert!(def.stages[0].input_from.is_empty());
    }

    #[test]
    fn four_stages_full_flow() {
        let def = compile(opts(
            "claude brainstorm, codex implement, kimi cross-check, review with opus",
        ))
        .unwrap();
        assert_eq!(def.stages.len(), 4);
        assert_eq!(def.stages[0].intent, "brainstorm");
        assert_eq!(def.stages[1].intent, "implement");
        assert_eq!(def.stages[2].intent, "cross_check");
        assert_eq!(def.stages[3].intent, "review");
        // Stages chain
        assert!(def.stages[0].input_from.is_empty());
        assert_eq!(def.stages[1].input_from, def.stages[0].id);
        assert_eq!(def.stages[2].input_from, def.stages[1].id);
        assert_eq!(def.stages[3].input_from, def.stages[2].id);
    }

    #[test]
    fn empty_prompt_errors() {
        let result = compile(opts(""));
        assert!(result.is_err());
    }

    #[test]
    fn no_matching_intent_creates_task_stage() {
        let def = compile(opts("just do something random")).unwrap();
        assert_eq!(def.stages.len(), 1);
        assert_eq!(def.stages[0].intent, "task");
        assert_eq!(def.stages[0].id, "task_1");
    }

    #[test]
    fn review_with_opus_sets_profile() {
        let def = compile(opts("review with opus")).unwrap();
        assert_eq!(def.stages[0].intent, "review");
        assert_eq!(def.stages[0].profile, "opus");
    }

    #[test]
    fn normalize_name_basic() {
        assert_eq!(normalize_name("My Flow Name"), "my_flow_name");
        assert_eq!(normalize_name("  hello world  "), "hello_world");
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn flow_roundtrip_json() {
        use tempfile::NamedTempFile;
        use crate::flow::{save_flow, load_flow};

        let def = compile(opts("brainstorm and implement")).unwrap();
        let file = NamedTempFile::new().unwrap();
        save_flow(file.path(), &def).unwrap();
        let loaded = load_flow(file.path()).unwrap();
        assert_eq!(loaded.name, def.name);
        assert_eq!(loaded.stages.len(), def.stages.len());
    }
}
